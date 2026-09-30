//! A jj workspace driven the way the jj CLI drives one: import from git, snapshot
//! the working copy, run one transaction, export to git, update the files.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, bail};
use jj_lib::backend::{ChangeId, CommitId};
use jj_lib::commit::Commit;
use jj_lib::config::ConfigGetResultExt as _;
use jj_lib::default_backend_factories::{
    default_backend_factories, default_working_copy_factories,
};
use jj_lib::fileset::{self, FilesetAliasesMap, FilesetDiagnostics, FilesetParseContext};
use jj_lib::git::{self, GitImportOptions, GitResetHeadError, GitSettings};
use jj_lib::git_backend::GitRepoAtWorkdirError;
use jj_lib::gitignore::GitIgnoreFile;
use jj_lib::hex_util::encode_reverse_hex;
use jj_lib::lock::FileLock;
use jj_lib::matchers::{EverythingMatcher, Matcher, NothingMatcher};
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::{OperationId, View};
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::{ReadonlyRepo, Repo};
use jj_lib::repo_path::RepoPath;
use jj_lib::revset::{
    RemoteRefSymbolExpression, RevsetExpression, SymbolResolver, SymbolResolverExtension,
};
use jj_lib::settings::{HumanByteSize, UserSettings};
use jj_lib::str_util::StringExpression;
use jj_lib::transaction::Transaction;
use jj_lib::ui_path::RepoPathUiConverter;
use jj_lib::working_copy::{SnapshotOptions, WorkingCopyFreshness};
use jj_lib::workspace::Workspace;
use pollster::FutureExt as _;

use crate::diff::{self, FileChange};
use crate::settings::{self, Env};

mod annotate;
mod oplog;
mod restore;
mod snapshots;
mod text;
mod workspaces;

pub use annotate::LineTurns;
pub use oplog::{OperationInfo, OperationKind};
pub use restore::KeptConflicts;
pub use snapshots::Snapshot;

/// jj's default for `snapshot.max-new-file-size`, set by the CLI, not jj-lib.
const DEFAULT_MAX_NEW_FILE_SIZE: u64 = 1024 * 1024;

/// The version control found at or above a project folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Vcs {
    Jj {
        root: PathBuf,
    },
    /// Git without jj: the app offers `jj git init --colocate` once.
    Git {
        root: PathBuf,
    },
    None,
}

/// Finds the nearest jj or git workspace containing `path`.
pub fn detect(path: &Path) -> Vcs {
    for dir in path.ancestors() {
        if dir.join(".jj").is_dir() {
            return Vcs::Jj {
                root: dir.to_owned(),
            };
        }
        if dir.join(".git").exists() {
            return Vcs::Git {
                root: dir.to_owned(),
            };
        }
    }
    Vcs::None
}

/// A running agent turn. Nothing is written to jj until it ends, and only if it
/// changed files; rewinding the conversation itself is pi's `/tree`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    description: String,
    /// The working-copy commit when the turn started: the files before it.
    base: CommitId,
}

impl Turn {
    pub fn description(&self) -> &str {
        &self.description
    }
}

/// Why a turn cannot be undone on its own: later changes edit the same lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoConflict {
    /// Later changes with edits of their own that depend on the turn.
    pub later: Vec<ChangeId>,
    /// Whether uncommitted edits in the working copy depend on it too.
    pub working_copy: bool,
}

impl std::fmt::Display for UndoConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts: Vec<String> = self.later.iter().map(short_change_id).collect();
        if self.working_copy {
            parts.push("your current edits".to_owned());
        }
        write!(f, "later changes build on it ({})", parts.join(", "))
    }
}

impl std::error::Error for UndoConflict {}

/// A turn as it is now, looked up from the change and commit it was recorded as.
#[derive(Clone, Debug)]
pub struct RecordedTurn {
    /// Still in the project; `false` once undone or abandoned.
    pub visible: bool,
    /// The change's visible commit, else the recorded one.
    pub commit: CommitId,
    pub description: String,
    pub files: Vec<FileChange>,
}

/// jj's display form of a change ID, e.g. `zvtmrpyq`.
pub fn short_change_id(id: &ChangeId) -> String {
    encode_reverse_hex(id.as_bytes())[..8].to_owned()
}

pub struct Project {
    workspace: Workspace,
    repo: Arc<ReadonlyRepo>,
    /// Whether git shares this working copy (`.git` beside `.jj`).
    colocated: bool,
}

