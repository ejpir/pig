//! Which jj change each turn made, kept in the session file so file history
//! survives closing the session, restarting the app and reloading the
//! conversation (design study 05, decision 01).
//!
//! The only code that knows the encoding: `custom` entries of type
//! [`CUSTOM_TYPE`], written by the desktop extension. pi keeps them out of the
//! model's context and copies them into forks. Ids are hex, so nothing here
//! needs jj types; another agent needs only another place to keep the same events.
use pi_core::protocol::Command;
use serde_json::{Value, json};

pub const CUSTOM_TYPE: &str = "pi-desktop-turn";

/// A recorded turn.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub change: String,
    pub commit: String,
    /// The `timestamp` of the message the turn's line sits under.
    pub after: Option<u64>,
    pub tools: Vec<String>,
    /// While undone: the undo operation, which redo restores from.
    pub undone: Option<String>,
}

/// One change to a session's file history.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Recorded(Link),
    Undone { change: String, operation: String },
    Redone { change: String },
}

impl Event {
    /// Appends the event to the session file.
    pub fn command(&self) -> Command {
        let data = match self {
            Self::Recorded(link) => json!({
                "event": "record",
                "change": link.change,
                "commit": link.commit,
                "after": link.after,
                "tools": link.tools,
            }),
            Self::Undone { change, operation } => {
                json!({ "event": "undo", "change": change, "operation": operation })
            }
            Self::Redone { change } => json!({ "event": "redo", "change": change }),
        };
        Command::AppendCustomEntry {
            custom_type: CUSTOM_TYPE.into(),
            data,
        }
    }

    fn parse(data: &Value) -> Option<Self> {
        let text = |key: &str| data[key].as_str().map(str::to_owned);
        let change = text("change")?;
        Some(match data["event"].as_str()? {
            "record" => Self::Recorded(Link {
                change,
                commit: text("commit")?,
                after: data["after"].as_u64(),
                tools: data["tools"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
                undone: None,
            }),
            "undo" => Self::Undone {
                change,
                operation: text("operation")?,
            },
            "redo" => Self::Redone { change },
            _ => return None,
        })
    }
}

/// Reads every link event in the session file, from all branches: file history
/// does not follow conversation navigation.
pub fn request() -> Command {
    Command::GetCustomEntries {
        custom_type: CUSTOM_TYPE.into(),
    }
}

/// The turns in a `get_custom_entries` answer, in the order they ran, each with
/// its last undo or redo applied. Entries this version does not read are skipped.
pub fn replay(data: &Value) -> Vec<Link> {
    let mut links: Vec<Link> = Vec::new();
    let events = data["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["customType"] == CUSTOM_TYPE)
        .filter_map(|entry| Event::parse(&entry["data"]));
    for event in events {
        match event {
            Event::Recorded(link) => {
                links.retain(|known| known.change != link.change);
                links.push(link);
            }
            Event::Undone { change, operation } => {
                if let Some(link) = links.iter_mut().find(|link| link.change == change) {
                    link.undone = Some(operation);
                }
            }
            Event::Redone { change } => {
                if let Some(link) = links.iter_mut().find(|link| link.change == change) {
                    link.undone = None;
                }
            }
        }
    }
    links
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(event: &Event) -> Value {
        let Command::AppendCustomEntry { custom_type, data } = event.command() else {
            unreachable!()
        };
        json!({ "type": "custom", "customType": custom_type, "data": data })
    }

    #[test]
    fn events_replay_into_each_turns_last_state() {
        let link = |change: &str| Link {
            change: change.into(),
            commit: format!("{change}-commit"),
            after: Some(42),
            tools: vec!["call-1".into()],
            undone: None,
        };
        let entries = json!({ "entries": [
            entry(&Event::Recorded(link("a"))),
            entry(&Event::Recorded(link("b"))),
            entry(&Event::Undone { change: "a".into(), operation: "op1".into() }),
            entry(&Event::Undone { change: "b".into(), operation: "op2".into() }),
            entry(&Event::Redone { change: "b".into() }),
            { "type": "custom", "customType": CUSTOM_TYPE, "data": { "event": "later-version" } },
            { "type": "custom", "customType": "plan-mode", "data": { "event": "record" } },
        ]});
        let links = replay(&entries);
        assert_eq!(
            links,
            [
                Link {
                    undone: Some("op1".into()),
                    ..link("a")
                },
                link("b")
            ]
        );
    }
}
