//! The standalone pi-desktop-backend that release builds carry inside the
//! executable (the `bundled-backend` feature). It runs from disk, so the first
//! launch of each build unpacks it into the cache folder; later launches find it
//! there. Sessions run it unless Settings or an environment variable name
//! another backend.

// Without the feature only the tests unpack anything.
#![cfg_attr(not(feature = "bundled-backend"), allow(dead_code))]

use anyhow::{Context as _, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const PROGRAM: &str = if cfg!(windows) {
    "pi-desktop-backend.exe"
} else {
    "pi-desktop-backend"
};

/// The unpacked backend's executable, or `None` in builds without one or when
/// unpacking failed (sessions then run `pi` from PATH, as before).
pub fn unpack() -> Option<PathBuf> {
    #[cfg(feature = "bundled-backend")]
    {
        static ARCHIVE: &[u8] = include_bytes!(env!("PI_DESKTOP_BACKEND_ARCHIVE"));
        let root = dirs::cache_dir()?.join("pi-desktop").join("backend");
        let started = std::time::Instant::now();
        match unpack_into(&root, env!("PI_DESKTOP_BACKEND_ID"), ARCHIVE) {
            Ok(program) => {
                log::info!(
                    "bundled backend {} ({:?})",
                    program.display(),
                    started.elapsed()
                );
                return Some(program);
            }
            Err(error) => log::error!("could not unpack the bundled backend: {error:#}"),
        }
    }
    None
}

/// Unpacks `archive` (a zstd-compressed tar) into `root/id` unless it is already
/// there, and removes other builds' folders. Unpacking goes to a temporary
/// folder that is renamed into place, so a crash never leaves half a backend.
pub(crate) fn unpack_into(root: &Path, id: &str, archive: &[u8]) -> Result<PathBuf> {
    let folder = root.join(id);
    let program = folder.join(PROGRAM);
    if !program.is_file() {
        fs::create_dir_all(root).with_context(|| format!("creating {}", root.display()))?;
        let partial = root.join(format!(".{id}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&partial);
        let unpacked = zstd::Decoder::new(archive)
            .and_then(|decoder| tar::Archive::new(decoder).unpack(&partial));
        if let Err(error) = unpacked {
            let _ = fs::remove_dir_all(&partial);
            return Err(error).with_context(|| format!("unpacking into {}", partial.display()));
        }
        // A folder without the program was damaged, for example by a cleaner.
        let _ = fs::remove_dir_all(&folder);
        if let Err(error) = fs::rename(&partial, &folder) {
            let _ = fs::remove_dir_all(&partial);
            // Another window of the same build may have won the race.
            if !program.is_file() {
                return Err(error).with_context(|| format!("moving into {}", folder.display()));
            }
        }
        anyhow::ensure!(program.is_file(), "{} has no {PROGRAM}", folder.display());
    }
    remove_others(root, id);
    Ok(program)
}

/// Removes other builds' folders, and temporary folders left by a crash.
/// Removal can fail while an older build still runs (Windows locks running
/// programs); the next launch tries again.
fn remove_others(root: &Path, id: &str) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let hour_ago = SystemTime::now() - Duration::from_secs(3600);
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let stale = if name.starts_with('.') {
            // Another launch may be unpacking right now.
            entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .is_ok_and(|modified| modified < hour_ago)
        } else {
            name != id
        };
        if stale {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, bytes, mode) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(*mode);
            header.set_cksum();
            builder.append_data(&mut header, path, *bytes).unwrap();
        }
        zstd::encode_all(&builder.into_inner().unwrap()[..], 3).unwrap()
    }

    #[test]
    fn unpacks_once_per_build_and_removes_other_builds() {
        let root = tempfile::tempdir().unwrap();
        let first = archive(&[(PROGRAM, b"one", 0o755), ("theme/dark.json", b"{}", 0o644)]);
        let program = unpack_into(root.path(), "a", &first).unwrap();
        assert_eq!(program, root.path().join("a").join(PROGRAM));
        assert_eq!(fs::read(&program).unwrap(), b"one");
        assert_eq!(
            fs::read(root.path().join("a/theme/dark.json")).unwrap(),
            b"{}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&program).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "the program stays executable");
        }

        // Already there: nothing is unpacked again.
        fs::write(&program, b"kept").unwrap();
        unpack_into(root.path(), "a", &first).unwrap();
        assert_eq!(fs::read(&program).unwrap(), b"kept");

        // A folder that lost its program is unpacked again.
        fs::remove_file(&program).unwrap();
        unpack_into(root.path(), "a", &first).unwrap();
        assert_eq!(fs::read(&program).unwrap(), b"one");

        // A new build gets its own folder; the old one and fresh partials of
        // other launches are left alone only while they may be in use.
        fs::create_dir(root.path().join(".b-999")).unwrap();
        let second = archive(&[(PROGRAM, b"two", 0o755)]);
        let program = unpack_into(root.path(), "b", &second).unwrap();
        assert_eq!(fs::read(&program).unwrap(), b"two");
        let mut left: Vec<_> = fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left, [".b-999", "b"]);
    }

    #[test]
    fn a_damaged_archive_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        assert!(unpack_into(root.path(), "a", b"not zstd").is_err());
        let no_program = archive(&[("README.md", b"hi", 0o644)]);
        assert!(unpack_into(root.path(), "a", &no_program).is_err());
        let partials = fs::read_dir(root.path())
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with('.')
            })
            .count();
        assert_eq!(partials, 0, "a failed unpack cleans up after itself");
    }
}
