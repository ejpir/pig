//! Notifications and the `pi://` links they open. A notification is an entry
//! point: it names the session and the computer, and opens the exact place
//! to act. Answering from one still opens the app, so the lock screen asks
//! to unlock first.

use crate::{
    model::{Answer, Mark, Session, SessionId, StageStatus, State, StretchKind, Turn},
    remote::HelperUpdate,
};
use gpui_android::activity::{Channel, Importance, Notification, Progress};

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

pub const HELPER_UPDATES: Channel = Channel {
    id: "helper_updates",
    name: "Computer helper updates",
    importance: Importance::Default,
};

/// The one ongoing notification while sessions work.
pub const WORKING_ID: i32 = 1;
pub const HELPER_UPDATE_ID: i32 = 2;

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
    /// Answers yes ("Allow once"), then shows the session.
    AllowOnce(SessionId),
    /// Answers no, then shows the session.
    Decline(SessionId),
    Review(SessionId),
}

impl Link {
    pub fn url(self) -> String {
        match self {
            Self::Session(id) => format!("pi://session/{}", id.0),
            Self::Question(id) => format!("pi://session/{}/question", id.0),
            Self::AllowOnce(id) => format!("pi://session/{}/question/allow-once", id.0),
            Self::Decline(id) => format!("pi://session/{}/question/no", id.0),
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
            (Some("question"), Some("no")) => Self::Decline(id),
            (Some("review"), None) => Self::Review(id),
            _ => return None,
        };
        parts.next().is_none().then_some(link)
    }
}

/// Pi's question. A yes-or-no question is answered from the notification,
/// after unlocking; any other opens with its choices.
pub fn question(session: &Session, computer: &str, accent: u32) -> Option<Notification> {
    let question = session.question.as_ref()?;
    let detail = if question.command.is_empty() {
        &question.body
    } else {
        &question.command
    };
    let label = |wanted: Answer| {
        question
            .choices
            .iter()
            .find(|choice| choice.answer == wanted)
            .map(|choice| choice.label.clone())
    };
    let actions = match (
        crate::screens::is_yes_no(question),
        label(Answer::Deny),
        label(Answer::AllowOnce),
    ) {
        (true, Some(no), Some(yes)) => vec![
            (no, Link::Decline(session.id).url()),
            (yes, Link::AllowOnce(session.id).url()),
        ],
        _ => vec![("Choose".into(), Link::Question(session.id).url())],
    };
    Some(Notification {
        id: question_id(session.id),
        channel: QUESTIONS,
        title: format!("{} needs you", session.title),
        text: if detail.is_empty() {
            question.title.clone()
        } else {
            format!("{} {detail}", question.title)
        },
        subtext: Some(computer.to_owned()),
        url: Link::Question(session.id).url(),
        actions,
        ongoing: false,
        color: accent,
        progress: None,
        promoted: false,
        started: None,
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
        progress: None,
        promoted: false,
        started: None,
    }
}

pub fn helper_update(update: &HelperUpdate, computer: &str, accent: u32) -> Notification {
    Notification {
        id: HELPER_UPDATE_ID,
        channel: HELPER_UPDATES,
        title: update.title().into(),
        text: update.details(computer),
        subtext: Some(computer.to_owned()),
        url: "pi://helper-update".into(),
        actions: vec![("Open Pi".into(), "pi://helper-update".into())],
        ongoing: false,
        color: accent,
        progress: None,
        promoted: false,
        started: None,
    }
}

/// The run that moved last, ongoing while the app is away: on Android 16 a
/// Live Update, its run line drawn as the progress bar, on the lock screen and
/// as a status bar chip counting up. More than one working says how many more.
pub fn working<'a>(
    sessions: impl Iterator<Item = &'a Session>,
    computer: &str,
    accent: u32,
) -> Option<Notification> {
    let sessions: Vec<_> = sessions.collect();
    let first = sessions.first()?;
    let stage = first
        .turn()
        .and_then(|turn| turn.live_stage())
        .map(|kind| kind.name(StageStatus::Live).to_lowercase());
    let title = match stage {
        Some(stage) => format!("{} · {stage}", first.title),
        None => first.title.clone(),
    };
    let mut text = capitalized(&first.activity);
    if sessions.len() > 1 {
        let more = sessions.len() - 1;
        text = format!("{text} · {more} more working");
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
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
        progress: first.turn().map(run_progress),
        promoted: true,
        // Whole seconds, so reposting doesn't nudge the timer.
        started: Some(now.saturating_sub(first.elapsed).as_secs() * 1000),
    })
}

/// How long a step of the notification's bar is: the bar moves in five-second
/// steps, so it isn't posted again every second.
const STEP: u64 = 5;

