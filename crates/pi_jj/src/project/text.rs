//! Plain text for pi's read-only jj tools (`jj_log`, `jj_diff`, `jj_show`), in
//! the terms the jj CLI prints. Output is capped so a large change cannot fill
//! the model's context.
use std::fmt::Write as _;

use jj_lib::repo_path::RepoPathBuf;

use super::*;
use crate::diff::{FileStatus, LineKind};

/// Answers longer than this are cut, with a note saying so.
const MAX_TEXT: usize = 48 * 1024;

impl Project {
    /// The changes that are not yet immutable, newest first, from the working
    /// copy down, as `jj log -r 'mutable() & ::@'`: change, files, lines, the
    /// description's first line.
    pub fn log_text(&mut self, limit: usize) -> Result<String> {
        self.snapshot()?;
        let wc = self
            .wc_commit()?
            .context("no working-copy commit in this workspace")?;
        let expression = RevsetExpression::commit(wc.id().clone())
            .ancestors()
            .minus(&immutable_heads().ancestors());
        let repo = self.repo.as_ref();
        let ids = commit_ids(repo, &expression, limit + 1)?;
        let mut text = String::new();
        for (shown, id) in ids.into_iter().enumerate() {
            if shown == limit {
                text.push_str("(older changes not shown)\n");
                break;
            }
            let commit = self.repo.store().get_commit(&id)?;
            let before = commit.parent_tree(repo).block_on()?;
            let files = diff::file_changes(self.repo.store(), &before, &commit.tree())?;
            let added: usize = files.iter().map(|file| file.added).sum();
            let removed: usize = files.iter().map(|file| file.removed).sum();
            let description = commit.description().lines().next().unwrap_or("");
            writeln!(
                text,
                "{}  {:>2} file{} +{added} -{removed}  {}{}",
                short_change_id(commit.change_id()),
                files.len(),
                if files.len() == 1 { " " } else { "s" },
                if id == *wc.id() {
                    "(working copy) "
                } else {
                    ""
                },
                if description.is_empty() {
                    "(no description)"
                } else {
                    description
                },
            )?;
        }
        if text.is_empty() {
            text.push_str("No changes since the immutable history.\n");
        }
        Ok(text)
    }

    /// A change's diff, of every file or one, in unified format. `revision` is
    /// what the jj CLI takes: a change ID (prefix), a commit ID, `@`, a bookmark.
    pub fn diff_text(&mut self, revision: &str, path: Option<&str>) -> Result<String> {
        self.snapshot()?;
        let commit = self.resolve(revision)?;
        let before = commit.parent_tree(self.repo.as_ref()).block_on()?;
        let path = path.map(RepoPathBuf::from_internal_string).transpose()?;
        let files = diff::file_changes(self.repo.store(), &before, &commit.tree())?;
        let mut text = String::new();
        for file in files.iter().filter(|file| {
            path.as_ref()
                .is_none_or(|path| path.as_internal_file_string() == file.path)
        }) {
            let (old, new) = match file.status {
                FileStatus::Added => ("/dev/null".to_owned(), format!("b/{}", file.path)),
                FileStatus::Deleted => (format!("a/{}", file.path), "/dev/null".to_owned()),
                FileStatus::Modified => (format!("a/{}", file.path), format!("b/{}", file.path)),
            };
            writeln!(text, "--- {old}\n+++ {new}")?;
            if file.binary {
                text.push_str("Binary file\n");
            }
            for hunk in &file.hunks {
                let old_lines = hunk
                    .lines
                    .iter()
                    .filter(|(kind, _)| *kind != LineKind::Added)
                    .count();
                let new_lines = hunk
                    .lines
                    .iter()
                    .filter(|(kind, _)| *kind != LineKind::Removed)
                    .count();
                writeln!(
                    text,
                    "@@ -{},{old_lines} +{},{new_lines} @@",
                    hunk.old_start, hunk.new_start
                )?;
                for (kind, line) in &hunk.lines {
                    let mark = match kind {
                        LineKind::Context => ' ',
                        LineKind::Added => '+',
                        LineKind::Removed => '-',
                    };
                    writeln!(text, "{mark}{line}")?;
                }
            }
        }
        if text.is_empty() {
            text = match path {
                Some(_) => "The change does not touch that file.\n".into(),
                None => "The change is empty.\n".into(),
            };
        }
        Ok(cap(text))
    }

    /// A change's full description, author, time and files, as `jj show --stat`.
    pub fn show_text(&mut self, revision: &str) -> Result<String> {
        self.snapshot()?;
        let commit = self.resolve(revision)?;
        let before = commit.parent_tree(self.repo.as_ref()).block_on()?;
        let files = diff::file_changes(self.repo.store(), &before, &commit.tree())?;
        let author = commit.author();
        let mut text = format!(
            "Change {} (commit {})\nAuthor: {} <{}>\nTime: {} ms since the Unix epoch\n\n{}\n\n",
            short_change_id(commit.change_id()),
            &commit.id().hex()[..12],
            author.name,
            author.email,
            author.timestamp.timestamp.0,
            match commit.description().trim_end() {
                "" => "(no description)",
                description => description,
            },
        );
        for file in &files {
            let status = match file.status {
                FileStatus::Added => 'A',
                FileStatus::Modified => 'M',
                FileStatus::Deleted => 'D',
            };
            writeln!(
                text,
                "{status} {}  +{} -{}",
                file.path, file.added, file.removed
            )?;
        }
        if files.is_empty() {
            text.push_str("(no files changed)\n");
        }
        Ok(cap(text))
    }

    /// One commit from a revision as the jj CLI reads it.
    fn resolve(&self, revision: &str) -> Result<Commit> {
        let expression = match revision.trim() {
            "@" => RevsetExpression::working_copy(self.workspace_name()),
            symbol => RevsetExpression::symbol(symbol.to_owned()),
        };
        let id = commit_ids(self.repo.as_ref(), &expression, 1)
            .with_context(|| format!("no change named {revision}"))?
            .into_iter()
            .next()
            .with_context(|| format!("no change named {revision}"))?;
        Ok(self.repo.store().get_commit(&id)?)
    }
}

fn cap(mut text: String) -> String {
    if text.len() > MAX_TEXT {
        let mut end = MAX_TEXT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n(cut: the answer was too long; ask for one file)\n");
    }
    text
}
