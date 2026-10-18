//! Bounded text-file service over SSH stdio, independent of Pi and its daemon.
#[cfg(unix)]
#[path = "files_unix.rs"]
mod unix_backend;

#[cfg(windows)]
use anyhow::bail;
use anyhow::{Context, Result, ensure};
#[cfg(any(test, windows))]
use pi_core::remote_files::MAX_FILE_BYTES;
#[cfg(windows)]
use pi_core::remote_files::{Entry, MAX_TREE_ENTRIES};
use pi_core::{
    protocol::{MAX_RECORD_BYTES, read_record},
    remote_files::{Document, FILE_PROTOCOL_VERSION, RecoveryArtifact, Request, Tree, response},
};
use serde_json::{Value, json};
#[cfg(windows)]
use sha2::{Digest, Sha256};
#[cfg(any(test, windows))]
use std::fs::OpenOptions;
use std::{
    fs,
    io::{BufReader, Write},
    path::Path,
};
#[cfg(windows)]
use std::{
    io::Read,
    path::{Component, PathBuf},
};

#[cfg(windows)]
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// The optimistic revision includes Unix ownership/mode as well as content, so
/// a chmod or ownership change at the save boundary is a conflict rather than
/// metadata that an otherwise-identical write silently undoes.
#[cfg(windows)]
fn document_revision(bytes: &[u8], metadata: &fs::Metadata) -> String {
    let mut digest = Sha256::new();
    digest.update(b"pi-remote-file-v2\0");
    digest.update(bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        digest.update(metadata.mode().to_le_bytes());
        digest.update(metadata.uid().to_le_bytes());
        digest.update(metadata.gid().to_le_bytes());
    }
    #[cfg(not(unix))]
    digest.update([u8::from(metadata.permissions().readonly())]);
    format!("{:x}", digest.finalize())
}

#[cfg(windows)]
fn preserve_metadata(file: &fs::File, metadata: &fs::Metadata) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::{fd::AsRawFd as _, unix::fs::MetadataExt as _};
        let status = unsafe { libc::fchown(file.as_raw_fd(), metadata.uid(), metadata.gid()) };
        if status != 0 {
            return Err(std::io::Error::last_os_error())
                .context("Could not preserve remote file ownership");
        }
    }
    // chown may clear set-id bits, so permissions are restored afterward.
    file.set_permissions(metadata.permissions())
        .context("Could not preserve remote file permissions")
}

#[cfg(any(test, windows))]
fn metadata_matches(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        left.mode() == right.mode() && left.uid() == right.uid() && left.gid() == right.gid()
    }
    #[cfg(not(unix))]
    {
        left.permissions().readonly() == right.permissions().readonly()
    }
}

#[cfg(windows)]
fn install_with_backup(path: &Path, staging: &Path, recovery: &Path) -> Result<()> {
    use std::{iter, os::windows::ffi::OsStrExt};
    use windows_sys::Win32::Storage::FileSystem::{REPLACEFILE_WRITE_THROUGH, ReplaceFileW};
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(iter::once(0))
            .collect::<Vec<_>>()
    };
    let (path, staging, recovery) = (wide(path), wide(staging), wide(recovery));
    let status = unsafe {
        ReplaceFileW(
            path.as_ptr(),
            staging.as_ptr(),
            recovery.as_ptr(),
            REPLACEFILE_WRITE_THROUGH,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if status != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error()).context("Atomic file replacement failed")
    }
}

