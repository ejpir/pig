//! jj for one session: the opt-in offer, the running turn, and the turns that
//! changed files. `pi_jj` blocks, so the controller runs every call on the
//! background executor behind a mutex; views only read the plain values here.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use pi_jj::{ChangeId, FileChange, ObjectId, OperationId, Project, Snapshot, Turn};

#[derive(Default)]
pub struct Jj {
    /// The open jj workspace, once loaded.
    pub project: Option<Arc<Mutex<Project>>>,
    pub root: Option<PathBuf>,
    /// A git-only project to offer jj for. Cleared once turned on or declined.
    pub offer: Option<PathBuf>,
    /// The run in progress, marked before its prompt was sent.
    pub turn: Option<Turn>,
    /// A prompt is waiting for `begin_turn` before it is sent.
    pub starting: bool,
    /// Opening, recording, undoing or redoing is still running.
    pub busy: bool,
    pub records: Vec<TurnRecord>,
    /// Tool ids already present when recording began. Survives later hydration
    /// through the stable ids copied into each completed record.
    pub tool_baseline: HashSet<String>,
    /// The request reading the session file's turn links.
    pub links_request: Option<String>,
    /// Turn links read from the session file, until the jj project is open to
    /// turn them back into records.
    pub saved_links: Option<Vec<super::turn_links::Link>>,
    /// What each `bash` call changed, by tool call id (design study 05, 03).
    pub commands: HashMap<String, CommandFiles>,
}

/// The files before one `bash` call and what it changed, from jj snapshots the
/// pi extension asks for around it. Kept while the session is open.
#[derive(Clone, Debug)]
pub struct CommandFiles {
    pub before: Snapshot,
    /// `None` until the command finished.
    pub files: Option<Vec<FileChange>>,
    pub restored: bool,
}

/// A finished turn that changed files, shown as one line under its last message.
#[derive(Clone, Debug)]
pub struct TurnRecord {
    pub after_message: usize,
    /// Message indexes cease to identify the same rows after history hydration;
    /// `after` finds the row again.
    pub anchored: bool,
    /// The `timestamp` of the message at `after_message`.
    pub after: Option<u64>,
    /// The turn's commit when it was recorded, in hex.
    pub commit: String,
    pub description: String,
    pub change: ChangeId,
    pub short: String,
    pub files: usize,
    pub added: usize,
    pub removed: usize,
    /// Immutable diff returned with the turn snapshot; no repository I/O in rendering.
    pub diff: Vec<FileChange>,
    pub tool_ids: HashSet<String>,
    /// While undone: the operation `redo` restores from.
    pub undone: Option<OperationId>,
}

impl TurnRecord {
    pub fn new(after_message: usize, change: ChangeId, files: &[FileChange]) -> Self {
        Self {
            after_message,
            anchored: true,
            after: None,
            commit: String::new(),
            description: String::new(),
            short: pi_jj::short_change_id(&change),
            change,
            files: files.len(),
            added: files.iter().map(|file| file.added).sum(),
            removed: files.iter().map(|file| file.removed).sum(),
            diff: files.to_vec(),
            tool_ids: HashSet::new(),
            undone: None,
        }
    }

    /// What the session file keeps of this turn.
    pub fn link(&self) -> super::turn_links::Link {
        let mut tools: Vec<String> = self.tool_ids.iter().cloned().collect();
        tools.sort();
        super::turn_links::Link {
            change: self.change.hex(),
            commit: self.commit.clone(),
            after: self.after,
            tools,
            undone: self.undone.as_ref().map(ObjectId::hex),
        }
    }

    pub fn files_label(&self) -> String {
        format!(
            "{} file{}",
            self.files,
            if self.files == 1 { "" } else { "s" }
        )
    }
}
