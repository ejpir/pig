//! Screen 1/2 behavior, not just mockup geometry.
use super::*;

fn review_fixture(session: &Tab, cx: &mut VisualTestContext) {
    receive(session, json!({"type":"agent_settled"}), cx);
    receive(
        session,
        json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[
            {"role":"user","content":"Make the provider guard narrow"},
            {"role":"assistant","content":[
                {"type":"toolCall","id":"review-a","name":"edit","arguments":{"path":"src/provider.ts","oldText":"old","newText":"new"}},
                {"type":"toolCall","id":"review-b","name":"edit","arguments":{"path":"test/provider.test.ts","oldText":"before","newText":"after"}}
            ]},
            {"role":"toolResult","toolCallId":"review-a","toolName":"edit","content":[{"type":"text","text":"ok"}],"details":{"diff":" 10 same\n-11 old\n+11 new\n+12 extra\n 12 end"}},
            {"role":"toolResult","toolCallId":"review-b","toolName":"edit","content":[{"type":"text","text":"ok"}],"details":{"diff":"-20 before\n+20 after"}},
            {"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"Only the provider guard changed. Review the test too."}]}
        ]}}),
        cx,
    );
    cx.run_until_parked();
}

#[gpui::test]
fn workbench_defaults_are_quiet_and_menus_keep_old_destinations_reachable(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    assert!(!desktop.read_with(&cx, |d, _| d.layout.inspector));
    assert!(cx.debug_bounds("inspector-usage").is_none());
    assert!(cx.debug_bounds("active-session-0").is_none());
    assert!(cx.debug_bounds("open-session-0").is_some());
    assert!(cx.debug_bounds("nav-Models").is_none());
    assert!(cx.debug_bounds("tab-tree").is_none());
    click("settings-tools", &mut cx);
    assert!(cx.debug_bounds("nav-Models").is_some());
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("settings-tools-menu").is_none());
    assert!(session.controller.read_with(&cx, |c, _| c.model().busy()));
    click("session-tools", &mut cx);
    let context = cx.debug_bounds("tab-context").unwrap();
    cx.simulate_mouse_move(context.center(), None, gpui::Modifiers::default());
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("context-view").is_some(),
        "hover must move the single selected menu row before keyboard activation"
    );
    click("tab-thread", &mut cx);
    click("settings-tools", &mut cx);
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    assert!(cx.debug_bounds("models-screen").is_some());
}

#[gpui::test]
fn global_search_does_not_filter_the_background_sidebar_or_consume_a_draft(
    cx: &mut TestAppContext,
) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    set_draft(&session, "Keep this exact draft", &mut cx);
    cx.simulate_keystrokes("secondary-k");
    cx.simulate_input("absent-search-result");
    cx.run_until_parked();
    assert!(cx.debug_bounds("open-session-0").is_some());
    assert!(
        cx.debug_bounds("global-search-empty").is_some(),
        "no matches is said plainly, not an empty heading"
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("global-search").is_none());
    assert_eq!(draft(&session, &cx), "Keep this exact draft");
    assert!(session.controller.read_with(&cx, |c, _| c.model().busy()));
}

#[gpui::test]
fn enter_steers_during_a_run_and_follow_up_is_explicit(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    set_draft(&session, "Change direction now", &mut cx);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(session.controller.read_with(&cx, |c, _| {
        c.model()
            .steering
            .iter()
            .any(|s| s == "Change direction now")
    }));
    set_draft(&session, "Do this next", &mut cx);
    click("queue-follow-up", &mut cx);
    cx.run_until_parked();
    assert!(session.controller.read_with(&cx, |c, _| {
        c.model().follow_up.iter().any(|s| s == "Do this next")
    }));
    set_draft(&session, "Unsent draft", &mut cx);
    click("stop", &mut cx);
    assert!(draft(&session, &cx).contains("Unsent draft"));
    assert!(draft(&session, &cx).contains("Change direction now"));
    assert!(draft(&session, &cx).contains("Do this next"));
    assert!(!session.controller.read_with(&cx, |c, _| c.model().busy()));
}

