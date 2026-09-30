//! Terminals for pi-desktop: Zed's `terminal` crate (alacritty's emulator on a PTY)
//! with a view and element of our own, because Zed's `terminal_view` needs a
//! `Workspace`.
//!
//! The host sets [`TerminalStyle`] as a global and calls [`init`] once. [`spawn`]
//! starts the user's shell; [`TerminalView`] shows one terminal and forwards keys,
//! IME text, the pointer and the clipboard to it.

mod element;
mod scrollbar;

use anyhow::Result;
use element::TerminalElement;
use gpui::{
    Action, App, AppContext as _, Context, Entity, FocusHandle, Focusable, Global, Hsla,
    InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, Keystroke, ParentElement, Pixels,
    Render, ScrollWheelEvent, SharedString, Styled, Subscription, Task, Window, div,
};
use scrollbar::TerminalScrollHandle;
use settings::Settings;
use std::{path::PathBuf, time::Duration};
use terminal::{
    Clear, Copy, HoveredWord, MaybeNavigationTarget, Modes, MouseInputMode, Paste, ScrollLineDown,
    ScrollLineUp, ScrollPageDown, ScrollPageUp, ScrollToBottom, ScrollToTop, SelectAll,
    TerminalBuilder, TerminalMode,
    terminal_settings::{AlternateScroll, CursorShape, TerminalSettings},
};
pub use terminal::{Event, Terminal};
use ui::{ScrollAxes, Scrollbars, WithScrollbar as _};
use util::ResultExt as _;
pub use util::shell::Shell;

/// Fonts and the colors the terminal theme does not cover. Cell colors come from
/// the global Zed theme's `terminal_*` colors.
#[derive(Clone)]
pub struct TerminalStyle {
    pub font_family: SharedString,
    pub font_size: Pixels,
    pub line_height: Pixels,
    pub cursor: Hsla,
    pub selection: Hsla,
    pub link: Hsla,
}

impl Global for TerminalStyle {}

/// Sends a keystroke to the shell, for keys the app binds elsewhere.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = pi_terminal, no_json)]
pub struct SendKeystroke(pub &'static str);

/// Sends text to the shell as if typed.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = pi_terminal, no_json)]
pub struct SendText(pub &'static str);

/// Binds the terminal's keys. Keys the app binds in an enclosing context (Enter
/// submits, Escape stops a run, arrows pick choices, and on Linux and Windows
/// the `ctrl-` shortcuts) go to the shell instead while a terminal has focus.
pub fn init(cx: &mut App) {
    let context = Some("Terminal");
    let shell = |key: &'static str| KeyBinding::new(key, SendKeystroke(key), context);
    cx.bind_keys([
        shell("enter"),
        shell("alt-enter"),
        shell("escape"),
        shell("tab"),
        shell("up"),
        shell("down"),
        KeyBinding::new("shift-pageup", ScrollPageUp, context),
        KeyBinding::new("shift-pagedown", ScrollPageDown, context),
        KeyBinding::new("shift-up", ScrollLineUp, context),
        KeyBinding::new("shift-down", ScrollLineDown, context),
        KeyBinding::new("shift-home", ScrollToTop, context),
        KeyBinding::new("shift-end", ScrollToBottom, context),
    ]);
    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("cmd-c", Copy, context),
        KeyBinding::new("cmd-v", Paste, context),
        KeyBinding::new("cmd-a", SelectAll, context),
        KeyBinding::new("cmd-k", Clear, context),
        // Terminal.app's line editing shortcuts.
        KeyBinding::new("cmd-left", SendKeystroke("ctrl-a"), context),
        KeyBinding::new("cmd-right", SendKeystroke("ctrl-e"), context),
        KeyBinding::new("cmd-backspace", SendKeystroke("ctrl-u"), context),
        KeyBinding::new("alt-left", SendText("\x1bb"), context),
        KeyBinding::new("alt-right", SendText("\x1bf"), context),
    ]);
    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([
        KeyBinding::new("ctrl-shift-c", Copy, context),
        KeyBinding::new("ctrl-shift-v", Paste, context),
        KeyBinding::new("ctrl-shift-a", SelectAll, context),
        KeyBinding::new("ctrl-shift-l", Clear, context),
        shell("ctrl-b"),
        shell("ctrl-j"),
        shell("ctrl-k"),
        shell("ctrl-n"),
        shell("ctrl-o"),
        shell("ctrl-q"),
    ]);
}

/// Starts the user's login shell in `cwd`, with the app's environment plus
/// `TERM=xterm-256color`. The PTY is opened off the UI thread.
pub fn spawn(cwd: PathBuf, window: &Window, cx: &mut App) -> Task<Result<Entity<Terminal>>> {
    spawn_shell(cwd, Shell::System, window, cx)
}

