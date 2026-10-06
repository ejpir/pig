//! Read-only, one-level directory browsing before a project/session is chosen.

use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{fs, path::Path};

const MAX_ENTRIES: usize = 512;
const MAX_SCANNED: usize = 8192;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Directory {
    version: u32,
    path: String,
    parent: Option<String>,
    entries: Vec<Entry>,
    truncated: bool,
}

#[derive(Debug, Serialize)]
struct Entry {
    name: String,
    path: String,
    project: bool,
    /// What makes it a project: "git", "jj" or "package".
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<&'static str>,
}

fn list(path: &Path, show_hidden: bool) -> Result<Directory> {
    ensure!(path.is_absolute(), "Choose an absolute folder path");
    let path = fs::canonicalize(path).context("This folder does not exist or is inaccessible")?;
    ensure!(path.is_dir(), "Choose a folder, not a file");
    let name = path.to_str().context("This folder's path is not UTF-8")?;
    ensure!(
        !name.contains(['\0', '\n', '\r']),
        "Unsupported folder path"
    );
    let mut result = Directory {
        version: 1,
        path: name.into(),
        parent: path.parent().and_then(Path::to_str).map(str::to_owned),
        entries: Vec::new(),
        truncated: false,
    };
    // No recursive scan and no file contents: only the requested directory's
    // children, bounded even for enormous build/cache folders.
    for (scanned, entry) in fs::read_dir(&path)
        .context("Cannot read this folder")?
        .enumerate()
    {
        if scanned == MAX_SCANNED || result.entries.len() == MAX_ENTRIES {
            result.truncated = true;
            break;
        }
        let Ok(entry) = entry else { continue };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if (!show_hidden && name.starts_with('.')) || name.contains(['\0', '\n', '\r']) {
            continue;
        }
        // Directory symlinks may be chosen explicitly. Navigating one resolves
        // its canonical location; this is the SSH account's browser, not a jail.
        if !entry.path().is_dir() {
            continue;
        }
        let entry_path = entry.path();
        let kind = [
            (".git", "git"),
            (".jj", "jj"),
            ("Cargo.toml", "package"),
            ("package.json", "package"),
            ("pyproject.toml", "package"),
        ]
        .into_iter()
        .find(|(marker, _)| entry_path.join(marker).exists())
        .map(|(_, kind)| kind);
        result.entries.push(Entry {
            name,
            path: entry_path
                .to_str()
                .context("Unsupported folder path")?
                .into(),
            project: kind.is_some(),
            kind,
        });
    }
    result.entries.sort_by(|a, b| {
        b.project
            .cmp(&a.project)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(result)
}

pub fn print(path: &str, show_hidden: bool) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string(&list(Path::new(path), show_hidden)?)?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_folders_without_reading_files_and_handles_real_names() {
        let root = tempfile::tempdir().unwrap();
        for name in ["Z notes", "café's project", ".private"] {
            fs::create_dir(root.path().join(name)).unwrap();
        }
        fs::write(root.path().join("not a directory"), "private contents").unwrap();
        fs::write(root.path().join("café's project/Cargo.toml"), "[package]").unwrap();
        let listed = list(root.path(), false).unwrap();
        assert_eq!(listed.entries.len(), 2);
        assert_eq!(listed.entries[0].name, "café's project");
        assert!(listed.entries[0].project);
        assert_eq!(listed.entries[0].kind, Some("package"));
        assert_eq!(list(root.path(), true).unwrap().entries.len(), 3);
        assert_eq!(
            Path::new(&listed.path),
            fs::canonicalize(root.path()).unwrap()
        );
        assert!(list(&root.path().join("not a directory"), false).is_err());
        assert!(list(&root.path().join("missing"), false).is_err());
        assert!(list(Path::new("relative"), false).is_err());
    }

    #[test]
    fn huge_folders_are_bounded_and_report_truncation() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..MAX_ENTRIES + 4 {
            fs::create_dir(root.path().join(format!("project-{index:04}"))).unwrap();
        }
        let listed = list(root.path(), false).unwrap();
        assert_eq!(listed.entries.len(), MAX_ENTRIES);
        assert!(listed.truncated);
    }

    #[cfg(unix)]
    #[test]
    fn navigating_a_symlink_resolves_its_location_without_recursing() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("actual")).unwrap();
        std::os::unix::fs::symlink(root.path().join("actual"), root.path().join("linked")).unwrap();
        std::os::unix::fs::symlink(root.path().join("missing"), root.path().join("broken"))
            .unwrap();
        assert_eq!(list(root.path(), false).unwrap().entries.len(), 2);
        let through_link = list(&root.path().join("linked"), false).unwrap();
        assert_eq!(
            through_link.path,
            fs::canonicalize(root.path().join("actual"))
                .unwrap()
                .to_str()
                .unwrap()
        );
    }
}
