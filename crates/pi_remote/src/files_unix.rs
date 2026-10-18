//! Descriptor-relative Unix file editing and persistent recovery records.

use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt as _;
use pi_core::{
    protocol::MAX_RECORD_BYTES,
    remote_files::{
        Document, Entry, MAX_FILE_BYTES, MAX_TREE_ENTRIES, Recovery, RecoveryArtifact, Tree,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    ffi::{CStr, CString, OsStr},
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt as _, fs::MetadataExt as _},
    },
    path::{Component, Path, PathBuf},
};

const RECOVERY_DIRECTORY: &str = ".pi-file-recovery";
pub(super) const MAX_RECOVERY_SLOTS: usize = 64;
const MAX_SLOT_ENTRIES: usize = 12;
const MAX_PATH_COMPONENTS: usize = 128;
const MAX_PATH_BYTES: usize = 16 * 1024;
const MAX_DIRECTORY_SCAN: usize = 100_000;
const MAX_PENDING_DIRECTORIES: usize = 4_096;
const MAX_XATTR_NAMES: usize = 128;
const MAX_XATTR_BYTES: usize = 256 * 1024;

const ORIGINAL: &str = "original";
const BOUNDARY: &str = "boundary";
const INSTALLED: &str = "installed";
const DRAFT: &str = "draft";
const RECORD: &str = "record.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RecoveryRecord {
    id: String,
    path: String,
    state: String,
}

struct Resolved {
    parent: File,
    parent_identity: (u64, u64),
    name: CString,
    relative: String,
    display: PathBuf,
}

#[derive(Clone)]
struct Snapshot {
    bytes: Vec<u8>,
    metadata: fs::Metadata,
    xattrs: Vec<(Vec<u8>, Vec<u8>)>,
    acl: Option<Vec<u8>>,
    revision: String,
}

pub(super) struct UnixFiles {
    root: File,
    root_path: PathBuf,
    root_device: u64,
    recovery_parent: File,
    recovery_name: CString,
    #[cfg(test)]
    recovery_path: PathBuf,
}

