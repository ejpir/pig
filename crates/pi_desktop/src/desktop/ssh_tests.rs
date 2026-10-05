use super::*;
use pi_core::{
    session::RunState,
    ssh::{PROTOCOL_VERSION, SshTarget},
};

#[gpui::test]
fn remote_disconnect_does_not_mark_an_agent_idle_and_snapshots_replace_history(
    cx: &mut TestAppContext,
) {
    let target = SshTarget::new("dev".into(), "/remote/work".into()).unwrap();
    let controller = cx.new(|cx| SessionController::remote_test(target.clone(), cx));
    let mut remote = Session::new("/remote/work".into());
    remote.apply(&json!({"type":"agent_start"})).unwrap();
    remote.apply(&json!({"type":"message_start","message":{"role":"assistant","content":[{"type":"text","text":"partial"}]}})).unwrap();
    let snapshot =
        json!({"type":"remote_snapshot","version":PROTOCOL_VERSION,"key":target.key,"data":remote});
    controller.update(cx, |controller, cx| {
        controller.receive(TransportEvent::Record(snapshot.clone()), cx);
        controller.receive(
            TransportEvent::Exited {
                description: "SSH disconnected".into(),
                stderr: String::new(),
            },
            cx,
        );
        assert_eq!(controller.model().run, RunState::Running);
        assert!(!controller.is_connected());
        controller.receive(TransportEvent::Record(snapshot), cx);
        assert!(controller.is_connected());
        assert_eq!(controller.model().messages.len(), 1);
        assert_eq!(controller.model().streaming_message_index(), Some(0));
        assert_eq!(controller.model().cwd, target.identity());
        assert!(controller.model().saved.is_empty());
    });
}

#[gpui::test]
fn mismatched_remote_identity_never_enables_submission(cx: &mut TestAppContext) {
    let target = SshTarget::new("dev".into(), "/remote/work".into()).unwrap();
    let controller = cx.new(|cx| SessionController::remote_test(target, cx));
    controller.update(cx, |controller, cx| {
        controller.receive(TransportEvent::Record(json!({"type":"remote_snapshot","version":PROTOCOL_VERSION,"key":"wrong","data":Session::default()})), cx);
        assert!(controller.bootstrap_failed());
        assert!(!controller.ready());
    });
}

#[gpui::test]
fn remote_services_cannot_open_local_files_or_spawn_local_terminals(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let target = SshTarget::new("dev".into(), "/remote/work".into()).unwrap();
    let controller = cx.update(|cx| {
        cx.set_global(Theme::new(false));
        init(cx);
        cx.new(|cx| SessionController::remote_test(target, cx))
    });
    let window = cx.update(|cx| {
        cx.open_window(WindowOptions::default(), |_, cx| {
            cx.new(|cx| SessionView::new(controller.clone(), "", cx))
        })
        .unwrap()
    });
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let view = window.root(&mut visual).unwrap();
    let file = directory.path().join("local.txt");
    std::fs::write(&file, "must stay local").unwrap();
    view.update_in(&mut visual, |view, window, cx| {
        view.files.update(cx, |files, cx| {
            files.open(file, cx);
        });
        view.toggle_terminal(window, cx);
        view.set_page(panels::SessionPage::Changes, cx);
    });
    visual.run_until_parked();
    view.read_with(&visual, |view, cx| {
        assert!(!view.files.read(cx).has_tabs());
        assert!(view.files.read(cx).file_entries().is_empty());
        assert_eq!(view.terminal.read(cx).count(), 0);
        assert_eq!(view.page, panels::SessionPage::Thread);
    });
}

#[gpui::test]
fn saved_ssh_sessions_can_be_removed_from_all_sessions_without_remote_operations(
    cx: &mut TestAppContext,
) {
    let config = tempfile::tempdir().unwrap();
    let target = SshTarget::new("dev".into(), "/remote/work".into()).unwrap();
    let workspace = cx.update(|cx| {
        cx.set_global(Theme::new(false));
        cx.set_global(crate::prefs::Prefs::load_from(Some(config.path())));
        init(cx);
        crate::prefs::remember_open_sessions(
            cx,
            &[crate::prefs::OpenSession {
                cwd: target.identity(),
                saved: None,
                remote: Some(target.clone()),
            }],
            0,
        );
        cx.new(|_| {
            let mut workspace = WorkspaceController::new(false);
            workspace.selected_project = Some(target.identity());
            workspace
        })
    });
    let window = cx.update(|cx| {
        let search = cx.new(|cx| TextInput::new("Search", cx));
        cx.open_window(WindowOptions::default(), |_, cx| {
            cx.new(|cx| app_views::SessionsView::new(workspace.clone(), search, cx))
        })
        .unwrap()
    });
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let bounds = visual
        .debug_bounds("remove-ssh-session-0")
        .expect("SSH session has a Remove action");
    visual.simulate_click(bounds.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    visual.update(|_, cx| assert!(crate::prefs::remote_sessions(cx).is_empty()));
    workspace.read_with(&visual, |workspace, cx| {
        assert!(
            workspace.tabs.is_empty(),
            "removing a shortcut never starts a remote connection"
        );
        assert!(
            workspace.selected_is_remote(cx),
            "forgetting a session does not turn its project into a local filesystem path"
        );
    });
    assert!(visual.debug_bounds("ssh-sessions").is_none());
}
