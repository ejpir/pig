use pi_core::{protocol::SavedSession, session::Session};
use serde_json::json;

#[test]
fn unnamed_saved_session_keeps_its_title_during_and_after_hydration() {
    let saved: SavedSession = serde_json::from_value(
        json!({"id":"saved","path":"/saved.jsonl","cwd":"/project","firstMessage":"hello"}),
    )
    .unwrap();
    let mut session = Session::new("/project".into());
    session.preview_title = Some(saved.title().into());
    assert_eq!(session.title(), "hello");
    session.apply(&json!({"type":"response","command":"get_state","success":true,"data":{"sessionId":"saved","sessionName":null}})).unwrap();
    assert_eq!(session.title(), "hello");
    session.apply(&json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]}})).unwrap();
    assert_eq!(session.title(), saved.title());
    session
        .apply(&json!({"type":"session_info_changed","name":"Renamed"}))
        .unwrap();
    assert_eq!(session.title(), "Renamed");
    session
        .apply(&json!({"type":"session_info_changed","name":null}))
        .unwrap();
    assert_eq!(session.title(), "hello");
}

#[test]
fn new_session_titles_derive_from_first_nonempty_user_text_and_are_unicode_safe() {
    let mut session = Session::default();
    assert_eq!(session.title(), "New session");
    session.apply(&json!({"type":"message_end","message":{"role":"user","content":"  hello\nmore details"}})).unwrap();
    assert_eq!(session.title(), "hello");
    session
        .apply(&json!({"type":"message_end","message":{"role":"user","content":"a follow up"}}))
        .unwrap();
    assert_eq!(session.title(), "hello");
    let saved: SavedSession = serde_json::from_value(json!({"id":"unicode","path":"/unicode.jsonl","cwd":"/project","name":" ","firstMessage":"猫".repeat(100)})).unwrap();
    assert_eq!(saved.title().chars().count(), 80);
    assert_eq!(saved.title(), "猫".repeat(80));
}