impl UnixFiles {
    pub(super) fn new(root_path: PathBuf) -> Result<Self> {
        let root_name = CString::new(root_path.as_os_str().as_bytes())?;
        let fd = unsafe {
            libc::open(
                root_name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd == -1 {
            return Err(std::io::Error::last_os_error())
                .context("Could not securely open the remote project");
        }
        let root = unsafe { File::from_raw_fd(fd) };
        let metadata = root.metadata()?;
        ensure!(metadata.is_dir(), "Remote project must be a directory");
        let parent_path = root_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .context("Secure file recovery needs a project parent directory")?
            .to_path_buf();
        let parent_name = CString::new(parent_path.as_os_str().as_bytes())?;
        let parent_fd = unsafe {
            libc::open(
                parent_name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if parent_fd == -1 {
            return Err(std::io::Error::last_os_error())
                .context("Could not securely open the project parent for file recovery");
        }
        let recovery_parent = unsafe { File::from_raw_fd(parent_fd) };
        ensure!(
            recovery_parent.metadata()?.dev() == metadata.dev(),
            "Secure file editing is unavailable when the project root is a filesystem mount point"
        );
        let mut recovery_identity = Sha256::new();
        recovery_identity.update(b"pi-file-recovery-v2\0");
        recovery_identity.update(root_path.as_os_str().as_bytes());
        recovery_identity.update(metadata.dev().to_le_bytes());
        recovery_identity.update(metadata.ino().to_le_bytes());
        let recovery_name = CString::new(format!(
            ".pi-file-recovery-{:x}",
            recovery_identity.finalize()
        ))?;
        Ok(Self {
            root_device: metadata.dev(),
            root,
            root_path,
            recovery_parent,
            #[cfg(test)]
            recovery_path: parent_path.join(OsStr::from_bytes(recovery_name.as_bytes())),
            recovery_name,
        })
    }

    pub(super) fn read(&self, name: &str) -> Result<Document> {
        let resolved = self.resolve(name)?;
        let file = open_at(
            &resolved.parent,
            &resolved.name,
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            0,
        )
        .context("Could not securely open the remote file")?;
        let snapshot = snapshot(&file)?;
        let text =
            String::from_utf8(snapshot.bytes).context("Remote editor requires UTF-8 text")?;
        Ok(Document {
            path: resolved.relative,
            text,
            revision: snapshot.revision,
        })
    }

    pub(super) fn list(&self) -> Result<Tree> {
        let mut tree = Tree {
            entries: Vec::new(),
            truncated: false,
        };
        let mut pending = vec![(self.root.try_clone()?, String::new(), 0usize)];
        let mut scanned = 0usize;
        let mut response_budget = 0usize;
        while let Some((directory, prefix, depth)) = pending.pop() {
            let (names, names_truncated) =
                directory_names(&directory, MAX_DIRECTORY_SCAN.saturating_sub(scanned))?;
            if names_truncated {
                tree.truncated = true;
                return Ok(tree);
            }
            for name in names {
                scanned += 1;
                if scanned > MAX_DIRECTORY_SCAN {
                    tree.truncated = true;
                    return Ok(tree);
                }
                let Some(name) = name.to_str().map(str::to_owned) else {
                    continue;
                };
                if hidden_tree_name(&name) || name.contains(['\\', ':', '\n', '\r']) {
                    continue;
                }
                let c_name = CString::new(name.as_bytes())?;
                let stat = stat_at(&directory, &c_name)?;
                let kind = stat.st_mode & libc::S_IFMT;
                if kind != libc::S_IFREG && kind != libc::S_IFDIR {
                    continue;
                }
                let relative = if prefix.is_empty() {
                    name
                } else {
                    format!("{prefix}/{name}")
                };
                response_budget = response_budget
                    .checked_add(relative.len() * 6 + 128)
                    .context("Remote tree size overflow")?;
                if tree.entries.len() == MAX_TREE_ENTRIES || response_budget > MAX_RECORD_BYTES / 2
                {
                    tree.truncated = true;
                    return Ok(tree);
                }
                let directory_kind = kind == libc::S_IFDIR;
                tree.entries.push(Entry {
                    path: relative.clone(),
                    directory: directory_kind,
                    size: (kind == libc::S_IFREG).then_some(stat.st_size.max(0) as u64),
                });
                if directory_kind {
                    if depth == MAX_PATH_COMPONENTS || pending.len() == MAX_PENDING_DIRECTORIES {
                        tree.truncated = true;
                        return Ok(tree);
                    }
                    let child = open_at(
                        &directory,
                        &c_name,
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                        0,
                    )?;
                    let child_metadata = child.metadata()?;
                    if child_metadata.dev() != stat.st_dev as u64
                        || child_metadata.ino() != stat.st_ino as u64
                    {
                        bail!("Remote directory changed while listing");
                    }
                    pending.push((child, relative, depth + 1));
                }
            }
        }
        tree.entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(tree)
    }

    pub(super) fn save(
        &self,
        name: &str,
        text: &str,
        revision: &str,
        before_install: impl FnOnce(&Path) -> Result<()>,
        before_exchange: impl FnOnce(&Path) -> Result<()>,
        after_install: impl FnOnce(&Path) -> Result<()>,
    ) -> Result<Document> {
        ensure!(
            text.len() as u64 <= MAX_FILE_BYTES && !text.contains('\0'),
            "Remote saves require UTF-8 text under 1 MiB"
        );
        let resolved = self.resolve(name)?;
        ensure!(
            resolved.parent.metadata()?.dev() == self.root_device,
            "Secure file recovery does not support nested mount points"
        );
        // Reject stale or read-only saves before consuming a durable recovery
        // slot. Revalidate the linked inode after reserving, since another
        // writer can still act while the recovery store is locked.
        let initial = open_at(
            &resolved.parent,
            &resolved.name,
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            0,
        )?;
        let initial_snapshot = snapshot(&initial)?;
        ensure!(
            initial_snapshot.revision == revision,
            "File changed on the remote host. Reload or copy your edits before saving"
        );
        ensure!(
            initial_snapshot.metadata.mode() & 0o222 != 0,
            "Remote file is read-only"
        );

        let (recovery, _recovery_lock) = self.lock_recovery_store()?;
        let (slot_id, slot) = reserve_slot(&recovery, &resolved.relative)?;
        let recovery_hint = format!("{RECOVERY_DIRECTORY}/{slot_id}");

        let original_name = c(ORIGINAL);
        if unsafe {
            libc::linkat(
                resolved.parent.as_raw_fd(),
                resolved.name.as_ptr(),
                slot.as_raw_fd(),
                original_name.as_ptr(),
                0,
            )
        } == -1
        {
            return Err(std::io::Error::last_os_error())
                .context("Could not retain the original file in the recovery record");
        }
        let original = open_at(
            &slot,
            &original_name,
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            0,
        )?;
        let source = snapshot(&original)?;
        ensure!(
            source.revision == revision && same_inode(&source.metadata, &initial_snapshot.metadata),
            "File changed at the save boundary. Recovery record: {recovery_hint}"
        );
        let live = open_at(
            &resolved.parent,
            &resolved.name,
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            0,
        )?;
        ensure!(
            same_inode(&live.metadata()?, &source.metadata),
            "File changed at the save boundary. Recovery record: {recovery_hint}"
        );

        let boundary = create_at(&slot, &c(BOUNDARY), 0o600)?;
        prepare_candidate(&boundary, text.as_bytes(), &source)?;
        let installed_name = c(INSTALLED);
        if unsafe {
            libc::linkat(
                slot.as_raw_fd(),
                c(BOUNDARY).as_ptr(),
                slot.as_raw_fd(),
                installed_name.as_ptr(),
                0,
            )
        } == -1
        {
            return Err(std::io::Error::last_os_error())
                .context("Could not retain the installed inode in the recovery record");
        }
        let draft = create_at(&slot, &c(DRAFT), 0o600)?;
        prepare_candidate(&draft, text.as_bytes(), &source)?;
        write_record(
            &slot,
            &RecoveryRecord {
                id: slot_id.clone(),
                path: resolved.relative.clone(),
                state: "prepared".into(),
            },
        )?;
        slot.sync_all()?;
        recovery.sync_all()?;

        before_install(&resolved.display)?;
        let original_again = snapshot(&original)?;
        ensure!(
            original_again.revision == revision,
            "File changed through an open editor before installation. Recovery record: {recovery_hint}"
        );
        let parent_again = self.resolve(&resolved.relative)?;
        ensure!(
            parent_again.parent_identity == resolved.parent_identity,
            "The file's parent directory changed while saving. Recovery record: {recovery_hint}"
        );
        let live_again = open_at(
            &parent_again.parent,
            &parent_again.name,
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            0,
        )?;
        ensure!(
            same_inode(&live_again.metadata()?, &source.metadata),
            "File changed at the final save boundary. Recovery record: {recovery_hint}"
        );
        let boundary_metadata = boundary.metadata()?;
        let installed_metadata = open_at(
            &slot,
            &installed_name,
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0,
        )?
        .metadata()?;
        ensure!(
            same_inode(&boundary_metadata, &installed_metadata),
            "Prepared save was replaced before installation. Recovery record: {recovery_hint}"
        );

        before_exchange(&resolved.display)?;
        exchange_at(&slot, &c(BOUNDARY), &resolved.parent, &resolved.name).map_err(|error| {
            error.context(format!(
                "Atomic save was not completed. Recovery record: {recovery_hint}"
            ))
        })?;
        // The exchange is provisional until its displaced inode is checked.
        // If a rename won the tiny interval after final validation, restore it
        // while our candidate is still the live inode. Every inode also has a
        // recovery hard link, so even a second racer remains reachable.
        let displaced = open_at(
            &slot,
            &c(BOUNDARY),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            0,
        )?;
        let displaced_snapshot = snapshot(&displaced)?;
        let provisional_live = open_at(
            &resolved.parent,
            &resolved.name,
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            0,
        )?;
        let candidate_is_live = same_inode(&provisional_live.metadata()?, &installed_metadata);
        let expected_was_displaced = same_inode(&displaced_snapshot.metadata, &source.metadata)
            && displaced_snapshot.revision == revision;
        if !expected_was_displaced || !candidate_is_live {
            if candidate_is_live {
                exchange_at(&slot, &c(BOUNDARY), &resolved.parent, &resolved.name)
                    .context("Could not restore a file that changed at the atomic save boundary")?;
                resolved.parent.sync_all()?;
                slot.sync_all()?;
                bail!(
                    "File changed at the atomic save boundary; the concurrent version was restored. Recovery record: {recovery_hint}"
                );
            }
            bail!(
                "File changed again during the atomic save boundary. All observed inodes remain in the recovery record: {recovery_hint}"
            );
        }
        // COMMIT POINT: no operation below may modify the live pathname.
        let post_commit = (|| -> Result<Document> {
            after_install(&resolved.display)?;
            resolved.parent.sync_all()?;
            slot.sync_all()?;

            let boundary_now = open_at(
                &slot,
                &c(BOUNDARY),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                0,
            )?;
            let boundary_snapshot = snapshot(&boundary_now)?;
            if boundary_snapshot.revision != revision {
                bail!("A writer changed or replaced the remote file at the commit boundary");
            }
            let installed_now = open_at(
                &resolved.parent,
                &resolved.name,
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                0,
            )?;
            let installed_snapshot = snapshot(&installed_now)?;
            ensure!(
                installed_snapshot.bytes == text.as_bytes(),
                "Another writer changed or replaced the installed save"
            );
            ensure!(
                metadata_equivalent(&installed_snapshot, &source),
                "File ownership, mode, ACL, or extended attributes were not preserved"
            );
            write_record(
                &slot,
                &RecoveryRecord {
                    id: slot_id.clone(),
                    path: resolved.relative.clone(),
                    state: "installed".into(),
                },
            )?;
            slot.sync_all()?;
            Ok(Document {
                path: resolved.relative.clone(),
                text: text.into(),
                revision: installed_snapshot.revision,
            })
        })();
        post_commit.with_context(|| {
            format!(
                "The save crossed its atomic commit point and was not rolled back. The live pathname was left untouched after the failure. Recovery record: {recovery_hint}"
            )
        })
    }

    pub(super) fn recoveries(&self) -> Result<Vec<Recovery>> {
        let (recovery, _lock) = self.lock_recovery_store()?;
        let mut result = Vec::new();
        for id in slot_names(&recovery)? {
            let record = (|| -> Result<RecoveryRecord> {
                let slot = open_slot(&recovery, &id)?;
                validate_slot(&slot)?;
                let record: RecoveryRecord =
                    serde_json::from_slice(&read_named_bounded(&slot, RECORD, 16 * 1024)?)?;
                ensure!(record.id == id, "Recovery record identity mismatch");
                Ok(record)
            })();
            match record {
                Ok(record) => result.push(Recovery {
                    id,
                    path: record.path,
                    state: record.state,
                }),
                Err(_) => result.push(Recovery {
                    id,
                    path: "(incomplete recovery record)".into(),
                    state: "incomplete".into(),
                }),
            }
        }
        result.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(result)
    }

    pub(super) fn recovery_read(&self, id: &str, artifact: RecoveryArtifact) -> Result<Document> {
        validate_slot_id(id)?;
        let (recovery, _lock) = self.lock_recovery_store()?;
        let slot = open_slot(&recovery, id)?;
        validate_slot(&slot)?;
        let name = artifact_name(artifact);
        let file = open_at(
            &slot,
            &c(name),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            0,
        )?;
        let snapshot = snapshot(&file)?;
        let text =
            String::from_utf8(snapshot.bytes).context("Recovery artifact is not UTF-8 text")?;
        Ok(Document {
            path: format!("{RECOVERY_DIRECTORY}/{id}/{name}"),
            text,
            revision: snapshot.revision,
        })
    }

    pub(super) fn recovery_delete(&self, id: &str) -> Result<()> {
        validate_slot_id(id)?;
        let (recovery, _lock) = self.lock_recovery_store()?;
        let slot = open_slot(&recovery, id)?;
        let (names, truncated) = directory_names(&slot, MAX_SLOT_ENTRIES)?;
        ensure!(
            !truncated,
            "Recovery slot has an unexpected number of entries"
        );
        for name in names {
            let name = CString::new(name.as_bytes())?;
            let stat = stat_at(&slot, &name)?;
            ensure!(
                stat.st_mode & libc::S_IFMT == libc::S_IFREG,
                "Recovery slot contains an unsupported entry"
            );
            if unsafe { libc::unlinkat(slot.as_raw_fd(), name.as_ptr(), 0) } == -1 {
                return Err(std::io::Error::last_os_error())
                    .context("Could not explicitly delete recovery artifact");
            }
        }
        slot.sync_all()?;
        if unsafe { libc::unlinkat(recovery.as_raw_fd(), c(id).as_ptr(), libc::AT_REMOVEDIR) } == -1
        {
            return Err(std::io::Error::last_os_error())
                .context("Could not explicitly delete recovery slot");
        }
        recovery.sync_all()?;
        Ok(())
    }

    fn resolve(&self, name: &str) -> Result<Resolved> {
        let (parts, relative) = parse_path(&self.root_path, name)?;
        let mut parent = self.root.try_clone()?;
        for part in &parts[..parts.len() - 1] {
            parent = open_at(
                &parent,
                &c(part),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0,
            )
            .with_context(|| format!("Could not securely open directory {part}"))?;
        }
        let metadata = parent.metadata()?;
        let leaf = parts.last().unwrap();
        Ok(Resolved {
            parent,
            parent_identity: (metadata.dev(), metadata.ino()),
            name: c(leaf),
            relative,
            display: self.root_path.join(parts.iter().collect::<PathBuf>()),
        })
    }

    fn lock_recovery_store(&self) -> Result<(File, File)> {
        let status = unsafe {
            libc::mkdirat(
                self.recovery_parent.as_raw_fd(),
                self.recovery_name.as_ptr(),
                0o700,
            )
        };
        if status == -1 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
            return Err(std::io::Error::last_os_error())
                .context("Could not create the file recovery store");
        }
        let recovery = open_at(
            &self.recovery_parent,
            &self.recovery_name,
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0,
        )?;
        let metadata = recovery.metadata()?;
        ensure!(
            metadata.dev() == self.root_device
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "File recovery store ownership, mode, or filesystem is unsafe"
        );
        let lock = open_at(
            &recovery,
            &c(".lock"),
            libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )?;
        lock.try_lock_exclusive()
            .context("Another file recovery operation is in progress")?;
        Ok((recovery, lock))
    }
}

#[cfg(test)]
impl Drop for UnixFiles {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.recovery_path);
    }
}

fn hidden_tree_name(name: &str) -> bool {
    matches!(
        name,
        ".git" | ".jj" | ".hg" | "node_modules" | "target" | RECOVERY_DIRECTORY
    )
}

fn parse_path(root: &Path, name: &str) -> Result<(Vec<String>, String)> {
    ensure!(
        !name.is_empty() && name.len() <= MAX_PATH_BYTES && !name.contains(['\0', '\n', '\r']),
        "Invalid remote file path"
    );
    let path = Path::new(name);
    let relative = if path.is_absolute() {
        path.strip_prefix(root)
            .context("File is outside the remote project")?
    } else {
        path
    };
    let mut parts = Vec::new();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            bail!("Parent traversal is not allowed")
        };
        let part = part.to_str().context("Non-UTF-8 paths are not supported")?;
        ensure!(
            !part.contains(['\\', ':']) && part != RECOVERY_DIRECTORY,
            "Unsupported path component"
        );
        parts.push(part.to_owned());
    }
    ensure!(
        !parts.is_empty() && parts.len() <= MAX_PATH_COMPONENTS,
        "Choose a file inside the remote project"
    );
    Ok((parts.clone(), parts.join("/")))
}

