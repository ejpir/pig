//! Project-scope study 04. All data belongs to an already running Pi session.
use super::*;
use pi_core::protocol::{Package, ProjectTrust};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ResourceTab {
    Packages,
    Extensions,
    Skills,
    Prompts,
    Context,
}
impl ResourceTab {
    fn title(self) -> &'static str {
        match self {
            Self::Packages => "Packages",
            Self::Extensions => "Extensions",
            Self::Skills => "Skills",
            Self::Prompts => "Prompts",
            Self::Context => "Context files",
        }
    }
}
#[derive(Clone)]
pub(super) struct Resource {
    pub key: String,
    title: String,
    description: String,
    scope: String,
    path: Option<String>,
    source: String,
    commands: Vec<String>,
    package: Option<Package>,
    status: Option<String>,
}
fn rows(session: &Session, tab: ResourceTab) -> Vec<Resource> {
    if tab == ResourceTab::Extensions
        && let Some(extensions) = session
            .project_trust
            .as_ref()
            .and_then(|trust| trust.loaded_extensions.as_ref())
    {
        return extensions
            .iter()
            .map(|extension| Resource {
                key: format!("extension:{}", extension.path),
                title: extension.path.clone(),
                description: if extension.status == "load-error" {
                    extension
                        .error
                        .clone()
                        .unwrap_or_else(|| "Details not reported".into())
                } else if extension.commands.is_empty() {
                    "No registered commands".into()
                } else {
                    format!("adds {}", extension.commands.join(", "))
                },
                scope: extension
                    .source_info
                    .as_ref()
                    .map(|info| info.scope.clone())
                    .unwrap_or_else(|| "Not reported".into()),
                path: Some(extension.path.clone()),
                source: extension
                    .source_info
                    .as_ref()
                    .map(|info| info.source.clone())
                    .unwrap_or_else(|| "Not reported".into()),
                commands: extension.commands.clone(),
                package: None,
                status: Some(extension.status.clone()),
            })
            .collect();
    }
    if tab == ResourceTab::Context {
        return session
            .project_trust
            .as_ref()
            .and_then(|trust| trust.context_files.as_ref())
            .into_iter()
            .flatten()
            .map(|path| Resource {
                key: format!("context:{path}"),
                title: path.clone(),
                description: "SDK-reported context path; per-file scope not reported".into(),
                scope: "Not reported".into(),
                path: Some(path.clone()),
                source: "SDK context files".into(),
                commands: vec![],
                package: None,
                status: Some("loaded".into()),
            })
            .collect();
    }
    if tab == ResourceTab::Packages {
        return session
            .packages
            .as_ref()
            .into_iter()
            .flatten()
            .map(|package| Resource {
                key: format!("{}:{}", package.scope, package.source),
                title: package.source.clone(),
                description: if package.filtered {
                    "Configured · selected resources only"
                } else {
                    "Configured package"
                }
                .into(),
                scope: package.scope.clone(),
                path: package.installed_path.clone(),
                source: package.source.clone(),
                commands: session
                    .commands
                    .iter()
                    .filter(|command| {
                        command.source_info.as_ref().is_some_and(|info| {
                            info.origin == "package"
                                && info.source == package.source
                                && info.scope == package.scope
                        })
                    })
                    .map(|command| format!("/{}", command.name))
                    .collect(),
                package: Some(package.clone()),
                status: None,
            })
            .collect();
    }
    let kind = match tab {
        ResourceTab::Extensions => "extension",
        ResourceTab::Skills => "skill",
        ResourceTab::Prompts => "prompt",
        _ => return vec![],
    };
    let mut rows = BTreeMap::<String, Resource>::new();
    for command in session
        .commands
        .iter()
        .filter(|command| command.source == kind)
    {
        let info = command.source_info.as_ref();
        let id = info
            .map(|info| format!("{}:{}", info.scope, info.path))
            .unwrap_or_else(|| format!("command:{}", command.name));
        let row = rows.entry(id.clone()).or_insert_with(|| Resource {
            key: id,
            title: if tab == ResourceTab::Extensions {
                info.map(|info| info.path.clone())
                    .unwrap_or_else(|| format!("/{}", command.name))
            } else {
                command.name.trim_start_matches("skill:").to_owned()
            },
            description: command
                .description
                .clone()
                .unwrap_or_else(|| "Reported command".into()),
            scope: info
                .map(|info| info.scope.clone())
                .unwrap_or_else(|| "Not reported".into()),
            path: info.map(|info| info.path.clone()),
            source: info
                .map(|info| info.source.clone())
                .unwrap_or_else(|| "Not reported".into()),
            commands: vec![],
            package: None,
            status: None,
        });
        row.commands.push(format!("/{}", command.name));
    }
    rows.into_values().collect()
}
fn scoped(session: &Session, tab: ResourceTab, project: bool) -> Vec<Resource> {
    rows(session, tab)
        .into_iter()
        .filter(|row| {
            row.scope == if project { "project" } else { "user" } || row.scope == "Not reported"
        })
        .collect()
}
fn reported(session: &Session, tab: ResourceTab) -> bool {
    match tab {
        ResourceTab::Packages => session.packages.is_some(),
        ResourceTab::Context => session
            .project_trust
            .as_ref()
            .is_some_and(|trust| trust.context_files.is_some()),
        ResourceTab::Extensions => {
            session.commands_loaded
                || session
                    .project_trust
                    .as_ref()
                    .is_some_and(|trust| trust.loaded_extensions.is_some())
        }
        _ => session.commands_loaded,
    }
}
fn trust_pending(trust: &ProjectTrust) -> bool {
    trust
        .saved_decision
        .as_ref()
        .is_some_and(|decision| decision.decision != trust.trusted)
}
fn compact(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl CatalogView {
    pub(super) fn project_resources(&mut self, query: &str, cx: &App) {
        self.resources = self
            .model(cx)
            .map(|session| scoped(session, self.resource_tab, self.resource_project))
            .unwrap_or_default()
            .into_iter()
            .filter(|row| {
                format!(
                    "{} {} {} {}",
                    row.title,
                    row.description,
                    row.scope,
                    row.path.as_deref().unwrap_or("")
                )
                .to_lowercase()
                .contains(query)
            })
            .collect();
    }
    fn settings_text(&self, cx: &App) -> Option<String> {
        let trust = self.model(cx)?.project_trust.as_ref()?;
        let settings = if self.resource_project {
            trust.project_settings.as_ref()?
        } else {
            trust.user_settings.as_ref()?
        };
        serde_json::to_string_pretty(settings).ok()
    }
    pub(super) fn sync_settings(&mut self, cx: &mut Context<Self>) {
        if !self.resource_project || self.resource_tab != ResourceTab::Extensions {
            self.settings_documents.sync(vec![], cx);
            self._settings_subscription = None;
            return;
        }
        let text = self.settings_text(cx).unwrap_or_default();
        let mut shown: String = text.chars().take(4096).collect();
        if shown.len() < text.len() {
            shown.push_str("\n// Preview truncated. Copy retains all reported keys.");
        }
        let key: SharedString = "resource-settings".into();
        self.settings_documents.update(
            vec![(
                key.clone(),
                crate::markdown_view::Source::Code {
                    text: &shown,
                    language: "json",
                },
            )],
            cx,
        );
        if self._settings_subscription.is_none()
            && let Some(document) = self.settings_documents.get(&key)
        {
            self._settings_subscription = Some(cx.observe(document, |_, _, cx| cx.notify()));
        }
    }
    pub(super) fn resources_screen(
        &self,
        window: &mut Window,
        cx: &Context<Self>,
        theme: Theme,
    ) -> AnyElement {
        let weak = cx.entity().downgrade();
        let count = self.resources.len();
        v_flex().id("resources-screen").debug_selector(|| "resources-screen".into())
            .track_focus(&self.focus).relative().size_full().min_w_0().overflow_hidden()
            .on_action(cx.listener(|this, _: &super::super::super::Stop, window, cx| {
                cx.stop_propagation(); this.project_picker = false; this.installing = false; this.focus.focus(window, cx); cx.notify();
            }))
            .child(self.resources_header(cx, theme))
            .child(v_flex().id("resources-body").flex_1().min_h_0().overflow_y_scroll().pt(px(12.))
                .when(count > 0, |body| body.child(gpui::uniform_list("catalog-list", count, move |range, _, cx| {
                    weak.update(cx, |this, cx| range.map(|index| this.resource_row(index, cx, theme)).collect()).unwrap_or_default()
                }).track_scroll(&self.scroll).h(px((count.min(6) * 52) as f32)).flex_shrink_0()))
                .when(count == 0, |body| body.child(div().mx(px(20.)).py(px(14.)).text_size(px(12.)).text_color(theme.muted)
                    .child(if self.controller.is_none() { "No active Pi process reports this scope. Choosing a project never starts a process." }
                        else if self.model(cx).is_none_or(|session| !reported(session, self.resource_tab)) { "This catalog has not been reported by the backend." }
                        else { "No matching resources reported in this scope." })))
                .when(self.resource_project && self.resource_tab == ResourceTab::Extensions, |body| body.child(self.project_settings(window, cx, theme)))
                .when(self.resource_project, |body| body.child(h_flex().mx(px(20.)).mt(px(14.)).gap(px(7.)).items_start().text_size(px(11.)).text_color(theme.faint)
                    .child(icon("info", theme.faint).size(px(12.)))
                    .child("Context files can load without project trust.")))
                .child(div().h(px(20.)).flex_shrink_0()))
            .when_some(self.toast(cx, theme), |view, toast| view.child(toast))
            .when(self.installing, |view| view.child(self.install_popup(cx, theme)))
            .into_any_element()
    }
    pub(super) fn resources_header(&self, cx: &Context<Self>, theme: Theme) -> AnyElement {
        let workspace = self.workspace.read(cx);
        let path = workspace.selected_project.as_ref();
        let name = path
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Choose project".into());
        let trust = self
            .model(cx)
            .and_then(|session| session.project_trust.as_ref());
        v_flex().px(px(20.)).pt(px(12.)).flex_shrink_0()
            .child(h_flex().h(px(28.)).gap(px(12.))
                .child(segments([false, true].into_iter().map(|project| segment(("resource-scope", project as usize), if project { "Project" } else { "User" }, self.resource_project == project, theme)
                    .debug_selector(move || if project { "resource-scope-project".into() } else { "resource-scope-user".into() })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.resource_project = project; this.local = project; this.selected = None; this.project_picker = false; this.bind(cx); this.project(cx);
                    }))), theme))
                .when(self.resource_project, |row| row.child(h_flex().id("resource-project-picker").debug_selector(|| "resource-project-picker".into())
                    .h(px(28.)).w(px(240.)).min_w_0().gap(px(8.)).px(px(10.)).rounded(px(6.))
                    .bg(theme.canvas).border_1().border_color(theme.focus).cursor_pointer()
                    .aria_expanded(self.project_picker).tooltip(ui::Tooltip::text(path.map(|path| path.display().to_string()).unwrap_or_default()))
                    .child(icon("folder", theme.secondary).size(px(14.)))
                    .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(name))
                    .child(div().flex_1().min_w_0().truncate().font_family(MONO).text_size(px(10.)).text_color(theme.faint).child(path.map(|path| short_path(&path.display().to_string())).unwrap_or_default()))
                    .child(icon(if self.project_picker { "chevron_up" } else { "chevron_down" }, theme.faint).size(px(11.)))
                    .on_hover(cx.listener(|this, hovered: &bool, _, _| this.project_picker_hovered = *hovered))
                    .on_click(cx.listener(|this, _, window, cx| { this.project_picker = !this.project_picker; this.focus.focus(window, cx); cx.notify(); }))))
                .child(div().flex_1())
                .child(div().id("resource-settings-source").truncate().font_family(MONO).text_size(px(10.)).text_color(theme.faint)
                    .tooltip(ui::Tooltip::text("Only effective resource/model keys reported by Pi are shown, not a complete settings-file manifest."))
                    .child(if self.resource_project { "Project scope" } else { "User scope" })))
            .when(self.resource_project, |header| header.child(h_flex().debug_selector(|| "resources-trust-strip".into())
                .h(px(40.)).mt(px(16.)).px(px(12.)).gap(px(8.)).rounded(px(8.)).bg(theme.panel)
                .border_1().border_color(theme.chip_line)
                .child(icon("shield", if trust.is_some_and(|trust| trust.trusted) { theme.green } else { theme.amber }).size(px(15.)))
                .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(match trust { Some(trust) if trust.trusted => "Trusted", Some(_) => "Not trusted", None => "Trust not reported" }))
                .child(div().id("resource-trust-detail").flex_1().min_w_0().truncate().text_size(px(11.)).text_color(theme.faint)
                    .when_some(trust, |label, trust| label.tooltip(ui::Tooltip::text(trust_description(trust)))
                        .debug_selector(move || if trust_pending(trust) { "trust-decision-pending".into() } else { "trust-decision-active".into() })
                        .child(if trust_pending(trust) { "saved decision differs · restart to apply" }
                            else if trust.saved_decision.is_some() { "saved decision active in this process" }
                            else { "effective in this Pi process" })))
                .when(self.mutable(cx) && trust.is_some(), |row| row.child(div().id("resources-trust-action").cursor_pointer().font_family(MONO).text_size(px(10.))
                    .text_color(theme.accent).child(if trust.is_some_and(|trust| trust.trusted) { "[ REVOKE ]" } else { "[ TRUST ]" })
                    .tooltip(ui::Tooltip::text("Save a project trust decision · affects the next process restart, not an OS sandbox"))
                    .on_click(cx.listener(|this, _, window, cx| this.confirm_trust(window, cx)))))))
            .child(h_flex().h(px(30.)).mt(px(12.)).gap(px(22.)).border_b_1().border_color(theme.line)
                .children([ResourceTab::Packages, ResourceTab::Extensions, ResourceTab::Skills, ResourceTab::Prompts, ResourceTab::Context].into_iter().map(|tab| {
                    let count = self.model(cx).filter(|session| reported(session, tab)).map(|session| scoped(session, tab, self.resource_project).len().to_string()).unwrap_or_else(|| "—".into());
                    let selected = tab == self.resource_tab;
                    h_flex().id(keyed("resource-tab", tab.title())).debug_selector(move || format!("resource-tab-{}", tab.title()))
                        .h_full().border_b_2().border_color(if selected { theme.accent } else { gpui::transparent_black() })
                        .cursor_pointer().child(label(format!("{}  {count}", tab.title().to_uppercase()), theme).text_size(px(9.)).when(selected, |label| label.text_color(theme.text)))
                        .tooltip(ui::Tooltip::text("Reported User/Project scopes only. CLI/built-in resources are not assigned to this folder; unscoped entries retain their reported metadata."))
                        .on_click(cx.listener(move |this, _, _, cx| { this.resource_tab = tab; this.selected = None; this.project(cx); }))
                })))
            .when(self.project_picker, |header| header.child(self.project_popup(cx, theme)))
            .into_any_element()
    }
    fn project_popup(&self, cx: &Context<Self>, theme: Theme) -> AnyElement {
        gpui::deferred(
            v_flex()
                .id("resource-projects")
                .debug_selector(|| "resource-projects".into())
                .occlude()
                .absolute()
                .top(px(44.))
                .left(px(144.))
                .right(px(20.))
                .max_w(px(400.))
                .max_h(px(260.))
                .overflow_y_scroll()
                .rounded(px(9.))
                .bg(theme.panel)
                .border_1()
                .border_color(theme.line)
                .shadow_lg()
                .py(px(10.))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    if !this.project_picker_hovered {
                        this.project_picker = false;
                        cx.notify();
                    }
                }))
                .child(label("PROJECT SCOPE", theme).mx(px(16.)).mb(px(6.)))
                .children(self.workspace.read(cx).projects.iter().enumerate().map(
                    |(index, path)| {
                        let project = path.clone();
                        let owner = self
                            .workspace
                            .read(cx)
                            .tabs
                            .iter()
                            .find(|tab| tab.controller.read(cx).model().cwd == *path)
                            .map(|tab| tab.controller.read(cx).model());
                        let trust = owner.and_then(|session| session.project_trust.as_ref());
                        let subtitle = owner.map(|session| {
                            if reported(session, ResourceTab::Packages)
                                && reported(session, ResourceTab::Extensions)
                            {
                                format!(
                                    "{} packages · {} extensions",
                                    scoped(session, ResourceTab::Packages, true).len(),
                                    scoped(session, ResourceTab::Extensions, true).len()
                                )
                            } else {
                                "Counts not reported".into()
                            }
                        });
                        h_flex()
                            .id(("resource-project", index))
                            .debug_selector(move || format!("resource-project-{index}"))
                            .mx(px(4.))
                            .px(px(10.))
                            .py(px(7.))
                            .gap(px(10.))
                            .rounded(px(5.))
                            .cursor_pointer()
                            .when(
                                self.workspace.read(cx).selected_project.as_ref() == Some(path),
                                |row| row.bg(theme.selected),
                            )
                            .hover(move |row| row.bg(theme.hover))
                            .child(icon("folder", theme.muted).size(px(14.)))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(2.))
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(
                                                path.file_name()
                                                    .unwrap_or(path.as_os_str())
                                                    .to_string_lossy()
                                                    .into_owned(),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .font_family(MONO)
                                            .text_size(px(10.))
                                            .text_color(theme.faint)
                                            .child(short_path(&path.display().to_string())),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .items_end()
                                    .gap(px(2.))
                                    .text_size(px(10.))
                                    .text_color(theme.faint)
                                    .child(match trust {
                                        Some(trust) if trust.trusted => "Trusted",
                                        Some(_) => "Not trusted",
                                        None => "Not reported",
                                    })
                                    .child(
                                        subtitle
                                            .unwrap_or_else(|| "No active resource report".into()),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.project_picker = false;
                                this.workspace.update(cx, |workspace, cx| {
                                    workspace.select_project(project.clone(), cx)
                                });
                                this.bind(cx);
                                this.project(cx);
                            }))
                    },
                ))
                .child(
                    div()
                        .mx(px(16.))
                        .mt(px(5.))
                        .text_size(px(10.))
                        .text_color(theme.faint)
                        .child("Trust is saved per folder. Scope changes do not start Pi."),
                ),
        )
        .with_priority(1)
        .into_any_element()
    }
    pub(super) fn project_settings(
        &self,
        window: &mut Window,
        cx: &Context<Self>,
        theme: Theme,
    ) -> AnyElement {
        let text = self.settings_text(cx);
        v_flex().mx(px(20.)).mt(px(16.)).gap(px(4.)).flex_shrink_0()
            .child(h_flex().child(label(if self.resource_project { "PROJECT SETTINGS" } else { "USER SETTINGS" }, theme)).child(div().flex_1())
                .when_some(text.clone(), |row, text| row.child(icon_button("copy-project-settings", "copy", "Copy reported settings", theme)
                    .size(px(18.)).tooltip(ui::Tooltip::text("Copy reported settings keys"))
                    .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))))))
            .child(div().id("project-settings-preview").debug_selector(|| "project-settings-preview".into())
                .min_h(px(104.)).max_h(px(140.)).overflow_y_scroll().rounded(px(6.)).bg(theme.deep).border_1().border_color(theme.line)
                .when(text.is_none(), |view| view.p(px(12.)).text_size(px(11.)).text_color(theme.faint).child("Settings values are not reported by this backend."))
                .when_some(text, |view, _| view.when_some(self.settings_documents.get(&SharedString::from("resource-settings")), |view, document| {
                    view.child(crate::markdown_view::element(document, crate::markdown_view::output_style(theme, window, cx)))
                })))
            .child(body_text(if self.resource_project { "Loaded after trust. Reported project values override user settings for this folder." } else { "Effective user resource/model keys reported by Pi; not the complete settings file." }, theme).mt(px(6.)))
            .into_any_element()
    }
    pub(super) fn resource_row(
        &self,
        index: usize,
        cx: &Context<Self>,
        theme: Theme,
    ) -> AnyElement {
        let row = &self.resources[index];
        let id = row.key.clone();
        let title = if row.path.as_ref() == Some(&row.title) {
            self.model(cx)
                .and_then(|session| {
                    std::path::Path::new(&row.title)
                        .strip_prefix(&session.cwd)
                        .ok()
                })
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| row.title.clone())
        } else {
            row.title.clone()
        };
        let status = row
            .status
            .as_deref()
            .map(|status| match status {
                "loaded" => "loaded",
                "load-error" => "1 error at load",
                _ => status,
            })
            .unwrap_or(if row.package.is_some() {
                if row.path.is_some() {
                    "installed"
                } else {
                    "not installed"
                }
            } else {
                "reported"
            });
        h_flex()
            .id(("resource-row", index))
            .debug_selector(move || format!("resource-row-{index}"))
            .h(px(52.))
            .w_full()
            .px(px(20.))
            .gap(px(12.))
            .cursor_pointer()
            .when(self.selected.as_ref() == Some(&id), |row| {
                row.bg(theme.selected)
            })
            .hover(move |row| row.bg(theme.hover))
            .tooltip(ui::Tooltip::text(format!(
                "{}\n{}\n{}\n{}",
                row.title,
                row.description,
                row.scope,
                row.path.as_deref().unwrap_or("Path not reported")
            )))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = Some(id.clone());
                cx.notify();
            }))
            .child(
                icon(
                    if self.resource_tab == ResourceTab::Packages {
                        "box"
                    } else {
                        "file"
                    },
                    theme.muted,
                )
                .size(px(15.)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(theme.line)
                    .child(
                        div()
                            .truncate()
                            .font_family(MONO)
                            .text_size(px(12.))
                            .text_color(theme.secondary)
                            .child(title),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(theme.faint)
                            .child(format!("{} · {}", row.scope, compact(&row.description))),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(if row.status.as_deref() == Some("load-error") {
                        theme.coral
                    } else {
                        theme.faint
                    })
                    .child(status.to_owned()),
            )
            .into_any_element()
    }
    fn confirm_trust(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let trusted = self
            .model(cx)
            .and_then(|session| session.project_trust.as_ref())
            .is_some_and(|trust| trust.trusted);
        self.confirm(Command::SetProjectTrust { choice: if trusted { "distrust" } else { "trust" }.into() },
            if trusted { "Revoke project trust?" } else { "Trust this project?" },
            if trusted { "Save distrust for this folder. Project settings, extensions and packages stop loading after the Pi process restarts. Loaded code is not revoked or sandboxed; context files can still load." }
            else { "Allow this folder's project settings, extensions and packages to run with your OS permissions on the next Pi process restart? This does not grant editor/LSP trust or per-extension sandbox permissions." }, window, cx);
    }
    fn project_details(&self, cx: &Context<Self>, theme: Theme) -> AnyElement {
        let workspace = self.workspace.read(cx);
        let path = workspace.selected_project.as_ref();
        let trust = self
            .model(cx)
            .and_then(|session| session.project_trust.as_ref());
        let title = if self.resource_project {
            path.and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Project".into())
        } else {
            "User".into()
        };
        let count = workspace
            .tabs
            .iter()
            .filter(|tab| Some(&tab.controller.read(cx).model().cwd) == path)
            .count();
        let working = workspace
            .tabs
            .iter()
            .filter(|tab| {
                Some(&tab.controller.read(cx).model().cwd) == path
                    && tab.controller.read(cx).working()
            })
            .count();
        v_flex().debug_selector(|| "resource-project-inspector".into()).gap(px(8.))
            .child(div().font_family(SERIF).italic().text_size(px(24.)).child(title))
            .child(h_flex().gap(px(8.)).text_size(px(12.)).child(div().size(px(6.)).rounded_full().bg(if self.resource_project && trust.is_some_and(|trust| trust.trusted) { theme.green } else { theme.faint }))
                .child(if !self.resource_project { "Pi user scope" } else { match trust { Some(trust) if trust.trusted => "Trusted", Some(_) => "Not trusted", None => "Trust not reported" } }))
            .when(self.resource_project, |view| view.child(divider(theme)).child(section("PROJECT", "", theme)).child(detail("Path", path.map(|path| short_path(&path.display().to_string())).unwrap_or_else(|| "Not selected".into()), false, theme))
                .child(detail("Branch", "Not reported", false, theme))
                .child(detail("Open sessions", count.to_string(), false, theme)).child(detail("Working now", working.to_string(), false, theme)))
            .child(divider(theme)).child(section("REPORTED RESOURCES", "", theme))
            .children([ResourceTab::Packages, ResourceTab::Extensions, ResourceTab::Skills, ResourceTab::Prompts, ResourceTab::Context].into_iter().map(|tab| {
                detail(tab.title(), self.model(cx).filter(|session| reported(session, tab)).map(|session| scoped(session, tab, self.resource_project).len().to_string()).unwrap_or_else(|| "Not reported".into()), false, theme)
            }))
            .when(self.resource_project, |view| view.child(divider(theme)).child(section("TRUST", "", theme))
                .child(body_text(trust.map(trust_description).unwrap_or_else(|| "Trust has not been reported by this project's Pi process.".into()), theme))
                .child(button("resources-revoke", if trust.is_some_and(|trust| trust.trusted) { "Revoke trust…" } else { "Trust project…" }, theme)
                    .w_full().text_color(theme.coral).bg(theme.danger).border_color(theme.danger_line)
                    .when(!self.mutable(cx) || trust.is_none(), |button| button.opacity(0.5))
                    .when(self.mutable(cx) && trust.is_some(), |button| button.on_click(cx.listener(|this, _, window, cx| this.confirm_trust(window, cx)))))
                .child(body_text("Revoking stops project code loading in new processes. It does not unload or sandbox code already running.", theme).mt(px(12.))))
            .child(body_text("Pi has no per-extension filesystem/network permission grants. Inspect source before loading code.", theme).mt(px(12.)))
            .child(button("resources-reload", "Reload resources…", theme).when(!self.mutable(cx), |button| button.opacity(0.5))
                .when(self.mutable(cx), |button| button.on_click(cx.listener(|this, _, window, cx| {
                    this.confirm(Command::Reload, "Reload resources?", "Load configured extensions and packages into this Pi process? They execute with your permissions. Saved trust changes still require a restart.", window, cx);
                }))))
            .into_any_element()
    }
    pub(super) fn resource_details(&self, cx: &Context<Self>, theme: Theme) -> AnyElement {
        let Some(row) = self
            .resources
            .iter()
            .find(|row| self.selected.as_ref() == Some(&row.key))
            .cloned()
        else {
            return self.project_details(cx, theme);
        };
        let copied = format!(
            "{}\n{}\nScope: {}\nSource: {}\nPath: {}\nCommands: {}",
            row.title,
            row.description,
            row.scope,
            row.source,
            row.path.as_deref().unwrap_or("Not reported"),
            row.commands.join(", ")
        );
        let enabled = self.mutable(cx);
        v_flex().gap(px(8.))
            .child(h_flex().gap(px(8.)).child(div().flex_1().min_w_0().text_size(px(14.)).child(row.title.clone()))
                .child(icon_button("resource-copy", "copy", "Copy resource details", theme).debug_selector(|| "resource-copy".into()).size(px(20.))
                    .tooltip(ui::Tooltip::text("Copy original resource metadata"))
                    .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copied.clone())))))
            .child(body_text(row.description, theme)).child(divider(theme)).child(section("SOURCE", "", theme))
            .child(detail("Scope", row.scope, false, theme)).child(body_text(row.source, theme).font_family(MONO))
            .child(body_text(row.path.unwrap_or_else(|| "Path not reported".into()), theme).font_family(MONO))
            .child(divider(theme)).child(section("REPORTED COMMANDS", "", theme))
            .child(body_text(if row.commands.is_empty() { "No command association reported".into() } else { row.commands.join(", ") }, theme))
            .when_some(row.package, |view, package| {
                let update = package.source.clone(); let remove = package.source.clone(); let local = package.scope == "project";
                let allowed = enabled && matches!(package.scope.as_str(), "project" | "user") && (!local || self.model(cx).and_then(|session| session.project_trust.as_ref()).is_some_and(|trust| trust.trusted));
                view.child(button("resource-update", "Update…", theme).when(!allowed, |b| b.opacity(0.5)).when(allowed, |b| b.on_click(cx.listener(move |this, _, window, cx| {
                    this.confirm(Command::UpdatePackages { source: update.clone() }, "Update package?", "Download dependencies and execute package code? Pi reconciles this source wherever configured; pinned versions stay pinned.", window, cx);
                }))))
                .child(button("resource-remove", "Remove…", theme).text_color(theme.coral).when(!allowed, |b| b.opacity(0.5)).when(allowed, |b| b.on_click(cx.listener(move |this, _, window, cx| {
                    this.confirm(Command::RemovePackage { source: remove.clone(), local }, "Remove package?", "Remove this source from its settings scope and uninstall managed files? Loaded resources remain until explicit reload or restart.", window, cx);
                }))))
            })
            .child(body_text("No automatic update checks. Pi project trust is not a per-extension OS sandbox.", theme))
            .child(button("resource-back-project", "Back to scope", theme).on_click(cx.listener(|this, _, _, cx| { this.selected = None; cx.notify(); })))
            .into_any_element()
    }
    fn install_popup(&self, cx: &Context<Self>, theme: Theme) -> AnyElement {
        gpui::deferred(
            v_flex()
                .id("resource-install-popup")
                .debug_selector(|| "resource-install-popup".into())
                .occlude()
                .absolute()
                .top(px(8.))
                .left(px(20.))
                .right(px(20.))
                .p(px(16.))
                .gap(px(12.))
                .bg(theme.panel)
                .border_1()
                .border_color(theme.line)
                .rounded(px(8.))
                .shadow_lg()
                .on_action(
                    cx.listener(|this, _: &super::super::super::Stop, window, cx| {
                        cx.stop_propagation();
                        this.installing = false;
                        this.focus.focus(window, cx);
                        cx.notify();
                    }),
                )
                .on_action(
                    cx.listener(|_, _: &super::super::super::Submit, _, cx| cx.stop_propagation()),
                )
                .child(
                    h_flex()
                        .child(label("INSTALL PACKAGE", theme))
                        .child(div().flex_1())
                        .child(
                            icon_button(
                                "cancel-resource-install",
                                "close",
                                "Cancel package entry",
                                theme,
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.installing = false;
                                    this.focus.focus(window, cx);
                                    cx.notify();
                                },
                            )),
                        ),
                )
                .child(self.resources_footer(cx, theme)),
        )
        .with_priority(2)
        .into_any_element()
    }
    pub(super) fn resources_footer(&self, cx: &Context<Self>, theme: Theme) -> AnyElement {
        let allowed = self.mutable(cx)
            && !self.install.read(cx).content().trim().is_empty()
            && (!self.local
                || self
                    .model(cx)
                    .and_then(|session| session.project_trust.as_ref())
                    .is_some_and(|trust| trust.trusted));
        v_flex().gap(px(8.)).child(h_flex().gap(px(8.)).flex_wrap()
            .child(div().flex_1().min_w(px(160.)).p(px(6.)).border_1().border_color(theme.chip_line).rounded(px(5.)).child(self.install.clone()))
            .child(segments([false, true].into_iter().map(|local| segment(if local { "install-project" } else { "install-user" }, if local { "Project" } else { "User" }, self.local == local, theme)
                .on_click(cx.listener(move |this, _, _, cx| { this.local = local; cx.notify(); }))), theme))
            .child(primary_button("resource-install", "Install…", allowed, theme).debug_selector(|| "resource-install".into())
                .when(allowed, |button| button.on_click(cx.listener(|this, _, window, cx| {
                    let source = this.install.read(cx).content().trim().to_owned();
                    this.confirm(Command::InstallPackage { source: source.clone(), local: this.local }, "Install package?",
                        &format!("Install {source} into {} settings? It may download dependencies and execute code with your permissions. Reload remains explicit.", if this.local { "project" } else { "user" }), window, cx);
                })))))
            .child(body_text("Installation can execute code. Project installs require effective trust; every install asks for confirmation.", theme)).into_any_element()
    }
}
fn trust_description(trust: &ProjectTrust) -> String {
    let effective = if trust.trusted {
        "Trusted in this Pi process."
    } else {
        "Not trusted in this Pi process."
    };
    match &trust.saved_decision {
        Some(decision) => format!(
            "{effective} Saved {} for {}. {}",
            if decision.decision {
                "trust"
            } else {
                "distrust"
            },
            decision.path,
            if trust_pending(trust) {
                "Restart to apply the saved decision."
            } else {
                "Saved decision is active."
            }
        ),
        None => format!("{effective} No saved decision reported."),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn extension_commands_group_by_reported_path_and_scope() {
        let mut session = Session::default();
        session.apply(&json!({"type":"response","command":"get_commands","success":true,"data":{"commands":[
            {"name":"one","source":"extension","sourceInfo":{"path":"/x/ext.ts","scope":"user","source":"local","origin":"top-level"}},
            {"name":"two","source":"extension","sourceInfo":{"path":"/x/ext.ts","scope":"user","source":"local","origin":"top-level"}},
            {"name":"three","source":"extension","sourceInfo":{"path":"/x/ext.ts","scope":"project","source":"local","origin":"top-level"}}
        ]}})).unwrap();
        let resources = rows(&session, ResourceTab::Extensions);
        assert_eq!(resources.len(), 2);
        assert_eq!(resources.iter().map(|r| r.commands.len()).sum::<usize>(), 3);
        assert!(rows(&session, ResourceTab::Context).is_empty());
    }
}
