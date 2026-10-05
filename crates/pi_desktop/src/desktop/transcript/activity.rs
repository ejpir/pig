//! Presentation-only grouping. Never summarize, reorder, or change session data.
use crate::components::{NodeState, Stage, StepKind};
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
    /// What the step did, for its rail node and hue.
    pub kind: StepKind,
}
impl Group {
    /// Only the step the run is in breathes; finished steps of a live turn are
    /// filled. Calls without a result are never shown as done once settled.
    /// `latest` is whether no later step exists yet.
    pub fn node_state(&self, latest: bool) -> NodeState {
        if self.failed > 0 {
            NodeState::Failed
        } else if self.live && (self.pending > 0 || latest) {
            NodeState::Live
        } else if !self.live && self.pending > 0 {
            NodeState::Incomplete
        } else {
            NodeState::Done
        }
    }
    pub fn auto_open(&self) -> bool {
        !self.tools.is_empty() && (self.pending > 0 || self.live && self.failed > 0)
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
    /// The closing text of each settled turn that used tools: the template's
    /// Hand off, filled.
    pub hand_offs: HashSet<Block>,
    /// Every call's kind, by call id. See `call_kinds`.
    pub kinds: HashMap<String, StepKind>,
}

/// A call's arguments: from its block, or from the tool when the block has none.
fn call_args<'a>(block: &'a serde_json::Value, model: &'a Session) -> &'a serde_json::Value {
    if block["arguments"].is_object() {
        return &block["arguments"];
    }
    let id = block["id"].as_str();
    model
        .tools
        .iter()
        .find(|tool| Some(tool.id.as_str()) == id)
        .map_or(&block["arguments"], |tool| &tool.args)
}

/// Every call's kind, by id, in session order. Running a file the agent wrote
/// or edited earlier in the session is verifying it, so a kind depends on the
/// calls before it.
pub(in crate::desktop) fn call_kinds(model: &Session) -> HashMap<String, StepKind> {
    let mut written: Vec<String> = vec![];
    let mut kinds = HashMap::new();
    let calls = model
        .messages
        .iter()
        .filter(|m| m["role"] == "assistant")
        .flat_map(|m| m["content"].as_array().into_iter().flatten())
        .filter(|block| block["type"] == "toolCall");
    for block in calls {
        let Some(id) = block["id"].as_str() else {
            continue;
        };
        let args = call_args(block, model);
        let name = block["name"].as_str().unwrap_or_else(|| {
            model
                .tools
                .iter()
                .find(|tool| tool.id == id)
                .map_or("", |tool| tool.name.as_str())
        });
        let kind = StepKind::of_call(name, args, &written);
        if kind == StepKind::Change
            && let Some(path) = args["path"].as_str()
            && !written.iter().any(|known| known == path)
        {
            written.push(path.to_owned());
        }
        kinds.insert(id.to_owned(), kind);
    }
    kinds
}

/// The template stages still ahead of the latest turn: those after the furthest
/// stage its calls reached. Before any call, the turn is understanding the
/// request. Callers show this only while the turn runs.
pub(in crate::desktop) fn stages_ahead(model: &Session) -> Vec<(Stage, NodeState)> {
    let start = model
        .messages
        .iter()
        .rposition(|m| m["role"] == "user")
        .unwrap_or(0);
    let calls: Vec<&serde_json::Value> = model.messages[start..]
        .iter()
        .filter(|m| m["role"] == "assistant")
        .flat_map(|m| m["content"].as_array().into_iter().flatten())
        .filter(|block| block["type"] == "toolCall")
        .collect();
    let kinds = call_kinds(model);
    let reached = calls
        .iter()
        .filter_map(|block| {
            let kind = *kinds.get(block["id"].as_str()?)?;
            Stage::of(kind, call_args(block, model))
        })
        .max();
    Stage::ALL
        .into_iter()
        .filter(|stage| reached.is_none_or(|reached| *stage > reached))
        .map(|stage| {
            let live = calls.is_empty() && stage == Stage::Understand;
            (
                stage,
                if live {
                    NodeState::Live
                } else {
                    NodeState::Planned
                },
            )
        })
        .collect()
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
        let mut result = Self {
            kinds: call_kinds(model),
            ..Self::default()
        };
        let mut current: Option<usize> = None;
        // Per turn: whether it called tools, and its last text block so far.
        let mut turn_calls = false;
        let mut closing: Option<Block> = None;
        let close_turn = |calls: &mut bool, closing: &mut Option<Block>, result: &mut Self| {
            if let Some(block) = closing.take().filter(|_| *calls) {
                result.hand_offs.insert(block);
            }
            *calls = false;
        };
        for (row, message) in model.messages.iter().enumerate() {
            if message["role"] == "user" {
                close_turn(&mut turn_calls, &mut closing, &mut result);
            }
            if message["role"] != "assistant" {
                // Tool results belong to the calls before them; anything else,
                // like a new prompt or a shell command, is a boundary.
                if message["role"] != "toolResult" {
                    current = None;
                }
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
                let position = Block { row, index };
                match kind {
                    Some("text") => closing = Some(position),
                    Some("toolCall") => {
                        turn_calls = true;
                        closing = None;
                    }
                    _ => {}
                }
                if !matches!(kind, Some("thinking" | "toolCall")) {
                    current = None;
                    continue;
                }
                let id = block["id"].as_str().unwrap_or("");
                let step = (kind == Some("toolCall"))
                    .then(|| result.kinds.get(id).copied().unwrap_or(StepKind::Other));
                // A new phase of work is a new step on the rail. Reasoning after the
                // previous step's last call leads into this one, so it moves along.
                if let (Some(step), Some(open)) = (step, current)
                    && !result.groups[open].tools.is_empty()
                    && result.groups[open].kind.phase() != step.phase()
                {
                    let old = &mut result.groups[open];
                    let last_call = old
                        .blocks
                        .iter()
                        .rposition(|b| {
                            model.messages[b.row]["content"][b.index]["type"] == "toolCall"
                        })
                        .map_or(0, |i| i + 1);
                    let moved = old.blocks.split_off(last_call);
                    result.groups.push(Group {
                        key: moved.first().copied().unwrap_or(position),
                        blocks: moved,
                        tools: vec![],
                        failed: 0,
                        pending: 0,
                        live: working && row >= current_turn,
                        kind: StepKind::Reasoning,
                    });
                    let next = result.groups.len() - 1;
                    for block in &result.groups[next].blocks {
                        result.by_block.insert(*block, next);
                    }
                    current = Some(next);
                }
                let group_index = *current.get_or_insert_with(|| {
                    result.groups.push(Group {
                        key: position,
                        blocks: vec![],
                        tools: vec![],
                        failed: 0,
                        pending: 0,
                        live: working && row >= current_turn,
                        kind: StepKind::Reasoning,
                    });
                    result.groups.len() - 1
                });
                let group = &mut result.groups[group_index];
                group.blocks.push(position);
                if let Some(step) = step {
                    group.tools.push(id.to_owned());
                    group.kind = group.kind.merge(step);
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
        // A running turn has not handed off yet, whatever it has written so far.
        if !working {
            close_turn(&mut turn_calls, &mut closing, &mut result);
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
    fn steps_follow_phases_of_work_across_tool_results() {
        let mut model = Session::new("/demo".into());
        let call = |id: &str, name: &str| json!({"type":"toolCall","id":id,"name":name});
        let result = |id: &str| json!({"role":"toolResult","toolCallId":id,"content":[]});
        model.messages = vec![
            json!({"role":"user","content":"Fix it"}),
            json!({"role":"assistant","content":[{"type":"thinking","thinking":"Look first"},call("r","read")]}),
            result("r"),
            json!({"role":"assistant","content":[call("g","grep"),{"type":"thinking","thinking":"Now change it"},call("e1","edit")]}),
            result("g"),
            result("e1"),
            json!({"role":"assistant","content":[call("e2","write")]}),
            result("e2"),
            json!({"role":"assistant","content":[call("b","bash")]}),
            result("b"),
            json!({"role":"bashExecution","command":"ls","output":""}),
            json!({"role":"assistant","content":[call("b2","bash")]}),
        ];
        let activity = Activity::project(&model, false, &HashSet::new());
        let steps: Vec<_> = activity
            .groups
            .iter()
            .map(|g| (g.tools.join(","), g.kind))
            .collect();
        assert_eq!(
            steps,
            [
                ("r,g".into(), StepKind::Explore),
                ("e1,e2".into(), StepKind::Change),
                ("b".into(), StepKind::Run),
                ("b2".into(), StepKind::Run),
            ],
            "reads and searches are one step; a shell command you ran is a boundary"
        );
        assert_eq!(
            activity.groups[1].key,
            Block { row: 3, index: 1 },
            "the reasoning that leads into a change starts that step"
        );
        assert_eq!(
            activity.group(Block { row: 3, index: 1 }),
            Some(&activity.groups[1])
        );
    }
    #[test]
    fn the_template_is_painted_before_calls_and_fills_in_as_they_arrive() {
        let mut model = Session::new("/demo".into());
        let call = |id: &str, name: &str, command: &str| json!({"type":"toolCall","id":id,"name":name,"arguments":{"command":command}});
        let ahead = |model: &Session| {
            stages_ahead(model)
                .into_iter()
                .map(|(stage, state)| (stage.slug(), state))
                .collect::<Vec<_>>()
        };
        model.messages = vec![
            json!({"role":"user","content":"Earlier"}),
            json!({"role":"assistant","content":[call("old","bash","npm test")]}),
            json!({"role":"user","content":"Fix it"}),
        ];
        assert_eq!(
            ahead(&model),
            [
                ("understand", NodeState::Live),
                ("change", NodeState::Planned),
                ("verify", NodeState::Planned),
                ("handoff", NodeState::Planned),
            ],
            "a new turn shows the whole flow, ignoring the previous turn"
        );
        model
            .messages
            .push(json!({"role":"assistant","content":[call("r","read","")]}));
        assert_eq!(
            ahead(&model),
            [
                ("change", NodeState::Planned),
                ("verify", NodeState::Planned),
                ("handoff", NodeState::Planned)
            ]
        );
        model
            .messages
            .push(json!({"role":"assistant","content":[call("c","bash","npm run check")]}));
        model
            .messages
            .push(json!({"role":"assistant","content":[call("l","bash","git diff")]}));
        assert_eq!(
            ahead(&model),
            [("handoff", NodeState::Planned)],
            "skipped stages are not offered again, and looking back does not rewind"
        );
    }
    #[test]
    fn trying_a_script_written_in_an_earlier_turn_verifies_it() {
        let mut model = Session::new("/demo".into());
        model.messages = vec![
            json!({"role":"user","content":"Write nick.sh"}),
            json!({"role":"assistant","content":[
                {"type":"toolCall","id":"w","name":"write","arguments":{"path":"/proj/nick.sh","content":"echo hi"}},
                {"type":"toolCall","id":"x","name":"bash","arguments":{"command":"chmod +x nick.sh"}}
            ]}),
            json!({"role":"user","content":"Test it"}),
            json!({"role":"assistant","content":[
                {"type":"toolCall","id":"t","name":"bash","arguments":{"command":"bash nick.sh"}}
            ]}),
        ];
        let kinds = call_kinds(&model);
        assert_eq!(kinds["x"], StepKind::Run);
        assert_eq!(kinds["t"], StepKind::Check);
        let activity = Activity::project(&model, true, &HashSet::new());
        assert_eq!(activity.groups.last().unwrap().kind, StepKind::Check);
        assert_eq!(
            stages_ahead(&model)
                .into_iter()
                .map(|(stage, _)| stage)
                .collect::<Vec<_>>(),
            [Stage::HandOff]
        );
    }
    #[test]
    fn a_settled_turn_that_used_tools_hands_off_with_its_closing_text() {
        let mut model = Session::new("/demo".into());
        let call = |id: &str| json!({"type":"toolCall","id":id,"name":"read"});
        model.messages = vec![
            json!({"role":"user","content":"Question"}),
            json!({"role":"assistant","content":[{"type":"text","text":"Just an answer"}]}),
            json!({"role":"user","content":"Fix it"}),
            json!({"role":"assistant","content":[{"type":"text","text":"Looking"},call("r")]}),
            json!({"role":"toolResult","toolCallId":"r","content":[]}),
            json!({"role":"assistant","content":[{"type":"text","text":"Done: fixed"}]}),
        ];
        let settled = Activity::project(&model, false, &HashSet::new());
        assert_eq!(
            settled.hand_offs,
            HashSet::from([Block { row: 5, index: 0 }])
        );
        let running = Activity::project(&model, true, &HashSet::new());
        assert!(
            running.hand_offs.is_empty(),
            "a running turn has not handed off yet"
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
            kind: StepKind::Run,
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
        assert_eq!(group.node_state(true), NodeState::Done);
        group.live = true;
        assert_eq!(
            group.node_state(false),
            NodeState::Done,
            "an earlier step of a live turn"
        );
        assert_eq!(group.node_state(true), NodeState::Live);
        group.live = false;
        group.pending = 1;
        assert_eq!(group.node_state(true), NodeState::Incomplete);
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