fn reserve_slot(recovery: &File, path: &str) -> Result<(String, File)> {
    let existing = slot_names(recovery)?;
    ensure!(
        existing.len() < MAX_RECOVERY_SLOTS,
        "Remote save recovery is full. Explicitly inspect and delete an old recovery record before saving"
    );
    for _ in 0..32 {
        let id = format!("{:032x}", rand::random::<u128>());
        let name = c(&id);
        if unsafe { libc::mkdirat(recovery.as_raw_fd(), name.as_ptr(), 0o700) } == 0 {
            let slot = open_slot(recovery, &id)?;
            write_record(
                &slot,
                &RecoveryRecord {
                    id: id.clone(),
                    path: path.into(),
                    state: "reserved".into(),
                },
            )?;
            slot.sync_all()?;
            recovery.sync_all()?;
            return Ok((id, slot));
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
            return Err(std::io::Error::last_os_error())
                .context("Could not reserve a recovery record");
        }
    }
    bail!("Could not allocate a unique recovery record")
}

fn slot_names(recovery: &File) -> Result<Vec<String>> {
    let mut slots = Vec::new();
    let (names, truncated) = directory_names(recovery, MAX_RECOVERY_SLOTS + 1)?;
    ensure!(!truncated, "Recovery store exceeds its fixed quota");
    for name in names {
        let Some(name) = name.to_str() else {
            bail!("Recovery store contains a non-UTF-8 entry")
        };
        if name == ".lock" {
            continue;
        }
        validate_slot_id(name)?;
        let stat = stat_at(recovery, &c(name))?;
        ensure!(
            stat.st_mode & libc::S_IFMT == libc::S_IFDIR,
            "Recovery store contains an unsupported entry"
        );
        slots.push(name.to_owned());
        ensure!(
            slots.len() <= MAX_RECOVERY_SLOTS,
            "Recovery store exceeds its fixed quota"
        );
    }
    Ok(slots)
}

