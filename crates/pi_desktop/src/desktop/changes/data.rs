use super::*;
use pi_jj::{FileChange, FileStatus, LineKind};
use std::collections::BTreeMap;

#[derive(Clone)]
pub(super) struct File {
    pub path: String,
    pub patch: String,
    pub preview: String,
    pub added: usize,
    pub removed: usize,
    pub turn: Option<usize>,
    pub status: &'static str,
    pub source: String,
    pub touches: Vec<(String, String)>,
}
impl File {
    pub fn recorded(file: &FileChange, turn: usize, record: &super::super::jj::TurnRecord) -> Self {
        Self {
            path: file.path.clone(),
            patch: patch(file),
            preview: super::super::diff_preview::numbered(file),
            added: file.added,
            removed: file.removed,
            turn: Some(turn),
            status: match file.status {
                FileStatus::Added => "Added",
                FileStatus::Modified => "Modified",
                FileStatus::Deleted => "Deleted",
            },
            source: format!(
                "{} · turn {}{}",
                record.short,
                turn + 1,
                if record.undone.is_some() {
                    " · undone"
                } else {
                    " · jj snapshot"
                }
            ),
            touches: vec![],
        }
    }
}
pub(super) fn patch(file: &FileChange) -> String {
    if file.binary {
        return "Binary file changed — no text diff available.".into();
    }
    let old = if file.status == FileStatus::Added {
        "/dev/null".into()
    } else {
        format!("a/{}", file.path)
    };
    let new = if file.status == FileStatus::Deleted {
        "/dev/null".into()
    } else {
        format!("b/{}", file.path)
    };
    let mut text = format!("--- {old}\n+++ {new}\n");
    for h in &file.hunks {
        let old = h
            .lines
            .iter()
            .filter(|(k, _)| *k != LineKind::Added)
            .count();
        let new = h
            .lines
            .iter()
            .filter(|(k, _)| *k != LineKind::Removed)
            .count();
        text.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            h.old_start, old, h.new_start, new
        ));
        for (kind, line) in &h.lines {
            text.push(match kind {
                LineKind::Context => ' ',
                LineKind::Added => '+',
                LineKind::Removed => '-',
            });
            text.push_str(line);
            text.push('\n');
        }
    }
    text
}

/// Never attribute older tool calls to a new snapshot merely because their file
/// paths match. Tool ids are captured at recording time and survive hydration.
pub(super) fn observed(model: &Session, recorded: &HashSet<String>) -> Vec<File> {
    let mut files: BTreeMap<String, File> = BTreeMap::new();
    for tool in &model.tools {
        if recorded.contains(&tool.id)
            || !tool.finished
            || tool.is_error
            || !matches!(tool.name.as_str(), "edit" | "write")
        {
            continue;
        }
        let path = tool.target();
        if path.is_empty() {
            continue;
        }
        let patch = tool.diff.clone().unwrap_or_else(|| {
            let lines = |text: &str, prefix| {
                text.lines()
                    .map(|l| format!("{prefix}{l}\n"))
                    .collect::<String>()
            };
            if tool.name == "write" {
                lines(tool.args["content"].as_str().unwrap_or(""), '+')
            } else {
                format!(
                    "{}{}",
                    lines(tool.args["oldText"].as_str().unwrap_or(""), '-'),
                    lines(tool.args["newText"].as_str().unwrap_or(""), '+')
                )
            }
        });
        let file = files.entry(path.clone()).or_insert_with(|| File {
            path,
            patch: String::new(),
            preview: String::new(),
            added: 0,
            removed: 0,
            turn: None,
            status: "Tool-reported edits",
            source: "Tool-reported edits · No snapshot".into(),
            touches: vec![],
        });
        file.added += patch
            .lines()
            .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
            .count();
        file.removed += patch
            .lines()
            .filter(|l| l.starts_with('-') && !l.starts_with("---"))
            .count();
        if !file.patch.is_empty() {
            file.patch.push('\n');
        }
        file.patch
            .push_str(&format!("# {} · {}\n{}", tool.name, tool.id, patch));
        if !file.preview.is_empty() {
            file.preview.push('\n');
        }
        let preview = super::super::diff_preview::reported(&patch).unwrap_or_else(|| patch.clone());
        file.preview.push_str(&format!(
            "# {} · {}\n{}",
            tool.name,
            short_call_id(&tool.id),
            preview.trim_end_matches('\n')
        ));
        file.touches.push((tool.id.clone(), tool.name.clone()));
    }
    files.into_values().collect()
}

pub(super) fn short_call_id(id: &str) -> String {
    if id.chars().count() <= 28 {
        return id.to_owned();
    }
    let start: String = id.chars().take(16).collect();
    let end: String = id
        .chars()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{start}…{end}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn same_path_does_not_hide_older_unrecorded_edits() {
        let mut model = Session::new("/fixture".into());
        let messages = json!([{"role":"assistant","content":[
            {"type":"toolCall","id":"old","name":"write","arguments":{"path":"a.txt","content":"before"}},
            {"type":"toolCall","id":"new","name":"write","arguments":{"path":"a.txt","content":"after"}}
        ]},{"role":"toolResult","toolCallId":"old","toolName":"write","content":[{"type":"text","text":"ok"}]},
        {"role":"toolResult","toolCallId":"new","toolName":"write","content":[{"type":"text","text":"ok"}]}]);
        for _ in 0..2 {
            model.apply(&json!({"type":"response","command":"get_messages","success":true,"data":{"messages":messages}})).unwrap();
            let files = observed(&model, &HashSet::from(["new".to_owned()]));
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].turn, None);
            assert!(files[0].patch.contains("before"));
            assert!(!files[0].patch.contains("after"));
        }
    }
}