/// A run's line as Android 16 draws a journey: each stretch a segment in its
/// colour, a point where a check passed or failed, and the rest of a
/// five-minute track (doubling as the run goes on) faded, still to come. The
/// colours are Moonstone's, which read on a light or a dark shade.
pub fn run_progress(turn: &Turn) -> Progress {
    let hue = |kind: StretchKind| match kind {
        StretchKind::Look => 0x2f7fa8,
        StretchKind::Change => 0xb0661a,
        StretchKind::Check => 0x949ca6,
        StretchKind::Wait => 0xa57514,
    };
    let mut progress = Progress::default();
    for stretch in &turn.run {
        let steps = stretch.time.as_secs().div_ceil(STEP).max(1) as u32;
        progress.segments.push((steps, hue(stretch.kind)));
        progress.position += steps;
        match stretch.mark {
            Some(Mark::Passed) => progress.points.push((progress.position, 0x2e7950)),
            Some(Mark::Failed) => progress.points.push((progress.position, 0xa44835)),
            Some(Mark::Answered) | None => {}
        }
    }
    let mut track = (300 / STEP) as u32;
    while track < progress.position {
        track *= 2;
    }
    if track > progress.position {
        progress
            .segments
            .push((track - progress.position, 0xddd7d0));
    }
    progress
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
            Link::Decline(SessionId(3)),
            Link::Review(SessionId(12)),
        ] {
            assert_eq!(Link::parse(&link.url()), Some(link));
        }
        assert_eq!(Link::parse("pi://session/x"), None);
        assert_eq!(Link::parse("pi://session/3/delete"), None);
        assert_eq!(Link::parse("https://example.com"), None);
    }

    #[test]
    fn a_question_opens_its_choices_or_answers_yes_or_no() {
        let mut store = Store::sample(Computer::from_address("nick@studio-mac.local"));
        store.tick(Duration::from_secs(10));
        let session = store.session(SessionId(1)).unwrap();
        let notification = question(session, "studio-mac", 0x4b607c).unwrap();
        assert_eq!(notification.title, "Qwen signatures needs you");
        assert_eq!(
            notification.text,
            "Run the provider tests? pnpm test --filter @pi/ai -- qwen"
        );
        // Three choices: the notification opens them.
        assert_eq!(
            notification.actions,
            [("Choose".to_owned(), "pi://session/1/question".to_owned())]
        );
        // Yes or no: answered from the notification, after unlocking.
        let mut asked = session.clone();
        asked.question = crate::projection::question(&serde_json::json!({
            "id": "q", "method": "confirm", "title": "Clear the build cache?",
            "message": "rm -rf target/debug/incremental"
        }));
        let notification = question(&asked, "studio-mac", 0x4b607c).unwrap();
        assert_eq!(
            notification.text,
            "Clear the build cache? rm -rf target/debug/incremental"
        );
        assert_eq!(
            notification.actions,
            [
                ("No".to_owned(), "pi://session/1/question/no".to_owned()),
                (
                    "Yes".to_owned(),
                    "pi://session/1/question/allow-once".to_owned()
                ),
            ]
        );

        store.answer(SessionId(1), crate::model::Answer::AllowOnce);
        store.tick(Duration::from_secs(10));
        let done = finished(store.session(SessionId(1)).unwrap(), "studio-mac", 0);
        assert_eq!(done.title, "Qwen signatures is done");
        assert_eq!(done.text, "2 files changed · checks passed");
        assert_eq!(done.actions[0].0, "Review changes");
    }

    #[test]
    fn the_working_notification_is_the_run_that_moved_last() {
        let store = Store::sample(Computer::from_address("nick@studio-mac.local"));
        let notification = working(store.inbox().working.into_iter(), "studio-mac", 0).unwrap();
        assert_eq!(notification.title, "Gutter blame width · checking");
        assert_eq!(notification.text, "Checking · cargo test · 2 more working");
        assert!(notification.ongoing && notification.promoted);
        assert!(notification.started.is_some());
        let progress = notification.progress.unwrap();
        // 12, 24 and 16 seconds in five-second steps, then the rest of five
        // minutes still to come.
        assert_eq!(
            progress.segments,
            [(3, 0x2f7fa8), (5, 0xb0661a), (4, 0x949ca6), (48, 0xddd7d0)]
        );
        assert_eq!(progress.position, 12);
        assert!(progress.points.is_empty());
    }

    #[test]
    fn the_bar_marks_each_checks_result_and_doubles_its_track() {
        let mut turn = Turn::new("", "");
        for (kind, seconds, mark) in [
            (StretchKind::Look, 60, None),
            (StretchKind::Change, 150, None),
            (StretchKind::Check, 40, Some(Mark::Failed)),
            (StretchKind::Change, 50, None),
            (StretchKind::Check, 60, Some(Mark::Passed)),
            (StretchKind::Look, 28, None),
        ] {
            turn.add_time(kind, Duration::from_secs(seconds));
            if let Some(mark) = mark {
                turn.end_stretch(kind, mark);
            }
        }
        let progress = run_progress(&turn);
        assert_eq!(progress.position, 78);
        assert_eq!(progress.points, [(50, 0xa44835), (72, 0x2e7950)]);
        // Past five minutes, the track is ten.
        assert_eq!(progress.segments.last(), Some(&(42, 0xddd7d0)));
    }

    #[test]
    fn helper_notification_names_the_computer_and_manual_release() {
        let update = HelperUpdate {
            state: crate::remote::HelperUpdateState::UpdateRequired,
            installed_release: Some("0.0.4".into()),
            app_release: "0.0.5".into(),
            platform: Some("linux-arm64".into()),
            reasons: vec!["missing session discovery".into()],
        };
        let notification = helper_update(&update, "build-box", 0);
        assert_eq!(notification.title, "Helper update required");
        assert!(notification.text.contains("pi-desktop-remote 0.0.4"));
        assert!(notification.text.contains("Pi Android 0.0.5"));
        assert!(notification.text.contains("pi-desktop-remote-linux-arm64"));
        assert!(notification.text.contains("then reconnect"));
    }
}