fn validate_slot_id(id: &str) -> Result<()> {
    ensure!(
        id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Invalid recovery record ID"
    );
    Ok(())
}

fn open_slot(recovery: &File, id: &str) -> Result<File> {
    validate_slot_id(id)?;
    open_at(
        recovery,
        &c(id),
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        0,
    )
    .context("Could not securely open the recovery record")
}

fn validate_slot(slot: &File) -> Result<()> {
    let metadata = slot.metadata()?;
    ensure!(
        metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
        "Recovery slot ownership or mode is unsafe"
    );
    let (_, truncated) = directory_names(slot, MAX_SLOT_ENTRIES)?;
    ensure!(!truncated, "Recovery slot exceeds its fixed entry quota");
    Ok(())
}

fn artifact_name(artifact: RecoveryArtifact) -> &'static str {
    match artifact {
        RecoveryArtifact::Original => ORIGINAL,
        RecoveryArtifact::Boundary => BOUNDARY,
        RecoveryArtifact::Installed => INSTALLED,
        RecoveryArtifact::Draft => DRAFT,
    }
}

fn write_record(slot: &File, record: &RecoveryRecord) -> Result<()> {
    let bytes = serde_json::to_vec(record)?;
    ensure!(bytes.len() <= 16 * 1024, "Recovery record is too large");
    let temporary_name = format!(".record-{:016x}", rand::random::<u64>());
    let temporary = create_at(slot, &c(&temporary_name), 0o600)?;
    (&temporary).write_all(&bytes)?;
    temporary.sync_all()?;
    if unsafe {
        libc::renameat(
            slot.as_raw_fd(),
            c(&temporary_name).as_ptr(),
            slot.as_raw_fd(),
            c(RECORD).as_ptr(),
        )
    } == -1
    {
        return Err(std::io::Error::last_os_error()).context("Could not publish recovery record");
    }
    slot.sync_all()?;
    Ok(())
}

