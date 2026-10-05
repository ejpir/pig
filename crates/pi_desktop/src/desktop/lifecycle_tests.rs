use super::*;
use crate::desktop::workspace::CloseTarget;

fn confirm_close(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds("prompt-choice-1")
        .expect("confirmation must be visible");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
}

#[gpui::test]
fn landing_uses_reported_catalog_without_submitting_or_overwriting_drafts(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    show_inspector(&desktop, &mut cx);
    let id = open(&desktop, "/demo/new", None, &mut cx);
    let session = tab(&desktop, id, &cx);
    assert!(cx.debug_bounds("landing").is_some());
    assert!(cx.debug_bounds("new-session-inspector").is_some());
    set_draft(&session, "Keep my own instructions", &mut cx);
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: point(px(600.), px(300.)),
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-600.))),
        ..Default::default()
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("landing-command-0").unwrap();
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert_eq!(draft(&session, &cx), "Keep my own instructions");
    session.composer.read_with(&cx, |view, _| {
        assert_eq!(view.attached.as_ref().unwrap().label(), "/fix-tests")
    });
    session
        .controller
        .read_with(&cx, |c, _| assert!(c.model().messages.is_empty()));
    receive(
        &session,
        json!({"type":"message_end","message":{"role":"user","content":"First real message"}}),
        &mut cx,
    );
    cx.run_until_parked();
    assert!(cx.debug_bounds("landing").is_none());
    assert!(cx.debug_bounds("inspector-usage").is_some());
}

#[gpui::test]
fn files_shortcut_opens_hidden_inspector_without_changing_the_draft(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    set_draft(&session, "Keep this draft", &mut cx);
    assert!(!desktop.read_with(&cx, |d, _| d.layout.inspector));
    let terminal = cx.debug_bounds("terminal-toggle").unwrap();
    let files = cx.debug_bounds("show-files").unwrap();
    assert_eq!(files.origin.y, terminal.origin.y);
    assert_eq!(terminal.origin.x - files.right(), px(2.));
    cx.simulate_click(files.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("file-browser").is_some());
    assert_eq!(draft(&session, &cx), "Keep this draft");
    assert!(
        !session
            .view
            .read_with(&cx, |view, cx| view.terminal.read(cx).is_open())
    );
    // Explicit navigation also works below the inspector's automatic-hide width.
    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.run_until_parked();
    let browser = cx.debug_bounds("file-browser").unwrap();
    assert!(browser.origin.x + browser.size.width <= px(1000.));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(draft(&session, &cx), "Keep this draft");
}

#[gpui::test]
fn middle_navigation_is_the_only_inspector_navigation(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let id = open(&desktop, "/demo/new", None, &mut cx);
    for name in ["inspector-overview", "inspector-tree", "inspector-files"] {
        assert!(cx.debug_bounds(name).is_none());
    }
    click("tab-tree", &mut cx);
    assert_eq!(
        tab(&desktop, id, &cx)
            .view
            .read_with(&cx, |view, _| view.page),
        panels::SessionPage::Tree
    );
}

