//! The terminal drawer: shells in the session's folder, under the thread.
//!
//! Terminals belong to one session and keep running while the drawer is hidden
//! or another session is selected. They are not part of pi's context.

use super::*;
use gpui::{DragMoveEvent, Empty, EventEmitter, Subscription};
use pi_terminal::{Event, TerminalView};

pub const DEFAULT_HEIGHT: Pixels = px(240.);
const MIN_HEIGHT: Pixels = px(120.);
/// Room kept above the drawer for the transcript and composer.
const MIN_ABOVE: Pixels = px(160.);

pub enum DrawerEvent {
    /// Terminals were added or closed; the status bar counts them.
    Count,
    /// The drawer was hidden while a terminal had focus.
    Hidden,
}

/// Drag payload for the drawer's top edge.
pub struct DrawerResize;
impl Render for DrawerResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

struct Tab {
    id: usize,
    view: Entity<TerminalView>,
    _subscription: Subscription,
}

pub struct TerminalDrawer {
    cwd: PathBuf,
    shell: pi_terminal::Shell,
    available: bool,
    tabs: Vec<Tab>,
    active: usize,
    next_id: usize,
    open: bool,
    maximized: bool,
    height: Pixels,
    /// Shells being started.
    starting: usize,
    error: Option<String>,
}

impl EventEmitter<DrawerEvent> for TerminalDrawer {}

