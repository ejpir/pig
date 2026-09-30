//! More workspaces on one repository: a session gets its own folder, and its
//! turns come back into the main folder afterwards (`jj workspace add`, rebase).
use std::collections::HashSet;
use std::path::Path;

use jj_lib::default_backend_factories::default_working_copy_factory;
use jj_lib::rewrite::rebase_commit;

use super::*;

impl Project {
    /// This workspace's name in the repository, such as `default`.
    pub fn workspace_name_text(&self) -> String {
        self.workspace.workspace_name().as_str().to_owned()
    }

    /// Adds a jj workspace at `path` (created if missing, must be empty) that
    /// shares this repository. Its files are as they were just before the change
    /// `before`, such as the first turn after a forked entry; without it, as of
    /// the last recorded turn (this working copy's parent). Returns the new
    /// workspace, open.
    pub fn add_workspace(&mut self, path: &Path, before: Option<&ChangeId>) -> Result<Project> {
        let _lock = self.lock_git()?;
        self.snapshot_locked()?;
        std::fs::create_dir_all(path)?;
        if std::fs::read_dir(path)?.next().is_some() {
            bail!("{} is not empty", path.display());
        }
        let child = match before {
            Some(change) => self
                .visible_commit(change)?
                .context("that change is gone")?,
            None => self
                .wc_commit()?
                .context("no working-copy commit in this workspace")?,
        };
        let parent = child
            .parent_ids()
            .first()
            .context("that change has no parent")?;
        let base = self.repo.store().get_commit(parent)?;
        let name = self.unused_workspace_name(path)?;
        let (workspace, repo) = Workspace::init_workspace_with_existing_repo(
            path,
            self.workspace.repo_path(),
            &self.repo,
            &*default_working_copy_factory(),
            name.clone(),
        )
        .block_on()?;
        let mut tx = start_transaction(&repo, &name);
        let wc = tx.repo_mut().check_out(name.clone(), &base).block_on()?;
        // The workspace starts on the root commit; that working copy is abandoned.
        tx.repo_mut().rebase_descendants().block_on()?;
        let repo = tx
            .commit(format!("pi: add workspace {}", name.as_str()))
            .block_on()?;
        let mut project = Project {
            workspace,
            repo,
            colocated: false,
        };
        project
            .workspace
            .check_out(project.repo.op_id().clone(), None, &wc)
            .block_on()
            .context("writing the workspace's files")?;
        self.repo = self.repo.reload_at_head().block_on()?;
        Ok(project)
    }

    /// Brings another workspace's turns into this one: they are rebased onto this
    /// working copy's parent, and this working copy onto the last of them, so
    /// this folder's files gain their edits. Conflicts, if any, stay in jj and
    /// show as conflict markers. Returns the operation, for undoing it.
    pub fn bring_in(&mut self, workspace: &str) -> Result<OperationId> {
        let _lock = self.lock_git()?;
        self.snapshot_locked()?;
        let other = WorkspaceNameBuf::from(workspace);
        let other_wc = self
            .repo
            .view()
            .get_wc_commit_id(&other)
            .with_context(|| format!("no workspace named {workspace}"))?
            .clone();
        let other_wc = self.repo.store().get_commit(&other_wc)?;
        let last = other_wc
            .parent_ids()
            .first()
            .context("the workspace's working copy has no parent")?
            .clone();
        let wc = self
            .wc_commit()?
            .context("no working-copy commit in this workspace")?;
        let here = wc
            .parent_ids()
            .first()
            .context("the working copy has no parent")?
            .clone();
        // The other workspace's own turns: on its side, not on this one.
        let theirs = RevsetExpression::commit(last.clone())
            .ancestors()
            .minus(&RevsetExpression::commit(here.clone()).ancestors());
        let theirs = commit_ids(self.repo.as_ref(), &theirs, usize::MAX)?;
        if theirs.is_empty() {
            bail!("That workspace has no turns this folder does not have.");
        }
        let mut tx = self.start_transaction();
        // Oldest first: each root of their side moves onto this side's parent;
        // descendants follow.
        let theirs_set: HashSet<_> = theirs.iter().cloned().collect();
        for id in theirs.iter().rev() {
            let commit = tx.repo().store().get_commit(id)?;
            if commit
                .parent_ids()
                .iter()
                .all(|parent| !theirs_set.contains(parent))
            {
                rebase_commit(tx.repo_mut(), commit, vec![here.clone()]).block_on()?;
            }
        }
        tx.repo_mut().rebase_descendants().block_on()?;
        let other_wc = tx
            .repo()
            .view()
            .get_wc_commit_id(&other)
            .context("the workspace went away")?
            .clone();
        let last = tx
            .repo()
            .store()
            .get_commit(&other_wc)?
            .parent_ids()
            .first()
            .context("the workspace's working copy has no parent")?
            .clone();
        let wc = tx
            .repo()
            .view()
            .get_wc_commit_id(self.workspace.workspace_name())
            .context("no working-copy commit in this workspace")?
            .clone();
        let wc = tx.repo().store().get_commit(&wc)?;
        rebase_commit(tx.repo_mut(), wc, vec![last]).block_on()?;
        tx.repo_mut().rebase_descendants().block_on()?;
        self.finish(
            tx,
            &format!("pi: bring in the turns of workspace {workspace}"),
        )?;
        Ok(self.repo.op_id().clone())
    }

    /// A workspace name from the folder's, unique in the repository.
    fn unused_workspace_name(&self, path: &Path) -> Result<WorkspaceNameBuf> {
        let base = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("workspace")
            .to_owned();
        let taken = |name: &str| {
            self.repo
                .view()
                .wc_commit_ids()
                .keys()
                .any(|existing| existing.as_str() == name)
        };
        let mut name = base.clone();
        let mut n = 2;
        while taken(&name) {
            name = format!("{base}-{n}");
            n += 1;
        }
        Ok(WorkspaceNameBuf::from(name))
    }
}
