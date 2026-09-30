//! File mutations behind the editor boundary. No-overwrite creation and rename,
//! workspace containment, protected VCS metadata, and system-trash deletion.
use super::*;
use std::path::Component;

#[derive(Clone, Debug)]
pub enum FileAction {
    Create {
        parent: PathBuf,
        name: String,
        directory: bool,
    },
    Rename {
        path: PathBuf,
        name: String,
    },
    Trash {
        path: PathBuf,
    },
}
fn valid_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0', ':']) {
        bail!("Enter a single file or folder name, not a path");
    }
    if name.eq_ignore_ascii_case(".git") || name.eq_ignore_ascii_case(".jj") {
        bail!("Version-control metadata cannot be changed here");
    }
    Ok(())
}
fn contained(root: &Path, path: &Path) -> Result<()> {
    let relative = path
        .strip_prefix(root)
        .context("Path is outside this project")?;
    if relative.as_os_str().is_empty() {
        bail!("The project root cannot be renamed or deleted");
    }
    for part in relative.components() {
        match part {
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                if name.eq_ignore_ascii_case(".git") || name.eq_ignore_ascii_case(".jj") {
                    bail!("Version-control metadata cannot be changed here");
                }
            }
            _ => bail!("Path must stay inside this project"),
        }
    }
    let parent = path
        .parent()
        .context("No parent directory")?
        .canonicalize()?;
    if !parent.starts_with(root) {
        bail!("External symlink directories cannot be changed here");
    }
    Ok(())
}
fn validate(root: &Path, action: &FileAction) -> Result<PathBuf> {
    match action {
        FileAction::Create { parent, name, .. } => {
            valid_name(name)?;
            let path = parent.join(name);
            contained(root, &path)?;
            Ok(path)
        }
        FileAction::Rename { path, name } => {
            contained(root, path)?;
            valid_name(name)?;
            let target = path.with_file_name(name);
            contained(root, &target)?;
            Ok(target)
        }
        FileAction::Trash { path } => {
            contained(root, path)?;
            Ok(path.clone())
        }
    }
}
impl EditorProject {
    pub fn file_action(&self, action: FileAction, cx: &mut App) -> Task<Result<PathBuf>> {
        let root = self.root.clone();
        let project = self.project.clone();
        let fs = cx.global::<Services>().fs.clone();
        let check_action = action.clone();
        let check = cx
            .background_executor()
            .spawn(async move { validate(&root, &check_action) });
        cx.spawn(async move |cx| {
            let destination = check.await?;
            match action {
                FileAction::Create { directory, .. } => {
                    if directory {
                        // create_dir (not create_dir_all) fails if the target already exists.
                        let path = destination.clone();
                        cx.background_spawn(async move { std::fs::create_dir(path) })
                            .await?;
                    } else {
                        // Project::create_entry currently writes/truncates; do not use it
                        // for New File. The shared worktree watcher discovers this entry.
                        fs.create_file(
                            &destination,
                            fs::CreateOptions {
                                overwrite: false,
                                ignore_if_exists: false,
                            },
                        )
                        .await?;
                    }
                }
                FileAction::Rename { path, .. } => {
                    if cx.update(|cx| has_unsaved_buffers(cx)) {
                        bail!("Save or close unsaved buffers before renaming");
                    }
                    project
                        .update(cx, |p, cx| -> Result<_> {
                            let (tree, relative) = p
                                .find_worktree(&path, cx)
                                .context("File is no longer in this project")?;
                            let old = project::ProjectPath {
                                worktree_id: tree.read(cx).id(),
                                path: relative,
                            };
                            let id = p
                                .entry_for_path(&old, cx)
                                .context("File no longer exists")?
                                .id;
                            let (tree, relative) = p
                                .find_worktree(&destination, cx)
                                .context("Destination is outside this project")?;
                            Ok(p.rename_entry(
                                id,
                                project::ProjectPath {
                                    worktree_id: tree.read(cx).id(),
                                    path: relative,
                                },
                                cx,
                            ))
                        })?
                        .await?;
                }
                FileAction::Trash { path } => {
                    if cx.update(|cx| has_unsaved_buffers(cx)) {
                        bail!("Save or close unsaved buffers before deleting");
                    }
                    project
                        .update(cx, |p, cx| -> Result<_> {
                            let (tree, relative) = p
                                .find_worktree(&path, cx)
                                .context("File is no longer in this project")?;
                            let path = project::ProjectPath {
                                worktree_id: tree.read(cx).id(),
                                path: relative,
                            };
                            p.trash_file(path, cx).context("File no longer exists")
                        })?
                        .await?;
                }
            }
            Ok(destination)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_cannot_escape_or_target_vcs_metadata() {
        for name in [
            "",
            ".",
            "..",
            "../secret",
            "a/b",
            "a\\b",
            ".git",
            ".JJ",
            "C:other",
        ] {
            assert!(valid_name(name).is_err(), "{name}");
        }
        for name in ["notes.md", "a folder", "é.ts"] {
            assert!(valid_name(name).is_ok());
        }
    }
}
