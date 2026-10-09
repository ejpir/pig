use pi_core::{
    protocol::{Command, ImageContent, StreamingBehavior, read_record},
    session::{BackendInfo, RunState, Session},
};
use serde_json::json;
use std::io::{BufReader, Cursor};

#[test]
fn jsonl_survives_byte_boundaries_crlf_and_unicode_separators() {
    let wire = "{\"type\":\"event\",\"text\":\"🐈\u{2028}line\u{2029}paragraph\"}\r\n{\"type\":\"next\"}\n";
    let mut reader = BufReader::with_capacity(1, Cursor::new(wire));
    assert_eq!(
        read_record(&mut reader).unwrap().unwrap()["text"],
        "🐈\u{2028}line\u{2029}paragraph"
    );
    assert_eq!(read_record(&mut reader).unwrap().unwrap()["type"], "next");
    assert!(read_record(&mut reader).unwrap().is_none());
}

#[test]
fn invalid_and_unterminated_records_are_errors() {
    for input in [
        "{}\n",
        "[]\n",
        "not-json\n",
        "{\"type\":\"x\"}",
        "{\"type\":1}\n",
    ] {
        assert!(read_record(&mut Cursor::new(input)).is_err(), "{input:?}");
    }
}

#[test]
fn commands_use_exact_pi_wire_names_and_preserve_text() {
    assert_eq!(
        Command::Prompt {
            message: "hello\n🐈".into(),
            images: vec![],
            streaming_behavior: Some(StreamingBehavior::FollowUp)
        }
        .record("id")
        .unwrap(),
        json!({"id":"id","type":"prompt","message":"hello\n🐈","streamingBehavior":"followUp"})
    );
    // Images go as pi's ImageContent.
    assert_eq!(
        Command::Prompt {
            message: "What is this?".into(),
            images: vec![ImageContent::new("iVBORw0K".into(), "image/png")],
            streaming_behavior: None
        }
        .record("id")
        .unwrap()["images"],
        json!([{"type":"image","data":"iVBORw0K","mimeType":"image/png"}])
    );
    assert_eq!(
        Command::GetSessionStats.record("1").unwrap()["type"],
        "get_session_stats"
    );
    let session = Session::default();
    assert_eq!(
        session
            .prompt("hi".into(), vec![], false)
            .record("2")
            .unwrap()["streamingBehavior"],
        "steer"
    );
    assert_eq!(
        session
            .prompt("later".into(), vec![], true)
            .record("3")
            .unwrap()["streamingBehavior"],
        "followUp"
    );
}

#[test]
fn backend_info_is_optional_and_never_an_error() {
    assert_eq!(
        Command::GetBackendInfo.record("1").unwrap()["type"],
        "get_backend_info"
    );
    let mut session = Session::default();
    session
        .apply(&json!({"type":"response","id":"1","command":"get_backend_info","success":false,"error":"Unknown command: get_backend_info"}))
        .unwrap();
    assert_eq!(session.backend, BackendInfo::Unsupported);
    assert_eq!(
        session.error, None,
        "plain pi RPC does not know the command"
    );
    session
        .apply(&json!({"type":"response","id":"2","command":"get_backend_info","success":true,"data":{"piVersion":"0.87.1"}}))
        .unwrap();
    assert_eq!(
        session.backend,
        BackendInfo::Found(json!({"piVersion":"0.87.1"}))
    );
}

#[test]
fn streaming_uses_block_indices_and_authoritative_final_message() {
    let mut session = Session::default();
    for record in [
        json!({"type":"agent_start"}),
        json!({"type":"message_start","message":{"role":"assistant","content":[]}}),
        json!({"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","contentIndex":0,"delta":"Consider…"}}),
        json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":1,"delta":"Hel"}}),
        json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":1,"delta":"lo"}}),
    ] {
        session.apply(&record).unwrap();
    }
    assert_eq!(session.messages[0]["content"][0]["thinking"], "Consider…");
    assert_eq!(session.messages[0]["content"][1]["text"], "Hello");
    session.apply(&json!({"type":"message_update","assistantMessageEvent":{"type":"text_end","contentIndex":1,"content":"Hello!"}})).unwrap();
    assert_eq!(session.messages[0]["content"][1]["text"], "Hello!");
    session.apply(&json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Final"}]}})).unwrap();
    assert_eq!(session.messages.len(), 1);
    assert_eq!(session.messages[0]["content"][0]["text"], "Final");
}

#[test]
fn only_settled_ends_a_run_not_agent_end_or_acknowledgement() {
    let mut session = Session::default();
    session.apply(&json!({"type":"agent_start"})).unwrap();
    session
        .apply(&json!({"type":"agent_end","willRetry":true}))
        .unwrap();
    assert!(session.busy());
    session.apply(&json!({"type":"auto_retry_start"})).unwrap();
    assert_eq!(session.run, RunState::Retrying);
    session.apply(&json!({"type":"agent_settled"})).unwrap();
    for disposition in ["started", "queued", "handled"] {
        session.apply(&json!({"type":"response","command":"prompt","success":true,"data":{"disposition":disposition}})).unwrap();
        assert!(
            !session.busy(),
            "a late acknowledgement must not restart a settled run"
        );
    }
}

