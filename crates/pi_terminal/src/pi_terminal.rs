//! Terminals for pi-desktop: Zed's `terminal` crate (alacritty's emulator on a PTY)
//! with a view and element of our own, because Zed's `terminal_view` needs a
//! `Workspace`.
//!
//! The host sets [`TerminalStyle`] as a global and calls [`init`] once. [`spawn`]
//! starts the user's shell; [`TerminalView`] shows one terminal and forwards keys,
//! IME text, the pointer and the clipboard to it.

mod element;
#[cfg(not(target_os = "android"))]
mod scrollbar;
#[cfg(target_os = "android")]
#[path = "scrollbar_android.rs"]
mod scrollbar;

use anyhow::Result;
use element::TerminalElement;
use gpui::{
    Action, App, AppContext as _, Context, Entity, FocusHandle, Focusable, FontFallbacks, Global,
    Hsla, InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, Keystroke, ParentElement,
    Pixels, Render, ScrollWheelEvent, SharedString, Styled, Subscription, Task, Window, div, px,
};
use scrollbar::TerminalScrollHandle;
#[cfg(not(target_os = "android"))]
use settings::Settings;
#[cfg(feature = "mobile")]
use std::sync::Arc;
use std::{ops::Range, path::PathBuf, time::Duration};
#[cfg(feature = "mobile")]
pub use terminal::ExternalPty;
use terminal::{
    Clear, Copy, HoveredWord, MaybeNavigationTarget, Modes, MouseInputMode, Paste, ScrollLineDown,
    ScrollLineUp, ScrollPageDown, ScrollPageUp, ScrollToBottom, ScrollToTop, SelectAll,
    TerminalBuilder, TerminalMode,
    terminal_settings::{AlternateScroll, CursorShape, TerminalSettings},
};
pub use terminal::{Event, Terminal, TerminalBounds};
#[cfg(feature = "mobile")]
use terminal_shim as terminal;
#[cfg(not(target_os = "android"))]
use ui::{ScrollAxes, Scrollbars, WithScrollbar as _};
use util::ResultExt as _;
pub use util::{paths::PathStyle, shell::Shell};
#[cfg(not(feature = "mobile"))]
use zed_terminal as terminal;

/// A non-editable leading character gives Android IMEs a document position to
/// delete from, so a backspace at an empty terminal becomes a real key event.
pub(crate) const INPUT_SENTINEL: &str = "\u{200b}";

fn apply_ime_replacement(composing: &str, range: Range<usize>, replacement: &str) -> String {
    let mut document: Vec<u16> = format!("{INPUT_SENTINEL}{composing}")
        .encode_utf16()
        .collect();
    let start = range.start.min(document.len());
    let end = range.end.max(start).min(document.len());
    document.splice(start..end, replacement.encode_utf16());
    String::from_utf16_lossy(&document)
        .strip_prefix(INPUT_SENTINEL)
        .unwrap_or_default()
        .to_owned()
}

