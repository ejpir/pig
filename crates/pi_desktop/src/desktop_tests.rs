use super::*;
use super::{
    composer::ComposerView, session::SessionController, session_view::SessionView,
    transcript::TranscriptView, workspace::SessionId,
};
use gpui::{Bounds, TestAppContext, VisualTestContext, WindowBounds, WindowOptions, point, size};
use pi_core::transport::TransportEvent;
use serde_json::{Value, json};

#[path = "desktop/catalog_tests.rs"]
mod catalog_tests;
#[path = "desktop/lifecycle_tests.rs"]
mod lifecycle_tests;
#[path = "desktop/projects_shell_tests.rs"]
mod projects_shell_tests;
#[path = "desktop/readability_tests.rs"]
mod readability_tests;
#[path = "desktop/status_tools_tests.rs"]
mod status_tools_tests;
#[path = "desktop/terminal_tests.rs"]
mod terminal_tests;
#[path = "desktop/tool_tests.rs"]
mod tool_tests;
#[path = "desktop/view_tests.rs"]
mod view_tests;

fn setup(cx: &mut TestAppContext) -> (Entity<Desktop>, VisualTestContext) {
    let window = cx.update(|cx| {
        cx.set_global(Theme::new(false));
        init(cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(1344.), px(740.)),
                ))),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    Desktop::new(vec![("/demo/repos/pi".into(), None)], 0, true, window, cx)
                })
            },
        )
        .unwrap()
    });
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let desktop = window.root(&mut visual).unwrap();
    visual.run_until_parked();
    (desktop, visual)
}
struct Tab {
    controller: Entity<SessionController>,
    view: Entity<SessionView>,
    composer: Entity<ComposerView>,
    transcript: Entity<TranscriptView>,
}
fn tab(desktop: &Entity<Desktop>, id: u64, cx: &VisualTestContext) -> Tab {
    desktop.read_with(cx, |desktop, cx| {
        let workspace = desktop.workspace.read(cx);
        let tab = workspace.tab(SessionId(id)).unwrap();
        let view = tab.view.read(cx);
        Tab {
            controller: tab.controller.clone(),
            view: tab.view.clone(),
            composer: view.composer.clone(),
            transcript: view.transcript.clone(),
        }
    })
}
fn expand_activity_for_tool(tab: &Tab, id: &str, cx: &mut VisualTestContext) {
    tab.transcript
        .update(cx, |view, cx| view.disclose_activity_for_tool(id, cx));
    cx.run_until_parked();
}
fn workspace(desktop: &Entity<Desktop>, cx: &VisualTestContext) -> Entity<WorkspaceController> {
    desktop.read_with(cx, |desktop, _| desktop.workspace.clone())
}
fn open(
    desktop: &Entity<Desktop>,
    cwd: &str,
    saved: Option<SavedSession>,
    cx: &mut VisualTestContext,
) -> u64 {
    let id = workspace(desktop, cx)
        .update(cx, |workspace, cx| workspace.open(cwd.into(), saved, cx))
        .0;
    cx.run_until_parked();
    id
}
fn select(desktop: &Entity<Desktop>, id: u64, cx: &mut VisualTestContext) {
    workspace(desktop, cx).update(cx, |workspace, cx| workspace.select(SessionId(id), cx));
    cx.run_until_parked();
}
fn receive(tab: &Tab, record: Value, cx: &mut VisualTestContext) {
    tab.controller.update(cx, |controller, cx| {
        controller.receive(TransportEvent::Record(record), cx)
    });
}
fn set_draft(tab: &Tab, text: &str, cx: &mut VisualTestContext) {
    let input = tab
        .composer
        .read_with(cx, |composer, _| composer.input.clone());
    input.update(cx, |input, cx| input.set_content(text, cx));
    cx.run_until_parked();
}
fn draft(tab: &Tab, cx: &VisualTestContext) -> String {
    tab.composer.read_with(cx, |composer, cx| {
        composer.input.read(cx).content().to_owned()
    })
}
fn saved(name: &str, cwd: &str) -> SavedSession {
    serde_json::from_value(
        json!({"id":name,"path":format!("/demo/{name}.jsonl"),"cwd":cwd,"firstMessage":name}),
    )
    .unwrap()
}

#[gpui::test]
fn keyboard_follow_up_and_escape_restore_queue(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    set_draft(&session, "Test this change", &mut cx);
    cx.simulate_keystrokes("alt-enter");
    session.controller.read_with(&cx, |controller, _| {
        assert_eq!(
            controller.model().follow_up.last().unwrap(),
            "Test this change"
        )
    });
    assert_eq!(draft(&session, &cx), "");
    cx.simulate_keystrokes("escape");
    session.controller.read_with(&cx, |controller, _| {
        assert!(!controller.model().busy());
        assert!(controller.model().follow_up.is_empty());
    });
    assert!(draft(&session, &cx).contains("Test this change"));
}

#[gpui::test]
fn session_switch_preserves_drafts_and_transient_slash_state_cannot_leak(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let a = tab(&desktop, 0, &cx);
    set_draft(&a, "private project A draft", &mut cx);
    a.composer.update_in(&mut cx, |composer, window, cx| {
        composer.open_slash(window, cx)
    });
    assert_eq!(draft(&a, &cx), "/");
    let id = open(&desktop, "/demo/repos/zed", None, &mut cx);
    let b = tab(&desktop, id, &cx);
    assert_eq!(draft(&b, &cx), "");
    assert_eq!(draft(&a, &cx), "private project A draft");
    set_draft(&b, "B draft", &mut cx);
    select(&desktop, 0, &mut cx);
    assert_eq!(draft(&a, &cx), "private project A draft");
    assert_eq!(draft(&b, &cx), "B draft");
    receive(&a, json!({"type":"future_event"}), &mut cx);
    assert_eq!(draft(&a, &cx), "private project A draft");
}

#[gpui::test]
fn rejected_submission_recovers_only_its_own_session_without_overwriting_new_text(
    cx: &mut TestAppContext,
) {
    let (desktop, mut cx) = setup(cx);
    let a = tab(&desktop, 0, &cx);
    a.controller.update(&mut cx, |controller, _| {
        controller.expect_submission("prompt", "rejected text")
    });
    set_draft(&a, "new A draft", &mut cx);
    let id = open(&desktop, "/demo/repos/zed", None, &mut cx);
    let b = tab(&desktop, id, &cx);
    set_draft(&b, "B draft", &mut cx);
    receive(
        &a,
        json!({"id":"prompt","type":"response","command":"prompt","success":false,"error":"rejected"}),
        &mut cx,
    );
    cx.run_until_parked();
    assert_eq!(draft(&a, &cx), "new A draft\nrejected text");
    assert_eq!(draft(&b, &cx), "B draft");
    a.controller.read_with(&cx, |controller, _| {
        assert_eq!(controller.model().error.as_deref(), Some("rejected"))
    });
}

