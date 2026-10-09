//! Stable, content-addressed installation of the running Unix helper.

use anyhow::{Context as _, Result, bail, ensure};
use fs2::FileExt as _;
use pi_core::ssh::{PROTOCOL_VERSION, VERSION};
use sha2::{Digest as _, Sha256};
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{Read as _, Seek as _, SeekFrom, Write as _},
    path::{Path, PathBuf},
    process::Command,
};

const HELPER_NAME: &str = "pi-desktop-remote";
const MAX_HELPER_SIZE: u64 = 512 * 1024 * 1024;

pub(crate) struct Activation {
    pub(crate) stable: PathBuf,
    pub(crate) target: PathBuf,
    pub(crate) hash: String,
    pub(crate) migrated: usize,
}

#[derive(Debug, PartialEq, Eq)]
struct InstallPaths {
    bin: PathBuf,
    immutable_dir: PathBuf,
    immutable: PathBuf,
    stable: PathBuf,
    relative_target: PathBuf,
}

fn install_paths(home: &Path, hash: &str) -> Result<InstallPaths> {
    ensure!(home.is_absolute(), "The home folder must be absolute");
    ensure!(
        hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Invalid helper digest"
    );
    let bin = home.join(".pi/desktop/bin");
    let immutable_dir = bin.join(hash);
    Ok(InstallPaths {
        immutable: immutable_dir.join(HELPER_NAME),
        stable: bin.join(HELPER_NAME),
        relative_target: PathBuf::from(hash).join(HELPER_NAME),
        bin,
        immutable_dir,
    })
}

pub(crate) fn stable_path() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("This account has no home folder")?
        .join(if cfg!(windows) {
            ".pi/desktop/bin/pi-desktop-remote.exe"
        } else {
            ".pi/desktop/bin/pi-desktop-remote"
        }))
}

pub(crate) fn active_path() -> Result<PathBuf> {
    let stable = stable_path()?;
    #[cfg(unix)]
    {
        let selected = stable
            .canonicalize()
            .context("The stable helper path is not active")?;
        let running = env::current_exe()?
            .canonicalize()
            .context("The running helper cannot be used for a lasting pairing")?;
        ensure!(
            selected == running,
            "The stable helper path does not name the running helper"
        );
    }
    Ok(stable)
}

/// Installs and activates the running helper. The command accepts no path from
/// its caller; its source is the current executable and its destination is
/// derived from this account's home directory and the executable's SHA-256.
pub(crate) fn activate() -> Result<()> {
    activate_with_report().map(|_| ())
}

pub(crate) fn activate_with_report() -> Result<Activation> {
    #[cfg(not(unix))]
    bail!("Helper activation is available only on Unix");

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;

        let source_path = env::current_exe()?
            .canonicalize()
            .context("The running helper cannot be activated")?;
        // This one descriptor remains open from validation and hashing through
        // copying, so pathname replacement cannot redirect the installation.
        let mut source = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&source_path)
            .with_context(|| {
                format!(
                    "Could not open the running helper at {}",
                    source_path.display()
                )
            })?;
        let metadata = source.metadata()?;
        ensure!(
            metadata.is_file(),
            "The running helper is not a regular file"
        );
        ensure!(
            metadata.len() > 0 && metadata.len() <= MAX_HELPER_SIZE,
            "The running helper has an invalid size"
        );
        validate_native(&mut source)?;
        let hash = digest_file(&mut source)?;

        let home = dirs::home_dir().context("This account has no home folder")?;
        let paths = install_paths(&home, &hash)?;
        create_private_directories(&paths.bin)?;
        let lock_path = paths.bin.join(".pi-desktop-remote.activation.lock");
        let lock = open_lock(&lock_path)?;
        lock.lock_exclusive()?;

        ensure_private_directory(&paths.immutable_dir)?;
        install_content_object(&mut source, &paths.immutable, &hash)?;
        validate_content_object(&paths.immutable, &hash)?;
        replace_stable_link(&paths.bin, &paths.stable, &paths.relative_target)?;
        validate_version(&paths.stable)
            .context("The activated stable helper failed its version check")?;

        let migrated = crate::pairing::migrate_phone_entries(&paths.stable)?;
        fs2::FileExt::unlock(&lock)?;
        Ok(Activation {
            stable: paths.stable,
            target: paths.relative_target,
            hash,
            migrated,
        })
    }
}

