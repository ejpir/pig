use super::*;
use gpui::{TestAppContext, VisualContext as _, VisualTestContext, px, rgb};
#[cfg(feature = "mobile")]
use std::{
    borrow::Cow,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

fn setup(cx: &mut TestAppContext) -> VisualTestContext {
    cx.executor().allow_parking();
    cx.update(|cx| {
        settings::init(cx);
        theme_settings::init(zed_theme::LoadThemes::JustBase, cx);
        cx.set_global(TerminalStyle {
            font_family: "Courier".into(),
            font_fallbacks: None,
            font_size: px(12.),
            line_height: px(18.),
            cursor: rgb(0x6a9fcc).into(),
            selection: rgb(0x273748).into(),
            link: rgb(0x9cc2e0).into(),
        });
        init(cx);
    });
    cx.add_empty_window().clone()
}

/// Opens `/bin/sh` in `cwd` and shows it focused in the test window.
async fn open_shell(cwd: PathBuf, cx: &mut VisualTestContext) -> Entity<TerminalView> {
    open_program(cwd, Shell::Program("/bin/sh".into()), cx).await
}

async fn open_program(
    cwd: PathBuf,
    shell: Shell,
    cx: &mut VisualTestContext,
) -> Entity<TerminalView> {
    let spawn = cx.update(|window, cx| spawn_shell(cwd, shell, window, cx));
    let terminal = spawn.await.unwrap();
    let view = cx.new_window_entity(|window, cx| TerminalView::new(terminal, window, cx));
    cx.update(|window, cx| {
        window.replace_root(cx, |_, _| RootView(view.clone()));
        let focus = view.read(cx).focus_handle.clone();
        focus.focus(window, cx);
    });
    cx.run_until_parked();
    view
}

struct RootView(Entity<TerminalView>);
impl Render for RootView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.0.clone())
    }
}

fn content(view: &Entity<TerminalView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |view, cx| view.terminal.read(cx).get_content())
}

/// Polls until the terminal shows `expected`; PTY output arrives on its own thread.
fn wait_for(view: &Entity<TerminalView>, expected: &str, cx: &mut VisualTestContext) -> String {
    for _ in 0..300 {
        cx.run_until_parked();
        let text = content(view, cx);
        if text.contains(expected) {
            return text;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "terminal never showed {expected:?}; it shows:\n{}",
        content(view, cx)
    );
}

#[gpui::test]
async fn typed_keys_reach_a_shell_started_in_the_folder(cx: &mut TestAppContext) {
    let cx = &mut setup(cx);
    let folder = std::env::temp_dir().canonicalize().unwrap();
    let view = open_shell(folder.clone(), cx).await;
    // Enter is bound in the Terminal context, the way the app's own Enter
    // binding is overridden inside terminals.
    cx.simulate_input("pwd");
    cx.simulate_keystrokes("enter");
    wait_for(&view, &folder.display().to_string(), cx);
}

#[gpui::test]
async fn type_at_prompt_types_without_running(cx: &mut TestAppContext) {
    let cx = &mut setup(cx);
    let view = open_shell(std::env::temp_dir(), cx).await;
    // `/bin/sh` has no bracketed paste, so single-line text is typed after the wait.
    view.update(cx, |view, cx| {
        view.type_at_prompt("echo typed-$((40 + 2))".into(), cx)
    });
    cx.executor().advance_clock(Duration::from_secs(3));
    wait_for(&view, "echo typed-$((40 + 2))", cx);
    std::thread::sleep(Duration::from_millis(200));
    cx.run_until_parked();
    assert!(
        !content(&view, cx).contains("typed-42"),
        "the command ran:\n{}",
        content(&view, cx)
    );
    cx.simulate_keystrokes("enter");
    wait_for(&view, "typed-42", cx);
}

#[gpui::test]
async fn multi_line_text_is_not_typed_without_bracketed_paste(cx: &mut TestAppContext) {
    let cx = &mut setup(cx);
    // /bin/sh may be Bash/readline and enable bracketed paste by default.
    // This fixture explicitly disables it and then only echoes PTY input.
    let shell = Shell::WithArguments {
        program: "/bin/sh".into(),
        args: vec![
            "-c".into(),
            "printf '\\033[?2004lready-no-paste\\n'; exec cat".into(),
        ],
        title_override: None,
    };
    let view = open_program(std::env::temp_dir(), shell, cx).await;
    wait_for(&view, "ready-no-paste", cx);
    view.update(cx, |view, cx| {
        view.type_at_prompt("echo one\necho two".into(), cx)
    });
    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(200));
    cx.run_until_parked();
    let text = content(&view, cx);
    assert!(!text.contains("echo one"), "typed:\n{text}");
    assert!(view.read_with(cx, |view, _| view.pending_input.is_none()));
}

