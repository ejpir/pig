use super::*;
use gpui::{TestAppContext, VisualContext as _, VisualTestContext, px, rgb};

fn setup(cx: &mut TestAppContext) -> VisualTestContext {
    cx.executor().allow_parking();
    cx.update(|cx| {
        settings::init(cx);
        theme_settings::init(zed_theme::LoadThemes::JustBase, cx);
        cx.set_global(TerminalStyle {
            font_family: "Courier".into(),
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
    let spawn =
        cx.update(|window, cx| spawn_shell(cwd, Shell::Program("/bin/sh".into()), window, cx));
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
    let view = open_shell(std::env::temp_dir(), cx).await;
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
