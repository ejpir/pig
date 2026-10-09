use super::super::app_views::AppView;
use super::*;

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
}

#[gpui::test]
fn shell_prefix_is_explicit_and_demo_never_executes_or_submits_it(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let tab = tab(&desktop, 0, &cx);
    let before = tab
        .controller
        .read_with(&cx, |controller, _| controller.model().messages.len());
    set_draft(&tab, "!!printf '雪\\n'", &mut cx);
    assert!(cx.debug_bounds("shell-mode").is_some());
    tab.composer
        .update(&mut cx, |composer, cx| composer.send_prompt(false, cx));
    assert_eq!(draft(&tab, &cx), "!!printf '雪\\n'");
    assert_eq!(
        tab.controller
            .read_with(&cx, |controller, _| controller.model().messages.len()),
        before
    );
    assert!(
        tab.controller
            .read_with(&cx, |controller, _| controller.model().shell.is_none())
    );
    tab.composer
        .update(&mut cx, |composer, cx| composer.send_prompt(true, cx));
    assert!(tab.controller.read_with(&cx, |controller, _| {
        controller
            .model()
            .notice
            .as_ref()
            .unwrap()
            .contains("explicitly")
    }));
}

#[gpui::test]
fn extension_confirm_select_are_cancel_default_correlated_and_do_not_leak_shortcuts(
    cx: &mut TestAppContext,
) {
    let (desktop, mut cx) = setup(cx);
    let tab = tab(&desktop, 0, &cx);
    set_draft(&tab, "Retain my draft", &mut cx);
    receive(
        &tab,
        json!({"type":"extension_ui_request","id":"confirm-1","method":"confirm","title":"Extension decision","message":"Only the extension owns this decision."}),
        &mut cx,
    );
    cx.run_until_parked();
    assert!(cx.debug_bounds("confirmation-dialog").is_some());
    cx.simulate_keystrokes("secondary-n");
    cx.run_until_parked();
    assert_eq!(
        workspace(&desktop, &cx).read_with(&cx, |workspace, _| workspace.tabs.len()),
        1
    );
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let response = tab.controller.read_with(&cx, |controller, _| {
        controller.extension_responses.last().unwrap().clone()
    });
    assert_eq!(
        response,
        json!({"type":"extension_ui_response","id":"confirm-1","cancelled":true})
    );
    assert_eq!(draft(&tab, &cx), "Retain my draft");
    receive(
        &tab,
        json!({"type":"extension_ui_request","id":"select-2","method":"select","title":"Synthetic permission request","options":["Allow once","Block"]}),
        &mut cx,
    );
    cx.run_until_parked();
    click("prompt-choice-2", &mut cx);
    let response = tab.controller.read_with(&cx, |controller, _| {
        controller.extension_responses.last().unwrap().clone()
    });
    assert_eq!(
        response,
        json!({"type":"extension_ui_response","id":"select-2","value":"Block"})
    );
    receive(
        &tab,
        json!({"type":"extension_ui_request","id":"cancel-3","method":"confirm","title":"Aborted request","message":"No work happens"}),
        &mut cx,
    );
    cx.run_until_parked();
    receive(
        &tab,
        json!({"type":"extension_ui_cancel","id":"cancel-3"}),
        &mut cx,
    );
    cx.run_until_parked();
    assert!(cx.debug_bounds("confirmation-dialog").is_none());

    receive(
        &tab,
        json!({"type":"extension_ui_request","id":"timeout-4","method":"confirm","title":"Timed request","message":"Pi resolves this automatically","timeout":10}),
        &mut cx,
    );
    cx.run_until_parked();
    assert!(cx.debug_bounds("confirmation-dialog").is_some());
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(11));
    cx.run_until_parked();
    assert!(cx.debug_bounds("confirmation-dialog").is_none());
    let response = tab.controller.read_with(&cx, |controller, _| {
        controller.extension_responses.last().unwrap().clone()
    });
    assert_eq!(
        response,
        json!({"type":"extension_ui_response","id":"timeout-4","cancelled":true})
    );
}

