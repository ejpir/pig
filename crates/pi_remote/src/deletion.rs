//! Permanent deletion of one explicitly confirmed durable session. The daemon
//! holds its stable session lock throughout shutdown and deletion. Project
//! files, credentials and other sessions are never deletion targets.
use anyhow::{Context, Result, ensure};
use pi_core::ssh::{RemoteBackend, SshTarget};
use serde_json::{Value, json};
use std::{fs, path::Path};

pub(crate) fn validate(target: &SshTarget, run: &Path, storage: &Path) -> Result<()> {
    target.validate()?;
    ensure!(
        target.backend == RemoteBackend::Durable,
        "Permanent deletion is supported for durable sessions. Remove legacy Pi history on the computer."
    );
    ensure!(
        storage.file_name().and_then(|name| name.to_str()) == Some(&target.key),
        "Session storage key mismatch"
    );
    ensure!(
        !fs::symlink_metadata(storage)?.file_type().is_symlink(),
        "Refusing linked session storage"
    );
    let parent = storage.parent().context("Missing session storage parent")?;
    ensure!(
        !fs::symlink_metadata(parent)?.file_type().is_symlink(),
        "Refusing linked storage parent"
    );
    let identity: Value = serde_json::from_slice(&fs::read(
        run.join(format!("{}.identity.json", target.key)),
    )?)?;
    ensure!(
        identity == json!({"cwd":target.cwd,"backend":target.backend}),
        "Remote session identity mismatch"
    );
    let identity_path = storage.join("identity.json");
    ensure!(
        !fs::symlink_metadata(&identity_path)?
            .file_type()
            .is_symlink(),
        "Refusing linked session identity"
    );
    let identity: Value = serde_json::from_slice(&fs::read(identity_path)?)?;
    ensure!(
        identity == json!({"version":1,"key":target.key,"cwd":fs::canonicalize(&target.cwd)?}),
        "Durable storage identity mismatch"
    );
    Ok(())
}

/// Call only after the backend and its process tree have stopped, with the
/// daemon's stable lock still held. Keep a tombstone so stale clients cannot
/// recreate a deleted conversation using its old identity.
pub(crate) fn remove(target: &SshTarget, run: &Path, storage: &Path) -> Result<()> {
    validate(target, run, storage)?;
    let owner_path = storage.join("owner.lock");
    ensure!(
        !fs::symlink_metadata(&owner_path)?.file_type().is_symlink(),
        "Refusing linked writer lock"
    );
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let owner = options.open(owner_path)?;
    owner
        .try_lock()
        .context("The session still has a writer; no history was deleted")?;
    let tombstone = run.join(format!("{}.deleted.json", target.key));
    let file = crate::server::private_file(&tombstone)?;
    serde_json::to_writer(&file, &json!({"key":target.key,"deleted":true}))?;
    file.sync_all()?;
    fs::remove_dir_all(storage).context("Could not finish removing the session's storage")?;
    for suffix in ["identity.json", "summary.json", "summary.partial", "log"] {
        let path = run.join(format!("{}.{suffix}", target.key));
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    // The daemon lock and tombstone intentionally remain. They contain no transcript.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(root: &Path) -> (SshTarget, std::path::PathBuf, std::path::PathBuf) {
        let mut target = SshTarget::new("test".into(), root.to_string_lossy().into()).unwrap();
        target.backend = RemoteBackend::Durable;
        let run = root.join("run");
        let storage = root.join("durable").join(&target.key);
        fs::create_dir_all(&run).unwrap();
        fs::create_dir_all(&storage).unwrap();
        fs::write(
            run.join(format!("{}.identity.json", target.key)),
            json!({"cwd":target.cwd,"backend":"durable"}).to_string(),
        )
        .unwrap();
        fs::write(
            storage.join("identity.json"),
            json!({"version":1,"key":target.key,"cwd":fs::canonicalize(root).unwrap()}).to_string(),
        )
        .unwrap();
        fs::write(storage.join("owner.lock"), "").unwrap();
        fs::write(storage.join("session.sqlite"), "disposable test history").unwrap();
        (target, run, storage)
    }

    #[test]
    fn deletion_removes_only_the_named_history_and_prevents_recreation() {
        let root = tempfile::tempdir().unwrap();
        let (target, run, storage) = fixture(root.path());
        let other = root.path().join("durable/other");
        fs::create_dir(&other).unwrap();
        fs::write(root.path().join("project.txt"), "keep project edits").unwrap();
        remove(&target, &run, &storage).unwrap();
        assert!(!storage.exists());
        assert!(other.exists());
        assert_eq!(
            fs::read_to_string(root.path().join("project.txt")).unwrap(),
            "keep project edits"
        );
        assert!(run.join(format!("{}.deleted.json", target.key)).exists());
        assert!(
            crate::sessions::list(&run).unwrap()["sessions"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn an_active_writer_or_wrong_identity_blocks_deletion() {
        let root = tempfile::tempdir().unwrap();
        let (mut target, run, storage) = fixture(root.path());
        let owner = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(storage.join("owner.lock"))
            .unwrap();
        owner.lock().unwrap();
        assert!(
            remove(&target, &run, &storage)
                .unwrap_err()
                .to_string()
                .contains("writer")
        );
        assert!(storage.join("session.sqlite").exists());
        drop(owner);
        target.key = "../../outside".into();
        assert!(remove(&target, &run, &storage).is_err());
        assert!(storage.join("session.sqlite").exists());
    }

    #[cfg(unix)]
    #[test]
    fn linked_storage_is_never_followed() {
        let root = tempfile::tempdir().unwrap();
        let (target, run, storage) = fixture(root.path());
        let real = storage.with_extension("original");
        fs::rename(&storage, &real).unwrap();
        std::os::unix::fs::symlink(&real, &storage).unwrap();
        assert!(remove(&target, &run, &storage).is_err());
        assert!(real.join("session.sqlite").exists());
    }
}
