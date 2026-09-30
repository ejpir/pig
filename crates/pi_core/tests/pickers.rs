use pi_core::{protocol::Command, session::Session};
use serde_json::json;

#[test]
fn picker_choices_use_exact_session_local_commands() {
    let record = Command::SetModel {
        provider: "anthropic".into(),
        model_id: "claude-opus-5-5".into(),
        persist: false,
    }
    .record("model-1")
    .unwrap();
    assert_eq!(
        record,
        json!({"type":"set_model","id":"model-1","provider":"anthropic","modelId":"claude-opus-5-5"})
    );
    assert_eq!(
        Command::SetThinkingLevel {
            level: "high".into()
        }
        .record("level-1")
        .unwrap(),
        json!({"type":"set_thinking_level","id":"level-1","level":"high"})
    );
    assert_eq!(
        Command::GetAvailableThinkingLevels.name(),
        "get_available_thinking_levels"
    );
    assert_eq!(Command::GetAvailableModels.name(), "get_available_models");
    assert_eq!(Command::GetCommands.name(), "get_commands");
    assert_eq!(
        Command::Compact {
            custom_instructions: Some("keep the plan".into())
        }
        .record("c")
        .unwrap(),
        json!({"type":"compact","id":"c","customInstructions":"keep the plan"})
    );
    assert_eq!(
        Command::Compact {
            custom_instructions: None
        }
        .record("c")
        .unwrap(),
        json!({"type":"compact","id":"c"})
    );
    assert_eq!(
        Command::SetSessionName {
            name: "Release".into(),
            session_path: None,
        }
        .record("n")
        .unwrap(),
        json!({"type":"set_session_name","id":"n","name":"Release"})
    );
    assert_eq!(
        Command::Reload.record("r").unwrap(),
        json!({"type":"reload","id":"r"})
    );
}

#[test]
fn picker_metadata_is_authoritative_and_does_not_change_the_active_selection() {
    let mut session = Session::default();
    session.apply(&json!({"type":"response","command":"get_available_thinking_levels","success":true,"data":{"levels":["off","low","high"]}})).unwrap();
    assert_eq!(session.thinking_levels, ["off", "low", "high"]);
    assert!(
        !session
            .thinking_levels
            .iter()
            .any(|level| level == "xhigh" || level == "max")
    );
    session.apply(&json!({"type":"response","command":"get_available_models","success":true,"data":{"models":[{"id":"same-id","provider":"a"},{"id":"same-id","provider":"b"}]}})).unwrap();
    assert_eq!(session.available_models.len(), 2);
    assert!(session.state.model.is_none());
    session.apply(&json!({"type":"response","command":"set_model","success":true,"data":{"id":"same-id","provider":"b"}})).unwrap();
    assert_eq!(session.state.model.as_ref().unwrap().provider, "b");
    session.apply(&json!({"type":"response","command":"get_commands","success":true,"data":{"commands":[{"name":"skill:check","source":"skill"}]}})).unwrap();
    assert_eq!(session.commands[0].name, "skill:check");
    assert!(session.messages.is_empty());
}
