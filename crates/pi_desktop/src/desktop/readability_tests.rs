use super::*;

fn activity_session(desktop: &Entity<Desktop>, cx: &mut VisualTestContext) -> Tab {
    let id = open(desktop, "/demo/activity", None, cx);
    let session = tab(desktop, id, cx);
    for event in [
        json!({"type":"agent_start"}),
        json!({"type":"message_end","message":{"role":"user","content":"Keep everything full width"}}),
        json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"thinking","thinking":"First thought"},{"type":"toolCall","id":"one","name":"read","arguments":{"path":"one.txt"}}]}}),
        json!({"type":"tool_execution_end","toolCallId":"one","result":{"content":[{"type":"text","text":"one output"}]}}),
        json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"thinking","thinking":"Second thought"},{"type":"toolCall","id":"two","name":"bash","arguments":{"command":"echo two"}}]}}),
        json!({"type":"tool_execution_end","toolCallId":"two","result":{"content":[{"type":"text","text":"two output"}]}}),
        json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"An answer, without a fixed width cap."}]}}),
    ] {
        receive(&session, event, cx);
    }
    cx.run_until_parked();
    session
}

#[gpui::test]
fn completed_activity_collapses_and_only_prose_has_a_reading_measure(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = activity_session(&desktop, &mut cx);
    assert!(cx.debug_bounds("work-status").is_some());
    assert!(cx.debug_bounds("activity-1-0-passed").is_none());
    assert!(cx.debug_bounds("tool-header-one").is_none());
    receive(&session, json!({"type":"agent_settled"}), &mut cx);
    cx.run_until_parked();
    let activity = cx.debug_bounds("activity-1-0").unwrap();
    assert!(cx.debug_bounds("activity-1-0-passed").is_some());
    assert!(cx.debug_bounds("activity-1-0-failed").is_none());
    // Reading and running are separate steps on the rail; settled steps collapse
    // to chips of what they touched.
    assert!(cx.debug_bounds("activity-2-0-passed").is_some());
    assert!(cx.debug_bounds("step-chip-one").is_some());
    assert!(cx.debug_bounds("tool-header-one").is_none());
    assert!(cx.debug_bounds("tool-header-two").is_none());
    assert!(cx.debug_bounds("working").is_none()); // no duplicate transcript status
    assert_eq!(
        cx.debug_bounds("composer").unwrap().size.height,
        px(112.),
        "an empty settled composer stays compact"
    );
    session.transcript.read_with(&cx, |view, _| {
        assert!(view.documents.get("1:thinking-0").is_none());
        assert!(view.documents.get("tool:two:output").is_none());
    });
    let prose = cx.debug_bounds("assistant-prose-3-0").unwrap();
    assert!(prose.size.width > px(700.));
    cx.simulate_click(activity.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("tool-header-one").is_some());
    assert!(cx.debug_bounds("step-chip-one").is_none());
    assert!(cx.debug_bounds("tool-header-two").is_none());
    click("activity-2-0", &mut cx);
    assert!(cx.debug_bounds("tool-header-two").is_some());
    let one = cx.debug_bounds("tool-header-one").unwrap();
    let two = cx.debug_bounds("tool-header-two").unwrap();
    assert!(one.origin.y < two.origin.y);
    cx.simulate_resize(size(px(1600.), px(900.)));
    cx.run_until_parked();
    assert_eq!(
        cx.debug_bounds("assistant-prose-3-0").unwrap().size.width,
        px(760.)
    );
    assert_eq!(prose.size.width, px(760.));
    assert!(cx.debug_bounds("composer").unwrap().size.width > px(1300.));
}

