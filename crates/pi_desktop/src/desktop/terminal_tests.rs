use super::*;
use gpui::Modifiers;
use pi_terminal::{Shell, TerminalView};
use std::time::Duration;

/// A session in a real folder whose drawer starts `/bin/sh`.
fn terminal_session(
    desktop: &Entity<Desktop>,
    cx: &mut VisualTestContext,
) -> (
    Tab,
    Entity<terminal::TerminalDrawer>,
    PathBuf,
    tempfile::TempDir,
) {
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().canonicalize().unwrap();
    let id = open(desktop, folder.to_str().unwrap(), None, cx);
    let t = tab(desktop, id, cx);
    let drawer = t.view.read_with(cx, |view, _| view.terminal.clone());
    drawer.update(cx, |drawer, _| {
        drawer.use_shell(Shell::Program("/bin/sh".into()))
    });
    (t, drawer, folder, dir)
}

/// PTYs are real processes; their output arrives on another thread.
fn wait_until(
    cx: &mut VisualTestContext,
    what: &str,
    mut done: impl FnMut(&mut VisualTestContext) -> bool,
) {
    for _ in 0..300 {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

fn active_terminal(
    drawer: &Entity<terminal::TerminalDrawer>,
    cx: &mut VisualTestContext,
) -> Entity<TerminalView> {
    let mut view = None;
    wait_until(cx, "a terminal", |cx| {
        view = drawer.read_with(cx, |drawer, _| drawer.active_view());
        view.is_some()
    });
    view.unwrap()
}

fn output(view: &Entity<TerminalView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |view, cx| view.terminal().read(cx).get_content())
}

#[gpui::test]
fn the_shortcut_opens_a_terminal_in_the_session_folder(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let (t, drawer, folder, _dir) = terminal_session(&desktop, &mut cx);
    set_draft(&t, "a draft for pi", &mut cx);
    assert!(cx.debug_bounds("terminal-drawer").is_none());

    cx.simulate_keystrokes("ctrl-`");
    let view = active_terminal(&drawer, &mut cx);
    assert!(cx.debug_bounds("terminal-drawer").is_some());
    // Enter submits a prompt elsewhere in the window; in a terminal it runs the command.
    cx.simulate_input("pwd");
    cx.simulate_keystrokes("enter");
    let folder = folder.display().to_string();
    wait_until(&mut cx, "pwd output", |cx| {
        output(&view, cx).contains(&folder)
    });
    assert_eq!(draft(&t, &cx), "a draft for pi");

    // Focused, the shortcut hides the drawer; the shell keeps running.
    cx.simulate_keystrokes("ctrl-`");
    cx.run_until_parked();
    assert!(cx.debug_bounds("terminal-drawer").is_none());
    drawer.read_with(&cx, |drawer, _| assert_eq!(drawer.count(), 1));
    let focus = t
        .composer
        .read_with(&cx, |composer, cx| composer.input.focus_handle(cx));
    let composer_focused = cx.update(|window, _| focus.is_focused(window));
    assert!(
        composer_focused,
        "hiding the drawer returns focus to the composer"
    );

    // The header button is always there and brings the same terminal back.
    let button = cx.debug_bounds("terminal-toggle").expect("terminal button");
    cx.simulate_click(button.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("terminal-drawer").is_some());
    drawer.read_with(&cx, |drawer, _| assert_eq!(drawer.count(), 1));
    assert!(output(&view, &mut cx).contains(&folder));

    // + starts a second shell in its own tab; closing a tab ends only that shell.
    let new = cx.debug_bounds("terminal-new").expect("+ button");
    cx.simulate_click(new.center(), Modifiers::none());
    wait_until(&mut cx, "a second terminal", |cx| {
        drawer.read_with(cx, |drawer, _| drawer.count() == 2)
    });
    assert!(cx.debug_bounds("terminal-tab-1").is_some());
    let second = active_terminal(&drawer, &mut cx);
    assert_ne!(second, view);
    let id = drawer.read_with(&cx, |drawer, _| drawer.tab_id(1));
    drawer.update_in(&mut cx, |drawer, window, cx| drawer.close(id, window, cx));
    assert_eq!(active_terminal(&drawer, &mut cx), view);
}

// Exiting the shell closes its tab (`Event::CloseTerminal`). Under the test
// executor on Linux the PTY's child exit is not reported, as in Zed's own tests,
// so that is checked in the running app instead.

#[gpui::test]
fn a_bash_row_types_its_command_in_a_new_terminal(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let (t, drawer, _folder, _dir) = terminal_session(&desktop, &mut cx);
    receive(
        &t,
        json!({"type":"message_end","message":{"role":"assistant","content":[
            {"type":"toolCall","id":"bash","name":"bash","arguments":{"command":"echo from-$((6 * 7))"}}
        ]}}),
        &mut cx,
    );
    receive(
        &t,
        json!({"type":"tool_execution_end","toolCallId":"bash","result":{"content":[{"type":"text","text":"from-42"}]}}),
        &mut cx,
    );
    cx.run_until_parked();

    expand_activity_for_tool(&t, "bash", &mut cx);
    // The button shows while the row is hovered.
    let row = cx.debug_bounds("tool-header-bash").expect("bash row");
    cx.simulate_mouse_move(row.center(), None, Modifiers::none());
    cx.run_until_parked();
    let button = cx
        .debug_bounds("open-in-terminal-bash")
        .expect("Open in terminal on a finished bash row");
    cx.simulate_click(button.center(), Modifiers::none());
    let view = active_terminal(&drawer, &mut cx);
    // `/bin/sh` has no bracketed paste, so the command is typed after a short wait.
    cx.executor().advance_clock(Duration::from_secs(3));
    wait_until(&mut cx, "the typed command", |cx| {
        output(&view, cx).contains("echo from-$((6 * 7))")
    });
    assert!(
        !output(&view, &mut cx).contains("from-42"),
        "the command must wait for Enter"
    );
    // The row was not expanded by the click.
    assert!(cx.debug_bounds("tool-details-bash").is_none());
}