#[gpui::test]
fn failed_bootstrap_disables_sending_without_losing_input(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let a = tab(&desktop, 0, &cx);
    a.controller.update(&mut cx, |controller, cx| {
        controller.expect_bootstrap("load");
        controller.receive(
            TransportEvent::RequestFailed {
                id: "load".into(),
                command: "get_state".into(),
                error: "offline".into(),
            },
            cx,
        );
        assert!(!controller.ready());
    });
    let before = draft(&a, &cx);
    a.composer
        .update(&mut cx, |composer, cx| composer.send_prompt(false, cx));
    assert_eq!(draft(&a, &cx), before);
    set_draft(&a, "/name must-not-change", &mut cx);
    a.composer
        .update(&mut cx, |composer, cx| composer.send_prompt(false, cx));
    assert_eq!(draft(&a, &cx), "/name must-not-change");
    a.controller.update(&mut cx, |controller, cx| {
        let before = controller.model().title().to_owned();
        assert!(
            controller
                .command(
                    Command::SetSessionName {
                        name: "denied".into(),
                        session_path: None,
                    },
                    cx
                )
                .is_none()
        );
        assert_eq!(controller.model().title(), before);
    });
}

#[gpui::test]
fn resumed_title_and_identity_are_stable_through_bootstrap(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let saved = saved("hello", "/demo/repos/pi");
    let id = open(&desktop, &saved.cwd.clone(), Some(saved.clone()), &mut cx);
    let a = tab(&desktop, id, &cx);
    assert_eq!(open(&desktop, &saved.cwd.clone(), Some(saved), &mut cx), id);
    a.controller.update(&mut cx, |controller, _| {
        controller.expect_bootstrap("state")
    });
    receive(
        &a,
        json!({"type":"response","id":"state","command":"get_state","success":true,"data":{"sessionId":"hello","sessionFile":"/demo/hello.jsonl","sessionName":null}}),
        &mut cx,
    );
    a.controller.read_with(&cx, |controller, _| {
        assert_eq!(controller.model().title(), "hello");
        assert!(!controller.bootstrap_failed());
    });
    // A later unrelated state response cannot change the immutable expected resume ID.
    receive(
        &a,
        json!({"type":"response","id":"unrelated","command":"get_state","success":true,"data":{"sessionId":"wrong"}}),
        &mut cx,
    );
    a.controller.update(&mut cx, |controller, _| {
        controller.expect_bootstrap("verify")
    });
    receive(
        &a,
        json!({"type":"response","id":"verify","command":"get_state","success":true,"data":{"sessionId":"wrong"}}),
        &mut cx,
    );
    a.controller
        .read_with(&cx, |controller, _| assert!(controller.bootstrap_failed()));
}

#[gpui::test]
fn pickers_filter_select_and_dismiss_without_submitting(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let a = tab(&desktop, 0, &cx);
    let before = draft(&a, &cx);
    a.composer.update_in(&mut cx, |composer, window, cx| {
        composer.toggle_picker(menus::Picker::Model, window, cx)
    });
    cx.simulate_input("sonnet");
    cx.run_until_parked();
    a.composer
        .read_with(&cx, |composer, _| assert_eq!(composer.choices.len(), 1));
    cx.simulate_keystrokes("enter");
    a.controller.read_with(&cx, |controller, _| {
        assert!(
            controller
                .model()
                .state
                .model
                .as_ref()
                .unwrap()
                .id
                .contains("sonnet")
        )
    });
    assert_eq!(draft(&a, &cx), before);
    a.composer.update_in(&mut cx, |composer, window, cx| {
        composer.toggle_picker(menus::Picker::Thinking, window, cx)
    });
    cx.simulate_keystrokes("down enter");
    a.controller.read_with(&cx, |controller, _| {
        assert_eq!(controller.model().state.thinking_level, "minimal")
    });
    a.composer.update_in(&mut cx, |composer, window, cx| {
        composer.toggle_picker(menus::Picker::Model, window, cx)
    });
    cx.simulate_keystrokes("escape");
    a.controller
        .read_with(&cx, |controller, _| assert!(controller.model().busy()));
}

#[gpui::test]
fn full_catalog_is_virtualized_and_row_height_matches_filtered_results(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let a = tab(&desktop, 0, &cx);
    let models: Vec<_> = (0..1000)
        .map(|i| json!({"id":format!("model-{i:04}"),"provider":"test-provider"}))
        .collect();
    receive(
        &a,
        json!({"type":"response","command":"get_available_models","success":true,"data":{"models":models}}),
        &mut cx,
    );
    a.composer.update_in(&mut cx, |composer, window, cx| {
        composer.toggle_picker(menus::Picker::Model, window, cx)
    });
    cx.run_until_parked();
    assert_eq!(
        cx.debug_bounds("picker-choice-0").unwrap().size.height,
        px(50.)
    );
    assert_eq!(
        cx.debug_bounds("picker-choice-0").unwrap().size.width,
        cx.debug_bounds("picker-list").unwrap().size.width
    );
    assert!(cx.debug_bounds("picker-choice-999").is_none());
    for _ in 0..9 {
        cx.simulate_keystrokes("down");
    }
    cx.run_until_parked();
    let selected = cx.debug_bounds("picker-choice-9").unwrap();
    let viewport = cx.debug_bounds("picker-list").unwrap();
    assert!(selected.origin.y >= viewport.origin.y);
    assert!(selected.origin.y + selected.size.height <= viewport.origin.y + viewport.size.height);
    cx.simulate_input("model-0019");
    cx.run_until_parked();
    assert_eq!(
        cx.debug_bounds("picker-choice-0").unwrap().size.height,
        px(50.)
    );
}

#[gpui::test]
fn menu_hover_search_and_typing_do_not_rebuild_transcript_or_inspector(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let a = tab(&desktop, 0, &cx);
    let inspector = a.view.read_with(&cx, |view, _| view.inspector.clone());
    a.composer.update_in(&mut cx, |composer, window, cx| {
        composer.toggle_picker(menus::Picker::Model, window, cx)
    });
    cx.run_until_parked();
    let transcript_before = a.transcript.read_with(&cx, |view, _| view.renders);
    let inspector_before = inspector.read_with(&cx, |view, _| view.renders);
    let row = cx.debug_bounds("picker-choice-1").unwrap();
    cx.simulate_mouse_move(row.center(), None, gpui::Modifiers::default());
    cx.run_until_parked();
    a.composer
        .read_with(&cx, |composer, _| assert_eq!(composer.picker_index, 1));
    assert_eq!(
        a.transcript.read_with(&cx, |view, _| view.renders),
        transcript_before
    );
    assert_eq!(
        inspector.read_with(&cx, |view, _| view.renders),
        inspector_before
    );
    cx.simulate_keystrokes("escape");
    set_draft(&a, "short", &mut cx);
    // Focus changes and composer height changes legitimately invalidate bounds.
    let transcript_before = a.transcript.read_with(&cx, |view, _| view.renders);
    cx.simulate_input(" text");
    cx.run_until_parked();
    assert_eq!(
        a.transcript.read_with(&cx, |view, _| view.renders),
        transcript_before
    );
    cx.simulate_keystrokes("secondary-k");
    cx.run_until_parked();
    let transcript_before = a.transcript.read_with(&cx, |view, _| view.renders);
    cx.simulate_input("qwen");
    cx.run_until_parked();
    assert_eq!(
        a.transcript.read_with(&cx, |view, _| view.renders),
        transcript_before
    );
}