#[gpui::test]
fn review_uses_available_width_and_falls_back_when_the_actual_work_area_is_narrow(
    cx: &mut TestAppContext,
) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    review_fixture(&session, &mut cx);
    click("tab-changes", &mut cx);
    assert!(cx.debug_bounds("diff-before").is_some());
    assert!(cx.debug_bounds("diff-after").is_some());
    assert!(cx.debug_bounds("changes-open").is_some());
    assert!(cx.debug_bounds("changes-copy").is_some());
    assert!(cx.debug_bounds("changes-restore").is_none());
    assert!(cx.debug_bounds("tool-reported-explanation").is_some());
    let before = cx.debug_bounds("composer").unwrap();
    assert_eq!(before.left(), px(440.));
    assert_eq!(before.right(), px(1320.));
    cx.simulate_resize(size(px(1600.), px(900.)));
    cx.run_until_parked();
    let wide = cx.debug_bounds("composer").unwrap();
    assert_eq!(wide.left(), before.left());
    assert_eq!(wide.right(), px(1576.));
    assert_eq!(wide.size.width - before.size.width, px(256.));
    click("changes-diff-mode", &mut cx);
    assert!(cx.debug_bounds("diff-unified").is_some());
    click("changes-diff-mode", &mut cx);
    cx.simulate_resize(size(px(1000.), px(720.)));
    cx.run_until_parked();
    assert!(cx.debug_bounds("diff-before").is_none());
    assert!(cx.debug_bounds("diff-unified").is_some());
    assert!(cx.debug_bounds("changes-file-picker").is_some());
    let compact = cx.debug_bounds("composer").unwrap();
    assert_eq!(compact.left(), px(240.));
    assert_eq!(compact.right(), px(976.));
    click("changes-file-picker", &mut cx);
    click("review-file-option-1", &mut cx);
    assert!(cx.debug_bounds("changes-file-options").is_none());
    cx.simulate_resize(size(px(1600.), px(900.)));
    cx.run_until_parked();
    show_inspector(&desktop, &mut cx);
    // 1064px with inspector remains split; drag it wider to force unified.
    let handle = cx.debug_bounds("inspector-resize").unwrap().center();
    cx.simulate_mouse_down(handle, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(
        point(px(1080.), handle.y),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        point(px(1080.), handle.y),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    assert!(cx.debug_bounds("diff-before").is_none());
}

#[gpui::test]
fn revision_context_is_explicit_and_not_retargeted_by_file_or_session_selection(
    cx: &mut TestAppContext,
) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    review_fixture(&session, &mut cx);
    set_draft(&session, "Keep Anthropic strict", &mut cx);
    click("tab-changes", &mut cx);
    assert!(cx.debug_bounds("revision-context").is_none());
    click("changes-request-revision", &mut cx);
    let attached = session
        .composer
        .read_with(&cx, |v, _| v.revision.clone())
        .unwrap();
    assert_eq!(attached.path, "src/provider.ts");
    click("changed-file-1", &mut cx);
    assert_eq!(
        session.composer.read_with(&cx, |v, _| v.revision.clone()),
        Some(attached.clone())
    );
    assert_eq!(draft(&session, &cx), "Keep Anthropic strict");
    let other = open(&desktop, "/demo/revision-other", None, &mut cx);
    assert!(
        tab(&desktop, other, &cx)
            .composer
            .read_with(&cx, |v, _| v.revision.is_none())
    );
    select(&desktop, 0, &mut cx);
    assert_eq!(
        session.composer.read_with(&cx, |v, _| v.revision.clone()),
        Some(attached)
    );
    session
        .composer
        .update_in(&mut cx, |v, window, cx| v.focus(window, cx));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let sent = session.controller.read_with(&cx, |c, _| {
        c.model().messages.last().unwrap()["content"]
            .as_str()
            .unwrap()
            .to_owned()
    });
    assert!(sent.contains("\"src/provider.ts\""));
    assert!(sent.ends_with("Keep Anthropic strict"));
    assert!(session.composer.read_with(&cx, |v, _| v.revision.is_none()));
    assert_eq!(draft(&session, &cx), "");
}

