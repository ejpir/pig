//! The operation log, as `jj op log` shows it and `jj op restore` uses it.
use super::*;

/// Who or what made an operation, from its description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    /// Recorded, undone or restored by pi-desktop (`pi: …`).
    Pi,
    /// Files on disk recorded: edits by pi's tools, commands or you.
    Snapshot,
    /// A git commit or checkout made outside jj.
    Git,
    /// Anything else, such as the jj CLI.
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationInfo {
    pub id: OperationId,
    /// When it finished, in milliseconds since the Unix epoch.
    pub time: i64,
    pub description: String,
    pub kind: OperationKind,
}

impl Project {
    /// The newest operations first, following first parents, at most `limit`.
    /// The root operation is not listed.
    pub fn operations(&mut self, limit: usize) -> Result<Vec<OperationInfo>> {
        self.repo = self.repo.reload_at_head().block_on()?;
        let mut operation = self.repo.operation().clone();
        let mut list = Vec::new();
        while list.len() < limit {
            let Some(parent) = operation.parents().block_on()?.into_iter().next() else {
                break;
            };
            let metadata = operation.metadata();
            let description = metadata.description.clone();
            let kind = if description.starts_with("pi: ") {
                OperationKind::Pi
            } else if metadata.is_snapshot || description == "snapshot working copy" {
                OperationKind::Snapshot
            } else if description.starts_with("import git") {
                OperationKind::Git
            } else {
                OperationKind::Other
            };
            list.push(OperationInfo {
                id: operation.id().clone(),
                time: metadata.time.end.timestamp.0,
                description,
                kind,
            });
            operation = parent;
        }
        Ok(list)
    }

    /// What restoring to an operation would do to this workspace's files: from
    /// the files now to the files then.
    pub fn operation_files(&self, operation: &OperationId) -> Result<Vec<FileChange>> {
        let operation = self.repo.loader().load_operation(operation).block_on()?;
        let view = operation.view().block_on()?;
        let Some(then) = view.get_wc_commit_id(self.workspace.workspace_name()) else {
            return Ok(Vec::new());
        };
        let then = self.repo.store().get_commit(then)?;
        let now = self
            .wc_commit()?
            .context("no working-copy commit in this workspace")?;
        diff::file_changes(self.repo.store(), &now.tree(), &then.tree())
    }

    /// `jj op restore`: the project, its files included, as it was after
    /// `operation`. Files on disk are recorded first, and restoring is an
    /// operation too, so it can be restored away again.
    pub fn restore_operation(&mut self, operation: &OperationId) -> Result<OperationId> {
        let _lock = self.lock_git()?;
        self.snapshot_locked()?;
        let target = self.repo.loader().load_operation(operation).block_on()?;
        let mut tx = self.start_transaction();
        let view = restored_view(
            target.view().block_on()?.store_view(),
            tx.base_repo().view().store_view(),
        );
        tx.repo_mut().set_view(view);
        let short = &operation.hex()[..12];
        self.finish(tx, &format!("pi: restore to operation {short}"))?;
        Ok(self.repo.op_id().clone())
    }
}