#[gpui::test]
fn background_streaming_does_not_render_or_parse_the_active_transcript(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let a = tab(&desktop, 0, &cx);
    receive(
        &a,
        json!({"type":"message_start","message":{"role":"assistant","content":[{"type":"text","text":""}],"timestamp":1}}),
        &mut cx,
    );
    let id = open(&desktop, "/demo/repos/zed", None, &mut cx);
    let b = tab(&desktop, id, &cx);
    let before = b
        .transcript
        .read_with(&cx, |view, _| (view.renders, view.rows_synced));
    let inactive_before = a.transcript.read_with(&cx, |view, _| view.rows_synced);
    let sidebar = desktop.read_with(&cx, |desktop, _| desktop.sidebar.clone());
    let sidebar_before = sidebar.read_with(&cx, |view, _| view.renders);
    for _ in 0..100 {
        receive(
            &a,
            json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"x"}}),
            &mut cx,
        );
    }
    cx.run_until_parked();
    assert_eq!(
        b.transcript
            .read_with(&cx, |view, _| (view.renders, view.rows_synced)),
        before
    );
    assert_eq!(
        a.transcript.read_with(&cx, |view, _| view.rows_synced),
        inactive_before
    );
    assert_eq!(
        sidebar.read_with(&cx, |view, _| view.renders),
        sidebar_before
    );
    select(&desktop, 0, &mut cx);
    a.controller.read_with(&cx, |controller, _| {
        assert_eq!(
            controller.model().messages.last().unwrap()["content"][0]["text"],
            "x".repeat(100)
        )
    });
    assert!(a.transcript.read_with(&cx, |view, _| view.rows_synced) > inactive_before);
}

#[gpui::test]
fn long_history_is_virtualized_and_metadata_does_not_resynchronize_documents(
    cx: &mut TestAppContext,
) {
    let (desktop, mut cx) = setup(cx);
    let a = tab(&desktop, 0, &cx);
    let messages: Vec<_> = (0..1000)
        .map(|i| json!({"role":"user","content":format!("Message {i}")}))
        .collect();
    receive(
        &a,
        json!({"type":"response","command":"get_messages","success":true,"data":{"messages":messages}}),
        &mut cx,
    );
    cx.run_until_parked();
    let synced = a.transcript.read_with(&cx, |view, _| view.rows_synced);
    assert!(
        synced < 50,
        "only visible rows and overdraw should create documents: {synced}"
    );
    receive(
        &a,
        json!({"type":"response","command":"get_state","success":true,"data":{"sessionId":"sample","thinkingLevel":"high"}}),
        &mut cx,
    );
    cx.run_until_parked();
    assert_eq!(
        a.transcript.read_with(&cx, |view, _| view.rows_synced),
        synced
    );
}

#[gpui::test]
fn streaming_preserves_markdown_identity_and_user_scroll_position(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let a = tab(&desktop, 0, &cx);
    for index in 0..20 {
        receive(
            &a,
            json!({"type":"message_end","message":{"role":"user","content":format!("Earlier message {index}")}}),
            &mut cx,
        );
    }
    receive(
        &a,
        json!({"type":"message_start","message":{"role":"assistant","content":[{"type":"text","text":"First"}]}}),
        &mut cx,
    );
    cx.run_until_parked();
    let index = a
        .controller
        .read_with(&cx, |controller, _| controller.model().messages.len() - 1);
    let key = format!("{index}:text-0");
    a.transcript.update(&mut cx, |view, cx| {
        view.list.set_follow_mode(gpui::FollowMode::Tail);
        cx.notify();
    });
    cx.run_until_parked();
    let entity = a
        .transcript
        .read_with(&cx, |view, _| view.documents.get(&key).unwrap().entity_id());
    a.transcript.update(&mut cx, |view, cx| {
        view.list.scroll_to(gpui::ListOffset {
            item_ix: 0,
            offset_in_item: px(0.),
        });
        cx.notify();
    });
    cx.run_until_parked();
    receive(
        &a,
        json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":" next"}}),
        &mut cx,
    );
    cx.run_until_parked();
    a.transcript.read_with(&cx, |view, _| {
        assert_eq!(view.documents.get(&key).unwrap().entity_id(), entity);
        assert!(!view.list.is_following_tail());
    });
}

#[gpui::test]
fn slash_commands_attach_and_dismiss_without_sending(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let a = tab(&desktop, 0, &cx);
    set_draft(&a, "/", &mut cx);
    assert!(cx.debug_bounds("slash-list").is_some());
    cx.simulate_input("commit");
    cx.simulate_keystrokes("tab");
    a.composer
        .read_with(&cx, |composer, _| assert!(composer.attached.is_some()));
    assert_eq!(draft(&a, &cx), "");
    a.controller.read_with(&cx, |controller, _| {
        assert!(controller.model().steering.is_empty())
    });
    set_draft(&a, "keep me", &mut cx);
    a.composer.update_in(&mut cx, |composer, window, cx| {
        composer.open_slash(window, cx)
    });
    cx.simulate_keystrokes("escape");
    assert_eq!(draft(&a, &cx), "keep me");
    a.controller
        .read_with(&cx, |controller, _| assert!(controller.model().busy()));
}