impl Project {
    /// Opens the jj workspace at `root` with the user's jj config.
    pub fn open(root: &Path) -> Result<Self> {
        Self::open_with(root, &Env::from_process())
    }

    /// Turns on jj for `root`: colocated with its git repo, or a new git repo when
    /// there is none. This is the explicit opt-in; nothing calls it implicitly.
    pub fn init(root: &Path) -> Result<Self> {
        Self::init_with(root, &Env::from_process())
    }

    pub fn open_with(root: &Path, env: &Env) -> Result<Self> {
        let settings = settings::load(env, Some(root))?;
        let workspace = Workspace::load(
            &settings,
            root,
            &default_backend_factories(),
            &default_working_copy_factories(),
        )?;
        let repo = workspace.repo_loader().load_at_head().block_on()?;
        let colocated = is_colocated(&workspace)?;
        Ok(Self {
            workspace,
            repo,
            colocated,
        })
    }

    pub fn init_with(root: &Path, env: &Env) -> Result<Self> {
        let settings = settings::load(env, None)?;
        let git_dir = root.join(".git");
        let (workspace, repo) = if git_dir.is_file() {
            bail!("Cannot turn on jj inside a git worktree; open the main repository instead.");
        } else if git_dir.is_dir() {
            Workspace::init_external_git(&settings, root, &git_dir).block_on()?
        } else {
            Workspace::init_colocated_git(&settings, root, gix::hash::Kind::Sha1).block_on()?
        };
        // Keeps `.jj` out of `git status`, as `jj git init` does.
        std::fs::write(root.join(".jj/.gitignore"), "/*\n").context("writing .jj/.gitignore")?;
        let colocated = is_colocated(&workspace)?;
        let mut project = Self {
            workspace,
            repo,
            colocated,
        };

        // Import existing branches, then make the working copy match git HEAD.
        let _lock = project.lock_git()?;
        let options = GitImportOptions {
            abandon_unreachable_commits: false,
            record_synthetic_predecessors: false,
            ..project.git_import_options()?
        };
        let mut tx = project.start_transaction();
        git::import_refs(tx.repo_mut(), &options).block_on()?;
        if tx.repo().has_changes() {
            if project.colocated {
                git::export_refs(tx.repo_mut())?;
            }
            project.repo = tx.commit("import git refs").block_on()?;
        }
        project.snapshot_locked()?;
        Ok(project)
    }

    pub fn root(&self) -> &Path {
        self.workspace.workspace_root()
    }

    pub fn is_colocated(&self) -> bool {
        self.colocated
    }

    /// Records what is on disk into the working-copy change, as every jj command
    /// does first. Run it before reading state and after pi or the user edits.
    pub fn snapshot(&mut self) -> Result<()> {
        let _lock = self.lock_git()?;
        self.snapshot_locked()
    }

    /// Marks the start of an agent turn. Only records the files on disk, which
    /// writes an operation only if they changed since jj last looked.
    pub fn begin_turn(&mut self, description: &str) -> Result<Turn> {
        self.snapshot()?;
        let wc = self
            .wc_commit()?
            .context("no working-copy commit in this workspace")?;
        Ok(Turn {
            description: description.to_owned(),
            base: wc.id().clone(),
        })
    }

    /// Ends a turn. When it changed files, its edits become their own change,
    /// described with the prompt, and a new empty working copy goes on top so
    /// later edits stay out of it. Returns that change, or `None` when the turn
    /// changed no files; then nothing is written.
    pub fn end_turn(&mut self, turn: &Turn) -> Result<Option<ChangeId>> {
        let _lock = self.lock_git()?;
        self.snapshot_locked()?;
        let base = self.repo.store().get_commit(&turn.base)?;
        let wc = self
            .wc_commit()?
            .context("no working-copy commit in this workspace")?;
        if wc.tree().tree_ids_and_labels() == base.tree().tree_ids_and_labels() {
            return Ok(None);
        }
        if wc.change_id() != base.change_id() {
            bail!(
                "The working copy moved to another change during the run, so this turn's edits \
                 stay in the working copy instead of a change of their own."
            );
        }

        let mut tx = self.start_transaction();
        // An empty, undescribed starting point becomes the turn itself. Otherwise
        // the work that was on disk before the turn keeps its change underneath.
        let turn_commit = if base.is_discardable(tx.repo()).block_on()? {
            tx.repo_mut()
                .rewrite_commit(&wc)
                .set_description(&turn.description)
                .write()
                .block_on()?
        } else {
            let before = tx
                .repo_mut()
                .rewrite_commit(&wc)
                .set_tree(base.tree())
                .write()
                .block_on()?;
            tx.repo_mut()
                .new_commit(vec![before.id().clone()], wc.tree())
                .set_description(&turn.description)
                .write()
                .block_on()?
        };
        tx.repo_mut().rebase_descendants().block_on()?;
        tx.repo_mut()
            .check_out(self.workspace_name(), &turn_commit)
            .block_on()?;
        self.finish(
            tx,
            &format!(
                "pi: record turn {}",
                short_change_id(turn_commit.change_id())
            ),
        )?;
        Ok(Some(turn_commit.change_id().clone()))
    }