impl TerminalDrawer {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            shell: pi_terminal::Shell::System,
            available: true,
            tabs: Vec::new(),
            active: 0,
            next_id: 0,
            open: false,
            maximized: false,
            height: DEFAULT_HEIGHT,
            starting: 0,
            error: None,
        }
    }

    pub fn unavailable(mut self) -> Self {
        self.available = false;
        self
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_maximized(&self) -> bool {
        self.open && self.maximized
    }

    pub fn count(&self) -> usize {
        self.tabs.len()
    }

    /// Each shell's tab name and its last non-empty lines, oldest first.
    pub fn outputs(&self, lines: usize, cx: &App) -> Vec<(String, Vec<String>)> {
        self.tabs
            .iter()
            .map(|tab| {
                let terminal = tab.view.read(cx).terminal().read(cx);
                (
                    pi_terminal::label(terminal),
                    terminal.last_n_non_empty_lines(lines),
                )
            })
            .collect()
    }

    #[cfg(test)]
    pub fn active_view(&self) -> Option<Entity<TerminalView>> {
        self.tabs.get(self.active).map(|tab| tab.view.clone())
    }

    #[cfg(test)]
    pub fn tab_id(&self, index: usize) -> usize {
        self.tabs[index].id
    }

    /// Tests use `/bin/sh` rather than the developer's shell and its startup files.
    #[cfg(test)]
    pub fn use_shell(&mut self, shell: pi_terminal::Shell) {
        self.shell = shell;
    }

    /// ⌃`: shows the drawer, focuses it when shown elsewhere, hides it when focused.
    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            self.open = true;
            if self.tabs.is_empty() {
                self.new_terminal(None, window, cx);
            } else {
                self.focus_active(window, cx);
            }
            cx.notify();
        } else if self.has_focus(window, cx) {
            self.hide(window, cx);
        } else {
            self.focus_active(window, cx);
        }
    }

    /// The session header's terminal button: shows or hides, whatever has focus.
    pub fn show_or_hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.hide(window, cx);
        } else {
            self.toggle(window, cx);
        }
    }

    pub fn hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let had_focus = self.has_focus(window, cx);
        self.open = false;
        self.maximized = false;
        if had_focus {
            cx.emit(DrawerEvent::Hidden);
        }
        cx.notify();
    }

    /// Starts a shell in a new tab. `command` is typed at its first prompt
    /// without running, so the user reads it and presses Enter.
    pub fn new_terminal(
        &mut self,
        command: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.available {
            self.open = true;
            self.error = Some(
                "Remote terminals are not supported yet. Agent bash tools run on the remote host."
                    .into(),
            );
            cx.notify();
            return;
        }
        self.open = true;
        self.starting += 1;
        self.error = None;
        // `terminal.shell`, unless a test chose one.
        let shell = match (&self.shell, crate::prefs::text(cx, "terminal.shell")) {
            (pi_terminal::Shell::System, Some(program)) => pi_terminal::Shell::Program(program),
            (shell, _) => shell.clone(),
        };
        let spawn = pi_terminal::spawn_shell(self.cwd.clone(), shell, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = spawn.await;
            this.update_in(cx, |this, window, cx| {
                this.starting -= 1;
                match result {
                    Ok(terminal) => this.add(terminal, command, window, cx),
                    Err(error) => {
                        this.error = Some(format!("Could not start a shell: {error:#}"));
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn add(
        &mut self,
        terminal: Entity<pi_terminal::Terminal>,
        command: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_id;
        self.next_id += 1;
        let view = cx.new(|cx| TerminalView::new(terminal.clone(), window, cx));
        if let Some(command) = command {
            view.update(cx, |view, cx| view.type_at_prompt(command, cx));
        }
        let subscription =
            cx.subscribe_in(
                &terminal,
                window,
                move |this, _, event, window, cx| match event {
                    // The shell exited (`exit`, ⌃D).
                    Event::CloseTerminal => this.close(id, window, cx),
                    // The foreground program changed: the tab's name and busy dot.
                    Event::TitleChanged => cx.notify(),
                    _ => {}
                },
            );
        self.tabs.push(Tab {
            id,
            view,
            _subscription: subscription,
        });
        self.active = self.tabs.len() - 1;
        self.open = true;
        self.focus_active(window, cx);
        cx.emit(DrawerEvent::Count);
        cx.notify();
    }

    /// Closes a tab and ends its shell.
    pub fn close(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        let had_focus = self.has_focus(window, cx);
        self.tabs.remove(index);
        if index < self.active || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        cx.emit(DrawerEvent::Count);
        if self.tabs.is_empty() {
            self.open = false;
            self.maximized = false;
            if had_focus {
                cx.emit(DrawerEvent::Hidden);
            }
        } else if had_focus {
            self.focus_active(window, cx);
        }
        cx.notify();
    }

    fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = index.min(self.tabs.len().saturating_sub(1));
        self.focus_active(window, cx);
        cx.notify();
    }

    fn focus_active(&self, window: &mut Window, cx: &mut App) {
        if let Some(tab) = self.tabs.get(self.active) {
            tab.view.focus_handle(cx).focus(window, cx);
        }
    }

    fn has_focus(&self, window: &Window, cx: &App) -> bool {
        self.tabs
            .iter()
            .any(|tab| tab.view.focus_handle(cx).contains_focused(window, cx))
    }

    /// Follows a drag of the top edge. `column` is the session column the
    /// drawer sits at the bottom of.
    pub fn drag(&mut self, event: &DragMoveEvent<DrawerResize>, cx: &mut Context<Self>) {
        let column = event.bounds;
        let max = (column.size.height - MIN_ABOVE).max(MIN_HEIGHT);
        let height = (column.bottom() - event.event.position.y).clamp(MIN_HEIGHT, max);
        if height != self.height {
            self.height = height;
            cx.notify();
        }
    }

    fn tab(&self, index: usize, tab: &Tab, cx: &mut Context<Self>, theme: Theme) -> AnyElement {
        let active = index == self.active;
        let terminal = tab.view.read(cx).terminal().read(cx);
        let name = pi_terminal::label(terminal);
        let busy = pi_terminal::is_busy(terminal);
        let id = tab.id;
        let group = SharedString::from(format!("terminal-tab-{id}"));
        h_flex()
            .id(("terminal-tab", id))
            .group(group.clone())
            .debug_selector(move || format!("terminal-tab-{index}"))
            .role(gpui::Role::Tab)
            .aria_label(name.clone())
            .h(px(20.))
            .pl(px(10.))
            .pr(px(4.))
            .gap(px(4.))
            .rounded(px(4.))
            .font_family(MONO)
            .text_size(px(11.))
            .cursor_pointer()
            .when(active, |tab| tab.bg(theme.selected).text_color(theme.text))
            .when(!active, |tab| {
                tab.text_color(theme.muted)
                    .hover(move |tab| tab.bg(theme.hover))
            })
            .child(name)
            .when(busy, |tab| {
                tab.child(div().size(px(6.)).rounded_full().bg(theme.accent))
            })
            .child(
                div()
                    .id(("terminal-tab-close", id))
                    .role(gpui::Role::Button)
                    .aria_label("Close terminal")
                    .invisible()
                    .group_hover(group, |close| close.visible())
                    .rounded(px(3.))
                    .hover(move |close| close.bg(theme.hover))
                    .child(icon("close", theme.muted).size(px(12.)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.close(id, window, cx);
                    })),
            )
            .on_click(cx.listener(move |this, _, window, cx| this.select(index, window, cx)))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _, window, cx| this.close(id, window, cx)),
            )
            .into_any_element()
    }
}

impl Render for TerminalDrawer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let tabs = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| self.tab(index, tab, cx, theme))
            .collect::<Vec<_>>();
        let body = match (self.tabs.get(self.active), &self.error) {
            (_, Some(error)) => div()
                .p(px(12.))
                .text_size(px(12.))
                .text_color(theme.coral)
                .child(error.clone())
                .into_any_element(),
            (Some(tab), None) => tab.view.clone().into_any_element(),
            (None, None) => div()
                .p(px(12.))
                .font_family(MONO)
                .text_size(px(11.5))
                .text_color(theme.faint)
                .child("Starting shell…")
                .into_any_element(),
        };
        v_flex()
            .id("terminal-drawer")
            .debug_selector(|| "terminal-drawer".into())
            .relative()
            .w_full()
            .when(self.maximized, |drawer| drawer.flex_1().min_h_0())
            .when(!self.maximized, |drawer| {
                drawer.h(self.height).flex_shrink_0()
            })
            .border_t_1()
            .border_color(theme.line_strong)
            .child(
                h_flex()
                    .h(px(30.))
                    .flex_shrink_0()
                    .pl(px(20.))
                    .pr(px(12.))
                    .gap(px(6.))
                    .bg(theme.panel)
                    .child(label("TERMINAL", theme).text_color(theme.muted).mr(px(18.)))
                    .children(tabs)
                    .child(
                        icon_button("terminal-new", "plus", "New terminal", theme)
                            .size(px(20.))
                            .debug_selector(|| "terminal-new".into())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.new_terminal(None, window, cx)
                            })),
                    )
                    .when(self.starting > 0 && !self.tabs.is_empty(), |header| {
                        header.child(spinner("terminal-starting", theme))
                    })
                    .child(div().flex_1())
                    .child(
                        icon_button(
                            "terminal-maximize",
                            if self.maximized {
                                "minimize"
                            } else {
                                "maximize"
                            },
                            if self.maximized {
                                "Restore terminal size"
                            } else {
                                "Maximize terminal"
                            },
                            theme,
                        )
                        .size(px(20.))
                        .debug_selector(|| "terminal-maximize".into())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.maximized = !this.maximized;
                            cx.notify();
                        })),
                    )
                    .child(
                        icon_button("terminal-hide", "close", "Hide terminal", theme)
                            .size(px(20.))
                            .debug_selector(|| "terminal-hide".into())
                            .on_click(cx.listener(|this, _, window, cx| this.hide(window, cx))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .bg(theme.deep)
                    .pl(px(20.))
                    .pr(px(4.))
                    .py(px(8.))
                    .child(body),
            )
            // The top edge resizes; double-click restores the default height.
            .when(!self.maximized, |drawer| {
                drawer.child(
                    div()
                        .id("terminal-resize")
                        .debug_selector(|| "terminal-resize".into())
                        .absolute()
                        .top(px(-3.))
                        .left_0()
                        .right_0()
                        .h(px(6.))
                        .cursor(CursorStyle::ResizeUpDown)
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .mt(px(1.))
                                .w(px(40.))
                                .h(px(4.))
                                .rounded(px(2.))
                                .bg(theme.line_strong),
                        )
                        .on_drag(DrawerResize, |_, _, _, cx| cx.new(|_| DrawerResize))
                        .on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                            if event.click_count() >= 2 {
                                this.height = DEFAULT_HEIGHT;
                                cx.notify();
                            }
                        })),
                )
            })
    }
}
