//! Study 02's project/worktree chooser. No Pi process starts before explicit Create.
use super::app_views::{segment, segments};
use super::session::{Changes, SessionEvent};
use super::*;
use gpui::{
    Bounds, EventEmitter, FocusHandle, Global, PromptHandle, PromptResponse,
    RenderablePromptHandle, point,
};
use std::{path::Path, process::Command as ProcessCommand, time::Duration};

gpui::actions!(
    new_session_models,
    [ChooseModel, NextModel, PreviousModel, DismissModel]
);
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", ChooseModel, Some("PiNewSessionModelPicker")),
        KeyBinding::new("tab", ChooseModel, Some("PiNewSessionModelPicker")),
        KeyBinding::new(
            "down",
            NextModel,
            Some("PiNewSessionModelPicker > TextInput"),
        ),
        KeyBinding::new(
            "up",
            PreviousModel,
            Some("PiNewSessionModelPicker > TextInput"),
        ),
        KeyBinding::new("escape", DismissModel, Some("PiNewSessionModelPicker")),
    ]);
}
const DETAIL: &str = "pi-desktop:new-session-form";
#[derive(Default)]
struct PendingForm(Option<Entity<WorkspaceController>>);
impl Global for PendingForm {}
pub fn show(workspace: Entity<WorkspaceController>, window: &mut Window, cx: &mut App) {
    if window.has_active_prompt() {
        return;
    }
    cx.set_global(PendingForm(Some(workspace)));
    let answer = window.prompt(
        gpui::PromptLevel::Info,
        "New session",
        Some(DETAIL),
        &[
            gpui::PromptButton::cancel("Cancel"),
            gpui::PromptButton::ok("Create session"),
        ],
        cx,
    );
    cx.spawn(async move |_| {
        let _ = answer.await;
    })
    .detach();
}
pub fn build(
    message: &str,
    detail: Option<&str>,
    handle: PromptHandle,
    window: &mut Window,
    cx: &mut App,
) -> Result<RenderablePromptHandle, PromptHandle> {
    if message != "New session" || detail != Some(DETAIL) {
        return Err(handle);
    }
    let Some(workspace) = cx.global_mut::<PendingForm>().0.take() else {
        return Err(handle);
    };
    let view = cx.new(|cx| NewSessionForm::new(workspace, cx));
    Ok(handle.with_view(view, window, cx))
}
struct NewSessionForm {
    workspace: Entity<WorkspaceController>,
    project: Option<PathBuf>,
    worktree: bool,
    remote: bool,
    remote_durable: bool,
    ssh_host: Entity<TextInput>,
    ssh_path: Entity<TextInput>,
    remote_key: Option<String>,
    branch: Entity<TextInput>,
    path: Entity<TextInput>,
    model: Option<(String, String)>,
    models_open: bool,
    model_filter: Entity<TextInput>,
    model_index: usize,
    model_scroll: gpui::UniformListScrollHandle,
    model_button_hovered: bool,
    model_anchor: std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
    creating: bool,
    error: Option<String>,
    focus: FocusHandle,
    _subscriptions: Vec<gpui::Subscription>,
}
impl NewSessionForm {
    fn new(workspace: Entity<WorkspaceController>, cx: &mut Context<Self>) -> Self {
        let project = workspace.read(cx).selected_project.clone();
        let remote_target = crate::prefs::remote_sessions(cx)
            .into_iter()
            .rev()
            .find(|target| project.as_ref() == Some(&target.identity()))
            .or_else(|| {
                workspace.read(cx).tabs.iter().find_map(|tab| {
                    tab.controller
                        .read(cx)
                        .remote_target()
                        .filter(|target| project.as_ref() == Some(&target.identity()))
                        .cloned()
                })
            });
        let remote = workspace.read(cx).selected_is_remote(cx);
        let recent = remote_target.clone().or_else(|| {
            if remote {
                None
            } else {
                crate::prefs::remote_sessions(cx).last().cloned()
            }
        });
        let ssh_host =
            cx.new(|cx| TextInput::new("SSH config host alias or user@host", cx).compact());
        let ssh_path =
            cx.new(|cx| TextInput::new("Project directory on the remote host", cx).compact());
        if let Some(target) = &recent {
            ssh_host.update(cx, |input, cx| input.set_content(target.host.clone(), cx));
            ssh_path.update(cx, |input, cx| input.set_content(target.cwd.clone(), cx));
        }
        let model_filter = cx.new(|cx| TextInput::new("Filter models…", cx).compact());
        let mut subscriptions = vec![
            cx.subscribe(&workspace, |_, _, _, cx| cx.notify()),
            cx.observe(&model_filter, |this, _, cx| {
                this.model_index = 0;
                this.model_scroll
                    .scroll_to_item(0, gpui::ScrollStrategy::Top);
                cx.notify();
            }),
        ];
        let controllers: Vec<_> = workspace
            .read(cx)
            .tabs
            .iter()
            .map(|tab| tab.controller.clone())
            .collect();
        for controller in controllers {
            subscriptions.push(cx.subscribe(&controller, |_, _, event, cx| {
                if matches!(event, SessionEvent::Changed(changes) if changes.intersects(Changes::CATALOG | Changes::RUN | Changes::STATUS)) { cx.notify(); }
            }));
        }
        if let Some(controller) = workspace
            .read(cx)
            .active_tab_opt()
            .map(|tab| tab.controller.clone())
        {
            controller.update(cx, |controller, cx| {
                controller.command(Command::GetAvailableModels, cx);
            });
        }
        Self {
            workspace,
            project,
            worktree: false,
            remote,
            remote_durable: recent
                .as_ref()
                .is_some_and(|target| target.backend == pi_core::ssh::RemoteBackend::Durable),
            ssh_host,
            ssh_path,
            remote_key: None,
            branch: cx.new(|cx| TextInput::new("pi/my-task", cx).compact()),
            path: cx.new(|cx| TextInput::new("Absolute path for the new worktree", cx).compact()),
            model: None,
            models_open: false,
            model_filter,
            model_index: 0,
            model_scroll: Default::default(),
            model_button_hovered: false,
            model_anchor: Default::default(),
            creating: false,
            error: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }
    fn cancel(&self, cx: &mut Context<Self>) {
        if !self.creating {
            cx.emit(PromptResponse(0));
        }
    }
    fn create(&mut self, cx: &mut Context<Self>) {
        if self.creating {
            return;
        }
        if self.remote {
            if self.workspace.read(cx).is_demo() {
                self.error = Some("SSH is disabled in the offline demo.".into());
                cx.notify();
                return;
            }
            let host = self.ssh_host.read(cx).content().trim().to_owned();
            let cwd = self.ssh_path.read(cx).content().trim().to_owned();
            match pi_core::ssh::SshTarget::new(host, cwd) {
                Ok(mut target) => {
                    if self.remote_durable {
                        target.backend = pi_core::ssh::RemoteBackend::Durable;
                    }
                    if let Some(existing) =
                        crate::prefs::remote_sessions(cx)
                            .into_iter()
                            .find(|existing| {
                                Some(&existing.key) == self.remote_key.as_ref()
                                    && existing.host == target.host
                                    && existing.cwd == target.cwd
                                    && existing.backend == target.backend
                            })
                    {
                        target = existing;
                    }
                    self.workspace.update(cx, |workspace, cx| {
                        workspace.open_remote(target, cx);
                    });
                    cx.emit(PromptResponse(1));
                }
                Err(error) => {
                    self.error = Some(error.to_string());
                    cx.notify();
                }
            }
            return;
        }
        let Some(project) = self.project.clone() else {
            self.error = Some("Choose a project folder first.".into());
            cx.notify();
            return;
        };
        if self.worktree && self.workspace.read(cx).is_demo() {
            self.error = Some("Worktree creation is disabled in the offline demo.".into());
            cx.notify();
            return;
        }
        let worktree = self.worktree;
        let branch = self.branch.read(cx).content().to_owned();
        let path = self.path.read(cx).content().to_owned();
        // Freeze the complete submitted form. Failed Git operations retain any files/refs.
        let model = self.model.clone();
        self.creating = true;
        self.error = None;
        self.models_open = false;
        cx.notify();
        let demo = self.workspace.read(cx).is_demo();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if worktree {
                        create_worktree(&project, &branch, Path::new(&path))
                    } else if demo {
                        Ok(project)
                    } else {
                        std::fs::canonicalize(project)
                            .map_err(anyhow::Error::from)
                            .and_then(|path| {
                                anyhow::ensure!(
                                    path.is_dir(),
                                    "Project must be an existing directory"
                                );
                                Ok(path)
                            })
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                this.creating = false;
                match result {
                    Ok(path) => {
                        this.workspace.update(cx, |workspace, cx| {
                            let id = workspace.open(path, None, cx);
                            if let Some(model) = model
                                && let Some(tab) = workspace.tab(id)
                            {
                                tab.controller.update(cx, |controller, cx| {
                                    controller.set_initial_model(model, cx)
                                });
                            }
                        });
                        cx.emit(PromptResponse(1));
                    }
                    Err(error) => {
                        this.error = Some(error.to_string());
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }
    fn pick(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose project folder".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
            {
                this.update(cx, |this, cx| {
                    if !this.creating {
                        this.project = Some(path);
                        this.worktree = false;
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }
    fn project_row(
        &self,
        index: usize,
        project: &Path,
        cx: &Context<Self>,
        theme: Theme,
    ) -> AnyElement {
        let workspace = self.workspace.read(cx);
        let selected = self.project.as_deref() == Some(project);
        let opened = workspace
            .tabs
            .iter()
            .filter(|tab| tab.controller.read(cx).model().cwd == project)
            .count();
        let working = workspace
            .tabs
            .iter()
            .filter(|tab| {
                tab.controller.read(cx).model().cwd == project && tab.controller.read(cx).working()
            })
            .count();
        let path = project.to_owned();
        h_flex()
            .id(("new-session-project", index))
            .debug_selector(move || format!("new-session-project-{index}"))
            .role(gpui::Role::RadioButton)
            .text_color(theme.text)
            .aria_toggled(if selected {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .h(px(38.))
            .w_full()
            .px(px(10.))
            .gap(px(9.))
            .rounded(px(6.))
            .when(selected, |row| row.bg(theme.selected))
            .when(!self.creating, |row| {
                row.cursor_pointer().hover(move |row| row.bg(theme.hover))
            })
            .tooltip(ui::Tooltip::text(project.display().to_string()))
            .child(
                h_flex()
                    .size(px(14.))
                    .justify_center()
                    .rounded_full()
                    .border_1()
                    .border_color(theme.line_strong)
                    .bg(theme.canvas)
                    .when(selected, |radio| {
                        radio.child(div().size(px(7.)).rounded_full().bg(theme.text))
                    }),
            )
            .child(
                icon(
                    "folder",
                    if selected {
                        theme.secondary
                    } else {
                        theme.faint
                    },
                )
                .size(px(14.)),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(
                        project
                            .file_name()
                            .unwrap_or(project.as_os_str())
                            .to_string_lossy()
                            .into_owned(),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(MONO)
                    .text_size(px(11.))
                    .text_color(theme.faint)
                    .child(short_path(&project.display().to_string())),
            )
            .child(
                h_flex()
                    .gap(px(6.))
                    .text_size(px(11.))
                    .text_color(theme.muted)
                    .when(working > 0 || opened > 0, |status| {
                        status.child(div().size(px(5.)).rounded_full().bg(if working > 0 {
                            theme.amber
                        } else {
                            theme.faint
                        }))
                    })
                    // Only a project with sessions has a status worth reading.
                    .child(if working > 0 {
                        format!("{working} working")
                    } else if opened > 0 {
                        format!("{opened} open")
                    } else {
                        String::new()
                    }),
            )
            .child(
                div()
                    .id(("new-session-project-branch", index))
                    .w(px(42.))
                    .text_right()
                    .font_family(MONO)
                    .text_size(px(10.))
                    .text_color(theme.faint)
                    .tooltip(ui::Tooltip::text(
                        "Project branch metadata is not reported here.",
                    ))
                    .child("—"),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.creating {
                    this.project = Some(path.clone());
                    this.error = None;
                    cx.notify();
                }
            }))
            .into_any_element()
    }
    fn available_models(&self, cx: &App) -> Vec<pi_core::protocol::Model> {
        let workspace = self.workspace.read(cx);
        let mut models = workspace
            .active_tab_opt()
            .map(|tab| tab.controller.read(cx).model().available_models.clone())
            .unwrap_or_default();
        if let Some(current) = workspace
            .active_tab_opt()
            .and_then(|tab| tab.controller.read(cx).model().state.model.clone())
            && !models
                .iter()
                .any(|model| model.id == current.id && model.provider == current.provider)
        {
            models.push(current);
        }
        let query = self.model_filter.read(cx).content().to_lowercase();
        models.retain(|model| {
            format!(
                "{} {} {}",
                model.provider,
                model.id,
                model.name.as_deref().unwrap_or("")
            )
            .to_lowercase()
            .contains(&query)
        });
        models
    }
    fn close_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.models_open = false;
        self.focus.focus(window, cx);
        cx.notify();
    }
    fn toggle_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.creating {
            return;
        }
        if self.models_open {
            self.close_models(window, cx);
            return;
        }
        self.model_index = 0;
        self.model_filter
            .update(cx, |filter, cx| filter.set_content("", cx));
        self.model_scroll
            .scroll_to_item(0, gpui::ScrollStrategy::Top);
        self.models_open = true;
        self.model_filter.focus_handle(cx).focus(window, cx);
        cx.notify();
    }
    fn choose_model(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.creating {
            return;
        }
        if index == 0 {
            self.model = None;
        } else if let Some(model) = self.available_models(cx).get(index - 1) {
            self.model = Some((model.provider.clone(), model.id.clone()));
        } else {
            return;
        }
        self.close_models(window, cx);
    }
    fn move_model(&mut self, direction: isize, cx: &mut Context<Self>) {
        let count = self.available_models(cx).len() + 1;
        self.model_index =
            (self.model_index as isize + direction).rem_euclid(count as isize) as usize;
        self.model_scroll
            .scroll_to_item(self.model_index, gpui::ScrollStrategy::Nearest);
        cx.notify();
    }
    fn model_row(&self, index: usize, cx: &Context<Self>, theme: Theme) -> AnyElement {
        let models = self.available_models(cx);
        let Some((title, detail, selected)) = (if index == 0 {
            Some((
                "Pi configured default".to_owned(),
                "Default for the chosen project".to_owned(),
                self.model.is_none(),
            ))
        } else {
            models.get(index - 1).map(|model| {
                (
                    model.id.clone(),
                    model.provider.clone(),
                    self.model.as_ref() == Some(&(model.provider.clone(), model.id.clone())),
                )
            })
        }) else {
            return div().into_any_element();
        };
        h_flex()
            .id(("new-model-choice", index))
            .debug_selector(move || format!("new-model-choice-{index}"))
            .h(px(50.))
            .w_full()
            .px(px(10.))
            .py(px(5.))
            .gap(px(9.))
            .rounded(px(5.))
            .cursor_pointer()
            .when(index == self.model_index, |row| row.bg(theme.selected))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered && this.model_index != index {
                    this.model_index = index;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.choose_model(index, window, cx)))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .font_family(MONO)
                            .text_size(px(11.))
                            .child(title),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(theme.faint)
                            .child(detail),
                    ),
            )
            .when(selected, |row| {
                row.child(
                    div()
                        .id(("new-model-selected", index))
                        .debug_selector(move || format!("new-model-selected-{index}"))
                        .child(icon("check", theme.accent)),
                )
            })
            .into_any_element()
    }
    fn model_popup(&self, window: &Window, cx: &Context<Self>, theme: Theme) -> AnyElement {
        let count = self.available_models(cx).len() + 1;
        let weak = cx.entity().downgrade();
        let bounds = self.model_anchor.get().unwrap_or_default();
        let above = bounds.top() - px(8.);
        let below = window.viewport_size().height - bounds.bottom() - px(8.);
        let opens_up = above >= below;
        let list_height = (if opens_up { above } else { below } - px(100.))
            .max(px(50.))
            .min(px(260.))
            .min(px(count as f32 * 50.));
        // GPUI prepares normal deferred draws *before* native prompts. A prompt
        // must paint its popovers as its own final sibling, not as Deferred.
        gpui::anchored()
            .position(point(
                bounds.left(),
                if opens_up {
                    bounds.top() - px(6.)
                } else {
                    bounds.bottom() + px(6.)
                },
            ))
            .anchor(if opens_up {
                gpui::Anchor::BottomLeft
            } else {
                gpui::Anchor::TopLeft
            })
            .snap_to_window_with_margin(px(8.))
            .child(
                v_flex()
                    .id("new-session-model-picker")
                    .debug_selector(|| "new-session-model-picker".into())
                    .key_context("PiNewSessionModelPicker")
                    .occlude()
                    .w(px(350.).min(window.viewport_size().width - px(48.)))
                    .text_color(theme.text)
                    .p(px(6.))
                    .gap(px(5.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme.chip_line)
                    .bg(if theme.light { theme.chip } else { theme.bar })
                    .shadow_lg()
                    .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                        if !this.model_button_hovered {
                            this.close_models(window, cx);
                        }
                    }))
                    .on_action(cx.listener(|_, _: &super::NewSession, _, cx| cx.stop_propagation()))
                    .on_action(cx.listener(|_, _: &super::OpenFolder, _, cx| cx.stop_propagation()))
                    .on_action(
                        cx.listener(|_, _: &super::ToggleTerminal, _, cx| cx.stop_propagation()),
                    )
                    .on_action(
                        cx.listener(|_, _: &super::ShowDiagnostics, _, cx| cx.stop_propagation()),
                    )
                    .on_action(
                        cx.listener(|_, _: &super::FocusSearch, _, cx| cx.stop_propagation()),
                    )
                    .on_action(
                        cx.listener(|_, _: &super::ToggleSidebar, _, cx| cx.stop_propagation()),
                    )
                    .on_action(cx.listener(|this, _: &ChooseModel, window, cx| {
                        cx.stop_propagation();
                        this.choose_model(this.model_index, window, cx);
                    }))
                    .on_action(cx.listener(|this, _: &NextModel, _, cx| {
                        cx.stop_propagation();
                        this.move_model(1, cx);
                    }))
                    .on_action(cx.listener(|this, _: &PreviousModel, _, cx| {
                        cx.stop_propagation();
                        this.move_model(-1, cx);
                    }))
                    .on_action(cx.listener(|this, _: &DismissModel, window, cx| {
                        cx.stop_propagation();
                        this.close_models(window, cx);
                    }))
                    .child(label("MODEL", theme).px(px(10.)).py(px(5.)))
                    .child(
                        div()
                            .mx(px(4.))
                            .px(px(8.))
                            .py(px(3.))
                            .rounded(px(5.))
                            .border_1()
                            .border_color(theme.line)
                            .bg(theme.canvas)
                            .child(self.model_filter.clone()),
                    )
                    .child(
                        gpui::uniform_list("new-session-model-list", count, move |range, _, cx| {
                            weak.update(cx, |this, cx| {
                                range
                                    .map(|index| this.model_row(index, cx, theme))
                                    .collect()
                            })
                            .unwrap_or_default()
                        })
                        .h(list_height)
                        .w_full()
                        .track_scroll(&self.model_scroll),
                    )
                    .child(
                        div()
                            .px(px(10.))
                            .text_size(px(10.))
                            .text_color(theme.faint)
                            .child("↑ ↓ select · Enter choose · Esc dismiss"),
                    ),
            )
            .into_any_element()
    }
    fn field(
        &self,
        id: &'static str,
        title: &'static str,
        input: Entity<TextInput>,
        theme: Theme,
    ) -> AnyElement {
        v_flex()
            .gap(px(4.))
            .child(label(title, theme).text_size(px(9.)))
            .child(
                h_flex()
                    .id(id)
                    .debug_selector(move || id.into())
                    .h(px(30.))
                    .px(px(11.))
                    .rounded(px(6.))
                    .bg(theme.canvas)
                    .border_1()
                    .border_color(theme.line_strong)
                    .font_family(MONO)
                    .child(input),
            )
            .into_any_element()
    }
}
impl EventEmitter<PromptResponse> for NewSessionForm {}
impl Focusable for NewSessionForm {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for NewSessionForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let workspace = self.workspace.read(cx);
        let mut projects = workspace.projects.clone();
        if let Some(project) = &self.project
            && !projects.contains(project)
        {
            projects.push(project.clone());
        }
        let overlap = self
            .project
            .as_ref()
            .map(|project| {
                workspace
                    .tabs
                    .iter()
                    .filter(|tab| {
                        tab.controller.read(cx).model().cwd == *project
                            && tab.controller.read(cx).working()
                    })
                    .count()
            })
            .unwrap_or(0);
        let width = (f32::from(window.viewport_size().width) - 48.).clamp(240., 500.);
        let height = (f32::from(window.viewport_size().height) - 48.).clamp(240., 620.);
        div().id("new-session-overlay").occlude().size_full().bg(gpui::rgba(0x00000040)).flex().items_center().justify_center()
            .child(v_flex().id("new-session-dialog").debug_selector(|| "new-session-dialog".into())
                .track_focus(&self.focus).key_context("PiPrompt").role(gpui::Role::Dialog).aria_label("New session")
                .w(px(width)).max_h(px(height)).overflow_hidden().rounded(px(10.)).text_color(theme.text).bg(if theme.light { theme.chip } else { theme.bar })
                .border_1().border_color(theme.chip_line).shadow_lg()
                .on_action(cx.listener(|this, _: &super::prompts::CancelPrompt, window, cx| { cx.stop_propagation(); if this.models_open { this.close_models(window, cx); } else { this.cancel(cx); } }))
                // Typing/Enter never implicitly runs Git or starts a subprocess.
                .on_action(cx.listener(|_, _: &super::prompts::ConfirmPrompt, _, cx| cx.stop_propagation()))
                .on_action(cx.listener(|_, _: &super::NewSession, _, cx| cx.stop_propagation()))
                .on_action(cx.listener(|_, _: &super::OpenFolder, _, cx| cx.stop_propagation()))
                .on_action(cx.listener(|_, _: &super::ToggleTerminal, _, cx| cx.stop_propagation()))
                .on_action(cx.listener(|_, _: &super::FocusSearch, _, cx| cx.stop_propagation()))
                .on_action(cx.listener(|_, _: &super::ShowDiagnostics, _, cx| cx.stop_propagation()))
                .on_action(cx.listener(|_, _: &super::ShowFiles, _, cx| cx.stop_propagation()))
                .on_action(cx.listener(|_, _: &super::ToggleInspector, _, cx| cx.stop_propagation()))
                .on_action(cx.listener(|_, _: &super::ToggleSidebar, _, cx| cx.stop_propagation()))
                .on_action(cx.listener(|_, _: &super::ExpandMessageEditor, _, cx| cx.stop_propagation()))
                .child(h_flex().px(px(24.)).pt(px(20.)).pb(px(16.)).justify_between().flex_shrink_0()
                    .child(div().font_family(SERIF).italic().text_size(px(22.)).line_height(px(28.)).child("New session"))
                    .child(h_flex().gap(px(6.))
                        .child(icon_button("new-session-folder", "plus", "Open folder", theme).size(px(28.))
                            .tooltip(ui::Tooltip::text("Add a project folder"))
                            .on_click(cx.listener(|this, _, _, cx| { if !this.creating { this.pick(cx); } })))
                        .child(icon_button("close-new-session", "close", "Cancel new session", theme).size(px(28.))
                            .tooltip(ui::Tooltip::text("Cancel · Escape")).on_click(cx.listener(|this, _, _, cx| this.cancel(cx))))))
                .child(v_flex().id("new-session-content").debug_selector(|| "new-session-content".into()).px(px(24.)).pb(px(16.))
                    .min_h_0().overflow_y_scroll()
                    .child(h_flex().mb(px(12.)).child(segments([(false, "Local"), (true, "SSH")].into_iter().map(|(remote, title)| {
                        segment(("new-session-backend", remote as usize), title, self.remote == remote, theme)
                            .on_click(cx.listener(move |this, _, _, cx| { if !this.creating { this.remote = remote; this.worktree = false; this.error = None; cx.notify(); } }))
                    }), theme)))
                    .when(self.remote, |form| form
                        .child(self.field("new-session-ssh-host", "HOST", self.ssh_host.clone(), theme))
                        .child(div().mt(px(8.)).child(self.field("new-session-ssh-path", "REMOTE PROJECT", self.ssh_path.clone(), theme)))
                        .child(h_flex().mt(px(8.)).child(segments([(false, "Stock Pi"), (true, "Durable · experimental")].into_iter().map(|(durable, title)| {
                            segment(("new-ssh-engine", durable as usize), title, self.remote_durable == durable, theme)
                                .on_click(cx.listener(move |this, _, _, cx| { this.remote_durable = durable; this.remote_key = None; cx.notify(); }))
                        }), theme)))
                        .when(self.remote_durable, |form| form.child(div().mt(px(6.)).text_size(px(11.)).text_color(theme.muted).child("Prototype: crash recovery on reconnect. Requires a durable-enabled helper. Uses the SSH host's Pi credentials (including OAuth) and built-in providers; no stock extensions, skills, images or session migration.")))
                        .child(div().mt(px(8.)).text_size(px(11.)).text_color(theme.muted).child("Uses SSH keys/agent and verified hosts. Installs a per-user helper; no sudo. Remote work continues after disconnecting."))
                        .child(button("new-ssh-session", "Start a new remote session", theme).mt(px(8.))
                            .on_click(cx.listener(|this, _, _, cx| { this.remote_key = None; cx.notify(); })))
                        .children(crate::prefs::remote_sessions(cx).into_iter().rev().take(5).enumerate().map(|(index, target)| {
                            let label = format!("Reconnect · {} · {} · {}{}", target.host, target.cwd, &target.key[..8], if target.backend == pi_core::ssh::RemoteBackend::Durable { " · durable" } else { "" });
                            let removed = target.clone();
                            h_flex().mt(px(4.)).gap(px(6.))
                                .child(button(("recent-ssh-session", index), label, theme).flex_1().min_w_0()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.ssh_host.update(cx, |input, cx| input.set_content(target.host.clone(), cx));
                                        this.ssh_path.update(cx, |input, cx| input.set_content(target.cwd.clone(), cx));
                                        this.remote_durable = target.backend == pi_core::ssh::RemoteBackend::Durable;
                                        this.remote_key = Some(target.key.clone()); cx.notify();
                                    })))
                                .child(button(("remove-recent-ssh", index), "Remove", theme)
                                    .tooltip(ui::Tooltip::text("Forget this saved shortcut. Open tabs, remote agents and remote files are unchanged."))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.workspace.update(cx, |workspace, cx| workspace.forget_remote_session(&removed, cx));
                                        if this.remote_key.as_ref() == Some(&removed.key) { this.remote_key = None; }
                                        cx.notify();
                                    })))
                        })))
                    .when(!self.remote, |form| form
                    .child(label("PROJECT", theme).mb(px(6.)))
                    .child(v_flex().id("new-session-projects").max_h(px(126.)).overflow_y_scroll().gap(px(4.)).flex_shrink_0()
                        .children(projects.iter().enumerate().map(|(index, path)| self.project_row(index, path, cx, theme))))
                    .child(label("RUN IN", theme).mt(px(8.)).mb(px(4.)))
                    .child(h_flex().child(segments([(false, "Project folder"), (true, "New git worktree")].into_iter().map(|(worktree, title)| {
                        segment(("new-session-location", worktree as usize), title, self.worktree == worktree, theme).h(px(20.))
                            .debug_selector(move || if worktree { "new-session-worktree".into() } else { "new-session-project-folder".into() })
                            .on_click(cx.listener(move |this, _, _, cx| { if !this.creating { this.worktree = worktree; cx.notify(); } }))
                    }), theme)))
                    .when(self.worktree, |form| form
                        .child(div().mt(px(8.)).child(self.field("new-session-branch", "BRANCH", self.branch.clone(), theme)))
                        .child(div().mt(px(6.)).child(self.field("new-session-path", "PATH", self.path.clone(), theme))))
                    .when(overlap > 0 || self.worktree, |form| form.child(h_flex().id("new-session-warning").debug_selector(|| "new-session-warning".into()).mt(px(16.)).gap(px(8.)).items_start()
                        .tooltip(ui::Tooltip::text("Git hooks may execute. Use an absolute, nonexistent destination. Failed operations retain files and refs; worktrees are never automatically deleted."))
                        .child(icon("warning", theme.amber).size(px(13.)))
                        .child(div().flex_1().min_w_0().text_size(px(11.)).line_height(px(16.)).text_color(theme.amber)
                            .child(if overlap > 0 { format!("{overlap} session(s) are working in this folder. A worktree keeps this one from editing the same files.") }
                                else { "Git hooks may run. New worktrees and branches are retained, including after failure.".into() })))))
                    .when(!self.remote, |form| form
                    .child(label("MODEL", theme).text_size(px(9.)).mt(px(18.)).mb(px(6.)))
                    .child(h_flex().relative().gap(px(8.))
                        .child(chip("new-session-model", "Choose initial model", theme).debug_selector(|| "new-session-model".into()).max_w(px(310.))
                            .tooltip(ui::Tooltip::text(self.model.as_ref().map(|(provider, model)| format!("{provider}/{model}")).unwrap_or_else(|| "Use Pi's configured default in the chosen project. No new process is started to discover models.".into())))
                            .child(icon("sparkle", theme.muted).size(px(13.)))
                            .child(div().min_w_0().truncate().child(self.model.as_ref().map(|(_, model)| model.clone()).unwrap_or_else(|| "Pi configured default".into())))
                            .child(icon("chevron_down", theme.faint).size(px(11.)))
                            .relative()
                            .child({ let anchor = self.model_anchor.clone(); gpui::canvas(move |bounds, _, _| anchor.set(Some(bounds)), |_, _, _, _| {}).absolute().size_full() })
                            .on_hover(cx.listener(|this, hovered: &bool, _, _| this.model_button_hovered = *hovered))
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_models(window, cx))))
                        // The thinking level is inherited, not chosen here; say so.
                        .child(h_flex().id("new-session-thinking").h(px(28.)).px(px(4.)).text_size(px(12.5)).text_color(theme.muted)
                            .tooltip(ui::Tooltip::text("Uses Pi's saved thinking preference for the chosen model. Change it in the new session; no unreported level is assumed here."))
                            .child("Thinking · inherited"))))
                    .when_some(self.error.clone(), |form, error| form.child(div().mt(px(10.)).text_size(px(12.)).line_height(px(18.)).text_color(theme.coral).child(error))))
                .child(h_flex().debug_selector(|| "new-session-footer".into()).px(px(24.)).py(px(12.)).gap(px(8.)).justify_end().border_t_1().border_color(theme.line).flex_shrink_0()
                    .child(button("cancel-new-session", "Cancel", theme).debug_selector(|| "cancel-new-session".into()).when(self.creating, |button| button.opacity(0.5))
                        .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))))
                    .child(primary_button("create-session", if self.creating { "Creating…" } else if self.remote && self.remote_key.is_some() { "Reconnect session" } else { "Create session" }, !self.creating && (self.remote || self.project.is_some()), theme)
                        .debug_selector(|| "create-session".into()).on_click(cx.listener(|this, _, _, cx| this.create(cx)))))
            )
            .when(self.models_open, |overlay| overlay.child(self.model_popup(window, cx, theme)))
    }
}