fn prepare_candidate(file: &File, bytes: &[u8], source: &Snapshot) -> Result<()> {
    let mut writer = file;
    writer.write_all(bytes)?;
    preserve_metadata(file, source)?;
    file.sync_all()?;
    let prepared = snapshot(file)?;
    ensure!(
        prepared.bytes == bytes && metadata_equivalent(&prepared, source),
        "Could not exactly preserve file ownership, mode, ACL, and extended attributes"
    );
    Ok(())
}

fn preserve_metadata(file: &File, source: &Snapshot) -> Result<()> {
    let status = unsafe {
        libc::fchown(
            file.as_raw_fd(),
            source.metadata.uid(),
            source.metadata.gid(),
        )
    };
    if status == -1 {
        return Err(std::io::Error::last_os_error())
            .context("Could not preserve remote file ownership");
    }
    let source_names: HashSet<&[u8]> = source
        .xattrs
        .iter()
        .map(|(name, _)| name.as_slice())
        .collect();
    let inherited = xattrs(file)?;
    for (name, _) in &inherited {
        if !source_names.contains(name.as_slice()) {
            remove_xattr(file, name)?;
        }
    }
    for (name, value) in &source.xattrs {
        // Protected labels such as security.selinux are commonly inherited
        // correctly but cannot be rewritten by the file owner. Avoid a
        // needless privileged syscall when the candidate already matches.
        if !inherited
            .iter()
            .any(|(current_name, current_value)| current_name == name && current_value == value)
        {
            set_xattr(file, name, value)?;
        }
    }
    set_acl(file, source.acl.as_deref())?;
    if unsafe { libc::fchmod(file.as_raw_fd(), source.metadata.mode() as libc::mode_t) } == -1 {
        return Err(std::io::Error::last_os_error())
            .context("Could not preserve remote file permissions");
    }
    Ok(())
}

