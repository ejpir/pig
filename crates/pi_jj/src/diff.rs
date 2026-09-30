//! File-level diffs between two trees, with unified hunks for the Changes view.

use std::sync::Arc;

use anyhow::Result;
use bstr::BStr;
use futures::StreamExt as _;
use jj_lib::conflict_labels::ConflictLabels;
use jj_lib::conflicts::{MaterializedTreeValue, materialized_diff_stream};
use jj_lib::copies::CopyRecords;
use jj_lib::diff_presentation::LineCompareMode;
use jj_lib::diff_presentation::unified::{DiffLineType, unified_diff_hunks};
use jj_lib::matchers::EverythingMatcher;
use jj_lib::merge::Diff;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo_path::RepoPath;
use jj_lib::store::Store;
use pollster::FutureExt as _;

/// Lines of context around each change, as in `git diff`.
const CONTEXT_LINES: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    /// 1-based first line in the old file.
    pub old_start: usize,
    /// 1-based first line in the new file.
    pub new_start: usize,
    /// Lines without their trailing newline.
    pub lines: Vec<(LineKind, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    /// Slash-separated, relative to the workspace root.
    pub path: String,
    pub status: FileStatus,
    pub added: usize,
    pub removed: usize,
    /// Binary files have counts of zero and no hunks.
    pub binary: bool,
    pub hunks: Vec<Hunk>,
}

pub fn file_changes(
    store: &Arc<Store>,
    from: &MergedTree,
    to: &MergedTree,
) -> Result<Vec<FileChange>> {
    let copy_records = CopyRecords::default();
    let labels = ConflictLabels::unlabeled();
    let entries: Vec<_> = materialized_diff_stream(
        store,
        from.diff_stream_with_copies(to, &EverythingMatcher, &copy_records),
        Diff::new(&labels, &labels),
    )
    .collect()
    .block_on();

    let mut changes = Vec::new();
    for entry in entries {
        let path = entry.path.target();
        let Diff { before, after } = entry.values?;
        let status = match (is_absent(&before), is_absent(&after)) {
            (true, true) => continue,
            (true, false) => FileStatus::Added,
            (false, true) => FileStatus::Deleted,
            (false, false) => FileStatus::Modified,
        };
        let old = contents(path, before)?;
        let new = contents(path, after)?;
        let binary = is_binary(&old) || is_binary(&new);
        let hunks = if binary {
            Vec::new()
        } else {
            hunks(&old, &new)
        };
        let count = |kind| {
            hunks
                .iter()
                .flat_map(|h| &h.lines)
                .filter(|(k, _)| *k == kind)
                .count()
        };
        changes.push(FileChange {
            path: path.as_internal_file_string().to_owned(),
            status,
            added: count(LineKind::Added),
            removed: count(LineKind::Removed),
            binary,
            hunks,
        });
    }
    Ok(changes)
}

fn is_absent(value: &MaterializedTreeValue) -> bool {
    matches!(value, MaterializedTreeValue::Absent)
}

/// File bytes for diffing. Symlinks diff as their target; conflicts, submodules
/// and unreadable entries as empty.
fn contents(path: &RepoPath, value: MaterializedTreeValue) -> Result<Vec<u8>> {
    Ok(match value {
        MaterializedTreeValue::File(mut file) => file.read_all(path).block_on()?,
        MaterializedTreeValue::Symlink { target, .. } => target.into_bytes(),
        _ => Vec::new(),
    })
}

/// Git's heuristic: a NUL byte in the first 8000 bytes.
fn is_binary(contents: &[u8]) -> bool {
    contents[..contents.len().min(8000)].contains(&0)
}

fn hunks(old: &[u8], new: &[u8]) -> Vec<Hunk> {
    unified_diff_hunks(
        Diff::new(BStr::new(old), BStr::new(new)),
        CONTEXT_LINES,
        LineCompareMode::Exact,
    )
    .into_iter()
    .map(|hunk| Hunk {
        old_start: hunk.left_line_range.start + 1,
        new_start: hunk.right_line_range.start + 1,
        lines: hunk
            .lines
            .into_iter()
            .map(|(kind, tokens)| {
                let bytes: Vec<u8> = tokens
                    .iter()
                    .flat_map(|(_, part)| part.iter().copied())
                    .collect();
                let text = String::from_utf8_lossy(&bytes);
                let text = text.strip_suffix('\n').unwrap_or(&text);
                let kind = match kind {
                    DiffLineType::Context => LineKind::Context,
                    DiffLineType::Added => LineKind::Added,
                    DiffLineType::Removed => LineKind::Removed,
                };
                (kind, text.strip_suffix('\r').unwrap_or(text).to_owned())
            })
            .collect(),
    })
    .collect()
}