#[gpui::test]
async fn copy_and_paste_use_the_clipboard(cx: &mut TestAppContext) {
    let cx = &mut setup(cx);
    let view = open_shell(std::env::temp_dir(), cx).await;
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
        "echo pasted-$((1 + 1))".into(),
    ));
    cx.dispatch_action(Paste);
    cx.simulate_keystrokes("enter");
    wait_for(&view, "pasted-2", cx);
    cx.dispatch_action(SelectAll);
    cx.dispatch_action(Copy);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert!(
        copied.is_some_and(|text| text.contains("pasted-2")),
        "Copy after Select All did not copy the output"
    );
}

#[gpui::test]
fn a_display_only_terminal_is_not_busy(cx: &mut TestAppContext) {
    let cx = &mut setup(cx);
    let terminal = cx.new(|cx| {
        TerminalBuilder::new_display_only(
            CursorShape::Bar,
            AlternateScroll::On,
            None,
            0,
            cx.background_executor(),
            util::paths::PathStyle::local(),
        )
        .subscribe(cx)
    });
    terminal.update(cx, |terminal, cx| {
        terminal.write_output(b"\x1b[32mgreen\x1b[0m plain", cx)
    });
    let view = cx.new_window_entity(|window, cx| TerminalView::new(terminal.clone(), window, cx));
    cx.update(|window, cx| window.replace_root(cx, |_, _| RootView(view.clone())));
    cx.run_until_parked();
    terminal.read_with(cx, |terminal, _| {
        assert!(!is_busy(terminal));
        assert!(terminal.get_content().contains("green plain"));
    });
}

#[cfg(feature = "mobile")]
#[derive(Default)]
struct TestExternalPty {
    writes: Mutex<Vec<u8>>,
    shutdown: AtomicBool,
}

#[cfg(feature = "mobile")]
impl ExternalPty for TestExternalPty {
    fn write(&self, bytes: Cow<'static, [u8]>) {
        self.writes.lock().unwrap().extend_from_slice(&bytes);
    }

    fn resize(&self, _: TerminalBounds) {}

    fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }
}

#[cfg(feature = "mobile")]
#[gpui::test]
fn external_stream_accepts_raw_output_and_shuts_down(cx: &mut TestAppContext) {
    let cx = &mut setup(cx);
    let pty = Arc::new(TestExternalPty::default());
    let terminal = cx.update(|window, cx| {
        spawn_external(
            ExternalTerminalBackend {
                pty: pty.clone(),
                path_style: PathStyle::Unix,
            },
            window,
            cx,
        )
    });
    let view = cx.new_window_entity(|window, cx| TerminalView::new(terminal.clone(), window, cx));
    cx.update(|window, cx| window.replace_root(cx, |_, _| RootView(view.clone())));
    terminal.update(cx, |terminal, cx| {
        terminal.write_pty_output(b"raw-\0-output", cx)
    });
    view.update(cx, |view, cx| view.send_text("typed", cx));
    cx.run_until_parked();

    assert!(content(&view, cx).contains("raw-"));
    assert_eq!(&*pty.writes.lock().unwrap(), b"typed");
    view.update(cx, |view, cx| {
        view.disconnect(cx);
        view.disconnect(cx);
    });
    assert!(pty.shutdown.load(Ordering::Acquire));
}

#[cfg(feature = "mobile")]
#[gpui::test]
fn soft_keyboard_sentinel_deletion_sends_backspace(cx: &mut TestAppContext) {
    let cx = &mut setup(cx);
    let pty = Arc::new(TestExternalPty::default());
    let terminal = cx.update(|window, cx| {
        spawn_external(
            ExternalTerminalBackend {
                pty: pty.clone(),
                path_style: PathStyle::Unix,
            },
            window,
            cx,
        )
    });
    let view = cx.new_window_entity(|window, cx| TerminalView::new(terminal, window, cx));

    view.update(cx, |view, cx| view.send_key("backspace", cx));
    let expected = std::mem::take(&mut *pty.writes.lock().unwrap());
    assert!(!expected.is_empty());
    view.update(cx, |view, cx| {
        view.replace_ime_text(Some(0..1), "", cx);
    });
    assert_eq!(&*pty.writes.lock().unwrap(), &expected);

    pty.writes.lock().unwrap().clear();
    view.update(cx, |view, cx| {
        view.replace_ime_text(Some(1..1), "a", cx);
        // Java can report deletion before the mirror reset requested by the
        // first edit reaches InputConnection.
        view.replace_ime_text(Some(1..2), "", cx);
    });
    assert_eq!(
        &*pty.writes.lock().unwrap(),
        &[b"a".as_slice(), expected.as_slice()].concat()
    );

    pty.writes.lock().unwrap().clear();
    view.update(cx, |view, cx| {
        view.set_marked_text(Some("x".into()), Some(1..1), cx);
        view.replace_ime_text(Some(0..1), "", cx);
    });
    assert!(
        pty.writes.lock().unwrap().is_empty(),
        "deleting inside a real composition is not a shell backspace"
    );
}