#[gpui::test]
fn manual_tool_disclosure_is_not_closed_when_a_run_settles(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = activity_session(&desktop, &mut cx);
    let activity = cx.debug_bounds("activity-2-0").unwrap();
    cx.simulate_click(activity.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let tool = cx.debug_bounds("tool-header-two").unwrap();
    cx.simulate_click(tool.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    receive(&session, json!({"type":"agent_settled"}), &mut cx);
    cx.run_until_parked();
    assert!(cx.debug_bounds("tool-details-two").is_some());
    let activity = cx.debug_bounds("activity-2-0").unwrap();
    cx.simulate_click(activity.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("tool-header-two").is_none());
    session.transcript.read_with(&cx, |view, _| {
        assert!(view.documents.get("tool:two:output").is_none())
    });
    // Navigation from Changes must open both the activity and the requested tool.
    session
        .controller
        .update(&mut cx, |c, cx| c.reveal_tool("two".into(), cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("tool-details-two").is_some());
}

#[gpui::test]
fn failed_activity_collapses_and_hidden_sessions_do_not_share_disclosures(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = activity_session(&desktop, &mut cx);
    receive(
        &session,
        json!({"type":"tool_execution_end","toolCallId":"two","isError":true,"result":{"content":[{"type":"text","text":"Command failed"}]}}),
        &mut cx,
    );
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("tool-header-two").is_some(),
        "live failures stay visible"
    );
    receive(&session, json!({"type":"agent_settled"}), &mut cx);
    cx.run_until_parked();
    assert!(cx.debug_bounds("activity-2-0-failed").is_some());
    assert!(cx.debug_bounds("activity-2-0-passed").is_none());
    assert!(
        cx.debug_bounds("activity-1-0-passed").is_some(),
        "only the failed step"
    );
    assert!(cx.debug_bounds("tool-header-two").is_none());
    assert!(cx.debug_bounds("assistant-prose-3-0").is_some());
    assert!(cx.debug_bounds("thread-result-issue").is_some());
    assert!(cx.debug_bounds("work-status").is_none());
    assert_eq!(cx.debug_bounds("composer").unwrap().size.height, px(112.));
    let review = cx.debug_bounds("review-failed-tool").unwrap();
    cx.simulate_click(review.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("tool-details-two").is_some());
    session.controller.read_with(&cx, |controller, _| {
        let failed = controller
            .model()
            .tools
            .iter()
            .find(|tool| tool.id == "two")
            .unwrap();
        assert!(failed.is_error);
        assert!(failed.output.contains("Command failed"));
    });
    let second = activity_session(&desktop, &mut cx);
    receive(&second, json!({"type":"agent_settled"}), &mut cx);
    cx.run_until_parked();
    assert!(cx.debug_bounds("tool-header-two").is_none());
}

#[gpui::test]
fn manually_expanded_failure_stays_open_after_settlement(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = activity_session(&desktop, &mut cx);
    receive(
        &session,
        json!({"type":"tool_execution_end","toolCallId":"two","isError":true,"result":{"content":[{"type":"text","text":"Command failed"}]}}),
        &mut cx,
    );
    cx.run_until_parked();
    let tool = cx.debug_bounds("tool-header-two").unwrap();
    cx.simulate_click(tool.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    receive(&session, json!({"type":"agent_settled"}), &mut cx);
    cx.run_until_parked();
    assert!(cx.debug_bounds("tool-details-two").is_some());
    assert!(cx.debug_bounds("activity-2-0-failed").is_some());
}

#[gpui::test]
fn changes_wrap_long_lines_and_contain_call_ids_without_changing_raw_copy(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let id = open(&desktop, "/demo/wrapping", None, &mut cx);
    let session = tab(&desktop, id, &cx);
    let call_id = format!("call_{}", "long-id-".repeat(30));
    let source = "長い行 abcdefghijklmnopqrstuvwxyz ".repeat(20);
    receive(
        &session,
        json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"toolCall","id":call_id,"name":"write","arguments":{"path":"long.txt","content":source}}]}}),
        &mut cx,
    );
    receive(
        &session,
        json!({"type":"tool_execution_end","toolCallId":call_id,"result":{"content":[{"type":"text","text":"written"}]}}),
        &mut cx,
    );
    let changes = cx.debug_bounds("tab-changes").unwrap();
    cx.simulate_click(changes.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let call = cx.debug_bounds("change-tool-0").unwrap();
    assert!(call.origin.x + call.size.width <= px(1344.));
    let mode = cx.debug_bounds("changes-diff-mode").unwrap();
    cx.simulate_click(mode.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let wrapped = cx.debug_bounds("changes-document").unwrap();
    let toggle = cx.debug_bounds("changes-wrap").unwrap();
    cx.simulate_click(toggle.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("changes-document").unwrap().size.height < wrapped.size.height);
    let copy = cx.debug_bounds("changes-copy").unwrap();
    cx.simulate_click(copy.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let copied = cx.read(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert!(copied.contains(&source));
    assert!(copied.contains(&call_id));
}

#[gpui::test]
fn diagnostics_keep_the_error_tail_after_the_banner_is_dismissed(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    session.controller.update(&mut cx, |c,cx| c.receive(TransportEvent::Exited {
        description: "RPC process exited: exit status: 1".into(),
        stderr: "node:events:487\nUnhandled error\nError: EACCES opening saved session\nAuthorization: Bearer private-token".into(),
    },cx));
    cx.run_until_parked();
    let details = cx.debug_bounds("view-error-details").unwrap();
    cx.simulate_click(details.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("session-diagnostics").is_some());
    session
        .controller
        .update(&mut cx, |c, cx| c.dismiss_error(cx));
    cx.run_until_parked();
    let copy = cx.debug_bounds("copy-diagnostics").unwrap();
    cx.simulate_click(copy.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let text = cx.read(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert!(text.contains("EACCES opening saved session"));
    assert!(text.contains("exit status: 1"));
    assert!(!text.contains("private-token"));
}

#[gpui::test]
fn context_details_fit_inside_their_tiles_even_when_narrow(cx: &mut TestAppContext) {
    let (_desktop, mut cx) = setup(cx);
    click("tab-context", &mut cx);
    for width in [1344., 1000.] {
        cx.simulate_resize(size(px(width), px(740.)));
        cx.run_until_parked();
        for (name, detail_name) in [
            ("context-tile-context", "context-detail-context"),
            ("context-tile-cost", "context-detail-cost"),
            ("context-tile-cache read", "context-detail-cache read"),
            ("context-tile-compactions", "context-detail-compactions"),
        ] {
            let tile = cx.debug_bounds(name).unwrap();
            let detail = cx.debug_bounds(detail_name).unwrap();
            assert!(
                detail.origin.y + detail.size.height <= tile.origin.y + tile.size.height - px(12.),
                "{name}: {detail:?} escapes {tile:?}"
            );
        }
    }
}
