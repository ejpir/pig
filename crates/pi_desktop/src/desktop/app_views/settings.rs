//! Settings (design study 02 screen 10, study 06): two groups in one view. PI is
//! pi's `settings.json` with every documented key (`pi_settings`); PI DESKTOP is
//! the app's own preferences (`crate::prefs`), which no agent reads. Each group
//! has a user file and a project file, and project scope lists only what a
//! project may override. A change is written to that scope's file at once;
//! resetting removes the key so the value is inherited. pi's running sessions
//! keep what they started with; desktop settings say when they apply.
use super::super::panels::note;
use super::super::workspace::WorkspaceController;
use super::*;
use gpui::Task;
use pi_core::session::BackendInfo;
use pi_settings::{Kind, Scope, Setting, SettingsFile, display};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const DOCS: &str =
    "https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/settings.md";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    User,
    Project,
}

/// Where a setting comes from: pi's settings files or the desktop's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Pi,
    Desktop,
}
const SOURCES: [Source; 2] = [Source::Pi, Source::Desktop];

impl Source {
    fn index(self) -> usize {
        self as usize
    }

    fn schema(self) -> &'static [Setting] {
        match self {
            Self::Pi => pi_settings::SETTINGS,
            Self::Desktop => crate::prefs::DESKTOP,
        }
    }

    fn group(self) -> &'static str {
        match self {
            Self::Pi => "PI",
            Self::Desktop => "PI DESKTOP",
        }
    }

    fn categories(self) -> Vec<&'static str> {
        match self {
            Self::Pi => pi_settings::categories(),
            Self::Desktop => crate::prefs::categories(),
        }
    }

    fn setting(self, key: &str) -> Option<&'static Setting> {
        self.schema().iter().find(|setting| setting.key == key)
    }

    fn project_path(self, root: &Path) -> PathBuf {
        match self {
            Self::Pi => pi_settings::project_path(root),
            Self::Desktop => crate::prefs::project_path(root),
        }
    }

    fn project_file(self) -> &'static str {
        match self {
            Self::Pi => ".pi/settings.json",
            Self::Desktop => ".pi/pi-desktop.json",
        }
    }

    /// Project scope lists only the desktop settings a project may override. pi's
    /// user-only settings stay listed, marked, as pi documents them.
    fn listed(self, which: Which, setting: &Setting) -> bool {
        !(self == Self::Desktop && which == Which::Project && setting.scope == Scope::UserOnly)
    }

    /// A value as the row shows it: `alt` is "While ⌥ is held".
    fn label(self, setting: &Setting, value: &Value) -> String {
        match (self, setting.kind) {
            (Self::Desktop, Kind::Choice(_)) => crate::prefs::choice_label(setting.key, value),
            _ => display(value),
        }
    }
}