#[cfg(windows)]
fn roll_back_installed(path: &Path, recovery: &Path) -> Result<()> {
    use std::{iter, os::windows::ffi::OsStrExt};
    use windows_sys::Win32::Storage::FileSystem::{REPLACEFILE_WRITE_THROUGH, ReplaceFileW};
    let parent = path.parent().context("File has no parent")?;
    let displaced_candidate = tempfile::Builder::new()
        .prefix(".pi-save-rollback-")
        .tempfile_in(parent)?;
    let displaced_candidate_path = displaced_candidate.path().to_owned();
    drop(displaced_candidate);
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(iter::once(0))
            .collect::<Vec<_>>()
    };
    let (path, recovery, candidate) = (wide(path), wide(recovery), wide(&displaced_candidate_path));
    let status = unsafe {
        ReplaceFileW(
            path.as_ptr(),
            recovery.as_ptr(),
            candidate.as_ptr(),
            REPLACEFILE_WRITE_THROUGH,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if status == 0 {
        return Err(std::io::Error::last_os_error()).context("Atomic save rollback failed");
    }
    // ReplaceFile captured whichever inode occupied the pathname. Keep it: a
    // third writer may have arrived between verification and rollback.
    Ok(())
}

#[cfg(windows)]
fn sync_parent(_: &Path) -> Result<()> {
    Ok(())
}

struct Files {
    #[cfg(windows)]
    root: PathBuf,
    #[cfg(unix)]
    unix: unix_backend::UnixFiles,
}
impl Files {
    fn new(root: &str) -> Result<Self> {
        let root = fs::canonicalize(root).context("Remote project must exist")?;
        ensure!(root.is_dir(), "Remote project must be a directory");
        #[cfg(unix)]
        let unix = unix_backend::UnixFiles::new(root.clone())?;
        Ok(Self {
            #[cfg(windows)]
            root,
            #[cfg(unix)]
            unix,
        })
    }
    // The Windows backend retains lexical validation until it gains a native
    // handle-relative no-reparse implementation.
    #[cfg(windows)]
    fn resolve(&self, name: &str) -> Result<(PathBuf, String)> {
        ensure!(
            !name.is_empty() && !name.contains(['\0', '\n', '\r']),
            "Invalid remote file path"
        );
        let path = Path::new(name);
        let relative = if path.is_absolute() {
            path.strip_prefix(&self.root)
                .context("File is outside the remote project")?
        } else {
            path
        };
        ensure!(
            !relative.as_os_str().is_empty(),
            "Choose a file inside the remote project"
        );
        let mut target = self.root.clone();
        let mut parts = Vec::new();
        for part in relative.components() {
            let Component::Normal(part) = part else {
                bail!("Parent traversal is not allowed")
            };
            let part = part.to_str().context("Non-UTF-8 paths are not supported")?;
            ensure!(!part.contains(['\\', ':']), "Unsupported path component");
            target.push(part);
            ensure!(
                !fs::symlink_metadata(&target)?.file_type().is_symlink(),
                "Symlink paths are not supported in the remote editor"
            );
            parts.push(part);
        }
        ensure!(
            fs::canonicalize(&target)?.starts_with(&self.root),
            "File is outside the remote project"
        );
        Ok((target, parts.join("/")))
    }
    fn read(&self, name: &str) -> Result<Document> {
        #[cfg(unix)]
        return self.unix.read(name);
        #[cfg(windows)]
        return self.read_windows(name);
    }

    #[cfg(windows)]
    fn read_windows(&self, name: &str) -> Result<Document> {
        let (path, relative) = self.resolve(name)?;
        let mut file = OpenOptions::new().read(true).open(&path)?;
        let metadata = file.metadata()?;
        ensure!(metadata.is_file(), "Not a regular file");
        ensure!(
            metadata.len() <= MAX_FILE_BYTES,
            "Remote text files over 1 MiB are not supported yet"
        );
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_FILE_BYTES,
            "Remote file grew beyond 1 MiB"
        );
        ensure!(
            !bytes.contains(&0),
            "Binary files are not supported in the text editor"
        );
        let revision = document_revision(&bytes, &metadata);
        let text = String::from_utf8(bytes).context("Remote editor requires UTF-8 text")?;
        Ok(Document {
            path: relative,
            text,
            revision,
        })
    }
    fn list(&self) -> Result<Tree> {
        #[cfg(unix)]
        return self.unix.list();
        #[cfg(windows)]
        return self.list_windows();
    }

    #[cfg(windows)]
    fn list_windows(&self) -> Result<Tree> {
        let mut tree = Tree {
            entries: Vec::new(),
            truncated: false,
        };
        let mut pending = vec![(self.root.clone(), String::new())];
        let mut budget = 0;
        while let Some((directory, prefix)) = pending.pop() {
            for child in fs::read_dir(directory)? {
                let child = child?;
                let Some(name) = child.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                if matches!(
                    name.as_str(),
                    ".git" | ".jj" | ".hg" | "node_modules" | "target"
                ) || name.contains(['\\', ':', '\n', '\r'])
                {
                    continue;
                }
                let kind = child.file_type()?;
                if kind.is_symlink() || !(kind.is_file() || kind.is_dir()) {
                    continue;
                }
                let relative = if prefix.is_empty() {
                    name
                } else {
                    format!("{prefix}/{name}")
                };
                budget += relative.len() * 6 + 128;
                if tree.entries.len() == MAX_TREE_ENTRIES || budget > MAX_RECORD_BYTES / 2 {
                    tree.truncated = true;
                    return Ok(tree);
                }
                if kind.is_dir() {
                    pending.push((child.path(), relative.clone()));
                }
                tree.entries.push(Entry {
                    path: relative,
                    directory: kind.is_dir(),
                    size: kind
                        .is_file()
                        .then(|| child.metadata().ok().map(|metadata| metadata.len()))
                        .flatten(),
                });
            }
        }
        tree.entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(tree)
    }
    fn save(&self, name: &str, text: &str, revision: &str) -> Result<Document> {
        self.save_with(name, text, revision, |_| Ok(()))
    }

    fn save_with(
        &self,
        name: &str,
        text: &str,
        revision: &str,
        before_install: impl FnOnce(&Path) -> Result<()>,
    ) -> Result<Document> {
        self.save_with_hooks(name, text, revision, before_install, |_| Ok(()))
    }

    fn save_with_hooks(
        &self,
        name: &str,
        text: &str,
        revision: &str,
        before_install: impl FnOnce(&Path) -> Result<()>,
        after_install: impl FnOnce(&Path) -> Result<()>,
    ) -> Result<Document> {
        #[cfg(unix)]
        return self.unix.save(
            name,
            text,
            revision,
            before_install,
            |_| Ok(()),
            after_install,
        );
        #[cfg(windows)]
        return self.save_with_hooks_windows(name, text, revision, before_install, after_install);
    }

    #[cfg(all(test, unix))]
    fn save_with_exchange_hook(
        &self,
        name: &str,
        text: &str,
        revision: &str,
        before_exchange: impl FnOnce(&Path) -> Result<()>,
    ) -> Result<Document> {
        self.unix.save(
            name,
            text,
            revision,
            |_| Ok(()),
            before_exchange,
            |_| Ok(()),
        )
    }

    #[cfg(windows)]
    fn save_with_hooks_windows(
        &self,
        name: &str,
        text: &str,
        revision: &str,
        before_install: impl FnOnce(&Path) -> Result<()>,
        after_install: impl FnOnce(&Path) -> Result<()>,
    ) -> Result<Document> {
        ensure!(
            text.len() as u64 <= MAX_FILE_BYTES && !text.contains('\0'),
            "Remote saves require UTF-8 text under 1 MiB"
        );
        let (path, relative) = self.resolve(name)?;
        // Coordinate simultaneous desktop writers across sessions/connections.
        let locks = dirs::cache_dir()
            .context("No remote cache directory")?
            .join("pi-desktop-remote/file-locks");
        fs::create_dir_all(&locks)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&locks, fs::Permissions::from_mode(0o700))?;
        }
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(locks.join(hash(path.to_string_lossy().as_bytes())))?;
        lock.try_lock()
            .context("Another desktop is saving this file; reload before retrying")?;
        ensure!(
            self.read(name)?.revision == revision,
            "File changed on the remote host. Reload or copy your edits before saving"
        );
        let parent = path.parent().context("File has no parent")?;
        let initial_metadata = fs::metadata(&path)?;
        ensure!(
            !initial_metadata.permissions().readonly(),
            "Remote file is read-only"
        );

        // Keep two independently synced copies before touching the live path:
        // one to install and one to retain if a writer replaces the installed
        // inode before verification. They must not be hard links, because an
        // in-place writer could otherwise alter the recovery copy too.
        let mut staging = tempfile::Builder::new()
            .prefix(".pi-save-recovery-")
            .tempfile_in(parent)?;
        staging.write_all(text.as_bytes())?;
        preserve_metadata(staging.as_file(), &initial_metadata)?;
        staging.as_file().sync_all()?;
        let mut candidate = tempfile::Builder::new()
            .prefix(".pi-save-candidate-")
            .tempfile_in(parent)?;
        candidate.write_all(text.as_bytes())?;
        preserve_metadata(candidate.as_file(), &initial_metadata)?;
        candidate.as_file().sync_all()?;

        before_install(&path)?;
        let (staging_file, staging_path) = staging
            .keep()
            .map_err(|error| error.error)
            .context("Could not retain the staged save")?;
        drop(staging_file);
        let (candidate_file, candidate_path) = candidate
            .keep()
            .map_err(|error| error.error)
            .context("Could not retain the save candidate")?;
        drop(candidate_file);
        #[cfg(unix)]
        let recovery_path = staging_path.clone();
        #[cfg(windows)]
        let recovery_path = {
            let recovery = tempfile::Builder::new()
                .prefix(".pi-save-recovery-")
                .tempfile_in(parent)?;
            let path = recovery.path().to_owned();
            drop(recovery);
            path
        };

        // Linux/macOS exchange the two names; Windows ReplaceFile writes an
        // explicit backup. In every case the live pathname remains present at
        // the atomic boundary and the exact displaced inode is recoverable.
        if let Err(error) = install_with_backup(&path, &staging_path, &recovery_path) {
            let _ = fs::remove_file(&staging_path);
            let _ = fs::remove_file(&candidate_path);
            let _ = fs::remove_file(&recovery_path);
            return Err(error);
        }
        if let Err(error) = after_install(&path) {
            return match roll_back_installed(&path, &recovery_path) {
                Ok(()) => Err(error).context(format!(
                    "The displaced file was restored; the save candidate is preserved at {}",
                    candidate_path.display()
                )),
                Err(rollback) => Err(rollback).context(format!(
                    "The installed save could not be rolled back after: {error}. The displaced file is preserved at {} and the save candidate at {}",
                    recovery_path.display(),
                    candidate_path.display()
                )),
            };
        }

        let displaced = fs::read(&recovery_path)
            .context("Could not verify the file displaced at the atomic save boundary")?;
        let displaced_metadata = fs::metadata(&recovery_path)?;
        if document_revision(&displaced, &displaced_metadata) != revision
            || displaced_metadata.permissions().readonly()
        {
            return match roll_back_installed(&path, &recovery_path) {
                Ok(()) => bail!(
                    "File content or permissions changed on the remote host while saving. The newer remote revision was restored and the save candidate is preserved at {}. Your phone buffer has been kept",
                    candidate_path.display()
                ),
                Err(error) => Err(error).context(format!(
                    "The displaced revision is preserved at {} and the save candidate at {}",
                    recovery_path.display(),
                    candidate_path.display()
                )),
            };
        }
        // Catch writes through a descriptor opened before the exchange, and
        // catch a second atomic writer that replaced the candidate. A second
        // writer wins the pathname, while both our candidate and the displaced
        // revision remain as explicit recovery files.
        let displaced_after = fs::read(&recovery_path)?;
        let displaced_metadata_after = fs::metadata(&recovery_path)?;
        if document_revision(&displaced_after, &displaced_metadata_after) != revision {
            return match roll_back_installed(&path, &recovery_path) {
                Ok(()) => bail!(
                    "File changed through an open editor while saving. The newer remote revision was restored and the save candidate is preserved at {}. Your phone buffer has been kept",
                    candidate_path.display()
                ),
                Err(error) => Err(error).context(format!(
                    "The displaced revision is preserved at {} and the save candidate at {}",
                    recovery_path.display(),
                    candidate_path.display()
                )),
            };
        }
        let installed = fs::read(&path).context(format!(
            "The installed save disappeared; the displaced revision is preserved at {} and the save candidate at {}",
            recovery_path.display(),
            candidate_path.display()
        ))?;
        if installed != text.as_bytes() {
            bail!(
                "Another writer replaced the installed save. Its revision remains at {}, the displaced revision at {}, and the phone save candidate at {}",
                path.display(),
                recovery_path.display(),
                candidate_path.display()
            );
        }
        let installed_metadata = fs::metadata(&path)?;
        if !metadata_matches(&installed_metadata, &displaced_metadata_after) {
            return match roll_back_installed(&path, &recovery_path) {
                Ok(()) => bail!(
                    "File ownership or permissions could not be preserved. The original file was restored and the save candidate is preserved at {}",
                    candidate_path.display()
                ),
                Err(error) => Err(error).context(format!(
                    "The displaced revision is preserved at {} and the save candidate at {}",
                    recovery_path.display(),
                    candidate_path.display()
                )),
            };
        }
        sync_parent(parent)?;
        fs::remove_file(&recovery_path)?;
        fs::remove_file(&candidate_path)?;
        sync_parent(parent)?;
        Ok(Document {
            path: relative,
            text: text.into(),
            revision: document_revision(text.as_bytes(), &installed_metadata),
        })
    }

    fn recoveries(&self) -> Result<Value> {
        #[cfg(unix)]
        return Ok(serde_json::to_value(self.unix.recoveries()?)?);
        #[cfg(windows)]
        bail!("Secure recovery records are unavailable on this platform");
    }

    fn recovery_read(&self, id: &str, artifact: RecoveryArtifact) -> Result<Document> {
        #[cfg(unix)]
        return self.unix.recovery_read(id, artifact);
        #[cfg(windows)]
        {
            let _ = (id, artifact);
            bail!("Secure recovery records are unavailable on this platform");
        }
    }

    fn recovery_delete(&self, id: &str) -> Result<()> {
        #[cfg(unix)]
        return self.unix.recovery_delete(id);
        #[cfg(windows)]
        {
            let _ = id;
            bail!("Secure recovery records are unavailable on this platform");
        }
    }
}

