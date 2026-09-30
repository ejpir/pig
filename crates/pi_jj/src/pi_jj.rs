//! Jujutsu (jj) for pi-desktop: each agent turn is its own jj change, so any turn
//! can be undone and redone.
//!
//! GPUI-independent. Everything here blocks on jj-lib's async API; callers run it
//! off the UI thread, and only between agent runs (never while pi is writing).

mod diff;
mod project;
mod settings;

pub use diff::{FileChange, FileStatus, Hunk, LineKind};
pub use jj_lib::backend::{ChangeId, CommitId};
/// For `hex()` on ids; parse them back with `ChangeId::try_from_hex` and friends.
pub use jj_lib::object_id::ObjectId;
pub use jj_lib::op_store::OperationId;
pub use project::{
    KeptConflicts, LineTurns, OperationInfo, OperationKind, Project, RecordedTurn, Snapshot, Turn,
    UndoConflict, Vcs, detect, short_change_id,
};
pub use settings::Env;

#[cfg(test)]
mod tests;
