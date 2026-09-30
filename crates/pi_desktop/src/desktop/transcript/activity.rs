//! Presentation-only grouping. Never summarize, reorder, or change session data.
use pi_core::session::Session;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Block {
    pub row: usize,
    pub index: usize,
}
impl Block {
    pub fn selector(self) -> String {
        format!("activity-{}-{}", self.row, self.index)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Group {
    pub key: Block,
    pub blocks: Vec<Block>,
    pub tools: Vec<String>,
    pub failed: usize,
    pub pending: usize,
    pub live: bool,
}
impl Group {
    pub fn auto_open(&self) -> bool {
        !self.tools.is_empty() && (self.live || self.pending > 0)
    }
    pub fn label(&self) -> String {
        if self.tools.is_empty() {
            return "Reasoning".into();
        }
        let n = self.tools.len();
        let title = format!("{n} tool call{}", if n == 1 { "" } else { "s" });
        if self.failed > 0 {
            format!("{title} · {} failed", self.failed)
        } else if self.live {
            title
        } else if self.pending > 0 {
            format!("{title} · not completed")
        } else {
            format!("{title} · complete")
        }
    }
}
#[derive(Default)]
pub(super) struct Activity {
    pub groups: Vec<Group>,
    by_block: HashMap<Block, usize>,
}
impl Activity {
    pub fn group(&self, block: Block) -> Option<&Group> {
        self.by_block.get(&block).map(|i| &self.groups[*i])
    }
    pub fn project(model: &Session, working: bool, anchors: &HashSet<usize>) -> Self {
        let tools: HashMap<_, _> = model.tools.iter().map(|t| (t.id.as_str(), t)).collect();
        let current_turn = model
            .messages
            .iter()
            .rposition(|m| m["role"] == "user")
            .unwrap_or(0);
        let mut result = Self::default();
        let mut current: Option<usize> = None;
        for (row, message) in model.messages.iter().enumerate() {
            if message["role"] != "assistant" {
                current = None;
                continue;
            }
            for (index, block) in message["content"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                let kind = block["type"].as_str();
                // Empty/redacted blocks are not a visible boundary.
                if matches!(kind, Some("text" | "thinking"))
                    && block[if kind == Some("text") {
                        "text"
                    } else {
                        "thinking"
                    }]
                    .as_str()
                    .is_none_or(|s| s.trim().is_empty())
                {
                    continue;
                }
                if !matches!(kind, Some("thinking" | "toolCall")) {
                    current = None;
                    continue;
                }
                let position = Block { row, index };
                let group_index = *current.get_or_insert_with(|| {
                    result.groups.push(Group {
                        key: position,
                        blocks: vec![],
                        tools: vec![],
                        failed: 0,
                        pending: 0,
                        live: working && row >= current_turn,
                    });
                    result.groups.len() - 1
                });
                let group = &mut result.groups[group_index];
                group.blocks.push(position);
                if kind == Some("toolCall") {
                    let id = block["id"].as_str().unwrap_or("");
                    group.tools.push(id.to_owned());
                    if let Some(tool) = tools.get(id) {
                        group.failed += usize::from(tool.is_error);
                        group.pending += usize::from(!tool.finished);
                    } else {
                        group.pending += 1;
                    }
                }
                result.by_block.insert(position, group_index);
            }
            // Errors and immutable turn footers must never disappear inside a group.
            if message["errorMessage"].as_str().is_some() || anchors.contains(&row) {
                current = None;
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn groups_across_messages_but_never_across_prose_users_errors_or_snapshots() {
        let mut model = Session::new("/demo".into());
        model.messages = vec![
            json!({"role":"assistant","content":[{"type":"thinking","thinking":"Plan"},{"type":"toolCall","id":"a"}]}),
            json!({"role":"assistant","content":[{"type":"thinking","thinking":"Check"},{"type":"toolCall","id":"b"},{"type":"text","text":"Explanation"},{"type":"toolCall","id":"c"}]}),
            json!({"role":"assistant","content":[{"type":"toolCall","id":"d"}]}),
            json!({"role":"assistant","errorMessage":"Stopped","content":[{"type":"toolCall","id":"e"}]}),
            json!({"role":"assistant","content":[{"type":"toolCall","id":"f"}]}),
            json!({"role":"user","content":"Next"}),
            json!({"role":"assistant","content":[{"type":"toolCall","id":"g"}]}),
        ];
        let activity = Activity::project(&model, true, &HashSet::from([1]));
        let ids: Vec<_> = activity.groups.iter().map(|g| g.tools.join(",")).collect();
        assert_eq!(ids, ["a,b", "c", "d,e", "f", "g"]);
        assert!(!activity.groups[0].live);
        assert!(activity.groups.last().unwrap().live);
        assert!(
            activity.groups[0].label().contains("not completed"),
            "missing tool results are not success"
        );
    }
    #[test]
    fn failure_does_not_pin_settled_activity_open() {
        let mut group = Group {
            key: Block { row: 0, index: 0 },
            blocks: vec![],
            tools: vec!["one".into(), "two".into()],
            failed: 1,
            pending: 0,
            live: false,
        };
        assert!(!group.auto_open());
        assert_eq!(group.label(), "2 tool calls · 1 failed");
        group.live = true;
        assert!(group.auto_open());
        group.live = false;
        group.pending = 1;
        assert!(group.auto_open(), "unfinished calls must still be visible");
        group.pending = 0;
        group.failed = 0;
        assert!(!group.auto_open());
        assert_eq!(group.label(), "2 tool calls · complete");
    }

    #[test]
    fn text_streaming_does_not_change_activity_identity() {
        let mut model = Session::new("/demo".into());
        model.messages = vec![
            json!({"role":"assistant","content":[{"type":"thinking","thinking":""},{"type":"toolCall","id":"a"},{"type":"text","text":"Hello"}]}),
        ];
        let before = Activity::project(&model, true, &HashSet::new());
        model.messages[0]["content"][2]["text"] = json!("Hello, still streaming");
        let after = Activity::project(&model, true, &HashSet::new());
        assert_eq!(before.groups, after.groups);
    }
}
