//! Work Pi handed to other agents through the `subagent` tool. The durable
//! backend's tool reports one summary per subagent in the call's details, and
//! returns a subagent's own messages on `get_subagent`; stock Pi's subagent
//! extension reports its results in a shape of its own, read here too.

use anyhow::Result;
use serde_json::{Value, json};

use crate::session::{Session, Tool};

pub const TOOL: &str = "subagent";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Single,
    /// Side by side.
    Parallel,
    /// One after another, each given the previous one's answer.
    Chain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// A later step of a chain, or a task queued behind others.
    Waiting,
    Running,
    Done,
    Failed,
    Stopped,
}

impl Status {
    pub fn finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Stopped)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Subagent {
    /// Its agent's name: "scout".
    pub agent: String,
    pub task: String,
    pub status: Status,
    /// What `get_subagent` takes; absent before it starts, and in stock Pi.
    pub conversation_id: Option<String>,
    /// Stable address inside an opt-in swarm; display names need not be unique.
    pub peer_id: Option<String>,
    /// "provider/model".
    pub model: Option<String>,
    /// Why it doesn't run on the model its agent file names.
    pub model_note: Option<String>,
    /// What it is doing now, or how it ended: "Reading retry.ts".
    pub now: Option<String>,
    /// Milliseconds since 1970.
    pub started_at: Option<u64>,
    pub ended_at: Option<u64>,
    /// In dollars.
    pub cost: Option<f64>,
    /// Its final answer, shortened.
    pub output: Option<String>,
    /// Commands a restart cut off, which weren't repeated by themselves.
    pub interrupted: Vec<String>,
}

impl Subagent {
    /// How long it has worked, as of `now_ms` while it runs.
    pub fn elapsed_ms(&self, now_ms: u64) -> Option<u64> {
        let start = self.started_at?;
        Some(self.ended_at.unwrap_or(now_ms).saturating_sub(start))
    }
}