    /// A turn recorded in an earlier run of the app: the change's visible commit,
    /// else the recorded commit, which stays readable after an undo hides it.
    /// Fails when neither exists, for example after garbage collection.
    pub fn recorded_turn(
        &self,
        change_id: &ChangeId,
        commit_id: &CommitId,
    ) -> Result<RecordedTurn> {
        let (commit, visible) = match self.visible_commit(change_id)? {
            Some(commit) => (commit, true),
            None => (self.repo.store().get_commit(commit_id)?, false),
        };
        let before = commit.parent_tree(self.repo.as_ref()).block_on()?;
        Ok(RecordedTurn {
            visible,
            commit: commit.id().clone(),
            description: commit.description().trim_end().to_owned(),
            files: diff::file_changes(self.repo.store(), &before, &commit.tree())?,
        })
    }

    /// The files a turn changed, compared with the state before it.
    pub fn changes(&self, change_id: &ChangeId) -> Result<Vec<FileChange>> {
        let commit = self
            .visible_commit(change_id)?
            .context("the turn's change is gone")?;
        let before = commit.parent_tree(self.repo.as_ref()).block_on()?;
        diff::file_changes(self.repo.store(), &before, &commit.tree())
    }

    /// Abandons a turn's change: its edits leave the files, and later changes are
    /// rebased onto its parent. Returns the operation to pass to [`Self::redo`].
    ///
    /// Refused with [`UndoConflict`], writing nothing, when a later change builds
    /// on this turn's edits: rebasing it would put conflict markers in files.
    pub fn undo_turn(&mut self, change_id: &ChangeId) -> Result<OperationId> {
        self.abandon(std::slice::from_ref(change_id), false)
    }

    /// Restores the state before `undo_op`. Refused once anything happened after
    /// the undo, including file edits, because restoring would discard them.
    pub fn redo(&mut self, undo_op: &OperationId) -> Result<()> {
        let _lock = self.lock_git()?;
        self.snapshot_locked()?;
        if self.repo.op_id() != undo_op {
            bail!("The project changed after the undo, so redo would discard those changes.");
        }
        let undo = self.repo.loader().load_operation(undo_op).block_on()?;
        let [before] = <[_; 1]>::try_from(undo.parents().block_on()?)
            .map_err(|_| anyhow!("the undo operation does not have exactly one parent"))?;
        let mut tx = self.start_transaction();
        let view = restored_view(
            before.view().block_on()?.store_view(),
            tx.base_repo().view().store_view(),
        );
        tx.repo_mut().set_view(view);
        self.finish(tx, "pi: redo turn")
    }

    /// The visible commit of a change, if it has one.
    pub fn visible_commit(&self, change_id: &ChangeId) -> Result<Option<Commit>> {
        let Some(targets) = self.repo.resolve_change_id(change_id).block_on()? else {
            return Ok(None);
        };
        let Some((_, id)) = targets.visible_with_offsets().next() else {
            return Ok(None);
        };
        Ok(Some(self.repo.store().get_commit(id)?))
    }

    /// The current operation; the app can compare it with an undo's to know
    /// whether redo is still possible.
    pub fn operation_id(&self) -> &OperationId {
        self.repo.op_id()
    }

    pub fn wc_commit(&self) -> Result<Option<Commit>> {
        let id = self
            .repo
            .view()
            .get_wc_commit_id(self.workspace.workspace_name());
        Ok(id.map(|id| self.repo.store().get_commit(id)).transpose()?)
    }

    fn workspace_name(&self) -> WorkspaceNameBuf {
        self.workspace.workspace_name().to_owned()
    }

    fn start_transaction(&self) -> Transaction {
        start_transaction(&self.repo, &self.workspace_name())
    }