/// [`spawn`] with a given shell instead of the user's.
pub fn spawn_shell(
    cwd: PathBuf,
    shell: Shell,
    window: &Window,
    cx: &mut App,
) -> Task<Result<Entity<Terminal>>> {
    // The PTY would silently start in the app's own folder instead.
    if !cwd.is_dir() {
        return Task::ready(Err(anyhow::anyhow!("{} is not a folder", cwd.display())));
    }
    let settings = TerminalSettings::get_global(cx).clone();
    let builder = TerminalBuilder::new(
        Some(cwd),
        TerminalMode::interactive(),
        shell,
        Default::default(),
        CursorShape::Bar,
        AlternateScroll::On,
        settings.max_scroll_history_lines,
        settings.path_hyperlink_regexes,
        Duration::from_millis(settings.path_hyperlink_timeout_ms),
        false,
        window.window_handle().window_id().as_u64(),
        cx,
        Vec::new(),
        util::paths::PathStyle::local(),
    );
    cx.spawn(async move |cx| {
        let builder = builder.await?;
        Ok(cx.new(|cx| builder.subscribe(cx)))
    })
}

/// A short name for a terminal's tab: the shell at the prompt (`zsh`), or the
/// program running in it (`npm`).
pub fn label(terminal: &Terminal) -> String {
    if is_busy(terminal)
        && let Some(command) = terminal.foreground_process_command_name()
    {
        return command;
    }
    // Zed's command names skip login shells (`-zsh`) and paths (`/bin/zsh`).
    std::env::var_os("SHELL")
        .and_then(|shell| {
            let name = PathBuf::from(shell).file_name()?.to_str()?.to_owned();
            Some(name)
        })
        .unwrap_or_else(|| "shell".into())
}

/// Whether a program other than the shell has the terminal (a command runs).
pub fn is_busy(terminal: &Terminal) -> bool {
    let shell = terminal.pid_getter().map(|pid| pid.fallback_pid());
    terminal
        .pid()
        .is_some_and(|foreground| Some(foreground) != shell)
}