#[cfg(unix)]
fn create_private_directories(bin: &Path) -> Result<()> {
    let desktop = bin.parent().context("The helper bin path has no parent")?;
    let pi = desktop
        .parent()
        .context("The helper desktop path has no parent")?;
    for directory in [pi, desktop, bin] {
        ensure_private_directory(directory)?;
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.file_type().is_dir(),
        "Helper installation path is not a directory: {}",
        path.display()
    );
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(unix)]
fn open_lock(path: &Path) -> Result<File> {
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    ensure!(
        lock.metadata()?.is_file(),
        "The activation lock is not a regular file"
    );
    lock.set_permissions(fs::Permissions::from_mode(0o600))?;
    Ok(lock)
}

#[cfg(unix)]
fn install_content_object(source: &mut File, object: &Path, hash: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    match fs::symlink_metadata(object) {
        Ok(metadata) => {
            ensure!(
                metadata.file_type().is_file(),
                "Existing content object {} is not a regular file",
                object.display()
            );
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    source.seek(SeekFrom::Start(0))?;
    let parent = object.parent().context("Content object has no parent")?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".pi-desktop-remote.")
        .suffix(".partial")
        .tempfile_in(parent)?;
    let copied_hash = copy_and_hash(source, temporary.as_file_mut())?;
    ensure!(
        copied_hash == hash,
        "The running helper changed while being copied"
    );
    temporary.flush()?;
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700))?;
    temporary.as_file().sync_all()?;
    ensure!(
        digest_path(temporary.path())? == hash,
        "The copied helper failed checksum verification"
    );
    match temporary.persist_noclobber(object) {
        Ok(_) => sync_directory(parent)?,
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            drop(error.file);
        }
        Err(error) => return Err(error.error.into()),
    }
    Ok(())
}

#[cfg(unix)]
fn validate_content_object(path: &Path, hash: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.file_type().is_file(),
        "Existing content object {} is not a regular file",
        path.display()
    );
    ensure!(
        digest_path(path)? == hash,
        "Existing content object {} has the wrong checksum",
        path.display()
    );
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    validate_version(path).context("Existing content object has the wrong version")?;
    Ok(())
}

#[cfg(unix)]
fn validate_version(path: &Path) -> Result<()> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .with_context(|| format!("Could not run {} --version", path.display()))?;
    ensure!(output.status.success(), "Helper --version failed");
    let stdout = String::from_utf8(output.stdout).context("Helper --version was not UTF-8")?;
    let line = stdout
        .strip_suffix('\n')
        .and_then(|line| line.strip_suffix('\r').or(Some(line)))
        .context("Helper --version did not print one line")?;
    let expected = format!(
        "pi-desktop-remote {VERSION} {PROTOCOL_VERSION} {}",
        crate::platform()
    );
    ensure!(
        !line.contains(['\r', '\n']) && line == expected,
        "Helper version mismatch: {line}"
    );
    Ok(())
}

#[cfg(unix)]
fn digest_path(path: &Path) -> Result<String> {
    use std::os::unix::fs::OpenOptionsExt as _;

    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    digest_file(&mut file)
}

#[cfg(unix)]
fn digest_file(file: &mut File) -> Result<String> {
    file.seek(SeekFrom::Start(0))?;
    hash_reader(file, None)
}

#[cfg(unix)]
fn copy_and_hash(source: &mut File, destination: &mut File) -> Result<String> {
    hash_reader(source, Some(destination))
}

#[cfg(unix)]
fn hash_reader(reader: &mut File, mut destination: Option<&mut File>) -> Result<String> {
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .context("The helper size overflowed")?;
        ensure!(total <= MAX_HELPER_SIZE, "The helper is too large");
        if let Some(destination) = destination.as_deref_mut() {
            destination.write_all(&buffer[..count])?;
        }
        hash.update(&buffer[..count]);
    }
    ensure!(total > 0, "The helper is empty");
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(unix)]
fn validate_native(file: &mut File) -> Result<()> {
    file.seek(SeekFrom::Start(0))?;
    let mut header = [0; 64];
    file.read_exact(&mut header)
        .context("The running helper is too small to be a native executable")?;
    let valid = match (env::consts::OS, env::consts::ARCH) {
        ("linux", "x86_64") => {
            header[..6] == *b"\x7fELF\x02\x01" && u16::from_le_bytes([header[18], header[19]]) == 62
        }
        ("linux", "aarch64") => {
            header[..6] == *b"\x7fELF\x02\x01"
                && u16::from_le_bytes([header[18], header[19]]) == 183
        }
        ("macos", "aarch64") => header[..8] == [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1],
        _ => false,
    };
    ensure!(
        valid,
        "The running helper is not a native executable for this platform"
    );
    Ok(())
}

