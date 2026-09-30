//! Session-backed app catalogs. No extra RPC process, disk scans or model calls.
//! Inspectors are retained entities so selection updates don't depend on shell redraws.
use super::super::session::{Changes, SessionController, SessionEvent};
use super::*;
use gpui::{PromptLevel, UniformListScrollHandle, uniform_list};

#[path = "models.rs"]
mod models;
#[path = "resources.rs"]
mod resources;
use resources::{Resource, ResourceTab};

pub struct CatalogView {
    workspace: Entity<WorkspaceController>,
    search: Entity<TextInput>,
    kind: AppView,
    controller: Option<Entity<SessionController>>,
    selected: Option<String>,
    model_filter: usize,
    provider: Option<String>,
    resource_tab: ResourceTab,
    resource_project: bool,
    project_picker: bool,
    project_picker_hovered: bool,
    installing: bool,
    install_focus: bool,
    settings_documents: crate::markdown_view::Documents,
    _settings_subscription: Option<gpui::Subscription>,
    models: Vec<pi_core::protocol::Model>,
    resources: Vec<Resource>,
    install: Entity<TextInput>,
    local: bool,
    confirming: bool,
    focus: gpui::FocusHandle,
    focus_pending: bool,
    scroll: UniformListScrollHandle,
    _session: Option<gpui::Subscription>,
    _subscriptions: Vec<gpui::Subscription>,
}