#[gpui::test]
fn built_ins_dispatch_to_their_session_and_new_uses_its_project(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let a = tab(&desktop, 0, &cx);
    set_draft(&a, "/name named session", &mut cx);
    cx.simulate_keystrokes("enter");
    a.controller.read_with(&cx, |controller, _| {
        assert_eq!(controller.model().title(), "named session")
    });
    set_draft(&a, "/new", &mut cx);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(cx.debug_bounds("new-session-dialog").is_some());
    assert_eq!(
        workspace(&desktop, &cx).read_with(&cx, |workspace, _| workspace.tabs.len()),
        1
    );
    let create = cx.debug_bounds("create-session").unwrap();
    cx.simulate_click(create.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    workspace(&desktop, &cx).read_with(&cx, |workspace, _| {
        assert_eq!(workspace.tabs.len(), 2);
        assert_eq!(
            workspace.active_summary().cwd,
            PathBuf::from("/demo/repos/pi")
        );
    });
}

#[gpui::test]
fn clear_queue_does_not_abort_and_search_cannot_submit(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let a = tab(&desktop, 0, &cx);
    set_draft(&a, "queued", &mut cx);
    cx.simulate_keystrokes("alt-enter");
    let queued = a.controller.read_with(&cx, |controller, _| {
        controller
            .model()
            .steering
            .iter()
            .chain(&controller.model().follow_up)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    });
    a.composer
        .update(&mut cx, |composer, cx| composer.clear_queue(false, cx));
    cx.run_until_parked();
    a.controller.read_with(&cx, |controller, _| {
        assert!(controller.model().busy());
        assert!(controller.model().follow_up.is_empty());
    });
    assert_eq!(draft(&a, &cx), queued);
    cx.simulate_keystrokes("secondary-k");
    cx.simulate_input("search text");
    cx.simulate_keystrokes("enter escape");
    assert_eq!(draft(&a, &cx), queued);
    a.controller
        .read_with(&cx, |controller, _| assert!(controller.model().busy()));
}

#[gpui::test]
fn sidebar_lists_active_sessions_and_project_scoped_saved_rows(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    open(&desktop, "/demo/repos/zed", None, &mut cx);
    assert!(cx.debug_bounds("active-session-0").is_some());
    assert!(cx.debug_bounds("active-session-1").is_some());
    assert!(cx.debug_bounds("open-session-0").is_some());
    assert!(cx.debug_bounds("open-session-1").is_some());
    assert!(cx.debug_bounds("saved-session-demo-mistral").is_some());
    assert!(cx.debug_bounds("saved-session-demo-zed").is_some());
    let search = desktop.read_with(&cx, |desktop, cx| desktop.sidebar.read(cx).search.clone());
    search.update(&mut cx, |input, cx| input.set_content("signatures", cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("active-session-0").is_some());
    assert!(cx.debug_bounds("active-session-1").is_none());
    assert!(cx.debug_bounds("saved-session-demo-zed").is_none());
}

#[gpui::test]
fn theme_resize_sidebar_toggles_and_message_expansion_remain_functional(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let a = tab(&desktop, 0, &cx);
    cx.simulate_keystrokes("ctrl-shift-t secondary-b");
    cx.run_until_parked();
    desktop.read_with(&cx, |desktop, cx| {
        assert!(theme(cx).light);
        assert!(!desktop.layout.show_sidebar);
    });
    assert!(cx.debug_bounds("sidebar-list").is_none());
    cx.simulate_keystrokes("secondary-b");
    cx.run_until_parked();
    assert!(cx.debug_bounds("sidebar-list").is_some());
    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.run_until_parked();
    cx.simulate_keystrokes("shift-alt-escape");
    cx.run_until_parked();
    a.composer
        .read_with(&cx, |composer, _| assert!(composer.expanded));
    assert!(cx.debug_bounds("composer").unwrap().size.height > px(250.));
    a.composer.read_with(&cx, |composer, cx| {
        assert!(composer.input.read(cx).height() > px(160.));
        assert!(composer.input.read(cx).vertical_scroll() >= px(0.));
    });
    cx.simulate_keystrokes("shift-alt-escape");
    cx.run_until_parked();
    a.composer
        .read_with(&cx, |composer, _| assert!(!composer.expanded));
}

#[gpui::test]
fn sidebar_and_inspector_resize_from_their_inner_edges(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let handle = cx.debug_bounds("sidebar-resize").unwrap().center();
    cx.simulate_mouse_down(handle, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(
        point(px(280.), handle.y),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        point(px(280.), handle.y),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    desktop.read_with(&cx, |desktop, _| {
        assert_eq!(desktop.layout.sidebar_width, px(280.))
    });
}

#[gpui::test]
fn a_live_failure_has_a_dismissible_banner(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let a = tab(&desktop, 0, &cx);
    a.controller.update(&mut cx, |controller, cx| {
        controller.error("test failure", cx)
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("session-error").is_some());
    let close = cx.debug_bounds("dismiss-error").unwrap().center();
    cx.simulate_click(close, gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("session-error").is_none());
}

#[gpui::test]
fn an_extension_message_added_as_a_run_settles_shows_in_the_transcript(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let id = open(&desktop, "/demo/lsp", None, &mut cx);
    let a = tab(&desktop, id, &cx);
    let index = a
        .controller
        .read_with(&cx, |controller, _| controller.model().messages.len());
    // What the language-server bridge's pi extension adds when a run ends.
    receive(
        &a,
        json!({"type":"entry_appended","entry":{"type":"custom_message","id":"e1","customType":"pi-desktop-lsp","display":true,
            "content":"Language server errors that appeared during this run:\n  src/desktop.rs:88:20 mismatched types"}}),
        &mut cx,
    );
    cx.run_until_parked();
    a.transcript.read_with(&cx, |view, cx| {
        let text = view.documents.copy_source(&format!("{index}:custom"), cx);
        assert!(
            text.is_some_and(|text| text.contains("src/desktop.rs:88:20 mismatched types")),
            "{text:?}"
        );
    });
}

#[gpui::test]
fn ask_pi_to_fix_sends_the_problem_and_shows_the_thread(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let id = open(&desktop, "/demo/fix", None, &mut cx);
    let t = tab(&desktop, id, &cx);
    let files = t.view.read_with(&cx, |view, _| view.files.clone());
    files.update(&mut cx, |_, cx| cx.emit(files::FileEvent::Selected));
    cx.run_until_parked();
    assert!(cx.debug_bounds("file-editor").is_some());

    // What the card's button and Alt+Enter emit.
    let prompt = "Fix this language server error in `a.ts:3:7`:\n\n',' expected. ts(1005)";
    files.update(&mut cx, |_, cx| {
        cx.emit(files::FileEvent::AskPi(prompt.into()))
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("file-editor").is_none(),
        "the thread is shown"
    );
    let last = t.controller.read_with(&cx, |controller, _| {
        controller.model().messages.last().cloned()
    });
    assert_eq!(last.unwrap()["content"], prompt);
    assert_eq!(draft(&t, &cx), "", "sent, so not left as a draft");
}

#[gpui::test]
fn at_mentions_become_chips_and_pi_gets_paths(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let id = open(&desktop, "/demo/mentions", None, &mut cx);
    let t = tab(&desktop, id, &cx);
    set_draft(&t, "Compare @op", &mut cx);
    assert!(cx.debug_bounds("mention-menu").is_some());
    assert!(cx.debug_bounds("mention-item-0").is_some());
    cx.simulate_keystrokes("enter");
    assert!(
        cx.debug_bounds("mention-menu").is_none(),
        "a choice closes the menu"
    );
    let chips = t
        .composer
        .read_with(&cx, |composer, cx| composer.input.read(cx).chips().to_vec());
    assert_eq!(chips.len(), 1);
    assert!(
        cx.debug_bounds("inspector-prompt-mentions").is_some(),
        "the inspector lists the prompt's mentions"
    );
    cx.simulate_input("with its tests");
    cx.simulate_keystrokes("enter");
    let index = t
        .controller
        .read_with(&cx, |controller, _| controller.model().messages.len() - 1);
    let sent = t.controller.read_with(&cx, |controller, _| {
        controller.model().messages[index]["content"].clone()
    });
    assert_eq!(
        sent,
        "Compare @packages/ai/src/providers/openai-completions.ts with its tests"
    );
    cx.run_until_parked();
    t.transcript.read_with(&cx, |view, cx| {
        let markdown = view.documents.copy_source(&format!("{index}:user"), cx);
        assert_eq!(
            markdown,
            Some(
                "Compare [`openai-completions.ts`](pi-mention:file:packages/ai/src/providers/openai-completions.ts) with its tests"
            ),
            "the sent message shows the chip again"
        );
    });

    // Escape closes the menu and keeps the draft.
    set_draft(&t, "Look at @nothing-matches", &mut cx);
    assert!(cx.debug_bounds("mention-menu").is_some());
    cx.simulate_keystrokes("escape");
    assert!(cx.debug_bounds("mention-menu").is_none());
    assert_eq!(draft(&t, &cx), "Look at @nothing-matches");
}

#[gpui::test]
fn directory_mentions_filter_insert_and_submit_without_attaching_contents(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let id = open(&desktop, "/demo/directories", None, &mut cx);
    let t = tab(&desktop, id, &cx);
    set_draft(&t, "Inspect @providers/", &mut cx);
    assert!(cx.debug_bounds("mention-item-0").is_some());
    assert!(
        cx.debug_bounds("mention-preview").is_none(),
        "Directory entries do not trigger file reads"
    );
    cx.simulate_keystrokes("enter");
    t.composer.read_with(&cx, |composer, cx| {
        let mentions = composer.draft_mentions(cx);
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].0.group(), "Directories");
        assert_eq!(mentions[0].1, "providers/");
        assert_eq!(mentions[0].2, "directory path · no contents attached");
    });
    cx.simulate_keystrokes("enter");
    t.controller.read_with(&cx, |controller, _| {
        assert_eq!(
            controller.model().messages.last().unwrap()["content"],
            "Inspect @packages/ai/src/providers/"
        );
    });
    set_draft(&t, "Inspect @./packages/ai/src/providers/", &mut cx);
    cx.simulate_keystrokes("tab");
    assert_eq!(draft(&t, &cx), "Inspect @packages/ai/src/providers/ ");
    assert!(cx.debug_bounds("mention-menu").is_none());
}

#[gpui::test]
fn mention_matching_cannot_publish_stale_queries_or_reopen_a_dismissed_menu(
    cx: &mut TestAppContext,
) {
    let (desktop, mut cx) = setup(cx);
    let id = open(&desktop, "/demo/matching", None, &mut cx);
    let t = tab(&desktop, id, &cx);
    let input = t
        .composer
        .read_with(&cx, |composer, _| composer.input.clone());
    input.update(&mut cx, |input, cx| input.set_content("@providers/", cx));
    input.update(&mut cx, |input, cx| {
        input.set_content("@nothing-matches", cx)
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("mention-item-0").is_none());
    input.update(&mut cx, |input, cx| input.set_content("@op", cx));
    t.composer
        .update(&mut cx, |composer, cx| composer.dismiss_mentions(cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("mention-menu").is_none());
    assert_eq!(draft(&t, &cx), "@op");
}

#[gpui::test]
fn attachments_send_images_and_file_paths(cx: &mut TestAppContext) {
    use base64::Engine as _;
    let (desktop, mut cx) = setup(cx);
    let id = open(&desktop, "/demo/attach", None, &mut cx);
    let t = tab(&desktop, id, &cx);
    // A 1×1 PNG, and a text file outside the project.
    let dot = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    let png = base64::engine::general_purpose::STANDARD
        .decode(dot)
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let notes = dir.path().join("notes.md");
    std::fs::write(&notes, "# Notes").unwrap();
    t.composer.update_in(&mut cx, |composer, window, cx| {
        composer.attachments.push(
            attachments::Attachment::new("dot.png".into(), gpui::ImageFormat::Png, png).unwrap(),
        );
        composer.add_paths(vec![notes.clone()], window, cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("composer-attachment-0").is_some());
    cx.simulate_input("What is in these?");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let (index, sent) = t.controller.read_with(&cx, |controller, _| {
        let messages = &controller.model().messages;
        (
            messages.len() - 1,
            messages.last().unwrap()["content"].clone(),
        )
    });
    assert_eq!(
        sent,
        json!([
            {"type":"text","text":format!("@{} What is in these?", notes.display())},
            {"type":"image","data":dot,"mimeType":"image/png"}
        ]),
        "the file goes as its path, the image as an image"
    );
    t.composer
        .read_with(&cx, |composer, _| assert!(composer.attachments.is_empty()));
    let selector: &'static str = format!("user-image-{index}-0").leak();
    assert!(
        cx.debug_bounds(selector).is_some(),
        "the sent message shows its image"
    );
}

#[gpui::test]
fn all_sessions_and_settings_open_from_the_sidebar(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let saved_count = workspace(&desktop, &cx).read_with(&cx, |workspace, _| workspace.saved.len());
    assert!(saved_count > 1, "the demo lists saved sessions");
    let nav = cx.debug_bounds("nav-All Sessions").unwrap();
    cx.simulate_click(nav.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("all-sessions").is_some());
    assert!(cx.debug_bounds("app-view-title").is_some());
    assert!(cx.debug_bounds("session-row-0").is_some());
    assert!(cx.debug_bounds("sessions-inspector").is_some());

    let resume = cx.debug_bounds("resume-session").unwrap();
    cx.simulate_click(resume.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("all-sessions").is_none(),
        "resuming shows the session"
    );
    assert_eq!(workspace(&desktop, &cx).read_with(&cx, |w, _| w.view), None);

    let nav = cx.debug_bounds("nav-Settings").unwrap();
    cx.simulate_click(nav.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("settings-view").is_some());
    assert!(cx.debug_bounds("settings-category-0").is_some());
    assert!(
        cx.debug_bounds("setting-row-defaultThinkingLevel")
            .is_some()
    );
    assert!(cx.debug_bounds("settings-inspector").is_some());
    let interaction = cx.debug_bounds("settings-category-1").unwrap();
    cx.simulate_click(interaction.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("setting-row-steeringMode").is_some());
    assert!(
        cx.debug_bounds("setting-row-defaultThinkingLevel")
            .is_none()
    );
}

#[gpui::test]
fn desktop_settings_sit_below_pi_and_apply_at_once(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| cx.set_global(crate::prefs::Prefs::load_from(Some(dir.path()))));
    let (_desktop, mut cx) = setup(cx);
    let click = |selector: &'static str, cx: &mut VisualTestContext| {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector}"));
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        cx.run_until_parked();
    };
    click("nav-Settings", &mut cx);
    click("desktop-category-2", &mut cx);
    assert!(cx.debug_bounds("setting-row-jj.offer").is_some());
    click("setting-toggle-jj.tools", &mut cx);
    cx.update(|_, cx| assert!(crate::prefs::flag(cx, "jj.tools", None)));
    let saved = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&saved).unwrap(),
        json!({"jj": {"tools": true}})
    );

    click("desktop-category-1", &mut cx);
    click("setting-appearance.theme-2", &mut cx);
    cx.update(|_, cx| assert!(theme(cx).light, "Moonstone applies at once"));

    click("settings-project", &mut cx);
    click("desktop-category-0", &mut cx);
    assert!(
        cx.debug_bounds("setting-row-general.reopenSessions")
            .is_none(),
        "a project cannot override General"
    );
    click("desktop-category-2", &mut cx);
    assert!(cx.debug_bounds("setting-row-jj.tools").is_some());
}

#[gpui::test]
fn session_local_view_state_survives_switching(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let a = tab(&desktop, 0, &cx);
    a.composer.update_in(&mut cx, |composer, window, cx| {
        composer.toggle_composer(window, cx)
    });
    open(&desktop, "/demo/repos/zed", None, &mut cx);
    let b = tab(&desktop, 1, &cx);
    b.composer
        .read_with(&cx, |composer, _| assert!(!composer.expanded));
    select(&desktop, 0, &mut cx);
    a.composer
        .read_with(&cx, |composer, _| assert!(composer.expanded));
}

#[gpui::test]
fn markdown_skill_cards_and_multiline_tool_commands_keep_their_behavior(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let saved = workspace(&desktop, &cx).read_with(&cx, |workspace, _| {
        workspace
            .saved
            .iter()
            .find(|saved| saved.id == "demo-mistral")
            .unwrap()
            .clone()
    });
    let id = open(&desktop, &saved.cwd.clone(), Some(saved), &mut cx);
    let a = tab(&desktop, id, &cx);
    a.transcript.update(&mut cx, |view, cx| {
        view.list.scroll_to(gpui::ListOffset {
            item_ix: 0,
            offset_in_item: px(0.),
        });
        cx.notify();
    });
    cx.run_until_parked();
    let card = cx
        .debug_bounds("skill-card-0")
        .expect("collapsed skill card");
    assert!(cx.debug_bounds("skill-body-0").is_none());
    cx.simulate_click(card.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("skill-body-0").is_some());
    a.transcript.read_with(&cx, |view, cx| {
        assert!(
            view.documents
                .get("0:skill")
                .unwrap()
                .read(cx)
                .source()
                .contains("Code review")
        )
    });
    receive(
        &a,
        json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"toolCall","id":"script","name":"bash","arguments":{"command":"echo first\necho second"}}]}}),
        &mut cx,
    );
    receive(
        &a,
        json!({"type":"tool_execution_end","toolCallId":"script","result":{"content":[{"type":"text","text":"first\nsecond"}]}}),
        &mut cx,
    );
    a.transcript.update(&mut cx, |view, cx| {
        view.list.set_follow_mode(gpui::FollowMode::Tail);
        cx.notify();
    });
    cx.run_until_parked();
    expand_activity_for_tool(&a, "script", &mut cx);
    a.transcript.update(&mut cx, |view, cx| {
        view.list.set_follow_mode(gpui::FollowMode::Tail);
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(
        cx.debug_bounds("tool-header-script").unwrap().size.height,
        px(24.)
    );
    assert!(cx.debug_bounds("tool-details-script").is_none());
    let header = cx.debug_bounds("tool-header-script").unwrap();
    cx.simulate_click(header.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    a.transcript.read_with(&cx, |view, cx| {
        assert_eq!(
            view.documents.copy_source("tool:script:command", cx),
            Some("echo first\necho second")
        );
    });
}

#[gpui::test]
fn slash_catalog_is_virtualized_and_keyboard_selection_scrolls_into_view(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let a = tab(&desktop, 0, &cx);
    let commands: Vec<_> = (0..500).map(|i| json!({"name":format!("command-{i:03}"),"description":"A test command","source":"extension"})).collect();
    receive(
        &a,
        json!({"type":"response","command":"get_commands","success":true,"data":{"commands":commands}}),
        &mut cx,
    );
    set_draft(&a, "/command", &mut cx);
    assert!(cx.debug_bounds("slash-item-0").is_some());
    assert!(cx.debug_bounds("slash-item-499").is_none());
    for _ in 0..20 {
        cx.simulate_keystrokes("down");
    }
    cx.run_until_parked();
    let selected = cx.debug_bounds("slash-item-20").unwrap();
    let viewport = cx.debug_bounds("slash-list").unwrap();
    assert!(selected.origin.y >= viewport.origin.y);
    assert!(selected.origin.y + selected.size.height <= viewport.origin.y + viewport.size.height);
    assert_eq!(selected.size.width, viewport.size.width - px(12.));
    cx.simulate_keystrokes("enter");
    a.composer.read_with(&cx, |composer, _| {
        assert_eq!(composer.attached.as_ref().unwrap().label(), "/command-020")
    });
}

#[gpui::test]
fn a_turn_that_edits_files_gets_a_line_with_undo_and_redo(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "one\n").unwrap();
    let env = pi_jj::Env::isolated("Test User", "test@example.com");
    let project = pi_jj::Project::init_with(dir.path(), &env).unwrap();
    let id = open(&desktop, "/demo/repos/other", None, &mut cx);
    let t = tab(&desktop, id, &cx);
    t.controller.update(&mut cx, |controller, _| {
        controller.use_jj(project, "Change a")
    });

    receive(
        &t,
        json!({"type":"message_end","message":{"role":"user","content":"Change a"}}),
        &mut cx,
    );
    std::fs::write(&file, "two\n").unwrap();
    receive(
        &t,
        json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Done."}]}}),
        &mut cx,
    );
    receive(&t, json!({"type":"agent_settled"}), &mut cx);
    cx.run_until_parked();
    let record = |cx: &VisualTestContext| {
        t.controller
            .read_with(cx, |controller, _| controller.jj().records.first().cloned())
    };
    let turn = record(&cx).expect("the turn was recorded");
    assert_eq!(
        (turn.after_message, turn.files, turn.added, turn.removed),
        (1, 1, 1, 1)
    );
    assert!(cx.debug_bounds("jj-turn-0").is_some());

    t.controller
        .update(&mut cx, |controller, cx| controller.undo_turn(0, cx));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "one\n");
    assert!(record(&cx).unwrap().undone.is_some());

    t.controller
        .update(&mut cx, |controller, cx| controller.redo_turn(0, cx));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "two\n");
    assert!(record(&cx).unwrap().undone.is_none());
}

