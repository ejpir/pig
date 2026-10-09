#![cfg(unix)]

use sha2::{Digest as _, Sha256};
use ssh_key::{HashAlg, PrivateKey, private::Ed25519Keypair};
use std::{
    fs,
    os::unix::fs::{MetadataExt as _, PermissionsExt as _},
    path::Path,
    process::Command,
};

fn digest(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}

fn run_activate(home: &Path, keys: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pi-desktop-remote"))
        .arg("activate")
        .env("HOME", home)
        .env("PI_DESKTOP_AUTHORIZED_KEYS_FILE", keys)
        .output()
        .unwrap()
}

#[test]
fn activation_self_installs_migrates_once_and_is_idempotent() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    let ssh = home.join(".ssh");
    fs::create_dir_all(&ssh).unwrap();
    let keys = ssh.join("authorized_keys");
    let mut private = PrivateKey::from(Ed25519Keypair::from_seed(&[11; 32]));
    let fingerprint = private
        .public_key()
        .fingerprint(HashAlg::Sha256)
        .to_string();
    let short: String = fingerprint
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(16)
        .collect();
    let marker = format!("pi-phone:{short}");
    private.set_comment(marker.as_str());
    let managed = format!(
        "restrict,command=\"exec /old/hash/pi-desktop-remote gateway {marker}\" {}\r\n",
        private.public_key().to_openssh().unwrap()
    );
    let mut original = b"# unrelated \xff bytes\r\n".to_vec();
    original.extend_from_slice(managed.as_bytes());
    fs::write(&keys, &original).unwrap();

    let first = run_activate(&home, &keys);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let stdout = String::from_utf8(first.stdout).unwrap();
    assert!(stdout.contains("migrated 1 paired phone key"));

    let source = Path::new(env!("CARGO_BIN_EXE_pi-desktop-remote"));
    let hash = digest(source);
    let bin = home.join(".pi/desktop/bin");
    let stable = bin.join("pi-desktop-remote");
    let relative = Path::new(&hash).join("pi-desktop-remote");
    assert_eq!(fs::read_link(&stable).unwrap(), relative);
    let object = bin.join(&hash).join("pi-desktop-remote");
    assert_eq!(fs::read(&object).unwrap(), fs::read(source).unwrap());
    assert_eq!(
        fs::metadata(&object).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for path in [
        home.join(".pi"),
        home.join(".pi/desktop"),
        bin.clone(),
        bin.join(&hash),
    ] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    assert_eq!(
        fs::metadata(bin.join(".pi-desktop-remote.activation.lock"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(&ssh).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&keys).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let migrated = fs::read(&keys).unwrap();
    assert!(migrated.starts_with(b"# unrelated \xff bytes\r\n"));
    assert!(migrated.ends_with(b"\r\n"));
    let text = String::from_utf8_lossy(&migrated);
    assert!(!text.contains("/old/hash/pi-desktop-remote"));
    assert!(text.contains(&format!("{} gateway {marker}", stable.display())));

    let link_inode = fs::symlink_metadata(&stable).unwrap().ino();
    let second = run_activate(&home, &keys);
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(
        String::from_utf8(second.stdout)
            .unwrap()
            .contains("migrated 0 paired phone keys")
    );
    assert_eq!(fs::read(&keys).unwrap(), migrated);
    assert_eq!(fs::symlink_metadata(&stable).unwrap().ino(), link_inode);
}

#[test]
fn activation_never_overwrites_an_existing_hash_object() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    let keys = directory.path().join("authorized_keys");
    let source = Path::new(env!("CARGO_BIN_EXE_pi-desktop-remote"));
    let hash = digest(source);
    let object = home
        .join(".pi/desktop/bin")
        .join(&hash)
        .join("pi-desktop-remote");
    fs::create_dir_all(object.parent().unwrap()).unwrap();
    fs::write(&object, b"corrupt but immutable").unwrap();

    let output = run_activate(&home, &keys);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("wrong checksum"));
    assert_eq!(fs::read(&object).unwrap(), b"corrupt but immutable");
}