pub struct CatalogInspector(Entity<CatalogView>, gpui::Subscription);
pub struct CatalogScreen {
    pub view: Entity<CatalogView>,
    pub inspector: Entity<CatalogInspector>,
}
impl CatalogScreen {
    pub fn new(
        workspace: Entity<WorkspaceController>,
        search: Entity<TextInput>,
        kind: AppView,
        cx: &mut App,
    ) -> Self {
        let view = cx.new(|cx| CatalogView::new(workspace, search, kind, cx));
        let inspector = cx.new(|cx| {
            let subscription = cx.observe(&view, |_, _, cx| cx.notify());
            CatalogInspector(view.clone(), subscription)
        });
        Self { view, inspector }
    }
}
impl Render for CatalogInspector {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = &self.1;
        self.0.update(cx, |view, cx| view.inspector(cx))
    }
}
impl CatalogView {
    fn new(
        workspace: Entity<WorkspaceController>,
        search: Entity<TextInput>,
        kind: AppView,
        cx: &mut Context<Self>,
    ) -> Self {
        let install = cx.new(|cx| {
            TextInput::new("npm:@scope/name@version, git:host/repo@ref, or ./path", cx).compact()
        });
        let subscriptions = vec![
            cx.observe(&search, |this, _, cx| this.project(cx)),
            cx.observe(&install, |_, _, cx| cx.notify()),
            cx.subscribe(&workspace, |this, workspace, event, cx| {
                if matches!(
                    event,
                    WorkspaceEvent::Selection(_) | WorkspaceEvent::Navigation
                ) {
                    this.bind(cx);
                }
                if matches!(event, WorkspaceEvent::View)
                    && workspace.read(cx).view == Some(this.kind)
                {
                    this.reload(cx);
                }
            }),
        ];
        let mut this = Self {
            workspace,
            search,
            kind,
            controller: None,
            selected: None,
            model_filter: 0,
            provider: None,
            resource_tab: ResourceTab::Extensions,
            resource_project: true,
            project_picker: false,
            project_picker_hovered: false,
            installing: false,
            install_focus: false,
            settings_documents: Default::default(),
            _settings_subscription: None,
            models: vec![],
            resources: vec![],
            install,
            local: false,
            confirming: false,
            focus: cx.focus_handle(),
            focus_pending: true,
            scroll: UniformListScrollHandle::new(),
            _session: None,
            _subscriptions: subscriptions,
        };
        this.bind(cx);
        this
    }
    fn bind(&mut self, cx: &mut Context<Self>) {
        let workspace = self.workspace.read(cx);
        let next = workspace
            .active_tab_opt()
            .into_iter()
            .chain(workspace.tabs.iter())
            .find(|tab| {
                self.kind == AppView::Resources && !self.resource_project
                    || workspace.selected_project.as_ref()
                        == Some(&tab.controller.read(cx).model().cwd)
            })
            .map(|tab| tab.controller.clone());
        if self.controller == next {
            return;
        }
        self._session = None;
        self.installing = false;
        self.controller = next.clone();
        self.selected = None;
        self.provider = None;
        self.local = self.kind == AppView::Resources && self.resource_project;
        if let Some(controller) = next {
            self._session = Some(cx.subscribe(&controller, |this, _, event, cx| {
                if this.workspace.read(cx).view != Some(this.kind) {
                    return;
                }
                if let SessionEvent::Changed(changes) = event {
                    if changes.intersects(Changes::CATALOG | Changes::CONTEXT) {
                        this.project(cx);
                    } else if changes.intersects(Changes::METADATA | Changes::STATUS | Changes::RUN)
                    {
                        cx.notify();
                    }
                }
            }));
            self.reload(cx);
        }
        self.project(cx);
    }
    pub fn refresh_resources(&mut self, cx: &mut Context<Self>) {
        self.reload(cx);
    }
    pub fn open_install(&mut self, cx: &mut Context<Self>) {
        self.installing = true;
        self.install_focus = true;
        self.local = self.resource_project;
        cx.notify();
    }
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.focus_pending = true;
        let commands = if self.kind == AppView::Models {
            vec![
                Command::GetAvailableModels,
                Command::GetAuthProviders,
                Command::GetSettings,
                Command::GetAvailableThinkingLevels,
            ]
        } else {
            vec![
                Command::ListPackages,
                Command::GetProjectTrust,
                Command::GetCommands,
            ]
        };
        if let Some(controller) = &self.controller {
            controller.update(cx, |controller, cx| {
                for command in commands {
                    controller.command(command, cx);
                }
            });
        }
        self.project(cx);
    }
    fn model<'a>(&self, cx: &'a App) -> Option<&'a Session> {
        self.controller
            .as_ref()
            .map(|controller| controller.read(cx).model())
    }
    fn mutable(&self, cx: &App) -> bool {
        !self.confirming
            && self.controller.as_ref().is_some_and(|controller| {
                let controller = controller.read(cx);
                !controller.is_demo() && controller.can_navigate()
            })
    }
    fn send(&mut self, command: Command, cx: &mut Context<Self>) {
        if !self.mutable(cx) {
            return;
        }
        if let Some(controller) = &self.controller {
            controller.update(cx, |controller, cx| {
                controller.command(command, cx);
            });
        }
        cx.notify();
    }
    fn confirm(
        &mut self,
        command: Command,
        title: &str,
        message: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.mutable(cx) {
            return;
        }
        let Some(controller) = self.controller.clone() else {
            return;
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            title,
            Some(message),
            &["Cancel", "Continue"],
            cx,
        );
        self.confirming = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let confirmed = answer.await == Ok(1);
            this.update(cx, |this, cx| {
                this.confirming = false;
                // Never apply an old dialog to a newly selected project/session.
                if confirmed && this.controller.as_ref() == Some(&controller) {
                    this.installing = false;
                    this.send(command, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    fn project(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).content().trim().to_lowercase();
        if self.kind == AppView::Models {
            self.project_models(&query, cx);
        } else {
            self.project_resources(&query, cx);
            self.sync_settings(cx);
        }
        let keys: Vec<String> = if self.kind == AppView::Models {
            self.models.iter().map(models::key).collect()
        } else {
            self.resources.iter().map(|row| row.key.clone()).collect()
        };
        if !self
            .selected
            .as_ref()
            .is_some_and(|selected| keys.contains(selected))
        {
            self.selected = if self.kind == AppView::Models {
                keys.first().cloned()
            } else {
                None
            };
        }
        cx.notify();
    }
    fn inspector(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme(cx);
        let content = if self.kind == AppView::Models {
            self.model_details(cx, theme)
        } else {
            self.resource_details(cx, theme)
        };
        v_flex()
            .id("catalog-inspector")
            .debug_selector(|| "catalog-inspector".into())
            .size_full()
            .overflow_y_scroll()
            .p(px(20.))
            .gap(px(8.))
            .child(content)
            .into_any_element()
    }
    fn notice(&self, cx: &App, theme: Theme) -> AnyElement {
        let message = match self.controller.as_ref() {
            None => Some("Open a session in this project to load its catalog. No background session is started automatically.".to_owned()),
            Some(controller) => {
                let controller = controller.read(cx);
                if !controller.is_connected() && !controller.is_demo() {
                    Some("Pi is disconnected. Displayed catalog data may be stale; changes are disabled.".into())
                } else if controller.working() {
                    Some("Session busy — configuration actions are paused until it settles.".into())
                } else { None }
            }
        };
        div()
            .when_some(message, |view, message| {
                view.p(px(12.))
                    .text_size(px(12.))
                    .text_color(theme.amber)
                    .child(message)
            })
            .into_any_element()
    }
}
impl CatalogView {
    fn toast(&self, cx: &Context<Self>, theme: Theme) -> Option<AnyElement> {
        let session = self.model(cx)?;
        let (error, message) = session
            .error
            .as_ref()
            .map(|message| (true, message))
            .or_else(|| session.notice.as_ref().map(|message| (false, message)))?;
        let displayed = message.clone();
        let owner = self.controller.clone();
        let close = icon_button(
            "catalog-toast-close",
            "close",
            "Dismiss notification",
            theme,
        )
        .debug_selector(|| "catalog-toast-close".into())
        .on_click(cx.listener(move |this, _, _, cx| {
            if this.controller == owner
                && let Some(controller) = &owner
            {
                controller.update(cx, |controller, cx| {
                    let current = if error {
                        &controller.model().error
                    } else {
                        &controller.model().notice
                    };
                    if current.as_ref() == Some(&displayed) {
                        if error {
                            controller.dismiss_error(cx);
                        } else {
                            controller.dismiss_notice(cx);
                        }
                    }
                });
            }
        }))
        .into_any_element();
        Some(crate::components::notification_overlay(
            crate::components::notification_card("catalog-toast", message, error, close, theme)
                .into_any_element(),
            px(16.),
        ))
    }
}
impl Render for CatalogView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !window.has_active_prompt() && std::mem::take(&mut self.focus_pending) {
            self.focus.focus(window, cx);
        }
        let theme = theme(cx);
        if self.kind == AppView::Resources {
            if self.installing
                && !window.has_active_prompt()
                && std::mem::take(&mut self.install_focus)
            {
                self.install.focus_handle(cx).focus(window, cx);
            }
            return self.resources_screen(window, cx, theme);
        }
        let weak = cx.entity().downgrade();
        let models = self.kind == AppView::Models;
        let count = if models {
            self.models.len()
        } else {
            self.resources.len()
        };
        v_flex().id(if models { "models-screen" } else { "resources-screen" })
            .debug_selector(move || if models { "models-screen".into() } else { "resources-screen".into() })
            .track_focus(&self.focus)
            .relative().size_full().min_w_0().overflow_hidden()
            .child(self.notice(cx, theme))
            .child(if models { self.models_header(cx, theme) } else { self.resources_header(cx, theme) })
            .child(div().flex_1().min_h(px(80.)).child(
                uniform_list("catalog-list", count, move |range, _, cx| {
                    weak.update(cx, |this, cx| range.map(|index| if this.kind == AppView::Models {
                        this.model_row(index, cx, theme)
                    } else { this.resource_row(index, cx, theme) }).collect()).unwrap_or_default()
                }).track_scroll(&self.scroll).size_full()
            ))
            .when(count == 0, |view| view.child(div().px(px(24.)).pb(px(18.)).text_color(theme.muted)
                .child(if models && self.model(cx).is_none_or(|session| !session.models_loaded) { "The model catalog has not been reported. Refresh or check session diagnostics." }
                    else if models { "No matching models. Refresh or configure a provider in Pi." }
                    else if self.resource_tab == ResourceTab::Context { "Pi RPC does not report the loaded context-file manifest." }
                    else { "No matching resources reported. An unavailable catalog is not proof that none are configured." })))
            .child(if models { self.models_footer(theme) } else { self.resources_footer(cx, theme) })
            .when_some(self.toast(cx, theme), |view, toast| view.child(toast)).into_any_element()
    }
}

fn body_text(text: impl Into<SharedString>, theme: Theme) -> Div {
    div()
        .min_w_0()
        .text_size(px(11.))
        .line_height(px(17.))
        .text_color(theme.muted)
        .child(text.into())
}
fn cell(text: impl Into<SharedString>, width: f32, theme: Theme) -> Div {
    div()
        .w(px(width))
        .flex_shrink_0()
        .truncate()
        .font_family(MONO)
        .text_size(px(11.))
        .text_color(theme.secondary)
        .child(text.into())
}