#[gpui::test]
fn turn_links_bring_back_file_history_after_reopening(cx: &mut TestAppContext) {
    use pi_jj::ObjectId as _;
    let (desktop, mut cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    let env = pi_jj::Env::isolated("Test User", "test@example.com");
    let mut project = pi_jj::Project::init_with(root, &env).unwrap();
    let record = |project: &mut pi_jj::Project, text: &str| {
        let turn = project.begin_turn(text).unwrap();
        std::fs::write(root.join("a.txt"), format!("{text}\n")).unwrap();
        let change = project.end_turn(&turn).unwrap().unwrap();
        let commit = project.visible_commit(&change).unwrap().unwrap().id().hex();
        (change, commit)
    };
    let (kept, kept_commit) = record(&mut project, "two");
    let (undone, undone_commit) = record(&mut project, "three");
    let operation = project.undo_turn(&undone).unwrap();
    let entry = |event: super::turn_links::Event| {
        let Command::AppendCustomEntry { custom_type, data } = event.command() else {
            unreachable!()
        };
        json!({"type":"custom","customType":custom_type,"data":data})
    };
    let link = |change: &pi_jj::ChangeId, commit: String, after: u64| super::turn_links::Link {
        change: change.hex(),
        commit,
        after: Some(after),
        tools: vec![],
        undone: None,
    };
    let data = json!({"entries":[
        entry(super::turn_links::Event::Recorded(link(&kept, kept_commit, 2000))),
        entry(super::turn_links::Event::Recorded(link(&undone, undone_commit, 4000))),
        entry(super::turn_links::Event::Undone { change: undone.hex(), operation: operation.hex() }),
    ]});

    let id = open(&desktop, "/demo/repos/other", None, &mut cx);
    let t = tab(&desktop, id, &cx);
    let messages = json!([
        {"role":"user","content":"two","timestamp":1000},
        {"role":"assistant","content":[{"type":"text","text":"Done."}],"timestamp":2000},
    ]);
    receive(
        &t,
        json!({"type":"response","command":"get_messages","success":true,"data":{"messages":messages}}),
        &mut cx,
    );
    t.controller.update(&mut cx, |controller, cx| {
        controller.restore_links_from(project, &data, cx)
    });
    cx.run_until_parked();
    let records = t
        .controller
        .read_with(&cx, |controller, _| controller.jj().records.clone());
    assert_eq!(records.len(), 2);
    assert_eq!((records[0].anchored, records[0].after_message), (true, 1));
    assert_eq!(records[0].description, "two");
    assert!(records[0].undone.is_none());
    assert_eq!(records[1].undone, Some(operation), "redo still works");
    assert!(
        !records[1].anchored,
        "its message is not in this conversation yet"
    );
    assert!(cx.debug_bounds("jj-turn-0").is_some());

    // A reload finds the message again by its time.
    let messages = json!([
        {"role":"user","content":"two","timestamp":1000},
        {"role":"assistant","content":[{"type":"text","text":"Done."}],"timestamp":2000},
        {"role":"user","content":"three","timestamp":3000},
        {"role":"assistant","content":[{"type":"text","text":"Done again."}],"timestamp":4000},
    ]);
    receive(
        &t,
        json!({"type":"response","command":"get_messages","success":true,"data":{"messages":messages}}),
        &mut cx,
    );
    let records = t
        .controller
        .read_with(&cx, |controller, _| controller.jj().records.clone());
    assert_eq!((records[1].anchored, records[1].after_message), (true, 3));
}

#[gpui::test]
fn jj_actions_restore_a_file_undo_with_conflicts_and_restore_operations(cx: &mut TestAppContext) {
    use pi_jj::ObjectId as _;
    let (desktop, mut cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_owned();
    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    let env = pi_jj::Env::isolated("Test User", "test@example.com");
    let mut project = pi_jj::Project::init_with(&root, &env).unwrap();
    let mut links = Vec::new();
    for (text, other) in [("two", "b one"), ("three", "b two")] {
        let turn = project.begin_turn(text).unwrap();
        std::fs::write(root.join("a.txt"), format!("{text}\n")).unwrap();
        std::fs::write(root.join("b.txt"), format!("{other}\n")).unwrap();
        let change = project.end_turn(&turn).unwrap().unwrap();
        let commit = project.visible_commit(&change).unwrap().unwrap().id().hex();
        let Command::AppendCustomEntry { custom_type, data } =
            super::turn_links::Event::Recorded(super::turn_links::Link {
                change: change.hex(),
                commit,
                after: None,
                tools: vec![],
                undone: None,
            })
            .command()
        else {
            unreachable!()
        };
        links.push(json!({"type":"custom","customType":custom_type,"data":data}));
    }
    let id = open(&desktop, "/demo/repos/other", None, &mut cx);
    let t = tab(&desktop, id, &cx);
    t.controller.update(&mut cx, |c, cx| {
        c.restore_links_from(project, &json!({ "entries": links }), cx)
    });
    cx.run_until_parked();

    // The second turn edits both files too: restoring the first turn's a.txt
    // alone is refused, and so is undoing the first turn.
    t.controller
        .update(&mut cx, |c, cx| c.restore_file(0, "a.txt".into(), cx));
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(root.join("a.txt")).unwrap(),
        "three\n"
    );
    let notice = t
        .controller
        .read_with(&cx, |c, _| c.model().notice.clone().unwrap_or_default());
    assert!(notice.starts_with("Can't restore a.txt alone"), "{notice}");

    // The later turn's b.txt restores alone.
    t.controller
        .update(&mut cx, |c, cx| c.restore_file(1, "b.txt".into(), cx));
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(root.join("b.txt")).unwrap(),
        "b one\n"
    );
    let files = t.controller.read_with(&cx, |c, _| c.jj().records[1].files);
    assert_eq!(files, 1, "the turn keeps a.txt only");

    // Undo with conflicts: jj keeps them and the composer gets the fix.
    t.controller
        .update(&mut cx, |c, cx| c.undo_turn_keeping_conflicts(0, cx));
    cx.run_until_parked();
    assert!(
        std::fs::read_to_string(root.join("a.txt"))
            .unwrap()
            .contains("<<<<<<<")
    );
    let draft = t.composer.read_with(&cx, |composer, cx| {
        composer.input.read(cx).content().to_owned()
    });
    assert!(
        draft.starts_with("Resolve the conflict markers in @a.txt left by undoing turn"),
        "{draft}"
    );

    // The operation log restores the project to before the undo.
    let task = t
        .controller
        .read_with(&cx, |c, cx| c.operations(cx))
        .unwrap();
    let loaded = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = loaded.clone();
    cx.update(|_, cx| {
        cx.spawn(async move |_| *slot.borrow_mut() = Some(task.await))
            .detach()
    });
    cx.run_until_parked();
    let operations = loaded.borrow_mut().take().unwrap().unwrap();
    assert!(operations[0].description.starts_with("pi: undo turn"));
    let before_undo = operations[1].id.clone();
    t.controller
        .update(&mut cx, |c, cx| c.restore_operation(before_undo, cx));
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(root.join("a.txt")).unwrap(),
        "three\n"
    );
    let undone = t
        .controller
        .read_with(&cx, |c, _| c.jj().records[0].undone.clone());
    assert!(undone.is_none(), "the turn is back");
}

