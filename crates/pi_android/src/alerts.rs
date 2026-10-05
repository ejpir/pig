//! Notifications and the `pi://` links they open. A notification is an entry
//! point: it names the session and the computer, and opens the exact place
//! to act. Answering from one still opens the app, so the lock screen asks
//! to unlock first.

use crate::model::{Session, SessionId, State};
use gpui_android::activity::{Channel, Importance, Notification};

pub const QUESTIONS: Channel = Channel {
    id: "questions",
    name: "When Pi needs you",
    importance: Importance::High,
};

pub const FINISHED: Channel = Channel {
    id: "finished",
    name: "When a session finishes",
    importance: Importance::Default,
};

pub const WORKING: Channel = Channel {
    id: "working",
    name: "While sessions work",
    importance: Importance::Low,
};

/// The one ongoing notification while sessions work.
pub const WORKING_ID: i32 = 1;

pub fn question_id(session: SessionId) -> i32 {
    1000 + session.0 as i32
}

pub fn finished_id(session: SessionId) -> i32 {
    2000 + session.0 as i32
}

/// Where a link leads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    Session(SessionId),
    /// The session with its question open.
    Question(SessionId),
    /// Answers "Allow once", then shows the session.
    AllowOnce(SessionId),
    Review(SessionId),
}

impl Link {
    pub fn url(self) -> String {
        match self {
            Self::Session(id) => format!("pi://session/{}", id.0),
            Self::Question(id) => format!("pi://session/{}/question", id.0),
            Self::AllowOnce(id) => format!("pi://session/{}/question/allow-once", id.0),
            Self::Review(id) => format!("pi://session/{}/review", id.0),
        }
    }

    pub fn parse(url: &str) -> Option<Self> {
        let path = url.strip_prefix("pi://session/")?;
        let mut parts = path.split('/');
        let id = SessionId(parts.next()?.parse().ok()?);
        let link = match (parts.next(), parts.next()) {
            (None | Some(""), None) => Self::Session(id),
            (Some("question"), None) => Self::Question(id),
            (Some("question"), Some("allow-once")) => Self::AllowOnce(id),
            (Some("review"), None) => Self::Review(id),
            _ => return None,
        };
        parts.next().is_none().then_some(link)
    }
}

pub fn question(session: &Session, computer: &str, accent: u32) -> Option<Notification> {
    let question = session.question.as_ref()?;
    Some(Notification {
        id: question_id(session.id),
        channel: QUESTIONS,
        title: format!("{} needs you", session.title),
        text: format!("{} {}", question.title, question.command),
        subtext: Some(computer.to_owned()),
        url: Link::Question(session.id).url(),
        actions: vec![
            ("Allow once".into(), Link::AllowOnce(session.id).url()),
            ("Open".into(), Link::Question(session.id).url()),
        ],
        ongoing: false,
        color: accent,
    })
}

pub fn finished(session: &Session, computer: &str, accent: u32) -> Notification {
    let title = match session.state {
        State::Failed => format!("{} stopped", session.title),
        _ => format!("{} is done", session.title),
    };
    let actions = if session.files.is_empty() {
        Vec::new()
    } else {
        vec![("Review changes".into(), Link::Review(session.id).url())]
    };
    Notification {
        id: finished_id(session.id),
        channel: FINISHED,
        title,
        text: capitalized(&session.status_line()),
        subtext: Some(computer.to_owned()),
        url: Link::Session(session.id).url(),
        actions,
        ongoing: false,
        color: accent,
    }
}

/// "2 sessions working": what each is doing, quietly, while the app is away.
pub fn working<'a>(
    sessions: impl Iterator<Item = &'a Session>,
    computer: &str,
    accent: u32,
) -> Option<Notification> {
    let sessions: Vec<_> = sessions.collect();
    let first = sessions.first()?;
    let title = match sessions.len() {
        1 => "1 session working".to_owned(),
        count => format!("{count} sessions working"),
    };
    let text = sessions
        .iter()
        .map(|session| format!("{}: {}", session.title, session.activity.to_lowercase()))
        .collect::<Vec<_>>()
        .join(" · ");
    Some(Notification {
        id: WORKING_ID,
        channel: WORKING,
        title,
        text,
        subtext: Some(computer.to_owned()),
        url: Link::Session(first.id).url(),
        actions: Vec::new(),
        ongoing: true,
        color: accent,
    })
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::Computer, store::Store};
    use std::time::Duration;

    #[test]
    fn links_round_trip() {
        for link in [
            Link::Session(SessionId(3)),
            Link::Question(SessionId(3)),
            Link::AllowOnce(SessionId(3)),
            Link::Review(SessionId(12)),
        ] {
            assert_eq!(Link::parse(&link.url()), Some(link));
        }
        assert_eq!(Link::parse("pi://session/x"), None);
        assert_eq!(Link::parse("pi://session/3/delete"), None);
        assert_eq!(Link::parse("https://example.com"), None);
    }

    #[test]
    fn a_question_offers_its_safe_answer() {
        let mut store = Store::sample(Computer::from_address("nick@studio-mac.local"));
        store.tick(Duration::from_secs(10));
        let session = store.session(SessionId(1)).unwrap();
        let notification = question(session, "studio-mac", 0x4b607c).unwrap();
        assert_eq!(notification.title, "Qwen signatures needs you");
        assert_eq!(
            notification.text,
            "Run the provider tests? pnpm test --filter @pi/ai -- qwen"
        );
        assert_eq!(
            notification.actions[0].1,
            "pi://session/1/question/allow-once"
        );

        store.answer(SessionId(1), crate::model::Answer::AllowOnce);
        store.tick(Duration::from_secs(10));
        let done = finished(store.session(SessionId(1)).unwrap(), "studio-mac", 0);
        assert_eq!(done.title, "Qwen signatures is done");
        assert_eq!(done.text, "2 files changed · checks passed");
        assert_eq!(done.actions[0].0, "Review changes");
    }

    #[test]
    fn the_working_notification_names_each_session() {
        let store = Store::sample(Computer::from_address("nick@studio-mac.local"));
        let notification = working(store.running(), "studio-mac", 0).unwrap();
        assert_eq!(notification.title, "3 sessions working");
        assert!(notification.ongoing);
        assert!(
            notification
                .text
                .starts_with("Qwen signatures: editing a file")
        );
    }
}