#[test]
fn tool_results_replace_partial_output_and_only_successful_writes_are_changes() {
    let mut session = Session::default();
    session.apply(&json!({"type":"tool_execution_start","toolCallId":"a","toolName":"edit","args":{"path":"src/a.rs"}})).unwrap();
    session.apply(&json!({"type":"tool_execution_update","toolCallId":"a","partialResult":{"content":[{"type":"text","text":"partial"}]}})).unwrap();
    assert!(session.changed_files().is_empty());
    session.apply(&json!({"type":"tool_execution_end","toolCallId":"a","result":{"content":[{"type":"text","text":"final"}],"details":{"diff":"+1 added"}},"isError":false})).unwrap();
    assert_eq!(session.tools[0].output, "final");
    assert_eq!(session.tools[0].diff.as_deref(), Some("+1 added"));
    assert_eq!(session.changed_files(), ["src/a.rs"]);
    session.apply(&json!({"type":"tool_execution_end","toolCallId":"a","result":{"content":[]},"isError":true})).unwrap();
    assert!(session.changed_files().is_empty());
}

#[test]
fn history_reconstructs_tool_cards_without_duplicating_results() {
    let mut session = Session::default();
    session.apply(&json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
        {"role":"user","content":"edit it"},
        {"role":"assistant","content":[{"type":"toolCall","id":"t","name":"write","arguments":{"path":"a.rs"}}]},
        {"role":"toolResult","toolCallId":"t","toolName":"write","content":[{"type":"text","text":"done"}],"isError":false}
    ]}})).unwrap();
    assert_eq!(session.messages.len(), 2);
    assert_eq!(session.tools.len(), 1);
    assert!(session.tools[0].finished);
    assert_eq!(session.changed_files(), ["a.rs"]);
}

#[test]
fn queues_are_authoritative_and_unknown_events_are_tolerated() {
    let mut session = Session::default();
    session
        .apply(&json!({"type":"queue_update","steering":["one"],"followUp":["two"]}))
        .unwrap();
    session
        .apply(&json!({"type":"future_event","payload":42}))
        .unwrap();
    session.apply(&json!({"type":"response","command":"follow_up","success":true,"data":{"disposition":"handled"}})).unwrap();
    assert_eq!(session.follow_up, ["two"]);
    session
        .apply(&json!({"type":"queue_update","steering":[],"followUp":[]}))
        .unwrap();
    assert!(session.follow_up.is_empty());
}

#[test]
fn context_counts_are_never_derived_from_lifetime_token_totals() {
    let mut session = Session::default();
    session.apply(&json!({"type":"response","command":"get_session_stats","success":true,"data":{"tokens":{"input":900000,"output":20000,"cacheRead":400000},"cost":0.41,"contextUsage":{"tokens":null,"contextWindow":200000,"percent":null}}})).unwrap();
    assert_eq!(session.stats.context_usage.as_ref().unwrap().tokens, None);
    assert_eq!(session.stats.context_usage.as_ref().unwrap().percent, None);
    session
        .apply(&json!({"type":"compaction_end","result":{"summary":"compacted"}}))
        .unwrap();
    assert!(session.stats.context_usage.is_none());
}

#[test]
fn fixture_uses_the_same_reducer_as_live_rpc() {
    let mut reader = Cursor::new(include_bytes!("../../../fixtures/thread.jsonl"));
    let mut session = Session::default();
    while let Some(record) = read_record(&mut reader).unwrap() {
        session.apply(&record).unwrap();
    }
    assert_eq!(session.title(), "Qwen signatures");
    assert_eq!(session.tools.len(), 4);
    assert_eq!(session.changed_files().len(), 1);
    assert_eq!(session.stats.context_usage.unwrap().tokens, Some(62400));
    assert_eq!(session.follow_up.len(), 1);
    assert_eq!(session.saved.len(), 10);
}

#[test]
fn only_a_live_failure_raises_the_session_error() {
    let mut session = Session::default();
    let aborted = json!({"role":"assistant","content":[],"stopReason":"aborted","errorMessage":"Operation aborted"});
    let failed = json!({"role":"assistant","content":[],"stopReason":"error","errorMessage":"529 overloaded"});
    // History: past stops and failures do not become the banner for a resumed session.
    session.apply(&json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[aborted, failed]}})).unwrap();
    assert_eq!(session.messages.len(), 2);
    assert!(session.error.is_none());
    session
        .apply(&json!({"type":"message_end","message":aborted}))
        .unwrap();
    assert!(session.error.is_none(), "stopping a run is not an error");
    session
        .apply(&json!({"type":"message_end","message":failed}))
        .unwrap();
    assert_eq!(session.error.as_deref(), Some("529 overloaded"));
}

#[test]
fn custom_messages_appended_as_a_run_settles_join_the_transcript() {
    let mut session = Session::default();
    session.apply(&json!({"type":"entry_appended","entry":{"type":"custom_message","id":"e1","customType":"pi-desktop-lsp","content":"Language server errors that appeared during this run:","display":true,"timestamp":"2026-09-30T10:00:00.000Z"}})).unwrap();
    // Other entries are not messages.
    session
        .apply(&json!({"type":"entry_appended","entry":{"type":"custom","id":"e2","customType":"state"}}))
        .unwrap();
    assert_eq!(
        session.messages,
        [
            // The time in ms, as pi's `get_messages` gives it: turn lines anchor by it.
            json!({"role":"custom","customType":"pi-desktop-lsp","content":"Language server errors that appeared during this run:","display":true,"details":null,"timestamp":1790762400000_u64})
        ]
    );
}