fn snapshot(file: &File) -> Result<Snapshot> {
    let before = file.metadata()?;
    ensure!(before.is_file(), "Not a regular file");
    ensure!(
        before.len() <= MAX_FILE_BYTES,
        "Remote text files over 1 MiB are not supported yet"
    );
    #[cfg(target_os = "macos")]
    ensure!(
        mac_file_flags(file)? == 0,
        "Files with BSD flags are not safely editable"
    );
    let xattrs_before = xattrs(file)?;
    let acl_before = acl(file)?;
    let mut reader = file.try_clone()?;
    reader.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::with_capacity(before.len() as usize);
    reader.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_FILE_BYTES,
        "Remote file grew beyond 1 MiB"
    );
    ensure!(
        !bytes.contains(&0),
        "Binary files are not supported in the text editor"
    );
    let after = file.metadata()?;
    let xattrs_after = xattrs(file)?;
    let acl_after = acl(file)?;
    ensure!(
        stable_metadata(&before, &after)
            && xattrs_before == xattrs_after
            && acl_before == acl_after,
        "Remote file changed while it was being read"
    );
    let revision = document_revision(&bytes, &after, &xattrs_after, acl_after.as_deref());
    Ok(Snapshot {
        bytes,
        metadata: after,
        xattrs: xattrs_after,
        acl: acl_after,
        revision,
    })
}

fn document_revision(
    bytes: &[u8],
    metadata: &fs::Metadata,
    xattrs: &[(Vec<u8>, Vec<u8>)],
    acl: Option<&[u8]>,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"pi-remote-file-v3\0");
    digest.update(metadata.dev().to_le_bytes());
    digest.update(metadata.ino().to_le_bytes());
    digest.update(metadata.len().to_le_bytes());
    digest.update(metadata.mtime().to_le_bytes());
    digest.update(metadata.mtime_nsec().to_le_bytes());
    digest.update(metadata.mode().to_le_bytes());
    digest.update(metadata.uid().to_le_bytes());
    digest.update(metadata.gid().to_le_bytes());
    digest.update(bytes);
    for (name, value) in xattrs {
        digest.update((name.len() as u64).to_le_bytes());
        digest.update(name);
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value);
    }
    if let Some(acl) = acl {
        digest.update((acl.len() as u64).to_le_bytes());
        digest.update(acl);
    }
    format!("{:x}", digest.finalize())
}

fn stable_metadata(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    same_inode(left, right)
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
        && left.mode() == right.mode()
        && left.uid() == right.uid()
        && left.gid() == right.gid()
}

fn same_inode(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.dev() == right.dev() && left.ino() == right.ino()
}

fn metadata_equivalent(left: &Snapshot, right: &Snapshot) -> bool {
    left.metadata.mode() == right.metadata.mode()
        && left.metadata.uid() == right.metadata.uid()
        && left.metadata.gid() == right.metadata.gid()
        && left.xattrs == right.xattrs
        && left.acl == right.acl
}

fn read_named_bounded(directory: &File, name: &str, limit: u64) -> Result<Vec<u8>> {
    let file = open_at(
        directory,
        &c(name),
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        0,
    )?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= limit,
        "Invalid bounded file"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "Bounded file grew while reading"
    );
    Ok(bytes)
}

fn open_at(directory: &File, name: &CStr, flags: i32, mode: u32) -> Result<File> {
    let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags, mode) };
    if fd == -1 {
        Err(std::io::Error::last_os_error()).context("Descriptor-relative open failed")
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

fn create_at(directory: &File, name: &CStr, mode: u32) -> Result<File> {
    open_at(
        directory,
        name,
        libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        mode,
    )
}

fn stat_at(directory: &File, name: &CStr) -> Result<libc::stat> {
    let mut stat = std::mem::MaybeUninit::uninit();
    if unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } == -1
    {
        Err(std::io::Error::last_os_error()).context("Descriptor-relative metadata failed")
    } else {
        Ok(unsafe { stat.assume_init() })
    }
}