pub fn serve() -> Result<()> {
    let mut input = BufReader::new(std::io::stdin());
    let mut output = std::io::stdout().lock();
    let mut files = None;
    let mut protocol = None;
    while let Some(record) = read_record(&mut input)? {
        let id = record["id"].as_str().context("Missing file request ID")?;
        let name = record["type"].as_str().unwrap_or("unknown");
        let result = (|| -> Result<Value> {
            let request: Request = serde_json::from_value(record.clone())?;
            if let Request::FilesAttach { version, target } = request {
                ensure!(
                    files.is_none() && matches!(version, 1..=FILE_PROTOCOL_VERSION),
                    "Unsupported file protocol or repeated attachment"
                );
                target.validate()?;
                files = Some(Files::new(&target.cwd)?);
                protocol = Some(version);
                return Ok(json!({"version":version,"target":target}));
            }
            let files = files.as_ref().context("Attach the file channel first")?;
            let mutating = matches!(
                &request,
                Request::FilesSave { .. }
                    | Request::FilesRecoveries
                    | Request::FilesRecoveryRead { .. }
                    | Request::FilesRecoveryDelete { .. }
            );
            ensure!(
                !mutating || protocol == Some(FILE_PROTOCOL_VERSION),
                "Update the helper and reconnect before saving or managing recoveries"
            );
            match request {
                Request::FilesList => Ok(serde_json::to_value(files.list()?)?),
                Request::FilesRead { path } => Ok(serde_json::to_value(files.read(&path)?)?),
                Request::FilesSave {
                    path,
                    text,
                    revision,
                } => Ok(serde_json::to_value(files.save(&path, &text, &revision)?)?),
                Request::FilesRecoveries => files.recoveries(),
                Request::FilesRecoveryRead { id, artifact } => {
                    Ok(serde_json::to_value(files.recovery_read(&id, artifact)?)?)
                }
                Request::FilesRecoveryDelete { id } => {
                    files.recovery_delete(&id)?;
                    Ok(Value::Null)
                }
                Request::FilesAttach { .. } => unreachable!(),
            }
        })();
        let bytes = serde_json::to_vec(&response(id, name, result))?;
        ensure!(
            bytes.len() < MAX_RECORD_BYTES,
            "File response exceeds transport limit"
        );
        output.write_all(&bytes)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recovery_texts(files: &Files) -> Vec<(String, String)> {
        let records: Vec<pi_core::remote_files::Recovery> =
            serde_json::from_value(files.recoveries().unwrap()).unwrap();
        records
            .into_iter()
            .flat_map(|record| {
                [
                    RecoveryArtifact::Original,
                    RecoveryArtifact::Boundary,
                    RecoveryArtifact::Installed,
                    RecoveryArtifact::Draft,
                ]
                .into_iter()
                .filter_map(move |artifact| {
                    files
                        .recovery_read(&record.id, artifact)
                        .ok()
                        .map(|document| (document.path, document.text))
                })
            })
            .collect()
    }

    #[test]
    fn text_round_trip_conflicts_and_project_boundary() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("hello 'world'.txt");
        fs::write(&path, "héllo\r\n").unwrap();
        let original_metadata = fs::metadata(&path).unwrap();
        let files = Files::new(root.path().to_str().unwrap()).unwrap();
        let first = files.read("hello 'world'.txt").unwrap();
        assert_eq!(first.text, "héllo\r\n");
        let saved = files
            .save(&first.path, "changed\n", &first.revision)
            .unwrap();
        assert_ne!(first.revision, saved.revision);
        assert!(files.save(&first.path, "stale", &first.revision).is_err());
        assert!(files.read("../outside").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "changed\n");
        assert!(
            !root.path().join(".pi-file-recovery").exists(),
            "recovery artifacts must not pollute the project tree"
        );
        assert!(metadata_matches(
            &original_metadata,
            &fs::metadata(path).unwrap()
        ));
    }
    #[test]
    fn a_change_at_the_commit_boundary_is_never_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("race.txt");
        fs::write(&path, "original").unwrap();
        let files = Files::new(root.path().to_str().unwrap()).unwrap();
        let revision = files.read("race.txt").unwrap().revision;

        let result = files.save_with("race.txt", "phone draft", &revision, |live| {
            assert!(live.exists(), "the live pathname is never made absent");
            fs::write(live, "concurrent edit")?;
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "concurrent edit");
        assert!(
            recovery_texts(&files)
                .iter()
                .any(|(name, text)| name.ends_with("/draft") && text == "phone draft"),
            "the phone candidate is retained when the boundary revision changed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_rename_after_final_validation_is_restored_by_the_provisional_exchange() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("exchange-race.txt");
        fs::write(&path, "original").unwrap();
        let files = Files::new(root.path().to_str().unwrap()).unwrap();
        let revision = files.read("exchange-race.txt").unwrap().revision;

        let result =
            files.save_with_exchange_hook("exchange-race.txt", "phone draft", &revision, |live| {
                let replacement = root.path().join("exchange-racer.tmp");
                fs::write(&replacement, "concurrent rename")?;
                fs::rename(replacement, live)?;
                Ok(())
            });
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "concurrent rename");
        assert!(
            recovery_texts(&files)
                .iter()
                .any(|(name, text)| name.ends_with("/draft") && text == "phone draft")
        );
    }

    #[test]
    fn an_open_editor_cannot_lose_a_late_write_to_the_displaced_inode() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("open.txt");
        fs::write(&path, "original").unwrap();
        let files = Files::new(root.path().to_str().unwrap()).unwrap();
        let revision = files.read("open.txt").unwrap().revision;
        let mut editor = OpenOptions::new().write(true).open(&path).unwrap();

        let result = files.save_with_hooks(
            "open.txt",
            "phone draft",
            &revision,
            |_| Ok(()),
            |_| {
                editor.set_len(0)?;
                editor.write_all(b"open editor edit")?;
                editor.sync_all()?;
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "phone draft",
            "post-commit error handling must not roll back the live pathname"
        );
        let artifacts = recovery_texts(&files);
        assert!(artifacts.iter().any(|(_, text)| text == "open editor edit"));
        assert!(
            artifacts
                .iter()
                .any(|(name, text)| { name.ends_with("/draft") && text == "phone draft" })
        );
    }

    #[test]
    fn an_atomic_writer_after_install_wins_without_erasing_the_phone_candidate() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("atomic.txt");
        fs::write(&path, "original").unwrap();
        let files = Files::new(root.path().to_str().unwrap()).unwrap();
        let revision = files.read("atomic.txt").unwrap().revision;

        let result = files.save_with_hooks(
            "atomic.txt",
            "phone draft",
            &revision,
            |_| Ok(()),
            |live| {
                let replacement = root.path().join("other-writer.tmp");
                fs::write(&replacement, "other writer")?;
                fs::rename(replacement, live)?;
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "other writer");
        let artifacts = recovery_texts(&files);
        assert!(artifacts.iter().any(|(_, text)| text == "original"));
        assert!(
            artifacts
                .iter()
                .any(|(name, text)| { name.ends_with("/draft") && text == "phone draft" })
        );
    }

    #[test]
    fn rejects_binary_huge_and_special_files_and_skips_generated_trees() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("binary"), [0, 1, 2]).unwrap();
        fs::File::create(root.path().join("huge"))
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();
        fs::create_dir(root.path().join(".git")).unwrap();
        fs::create_dir(root.path().join("src")).unwrap();
        fs::write(root.path().join("src/code.rs"), "fn main() {}").unwrap();
        let files = Files::new(root.path().to_str().unwrap()).unwrap();
        assert!(files.read("binary").is_err());
        assert!(files.read("huge").is_err());
        assert!(files.read("src").is_err());
        let tree = files.list().unwrap();
        let repeated = files.list().unwrap();
        assert_eq!(
            tree.entries
                .iter()
                .map(|entry| &entry.path)
                .collect::<Vec<_>>(),
            repeated
                .entries
                .iter()
                .map(|entry| &entry.path)
                .collect::<Vec<_>>(),
            "repeated listings must use a fresh directory cursor"
        );
        assert!(!tree.entries.iter().any(|e| e.path.starts_with(".git")));
        let size = |path: &str| tree.entries.iter().find(|e| e.path == path).unwrap().size;
        assert_eq!(size("src/code.rs"), Some(12));
        assert_eq!(size("src"), None);
    }
    #[cfg(unix)]
    #[test]
    fn late_writes_remain_reachable_after_a_successful_save() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("late.txt");
        fs::write(&path, "original").unwrap();
        let files = Files::new(root.path().to_str().unwrap()).unwrap();
        let revision = files.read("late.txt").unwrap().revision;
        let mut old_editor = OpenOptions::new().write(true).open(&path).unwrap();
        files.save("late.txt", "phone draft", &revision).unwrap();

        old_editor.set_len(0).unwrap();
        old_editor.write_all(b"late remote write").unwrap();
        old_editor.sync_all().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "phone draft");
        assert!(
            recovery_texts(&files)
                .iter()
                .any(|(_, text)| text == "late remote write"),
            "the old open inode must keep a durable recovery name"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_swapped_parent_never_redirects_the_descriptor_anchored_commit() {
        use std::os::unix::fs::symlink;
        let holder = tempfile::tempdir().unwrap();
        let root = holder.path().join("project");
        let moved = holder.path().join("project-held");
        let outside = holder.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(root.join("file.txt"), "original").unwrap();
        fs::write(outside.join("file.txt"), "outside").unwrap();
        let files = Files::new(root.to_str().unwrap()).unwrap();
        let revision = files.read("file.txt").unwrap().revision;

        let saved = files.save_with("file.txt", "phone", &revision, |_| {
            fs::rename(&root, &moved)?;
            symlink(&outside, &root)?;
            Ok(())
        });
        assert!(saved.is_ok(), "{saved:?}");
        assert_eq!(fs::read_to_string(moved.join("file.txt")).unwrap(), "phone");
        assert_eq!(
            fs::read_to_string(outside.join("file.txt")).unwrap(),
            "outside"
        );
        fs::remove_file(&root).unwrap();
        fs::rename(&moved, &root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn recovery_quota_backpressures_until_an_explicit_delete() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("quota.txt");
        fs::write(&path, "0").unwrap();
        let files = Files::new(root.path().to_str().unwrap()).unwrap();
        for index in 1..=unix_backend::MAX_RECOVERY_SLOTS {
            let current = files.read("quota.txt").unwrap();
            files
                .save("quota.txt", &index.to_string(), &current.revision)
                .unwrap();
        }
        let current = files.read("quota.txt").unwrap();
        let error = files
            .save("quota.txt", "blocked", &current.revision)
            .unwrap_err();
        assert!(format!("{error:#}").contains("recovery is full"));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            unix_backend::MAX_RECOVERY_SLOTS.to_string()
        );

        let records: Vec<pi_core::remote_files::Recovery> =
            serde_json::from_value(files.recoveries().unwrap()).unwrap();
        files.recovery_delete(&records[0].id).unwrap();
        let current = files.read("quota.txt").unwrap();
        files
            .save("quota.txt", "after delete", &current.revision)
            .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "after delete");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn saves_preserve_extended_attributes_exactly() {
        use std::{ffi::CString, os::fd::AsRawFd as _};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("xattr.txt");
        fs::write(&path, "original").unwrap();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let name = CString::new("user.pi-test").unwrap();
        let value = b"metadata";
        let status = unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        };
        if status == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOTSUP) {
            return;
        }
        assert_eq!(status, 0);
        let files = Files::new(root.path().to_str().unwrap()).unwrap();
        let revision = files.read("xattr.txt").unwrap().revision;
        files.save("xattr.txt", "saved", &revision).unwrap();
        let saved = OpenOptions::new().read(true).open(&path).unwrap();
        let mut actual = [0u8; 32];
        let read = unsafe {
            libc::fgetxattr(
                saved.as_raw_fd(),
                name.as_ptr(),
                actual.as_mut_ptr().cast(),
                actual.len(),
            )
        };
        assert_eq!(&actual[..read as usize], value);
    }

    #[cfg(unix)]
    #[test]
    fn a_permission_change_at_the_commit_boundary_is_a_conflict() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("mode.txt");
        fs::write(&path, "original").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let files = Files::new(root.path().to_str().unwrap()).unwrap();
        let revision = files.read("mode.txt").unwrap().revision;
        let result = files.save_with("mode.txt", "phone draft", &revision, |live| {
            fs::set_permissions(live, fs::Permissions::from_mode(0o444))?;
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "original");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o444
        );
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinks_and_preserves_executable_permissions() {
        use std::os::unix::{
            ffi::OsStrExt,
            fs::{PermissionsExt, symlink},
        };
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), "outside").unwrap();
        symlink(outside.path(), root.path().join("link")).unwrap();
        let files = Files::new(root.path().to_str().unwrap()).unwrap();
        assert!(files.read("link/secret").is_err());
        let fifo = root.path().join("fifo");
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        unsafe {
            libc::mkfifo(name.as_ptr(), 0o600);
        }
        assert!(files.read("fifo").is_err());
        fs::write(root.path().join("script"), "#!/bin/sh\n").unwrap();
        fs::set_permissions(
            root.path().join("script"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let doc = files.read("script").unwrap();
        files
            .save("script", "#!/bin/sh\necho yes\n", &doc.revision)
            .unwrap();
        assert_eq!(
            fs::metadata(root.path().join("script"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }
}