/// Fonts and the colors the terminal theme does not cover. Cell colors come from
/// the global Zed theme's `terminal_*` colors.
#[derive(Clone)]
pub struct TerminalStyle {
    pub font_family: SharedString,
    pub font_fallbacks: Option<FontFallbacks>,
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
    #[cfg(target_os = "android")]
    if !cx.has_global::<TerminalSettings>() {
        cx.set_global(TerminalSettings::default());
    }
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

/// The source of a terminal's byte stream.
pub enum TerminalBackend {
    Local {
        cwd: PathBuf,
        shell: Shell,
    },
    #[cfg(feature = "mobile")]
    External(ExternalTerminalBackend),
}

/// A PTY byte stream owned by another transport (SSH today, HTTP/WebSocket in
/// the future). The terminal emulator never sees that transport.
#[cfg(feature = "mobile")]
pub struct ExternalTerminalBackend {
    pub pty: Arc<dyn ExternalPty>,
    pub path_style: PathStyle,
}

/// Starts the user's login shell in `cwd`, with the app's environment plus
/// `TERM=xterm-256color`. The PTY is opened off the UI thread.
pub fn spawn(cwd: PathBuf, window: &Window, cx: &mut App) -> Task<Result<Entity<Terminal>>> {
    spawn_shell(cwd, Shell::System, window, cx)
}

/// Creates a terminal synchronously around an externally owned PTY stream.
#[cfg(feature = "mobile")]
pub fn spawn_external(
    backend: ExternalTerminalBackend,
    window: &Window,
    cx: &mut App,
) -> Entity<Terminal> {
    let settings = TerminalSettings::get_global(cx).clone();
    let builder = TerminalBuilder::new_external(
        backend.pty,
        None,
        CursorShape::Bar,
        AlternateScroll::On,
        settings.max_scroll_history_lines,
        settings.path_hyperlink_regexes,
        Duration::from_millis(settings.path_hyperlink_timeout_ms),
        window.window_handle().window_id().as_u64(),
        cx.background_executor(),
        backend.path_style,
    );
    cx.new(|cx| builder.subscribe(cx))
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
    pub(crate) marked_selection: Option<Range<usize>>,
    pending_ime_commit: Option<String>,
    ime_commit_generation: u64,
    /// Text to type once the shell shows its prompt.
    pending_input: Option<String>,
    font_scale: f32,
    pending_font_scale: Option<f32>,
    horizontal_offset: Pixels,
    horizontal_max: Pixels,
    scroll: TerminalScrollHandle,
    _wait: Task<()>,
    _ime_commit: Task<()>,
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
            marked_selection: None,
            pending_ime_commit: None,
            ime_commit_generation: 0,
            pending_input: None,
            font_scale: 1.,
            pending_font_scale: None,
            horizontal_offset: px(0.),
            horizontal_max: px(0.),
            scroll,
            _wait: Task::ready(()),
            _ime_commit: Task::ready(()),
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

    pub(crate) fn set_marked_text(
        &mut self,
        text: Option<String>,
        selection: Option<Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        let text = text.filter(|text| !text.is_empty());
        if text.is_some()
            && let Some(commit) = self.pending_ime_commit.take()
        {
            self.ime_commit_generation = self.ime_commit_generation.wrapping_add(1);
            self.commit_text(&commit, cx);
        }
        let length = text
            .as_ref()
            .map(|text| text.encode_utf16().count())
            .unwrap_or(0);
        let selection = text.as_ref().map(|_| {
            let selection = selection.unwrap_or(length..length);
            selection.start.min(length)..selection.end.min(length)
        });
        if self.marked_text != text || self.marked_selection != selection {
            self.marked_text = text;
            self.marked_selection = selection;
            cx.notify();
        }
    }

    pub(crate) fn set_marked_selection(&mut self, selection: Range<usize>, cx: &mut Context<Self>) {
        let Some(text) = &self.marked_text else {
            return;
        };
        let length = text.encode_utf16().count();
        let selection = selection.start.min(length)..selection.end.min(length);
        if self.marked_selection.as_ref() != Some(&selection) {
            self.marked_selection = Some(selection);
            cx.notify();
        }
    }

    /// Defers a plain composition commit by one executor turn. Android can
    /// immediately follow `unmark` with a delta replacement for autocorrection;
    /// waiting lets us send the corrected whole candidate rather than both.
    pub(crate) fn defer_marked_commit(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.marked_text.take() else {
            return;
        };
        self.marked_selection = None;
        if let Some(previous) = self.pending_ime_commit.replace(text) {
            self.commit_text(&previous, cx);
        }
        self.ime_commit_generation = self.ime_commit_generation.wrapping_add(1);
        let generation = self.ime_commit_generation;
        self._ime_commit = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::ZERO).await;
            this.update(cx, |this, cx| {
                if this.ime_commit_generation == generation
                    && let Some(text) = this.pending_ime_commit.take()
                {
                    this.commit_text(&text, cx);
                    cx.notify();
                }
            })
            .log_err();
        });
        cx.notify();
    }

    /// Applies an IME's UTF-16 delta to its old composition, then commits the
    /// resulting whole candidate to the append-only terminal stream.
    pub(crate) fn replace_ime_text(
        &mut self,
        replacement_range: Option<Range<usize>>,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let deletion_without_composition = text.is_empty()
            && replacement_range
                .as_ref()
                .is_some_and(|range| range.start < range.end)
            && self.marked_text.is_none()
            && self.pending_ime_commit.is_none();
        if deletion_without_composition {
            // Android soft keyboards express Backspace by deleting either our
            // sentinel or a just-committed character still present in Java's
            // asynchronous mirror. Neither exists in the append-only terminal
            // model, so route one deletion through the emulator's Backspace.
            self.send_key("backspace", cx);
            return;
        }
        self.ime_commit_generation = self.ime_commit_generation.wrapping_add(1);
        let composing = self
            .pending_ime_commit
            .take()
            .or_else(|| self.marked_text.take());
        self.marked_selection = None;
        let committed = match (composing, replacement_range) {
            (Some(composing), Some(range)) => apply_ime_replacement(&composing, range, text),
            _ => text.to_owned(),
        };
        self.commit_text(&committed, cx);
        cx.notify();
    }

    fn cancel_ime_composition(&mut self, cx: &mut Context<Self>) {
        self.ime_commit_generation = self.ime_commit_generation.wrapping_add(1);
        self.pending_ime_commit = None;
        self.marked_text = None;
        self.marked_selection = None;
        cx.notify();
    }

    pub(crate) fn commit_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if !text.is_empty() {
            self.terminal.update(cx, |terminal, _| {
                terminal.input(text.to_owned().into_bytes());
            });
        }
    }

    /// Scale used to paint the current frame. During a pinch this follows the
    /// fingers immediately, while the PTY keeps its last committed geometry.
    pub(crate) fn paint_font_scale(&self) -> f32 {
        self.pending_font_scale.unwrap_or(self.font_scale)
    }

    /// Scale whose cell grid has been sent to the PTY.
    pub(crate) fn terminal_font_scale(&self) -> f32 {
        self.font_scale
    }

    fn pinch_zoom(&mut self, phase: gpui::TouchPhase, delta: f32, cx: &mut Context<Self>) {
        match phase {
            gpui::TouchPhase::Started => self.pending_font_scale = Some(self.font_scale),
            gpui::TouchPhase::Moved => {
                let scale = self.pending_font_scale.unwrap_or(self.font_scale);
                let next = (scale * (1. + delta).max(0.1)).clamp(0.75, 2.5);
                if self.pending_font_scale != Some(next) {
                    self.pending_font_scale = Some(next);
                    // Repaint at the preview scale without resizing the PTY.
                    // This keeps the gesture attached to the fingers but avoids
                    // asking a full-screen program to reflow every touch sample.
                    cx.notify();
                }
            }
            gpui::TouchPhase::Ended => {
                let next = self.pending_font_scale.take().unwrap_or(self.font_scale);
                if (next - self.font_scale).abs() > f32::EPSILON {
                    // Commit one PTY resize after the gesture.
                    self.font_scale = next;
                }
                cx.notify();
            }
            gpui::TouchPhase::Cancelled => {
                if self.pending_font_scale.take().is_some() {
                    cx.notify();
                }
            }
        }
    }

    pub(crate) fn horizontal_layout(&mut self, maximum: Pixels) -> Pixels {
        let maximum = f32::from(maximum).max(0.);
        self.horizontal_max = px(maximum);
        self.horizontal_offset = px(f32::from(self.horizontal_offset).clamp(0., maximum));
        self.horizontal_offset
    }

    pub(crate) fn scroll_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let line_height = cx.global::<TerminalStyle>().line_height;
        let delta = event.delta.pixel_delta(line_height);
        if f32::from(self.horizontal_max) > 0.
            && f32::from(delta.x).abs() > f32::from(delta.y).abs()
        {
            let next = (f32::from(self.horizontal_offset) - f32::from(delta.x))
                .clamp(0., f32::from(self.horizontal_max));
            if self.horizontal_offset != px(next) {
                self.horizontal_offset = px(next);
                cx.notify();
            }
            return;
        }

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

    /// Sends a key from a touch accessory. An active IME preedit is cancelled,
    /// just as terminal emulators cancel it before an interrupt/navigation key.
    pub fn send_key(&mut self, key: &'static str, cx: &mut Context<Self>) {
        self.cancel_ime_composition(cx);
        if let Some(keystroke) = Keystroke::parse(key).log_err() {
            self.send_keystroke(&keystroke, cx);
        }
    }

    pub fn send_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.cancel_ime_composition(cx);
        self.commit_text(text, cx);
    }

    pub fn copy_selection(&mut self, cx: &mut Context<Self>) {
        self.terminal.update(cx, |terminal, _| terminal.copy(None));
        cx.notify();
    }

    pub fn paste_clipboard(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.terminal
                .update(cx, |terminal, _| terminal.paste(&text));
        }
    }

    /// Stops an external PTY deterministically before this view is discarded.
    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.cancel_ime_composition(cx);
        self.terminal
            .update(cx, |terminal, _| terminal.release_pty_resources());
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
        let content = div()
            .id("terminal-content")
            .size_full()
            .child(TerminalElement::new(
                self.terminal.clone(),
                cx.entity(),
                self.focus_handle.clone(),
                focused,
                true,
            ));
        #[cfg(not(target_os = "android"))]
        let content = content.custom_scrollbars(
            Scrollbars::new(ScrollAxes::Vertical)
                .id("terminal-scrollbar")
                .tracked_scroll_handle(&self.scroll),
            window,
            cx,
        );
        div()
            .id("terminal-view")
            .size_full()
            .relative()
            .key_context("Terminal")
            .on_pinch(cx.listener(|this, event: &gpui::PinchEvent, _, cx| {
                this.pinch_zoom(event.phase, event.delta, cx);
                cx.stop_propagation();
            }))
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
            .child(content)
    }
}

#[cfg(test)]
mod tests;
