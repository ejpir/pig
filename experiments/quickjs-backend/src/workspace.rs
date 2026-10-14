//! Descriptor-relative workspace access: no absolute paths, '..', or symlink traversal.
//! This is a narrow file capability, not isolation from another process with the same UID.
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path},
};

pub const MAX_FILE: usize = 256 * 1024;

pub struct Workspace(File);

impl Workspace {
    pub fn open(path: &Path) -> Result<Self> {
        let path = CString::new(path.as_os_str().as_bytes())?;
        // SAFETY: valid C string; own the returned descriptor exactly once.
        let fd = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        ensure!(
            fd >= 0,
            "open workspace: {}",
            std::io::Error::last_os_error()
        );
        Ok(Self(unsafe { File::from_raw_fd(fd) }))
    }

    fn parent(&self, path: &str) -> Result<(File, CString)> {
        ensure!(
            !path.is_empty() && path.len() <= 4096,
            "invalid workspace path"
        );
        let mut parts = Vec::new();
        for part in Path::new(path).components() {
            match part {
                Component::Normal(name) => parts.push(CString::new(name.as_bytes())?),
                _ => bail!("only relative workspace paths without '..' are allowed"),
            }
        }
        let name = parts.pop().context("empty workspace path")?;
        let mut dir = self.0.try_clone()?;
        // Parent directories must already exist. Never resolve symlinks.
        for part in parts {
            dir = open_at(&dir, &part, libc::O_RDONLY | libc::O_DIRECTORY)?;
        }
        Ok((dir, name))
    }

    pub fn read(&self, path: &str) -> Result<String> {
        let (dir, name) = self.parent(path)?;
        let file = open_at(&dir, &name, libc::O_RDONLY | libc::O_NONBLOCK)?;
        check_regular(&file)?;
        let mut text = String::new();
        file.take((MAX_FILE + 1) as u64).read_to_string(&mut text)?;
        ensure!(text.len() <= MAX_FILE, "file exceeds 256 KiB limit");
        Ok(text)
    }

    pub fn write(&self, path: &str, text: &str) -> Result<()> {
        ensure!(text.len() <= MAX_FILE, "file exceeds 256 KiB limit");
        let (dir, name) = self.parent(path)?;
        // Do not truncate until after checking the opened object (including hard links).
        let mut file = open_at(
            &dir,
            &name,
            libc::O_WRONLY | libc::O_CREAT | libc::O_NONBLOCK,
        )?;
        check_regular(&file)?;
        file.set_len(0)?;
        file.write_all(text.as_bytes())?;
        Ok(())
    }
}

fn check_regular(file: &File) -> Result<()> {
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.nlink() == 1,
        "only regular, non-hardlinked files are allowed"
    );
    ensure!(
        metadata.len() <= MAX_FILE as u64,
        "file exceeds 256 KiB limit"
    );
    Ok(())
}

fn open_at(dir: &File, name: &CString, flags: i32) -> Result<File> {
    // SAFETY: live directory fd and valid C string; mode is supplied for O_CREAT.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    ensure!(
        fd >= 0,
        "workspace file access: {}",
        std::io::Error::last_os_error()
    );
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn round_trip_and_bounds() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let ws = Workspace::open(temp.path())?;
        ws.write("hello.txt", "hello 🌍")?;
        assert_eq!(ws.read("hello.txt")?, "hello 🌍");
        ws.write("hello.txt", "short")?;
        assert_eq!(ws.read("hello.txt")?, "short");
        assert!(ws.write("large", &"x".repeat(MAX_FILE + 1)).is_err());
        for path in [
            "../escape",
            "/tmp/escape",
            "a/../../escape",
            "",
            ".",
            "a\0b",
        ] {
            assert!(ws.read(path).is_err(), "read {path}");
            assert!(ws.write(path, "bad").is_err(), "write {path}");
        }
        Ok(())
    }

    #[test]
    fn rejects_symlinks_hardlinks_and_directories() -> Result<()> {
        let root = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        let target = outside.path().join("keep");
        std::fs::write(&target, "unchanged")?;
        symlink(outside.path(), root.path().join("dir"))?;
        symlink(&target, root.path().join("link"))?;
        std::fs::hard_link(&target, root.path().join("hard"))?;
        std::fs::create_dir(root.path().join("folder"))?;
        let ws = Workspace::open(root.path())?;
        for path in ["dir/keep", "link", "hard", "folder"] {
            assert!(ws.read(path).is_err(), "read {path}");
            assert!(ws.write(path, "bad").is_err(), "write {path}");
        }
        assert_eq!(std::fs::read_to_string(target)?, "unchanged");
        Ok(())
    }
}