    /// Serializes git import/export with other jj processes, as the CLI does.
    fn lock_git(&self) -> Result<Option<FileLock>> {
        if !self.colocated {
            return Ok(None);
        }
        let path = self.workspace.repo_path().join("git_import_export.lock");
        Ok(Some(FileLock::lock(path)?))
    }

    fn git_import_options(&self) -> Result<GitImportOptions> {
        let git_settings = GitSettings::from_settings(self.settings())?;
        Ok(GitImportOptions {
            abandon_unreachable_commits: git_settings.abandon_unreachable_commits,
            record_synthetic_predecessors: git_settings.record_synthetic_predecessors,
            // TODO: honour `remotes.<name>.auto-track-bookmarks`.
            remote_auto_track_bookmarks: Default::default(),
        })
    }

    fn settings(&self) -> &UserSettings {
        self.workspace.settings()
    }

    /// The CLI's command start: load the latest operation (another session or
    /// the jj CLI may have written since), import git HEAD, snapshot, import git
    /// refs.
    fn snapshot_locked(&mut self) -> Result<()> {
        self.repo = self.repo.reload_at_head().block_on()?;
        if self.colocated {
            self.import_git_head()?;
        }
        self.snapshot_working_copy()?;
        if self.colocated {
            self.import_git_refs()?;
        }
        Ok(())
    }

    /// Follows a `git checkout` or `git commit` made outside jj.
    fn import_git_head(&mut self) -> Result<()> {
        let name = self.workspace_name();
        let mut tx = self.start_transaction();
        git::import_head(tx.repo_mut(), &name, self.workspace.workspace_root()).block_on()?;
        if !tx.repo().has_changes() {
            return Ok(());
        }
        let new_head = tx.repo().view().git_head(&name).clone();
        if let Some(head_id) = new_head.as_normal() {
            let head = tx.repo().store().get_commit_async(head_id).block_on()?;
            let wc = tx.repo_mut().check_out(name, &head).block_on()?;
            // Git already updated the files; only jj's record of them moves.
            let mut locked = self.workspace.start_working_copy_mutation().block_on()?;
            locked.locked_wc().reset(&wc).block_on()?;
            tx.repo_mut().rebase_descendants().block_on()?;
            self.repo = tx.commit("import git head").block_on()?;
            locked.finish(self.repo.op_id().clone()).block_on()?;
            Ok(())
        } else {
            tx.repo_mut().rebase_descendants().block_on()?;
            self.finish(tx, "import git head")
        }
    }

    fn import_git_refs(&mut self) -> Result<()> {
        let options = self.git_import_options()?;
        let mut tx = self.start_transaction();
        git::import_refs(tx.repo_mut(), &options).block_on()?;
        if !tx.repo().has_changes() {
            return Ok(());
        }
        tx.repo_mut().rebase_descendants().block_on()?;
        self.finish(tx, "import git refs")
    }

    fn snapshot_working_copy(&mut self) -> Result<()> {
        let name = self.workspace_name();
        let root = self.workspace.workspace_root().to_owned();
        let base_ignores = self.base_ignores()?;
        let auto_track = self.auto_track_matcher()?;
        let max_new_file_size = match self
            .settings()
            .get_value_with(
                "snapshot.max-new-file-size",
                TryInto::<HumanByteSize>::try_into,
            )
            .optional()?
        {
            Some(HumanByteSize(0)) => u64::MAX,
            Some(HumanByteSize(size)) => size,
            None => DEFAULT_MAX_NEW_FILE_SIZE,
        };
        let options = SnapshotOptions {
            base_ignores,
            progress: None,
            start_tracking_matcher: auto_track.as_ref(),
            force_tracking_matcher: &NothingMatcher,
            max_new_file_size,
        };

        let mut locked = self.workspace.start_working_copy_mutation().block_on()?;
        let Some(wc_id) = self.repo.view().get_wc_commit_id(&name).cloned() else {
            return Ok(());
        };
        let mut wc = self.repo.store().get_commit(&wc_id)?;
        match WorkingCopyFreshness::check_stale(locked.locked_wc(), &wc, &self.repo).block_on()? {
            WorkingCopyFreshness::Fresh => {}
            WorkingCopyFreshness::Updated(operation) => {
                self.repo = self.repo.reload_at(&operation).block_on()?;
                let Some(id) = self.repo.view().get_wc_commit_id(&name) else {
                    return Ok(());
                };
                wc = self.repo.store().get_commit(id)?;
            }
            WorkingCopyFreshness::WorkingCopyStale => {
                bail!(
                    "The working copy is stale. Run `jj workspace update-stale` in {}.",
                    root.display()
                )
            }
            WorkingCopyFreshness::SiblingOperation => {
                bail!(
                    "The working copy's operation is not in the operation log. Run `jj op integrate`."
                )
            }
        }

        let (new_tree, _stats) = locked.locked_wc().snapshot(&options).block_on()?;
        if new_tree.tree_ids_and_labels() != wc.tree().tree_ids_and_labels() {
            let mut tx = start_transaction(&self.repo, &name);
            tx.set_is_snapshot(true);
            let immutable = is_immutable(tx.repo(), wc.id())?;
            let new_wc = if immutable {
                tx.repo_mut()
                    .new_commit(vec![wc.id().clone()], new_tree.clone())
                    .write()
                    .block_on()?
            } else {
                tx.repo_mut()
                    .rewrite_commit(&wc)
                    .set_tree(new_tree.clone())
                    .write()
                    .block_on()?
            };
            tx.repo_mut()
                .set_wc_commit(name.clone(), new_wc.id().clone())?;
            tx.repo_mut().rebase_descendants().block_on()?;
            if self.colocated {
                if immutable {
                    reset_git_head(tx.repo_mut(), &name, &root, &new_wc)?;
                } else {
                    git::update_intent_to_add(
                        tx.base_repo().as_ref(),
                        &root,
                        &wc.tree(),
                        &new_wc.tree(),
                    )
                    .block_on()?;
                }
                git::export_refs(tx.repo_mut())?;
            }
            self.repo = tx.commit("snapshot working copy").block_on()?;
        }
        locked.finish(self.repo.op_id().clone()).block_on()?;
        Ok(())
    }