#[gpui::test]
fn new_session_chooser_preserves_drafts_and_starts_only_on_create(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let first = tab(&desktop, 0, &cx);
    set_draft(&first, "Keep this text", &mut cx);
    open(&desktop, "/demo/repos/zed", None, &mut cx);
    select(&desktop, 0, &mut cx);
    cx.simulate_keystrokes("secondary-n");
    cx.run_until_parked();
    assert!(cx.debug_bounds("new-session-dialog").is_some());
    assert_eq!(
        workspace(&desktop, &cx).read_with(&cx, |workspace, _| workspace.tabs.len()),
        2
    );
    click("cancel-new-session", &mut cx);
    assert_eq!(draft(&first, &cx), "Keep this text");
    cx.simulate_keystrokes("secondary-n");
    cx.run_until_parked();
    click("new-session-project-1", &mut cx);
    click("create-session", &mut cx);
    workspace(&desktop, &cx).read_with(&cx, |workspace, _| {
        assert_eq!(workspace.tabs.len(), 3);
        assert_eq!(
            workspace.active_summary().cwd,
            PathBuf::from("/demo/repos/zed")
        );
    });
    assert_eq!(draft(&first, &cx), "Keep this text");
}

#[gpui::test]
fn many_processes_scroll_inside_the_popup_without_moving_header_or_footer(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    for _ in 0..20 {
        workspace(&desktop, &cx).update(&mut cx, |workspace, cx| {
            workspace.open(PathBuf::from("/demo/repos/pi"), None, cx)
        });
    }
    cx.run_until_parked();
    click("status-processes", &mut cx);
    let popup = cx.debug_bounds("pi-processes-popup").unwrap();
    let list = cx.debug_bounds("pi-processes-list").unwrap();
    assert!(list.size.height < popup.size.height);
    assert!(popup.size.height <= px(668.));
    let status = desktop.read_with(&cx, |desktop, _| desktop.status.clone());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: list.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-400.))),
        ..Default::default()
    });
    cx.run_until_parked();
    assert!(status.read_with(&cx, |status, _| status.process_scroll.offset().y) < px(0.));
    assert_eq!(cx.debug_bounds("pi-processes-popup").unwrap(), popup);
    assert_eq!(cx.debug_bounds("pi-processes-list").unwrap(), list);
    cx.simulate_keystrokes("escape");
    assert!(cx.debug_bounds("pi-processes-popup").is_none());
}

#[gpui::test]
fn process_popup_lists_owned_sessions_without_fabricated_pids_and_restores_focus(
    cx: &mut TestAppContext,
) {
    let (desktop, mut cx) = setup(cx);
    let first = tab(&desktop, 0, &cx);
    set_draft(&first, "Process popup keeps this draft", &mut cx);
    open(&desktop, "/demo/second", None, &mut cx);
    select(&desktop, 0, &mut cx);
    click("status-processes", &mut cx);
    assert!(cx.debug_bounds("pi-processes-popup").is_some());
    assert!(cx.debug_bounds("pi-process-0").is_some());
    assert!(cx.debug_bounds("pi-process-1").is_some());
    assert!(
        first
            .controller
            .read_with(&cx, |controller, _| controller.pid().is_none())
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("pi-processes-popup").is_none());
    assert_eq!(draft(&first, &cx), "Process popup keeps this draft");
    click("status-processes", &mut cx);
    click("pi-process-1", &mut cx);
    assert_eq!(
        workspace(&desktop, &cx).read_with(&cx, |workspace, _| workspace.active),
        SessionId(1)
    );
}