#[gpui::test]
fn review_keyboard_selection_requires_an_explicit_revision_and_keeps_the_draft(
    cx: &mut TestAppContext,
) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    review_fixture(&session, &mut cx);
    set_draft(&session, "Preserve this draft", &mut cx);
    click("tab-changes", &mut cx);
    let before = session
        .controller
        .read_with(&cx, |c, _| c.model().messages.len());
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert!(session.composer.read_with(&cx, |v, _| v.revision.is_none()));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        session
            .composer
            .read_with(&cx, |v, _| v.revision.as_ref().unwrap().path.clone()),
        "test/provider.test.ts"
    );
    assert_eq!(
        session
            .controller
            .read_with(&cx, |c, _| c.model().messages.len()),
        before
    );
    assert_eq!(draft(&session, &cx), "Preserve this draft");
    assert!(cx.debug_bounds("revision-context").is_some());
    click("remove-revision", &mut cx);
    assert!(session.composer.read_with(&cx, |v, _| v.revision.is_none()));
    assert_eq!(draft(&session, &cx), "Preserve this draft");
}

#[gpui::test]
fn changes_refresh_live_and_copy_is_the_full_original_tool_report(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    review_fixture(&session, &mut cx);
    click("tab-changes", &mut cx);
    click("changes-copy", &mut cx);
    let copy = cx.read(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert!(copy.contains("review-a"));
    assert!(copy.contains(" 10 same\n-11 old\n+11 new"));
    assert!(!copy.contains("Before"));
    receive(
        &session,
        json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"toolCall","id":"live","name":"write","arguments":{"path":"new.txt","content":"added"}}]}}),
        &mut cx,
    );
    receive(
        &session,
        json!({"type":"tool_execution_end","toolCallId":"live","isError":false,"result":{"content":[{"type":"text","text":"ok"}]}}),
        &mut cx,
    );
    cx.run_until_parked();
    assert_eq!(
        session.controller.read_with(&cx, |c, _| c.change_count()),
        3
    );
    assert!(cx.debug_bounds("changed-file-2").is_some());
}

#[gpui::test]
fn grouped_thread_result_navigates_to_review_and_keeps_a_stable_measure(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    review_fixture(&session, &mut cx);
    // Assistant output hangs off the run rail, one gutter in from the thread edge.
    let result = cx.debug_bounds("thread-result-card-2").unwrap();
    assert_eq!(result.left(), px(240.) + RAIL);
    assert_eq!(result.right(), px(1220.) + RAIL);
    let card = cx.debug_bounds("thread-file-2-0").unwrap();
    assert_eq!(card.left(), px(241.) + RAIL);
    assert_eq!(card.right(), px(1219.) + RAIL);
    let prose = cx.debug_bounds("assistant-prose-2-0").unwrap();
    assert_eq!(prose.size.width, px(760.));
    cx.simulate_resize(size(px(1600.), px(900.)));
    cx.run_until_parked();
    assert_eq!(
        cx.debug_bounds("thread-file-2-0").unwrap().right(),
        px(1219.) + RAIL,
        "the grouped result keeps its operational measure on wide windows"
    );
    click("thread-file-2-0", &mut cx);
    assert_eq!(
        session.view.read_with(&cx, |v, _| v.page),
        panels::SessionPage::Changes
    );
    assert!(cx.debug_bounds("changes-document").is_some());
    assert!(!desktop.read_with(&cx, |d, _| d.layout.inspector));
}

