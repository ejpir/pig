//! Undoing part of history: one file of a turn, several turns in one operation,
//! or a turn whose later changes then keep jj's conflict.
use std::collections::HashSet;

use jj_lib::matchers::FilesMatcher;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::rewrite::{RebaseOptions, RebasedCommit, restore_tree};

use super::*;

/// An undo that kept conflicts: the files in the working copy with conflict markers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeptConflicts {
    pub operation: OperationId,
    pub files: Vec<String>,
}

impl Project {
    /// Puts one file of a turn back as it was before the turn; later changes are
    /// rebased. Refused with [`UndoConflict`], writing nothing, when a later
    /// change edits the file too. Returns the operation, for the operation log.
    pub fn restore_file(&mut self, change_id: &ChangeId, path: &str) -> Result<OperationId> {
        let _lock = self.lock_git()?;
        self.snapshot_locked()?;
        let commit = self
            .visible_commit(change_id)?
            .context("the turn's change is gone")?;
        let path = RepoPathBuf::from_internal_string(path)?;
        let before = commit.parent_tree(self.repo.as_ref()).block_on()?;
        let tree = restore_tree(
            &before,
            &commit.tree(),
            "before the turn".into(),
            "the turn".into(),
            &FilesMatcher::new([&path]),
        )
        .block_on()?;
        let wc = self.wc_commit()?;
        let mut tx = self.start_transaction();
        tx.repo_mut()
            .rewrite_commit(&commit)
            .set_tree(tree)
            .write()
            .block_on()?;
        rebase_descendants(&mut tx, wc.as_ref(), true)?;
        self.finish(
            tx,
            &format!(
                "pi: restore {} in turn {}",
                path.as_internal_file_string(),
                short_change_id(change_id)
            ),
        )?;
        Ok(self.repo.op_id().clone())
    }

    /// Undoes several turns in one operation, which one [`Self::redo`] brings back.
    /// Refused like [`Self::undo_turn`].
    pub fn undo_turns(&mut self, change_ids: &[ChangeId]) -> Result<OperationId> {
        self.abandon(change_ids, false)
    }

    /// Undoes a turn although later changes build on it: they keep jj's
    /// conflict, so nothing is lost, and the files involved get conflict markers.
    pub fn undo_turn_keeping_conflicts(&mut self, change_id: &ChangeId) -> Result<KeptConflicts> {
        let operation = self.abandon(std::slice::from_ref(change_id), true)?;
        let files = match self.wc_commit()? {
            Some(wc) => wc
                .tree()
                .conflicts()
                .map(|(path, _)| path.as_internal_file_string().to_owned())
                .collect(),
            None => Vec::new(),
        };
        Ok(KeptConflicts { operation, files })
    }

    /// Abandons turns' changes: their edits leave the files, and later changes
    /// are rebased onto their parents. Returns the operation to pass to redo.
    pub(super) fn abandon(
        &mut self,
        change_ids: &[ChangeId],
        keep_conflicts: bool,
    ) -> Result<OperationId> {
        let _lock = self.lock_git()?;
        self.snapshot_locked()?;
        let commits = change_ids
            .iter()
            .map(|id| {
                self.visible_commit(id)?
                    .context("the turn's change is gone")
            })
            .collect::<Result<Vec<_>>>()?;
        let wc = self.wc_commit()?;
        let mut tx = self.start_transaction();
        for commit in &commits {
            tx.repo_mut().record_abandoned_commit(commit);
        }
        rebase_descendants(&mut tx, wc.as_ref(), !keep_conflicts)?;
        let names: Vec<_> = change_ids.iter().map(short_change_id).collect();
        let what = if names.len() == 1 { "turn" } else { "turns" };
        self.finish(tx, &format!("pi: undo {what} {}", names.join(", ")))?;
        Ok(self.repo.op_id().clone())
    }
}

/// Rebases what a rewrite or abandon left behind. With `refuse`, fails with
/// [`UndoConflict`], writing nothing, when a change's own edits now conflict:
/// rebasing it would put conflict markers in files.
fn rebase_descendants(tx: &mut Transaction, wc: Option<&Commit>, refuse: bool) -> Result<()> {
    let mut conflicted = Vec::new();
    tx.repo_mut()
        .rebase_descendants_with_options(
            &RevsetExpression::none(),
            &RebaseOptions::default(),
            |old, new| {
                if let RebasedCommit::Rewritten(new) = new
                    && new.has_conflict()
                    && !old.has_conflict()
                {
                    conflicted.push((old, new));
                }
            },
        )
        .block_on()?;
    if conflicted.is_empty() || !refuse {
        return Ok(());
    }
    // Only changes whose own edits conflict; their descendants merely inherit
    // the conflict.
    let conflicted_ids: HashSet<_> = conflicted.iter().map(|(_, new)| new.id().clone()).collect();
    let causes: Vec<_> = conflicted
        .iter()
        .filter(|(_, new)| {
            !new.parent_ids()
                .iter()
                .any(|id| conflicted_ids.contains(id))
        })
        .map(|(old, _)| old)
        .collect();
    let wc_id = wc.map(Commit::id);
    let working_copy = causes.iter().any(|c| Some(c.id()) == wc_id);
    let later = causes
        .iter()
        .filter(|c| Some(c.id()) != wc_id)
        .map(|c| c.change_id().clone())
        .collect();
    Err(UndoConflict {
        later,
        working_copy,
    }
    .into())
}
