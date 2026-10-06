//! Bounded text-file service over SSH stdio, independent of Pi and its daemon.
use anyhow::{Context, Result, bail, ensure};
use pi_core::{
    protocol::{MAX_RECORD_BYTES, read_record},
    remote_files::{
        Document, Entry, FILE_PROTOCOL_VERSION, MAX_FILE_BYTES, MAX_TREE_ENTRIES, Request, Tree,
        response,
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{BufReader, Read, Write},
    path::{Component, Path, PathBuf},
};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
struct Files {
    root: PathBuf,
}
impl Files {
    fn new(root: &str) -> Result<Self> {
        let root = fs::canonicalize(root).context("Remote project must exist")?;
        ensure!(root.is_dir(), "Remote project must be a directory");
        Ok(Self { root })
    }
    // Never follow symlink components or accept parent traversal. This is a project
    // boundary, not an OS sandbox against a malicious writer with the same UID.
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
        let (path, relative) = self.resolve(name)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(&path)?;
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
        let revision = hash(&bytes);
        let text = String::from_utf8(bytes).context("Remote editor requires UTF-8 text")?;
        Ok(Document {
            path: relative,
            text,
            revision,
        })
    }
    fn list(&self) -> Result<Tree> {
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
        let permissions = fs::metadata(&path)?.permissions();
        ensure!(!permissions.readonly(), "Remote file is read-only");
        let mut staging =
            tempfile::NamedTempFile::new_in(path.parent().context("File has no parent")?)?;
        staging.write_all(text.as_bytes())?;
        staging.as_file().set_permissions(permissions)?;
        staging.as_file().sync_all()?;
        // Check again after writing the temporary file, including edits by Pi.
        ensure!(
            self.read(name)?.revision == revision,
            "File changed on the remote host while saving. Your buffer has been kept"
        );
        staging.persist(&path).map_err(|error| error.error)?;
        Ok(Document {
            path: relative,
            text: text.into(),
            revision: hash(text.as_bytes()),
        })
    }
}

pub fn serve() -> Result<()> {
    let mut input = BufReader::new(std::io::stdin());
    let mut output = std::io::stdout().lock();
    let mut files = None;
    while let Some(record) = read_record(&mut input)? {
        let id = record["id"].as_str().context("Missing file request ID")?;
        let name = record["type"].as_str().unwrap_or("unknown");
        let result = (|| -> Result<Value> {
            let request: Request = serde_json::from_value(record.clone())?;
            if let Request::FilesAttach { version, target } = request {
                ensure!(
                    files.is_none() && version == FILE_PROTOCOL_VERSION,
                    "Unsupported file protocol or repeated attachment"
                );
                target.validate()?;
                files = Some(Files::new(&target.cwd)?);
                return Ok(json!({"version":FILE_PROTOCOL_VERSION,"target":target}));
            }
            let files = files.as_ref().context("Attach the file channel first")?;
            match request {
                Request::FilesList => Ok(serde_json::to_value(files.list()?)?),
                Request::FilesRead { path } => Ok(serde_json::to_value(files.read(&path)?)?),
                Request::FilesSave {
                    path,
                    text,
                    revision,
                } => Ok(serde_json::to_value(files.save(&path, &text, &revision)?)?),
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
    #[test]
    fn text_round_trip_conflicts_and_project_boundary() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("hello 'world'.txt"), "héllo\r\n").unwrap();
        let files = Files::new(root.path().to_str().unwrap()).unwrap();
        let first = files.read("hello 'world'.txt").unwrap();
        assert_eq!(first.text, "héllo\r\n");
        let saved = files
            .save(&first.path, "changed\n", &first.revision)
            .unwrap();
        assert_ne!(first.revision, saved.revision);
        assert!(files.save(&first.path, "stale", &first.revision).is_err());
        assert!(files.read("../outside").is_err());
        assert_eq!(
            fs::read_to_string(root.path().join(&first.path)).unwrap(),
            "changed\n"
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
        assert!(!tree.entries.iter().any(|e| e.path.starts_with(".git")));
        let size = |path: &str| tree.entries.iter().find(|e| e.path == path).unwrap().size;
        assert_eq!(size("src/code.rs"), Some(12));
        assert_eq!(size("src"), None);
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