#[gpui::test]
fn project_resources_bind_an_existing_background_owner_and_keep_unknown_distinct(
    cx: &mut TestAppContext,
) {
    let (desktop, mut cx) = setup(cx);
    show_inspector(&desktop, &mut cx);
    let first = tab(&desktop, 0, &cx);
    open(&desktop, "/demo/repos/zed", None, &mut cx);
    let second = tab(&desktop, 1, &cx);
    select(&desktop, 0, &mut cx);
    receive(
        &first,
        json!({"type":"response","command":"list_packages","success":true,"data":{"packages":[
        {"source":"user-package","scope":"user","filtered":false}, {"source":"first-project","scope":"project","filtered":false}]}}),
        &mut cx,
    );
    receive(
        &second,
        json!({"type":"response","command":"list_packages","success":true,"data":{"packages":[{"source":"second-project","scope":"project","filtered":false}]}}),
        &mut cx,
    );
    receive(
        &second,
        json!({"type":"response","command":"get_project_trust","success":true,"data":{"cwd":"/demo/repos/zed","trusted":true,"hasProjectResources":true,
        "projectSettings":{"extensions":["./guard.ts"]}, "loadedExtensions":[{"path":"/demo/repos/zed/guard.ts","status":"loaded","commands":[],"sourceInfo":{"path":"/demo/repos/zed/guard.ts","source":"local","scope":"project","origin":"top-level"}}]}}),
        &mut cx,
    );
    workspace(&desktop, &cx).update(&mut cx, |workspace, cx| {
        workspace.show_view(AppView::Resources, cx)
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("resource-scope-user").is_some());
    click("resource-project-picker", &mut cx);
    click("resource-project-1", &mut cx);
    assert_eq!(
        workspace(&desktop, &cx).read_with(&cx, |workspace, _| workspace.tabs.len()),
        2
    );
    assert_eq!(
        workspace(&desktop, &cx).read_with(&cx, |workspace, _| workspace.active),
        SessionId(0)
    );
    assert!(cx.debug_bounds("project-settings-preview").is_some());
    click("resource-tab-Packages", &mut cx);
    assert!(cx.debug_bounds("project-settings-preview").is_none());
    click("resource-row-0", &mut cx);
    click("resource-copy", &mut cx);
    let copied = cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert!(copied.contains("second-project"));
    assert!(!copied.contains("first-project"));
    click("resource-tab-Extensions", &mut cx);
    click("resource-row-0", &mut cx);
    click("resource-copy", &mut cx);
    assert!(
        cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap())
            .contains("guard.ts")
    );
    click("resource-scope-user", &mut cx);
    for selector in [
        "resource-tab-Extensions",
        "resource-tab-Skills",
        "resource-tab-Prompts",
        "resource-tab-Context files",
    ] {
        click(selector, &mut cx);
        assert!(
            cx.debug_bounds("project-settings-preview").is_none(),
            "User scope must remain resource-focused for {selector}"
        );
    }
    click("resource-tab-Packages", &mut cx);
    click("resource-row-0", &mut cx);
    click("resource-copy", &mut cx);
    assert!(
        cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap())
            .contains("user-package")
    );
}

#[gpui::test]
fn new_session_study_geometry_keeps_footer_and_fields_visible(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    open(&desktop, "/demo/zed", None, &mut cx);
    open(&desktop, "/demo/minivm", None, &mut cx);
    cx.simulate_keystrokes("secondary-n");
    cx.run_until_parked();
    click("new-session-worktree", &mut cx);
    let dialog = cx.debug_bounds("new-session-dialog").unwrap();
    assert_eq!(dialog.size.width, px(500.));
    assert!(dialog.size.height <= px(620.));
    for selector in [
        "new-session-branch",
        "new-session-path",
        "new-session-warning",
        "new-session-footer",
        "create-session",
    ] {
        let bounds = cx.debug_bounds(selector).unwrap();
        assert!(
            bounds.origin.y >= dialog.origin.y && bounds.bottom() <= dialog.bottom(),
            "{selector}: {bounds:?} outside {dialog:?}"
        );
    }
    cx.simulate_keystrokes("enter ctrl-shift-d secondary-n secondary-b");
    cx.run_until_parked();
    assert!(cx.debug_bounds("new-session-dialog").is_some());
    assert_eq!(
        workspace(&desktop, &cx).read_with(&cx, |workspace, _| workspace.tabs.len()),
        3
    );
    cx.simulate_resize(size(px(1000.), px(540.)));
    cx.run_until_parked();
    let dialog = cx.debug_bounds("new-session-dialog").unwrap();
    let create = cx.debug_bounds("create-session").unwrap();
    assert!(dialog.bottom() <= px(540.) && create.bottom() <= dialog.bottom());
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("new-session-dialog").is_none());
}