    /// The CLI's `finish_transaction`: keep the working-copy commit mutable, move
    /// git HEAD and refs, commit the operation, then update the files on disk.
    fn finish(&mut self, mut tx: Transaction, description: &str) -> Result<()> {
        let name = self.workspace_name();
        let root = self.workspace.workspace_root().to_owned();
        let old_wc = tx
            .base_repo()
            .view()
            .get_wc_commit_id(&name)
            .map(|id| tx.base_repo().store().get_commit(id))
            .transpose()?;
        let mut new_wc = tx
            .repo()
            .view()
            .get_wc_commit_id(&name)
            .map(|id| tx.repo().store().get_commit(id))
            .transpose()?;
        if let Some(wc) = &new_wc
            && is_immutable(tx.repo(), wc.id())?
        {
            let on_top = tx
                .repo_mut()
                .new_commit(vec![wc.id().clone()], wc.tree())
                .write()
                .block_on()?;
            tx.repo_mut()
                .set_wc_commit(name.clone(), on_top.id().clone())?;
            new_wc = Some(on_top);
        }
        if self.colocated {
            if let Some(wc) = &new_wc {
                reset_git_head(tx.repo_mut(), &name, &root, wc)?;
            }
            git::export_refs(tx.repo_mut())?;
        }
        self.repo = tx.commit(description).block_on()?;
        if let Some(new_wc) = &new_wc {
            let old_tree = old_wc.map(|commit| commit.tree());
            self.workspace
                .check_out(self.repo.op_id().clone(), old_tree.as_ref(), new_wc)
                .block_on()
                .with_context(|| format!("checking out {}", new_wc.id().hex()))?;
        }
        Ok(())
    }

    /// Git's global excludes file and `.git/info/exclude`, as the CLI reads them.
    fn base_ignores(&self) -> Result<Arc<GitIgnoreFile>> {
        let mut ignores = GitIgnoreFile::empty();
        let Ok(backend) = git::get_git_backend(self.repo.store()) else {
            return Ok(ignores);
        };
        let excludes = match backend
            .git_repo()
            .config_snapshot()
            .string("core.excludesFile")
        {
            Some(value) => std::str::from_utf8(&value)
                .ok()
                .map(|path| self.root().join(jj_lib::file_util::expand_home_path(path))),
            None => std::env::var_os("XDG_CONFIG_HOME")
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from)
                .or_else(|| etcetera::home_dir().ok().map(|home| home.join(".config")))
                .map(|dir| dir.join("git/ignore")),
        };
        if let Some(path) = excludes {
            ignores = ignores.chain_with_file(RepoPath::root(), path)?;
        }
        Ok(ignores.chain_with_file(
            RepoPath::root(),
            backend.git_repo_path().join("info/exclude"),
        )?)
    }

    /// `snapshot.auto-track`, which decides which new files jj starts tracking.
    fn auto_track_matcher(&self) -> Result<Box<dyn Matcher>> {
        let Some(pattern) = self
            .settings()
            .get_string("snapshot.auto-track")
            .optional()?
        else {
            return Ok(Box::new(EverythingMatcher));
        };
        let root = self.root().to_owned();
        let context = FilesetParseContext {
            aliases_map: &FilesetAliasesMap::new(),
            path_converter: &RepoPathUiConverter::Fs {
                cwd: root.clone(),
                base: root,
            },
        };
        let expression = fileset::parse(&mut FilesetDiagnostics::new(), &pattern, &context)?;
        Ok(expression.to_matcher())
    }
}