pub struct TerminalView {
    pub(crate) terminal: Entity<Terminal>,
    focus_handle: FocusHandle,
    /// A URL under the pointer, underlined while the platform modifier is held.
    pub(crate) hovered_link: Option<HoveredWord>,
    /// IME pre-edit text, drawn over the cursor until committed.
    pub(crate) marked_text: Option<String>,
    /// Text to type once the shell shows its prompt.
    pending_input: Option<String>,
    scroll: TerminalScrollHandle,
    _wait: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl TerminalView {
    pub fn new(terminal: Entity<Terminal>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let subscriptions = vec![
            cx.observe(&terminal, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &terminal,
                window,
                |this, terminal, event, window, cx| match event {
                    Event::Wakeup => {
                        window.invalidate_character_coordinates();
                        this.type_pending_input(false, cx);
                        cx.notify();
                    }
                    Event::NewNavigationTarget(target) => {
                        this.hovered_link = match target {
                            Some(MaybeNavigationTarget::Url(_)) => {
                                terminal.read(cx).last_content.last_hovered_word.clone()
                            }
                            _ => None,
                        };
                        cx.notify();
                    }
                    Event::Open(MaybeNavigationTarget::Url(url)) => cx.open_url(url),
                    Event::SelectionsChanged => window.invalidate_character_coordinates(),
                    _ => {}
                },
            ),
            cx.on_focus_in(&focus_handle, window, |this, window, cx| {
                this.terminal.update(cx, |terminal, _| terminal.focus_in());
                window.invalidate_character_coordinates();
                cx.notify();
            }),
            cx.on_focus_out(&focus_handle, window, |this, _, _, cx| {
                this.terminal.update(cx, |terminal, _| terminal.focus_out());
                cx.notify();
            }),
        ];
        let scroll = TerminalScrollHandle::new(terminal.read(cx));
        Self {
            terminal,
            focus_handle,
            hovered_link: None,
            marked_text: None,
            pending_input: None,
            scroll,
            _wait: Task::ready(()),
            _subscriptions: subscriptions,
        }
    }

    pub fn terminal(&self) -> &Entity<Terminal> {
        &self.terminal
    }

    /// Types `text` at the shell's prompt without running it, once the shell is
    /// ready. It is pasted with bracketed paste, so a multi-line command waits
    /// for Enter too. A shell without bracketed paste gets single-line text
    /// after a short wait; multi-line text is not typed there, because each
    /// newline would run a line.
    pub fn type_at_prompt(&mut self, text: String, cx: &mut Context<Self>) {
        self.pending_input = Some(text);
        self.type_pending_input(false, cx);
        self._wait = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            this.update(cx, |this, cx| this.type_pending_input(true, cx))
                .log_err();
        });
    }

    fn type_pending_input(&mut self, timed_out: bool, cx: &mut Context<Self>) {
        if self.pending_input.is_none() {
            return;
        }
        let bracketed = self
            .terminal
            .read(cx)
            .last_content
            .mode
            .contains(Modes::BRACKETED_PASTE);
        if !bracketed && !timed_out {
            return;
        }
        let text = self.pending_input.take().unwrap_or_default();
        if bracketed || !text.contains('\n') {
            self.terminal
                .update(cx, |terminal, _| terminal.paste(&text));
        }
    }

    pub(crate) fn mouse_input_mode(&self) -> MouseInputMode {
        MouseInputMode::ReportToTerminal
    }

    pub(crate) fn set_marked_text(&mut self, text: Option<String>, cx: &mut Context<Self>) {
        let text = text.filter(|text| !text.is_empty());
        if self.marked_text != text {
            self.marked_text = text;
            cx.notify();
        }
    }

    pub(crate) fn commit_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if !text.is_empty() {
            self.terminal.update(cx, |terminal, _| {
                terminal.input(text.to_owned().into_bytes());
            });
        }
    }

    pub(crate) fn scroll_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let multiplier = TerminalSettings::get_global(cx).scroll_multiplier.max(0.01);
        let mode = self.mouse_input_mode();
        self.terminal.update(cx, |terminal, _| {
            terminal.scroll_wheel(event, multiplier, mode)
        });
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        // Text goes through the input handler, so IME composition works.
        if event.prefer_character_input
            && event.keystroke.key_char.is_some()
            && !self.terminal.read(cx).vi_mode_enabled()
        {
            return;
        }
        if self.send_keystroke(&event.keystroke, cx) {
            cx.stop_propagation();
        }
    }

    fn send_keystroke(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        let option_as_meta = TerminalSettings::get_global(cx).option_as_meta;
        let (handled, vi_mode) = self.terminal.update(cx, |terminal, _| {
            (
                terminal.try_keystroke(keystroke, option_as_meta),
                terminal.vi_mode_enabled(),
            )
        });
        // Vi motions move the cursor without output that would redraw.
        if handled && vi_mode {
            cx.notify();
        }
        handled
    }

    fn is_alt_screen(&self, cx: &App) -> bool {
        self.terminal
            .read(cx)
            .last_content
            .mode
            .contains(Modes::ALT_SCREEN)
    }

    /// Scrolls the scrollback, unless a full-screen program (an editor, a
    /// pager) owns the screen and should get the key.
    fn scroll(&mut self, cx: &mut Context<Self>, scroll: impl FnOnce(&mut Terminal)) {
        if self.is_alt_screen(cx) {
            cx.propagate();
            return;
        }
        self.terminal.update(cx, |terminal, _| scroll(terminal));
        cx.notify();
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.scroll.update(self.terminal.read(cx));
        if let Some(offset) = self.scroll.future_display_offset.take() {
            self.terminal.update(cx, |terminal, _| {
                let current = terminal.last_content.display_offset;
                if offset > current {
                    terminal.scroll_up_by(offset - current);
                } else {
                    terminal.scroll_down_by(current - offset);
                }
            });
        }
        let focused = self.focus_handle.is_focused(window);
        div()
            .id("terminal-view")
            .size_full()
            .relative()
            .key_context("Terminal")
            .on_action(cx.listener(|this, action: &SendKeystroke, _, cx| {
                if let Some(keystroke) = Keystroke::parse(action.0).log_err() {
                    this.send_keystroke(&keystroke, cx);
                }
            }))
            .on_action(cx.listener(|this, action: &SendText, _, cx| {
                this.terminal
                    .update(cx, |terminal, _| terminal.input(action.0.as_bytes()));
            }))
            .on_action(cx.listener(|this, _: &Copy, _, cx| {
                this.terminal.update(cx, |terminal, _| terminal.copy(None));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Paste, _, cx| {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    this.terminal
                        .update(cx, |terminal, _| terminal.paste(&text));
                }
            }))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                this.terminal
                    .update(cx, |terminal, _| terminal.select_all());
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Clear, _, cx| {
                this.terminal.update(cx, |terminal, _| terminal.clear());
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ScrollLineUp, _, cx| {
                this.scroll(cx, Terminal::scroll_line_up)
            }))
            .on_action(cx.listener(|this, _: &ScrollLineDown, _, cx| {
                this.scroll(cx, Terminal::scroll_line_down)
            }))
            .on_action(cx.listener(|this, _: &ScrollPageUp, _, cx| {
                this.scroll(cx, Terminal::scroll_page_up)
            }))
            .on_action(cx.listener(|this, _: &ScrollPageDown, _, cx| {
                this.scroll(cx, Terminal::scroll_page_down)
            }))
            .on_action(
                cx.listener(|this, _: &ScrollToTop, _, cx| {
                    this.scroll(cx, Terminal::scroll_to_top)
                }),
            )
            .on_action(cx.listener(|this, _: &ScrollToBottom, _, cx| {
                this.scroll(cx, Terminal::scroll_to_bottom)
            }))
            .on_key_down(cx.listener(Self::key_down))
            .child(
                div()
                    .id("terminal-content")
                    .size_full()
                    .child(TerminalElement::new(
                        self.terminal.clone(),
                        cx.entity(),
                        self.focus_handle.clone(),
                        focused,
                        true,
                    ))
                    .custom_scrollbars(
                        Scrollbars::new(ScrollAxes::Vertical)
                            .id("terminal-scrollbar")
                            .tracked_scroll_handle(&self.scroll),
                        window,
                        cx,
                    ),
            )
    }
}

#[cfg(test)]
mod tests;