pub(super) fn create_worktree(
    project: &Path,
    branch: &str,
    path: &Path,
) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        !branch.is_empty() && !branch.starts_with('-'),
        "Enter a new branch name, not an option."
    );
    anyhow::ensure!(path.is_absolute(), "Worktree path must be absolute.");
    let parent =
        std::fs::canonicalize(path.parent().ok_or_else(|| {
            anyhow::anyhow!("Choose a worktree folder, not the filesystem root.")
        })?)?;
    let target = parent.join(
        path.file_name()
            .ok_or_else(|| anyhow::anyhow!("Choose a new folder name."))?,
    );
    anyhow::ensure!(
        !target
            .components()
            .any(|part| matches!(part.as_os_str().to_str(), Some(".git" | ".jj" | ".pi"))),
        "Worktree path cannot use protected metadata directories."
    );
    anyhow::ensure!(
        std::fs::symlink_metadata(&target)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        "Worktree destination already exists or cannot be checked."
    );
    fn git(project: &Path, args: &[&std::ffi::OsStr]) -> anyhow::Result<()> {
        let output = pi_core::bounded_output(
            ProcessCommand::new("git")
                .current_dir(project)
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env("GIT_TERMINAL_PROMPT", "0")
                .args(args),
            Duration::from_secs(30),
        )?;
        anyhow::ensure!(
            output.status.success(),
            "Git worktree operation failed; files/refs are retained: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }
    git(
        project,
        &[
            "check-ref-format".as_ref(),
            "--branch".as_ref(),
            branch.as_ref(),
        ],
    )?;
    git(
        project,
        &[
            "worktree".as_ref(),
            "add".as_ref(),
            "-b".as_ref(),
            branch.as_ref(),
            "--".as_ref(),
            target.as_os_str(),
            "HEAD".as_ref(),
        ],
    )?;
    Ok(std::fs::canonicalize(target)?)
}
