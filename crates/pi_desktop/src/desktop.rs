use crate::{
    components::*,
    input::TextInput,
    theme::{MONO, SANS, SERIF, Theme, theme},
};
use chrome::{HeaderView, ShellEvent, StatusBarView};
use gpui::{
    AnyElement, App, ClipboardItem, Context, CursorStyle, Entity, Focusable, FontWeight,
    KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent, PathPromptOptions, Pixels,
    ScrollHandle, SharedString, Window, actions, div, prelude::*, px, relative,
};
use pi_core::{
    protocol::{Command, SavedSession, SlashCommand},
    session::{Session, Tool, content_text, text},
};
use sidebar::SidebarView;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};
use ui::{ScrollAxes, Scrollbars, WithScrollbar as _};
use workspace::{WorkspaceController, WorkspaceEvent};

actions!(
    desktop,
    [
        Submit,
        FollowUp,
        Stop,
        NewSession,
        OpenFolder,
        ToggleTheme,
        ToggleInspector,
        ShowFiles,
        ShowDiagnostics,
        ToggleSidebar,
        ExpandMessageEditor,
        FocusSearch,
        PreviousChoice,
        NextChoice,
        Complete,
        ToggleTerminal,
        Quit
    ]
);

pub fn init(cx: &mut App) {
    prompts::init(cx);
    crate::input::init(cx);
    crate::markdown_view::init(cx);
    pi_terminal::init(cx);
    cx.bind_keys([
        KeyBinding::new("enter", Submit, Some("Desktop")),
        KeyBinding::new("alt-enter", FollowUp, Some("Desktop")),
        KeyBinding::new("escape", Stop, Some("Desktop")),
        KeyBinding::new("secondary-n", NewSession, Some("Desktop")),
        KeyBinding::new("secondary-o", OpenFolder, Some("Desktop")),
        KeyBinding::new("ctrl-shift-t", ToggleTheme, Some("Desktop")),
        KeyBinding::new("ctrl-shift-i", ToggleInspector, Some("Desktop")),
        KeyBinding::new("ctrl-shift-d", ShowDiagnostics, Some("Desktop")),
        KeyBinding::new("secondary-b", ToggleSidebar, Some("Desktop")),
        // Zed's shortcut for the agent panel's expanded message editor.
        KeyBinding::new("shift-alt-escape", ExpandMessageEditor, Some("Desktop")),
        KeyBinding::new("secondary-k", FocusSearch, Some("Desktop")),
        KeyBinding::new("up", PreviousChoice, Some("Desktop")),
        KeyBinding::new("down", NextChoice, Some("Desktop")),
        KeyBinding::new("tab", Complete, Some("Desktop")),
        KeyBinding::new("ctrl-`", ToggleTerminal, Some("Desktop")),
        // VS Code's panel shortcut; backtick is a dead key on many layouts.
        KeyBinding::new("secondary-j", ToggleTerminal, Some("Desktop")),
        KeyBinding::new("secondary-q", Quit, None),
    ]);
    new_session::init(cx);
    cx.on_action(|_: &Quit, cx| cx.quit());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pane {
    Sidebar,
    Inspector,
}
const SIDEBAR_WIDTH: Pixels = px(208.);
const INSPECTOR_WIDTH: Pixels = px(328.);
#[derive(Clone, Copy, PartialEq)]
struct Layout {
    show_sidebar: bool,
    inspector: bool,
    inspector_requested: bool,
    sidebar_width: Pixels,
    inspector_width: Pixels,
}

impl Layout {
    fn show_inspector(self, width: Pixels) -> bool {
        self.inspector && (self.inspector_requested || width >= px(1150.))
    }
}

pub struct Desktop {
    workspace: Entity<WorkspaceController>,
    sidebar: Entity<SidebarView>,
    header: Entity<HeaderView>,
    status: Entity<StatusBarView>,
    welcome: Entity<welcome::WelcomeView>,
    layout: Layout,
    resizing: Option<Pane>,
    focus_selected: bool,
    new_session_requested: bool,
    /// App views, made when first shown.
    sessions_view: Option<Entity<app_views::SessionsView>>,
    settings_view: Option<Entity<app_views::SettingsView>>,
    models_view: Option<app_views::CatalogScreen>,
    resources_view: Option<app_views::CatalogScreen>,
    /// `appearance.theme` as last applied, so other changes keep a Ctrl+Shift+T switch.
    theme_setting: String,
    appearance: gpui::WindowAppearance,
    _subscriptions: Vec<gpui::Subscription>,
}
impl Desktop {
    /// Opens `sessions`, a folder with an optional saved session each, and shows
    /// the one at `active`.
    pub fn new(
        sessions: Vec<(PathBuf, Option<SavedSession>)>,
        active: usize,
        demo: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let workspace = cx.new(|_| WorkspaceController::new(demo));
        workspace.update(cx, |workspace, cx| {
            let ids: Vec<_> = sessions
                .into_iter()
                .map(|(cwd, saved)| workspace.open(cwd, saved, cx))
                .collect();
            if let Some(id) = ids.get(active) {
                workspace.select(*id, cx);
            }
        });
        let sidebar = cx.new(|cx| SidebarView::new(workspace.clone(), cx));
        let search = sidebar.read(cx).search.clone();
        let layout = Layout {
            show_sidebar: true,
            inspector: true,
            inspector_requested: false,
            sidebar_width: SIDEBAR_WIDTH,
            inspector_width: INSPECTOR_WIDTH,
        };
        let header = cx.new(|cx| HeaderView::new(workspace.clone(), search, layout, cx));
        let status = cx.new(|cx| StatusBarView::new(workspace.clone(), cx));
        let welcome = cx.new(|cx| welcome::WelcomeView::new(workspace.clone(), cx));
        let subscriptions = vec![
            cx.subscribe(&workspace, |this, _, event, cx| {
                if matches!(event, WorkspaceEvent::NewSessionRequested) {
                    this.new_session_requested = true;
                    cx.notify();
                }
                if let WorkspaceEvent::Selection(id) = event
                    && *id == this.workspace.read(cx).active
                {
                    this.focus_selected = true;
                    cx.notify();
                }
                if matches!(event, WorkspaceEvent::View | WorkspaceEvent::Selection(_)) {
                    // The header's search filters the app view shown, else the sidebar.
                    let placeholder = match this.workspace.read(cx).view {
                        Some(app_views::AppView::Settings) => "Search settings",
                        Some(app_views::AppView::Models) => "Search models",
                        Some(app_views::AppView::Resources) => "Search resources",
                        _ => "Search sessions",
                    };
                    let search = this.sidebar.read(cx).search.clone();
                    search.update(cx, |search, cx| search.set_placeholder(placeholder, cx));
                    cx.notify();
                }
            }),
            cx.observe_global::<crate::prefs::Prefs>(|this, cx| this.apply_prefs(cx)),
            // GPUI also calls this when the window opens; only a change counts.
            cx.observe_window_appearance(window, |this, window, cx| {
                let appearance = window.appearance();
                if appearance != this.appearance {
                    this.appearance = appearance;
                    if this.theme_setting == "system" {
                        this.set_theme(crate::prefs::light(cx), cx);
                    }
                }
            }),
            cx.subscribe(&header, |this, _, event, cx| match event {
                ShellEvent::Sidebar => this.toggle_sidebar(cx),
                ShellEvent::Inspector => this.toggle_inspector(cx),
                ShellEvent::Theme => this.toggle_theme(cx),
                ShellEvent::ResourcesInstall => {
                    if let Some(screen) = &this.resources_view {
                        screen.view.update(cx, |view, cx| view.open_install(cx));
                    }
                }
                ShellEvent::ResourcesRefresh => {
                    if let Some(screen) = &this.resources_view {
                        screen
                            .view
                            .update(cx, |view, cx| view.refresh_resources(cx));
                    }
                }
            }),
        ];
        let this = Self {
            workspace,
            sidebar,
            header,
            status,
            welcome,
            layout,
            resizing: None,
            focus_selected: false,
            new_session_requested: false,
            sessions_view: None,
            settings_view: None,
            models_view: None,
            resources_view: None,
            theme_setting: crate::prefs::choice(cx, "appearance.theme", None),
            appearance: window.appearance(),
            _subscriptions: subscriptions,
        };
        this.apply_editor_prefs(cx);
        this.focus_composer(window, cx);
        this
    }
    /// Settings that apply at once: the theme and embedded editors.
    fn apply_prefs(&mut self, cx: &mut Context<Self>) {
        let theme = crate::prefs::choice(cx, "appearance.theme", None);
        if theme != self.theme_setting {
            self.theme_setting = theme;
            self.set_theme(crate::prefs::light(cx), cx);
        }
        self.apply_editor_prefs(cx);
    }
    fn apply_editor_prefs(&self, cx: &mut App) {
        let problem_card = crate::prefs::flag(cx, "editor.problemCard", None);
        pi_editor::set_preferences(pi_editor::Preferences { problem_card }, cx);
    }
    fn focus_composer(&self, window: &mut Window, cx: &mut App) {
        if let Some(tab) = self.workspace.read(cx).active_tab_opt() {
            let view = tab.view.clone();
            view.update(cx, |view, cx| view.focus(window, cx));
        } else {
            self.welcome.update(cx, |view, cx| view.focus(window, cx));
        }
    }
    fn sync_header(&self, cx: &mut Context<Self>) {
        self.header
            .update(cx, |header, cx| header.set_layout(self.layout, cx));
    }
    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.layout.show_sidebar = !self.layout.show_sidebar;
        self.resizing = None;
        self.sync_header(cx);
        cx.notify();
    }
    fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        self.layout.inspector = !self.layout.inspector;
        self.layout.inspector_requested = false;
        self.resizing = None;
        self.sync_header(cx);
        cx.notify();
    }
    /// Until the next launch or theme setting change; the setting is what lasts.
    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        self.set_theme(!theme(cx).light, cx);
    }
    fn set_theme(&mut self, light: bool, cx: &mut Context<Self>) {
        if theme(cx).light != light {
            cx.set_global(Theme::new(light));
            crate::markdown_view::sync_theme(cx);
            cx.refresh_windows();
        }
    }
    /// A drag handle on the pane's inner edge. Double-click restores the default width.
    fn resize_handle(&self, pane: Pane, cx: &Context<Self>, theme: Theme) -> impl IntoElement {
        let (id, default) = match pane {
            Pane::Sidebar => ("sidebar-resize", SIDEBAR_WIDTH),
            Pane::Inspector => ("inspector-resize", INSPECTOR_WIDTH),
        };
        div()
            .id(id)
            .debug_selector(move || id.into())
            .absolute()
            .top_0()
            .bottom_0()
            .when(pane == Pane::Sidebar, |handle| handle.right_0())
            .when(pane == Pane::Inspector, |handle| handle.left_0())
            .w(px(5.))
            .cursor(CursorStyle::ResizeLeftRight)
            .when(self.resizing == Some(pane), |handle| handle.bg(theme.focus))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    if event.click_count >= 2 {
                        match pane {
                            Pane::Sidebar => this.layout.sidebar_width = default,
                            Pane::Inspector => this.layout.inspector_width = default,
                        }
                    } else {
                        this.resizing = Some(pane);
                    }
                    cx.stop_propagation();
                    this.sync_header(cx);
                    cx.notify();
                }),
            )
    }

    fn resize(&mut self, x: Pixels, window: &Window, cx: &mut Context<Self>) {
        match self.resizing {
            Some(Pane::Sidebar) => self.layout.sidebar_width = x.max(px(160.)).min(px(400.)),
            Some(Pane::Inspector) => {
                self.layout.inspector_width = (window.viewport_size().width - x)
                    .max(px(260.))
                    .min(px(520.))
            }
            None => return,
        }
        self.sync_header(cx);
        cx.notify();
    }
}
impl Render for Desktop {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !window.has_active_prompt() && std::mem::take(&mut self.new_session_requested) {
            new_session::show(self.workspace.clone(), window, cx);
        }
        if !window.has_active_prompt() && std::mem::take(&mut self.focus_selected) {
            self.focus_composer(window, cx);
        }
        let theme = theme(cx);
        let session = self
            .workspace
            .read(cx)
            .active_tab_opt()
            .map(|tab| tab.view.clone());
        let inspector = session
            .as_ref()
            .map(|session| session.read(cx).inspector.clone());
        let show_inspector = self.layout.show_inspector(window.viewport_size().width);
        // An app view takes the middle column and the inspector.
        let search = self.sidebar.read(cx).search.clone();
        let app_view = match self.workspace.read(cx).view {
            Some(app_views::AppView::Sessions) => {
                let workspace = self.workspace.clone();
                let view = self
                    .sessions_view
                    .get_or_insert_with(|| {
                        cx.new(|cx| app_views::SessionsView::new(workspace, search, cx))
                    })
                    .clone();
                let details = view.update(cx, |view, cx| view.inspector(cx));
                Some((view.into_any_element(), details))
            }
            Some(app_views::AppView::Settings) => {
                let workspace = self.workspace.clone();
                let view = self
                    .settings_view
                    .get_or_insert_with(|| {
                        cx.new(|cx| app_views::SettingsView::new(workspace, search, cx))
                    })
                    .clone();
                let details = view.update(cx, |view, cx| view.inspector(cx));
                Some((view.into_any_element(), details))
            }
            Some(kind @ (app_views::AppView::Models | app_views::AppView::Resources)) => {
                let slot = if kind == app_views::AppView::Models {
                    &mut self.models_view
                } else {
                    &mut self.resources_view
                };
                let workspace = self.workspace.clone();
                let screen = slot.get_or_insert_with(|| {
                    app_views::CatalogScreen::new(workspace, search, kind, cx)
                });
                Some((
                    screen.view.clone().into_any_element(),
                    screen.inspector.clone().into_any_element(),
                ))
            }
            None => None,
        };
        v_flex()
            .id("desktop")
            .key_context("Desktop")
            .size_full()
            .bg(theme.canvas)
            .font_family(SANS)
            .text_size(px(14.))
            .line_height(px(20.))
            .text_color(theme.text)
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                this.resize(event.position.x, window, cx)
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.resizing.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &NewSession, window, cx| {
                new_session::show(this.workspace.clone(), window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenFolder, _, cx| {
                this.workspace
                    .update(cx, |workspace, cx| workspace.open_folder(cx))
            }))
            .on_action(cx.listener(|this, _: &ToggleTheme, _, cx| this.toggle_theme(cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &ToggleInspector, _, cx| this.toggle_inspector(cx)))
            .on_action(cx.listener(|this, _: &ShowFiles, window, cx| {
                let view = this
                    .workspace
                    .read(cx)
                    .active_tab_opt()
                    .map(|tab| tab.view.clone());
                if let Some(view) = view {
                    this.layout.inspector = true;
                    this.layout.inspector_requested = true;
                    this.sync_header(cx);
                    view.update(cx, |view, cx| view.show_files(window, cx));
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &ShowDiagnostics, window, cx| {
                let view = this
                    .workspace
                    .read(cx)
                    .active_tab_opt()
                    .map(|tab| tab.view.clone());
                if let Some(view) = view {
                    this.layout.inspector = true;
                    this.layout.inspector_requested = true;
                    this.sync_header(cx);
                    this.workspace.update(cx, |workspace, cx| {
                        workspace.view = None;
                        cx.emit(WorkspaceEvent::View);
                    });
                    view.update(cx, |view, cx| view.show_diagnostics(window, cx));
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &ExpandMessageEditor, window, cx| {
                if let Some(tab) = this.workspace.read(cx).active_tab_opt() {
                    let composer = tab.view.read(cx).composer.clone();
                    composer.update(cx, |composer, cx| composer.toggle_composer(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleTerminal, window, cx| {
                if let Some(tab) = this.workspace.read(cx).active_tab_opt() {
                    let view = tab.view.clone();
                    view.update(cx, |view, cx| view.toggle_terminal(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.sidebar
                    .read(cx)
                    .search
                    .focus_handle(cx)
                    .focus(window, cx)
            }))
            .on_action(cx.listener(|this, _: &Stop, window, cx| {
                let search = this.sidebar.read(cx).search.clone();
                if search.focus_handle(cx).is_focused(window) {
                    search.update(cx, |search, cx| search.set_content("", cx));
                    this.focus_composer(window, cx);
                } else if let Some(tab) = this.workspace.read(cx).active_tab_opt() {
                    let controller = tab.controller.clone();
                    controller.update(cx, |controller, cx| controller.clear_queue(true, cx));
                }
            }))
            .child(
                self.header.clone().cached(
                    gpui::StyleRefinement::default()
                        .h(px(52.))
                        .w_full()
                        .flex_shrink_0(),
                ),
            )
            .child(
                h_flex()
                    .items_stretch()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .when(self.layout.show_sidebar, |row| {
                        row.child(
                            div()
                                .relative()
                                .w(self.layout.sidebar_width)
                                .h_full()
                                .flex_shrink_0()
                                .child(
                                    self.sidebar
                                        .clone()
                                        .cached(gpui::StyleRefinement::default().size_full()),
                                )
                                .child(self.resize_handle(Pane::Sidebar, cx, theme)),
                        )
                    })
                    .when_some(app_view, |row, (view, details)| {
                        row.child(div().flex_1().min_w_0().h_full().child(view))
                            .when(show_inspector, |row| {
                                row.child(
                                    div()
                                        .relative()
                                        .w(self.layout.inspector_width)
                                        .h_full()
                                        .flex_shrink_0()
                                        .bg(theme.panel)
                                        .border_l_1()
                                        .border_color(theme.edge)
                                        .child(details)
                                        .child(self.resize_handle(Pane::Inspector, cx, theme)),
                                )
                            })
                    })
                    .when(self.workspace.read(cx).view.is_none(), |row| {
                        row.child(div().flex_1().min_w_0().h_full().child(match session {
                            Some(session) => session.into_any_element(),
                            None => self.welcome.clone().into_any_element(),
                        }))
                    })
                    .when_some(
                        inspector
                            .filter(|_| show_inspector && self.workspace.read(cx).view.is_none()),
                        |row, inspector| {
                            row.child(
                                div()
                                    .relative()
                                    .w(self.layout.inspector_width)
                                    .h_full()
                                    .flex_shrink_0()
                                    .child(
                                        inspector
                                            .cached(gpui::StyleRefinement::default().size_full()),
                                    )
                                    .child(self.resize_handle(Pane::Inspector, cx, theme)),
                            )
                        },
                    ),
            )
            .child(
                self.status.clone().cached(
                    gpui::StyleRefinement::default()
                        .h(px(24.))
                        .w_full()
                        .flex_shrink_0(),
                ),
            )
    }
}
/// A vertical scrollbar that shows whenever the content overflows, not only while
/// scrolling. Attach it to a non-scrolling wrapper around the element that tracks
/// `handle`: on the scrolling element itself, the thumb would scroll away with the content. With a `gutter` colour it reserves its own space, so the thumb never
/// covers content that reaches the pane's edge.
fn scrollbar(id: &'static str, handle: &ScrollHandle, gutter: Option<gpui::Hsla>) -> Scrollbars {
    let scrollbar = Scrollbars::always_visible(ScrollAxes::Vertical)
        .id(id)
        .tracked_scroll_handle(handle);
    match gutter {
        Some(color) => scrollbar.with_track_along(ScrollAxes::Vertical, color),
        None => scrollbar,
    }
}

mod app_views;
mod attachments;
mod changes;
mod chrome;
mod composer;
mod context;
mod demo;
mod diagnostics;
mod diff_preview;
mod extension_dialogs;
mod files;
mod inspector;
mod jj;
mod landing;
mod mention_menu;
mod mentions;
mod menus;
mod new_session;
mod panels;
mod parallel;
mod prompts;
mod session;
mod session_view;
mod sidebar;
mod slash;
mod terminal;
mod transcript;
mod tree;
mod tree_graph;
mod turn_links;
mod welcome;
mod workspace;

#[cfg(test)]
#[path = "desktop_tests.rs"]
mod tests;
