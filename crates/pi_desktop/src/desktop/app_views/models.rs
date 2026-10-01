use super::*;
use pi_core::protocol::Model;
use std::collections::BTreeSet;

pub(super) fn key(model: &Model) -> String {
    format!("{}/{}", model.provider, model.id)
}
fn number(n: Option<u64>) -> String {
    match n {
        Some(n) if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1_000_000.),
        Some(n) if n >= 1_000 => format!("{}k", n / 1_000),
        Some(n) if n > 0 => n.to_string(),
        _ => "—".into(),
    }
}
fn price(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite() && *value >= 0.)
        .map_or_else(|| "—".into(), |value| format!("${value:.2}"))
}
fn cycle(session: &Session, model: &Model) -> Option<bool> {
    let scope = session.settings.as_ref()?["scopedModels"].as_array()?;
    Some(scope.is_empty() || scope.iter().any(|item| item.as_str() == Some(&key(model))))
}
fn auth(session: &Session, provider: &str) -> String {
    let Some(providers) = &session.auth_providers else {
        return "Auth not reported".into();
    };
    let entries: Vec<_> = providers
        .iter()
        .filter(|entry| entry.id == provider)
        .collect();
    if let Some(status) = entries.iter().find_map(|entry| entry.status.as_ref()) {
        format!(
            "{} · {}",
            if status.kind == "oauth" {
                "OAuth"
            } else {
                "API key"
            },
            status.source
        )
    } else if entries.is_empty() {
        "Auth not reported".into()
    } else {
        "Not configured".into()
    }
}
impl CatalogView {
    pub(super) fn project_models(&mut self, query: &str, cx: &App) {
        self.models = self
            .model(cx)
            .map(|session| {
                session
                    .available_models
                    .iter()
                    .filter(|model| self.provider.as_ref().is_none_or(|p| p == &model.provider))
                    .filter(|model| match self.model_filter {
                        1 => cycle(session, model) == Some(true),
                        2 => model.reasoning == Some(true),
                        3 => model.input.iter().any(|kind| kind == "image"),
                        _ => true,
                    })
                    .filter(|model| {
                        format!("{} {}", key(model), model.name.as_deref().unwrap_or(""))
                            .to_lowercase()
                            .contains(query)
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        self.models.sort_by_key(key);
    }
    pub(super) fn models_header(&self, cx: &Context<Self>, theme: Theme) -> AnyElement {
        let mut providers = BTreeSet::new();
        if let Some(session) = self.model(cx) {
            providers.extend(
                session
                    .available_models
                    .iter()
                    .map(|model| model.provider.clone()),
            );
            if let Some(auth) = &session.auth_providers {
                providers.extend(auth.iter().map(|provider| provider.id.clone()));
            }
        }
        // Provider cards scroll independently; a large provider catalog must not consume the table.
        let cards = h_flex()
            .id("model-providers")
            .overflow_x_scroll()
            .gap(px(8.))
            .h(px(86.))
            .flex_shrink_0()
            .children(providers.into_iter().map(|provider| {
                let session = self.model(cx).expect("providers have a session");
                let count = session
                    .available_models
                    .iter()
                    .filter(|model| model.provider == provider)
                    .count();
                let status = auth(session, &provider);
                let name = session
                    .auth_providers
                    .as_ref()
                    .and_then(|entries| entries.iter().find(|entry| entry.id == provider))
                    .map(|entry| entry.name.clone())
                    .unwrap_or_else(|| provider.clone());
                let selected = self.provider.as_ref() == Some(&provider);
                v_flex()
                    .id(keyed("provider", &provider))
                    .w(px(152.))
                    .h(px(66.))
                    .flex_shrink_0()
                    .p(px(10.))
                    .gap(px(4.))
                    .rounded(px(7.))
                    .border_1()
                    .border_color(theme.line)
                    .bg(if selected {
                        theme.selected
                    } else {
                        theme.panel
                    })
                    .cursor_pointer()
                    .tooltip(ui::Tooltip::text(format!("{name} · {status}")))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.provider = if this.provider.as_ref() == Some(&provider) {
                            None
                        } else {
                            Some(provider.clone())
                        };
                        this.project(cx);
                    }))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .child(name),
                            )
                            .child(body_text(count.to_string(), theme)),
                    )
                    .child(body_text(status, theme).truncate())
            }));
        v_flex().flex_shrink_0()
            .child(h_flex().px(px(24.)).pt(px(12.)).gap(px(8.))
                .child(body_text("Configured models · selected session", theme).flex_1())
                .child(button("model-login", "Log in…", theme).on_click(cx.listener(|this, _, window, cx| {
                    // Authentication remains in Pi's interactive UI; never read auth.json or put secrets in a command.
                    let _ = window;
                    if let Some(controller) = this.controller.clone() {
                        this.workspace.update(cx, |workspace, cx| workspace.select(workspace.active, cx));
                        controller.update(cx, |controller, cx| {
                            controller.notice("Sign in: run pi in the terminal, then /login. Restart this session afterward to read the new credentials.", cx);
                            controller.open_in_terminal("pi".into(), cx);
                        });
                    }
                })))
                .child(button("models-refresh", "Refresh", theme).debug_selector(|| "models-refresh".into())
                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx)))))
            .child(div().px(px(24.)).child(cards))
            .child(h_flex().px(px(24.)).pb(px(16.)).gap(px(8.)).flex_wrap()
                .child(segments(["All", "In cycle", "Reasoning", "Images"].into_iter().enumerate().map(|(index, name)| {
                    segment(keyed("model-filter", name), name, self.model_filter == index, theme)
                        .debug_selector(move || format!("model-filter-{index}"))
                        .on_click(cx.listener(move |this, _, _, cx| { this.model_filter = index; this.project(cx); }))
                }), theme))
                .child(body_text(format!("{} models · USD / 1M tokens", self.models.len()), theme)))
            .child(h_flex().h(px(28.)).px(px(20.)).gap(px(10.)).bg(theme.hover)
                .border_l_2().border_color(gpui::transparent_black())
                .child(cell("Cycle", 36., theme))
                .child(div().flex_1().min_w_0().text_size(px(11.)).child("Model / provider"))
                .child(cell("Context", 62., theme)).child(cell("Max out", 62., theme))
                .child(cell("Input", 74., theme)).child(cell("Reasoning", 62., theme))
                .child(cell("In / Out", 104., theme).text_right().debug_selector(|| "model-price-heading".into()))).into_any_element()
    }
    pub(super) fn model_row(&self, index: usize, cx: &Context<Self>, theme: Theme) -> AnyElement {
        let model = &self.models[index];
        let id = key(model);
        let selected = self.selected.as_ref() == Some(&id);
        let in_cycle = self.model(cx).and_then(|session| cycle(session, model));
        h_flex()
            .id(("model-row", index))
            .debug_selector(move || format!("model-row-{index}"))
            .w_full()
            .h(px(48.))
            .px(px(20.))
            .gap(px(10.))
            .cursor_pointer()
            .bg(if selected {
                theme.selected
            } else if index % 2 == 1 {
                theme.hover.opacity(0.4)
            } else {
                theme.canvas
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = Some(id.clone());
                cx.notify();
            }))
            .child(
                cell(
                    match in_cycle {
                        Some(true) => "★",
                        Some(false) => "☆",
                        None => "—",
                    },
                    36.,
                    theme,
                )
                .text_color(if in_cycle == Some(true) {
                    theme.amber
                } else {
                    theme.faint
                }),
            )
            .child(
                v_flex()
                    .id(("model-name", index))
                    .flex_1()
                    .min_w_0()
                    .tooltip(ui::Tooltip::text(key(model)))
                    .child(
                        div()
                            .truncate()
                            .font_family(MONO)
                            .text_size(px(12.))
                            .child(model.id.clone()),
                    )
                    .child(body_text(model.provider.clone(), theme).truncate()),
            )
            .child(cell(number(Some(model.context_window)), 62., theme))
            .child(cell(number(model.max_tokens), 62., theme))
            .child(cell(
                if model.input.is_empty() {
                    "—".into()
                } else {
                    model.input.join(", ")
                },
                74.,
                theme,
            ))
            .child(cell(
                match model.reasoning {
                    Some(true) => "Yes",
                    Some(false) => "No",
                    None => "—",
                },
                62.,
                theme,
            ))
            .child(
                cell(
                    format!(
                        "{} / {}",
                        price(model.cost.as_ref().and_then(|c| c.input)),
                        price(model.cost.as_ref().and_then(|c| c.output))
                    ),
                    104.,
                    theme,
                )
                .text_right()
                .debug_selector(move || format!("model-price-{index}")),
            )
            .into_any_element()
    }
    pub(super) fn models_footer(&self, theme: Theme) -> AnyElement {
        v_flex().p(px(24.)).gap(px(4.)).flex_shrink_0()
            .child(body_text("Amber stars mark the reported session cycle. An empty scope cycles through all available models.", theme))
            .child(body_text("Pi reports configured models, not the entire public catalog. Missing prices and capabilities stay unknown.", theme)).into_any_element()
    }
    pub(super) fn model_details(&self, cx: &Context<Self>, theme: Theme) -> AnyElement {
        let Some(model) = self
            .models
            .iter()
            .find(|model| self.selected.as_ref() == Some(&key(model)))
            .cloned()
        else {
            return body_text(
                "Select a model to inspect its limits, pricing and configuration.",
                theme,
            )
            .into_any_element();
        };
        let session = self.model(cx).expect("selected model has a session");
        let current = session
            .state
            .model
            .as_ref()
            .is_some_and(|current| key(current) == key(&model));
        let enabled = self.mutable(cx);
        let in_cycle = cycle(session, &model);
        let model_use = model.clone();
        let model_default = model.clone();
        let model_cycle = model.clone();
        let detail_id = key(&model);
        v_flex().id("model-details").debug_selector(move || format!("model-detail-{detail_id}")).gap(px(6.))
            .child(inspector_title(model.name.clone().unwrap_or_else(|| model.id.clone()), true))
            .child(body_text(auth(session, &model.provider), theme))
            .child(body_text(key(&model), theme).font_family(MONO))
            .child(divider(theme)).child(section("LIMITS", "", theme))
            .child(detail("Context window", number(Some(model.context_window)), true, theme))
            .child(detail("Max output", number(model.max_tokens), true, theme))
            .child(detail("Input", if model.input.is_empty() { "Not reported".into() } else { model.input.join(", ") }, false, theme))
            .child(divider(theme)).child(section("THINKING", if current { "current model" } else { "" }, theme))
            .when(current, |view| view.child(h_flex().gap(px(4.)).flex_wrap().children(session.thinking_levels.iter().map(|level| {
                let level = level.clone();
                let model = model.clone();
                let selected = session.state.thinking_level == level;
                button(keyed("model-thinking", &level), level.clone(), theme)
                    .text_color(theme.thinking(&level)).when(selected, |button| button.bg(theme.selected))
                    .when(enabled, |button| button.on_click(cx.listener(move |this, _, _, cx| {
                        this.send(Command::SetModelThinkingLevel { provider: model.provider.clone(), model_id: model.id.clone(), level: level.clone() }, cx);
                    })))
            }))))
            .child(body_text(if current { "Choosing a level saves this model's thinking preference in Pi." }
                else { "Supported levels are reported for the current model only. Use this model to inspect them." }, theme))
            .child(divider(theme)).child(section("PRICE", "USD per 1M tokens", theme))
            .children([
                ("Input", model.cost.as_ref().and_then(|c| c.input)),
                ("Output", model.cost.as_ref().and_then(|c| c.output)),
                ("Cache read", model.cost.as_ref().and_then(|c| c.cache_read)),
                ("Cache write", model.cost.as_ref().and_then(|c| c.cache_write)),
            ].into_iter().map(|(name, value)| detail(name, price(value), true, theme)))
            .child(divider(theme))
            .child(h_flex().justify_between().child(body_text("In the cycle", theme))
                .when_some(in_cycle, |row, on| row.child(toggle("model-cycle", on, theme)
                    .aria_label("In the model cycle")
                    .debug_selector(|| "model-cycle".into())
                    .when(!enabled, |switch| switch.opacity(0.5).cursor_default())
                    .when(enabled, |switch| switch.on_click(cx.listener(move |this, _, window, cx| {
                        this.change_cycle(&model_cycle, window, cx);
                    })))))
                .when(in_cycle.is_none(), |row| row.child(body_text("Not reported", theme))))
            .child(primary_button("model-use", if current { "Current session model" } else { "Use in this session" }, enabled && !current, theme)
                .debug_selector(|| "model-use".into())
                .when(enabled && !current, |button| button.on_click(cx.listener(move |this, _, _, cx| {
                    this.send(Command::SetModel { provider: model_use.provider.clone(), model_id: model_use.id.clone(), persist: false }, cx);
                }))))
            .child(button("model-default", "Use and set as default", theme)
                .when(!enabled, |button| button.opacity(0.5))
                .when(enabled, |button| button.on_click(cx.listener(move |this, _, window, cx| {
                    this.confirm(Command::SetModel { provider: model_default.provider.clone(), model_id: model_default.id.clone(), persist: true },
                        "Set default model?", "This switches the selected session and saves the model for new sessions.", window, cx);
                }))))
            .child(body_text("Changes target the selected session. No prompt is sent when inspecting or selecting a model.", theme)).into_any_element()
    }
    fn change_cycle(&mut self, model: &Model, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.model(cx) else {
            return;
        };
        let Some(scope) = session
            .settings
            .as_ref()
            .and_then(|settings| settings["scopedModels"].as_array())
        else {
            return;
        };
        let mut ids: Vec<String> = if scope.is_empty() {
            session.available_models.iter().map(key).collect()
        } else {
            scope
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        };
        let target = key(model);
        if ids.contains(&target) {
            ids.retain(|id| id != &target);
        } else {
            ids.push(target);
        }
        if ids.is_empty() {
            drop(window.prompt(
                PromptLevel::Info,
                "Keep one model in the cycle",
                Some("Pi interprets an empty cycle as all available models."),
                &["OK"],
                cx,
            ));
            return;
        }
        self.confirm(Command::SetScopedModels { patterns: Some(ids), persist: true }, "Save model cycle?",
            "This replaces saved enabledModels patterns (including any :level suffixes) with this explicit model list, and changes the selected session's cycle. Per-model thinking preferences are unchanged.", window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn unknown_metadata_and_empty_scope_are_distinct() {
        let model: Model = serde_json::from_value(json!({"id":"same", "provider":"a"})).unwrap();
        let mut session = Session::default();
        assert_eq!(cycle(&session, &model), None);
        assert_eq!(price(None), "—");
        assert_eq!(price(Some(0.)), "$0.00");
        session.settings = Some(json!({"scopedModels":[]}));
        assert_eq!(cycle(&session, &model), Some(true));
        session.settings = Some(json!({"scopedModels":["b/same"]}));
        assert_eq!(cycle(&session, &model), Some(false));
    }
}