fn start_transaction(repo: &Arc<ReadonlyRepo>, name: &WorkspaceNameBuf) -> Transaction {
    let mut tx = repo.start_transaction();
    tx.set_workspace_name(name);
    tx
}

fn is_colocated(workspace: &Workspace) -> Result<bool> {
    let Ok(backend) = git::get_git_backend(workspace.repo_loader().store()) else {
        return Ok(false);
    };
    match backend.open_git_repo_at_workdir(workspace.workspace_root()) {
        Ok(_) => Ok(true),
        Err(GitRepoAtWorkdirError::NotFound { .. } | GitRepoAtWorkdirError::Unrelated { .. }) => {
            Ok(false)
        }
        Err(err) => Err(err.into()),
    }
}

/// Git HEAD follows the working copy's parent. A HEAD moved concurrently by git
/// is left alone; the next snapshot imports it.
fn reset_git_head(
    repo: &mut jj_lib::repo::MutableRepo,
    name: &WorkspaceNameBuf,
    root: &Path,
    wc: &Commit,
) -> Result<()> {
    match git::reset_head(repo, name, root, wc).block_on() {
        Ok(()) | Err(GitResetHeadError::UpdateHeadRef(_)) => Ok(()),
        Err(err) => Err(err.into()),
    }
}

/// Whether jj must not rewrite `id`: the root, anything under a tag, or anything
/// already on a remote. Stricter than the CLI's default `immutable_heads()`,
/// which only counts `trunk()` and untracked remote bookmarks.
fn is_immutable(repo: &dyn Repo, id: &CommitId) -> Result<bool> {
    let expression = immutable_heads()
        .ancestors()
        .intersection(&RevsetExpression::commit(id.clone()));
    let no_extensions: [Box<dyn SymbolResolverExtension>; 0] = [];
    let resolved =
        expression.resolve_user_expression(repo, &SymbolResolver::new(repo, &no_extensions))?;
    Ok(!resolved.evaluate(repo)?.is_empty()?)
}

/// The commits of `expression`, newest first, at most `limit`.
fn commit_ids(
    repo: &dyn Repo,
    expression: &Arc<jj_lib::revset::UserRevsetExpression>,
    limit: usize,
) -> Result<Vec<CommitId>> {
    use futures::StreamExt as _;
    let no_extensions: [Box<dyn SymbolResolverExtension>; 0] = [];
    let resolved =
        expression.resolve_user_expression(repo, &SymbolResolver::new(repo, &no_extensions))?;
    let ids = resolved
        .evaluate(repo)?
        .stream()
        .take(limit)
        .collect::<Vec<_>>()
        .block_on();
    Ok(ids.into_iter().collect::<Result<_, _>>()?)
}

/// The root, tags and anything on a remote: history jj must not rewrite.
fn immutable_heads() -> Arc<jj_lib::revset::UserRevsetExpression> {
    let remote = RemoteRefSymbolExpression {
        name: StringExpression::all(),
        remote: StringExpression::exact(git::REMOTE_NAME_FOR_LOCAL_GIT_REPO.as_str()).negated(),
    };
    RevsetExpression::tags(StringExpression::all())
        .union(&RevsetExpression::remote_bookmarks(remote, None))
        .union(&RevsetExpression::root())
}

/// `jj undo`'s restore: the repo and remote-tracking state from `restored`, git
/// refs and HEAD as they are now (the next export moves them).
fn restored_view(restored: &View, current: &View) -> View {
    View {
        head_ids: restored.head_ids.clone(),
        local_bookmarks: restored.local_bookmarks.clone(),
        local_tags: restored.local_tags.clone(),
        remote_views: restored.remote_views.clone(),
        git_refs: current.git_refs.clone(),
        git_heads: current.git_heads.clone(),
        wc_commit_ids: restored.wc_commit_ids.clone(),
    }
}