#[gpui::test]
fn initial_model_picker_floats_filters_and_handles_keyboard_without_changing_current_session(
    cx: &mut TestAppContext,
) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    receive(
        &session,
        json!({"type":"response","command":"get_available_models","success":true,"data":{"models":[
            {"id":"alpha","provider":"fixture"}, {"id":"beta","provider":"fixture"}
        ]}}),
        &mut cx,
    );
    let original = session.controller.read_with(&cx, |controller, _| {
        controller
            .model()
            .state
            .model
            .as_ref()
            .map(|model| (model.provider.clone(), model.id.clone()))
    });
    cx.simulate_keystrokes("secondary-n");
    cx.run_until_parked();
    let dialog = cx.debug_bounds("new-session-dialog").unwrap();
    let footer = cx.debug_bounds("new-session-footer").unwrap();
    click("new-session-model", &mut cx);
    assert!(cx.debug_bounds("new-session-model-picker").is_some());
    assert_eq!(cx.debug_bounds("new-session-dialog").unwrap(), dialog);
    assert_eq!(cx.debug_bounds("new-session-footer").unwrap(), footer);
    cx.simulate_input("beta");
    cx.run_until_parked();
    assert!(cx.debug_bounds("new-model-choice-1").is_some());
    assert!(cx.debug_bounds("new-model-choice-2").is_none());
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    assert!(cx.debug_bounds("new-session-model-picker").is_none());
    assert!(cx.debug_bounds("new-session-dialog").is_some());
    assert_eq!(
        session.controller.read_with(&cx, |controller, _| controller
            .model()
            .state
            .model
            .as_ref()
            .map(|model| (model.provider.clone(), model.id.clone()))),
        original
    );
    click("new-session-model", &mut cx);
    cx.simulate_input("beta");
    cx.run_until_parked();
    assert!(cx.debug_bounds("new-model-selected-1").is_some());
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("new-session-model-picker").is_none());
    assert!(cx.debug_bounds("new-session-dialog").is_some());
    click("new-session-model", &mut cx);
    click("new-session-project-folder", &mut cx);
    assert!(cx.debug_bounds("new-session-model-picker").is_none());
    click("cancel-new-session", &mut cx);
    assert_eq!(
        workspace(&desktop, &cx).read_with(&cx, |workspace, _| workspace.tabs.len()),
        1
    );
}

#[gpui::test]
fn session_notices_float_without_reflow_and_share_catalog_dismissal(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    let composer = cx.debug_bounds("composer").unwrap();
    session.controller.update(&mut cx, |controller, cx| {
        controller.notice(
            "Sign in: run pi in the terminal, then /login. Restart afterward.",
            cx,
        )
    });
    cx.run_until_parked();
    let toast = cx.debug_bounds("session-notice").unwrap();
    assert!(toast.size.width <= px(440.));
    assert_eq!(cx.debug_bounds("composer").unwrap(), composer);
    workspace(&desktop, &cx).update(&mut cx, |workspace, cx| {
        workspace.show_view(AppView::Resources, cx)
    });
    cx.run_until_parked();
    assert_eq!(cx.debug_bounds("catalog-toast").unwrap().size, toast.size);
    click("catalog-toast-close", &mut cx);
    select(&desktop, 0, &mut cx);
    assert!(cx.debug_bounds("session-notice").is_none());
    session.controller.update(&mut cx, |controller, cx| {
        controller.notice("A separate notification", cx)
    });
    cx.run_until_parked();
    click("session-notice-close", &mut cx);
    assert!(
        session
            .controller
            .read_with(&cx, |controller, _| controller.model().notice.is_none())
    );
}

