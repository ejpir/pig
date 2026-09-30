use pi_core::{
    protocol::Command,
    session::{Session, ShellExecution},
};
use serde_json::json;

#[test]
fn shell_protocol_is_explicit_long_running_and_context_policy_survives() {
    let command = Command::Bash {
        command: "printf '雪\\n'".into(),
        exclude_from_context: true,
    };
    let record = command.record("shell-1").unwrap();
    assert_eq!(
        record,
        json!({"type":"bash","id":"shell-1","command":"printf '雪\\n'","excludeFromContext":true})
    );
    assert!(command.replies_when_finished());
    assert_eq!(Command::AbortBash.name(), "abort_bash");
}

#[test]
fn streamed_shell_preview_is_bounded_correlated_and_final_history_is_sdk_owned() {
    let mut session = Session::default();
    let original = json!({"role":"user","content":"Original prompt","timestamp":1});
    session.messages.push(original.clone());
    session.shell = Some(ShellExecution {
        id: "shell-1".into(),
        command: "printf fixture".into(),
        exclude_from_context: true,
        output: String::new(),
        finished: false,
        result: None,
    });
    assert!(session.busy());
    assert_eq!(session.run_label(), "Shell running");
    session
        .apply(&json!({"type":"bash_execution_update","id":"other","delta":"not ours"}))
        .unwrap();
    assert!(session.shell.as_ref().unwrap().output.is_empty());
    session
        .apply(&json!({"type":"bash_execution_update","id":"shell-1","delta":"雪".repeat(50_000)}))
        .unwrap();
    assert!(session.shell.as_ref().unwrap().output.len() <= 64 * 1024);
    let result = json!({"output":"complete 雪\n","exitCode":7,"cancelled":false,"truncated":false});
    session.apply(&json!({"type":"response","id":"shell-1","command":"bash","success":true,"data":result})).unwrap();
    assert!(!session.busy());
    assert_eq!(
        session.messages,
        vec![original.clone()],
        "do not fabricate a persisted message or timestamp"
    );
    let bash = json!({"role":"bashExecution","command":"printf fixture","output":"complete 雪\n","exitCode":7,"cancelled":false,"truncated":false,"excludeFromContext":true,"timestamp":22});
    let data = json!({"messages":[original,bash.clone()]});
    assert!(session.shell_snapshot_appends(&data));
    session
        .apply(&json!({"type":"response","command":"get_messages","success":true,"data":data}))
        .unwrap();
    assert!(session.shell.is_none());
    assert_eq!(session.messages[1], bash);
    assert_eq!(session.tools.len(), 0);
}

#[test]
fn reported_external_shell_state_and_unknown_are_distinct() {
    let mut session = Session::default();
    assert_eq!(session.state.is_bash_running, None);
    session.apply(&json!({"type":"response","command":"get_state","success":true,"data":{"isBashRunning":true}})).unwrap();
    assert!(session.busy());
    session.apply(&json!({"type":"response","command":"get_state","success":true,"data":{"isBashRunning":false}})).unwrap();
    assert!(!session.busy());
    assert_eq!(session.state.is_bash_running, Some(false));
}