/// A setting or category, by source: both groups have a Terminal.
type Key = (Source, &'static str);

pub struct SettingsView {
    workspace: Entity<WorkspaceController>,
    search: Entity<TextInput>,
    which: Which,
    user: [SettingsFile; 2],
    project: [Option<SettingsFile>; 2],
    project_root: Option<PathBuf>,
    category: Key,
    selected: Key,
    editing: Option<(Key, Entity<TextInput>)>,
    models_open: bool,
    error: Option<String>,
    _load: Option<Task<()>>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl SettingsView {
    pub fn new(
        workspace: Entity<WorkspaceController>,
        search: Entity<TextInput>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&search, |_, _, cx| cx.notify()),
            cx.subscribe(&workspace, |this, workspace, _, cx| {
                if workspace.read(cx).selected_project != this.project_root {
                    this.load(cx);
                }
            }),
        ];
        let mut this = Self {
            workspace,
            search,
            which: Which::User,
            user: Default::default(),
            project: Default::default(),
            project_root: None,
            category: (Source::Pi, pi_settings::categories()[0]),
            selected: (Source::Pi, "defaultThinkingLevel"),
            editing: None,
            models_open: false,
            error: None,
            _load: None,
            _subscriptions: subscriptions,
        };
        this.load(cx);
        this
    }

    /// Reads the files off the UI thread. The desktop's go to `prefs` too, so a
    /// hand edit applies once Settings shows it.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        let root = self.workspace.read(cx).selected_project.clone();
        self.project_root = root.clone();
        let desktop_user = crate::prefs::user_path(cx);
        let read = cx.background_executor().spawn(async move {
            let user = [
                pi_settings::user_path().map(SettingsFile::load),
                Some(desktop_user.map(SettingsFile::load).unwrap_or_default()),
            ];
            let project = SOURCES.map(|source| {
                root.as_deref()
                    .map(|root| SettingsFile::load(source.project_path(root)))
            });
            (user, project)
        });
        self._load = Some(cx.spawn(async move |this, cx| {
            let ([pi_user, desktop_user], project) = read.await;
            this.update(cx, |this, cx| {
                match pi_user {
                    Some(user) => this.user[Source::Pi.index()] = user,
                    None => {
                        this.error = Some("Cannot find the home folder for ~/.pi/agent.".into())
                    }
                }
                if let Some(user) = desktop_user {
                    if !user.path.as_os_str().is_empty() {
                        crate::prefs::reloaded(cx, user.clone());
                    }
                    this.user[Source::Desktop.index()] = user;
                }
                if let Some(file) = &project[Source::Desktop.index()] {
                    crate::prefs::reloaded(cx, file.clone());
                }
                this.project = project;
                cx.notify();
            })
            .ok();
        }));
    }

    fn file(&self, source: Source) -> Option<&SettingsFile> {
        match self.which {
            Which::User => Some(&self.user[source.index()]),
            Which::Project => self.project[source.index()].as_ref(),
        }
    }

    fn is_set(&self, (source, key): Key) -> bool {
        self.file(source)
            .is_some_and(|file| file.get(key).is_some())
    }

    /// The scope's value, else what applies without it: the user file's, then the default.
    fn value(&self, source: Source, setting: &Setting) -> (Option<Value>, bool) {
        if let Some(value) = self.file(source).and_then(|file| file.get(setting.key)) {
            return (Some(value.clone()), true);
        }
        let inherited = match self.which {
            Which::Project => self.user[source.index()].get(setting.key).cloned(),
            Which::User => None,
        };
        (inherited.or_else(|| setting.default_value()), false)
    }

    fn locked(&self, setting: &Setting) -> bool {
        self.which == Which::Project && setting.scope == Scope::UserOnly
    }

    /// Changes one key in the scope's file and writes it; desktop settings apply at once.
    fn write(&mut self, (source, key): Key, value: Option<Value>, cx: &mut Context<Self>) {
        let file = match self.which {
            Which::User => &mut self.user[source.index()],
            Which::Project => match self.project[source.index()].as_mut() {
                Some(file) => file,
                None => return,
            },
        };
        if let Some(error) = &file.error {
            self.error = Some(format!("{error}; fix the file by hand first."));
            cx.notify();
            return;
        }
        match value {
            Some(value) => file.set(key, value),
            None => file.remove(key),
        }
        let snapshot = file.clone();
        self.error = None;
        let saved = match source {
            Source::Pi => cx
                .background_executor()
                .spawn(async move { snapshot.save() }),
            Source::Desktop => crate::prefs::write(cx, snapshot),
        };
        cx.spawn(async move |this, cx| {
            if let Err(error) = saved.await {
                this.update(cx, |this, cx| {
                    this.error = Some(format!("Not saved: {error:#}"));
                    this.load(cx);
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }

    fn start_edit(
        &mut self,
        source: Source,
        setting: &'static Setting,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (value, _) = self.value(source, setting);
        let input = cx.new(|cx| TextInput::new(setting.default_text, cx).compact());
        input.update(cx, |input, cx| {
            input.set_content(value.as_ref().map(display).unwrap_or_default(), cx)
        });
        input.focus_handle(cx).focus(window, cx);
        self.selected = (source, setting.key);
        self.editing = Some(((source, setting.key), input));
        cx.notify();
    }

    fn finish_edit(&mut self, cx: &mut Context<Self>) {
        let Some(((source, key), input)) = self.editing.take() else {
            return;
        };
        let Some(setting) = source.setting(key) else {
            return;
        };
        let text = input.read(cx).content().to_owned();
        if text.trim().is_empty() {
            self.write((source, key), None, cx);
            return;
        }
        match setting.parse(&text) {
            Ok(value) => self.write((source, key), Some(value), cx),
            Err(error) => {
                self.error = Some(format!("{}: {error:#}", setting.key));
                cx.notify();
            }
        }
    }

    fn open_json(&mut self, source: Source, cx: &mut Context<Self>) {
        let Some(path) = self
            .file(source)
            .map(|file| file.path.clone())
            .filter(|path| !path.as_os_str().is_empty())
        else {
            return;
        };
        // An explicit open: a missing file starts as an empty object.
        let open = cx.background_executor().spawn({
            let path = path.clone();
            async move {
                if !path.exists() {
                    std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
                    std::fs::write(&path, "{}\n")?;
                }
                anyhow::Ok(())
            }
        });
        cx.spawn(async move |this, cx| {
            let result = open.await;
            this.update(cx, |this, cx| match result {
                Ok(()) => cx.open_with_system(&path),
                Err(error) => {
                    this.error = Some(format!("{error:#}"));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The rows shown: the category's, or every match for a search.
    fn rows(&self, cx: &App) -> Vec<(Source, &'static Setting)> {
        let query = self.search.read(cx).content().trim().to_lowercase();
        SOURCES
            .into_iter()
            .flat_map(|source| source.schema().iter().map(move |setting| (source, setting)))
            .filter(|(source, setting)| {
                source.listed(self.which, setting)
                    && if query.is_empty() {
                        (*source, setting.category) == self.category
                    } else {
                        setting.key.to_lowercase().contains(&query)
                            || setting.title.to_lowercase().contains(&query)
                            || setting.description.to_lowercase().contains(&query)
                    }
            })
            .collect()
    }

    fn control(
        &self,
        source: Source,
        setting: &'static Setting,
        cx: &Context<Self>,
        theme: Theme,
    ) -> AnyElement {
        let key = setting.key;
        if self.locked(setting) {
            return div()
                .text_size(px(10.5))
                .text_color(theme.faint)
                .child("user file only")
                .into_any_element();
        }
        let (value, set) = self.value(source, setting);
        let muted = !set;
        match setting.kind {
            Kind::Bool => {
                let on = value.as_ref().and_then(Value::as_bool).unwrap_or(false);
                toggle(keyed("setting-toggle", key), on, theme)
                    .debug_selector(move || format!("setting-toggle-{key}"))
                    .when(muted, |toggle| toggle.opacity(0.8))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = (source, key);
                        this.write((source, key), Some(json!(!on)), cx)
                    }))
                    .into_any_element()
            }
            Kind::Choice(_) => segments(
                setting
                    .choices()
                    .into_iter()
                    .enumerate()
                    .map(|(i, choice)| {
                        let selected = value.as_ref() == Some(&choice);
                        let label = source.label(setting, &choice);
                        let color = (source == Source::Pi && key == "defaultThinkingLevel")
                            .then(|| theme.thinking(&label));
                        segment(
                            keyed(&format!("setting-choice-{i}"), key),
                            label,
                            selected,
                            theme,
                        )
                        .debug_selector(move || format!("setting-{key}-{i}"))
                        .when_some(color.filter(|_| !selected), |segment, color| {
                            segment.text_color(color)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected = (source, key);
                            this.write((source, key), Some(choice.clone()), cx)
                        }))
                    }),
                theme,
            )
            .when(muted, |segments| segments.opacity(0.8))
            .into_any_element(),
            Kind::Json => button(keyed("setting-json", key), "Edit in settings.json", theme)
                .on_click(cx.listener(move |this, _, _, cx| this.open_json(source, cx)))
                .into_any_element(),
            Kind::Text | Kind::Number | Kind::List => {
                if let Some((editing, input)) = &self.editing
                    && *editing == (source, key)
                {
                    return div()
                        .key_context("SettingEdit")
                        .on_action(cx.listener(|this, _: &Submit, _, cx| this.finish_edit(cx)))
                        .on_action(cx.listener(|this, _: &Stop, _, cx| {
                            this.editing = None;
                            cx.notify();
                        }))
                        .w(px(250.))
                        .h(px(26.))
                        .px(px(8.))
                        .rounded(px(5.))
                        .border_1()
                        .border_color(theme.focus)
                        .bg(theme.canvas)
                        .child(input.clone())
                        .into_any_element();
                }
                let shown = value
                    .as_ref()
                    .map(display)
                    .filter(|text| !text.is_empty())
                    .unwrap_or_else(|| setting.default_text.to_owned());
                let models =
                    source == Source::Pi && key == "defaultModel" && !self.models(cx).is_empty();
                value_chip(keyed("setting-value", key), theme)
                    .max_w(px(250.))
                    .when(muted, |chip| chip.text_color(theme.faint))
                    .child(div().min_w_0().truncate().child(if models {
                        let provider = self
                            .value(Source::Pi, pi_settings::setting("defaultProvider").unwrap())
                            .0;
                        match provider.as_ref().map(display) {
                            Some(provider) => format!("{provider} / {shown}"),
                            None => shown,
                        }
                    } else {
                        shown
                    }))
                    .when(models, |chip| {
                        chip.child(icon("chevron_down", theme.faint).size(px(11.)))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if models {
                            this.selected = (source, key);
                            this.models_open = !this.models_open;
                            cx.notify();
                        } else {
                            this.start_edit(source, setting, window, cx)
                        }
                    }))
                    .into_any_element()
            }
        }
    }

    /// Models the open sessions' pi reported, for the default-model picker.
    fn models(&self, cx: &App) -> Vec<(String, String)> {
        let mut models: Vec<(String, String)> = self
            .workspace
            .read(cx)
            .tabs
            .iter()
            .flat_map(|tab| tab.controller.read(cx).model().available_models.iter())
            .map(|model| (model.provider.clone(), model.id.clone()))
            .collect();
        models.sort();
        models.dedup();
        models
    }

    /// "changed" in user scope; "this project" or "inherits" in project scope.
    fn badge(&self, key: Key, theme: Theme) -> Option<impl IntoElement> {
        let set = self.is_set(key);
        let text = match self.which {
            Which::User if set => "changed",
            Which::User => return None,
            Which::Project if self.project[key.0.index()].is_none() => return None,
            Which::Project if set => "this project",
            Which::Project => "inherits",
        };
        let (fg, bg, border) = if set {
            (theme.accent, theme.selected, theme.focus)
        } else {
            (theme.faint, theme.hover, theme.line)
        };
        Some(
            div()
                .px(px(5.))
                .h(px(15.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .border_1()
                .border_color(border)
                .bg(bg)
                .text_size(px(9.))
                .text_color(fg)
                .child(text),
        )
    }

    fn row(
        &self,
        source: Source,
        setting: &'static Setting,
        cx: &Context<Self>,
        theme: Theme,
    ) -> impl IntoElement {
        let key = setting.key;
        let selected = self.selected == (source, key);
        h_flex()
            .id(keyed("setting-row", key))
            .debug_selector(move || format!("setting-row-{key}"))
            .relative()
            .min_h(px(50.))
            .px(px(8.))
            .gap(px(16.))
            .rounded(px(6.))
            .border_b_1()
            .border_color(theme.hover)
            .when(selected, |row| row.bg(theme.hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = (source, key);
                cx.notify();
            }))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .py(px(7.))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(theme.text)
                                    .child(setting.title),
                            )
                            .children(self.badge((source, key), theme)),
                    )
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(10.5))
                            .text_color(theme.faint)
                            .child(key),
                    ),
            )
            .child(self.control(source, setting, cx, theme))
            .when(
                self.models_open && source == Source::Pi && key == "defaultModel",
                |row| {
                    let models = self.models(cx);
                    row.child(
                        v_flex()
                            .id("default-models")
                            .absolute()
                            .top(px(44.))
                            .right(px(8.))
                            .w(px(260.))
                            .max_h(px(260.))
                            .overflow_y_scroll()
                            .p(px(4.))
                            .rounded(px(7.))
                            .border_1()
                            .border_color(theme.chip_line)
                            .bg(if theme.light { theme.chip } else { theme.bar })
                            .shadow_lg()
                            .occlude()
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.models_open = false;
                                cx.notify();
                            }))
                            .children(models.into_iter().enumerate().map(
                                |(i, (provider, model))| {
                                    h_flex()
                                        .id(("default-model", i))
                                        .h(px(26.))
                                        .px(px(8.))
                                        .rounded(px(4.))
                                        .font_family(MONO)
                                        .text_size(px(11.))
                                        .cursor_pointer()
                                        .hover(move |row| row.bg(theme.hover))
                                        .child(format!("{provider} / {model}"))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.models_open = false;
                                            this.write(
                                                (Source::Pi, "defaultProvider"),
                                                Some(json!(provider.clone())),
                                                cx,
                                            );
                                            this.write(
                                                (Source::Pi, "defaultModel"),
                                                Some(json!(model.clone())),
                                                cx,
                                            );
                                        }))
                                },
                            )),
                    )
                },
            )
    }

    /// Which pi runs sessions, from the open sessions' `get_backend_info` answers.
    fn found(&self, cx: &App, theme: Theme) -> AnyElement {
        let workspace = self.workspace.read(cx);
        let controllers: Vec<_> = workspace
            .tabs
            .iter()
            .map(|tab| tab.controller.read(cx))
            .filter(|controller| controller.program().is_some())
            .collect();
        let Some(first) = controllers.first() else {
            return note("Open a session to see what runs it.", theme).into_any_element();
        };
        let answered = controllers
            .iter()
            .find(|controller| matches!(controller.model().backend, BackendInfo::Found(_)))
            .or_else(|| {
                controllers
                    .iter()
                    .find(|controller| controller.model().backend == BackendInfo::Unsupported)
            })
            .unwrap_or(first);
        let program = answered.program().unwrap_or_default().to_owned();
        let mut panel = v_flex().child(detail("Runs", program, true, theme));
        match &answered.model().backend {
            BackendInfo::Found(info) => {
                let text = |key: &str| match &info[key] {
                    Value::Null => "not reported".to_owned(),
                    value => display(value),
                };
                panel = panel
                    .child(detail("pi", text("piVersion"), true, theme))
                    .child(detail("Extension", text("version"), true, theme))
                    .child(detail("Protocol", text("protocolVersion"), true, theme))
                    .child(match &info["bunVersion"] {
                        Value::String(version) => {
                            detail("Runtime", format!("Bun {version}"), true, theme)
                        }
                        _ => detail("Node.js", text("nodeVersion"), true, theme),
                    });
                if info["piVersion"].as_str() != Some(pi_core::extension::PI_VERSION) {
                    panel = panel.child(
                        note(
                            format!(
                                "Pi Desktop supports pi {}; this pi may lack what it needs.",
                                pi_core::extension::PI_VERSION
                            ),
                            theme,
                        )
                        .mt(px(4.)),
                    );
                }
            }
            BackendInfo::Unsupported => {
                panel = panel.child(
                    note(
                        "A pi without Pi Desktop's extension, which reports no versions.",
                        theme,
                    )
                    .mt(px(4.)),
                )
            }
            BackendInfo::Unknown => {
                panel = panel.child(note("Waiting for the session to answer.", theme).mt(px(4.)))
            }
        }
        panel.into_any_element()
    }

    pub fn inspector(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme(cx);
        let (source, key) = self.selected;
        let Some(setting) = source.setting(key) else {
            return div().into_any_element();
        };
        let (value, set) = self.value(source, setting);
        let project = self.which == Which::Project;
        let path = self
            .file(source)
            .map(|file| file.path.as_path())
            .filter(|path| !path.as_os_str().is_empty())
            .map(tilde);
        let file_name = match (source, project) {
            (Source::Desktop, true) => "PI-DESKTOP.JSON",
            _ => "SETTINGS.JSON",
        };
        let snippet = set.then(|| {
            let mut nested = value.clone().unwrap_or(Value::Null);
            for part in key.rsplit('.') {
                nested = json!({ part: nested });
            }
            serde_json::to_string_pretty(&nested).unwrap_or_default()
        });
        let shown = |value: Option<&Value>| {
            value
                .map(|value| source.label(setting, value))
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| setting.default_text.to_owned())
        };
        let default = setting.default_value();
        let yours = self.user[source.index()]
            .get(key)
            .cloned()
            .or_else(|| default.clone());
        let dismissed = crate::prefs::dismissed(cx);
        v_flex()
            .id("settings-inspector")
            .debug_selector(|| "settings-inspector".into())
            .size_full()
            .overflow_y_scroll()
            .px(px(20.))
            .pt(px(16.))
            .pb(px(16.))
            .gap(px(2.))
            .text_size(px(12.))
            .child(inspector_title(key, false))
            .child(
                h_flex()
                    .mt(px(4.))
                    .h(px(22.))
                    .gap(px(8.))
                    .text_color(theme.secondary)
                    .child(div().size(px(7.)).rounded_full().bg(if set { theme.accent } else { theme.faint }))
                    .child(match (set, project) {
                        (true, false) => "Changed in user settings",
                        (true, true) => "Set for this project",
                        (false, false) => "Default",
                        (false, true) => "Inherited",
                    })
                    .child(div().flex_1())
                    .when(set, |line| {
                        line.child(
                            div()
                                .font_family(MONO)
                                .text_size(px(11.))
                                .text_color(theme.faint)
                                .child(format!("was {}", shown(if project { yours.as_ref() } else { default.as_ref() }))),
                        )
                    }),
            )
            .child(divider(theme).my(px(10.)))
            .child(section("DESCRIPTION", "", theme))
            .child(div().mt(px(4.)).text_size(px(12.)).line_height(px(18.)).text_color(theme.secondary).child(setting.description))
            .when(source == Source::Pi && setting.scope == Scope::UserOnly, |panel| {
                panel.child(note("Only the user file sets this; pi ignores it in project files.", theme).mt(px(4.)))
            })
            .child(divider(theme).my(px(10.)))
            .child(section("VALUE", "", theme))
            .child(detail(if project { "This project" } else { "Current" }, shown(value.as_ref()), true, theme))
            .when(project, |panel| panel.child(detail("Your default", shown(yours.as_ref()), true, theme)))
            .child(detail("Default", shown(default.as_ref()), true, theme))
            .child(detail("Allowed", match setting.kind {
                Kind::Choice(_) => setting.choices().iter().map(|choice| source.label(setting, choice)).collect::<Vec<_>>().join(", "),
                _ => setting.allowed(),
            }, true, theme))
            .when(source == Source::Desktop && key == "general.rememberDismissed" && dismissed > 0, |panel| {
                panel
                    .child(divider(theme).my(px(10.)))
                    .child(section("REMEMBERED", "", theme))
                    .child(detail("jj offer declined", format!("{dismissed} project{}", if dismissed == 1 { "" } else { "s" }), true, theme))
                    .child(
                        h_flex().mt(px(6.)).child(
                            button("forget-dismissed", "Ask again", theme)
                                .debug_selector(|| "forget-dismissed".into())
                                .on_click(cx.listener(|_, _, _, cx| {
                                    crate::prefs::forget_dismissed(cx);
                                    cx.notify();
                                })),
                        ),
                    )
            })
            .child(divider(theme).my(px(10.)))
            .child(section(&format!("IN {file_name}"), "", theme))
            .child(match snippet {
                Some(snippet) => div()
                    .mt(px(4.))
                    .p(px(12.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme.line)
                    .bg(theme.deep)
                    .font_family(MONO)
                    .text_size(px(11.))
                    .line_height(px(18.))
                    .text_color(theme.secondary)
                    .children(snippet.lines().map(|line| div().whitespace_nowrap().child(line.to_owned())))
                    .into_any_element(),
                None => note(if project { "Not set here; the value above is inherited." } else { "Not set here; the default applies." }, theme).into_any_element(),
            })
            .when_some(path, |panel, path| {
                panel.child(div().mt(px(6.)).font_family(MONO).text_size(px(10.5)).text_color(theme.faint).child(path))
            })
            .child(
                h_flex()
                    .mt(px(10.))
                    .gap(px(8.))
                    .child(
                        button("reset-setting", if project { "Remove override" } else { "Reset to default" }, theme)
                            .debug_selector(|| "reset-setting".into())
                            .when(!set, |button| button.opacity(0.5))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if set {
                                    this.write((source, key), None, cx)
                                }
                            })),
                    )
                    .when(source == Source::Pi, |buttons| {
                        buttons.child(
                            button("setting-docs", "[ DOCS ]", theme)
                                .font_family(MONO)
                                .on_click(|_, _, cx| cx.open_url(DOCS)),
                        )
                    }),
            )
            // pi reads these settings; which pi that is.
            .when(source == Source::Pi, |panel| {
                panel
                    .child(divider(theme).my(px(10.)))
                    .child(section("PI", "", theme))
                    .child(self.found(cx, theme))
            })
            .child(div().flex_1().min_h(px(16.)))
            .child(note(match (source, project) {
                (Source::Pi, _) => "New sessions start with these settings. Running sessions keep theirs until they restart.",
                (Source::Desktop, false) => "pi-desktop's own settings; pi never reads this file.",
                (Source::Desktop, true) => "Everyone who opens the project gets these; commit the file to share them.",
            }, theme))
            .into_any_element()
    }

    fn category_row(
        &self,
        (source, category): Key,
        i: usize,
        cx: &Context<Self>,
        theme: Theme,
    ) -> impl IntoElement {
        let searching = !self.search.read(cx).content().trim().is_empty();
        let selected = !searching && self.category == (source, category);
        let usable = source
            .schema()
            .iter()
            .any(|setting| setting.category == category && source.listed(self.which, setting));
        let changed = self.file(source).is_some_and(|file| {
            file.changed(source.schema())
                .iter()
                .any(|setting| setting.category == category && source.listed(self.which, setting))
        });
        let id = match source {
            Source::Pi => format!("settings-category-{i}"),
            Source::Desktop => format!("desktop-category-{i}"),
        };
        h_flex()
            .id(SharedString::from(id.clone()))
            .debug_selector(move || id.clone())
            .relative()
            .h(px(26.))
            .px(px(14.))
            .rounded(px(5.))
            .cursor_pointer()
            .text_size(px(12.5))
            .text_color(if selected {
                theme.text
            } else if usable {
                theme.secondary
            } else {
                theme.faint
            })
            .when(selected, |row| row.bg(theme.selected))
            .hover(move |row| row.bg(theme.hover))
            .child(div().flex_1().child(category))
            .when(changed, |row| {
                row.child(div().size(px(5.)).rounded_full().bg(theme.accent))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.category = (source, category);
                let search = this.search.clone();
                search.update(cx, |search, cx| search.set_content("", cx));
                if let Some(first) = source.schema().iter().find(|setting| {
                    setting.category == category && source.listed(this.which, setting)
                }) {
                    this.selected = (source, first.key);
                }
                this.editing = None;
                cx.notify();
            }))
    }

    fn set_scope(&mut self, which: Which, cx: &mut Context<Self>) {
        self.which = which;
        self.editing = None;
        // A desktop setting a project cannot override is not listed there.
        let (source, key) = self.selected;
        if let Some(setting) = source.setting(key)
            && !source.listed(which, setting)
            && let Some(first) = source.schema().iter().find(|setting| {
                setting.category == self.category.1 && source.listed(which, setting)
            })
        {
            self.selected = (source, first.key);
        }
        cx.notify();
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let searching = !self.search.read(cx).content().trim().is_empty();
        let source = self.selected.0;
        let changed = self
            .file(source)
            .map_or(0, |file| file.changed(source.schema()).len());
        let path = match self.which {
            Which::User => self
                .file(source)
                .map(|file| file.path.as_path())
                .filter(|path| !path.as_os_str().is_empty())
                .map_or_else(|| "no settings folder".into(), tilde),
            Which::Project => source.project_file().to_owned(),
        };
        let project_label = match &self.project_root {
            Some(root) => format!("Project  {}", tilde(root)),
            None => "Project".into(),
        };
        let rows = self.rows(cx);
        let category_empty = !searching
            && self.category.0 == Source::Desktop
            && self.which == Which::Project
            && rows.is_empty();
        let mut categories = v_flex()
            .id("settings-categories")
            .w(px(180.))
            .h_full()
            .flex_shrink_0()
            .overflow_y_scroll()
            .p(px(8.))
            .gap(px(2.))
            .border_r_1()
            .border_color(theme.hover);
        for group in SOURCES {
            categories = categories.child(
                div()
                    .px(px(14.))
                    .pt(px(if group == Source::Pi { 4. } else { 14. }))
                    .pb(px(4.))
                    .child(label(group.group(), theme)),
            );
            for (i, category) in group.categories().into_iter().enumerate() {
                categories = categories.child(self.category_row((group, category), i, cx, theme));
            }
        }
        v_flex()
            .id("settings-view")
            .debug_selector(|| "settings-view".into())
            .size_full()
            .bg(theme.canvas)
            .child(
                h_flex()
                    .h(px(46.))
                    .flex_shrink_0()
                    .px(px(20.))
                    .gap(px(12.))
                    .border_b_1()
                    .border_color(theme.hover)
                    .child(segments(
                        [
                            segment("settings-user", "User", self.which == Which::User, theme)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.set_scope(Which::User, cx)),
                                ),
                            segment(
                                "settings-project",
                                project_label,
                                self.which == Which::Project,
                                theme,
                            )
                            .debug_selector(|| "settings-project".into())
                            .on_click(
                                cx.listener(|this, _, _, cx| this.set_scope(Which::Project, cx)),
                            ),
                        ],
                        theme,
                    ))
                    .child(div().flex_1())
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(MONO)
                            .text_size(px(10.5))
                            .text_color(theme.faint)
                            .child(format!("{changed} changed · {path}")),
                    )
                    .child(
                        button("open-settings-json", "Open JSON", theme)
                            .debug_selector(|| "open-settings-json".into())
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.open_json(source, cx)),
                            ),
                    ),
            )
            .when_some(self.error.clone(), |view, error| {
                view.child(
                    div()
                        .px(px(20.))
                        .py(px(6.))
                        .text_size(px(11.))
                        .text_color(theme.coral)
                        .child(error),
                )
            })
            .when_some(
                self.file(source).and_then(|file| file.error.clone()),
                |view, error| {
                    view.child(
                        div()
                            .px(px(20.))
                            .py(px(6.))
                            .text_size(px(11.))
                            .text_color(theme.coral)
                            .child(format!(
                                "{error}. Settings are read-only until it is fixed."
                            )),
                    )
                },
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(categories)
                    .child(
                        v_flex()
                            .id("settings-rows")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_y_scroll()
                            .px(px(12.))
                            .py(px(10.))
                            .child(div().px(px(8.)).pb(px(6.)).child(label(
                                if searching {
                                    "MATCHING SETTINGS".to_owned()
                                } else {
                                    self.category.1.to_uppercase()
                                },
                                theme,
                            )))
                            .when(
                                self.which == Which::Project && self.project_root.is_none(),
                                |rows| {
                                    rows.child(
                                        note("Select a project to edit its settings.", theme)
                                            .px(px(8.)),
                                    )
                                },
                            )
                            .when(category_empty, |list| {
                                list.child(
                                    note(
                                        "These apply to every project. Change them under User.",
                                        theme,
                                    )
                                    .px(px(8.)),
                                )
                            })
                            .when(rows.is_empty() && !category_empty, |list| {
                                list.child(note("No settings match.", theme).px(px(8.)))
                            })
                            .children(
                                rows.into_iter()
                                    .map(|(source, setting)| self.row(source, setting, cx, theme)),
                            ),
                    ),
            )
    }
}

fn tilde(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}