#[cfg(feature = "mobile")]
#[gpui::test]
fn corrected_commit_and_new_preedit_are_each_sent_once(cx: &mut TestAppContext) {
    let cx = &mut setup(cx);
    let pty = Arc::new(TestExternalPty::default());
    let terminal = cx.update(|window, cx| {
        spawn_external(
            ExternalTerminalBackend {
                pty: pty.clone(),
                path_style: PathStyle::Unix,
            },
            window,
            cx,
        )
    });
    let view = cx.new_window_entity(|window, cx| TerminalView::new(terminal, window, cx));

    view.update(cx, |view, cx| {
        view.set_marked_text(Some("helo".into()), Some(4..4), cx);
        view.defer_marked_commit(cx);
        view.replace_ime_text(Some(4..5), "lo ", cx);
        view.set_marked_text(Some("n".into()), Some(1..1), cx);
        assert_eq!(view.marked_text.as_deref(), Some("n"));
    });
    assert_eq!(&*pty.writes.lock().unwrap(), b"hello ");

    view.update(cx, |view, cx| view.defer_marked_commit(cx));
    cx.run_until_parked();
    assert_eq!(&*pty.writes.lock().unwrap(), b"hello n");
}

#[gpui::test]
fn horizontal_gesture_pans_and_clamps_the_mobile_canvas(cx: &mut TestAppContext) {
    let cx = &mut setup(cx);
    let terminal = cx.new(|cx| {
        TerminalBuilder::new_display_only(
            CursorShape::Bar,
            AlternateScroll::On,
            None,
            0,
            cx.background_executor(),
            PathStyle::Unix,
        )
        .subscribe(cx)
    });
    let view = cx.new_window_entity(|window, cx| TerminalView::new(terminal, window, cx));
    let pan = |delta| ScrollWheelEvent {
        position: Default::default(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(delta), px(0.))),
        modifiers: gpui::Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    };

    view.update(cx, |view, cx| {
        view.horizontal_layout(px(40.));
        view.scroll_wheel(&pan(-25.), cx);
        assert_eq!(view.horizontal_offset, px(25.));
        view.scroll_wheel(&pan(-25.), cx);
        assert_eq!(view.horizontal_offset, px(40.));
        view.scroll_wheel(&pan(100.), cx);
        assert_eq!(view.horizontal_offset, px(0.));
    });
}

#[gpui::test]
fn pinch_previews_live_and_commits_one_terminal_geometry(cx: &mut TestAppContext) {
    let cx = &mut setup(cx);
    let terminal = cx.new(|cx| {
        TerminalBuilder::new_display_only(
            CursorShape::Bar,
            AlternateScroll::On,
            None,
            0,
            cx.background_executor(),
            PathStyle::Unix,
        )
        .subscribe(cx)
    });
    let view = cx.new_window_entity(|window, cx| TerminalView::new(terminal, window, cx));
    cx.update(|window, cx| window.replace_root(cx, |_, _| RootView(view.clone())));
    cx.run_until_parked();
    let before = view.read_with(cx, |view, cx| {
        view.terminal.read(cx).last_content.terminal_bounds
    });

    view.update(cx, |view, cx| {
        view.pinch_zoom(gpui::TouchPhase::Started, 0., cx);
        view.pinch_zoom(gpui::TouchPhase::Moved, 0.25, cx);
        view.pinch_zoom(gpui::TouchPhase::Moved, 0.2, cx);
        assert_eq!(view.paint_font_scale(), 1.5, "preview follows the fingers");
        assert_eq!(
            view.terminal_font_scale(),
            1.,
            "PTY geometry stays fixed while pinching"
        );
    });
    cx.run_until_parked();
    let during = view.read_with(cx, |view, cx| {
        view.terminal.read(cx).last_content.terminal_bounds
    });
    assert_eq!(during, before, "preview must not reflow the PTY");

    view.update(cx, |view, cx| {
        view.pinch_zoom(gpui::TouchPhase::Ended, 0., cx);
        assert_eq!(view.paint_font_scale(), 1.5);
        assert_eq!(view.terminal_font_scale(), 1.5);
    });
    cx.run_until_parked();
    let after = view.read_with(cx, |view, cx| {
        view.terminal.read(cx).last_content.terminal_bounds
    });
    assert_ne!(after, before, "gesture end commits the new PTY geometry");
}

#[test]
fn ime_commit_applies_the_correction_delta_to_the_whole_preedit() {
    // Android reports Commit("helo") followed by Replace(4..5, "lo ").
    assert_eq!(apply_ime_replacement("helo", 4..5, "lo "), "hello ");
    assert_eq!(apply_ime_replacement("猫", 2..2, "!"), "猫!");
}

#[gpui::test]
async fn a_missing_folder_is_an_error_not_another_folder(cx: &mut TestAppContext) {
    let cx = &mut setup(cx);
    let missing = std::env::temp_dir().join("pi-terminal-missing-folder");
    let spawn = cx.update(|window, cx| spawn(missing.clone(), window, cx));
    let Err(error) = spawn.await else {
        panic!("a shell started in a missing folder");
    };
    assert!(error.to_string().contains("is not a folder"), "{error}");
}