#[gpui::test]
fn close_last_session_has_a_real_empty_workspace_and_preserves_saved_history(
    cx: &mut TestAppContext,
) {
    let (desktop, mut cx) = setup(cx);
    let ws = workspace(&desktop, &cx);
    ws.update_in(&mut cx, |ws, window, cx| {
        ws.request_close(CloseTarget::Session(SessionId(0)), window, cx)
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(ws.read_with(&cx, |ws, _| ws.tabs.len()), 1);
    ws.update_in(&mut cx, |ws, window, cx| {
        ws.request_close(CloseTarget::Session(SessionId(0)), window, cx)
    });
    confirm_close(&mut cx);
    ws.read_with(&cx, |ws, _| {
        assert!(ws.tabs.is_empty());
        assert!(!ws.saved.is_empty());
        assert!(!ws.projects.is_empty());
    });
    assert!(cx.debug_bounds("workspace-welcome").is_some());
    ws.update(&mut cx, |ws, cx| ws.new_session(cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("new-session-dialog").is_some());
    assert!(
        ws.read_with(&cx, |ws, _| ws.tabs.is_empty()),
        "choosing must not start a process"
    );
    let create = cx.debug_bounds("create-session").unwrap();
    cx.simulate_click(create.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    ws.read_with(&cx, |ws, _| {
        assert_eq!(ws.active, SessionId(1));
        assert_eq!(ws.active_summary().cwd, PathBuf::from("/demo/repos/pi"));
    });
    assert!(cx.debug_bounds("landing").is_some());
    assert!(cx.debug_bounds("workspace-welcome").is_none());
    ws.update_in(&mut cx, |ws, window, cx| {
        ws.request_close(CloseTarget::Session(SessionId(1)), window, cx)
    });
    cx.run_until_parked();
    // Select a different project without opening a session, then use Ctrl+N.
    let project = cx.debug_bounds("project-1").unwrap();
    cx.simulate_click(project.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        ws.read_with(&cx, |ws, _| ws.selected_project.clone()),
        Some(PathBuf::from("/demo/repos/zed"))
    );
    cx.simulate_keystrokes("secondary-n");
    cx.run_until_parked();
    assert!(cx.debug_bounds("new-session-dialog").is_some());
    let create = cx.debug_bounds("create-session").unwrap();
    cx.simulate_click(create.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    ws.read_with(&cx, |ws, _| {
        assert_eq!(ws.active_summary().cwd, PathBuf::from("/demo/repos/zed"))
    });
}

#[gpui::test]
fn project_removal_closes_only_its_sessions_and_is_not_undone_by_catalog_refresh(
    cx: &mut TestAppContext,
) {
    let (desktop, mut cx) = setup(cx);
    let ws = workspace(&desktop, &cx);
    let second = open(&desktop, "/demo/other", None, &mut cx);
    set_draft(&tab(&desktop, second, &cx), "Other project draft", &mut cx);
    ws.update_in(&mut cx, |ws, window, cx| {
        ws.request_close(CloseTarget::Project("/demo/repos/pi".into()), window, cx)
    });
    confirm_close(&mut cx);
    assert_eq!(
        draft(&tab(&desktop, second, &cx), &cx),
        "Other project draft"
    );
    ws.read_with(&cx, |ws, _| {
        assert_eq!(ws.tabs.len(), 1);
        assert!(!ws.projects.contains(&PathBuf::from("/demo/repos/pi")));
    });
    select(&desktop, second, &mut cx);
    ws.read_with(&cx, |ws, _| {
        assert!(!ws.projects.contains(&PathBuf::from("/demo/repos/pi")))
    });
    open(&desktop, "/demo/repos/pi", None, &mut cx);
    ws.read_with(&cx, |ws, _| {
        assert!(ws.projects.contains(&PathBuf::from("/demo/repos/pi")))
    });
}

#[gpui::test]
fn hovering_close_does_not_select_or_reopen_the_closed_session(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let second = open(&desktop, "/demo/other", None, &mut cx);
    assert_eq!(second, 1);
    let row = cx.debug_bounds("open-session-1").unwrap();
    cx.simulate_mouse_move(row.center(), None, gpui::Modifiers::default());
    cx.run_until_parked();
    let close = cx.debug_bounds("close-open-session-1").unwrap();
    cx.simulate_click(close.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    workspace(&desktop, &cx).read_with(&cx, |ws, _| {
        assert!(ws.tab(SessionId(second)).is_none());
        assert_eq!(ws.active, SessionId(0));
    });
}

#[gpui::test]
fn session_selection_cannot_steal_confirmation_keyboard_focus(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let ws = workspace(&desktop, &cx);
    ws.update_in(&mut cx, |ws, window, cx| {
        ws.request_close(CloseTarget::Session(SessionId(0)), window, cx)
    });
    // Like a fork completing in the background while confirmation is visible.
    open(&desktop, "/demo/background", None, &mut cx);
    assert!(cx.debug_bounds("confirmation-dialog").is_some());
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("confirmation-dialog").is_none());
    ws.read_with(&cx, |w, _| assert_eq!(w.tabs.len(), 2));
}

#[gpui::test]
fn inspector_navigation_keeps_workspace_shortcuts_and_hidden_drafts_safe(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let first = tab(&desktop, 0, &cx);
    set_draft(&first, "Do not submit this", &mut cx);
    let count = first
        .controller
        .read_with(&cx, |c, _| c.model().messages.len());
    let button = cx.debug_bounds("tab-changes").unwrap();
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    cx.simulate_keystrokes("enter secondary-n");
    cx.run_until_parked();
    assert!(cx.debug_bounds("new-session-dialog").is_some());
    let create = cx.debug_bounds("create-session").unwrap();
    cx.simulate_click(create.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    workspace(&desktop, &cx).read_with(&cx, |w, _| assert_eq!(w.tabs.len(), 2));
    assert_eq!(draft(&first, &cx), "Do not submit this");
    first
        .controller
        .read_with(&cx, |c, _| assert_eq!(c.model().messages.len(), count));
}

#[gpui::test]
fn all_tools_are_single_line_calls_until_explicitly_disclosed(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    // Finished steps of a live run collapse to chips; open the edit step to see its call.
    assert!(cx.debug_bounds("step-chip-edit-1").is_some());
    expand_activity_for_tool(&tab(&desktop, 0, &cx), "edit-1", &mut cx);
    for (header, details, height, disclosed) in [
        ("tool-header-edit-1", "tool-details-edit-1", 24., false),
        ("tool-header-bash-1", "tool-details-bash-1", 32., true),
    ] {
        assert_eq!(cx.debug_bounds(header).unwrap().size.height, px(height));
        assert_eq!(cx.debug_bounds(details).is_some(), disclosed);
    }
}

#[gpui::test]
fn the_landing_leads_with_the_task_and_starters_only_fill_the_draft(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let id = open(&desktop, "/demo/new", None, &mut cx);
    let session = tab(&desktop, id, &cx);
    let landing = cx
        .debug_bounds("landing")
        .expect("a new session lands here");
    let composer = cx.debug_bounds("composer").expect("with the composer");
    let starters = cx.debug_bounds("landing-starters").unwrap();
    assert!(
        landing.contains(&composer.origin) && composer.bottom() < starters.top(),
        "the composer sits in the landing, above the starting points"
    );
    assert!(
        cx.debug_bounds("tab-changes").is_none(),
        "nothing to review before the first prompt"
    );
    let explain = cx.debug_bounds("landing-starter-1").unwrap();
    cx.simulate_click(explain.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        draft(&session, &cx),
        "Explain how this project is organized and where I should start."
    );
    session.controller.read_with(&cx, |c, _| {
        assert!(c.model().messages.is_empty(), "nothing is sent")
    });
    set_draft(&session, "Keep mine", &mut cx);
    let review = cx.debug_bounds("landing-starter-0").unwrap();
    cx.simulate_click(review.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(
        draft(&session, &cx).starts_with("Keep mine\n\nReview the local changes"),
        "a starting point never replaces a draft"
    );
}

#[gpui::test]
fn on_the_landing_the_command_menu_opens_below_the_composer(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    open(&desktop, "/demo/new", None, &mut cx);
    let composer = cx.debug_bounds("composer").unwrap();
    cx.simulate_input("/");
    cx.run_until_parked();
    let menu = cx
        .debug_bounds("slash-menu")
        .expect("typing / opens the menu");
    assert!(
        menu.top() >= composer.bottom(),
        "near the top of the page there is no room above: {menu:?} vs {composer:?}"
    );
}

#[gpui::test]
fn the_slash_button_opens_its_menu_in_its_own_column(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    let composer = cx.debug_bounds("composer").unwrap();
    let attach = cx.debug_bounds("attach").unwrap();
    let button = cx
        .debug_bounds("slash-button")
        .expect("the slash button sits in the composer toolbar");
    assert!(
        (button.top() - attach.top()).abs() < px(1.) && button.left() > attach.right(),
        "beside the paperclip: {button:?} vs {attach:?}"
    );
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert_eq!(draft(&session, &cx), "/");
    let menu = cx
        .debug_bounds("slash-menu")
        .expect("the button opens the menu");
    assert!(
        menu.left() <= button.left()
            && button.right() <= menu.right()
            && menu.bottom() <= composer.top(),
        "above the composer, over the button and the caret: {menu:?} vs {button:?}"
    );
    let tags = [
        cx.debug_bounds("slash-tag-0").unwrap(),
        cx.debug_bounds("slash-tag-1").unwrap(),
    ];
    let row = cx.debug_bounds("slash-item-0").unwrap();
    assert!(
        (tags[0].right() - tags[1].right()).abs() < px(1.)
            && row.right() - tags[0].right() < px(12.),
        "tags line up on the right: {tags:?} in {row:?}"
    );
}

#[gpui::test]
fn on_a_narrow_window_requested_details_open_as_a_drawer(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    set_draft(&session, "Keep this draft", &mut cx);
    cx.simulate_resize(size(px(1000.), px(680.)));
    cx.run_until_parked();
    let composer = cx.debug_bounds("composer").unwrap();
    cx.simulate_keystrokes("ctrl-shift-i");
    cx.run_until_parked();
    let drawer = cx
        .debug_bounds("inspector-drawer")
        .expect("an explicit request shows details below the docking width");
    assert!(drawer.right() <= px(1000.) && drawer.size.width <= px(360.));
    assert_eq!(
        cx.debug_bounds("composer").unwrap().size.width,
        composer.size.width,
        "the work keeps its width under the drawer"
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("inspector-drawer").is_none(),
        "Escape closes it"
    );
    assert_eq!(draft(&session, &cx), "Keep this draft");
    cx.simulate_resize(size(px(1440.), px(900.)));
    cx.simulate_keystrokes("ctrl-shift-i");
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("inspector-drawer").is_none()
            && cx.debug_bounds("inspector-usage").is_some(),
        "with room, details dock beside the work"
    );
}