#[gpui::test]
fn follow_mode_shows_the_latest_call_and_picking_a_step_pauses_following(cx: &mut TestAppContext) {
    let (_desktop, mut cx) = setup(cx);
    let session = tab(&_desktop, 0, &cx);
    review_fixture(&session, &mut cx);
    assert!(
        cx.debug_bounds("follow-stage").is_none(),
        "follow mode is opened on request"
    );
    let thread = cx.debug_bounds("composer").unwrap();
    click("follow-toggle", &mut cx);
    let stage = cx
        .debug_bounds("follow-stage")
        .expect("stage beside the thread");
    assert!(stage.left() >= cx.debug_bounds("composer").unwrap().right());
    assert!(cx.debug_bounds("composer").unwrap().size.width < thread.size.width);
    let shown = |cx: &mut VisualTestContext| {
        session
            .view
            .read_with(cx, |view, cx| view.follow.read(cx).shown_id(cx))
    };
    assert_eq!(
        shown(&mut cx).as_deref(),
        Some("review-b"),
        "the latest call"
    );
    assert!(cx.debug_bounds("follow-step-1").is_some());
    assert!(
        cx.debug_bounds("diff-unified").is_some(),
        "a reported edit shows its diff"
    );

    // Picking a call in the thread shows it on stage instead of expanding it inline.
    expand_activity_for_tool(&session, "review-a", &mut cx);
    click("tool-header-review-a", &mut cx);
    assert_eq!(shown(&mut cx).as_deref(), Some("review-a"));
    assert!(cx.debug_bounds("tool-details-review-a").is_none());
    assert!(
        !session
            .view
            .read_with(&cx, |v, cx| v.follow.read(cx).following())
    );
    click("follow-next", &mut cx);
    assert_eq!(shown(&mut cx).as_deref(), Some("review-b"));
    click("follow-step-0", &mut cx);
    assert_eq!(shown(&mut cx).as_deref(), Some("review-a"));
    click("follow-latest", &mut cx);
    assert_eq!(
        shown(&mut cx).as_deref(),
        Some("review-b"),
        "resume follows the latest call"
    );

    // Small windows keep the whole width for the thread; the choice is kept.
    cx.simulate_resize(size(px(1000.), px(740.)));
    cx.run_until_parked();
    assert!(cx.debug_bounds("follow-stage").is_none());
    cx.simulate_resize(size(px(1344.), px(740.)));
    cx.run_until_parked();
    assert!(cx.debug_bounds("follow-stage").is_some());

    click("follow-close", &mut cx);
    assert!(cx.debug_bounds("follow-stage").is_none());
    click("tool-header-review-a", &mut cx);
    assert!(
        cx.debug_bounds("tool-details-review-a").is_some(),
        "without the stage, a call expands inline again"
    );
    cx.simulate_keystrokes("ctrl-shift-f");
    cx.run_until_parked();
    assert!(cx.debug_bounds("follow-stage").is_some(), "keyboard route");
}

#[gpui::test]
fn the_rail_marks_steps_and_shows_queued_messages_as_its_future(cx: &mut TestAppContext) {
    let (_desktop, mut cx) = setup(cx);
    let session = tab(&_desktop, 0, &cx);
    review_fixture(&session, &mut cx);
    let row = cx
        .debug_bounds("rail-row-1")
        .expect("assistant rows sit on the rail");
    let step = cx.debug_bounds("activity-1-0").unwrap();
    assert_eq!(step.left(), row.left() + RAIL);
    assert!(cx.debug_bounds("activity-1-0-passed").is_some());
    // The demo session starts with a queued follow-up.
    assert!(cx.debug_bounds("rail-queued").is_some());
    receive(
        &session,
        json!({"type":"queue_update","steering":[],"followUp":[]}),
        &mut cx,
    );
    cx.run_until_parked();
    assert!(cx.debug_bounds("rail-queued").is_none());
    receive(
        &session,
        json!({"type":"queue_update","steering":[],"followUp":["Also cover the streaming path"]}),
        &mut cx,
    );
    cx.run_until_parked();
    assert!(cx.debug_bounds("rail-queued").is_some());
    receive(
        &session,
        json!({"type":"queue_update","steering":[],"followUp":[]}),
        &mut cx,
    );
    cx.run_until_parked();
    assert!(cx.debug_bounds("rail-queued").is_none());
}