fn directory_names(directory: &File, limit: usize) -> Result<(Vec<std::ffi::OsString>, bool)> {
    // dup(2) shares a directory cursor with the retained descriptor. Open a
    // fresh description for each traversal so repeated list/recovery requests
    // do not inherit EOF from an earlier readdir(3).
    let dot = c(".");
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            dot.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd == -1 {
        return Err(std::io::Error::last_os_error()).context("Could not reopen directory handle");
    }
    let stream = unsafe { libc::fdopendir(fd) };
    if stream.is_null() {
        unsafe { libc::close(fd) };
        return Err(std::io::Error::last_os_error()).context("Could not read directory handle");
    }
    let mut names = Vec::new();
    loop {
        set_errno(0);
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            let error = errno();
            unsafe { libc::closedir(stream) };
            if error != 0 {
                return Err(std::io::Error::from_raw_os_error(error))
                    .context("Could not enumerate directory");
            }
            return Ok((names, false));
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if name != b"." && name != b".." {
            if names.len() == limit {
                unsafe { libc::closedir(stream) };
                return Ok((names, true));
            }
            names.push(OsStr::from_bytes(name).to_owned());
        }
    }
}

fn c(value: &str) -> CString {
    CString::new(value).expect("validated internal or UTF-8 path component")
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn exchange_at(from_dir: &File, from: &CStr, to_dir: &File, to: &CStr) -> Result<()> {
    let status = unsafe {
        libc::renameat2(
            from_dir.as_raw_fd(),
            from.as_ptr(),
            to_dir.as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    if status == -1 {
        Err(std::io::Error::last_os_error()).context("Atomic file exchange failed")
    } else {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn exchange_at(from_dir: &File, from: &CStr, to_dir: &File, to: &CStr) -> Result<()> {
    let status = unsafe {
        libc::renameatx_np(
            from_dir.as_raw_fd(),
            from.as_ptr(),
            to_dir.as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_SWAP,
        )
    };
    if status == -1 {
        Err(std::io::Error::last_os_error()).context("Atomic file exchange failed")
    } else {
        Ok(())
    }
}

fn xattrs(file: &File) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let size = list_xattrs(file, std::ptr::null_mut(), 0)?;
    ensure!(
        size <= MAX_XATTR_BYTES,
        "File has too much extended metadata"
    );
    let mut names = vec![0u8; size];
    let read = list_xattrs(file, names.as_mut_ptr().cast(), names.len())?;
    ensure!(
        read <= names.len(),
        "Extended metadata changed while listing"
    );
    names.truncate(read);
    let mut result = Vec::new();
    let mut total = names.len();
    for name in names
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        ensure!(
            result.len() < MAX_XATTR_NAMES,
            "File has too many extended attributes"
        );
        let name_c = CString::new(name)?;
        let size = get_xattr(file, &name_c, std::ptr::null_mut(), 0)?;
        total = total
            .checked_add(size)
            .context("Extended metadata size overflow")?;
        ensure!(
            total <= MAX_XATTR_BYTES,
            "File has too much extended metadata"
        );
        let mut value = vec![0u8; size];
        let read = get_xattr(file, &name_c, value.as_mut_ptr().cast(), value.len())?;
        ensure!(
            read <= value.len(),
            "Extended attribute changed while reading"
        );
        value.truncate(read);
        result.push((name.to_vec(), value));
    }
    result.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(result)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn list_xattrs(file: &File, buffer: *mut libc::c_char, size: usize) -> Result<usize> {
    let result = unsafe { libc::flistxattr(file.as_raw_fd(), buffer, size) };
    syscall_size(result, "Could not list remote file extended attributes")
}

#[cfg(target_os = "macos")]
fn list_xattrs(file: &File, buffer: *mut libc::c_char, size: usize) -> Result<usize> {
    let result = unsafe { libc::flistxattr(file.as_raw_fd(), buffer, size, 0) };
    syscall_size(result, "Could not list remote file extended attributes")
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn get_xattr(file: &File, name: &CStr, buffer: *mut libc::c_void, size: usize) -> Result<usize> {
    let result = unsafe { libc::fgetxattr(file.as_raw_fd(), name.as_ptr(), buffer, size) };
    syscall_size(result, "Could not read remote file extended attribute")
}

#[cfg(target_os = "macos")]
fn get_xattr(file: &File, name: &CStr, buffer: *mut libc::c_void, size: usize) -> Result<usize> {
    let result = unsafe { libc::fgetxattr(file.as_raw_fd(), name.as_ptr(), buffer, size, 0, 0) };
    syscall_size(result, "Could not read remote file extended attribute")
}

fn syscall_size(result: isize, context: &'static str) -> Result<usize> {
    if result == -1 {
        Err(std::io::Error::last_os_error()).context(context)
    } else {
        usize::try_from(result).context("System metadata size was invalid")
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn set_xattr(file: &File, name: &[u8], value: &[u8]) -> Result<()> {
    let name = CString::new(name)?;
    let status = unsafe {
        libc::fsetxattr(
            file.as_raw_fd(),
            name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    syscall_status(status, "Could not preserve remote file extended attribute")
}

#[cfg(target_os = "macos")]
fn set_xattr(file: &File, name: &[u8], value: &[u8]) -> Result<()> {
    let name = CString::new(name)?;
    let status = unsafe {
        libc::fsetxattr(
            file.as_raw_fd(),
            name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
            0,
        )
    };
    syscall_status(status, "Could not preserve remote file extended attribute")
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn remove_xattr(file: &File, name: &[u8]) -> Result<()> {
    let name = CString::new(name)?;
    syscall_status(
        unsafe { libc::fremovexattr(file.as_raw_fd(), name.as_ptr()) },
        "Could not clear inherited extended attribute",
    )
}

#[cfg(target_os = "macos")]
fn remove_xattr(file: &File, name: &[u8]) -> Result<()> {
    let name = CString::new(name)?;
    syscall_status(
        unsafe { libc::fremovexattr(file.as_raw_fd(), name.as_ptr(), 0) },
        "Could not clear inherited extended attribute",
    )
}

fn syscall_status(status: i32, context: &'static str) -> Result<()> {
    if status == -1 {
        Err(std::io::Error::last_os_error()).context(context)
    } else {
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn acl(_: &File) -> Result<Option<Vec<u8>>> {
    // Linux exposes POSIX ACLs as system.posix_acl_access in the exact xattr
    // snapshot above.
    Ok(None)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn set_acl(_: &File, _: Option<&[u8]>) -> Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn acl(file: &File) -> Result<Option<Vec<u8>>> {
    const ACL_TYPE_EXTENDED: i32 = 0x0000_0100;
    unsafe {
        let value = acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED);
        if value.is_null() {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ENOENT) {
                return Ok(None);
            }
            return Err(error).context("Could not read remote file ACL");
        }
        let mut length = 0isize;
        let text = acl_to_text(value, &mut length);
        let free_acl = acl_free(value);
        if text.is_null() || free_acl == -1 {
            return Err(std::io::Error::last_os_error()).context("Could not serialize remote ACL");
        }
        let bytes = std::slice::from_raw_parts(text.cast::<u8>(), length as usize).to_vec();
        if acl_free(text.cast()) == -1 {
            return Err(std::io::Error::last_os_error()).context("Could not release remote ACL");
        }
        Ok(Some(bytes))
    }
}

#[cfg(target_os = "macos")]
fn set_acl(file: &File, acl_text: Option<&[u8]>) -> Result<()> {
    const ACL_TYPE_EXTENDED: i32 = 0x0000_0100;
    unsafe {
        match acl_text {
            Some(text) => {
                let text = CString::new(text)?;
                let value = acl_from_text(text.as_ptr());
                if value.is_null() {
                    return Err(std::io::Error::last_os_error())
                        .context("Could not parse remote file ACL");
                }
                let status = acl_set_fd_np(file.as_raw_fd(), value, ACL_TYPE_EXTENDED);
                let free = acl_free(value);
                if status == -1 || free == -1 {
                    return Err(std::io::Error::last_os_error())
                        .context("Could not preserve remote file ACL");
                }
            }
            None => {
                let status = acl_delete_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED);
                if status == -1
                    && std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOENT)
                {
                    return Err(std::io::Error::last_os_error())
                        .context("Could not clear inherited remote file ACL");
                }
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn mac_file_flags(file: &File) -> Result<u32> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(file.as_raw_fd(), stat.as_mut_ptr()) } == -1 {
        return Err(std::io::Error::last_os_error()).context("Could not inspect BSD file flags");
    }
    Ok(unsafe { stat.assume_init() }.st_flags)
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn acl_get_fd_np(fd: libc::c_int, acl_type: libc::c_int) -> *mut libc::c_void;
    fn acl_set_fd_np(fd: libc::c_int, acl: *mut libc::c_void, acl_type: libc::c_int)
    -> libc::c_int;
    fn acl_delete_fd_np(fd: libc::c_int, acl_type: libc::c_int) -> libc::c_int;
    fn acl_to_text(acl: *mut libc::c_void, len: *mut isize) -> *mut libc::c_char;
    fn acl_from_text(text: *const libc::c_char) -> *mut libc::c_void;
    fn acl_free(value: *mut libc::c_void) -> libc::c_int;
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn errno() -> i32 {
    unsafe { *libc::__errno_location() }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn set_errno(value: i32) {
    unsafe { *libc::__errno_location() = value }
}

#[cfg(target_os = "macos")]
fn errno() -> i32 {
    unsafe { *libc::__error() }
}

#[cfg(target_os = "macos")]
fn set_errno(value: i32) {
    unsafe { *libc::__error() = value }
}