#[gpui::test]
fn a_command_that_changes_files_shows_them_and_restores_to_before(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_owned();
    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    let env = pi_jj::Env::isolated("Test User", "test@example.com");
    let project = pi_jj::Project::init_with(&root, &env).unwrap();
    let id = open(&desktop, "/demo/repos/other", None, &mut cx);
    let t = tab(&desktop, id, &cx);
    t.controller.update(&mut cx, |c, cx| {
        c.restore_links_from(project, &json!({ "entries": [] }), cx)
    });
    let ask = |request: Value, cx: &mut VisualTestContext| {
        let task = t
            .controller
            .update(cx, |c, cx| c.bridge_request(request, cx));
        let answer = std::rc::Rc::new(std::cell::RefCell::new(None));
        let slot = answer.clone();
        cx.update(|_, cx| {
            cx.spawn(async move |_| *slot.borrow_mut() = Some(task.await))
                .detach()
        });
        cx.run_until_parked();
        answer.borrow_mut().take().unwrap().unwrap()
    };
    receive(
        &t,
        json!({"type":"message_end","message":{"role":"user","content":"Clean up"}}),
        &mut cx,
    );
    receive(
        &t,
        json!({"type":"message_end","message":{"role":"assistant","content":[
            {"type":"toolCall","id":"call-1","name":"bash","arguments":{"command":"rm a.txt && touch b.txt"}}
        ]}}),
        &mut cx,
    );
    assert_eq!(
        ask(
            json!({"op":"snapshot_before","toolCallId":"call-1"}),
            &mut cx
        ),
        ""
    );
    std::fs::remove_file(root.join("a.txt")).unwrap();
    std::fs::write(root.join("b.txt"), "").unwrap();
    receive(
        &t,
        json!({"type":"tool_execution_end","toolCallId":"call-1","toolName":"bash","result":{"content":[{"type":"text","text":""}]},"isError":false}),
        &mut cx,
    );
    assert_eq!(
        ask(
            json!({"op":"snapshot_after","toolCallId":"call-1"}),
            &mut cx
        ),
        ""
    );
    receive(&t, json!({"type":"agent_settled"}), &mut cx);
    cx.run_until_parked();
    expand_activity_for_tool(&t, "call-1", &mut cx);
    assert!(cx.debug_bounds("command-files-call-1").is_some());

    t.controller
        .update(&mut cx, |c, cx| c.restore_command("call-1".into(), cx));
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(root.join("a.txt")).unwrap(),
        "one\n"
    );
    assert!(!root.join("b.txt").exists());
    let restored = t
        .controller
        .read_with(&cx, |c, _| c.jj().commands["call-1"].restored);
    assert!(restored);

    // pi's jj tools answer from the same project.
    let log = ask(json!({"op":"jj_log","limit":5}), &mut cx);
    assert!(log.contains("(working copy)"), "{log}");
    let missing = ask(json!({"op":"jj_show","revision":"nosuchchange"}), &mut cx);
    assert!(missing.starts_with("jj could not answer"), "{missing}");
}