#[gpui::test]
fn the_rail_paints_the_whole_flow_upfront_and_calls_fill_it_in(cx: &mut TestAppContext) {
    let (_desktop, mut cx) = setup(cx);
    let session = tab(&_desktop, 0, &cx);
    review_fixture(&session, &mut cx);
    // Outside the demo, the thread follows the newest work.
    session.transcript.update(&mut cx, |transcript, _| {
        transcript.list.set_follow_mode(gpui::FollowMode::Tail)
    });
    let ahead = |cx: &mut VisualTestContext| {
        [
            ("understand", "rail-plan-understand"),
            ("change", "rail-plan-change"),
            ("verify", "rail-plan-verify"),
            ("handoff", "rail-plan-handoff"),
        ]
        .into_iter()
        .filter(|(_, selector)| cx.debug_bounds(selector).is_some())
        .map(|(stage, _)| stage)
        .collect::<Vec<_>>()
    };
    assert!(
        ahead(&mut cx).is_empty(),
        "a settled turn has nothing ahead"
    );
    assert!(
        cx.debug_bounds("rail-handoff-2").is_some(),
        "its closing text fills the hand-off"
    );
    receive(&session, json!({"type":"agent_start"}), &mut cx);
    receive(
        &session,
        json!({"type":"message_end","message":{"role":"user","content":"Now check it"}}),
        &mut cx,
    );
    cx.run_until_parked();
    assert_eq!(
        ahead(&mut cx),
        ["understand", "change", "verify", "handoff"],
        "the whole flow is on the rail before any call"
    );
    let first = cx.debug_bounds("rail-plan-understand").unwrap();
    let last = cx.debug_bounds("rail-plan-handoff").unwrap();
    assert!(first.top() < last.top());
    receive(
        &session,
        json!({"type":"message_end","message":{"role":"assistant","content":[
            {"type":"toolCall","id":"look","name":"read","arguments":{"path":"src/provider.ts"}}
        ]}}),
        &mut cx,
    );
    cx.run_until_parked();
    assert_eq!(ahead(&mut cx), ["change", "verify", "handoff"]);
    receive(
        &session,
        json!({"type":"message_end","message":{"role":"assistant","content":[
            {"type":"toolCall","id":"verify","name":"bash","arguments":{"command":"npm run check"}}
        ]}}),
        &mut cx,
    );
    cx.run_until_parked();
    assert_eq!(
        ahead(&mut cx),
        ["handoff"],
        "a skipped change is not claimed; it is simply not ahead any more"
    );
    let check = cx
        .debug_bounds("rail-row-5")
        .expect("the running check is on the rail");
    assert!(check.top() < cx.debug_bounds("rail-plan-handoff").unwrap().top());
    receive(&session, json!({"type":"agent_settled"}), &mut cx);
    cx.run_until_parked();
    assert!(ahead(&mut cx).is_empty());
}

#[gpui::test]
fn an_opened_file_keeps_the_composer_and_details_wait_for_the_inspector(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    review_fixture(&session, &mut cx);
    session
        .controller
        .update(&mut cx, |c, cx| c.open_file("src/provider.ts".into(), cx));
    cx.run_until_parked();
    let code = cx
        .debug_bounds("file-code")
        .expect("the file takes the stage");
    let composer = cx
        .debug_bounds("composer")
        .expect("the composer stays below it");
    assert!(code.bottom() <= composer.top());
    assert!(cx.debug_bounds("file-breadcrumb").is_some());
    assert!(
        cx.debug_bounds("file-details").is_none(),
        "details are closed until requested"
    );
    show_inspector(&desktop, &mut cx);
    assert!(cx.debug_bounds("file-details").is_some());
    assert!(
        cx.debug_bounds("file-details-reveal").is_some(),
        "the edit came from this session"
    );
    let compare = cx
        .debug_bounds("file-compare-changes")
        .expect("observed edits point to Changes");
    cx.simulate_click(compare.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("changes-open").is_some(),
        "Changes compares the edit"
    );
}

#[gpui::test]
fn global_search_is_a_list_arrows_move_and_enter_opens(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    set_draft(&session, "Keep this draft", &mut cx);
    cx.simulate_keystrokes("secondary-k");
    cx.simulate_input("s");
    cx.run_until_parked();
    // "s" finds New session, All Sessions, Models…; two down is Models.
    cx.simulate_keystrokes("down down enter");
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("global-search").is_none(),
        "Enter opens and closes"
    );
    let view = desktop.read_with(&cx, |d, cx| d.workspace.read(cx).view);
    assert_eq!(view, Some(super::app_views::AppView::Models));
    assert_eq!(draft(&session, &cx), "Keep this draft");
}