#[gpui::test]
fn shell_copy_controls_are_small_corner_icons_and_copy_original_text(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    let command = "printf '雪\\n'";
    let output = "10%\r100%\r\n雪\n  original trailing spaces  \n";
    receive(
        &session,
        json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[{
            "role":"bashExecution", "command":command, "output":output, "exitCode":0, "excludeFromContext":true, "timestamp":1700000000000u64
        }]}}),
        &mut cx,
    );
    cx.run_until_parked();
    session.transcript.read_with(&cx, |view, cx| {
        assert_eq!(
            view.documents.copy_source("0:shell-output", cx),
            Some("100%\n雪\n  original trailing spaces  \n")
        )
    });
    let first = cx.debug_bounds("copy-shell-command-0").unwrap();
    let second = cx.debug_bounds("copy-shell-output-0").unwrap();
    assert_eq!(first.size, size(px(20.), px(20.)));
    assert_eq!(second.size, size(px(20.), px(20.)));
    assert!(second.origin.x > first.origin.x);
    click("copy-shell-command-0", &mut cx);
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        command
    );
    click("copy-shell-output-0", &mut cx);
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        output
    );
}

#[test]
fn worktree_creation_is_argv_based_no_overwrite_and_preserves_source_files() {
    use std::{ffi::OsStr, fs, process::Command as ProcessCommand, time::Duration};
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    fs::create_dir(&project).unwrap();
    let hooks = root.path().join("empty-hooks");
    fs::create_dir(&hooks).unwrap();
    let git = |args: &[&OsStr]| {
        let output = pi_core::bounded_output(
            ProcessCommand::new("git")
                .current_dir(&project)
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .args(["-c", "commit.gpgsign=false", "-c", "core.fsmonitor=false"])
                .arg("-c")
                .arg(format!("core.hooksPath={}", hooks.display()))
                .args(args),
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init".as_ref(), "-b".as_ref(), "main".as_ref()]);
    git(&[
        "config".as_ref(),
        "core.hooksPath".as_ref(),
        hooks.as_os_str(),
    ]);
    git(&[
        "config".as_ref(),
        "core.fsmonitor".as_ref(),
        "false".as_ref(),
    ]);
    fs::write(project.join("sentinel.txt"), "untouched source\n").unwrap();
    git(&["add".as_ref(), "sentinel.txt".as_ref()]);
    git(&[
        "-c".as_ref(),
        "user.name=Fixture".as_ref(),
        "-c".as_ref(),
        "user.email=fixture@example.invalid".as_ref(),
        "commit".as_ref(),
        "-m".as_ref(),
        "Synthetic fixture".as_ref(),
    ]);
    // Resolved, as the created worktree is (macOS's /var is /private/var).
    let target = root
        .path()
        .canonicalize()
        .unwrap()
        .join("worktree with spaces");
    let actual =
        super::super::new_session::create_worktree(&project, "pi/fixture;literal", &target)
            .unwrap();
    assert_eq!(actual, target);
    assert_eq!(
        fs::read_to_string(project.join("sentinel.txt")).unwrap(),
        "untouched source\n"
    );
    assert_eq!(
        fs::read_to_string(target.join("sentinel.txt")).unwrap(),
        "untouched source\n"
    );
    assert!(super::super::new_session::create_worktree(&project, "pi/other", &target).is_err());
    assert!(
        super::super::new_session::create_worktree(
            &project,
            "--force",
            &root.path().join("invalid")
        )
        .is_err()
    );
    assert!(
        super::super::new_session::create_worktree(
            &project,
            "pi/metadata",
            &project.join(".git").join("bad")
        )
        .is_err()
    );
}