#[gpui::test]
fn a_second_session_in_a_busy_folder_can_work_in_its_own_workspace(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("pi");
    std::fs::create_dir(&root).unwrap();
    let env = pi_jj::Env::isolated("Test User", "test@example.com");
    // A recorded turn: workspaces start from the last one.
    let mut project = pi_jj::Project::init_with(&root, &env).unwrap();
    let turn = project.begin_turn("Add a").unwrap();
    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    project.end_turn(&turn).unwrap();
    drop(project);
    let attach = |t: &Tab, cx: &mut VisualTestContext| {
        let project = pi_jj::Project::open_with(&root, &env).unwrap();
        t.controller.update(cx, |c, cx| {
            c.restore_links_from(project, &json!({ "entries": [] }), cx)
        });
    };
    let a = tab(
        &desktop,
        open(&desktop, "/demo/repos/a", None, &mut cx),
        &cx,
    );
    let b = tab(
        &desktop,
        open(&desktop, "/demo/repos/b", None, &mut cx),
        &cx,
    );
    attach(&a, &mut cx);
    attach(&b, &mut cx);
    // The first session is running.
    receive(&a, json!({"type":"agent_start"}), &mut cx);
    cx.run_until_parked();

    assert!(a.controller.read_with(&cx, |c, _| c.working()));
    let sent = b.controller.update(&mut cx, |c, cx| {
        c.submit("Add a regression test".into(), false, cx)
    });
    assert!(sent);
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("parallel-dialog").is_some(),
        "asked where to work"
    );
    let start = cx.debug_bounds("parallel-start").unwrap();
    cx.simulate_click(start.center(), gpui::Modifiers::none());
    cx.run_until_parked();

    let folder = dir.path().join("pi-ws/add-a-regression-test");
    assert_eq!(
        std::fs::read_to_string(folder.join("a.txt")).unwrap(),
        "one\n"
    );
    let (cwds, moved) = workspace(&desktop, &cx).read_with(&cx, |w, cx| {
        let cwds: Vec<PathBuf> = w
            .tabs
            .iter()
            .map(|t| t.controller.read(cx).model().cwd.clone())
            .collect();
        let moved = w
            .tabs
            .iter()
            .find(|t| t.controller.read(cx).model().cwd == folder)
            .map(|t| {
                (
                    t.controller.read(cx).model().messages.len(),
                    t.controller
                        .read(cx)
                        .main_folder()
                        .map(std::path::Path::to_path_buf),
                )
            });
        (cwds, moved)
    });
    assert!(
        !cwds.contains(&PathBuf::from("/demo/repos/b")),
        "the unstarted session gave way"
    );
    let (messages, main) = moved.expect("a session in the workspace");
    assert_eq!(messages, 1, "the prompt went to the new session");
    assert_eq!(main.as_deref(), Some(root.as_path()));
}

