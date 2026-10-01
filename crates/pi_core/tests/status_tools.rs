use pi_core::session::Session;
use serde_json::json;

const METRICS: &str =
    "TPS 87.3 tok/s. out 2,344, in 8,132, cache r/w 13,824/0, total 24,300, 26.9s";

#[test]
fn tps_info_is_quiet_verbatim_and_does_not_replace_a_notice() {
    let mut session = Session::default();
    session.apply(&json!({"type":"extension_ui_request","method":"notify","message":"Resources reloaded."})).unwrap();
    session.apply(&json!({"type":"extension_ui_request","method":"notify","notifyType":"info","message":METRICS})).unwrap();
    assert_eq!(session.turn_metrics.as_deref(), Some(METRICS));
    assert_eq!(session.notice.as_deref(), Some("Resources reloaded."));
    let updated = METRICS.replace("87.3", "90.1");
    session
        .apply(&json!({"type":"extension_ui_request","method":"notify","message":updated}))
        .unwrap();
    assert_eq!(session.turn_metrics.as_deref(), Some(updated.as_str()));
    assert!(session.messages.is_empty());
    session.apply(&json!({"type":"agent_start"})).unwrap();
    assert!(session.turn_metrics.is_none());
    assert_eq!(session.notice.as_deref(), Some("Resources reloaded."));
}

#[test]
fn warnings_errors_and_unrelated_tps_text_still_raise_notices() {
    let mut session = Session::default();
    for severity in ["warning", "error"] {
        session.apply(&json!({"type":"extension_ui_request","method":"notify","notifyType":severity,"message":METRICS})).unwrap();
        assert_eq!(session.notice.as_deref(), Some(METRICS));
        assert!(session.turn_metrics.is_none());
    }
    for message in [
        "TPS connection failed",
        "TPS NaN tok/s. out 1, in 1, cache r/w 0/0, total 2, 1s",
        &format!("{METRICS}\nDo not ignore this warning"),
    ] {
        session.apply(&json!({"type":"extension_ui_request","method":"notify","notifyType":"info","message":message})).unwrap();
        assert_eq!(session.notice.as_deref(), Some(message));
        assert!(session.turn_metrics.is_none());
    }
}

#[test]
fn active_tool_metadata_is_reported_not_inferred_from_history() {
    let mut session = Session::default();
    session
        .apply(&json!({"type":"response","command":"get_state","success":true,"data":{}}))
        .unwrap();
    assert!(session.state.active_tools.is_none(), "not reported yet");
    session.apply(&json!({"type":"response","command":"get_active_tools","success":true,"data":{"activeTools":[]}})).unwrap();
    assert!(session.state.active_tools.as_ref().unwrap().is_empty());
    session.apply(&json!({"type":"response","command":"get_active_tools","success":true,"data":{"activeTools":[{"name":"custom","description":"Original description\nsecond line","sourceInfo":{"path":"/fixture/tool.ts","source":"local","scope":"project","origin":"top-level"}}]}})).unwrap();
    let tool = &session.state.active_tools.as_ref().unwrap()[0];
    assert_eq!(tool.name, "custom");
    assert_eq!(
        tool.description.as_deref(),
        Some("Original description\nsecond line")
    );
    assert_eq!(tool.source_info.as_ref().unwrap().path, "/fixture/tool.ts");
    session.apply(&json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"toolCall","id":"old","name":"historical-only","arguments":{}}]}})).unwrap();
    assert_eq!(session.tools[0].name, "historical-only");
    assert_eq!(
        session.state.active_tools.as_ref().unwrap()[0].name,
        "custom"
    );
    // pi's state has no tools: it keeps the last report until the next one.
    session
        .apply(&json!({"type":"response","command":"get_state","success":true,"data":{"sessionId":"next"}}))
        .unwrap();
    assert_eq!(session.state.session_id.as_deref(), Some("next"));
    assert_eq!(
        session.state.active_tools.as_ref().unwrap()[0].name,
        "custom"
    );
    session.apply(&json!({"type":"response","command":"get_active_tools","success":true,"data":{"activeTools":[]}})).unwrap();
    assert!(session.state.active_tools.as_ref().unwrap().is_empty());
}
