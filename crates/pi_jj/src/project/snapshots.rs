//! The files before and after one command, so what it changed can be put back.
use jj_lib::matchers::FilesMatcher;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::rewrite::restore_tree;

use super::*;

/// The files at one moment: the working-copy commit a snapshot recorded. It
/// stays readable after later operations rewrite or hide it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot(CommitId);

impl Project {
    /// Records the files on disk and returns them. Writes an operation only if
    /// they changed since jj last looked.
    pub fn take_snapshot(&mut self) -> Result<Snapshot> {
        self.snapshot()?;
        let wc = self
            .wc_commit()?
            .context("no working-copy commit in this workspace")?;
        Ok(Snapshot(wc.id().clone()))
    }

    /// The files that changed since `before`, and a snapshot of now.
    pub fn changed_since(&mut self, before: &Snapshot) -> Result<(Snapshot, Vec<FileChange>)> {
        let now = self.take_snapshot()?;
        let store = self.repo.store();
        let from = store.get_commit(&before.0)?.tree();
        let to = store.get_commit(&now.0)?.tree();
        let files = diff::file_changes(store, &from, &to)?;
        Ok((now, files))
    }

    /// Puts `paths` in the working copy back as they were at `snapshot`, as
    /// `jj restore --from` does. The result is an edit in the working copy, like
    /// one of yours. Returns the operation.
    pub fn restore_paths(&mut self, snapshot: &Snapshot, paths: &[String]) -> Result<OperationId> {
        let _lock = self.lock_git()?;
        self.snapshot_locked()?;
        let wc = self
            .wc_commit()?
            .context("no working-copy commit in this workspace")?;
        let source = self.repo.store().get_commit(&snapshot.0)?.tree();
        let paths = paths
            .iter()
            .map(|path| RepoPathBuf::from_internal_string(path.as_str()))
            .collect::<Result<Vec<_>, _>>()?;
        let tree = restore_tree(
            &source,
            &wc.tree(),
            "before the command".into(),
            "now".into(),
            &FilesMatcher::new(&paths),
        )
        .block_on()?;
        let mut tx = self.start_transaction();
        let restored = tx
            .repo_mut()
            .rewrite_commit(&wc)
            .set_tree(tree)
            .write()
            .block_on()?;
        tx.repo_mut().rebase_descendants().block_on()?;
        tx.repo_mut()
            .edit(self.workspace_name(), &restored)
            .block_on()?;
        self.finish(tx, "pi: restore files to before a command")?;
        Ok(self.repo.op_id().clone())
    }
}