/// Explicit peer communication attached to one opt-in, session-owned crew.
/// This is a bounded display projection, not mailbox admission authority.
#[derive(Clone, Debug, PartialEq)]
pub struct Swarm {
    pub id: String,
    pub members: Vec<SwarmMember>,
    pub messages: Vec<SwarmMessage>,
    pub closed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SwarmMember {
    pub id: String,
    pub agent: String,
    pub task: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwarmMessageKind {
    Finding,
    Question,
    Reply,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwarmMessageStatus {
    Queued,
    /// The recipient explicitly retrieved it through its peer tool. Not agreement,
    /// a passing check, or proof of successful downstream execution.
    Received,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SwarmMessage {
    pub id: String,
    pub sequence: u64,
    pub from: String,
    pub to: String,
    pub kind: SwarmMessageKind,
    pub text: String,
    pub reply_to: Option<String>,
    pub status: SwarmMessageStatus,
    pub created_at: u64,
    pub received_at: Option<u64>,
}

impl Swarm {
    /// Old helpers and unsupported versions remain ordinary handoffs. Reject
    /// ambiguous rosters rather than guessing which duplicate name was addressed.
    pub fn from_details(value: &Value) -> Option<Self> {
        if value["version"] != 1 {
            return None;
        }
        let id = bounded(value, "id", 256)?;
        let rows = value["members"].as_array()?;
        if !(2..=8).contains(&rows.len()) {
            return None;
        }
        let mut members = Vec::<SwarmMember>::new();
        for row in rows {
            let member_id = bounded(row, "id", 64)?;
            if members.iter().any(|member| member.id == member_id) {
                return None;
            }
            members.push(SwarmMember {
                id: member_id,
                agent: bounded(row, "agent", 64)?,
                task: shortened(&text(row, "task"), 512),
            });
        }
        let mut messages = Vec::<SwarmMessage>::new();
        // Defensive UI bound in addition to the computer-side projection limit.
        let rows = value["messages"].as_array().map_or(&[][..], Vec::as_slice);
        for row in rows.iter().skip(rows.len().saturating_sub(32)) {
            let Some(message) = SwarmMessage::from_value(row) else {
                continue;
            };
            if !members.iter().any(|member| member.id == message.from)
                || !members.iter().any(|member| member.id == message.to)
                || messages.iter().any(|existing| existing.id == message.id)
            {
                continue;
            }
            messages.push(message);
        }
        Some(Self {
            id,
            members,
            messages,
            closed: value["closed"] == true,
        })
    }

    pub fn member(&self, id: &str) -> Option<&SwarmMember> {
        self.members.iter().find(|member| member.id == id)
    }
}

impl SwarmMessage {
    fn from_value(value: &Value) -> Option<Self> {
        let kind = match value["kind"].as_str()? {
            "finding" => SwarmMessageKind::Finding,
            "question" => SwarmMessageKind::Question,
            "reply" => SwarmMessageKind::Reply,
            _ => return None,
        };
        let status = match value["status"].as_str()? {
            "queued" => SwarmMessageStatus::Queued,
            "received" => SwarmMessageStatus::Received,
            "cancelled" => SwarmMessageStatus::Cancelled,
            _ => return None,
        };
        let reply_to = bounded(value, "replyTo", 256);
        if kind == SwarmMessageKind::Reply && reply_to.is_none() {
            return None;
        }
        Some(Self {
            id: bounded(value, "id", 256)?,
            sequence: value["sequence"].as_u64()?,
            from: bounded(value, "from", 64)?,
            to: bounded(value, "to", 64)?,
            kind,
            text: shortened(value["text"].as_str()?, 4096),
            reply_to,
            status,
            created_at: value["createdAt"].as_u64()?,
            received_at: value["receivedAt"].as_u64(),
        })
    }
}

fn bounded(value: &Value, key: &str, bytes: usize) -> Option<String> {
    value[key]
        .as_str()
        .filter(|text| !text.trim().is_empty() && text.len() <= bytes)
        .map(str::to_owned)
}

fn shortened(text: &str, bytes: usize) -> String {
    let mut end = text.len().min(bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// One `subagent` call: its subagents in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Handoff {
    pub mode: Mode,
    /// The call carried on after the computer restarted.
    pub resumed: bool,
    /// Its subagents work on after the call returns, and report through the session.
    pub background: bool,
    pub subagents: Vec<Subagent>,
    /// Present only when the computer reports opt-in peer communications.
    pub swarm: Option<Swarm>,
}

impl Handoff {
    /// The call's handoff: from its details once it reports, else from what it was asked.
    pub fn of(tool: &Tool) -> Option<Self> {
        (tool.name == TOOL).then(|| {
            let mut handoff =
                Self::from_details(&tool.details).unwrap_or_else(|| Self::from_args(&tool.args));
            // A call its subagents don't outlive: once it ended, none is at
            // work, whatever it said last (a stopped call says "running").
            if tool.finished && !handoff.background {
                handoff.ended(tool.is_error);
            }
            handoff
        })
    }

    /// Its call ended without saying how each subagent did: none is still at work.
    pub fn ended(&mut self, cut_short: bool) {
        for subagent in &mut self.subagents {
            if !subagent.status.finished() {
                subagent.status = if cut_short {
                    Status::Stopped
                } else {
                    Status::Done
                };
            }
        }
    }

    /// From the arguments alone: every task waiting.
    pub fn from_args(args: &Value) -> Self {
        let item = |value: &Value| Subagent::waiting(text(value, "agent"), text(value, "task"));
        let (mode, subagents) =
            if let Some(chain) = args["chain"].as_array().filter(|items| !items.is_empty()) {
                (Mode::Chain, chain.iter().map(item).collect())
            } else if let Some(tasks) = args["tasks"].as_array().filter(|items| !items.is_empty()) {
                (Mode::Parallel, tasks.iter().map(item).collect())
            } else {
                (Mode::Single, vec![item(args)])
            };
        Self {
            mode,
            resumed: false,
            background: false,
            subagents,
            swarm: None,
        }
    }

    pub fn from_details(details: &Value) -> Option<Self> {
        let results = details["results"].as_array()?;
        let mode = match details["mode"].as_str() {
            Some("parallel") => Mode::Parallel,
            Some("chain") => Mode::Chain,
            _ => Mode::Single,
        };
        let subagents = if details["version"] == 1 {
            results.iter().map(Subagent::durable).collect()
        } else {
            results.iter().map(Subagent::stock).collect()
        };
        Some(Self {
            mode,
            resumed: details["resumed"] == true,
            background: details["background"] == true,
            subagents,
            swarm: Swarm::from_details(&details["swarm"]),
        })
    }

    pub fn cost(&self) -> f64 {
        self.subagents
            .iter()
            .filter_map(|subagent| subagent.cost)
            .sum()
    }

    pub fn running(&self) -> usize {
        self.subagents
            .iter()
            .filter(|subagent| subagent.status == Status::Running)
            .count()
    }
}

impl Subagent {
    fn waiting(agent: String, task: String) -> Self {
        Self {
            agent,
            task,
            status: Status::Waiting,
            conversation_id: None,
            peer_id: None,
            model: None,
            model_note: None,
            now: None,
            started_at: None,
            ended_at: None,
            cost: None,
            output: None,
            interrupted: Vec::new(),
        }
    }

    /// The durable runner's summary.
    fn durable(result: &Value) -> Self {
        let mut subagent = Self::waiting(text(result, "agent"), text(result, "task"));
        subagent.status = match result["status"].as_str() {
            Some("running") => Status::Running,
            Some("done") => Status::Done,
            Some("failed") => Status::Failed,
            Some("stopped") => Status::Stopped,
            _ => Status::Waiting,
        };
        subagent.conversation_id = optional(result, "conversationId");
        subagent.peer_id = bounded(result, "peerId", 64);
        subagent.model = optional(result, "model");
        subagent.model_note = optional(result, "modelNote");
        subagent.now = optional(result, "now");
        subagent.started_at = result["startedAt"].as_u64();
        subagent.ended_at = result["endedAt"].as_u64();
        subagent.cost = result["cost"].as_f64();
        subagent.output = optional(result, "output");
        subagent.interrupted = result["interrupted"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|command| command.as_str().map(str::to_owned))
            .collect();
        subagent
    }

    /// Stock Pi's subagent extension: an exit code and the subagent's messages.
    fn stock(result: &Value) -> Self {
        let mut subagent = Self::waiting(text(result, "agent"), text(result, "task"));
        let answer = result["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .rev()
            .find(|message| message["role"] == "assistant")
            .map(|message| crate::session::content_text(&message["content"]))
            .unwrap_or_default();
        subagent.status = match result["exitCode"].as_i64() {
            Some(-1) => Status::Running,
            Some(0) if result["stopReason"] != "error" => Status::Done,
            Some(_) if result["stopReason"] == "aborted" => Status::Stopped,
            Some(_) => Status::Failed,
            None => Status::Waiting,
        };
        subagent.model = optional(result, "model");
        subagent.cost = result["usage"]["cost"].as_f64();
        subagent.now = optional(result, "errorMessage").or_else(|| {
            answer
                .lines()
                .find(|line| !line.trim().is_empty())
                .map(str::to_owned)
        });
        subagent.output = (!answer.is_empty()).then_some(answer);
        subagent
    }
}

/// The message that brings a background crew's answers back to Pi: what they
/// said, without the wrapper. Pi reads it as input, but nobody typed it.
pub fn report(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("<subagent_report")?;
    let body = rest.split_once('>')?.1;
    let body = body.trim_end().strip_suffix("</subagent_report>")?;
    // After the line that tells Pi what this is.
    Some(
        body.trim()
            .split_once("\n\n")
            .map_or("", |(_, said)| said)
            .trim(),
    )
}

/// A subagent's conversation from `get_subagent`, as a session the apps draw.
pub fn session(data: &Value, cwd: std::path::PathBuf) -> Result<Session> {
    let mut session = Session::new(cwd);
    session.apply(&json!({"type":"response","command":"get_messages","success":true,"data":{"messages":data["messages"]}}))?;
    if data["busy"] == true {
        session.apply(&json!({"type":"agent_start"}))?;
        session.state.is_streaming = true;
    }
    for slot in data["tools"].as_array().into_iter().flatten() {
        let id = text(slot, "callId");
        if !session.tools.iter().any(|tool| tool.id == id) {
            session.apply(&json!({"type":"tool_execution_start","toolCallId":id,"toolName":slot["name"],"args":{}}))?;
        }
        session.apply(&json!({"type":"tool_execution_update","toolCallId":id,"toolName":slot["name"],"partialResult":{"content":[{"type":"text","text":slot["output"]}]}}))?;
    }
    Ok(session)
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

fn optional(value: &Value, key: &str) -> Option<String> {
    value[key]
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_reads_as_waiting_tasks_until_it_reports() {
        let handoff = Handoff::from_args(
            &json!({"chain":[{"agent":"scout","task":"look"},{"agent":"planner","task":"plan {previous}"}]}),
        );
        assert_eq!(handoff.mode, Mode::Chain);
        assert_eq!(
            handoff
                .subagents
                .iter()
                .map(|subagent| (subagent.agent.as_str(), subagent.status))
                .collect::<Vec<_>>(),
            [("scout", Status::Waiting), ("planner", Status::Waiting)]
        );
        assert_eq!(
            Handoff::from_args(&json!({"agent":"worker","task":"build"})).mode,
            Mode::Single
        );
    }

    #[test]
    fn the_durable_summary_and_stock_results_read_alike() {
        let durable = Handoff::from_details(&json!({"version":1,"mode":"parallel","resumed":true,"results":[
            {"index":0,"agent":"scout","task":"find alpha","status":"done","conversationId":"3","cost":0.02,"output":"found alpha","startedAt":1000,"endedAt":4000},
            {"index":1,"agent":"scout","task":"find beta","status":"running","now":"Reading beta.ts","cost":0.01,"interrupted":["pnpm test"],"startedAt":1000}
        ]})).unwrap();
        assert!(durable.resumed);
        assert_eq!(durable.running(), 1);
        assert!((durable.cost() - 0.03).abs() < 1e-9);
        assert_eq!(durable.subagents[0].elapsed_ms(9000), Some(3000));
        assert_eq!(durable.subagents[1].elapsed_ms(9000), Some(8000));
        assert_eq!(durable.subagents[1].interrupted, ["pnpm test"]);
        assert_eq!(durable.subagents[0].conversation_id.as_deref(), Some("3"));

        let stock = Handoff::from_details(&json!({"mode":"single","agentScope":"user","projectAgentsDir":null,"results":[
            {"agent":"scout","agentSource":"user","task":"look","exitCode":0,"messages":[{"role":"assistant","content":[{"type":"text","text":"Found it.\nDetails."}]}],"stderr":"","usage":{"cost":0.05}}
        ]})).unwrap();
        let scout = &stock.subagents[0];
        assert_eq!(
            (scout.status, scout.now.as_deref(), scout.cost),
            (Status::Done, Some("Found it."), Some(0.05))
        );
    }

    #[test]
    fn a_call_that_ended_without_progress_has_nobody_at_work() {
        let mut tool = Tool {
            id: "hand".into(),
            name: TOOL.into(),
            args: json!({"tasks":[{"agent":"scout","task":"a"},{"agent":"scout","task":"b"}]}),
            output: String::new(),
            diff: None,
            finished: false,
            started: None,
            is_error: false,
            images: Vec::new(),
            details: Value::Null,
        };
        assert_eq!(
            Handoff::of(&tool).unwrap().subagents[0].status,
            Status::Waiting
        );
        tool.finished = true;
        assert!(
            Handoff::of(&tool)
                .unwrap()
                .subagents
                .iter()
                .all(|s| s.status == Status::Done)
        );
        tool.is_error = true;
        assert_eq!(
            Handoff::of(&tool).unwrap().subagents[1].status,
            Status::Stopped
        );
        // What a stopped call said last.
        tool.details = json!({"version":1,"mode":"parallel","results":[{"index":0,"agent":"scout","task":"a","status":"running"}]});
        assert_eq!(
            Handoff::of(&tool).unwrap().subagents[0].status,
            Status::Stopped
        );
        // Subagents that outlive their call say for themselves.
        tool.details["background"] = json!(true);
        assert_eq!(
            Handoff::of(&tool).unwrap().subagents[0].status,
            Status::Running
        );
    }

    fn swarm_details() -> Value {
        json!({"version":1,"id":"crew-123","closed":false,"members":[
            {"id":"peer-1","agent":"scout","task":"Find clients"},
            {"id":"peer-2","agent":"scout","task":"Find protocol constraints"}
        ],"messages":[
            {"id":"m-1","sequence":1,"from":"peer-1","to":"peer-2","kind":"question","text":"Can we keep snapshots?","status":"received","createdAt":1000,"receivedAt":2000},
            {"id":"m-2","sequence":2,"from":"peer-2","to":"peer-1","kind":"reply","replyTo":"m-1","text":"Yes, keep v1.","status":"queued","createdAt":3000}
        ]})
    }

    #[test]
    fn swarm_addresses_disambiguate_repeated_names_and_keep_receipts_separate() {
        let handoff = Handoff::from_details(&json!({"version":1,"mode":"parallel","background":true,
            "results":[{"agent":"scout","task":"Find clients","status":"running","conversationId":"3","peerId":"peer-1"}],
            "swarm":swarm_details()
        })).unwrap();
        let swarm = handoff.swarm.unwrap();
        assert_eq!(handoff.subagents[0].peer_id.as_deref(), Some("peer-1"));
        assert_eq!(swarm.members.len(), 2);
        assert_eq!(
            swarm.member("peer-2").unwrap().task,
            "Find protocol constraints"
        );
        assert_eq!(swarm.messages[0].status, SwarmMessageStatus::Received);
        assert_eq!(swarm.messages[0].received_at, Some(2000));
        assert_eq!(swarm.messages[1].kind, SwarmMessageKind::Reply);
        assert_eq!(swarm.messages[1].reply_to.as_deref(), Some("m-1"));
        assert_eq!(swarm.messages[1].status, SwarmMessageStatus::Queued);
        assert!(!swarm.closed);
    }

    #[test]
    fn invalid_rosters_and_foreign_messages_are_not_invented_as_live_swarm_state() {
        let mut details = swarm_details();
        details["version"] = json!(2);
        assert!(Swarm::from_details(&details).is_none());
        details["version"] = json!(1);
        details["members"][1]["id"] = json!("peer-1");
        assert!(Swarm::from_details(&details).is_none());
        details["members"][1]["id"] = json!("peer-2");
        details["messages"][0]["to"] = json!("foreign-peer");
        details["messages"][1]["kind"] = json!("unknown");
        assert!(Swarm::from_details(&details).unwrap().messages.is_empty());
        assert!(
            Handoff::from_args(&json!({"swarm":true,"tasks":[]}))
                .swarm
                .is_none()
        );
    }

    #[test]
    fn swarm_history_is_bounded_and_unicode_text_remains_valid() {
        let mut details = swarm_details();
        let mut messages = Vec::new();
        for sequence in 1..=60 {
            messages.push(json!({"id":format!("m-{sequence}"),"sequence":sequence,"from":"peer-1","to":"peer-2",
                "kind":"finding","text":"🦀".repeat(2000),"status":"cancelled","createdAt":1000}));
        }
        details["messages"] = json!(messages);
        details["closed"] = json!(true);
        let swarm = Swarm::from_details(&details).unwrap();
        assert_eq!(swarm.messages.len(), 32);
        assert_eq!(swarm.messages[0].sequence, 29);
        assert_eq!(swarm.messages[31].sequence, 60);
        assert_eq!(swarm.messages[0].text.len(), 4096);
        assert!(swarm.closed);
        assert_eq!(swarm.messages[0].status, SwarmMessageStatus::Cancelled);
    }

    #[test]
    fn ordinary_handoffs_do_not_gain_peer_capabilities() {
        let handoff = Handoff::from_details(&json!({"version":1,"mode":"single","results":[
            {"agent":"worker","task":"build","status":"running"}
        ]}))
        .unwrap();
        assert!(handoff.swarm.is_none());
        assert!(handoff.subagents[0].peer_id.is_none());
    }

    #[test]
    fn a_crew_report_reads_as_what_its_subagents_said() {
        let text = "<subagent_report call=\"c1\">\nThe subagents you started have finished. This is their report, not a message from the user.\n\n## scout: find alpha\n\nfound alpha\n</subagent_report>";
        assert_eq!(report(text), Some("## scout: find alpha\n\nfound alpha"));
        assert_eq!(report("hello"), None);
    }

    #[test]
    fn a_subagent_reads_as_a_session_with_its_running_tool() {
        let session = session(
            &json!({"busy":true,"messages":[
                {"role":"user","content":"find alpha"},
                {"role":"assistant","content":[{"type":"toolCall","id":"t1","name":"read","arguments":{"path":"/repo/a.ts"}}]}
            ],"tools":[{"callId":"t1","name":"read","output":"partial"}]}),
            "/repo".into(),
        )
        .unwrap();
        assert!(session.busy());
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.tools[0].output, "partial");
        assert!(!session.tools[0].finished);
    }
}