#[gpui::test]
fn forking_before_later_turns_asks_which_files_to_start_with(cx: &mut TestAppContext) {
    use pi_jj::ObjectId as _;
    let (desktop, mut cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_owned();
    let env = pi_jj::Env::isolated("Test User", "test@example.com");
    let mut project = pi_jj::Project::init_with(&root, &env).unwrap();
    let turn = project.begin_turn("Add a").unwrap();
    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    let change = project.end_turn(&turn).unwrap().unwrap();
    let commit = project.visible_commit(&change).unwrap().unwrap().id().hex();
    let Command::AppendCustomEntry { custom_type, data } =
        super::turn_links::Event::Recorded(super::turn_links::Link {
            change: change.hex(),
            commit,
            // 10:05, after the entry below.
            after: Some(pi_core::clock::parse_timestamp("2026-09-30T10:05:00.000Z").unwrap()),
            tools: vec![],
            undone: None,
        })
        .command()
    else {
        unreachable!()
    };
    let t = tab(
        &desktop,
        open(&desktop, "/demo/repos/other", None, &mut cx),
        &cx,
    );
    t.controller.update(&mut cx, |c, cx| {
        c.restore_links_from(
            project,
            &json!({"entries":[{"type":"custom","customType":custom_type,"data":data}]}),
            cx,
        )
    });
    receive(
        &t,
        json!({"type":"response","command":"get_backend_info","success":true,"data":{"features":["fork_cwd"]}}),
        &mut cx,
    );
    receive(
        &t,
        json!({"type":"response","command":"get_entries","success":true,"data":{"leafId":"u1","entries":[
            {"id":"u1","parentId":null,"type":"message","timestamp":"2026-09-30T10:00:00.000Z","message":{"role":"user","content":"Add a"}}
        ]}}),
        &mut cx,
    );
    cx.run_until_parked();

    t.controller
        .update(&mut cx, |c, cx| c.request_fork("u1".into(), cx));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("confirmation-dialog").is_some(),
        "asked which files"
    );
    // As they are now: an ordinary fork, which the offline demo cannot make.
    let now = cx.debug_bounds("prompt-choice-1").unwrap();
    cx.simulate_click(now.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    let notice = t
        .controller
        .read_with(&cx, |c, _| c.model().notice.clone().unwrap_or_default());
    assert!(
        notice.starts_with("Fork requires a connected Pi session"),
        "{notice}"
    );
}