#[cfg(unix)]
fn replace_stable_link(bin: &Path, stable: &Path, target: &Path) -> Result<()> {
    use std::os::unix::fs::symlink;

    match fs::symlink_metadata(stable) {
        Ok(metadata) if metadata.file_type().is_dir() => {
            bail!("The stable helper path {} is a directory", stable.display())
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if fs::read_link(stable).ok().as_deref() == Some(target) {
        return Ok(());
    }

    let temporary = (0..128)
        .find_map(|_| {
            let candidate = bin.join(format!(
                ".{HELPER_NAME}.{:016x}.partial",
                rand::random::<u64>()
            ));
            match symlink(target, &candidate) {
                Ok(()) => Some(Ok(candidate)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(error)),
            }
        })
        .transpose()?
        .context("Could not allocate a temporary activation link")?;
    if let Err(error) = fs::rename(&temporary, stable) {
        fs::remove_file(&temporary).ok();
        return Err(error).context("Could not atomically activate the stable helper path");
    }
    sync_directory(bin)?;
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn self_install_paths_are_derived_from_home_and_sha256() {
        let home = Path::new("/home/alice");
        let hash = "a".repeat(64);
        let paths = install_paths(home, &hash).unwrap();
        assert_eq!(
            paths.immutable,
            home.join(format!(".pi/desktop/bin/{hash}/{HELPER_NAME}"))
        );
        assert_eq!(paths.stable, home.join(".pi/desktop/bin/pi-desktop-remote"));
        assert_eq!(
            paths.relative_target,
            PathBuf::from(format!("{hash}/{HELPER_NAME}"))
        );
        assert!(install_paths(Path::new("relative"), &hash).is_err());
    }

    #[test]
    fn stable_switch_selects_the_requested_relative_target() {
        let root = tempfile::tempdir().unwrap();
        let stable = root.path().join(HELPER_NAME);
        let target = PathBuf::from("a".repeat(64)).join(HELPER_NAME);
        fs::write(&stable, b"old").unwrap();
        let mut running = File::open(&stable).unwrap();

        replace_stable_link(root.path(), &stable, &target).unwrap();
        assert_eq!(fs::read_link(&stable).unwrap(), target);
        let mut old = Vec::new();
        running.read_to_end(&mut old).unwrap();
        assert_eq!(old, b"old", "an already-open helper remains usable");

        fs::remove_file(&stable).unwrap();
        symlink("missing/pi-desktop-remote", &stable).unwrap();
        replace_stable_link(root.path(), &stable, &target).unwrap();
        assert_eq!(fs::read_link(&stable).unwrap(), target);
    }

    #[test]
    fn stable_switch_is_idempotent_and_refuses_directories() {
        let root = tempfile::tempdir().unwrap();
        let stable = root.path().join(HELPER_NAME);
        let target = PathBuf::from("a".repeat(64)).join(HELPER_NAME);
        symlink(&target, &stable).unwrap();
        let before = fs::symlink_metadata(&stable).unwrap();
        replace_stable_link(root.path(), &stable, &target).unwrap();
        let after = fs::symlink_metadata(&stable).unwrap();
        use std::os::unix::fs::MetadataExt as _;
        assert_eq!(before.ino(), after.ino());

        fs::remove_file(&stable).unwrap();
        fs::create_dir(&stable).unwrap();
        assert!(replace_stable_link(root.path(), &stable, &target).is_err());
    }

    #[test]
    fn content_objects_and_locks_never_follow_symlinks_or_keep_partial_copies() {
        let root = tempfile::tempdir().unwrap();
        let victim = root.path().join("victim");
        fs::write(&victim, b"keep").unwrap();

        let object = root.path().join(HELPER_NAME);
        symlink(&victim, &object).unwrap();
        let mut source = File::open(&victim).unwrap();
        assert!(install_content_object(&mut source, &object, &"a".repeat(64)).is_err());
        assert_eq!(fs::read(&victim).unwrap(), b"keep");

        fs::remove_file(&object).unwrap();
        assert!(install_content_object(&mut source, &object, &"a".repeat(64)).is_err());
        assert!(!object.exists());
        assert!(fs::read_dir(root.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".partial")
        }));

        let lock = root.path().join("activation.lock");
        symlink(&victim, &lock).unwrap();
        assert!(open_lock(&lock).is_err());
        assert_eq!(fs::read(&victim).unwrap(), b"keep");
    }
}
