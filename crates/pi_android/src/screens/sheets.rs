//! Bottom sheets: a question to answer (05), a session's details (09), and
//! the short choices: attaching, the model, the project, more actions, and
//! what Pi has on the computer.

use crate::{
    app::{PhoneApp, Route, Sheet, Target},
    model::{Reference, Session, SessionId},
    theme::{MONO, Theme, theme},
    ui::{self, Button, icon},
};
use gpui::{
    AnyElement, Context, Div, FontWeight, SharedString, Window, div, prelude::*, px, relative,
};

const MODELS: [(&str, &str); 4] = [
    ("Opus 5.5", "Anthropic · the most capable"),
    ("Sonnet 5.5", "Anthropic · fast and capable"),
    ("Fable 5.1", "Anthropic"),
    ("Haiku 4.5", "Anthropic · the fastest"),
];

const THINKING: [&str; 6] = ["Off", "Minimal", "Low", "Medium", "High", "Max"];

/// What a row of the More sheet does.
type Action = Box<dyn Fn(&mut PhoneApp, &mut Window, &mut Context<PhoneApp>)>;

fn model_matches(query: &str, name: &str, provider: &str, id: &str) -> bool {
    let searchable = format!("{name} {provider} {id}").to_lowercase();
    query
        .split_whitespace()
        .all(|word| searchable.contains(word))
}

impl PhoneApp {
    pub(crate) fn sheet_content(
        &mut self,
        sheet: Sheet,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = theme(cx);
        // Search stays in place even when hundreds of models are offered.
        if sheet == Sheet::Model {
            return self.model_sheet(&colors, cx).into_any_element();
        }
        let content = match sheet {
            Sheet::Question(id) => self.question_sheet(id, &colors, cx),
            Sheet::Details(id) => self.details_sheet(id, &colors, cx),
            Sheet::Attach(target) => self.attach_sheet(target, &colors, cx),
            Sheet::Model => self.model_sheet(&colors, cx),
            Sheet::Thinking => self.thinking_sheet(&colors, cx),
            Sheet::Project => self.project_sheet(&colors, cx),
            Sheet::More(id) => self.more_sheet(id, &colors, cx),
            Sheet::Models => self.models_sheet(&colors, cx),
            Sheet::Resources => self.resources_sheet(&colors, cx),
            Sheet::Activity(id, turn, stage) => self.activity_sheet(id, turn, stage, &colors, cx),
            Sheet::Delete(id) => self.delete_sheet(id, &colors, cx),
            Sheet::Image(target, index) => {
                let attachment = self
                    .composer(target)
                    .and_then(|composer| composer.read(cx).attachments().get(index).cloned());
                if let Some(crate::composer::Attachment::Image { name, image }) = attachment {
                    div()
                        .child(ui::hint(name, &colors).mb(px(12.)))
                        .child(gpui::img(image).w_full().h(window.fully_visible_bounds().size.height * 0.55).object_fit(gpui::ObjectFit::Contain))
                        .child(ui::hint("Included with your next message. Images are resized to fit the upload limit.", &colors).mt(px(12.)))
                } else {
                    div()
                }
            }
        };
        // Every sheet keeps its identity and explicit close action visible,
        // independently of the scroll position of long output or lists.
        let header = match sheet {
            Sheet::Question(id) if self.session(id).and_then(|s| s.question.as_ref()).is_some() => {
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(ui::badge("Needs you", colors.wait, colors.amber, &colors))
                    .child(
                        ui::tap("close-sheet", "x", &colors)
                            .mr(px(-12.))
                            .on_click(cx.listener(|this, _, _, cx| this.close_sheet(cx))),
                    )
            }
            _ => {
                let title: SharedString = match sheet {
                    Sheet::Question(_) => "Already answered".into(),
                    Sheet::Details(_) => "Details".into(),
                    Sheet::Attach(_) => "Add to the message".into(),
                    Sheet::Model => "Choose model".into(),
                    Sheet::Thinking => "Thinking level".into(),
                    Sheet::Project => format!("Projects on {}", self.computer_name()).into(),
                    Sheet::More(id) => self
                        .session(id)
                        .map_or_else(|| "Session".into(), |s| s.title.clone().into()),
                    Sheet::Models => format!("Models on {}", self.computer_name()).into(),
                    Sheet::Resources => format!("Resources on {}", self.computer_name()).into(),
                    Sheet::Activity(id, turn, stage) => self
                        .session(id)
                        .and_then(|s| s.turns.get(turn))
                        .and_then(|t| t.stages.get(stage))
                        .map_or("Activity", |s| s.kind.name(s.status))
                        .into(),
                    Sheet::Delete(_) => "Delete session?".into(),
                    Sheet::Image(_, _) => "Attached image".into(),
                };
                self.sheet_title(title, &colors, cx)
            }
        };
        div()
            .flex()
            .flex_col()
            .min_h_0()
            .child(
                header
                    .flex_none()
                    .px(px(20.))
                    .debug_selector(|| "sheet-header".into()),
            )
            .child(
                crate::scroll::vertical(
                    SharedString::from(format!("sheet-body-{sheet:?}")),
                    &self.sheet_scroll,
                )
                .min_h_0()
                .px(px(20.))
                .child(content),
            )
            .into_any_element()
    }

    fn session(&self, id: SessionId) -> Option<&Session> {
        self.store.as_ref()?.session(id)
    }

    fn delete_sheet(&self, id: SessionId, colors: &Theme, cx: &Context<Self>) -> Div {
        let Some(session) = self.session(id) else {
            return div();
        };
        let running = session.state.is_running();
        let deleting = self.deleting_session == Some(id);
        div().pb(px(8.))
            .child(ui::card(colors).p(px(14.)).my(px(12.))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(session.title.clone()))
                .child(ui::hint(format!("{} · {}", session.project, self.computer_name()), colors).mt(px(4.))))
            .child(ui::hint(
                if self.live() {
                    "Permanently deletes this session's conversation and execution history from the computer. This cannot be undone. Project files and edits are kept."
                } else {
                    "Removes this sample conversation. No files on your computer are affected."
                }, colors))
            .when(running, |body| body.child(ui::hint("This session is still running. Stop it before deleting its history.", colors).mt(px(12.)).text_color(colors.coral)))
            .child(div().flex().gap(px(12.)).mt(px(20.))
                .child(ui::button("cancel-delete", Button::Plain, None, "Cancel", false, colors)
                    .flex_1().on_click(cx.listener(|this, _, _, cx| this.close_sheet(cx))))
                .child(ui::button("confirm-delete", Button::Primary, Some("trash"), if deleting { "Deleting…" } else { "Delete" }, false, colors)
                    .debug_selector(|| "confirm-delete".into())
                    .flex_1().min_w_0().bg(colors.coral).border_color(colors.coral)
                    .when(running || deleting, |button| button.opacity(0.45))
                    .when(!running && !deleting, |button| button.on_click(cx.listener(move |this, _, _, cx| this.delete_session(id, cx))))))
    }

    fn activity_sheet(
        &self,
        id: SessionId,
        turn_index: usize,
        stage_index: usize,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let Some(session) = self.session(id) else {
            return div();
        };
        let Some(stage) = session
            .turns
            .get(turn_index)
            .and_then(|turn| turn.stages.get(stage_index))
        else {
            return div();
        };
        div()
            .pb(px(12.))
            .min_w_0()
            .child(
                ui::hint(
                    format!(
                        "Turn {} · {} · {}",
                        turn_index + 1,
                        session.project,
                        self.computer_name()
                    ),
                    colors,
                )
                .mb(px(12.)),
            )
            .child(
                div()
                    .text_size(px(15.))
                    .child(stage.what.clone())
                    .mb(px(12.)),
            )
            .children(
                stage
                    .references
                    .iter()
                    .enumerate()
                    .map(|(index, reference)| {
                        let (glyph, target) = match reference {
                            Reference::File(path) => ("file", path),
                            Reference::Search(query) => ("search", query),
                        };
                        let file = session
                            .files
                            .iter()
                            .position(|file| file.path == *target || file.name() == target);
                        let copied = target.clone();
                        ui::row(("activity-reference", index), index == 0, colors)
                            .child(icon(glyph, 18., colors.muted))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(ui::mono(target.clone(), 12.5)),
                            )
                            .child(
                                ui::button(
                                    ("open-reference", index),
                                    Button::Quiet,
                                    None,
                                    if file.is_some() { "Review" } else { "Copy" },
                                    true,
                                    colors,
                                )
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        if let Some(file) = file {
                                            this.open_review(id, file, window, cx);
                                        } else {
                                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                                copied.clone(),
                                            ));
                                            this.notify_user("Copied", cx);
                                        }
                                    },
                                )),
                            )
                    }),
            )
            .children(stage.tools.iter().enumerate().map(|(index, tool)| {
                let target = tool.target.clone();
                let output = tool.output.clone();
                let status = if tool.failed {
                    "Failed"
                } else if tool.finished {
                    "Finished"
                } else {
                    "Running…"
                };
                ui::card(colors)
                    .mt(px(12.))
                    .p(px(14.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(ui::label(tool.name.clone(), colors).flex_1())
                            .child(ui::hint(status, colors).text_color(if tool.failed {
                                colors.coral
                            } else {
                                colors.muted
                            })),
                    )
                    .child(ui::mono(tool.target.clone(), 12.5).mt(px(8.)))
                    .child(
                        ui::button(
                            ("copy-command", index),
                            Button::Quiet,
                            Some("copy"),
                            "Copy",
                            true,
                            colors,
                        )
                        .h(px(48.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(target.clone()));
                            this.notify_user("Copied", cx);
                        })),
                    )
                    .when(!tool.output.is_empty(), |card| {
                        card.child(ui::label("Output", colors).mt(px(8.)))
                            .child(
                                ui::mono(tool.output.clone(), 12.)
                                    .mt(px(6.))
                                    .line_height(relative(1.5)),
                            )
                            .child(
                                ui::button(
                                    ("copy-output", index),
                                    Button::Quiet,
                                    Some("copy"),
                                    "Copy output",
                                    true,
                                    colors,
                                )
                                .h(px(48.))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                            output.clone(),
                                        ));
                                        this.notify_user("Copied output", cx);
                                    },
                                )),
                            )
                    })
                    .when(tool.output.is_empty(), |card| {
                        card.child(ui::hint(
                            if tool.finished {
                                "No output was recorded."
                            } else {
                                "Waiting for output…"
                            },
                            colors,
                        ))
                    })
            }))
            .when(
                stage.tools.is_empty() && stage.references.is_empty(),
                |body| body.child(ui::hint("No tool calls recorded for this stage.", colors)),
            )
    }

    /// The models to offer, with a detail: the computer's once a session
    /// reported them, the sample's otherwise.
    fn offered_models(&self) -> Vec<(String, String, String)> {
        if self.paused && !self.preview_models.is_empty() {
            return self
                .preview_models
                .iter()
                .map(|model| {
                    (
                        model.name.clone().unwrap_or_else(|| model.id.clone()),
                        model.provider.clone(),
                        model.id.clone(),
                    )
                })
                .collect();
        }
        match self.store.as_ref().and_then(|store| store.live.as_ref()) {
            Some(live) => live
                .models
                .iter()
                .map(|model| {
                    let name = model.name.clone().unwrap_or_else(|| model.id.clone());
                    (name, model.provider.clone(), model.id.clone())
                })
                .collect(),
            None => MODELS
                .iter()
                .map(|(name, detail)| (name.to_string(), detail.to_string(), name.to_string()))
                .collect(),
        }
    }

    fn live(&self) -> bool {
        self.store.as_ref().is_some_and(|store| !store.is_sample())
    }

    fn computer_name(&self) -> String {
        self.store.as_ref().map_or_else(
            || "the computer".to_owned(),
            |store| store.computer.name.clone(),
        )
    }

    /// A sheet's title with a close button.
    fn sheet_title(
        &self,
        title: impl Into<SharedString>,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        div()
            .flex()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(20.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(title.into()),
            )
            .child(
                ui::tap("close-sheet", "x", colors)
                    .mr(px(-12.))
                    .on_click(cx.listener(|this, _, _, cx| this.close_sheet(cx))),
            )
    }

    fn question_sheet(&self, id: SessionId, colors: &Theme, cx: &Context<Self>) -> Div {
        let Some(question) = self
            .session(id)
            .and_then(|session| session.question.as_ref())
        else {
            return div().pb(px(8.)).child(
                ui::hint("This question was answered, or the run ended.", colors).mt(px(4.)),
            );
        };
        let choices = question.choices.iter().enumerate().map(|(index, choice)| {
            let on = self.choice == Some(choice.answer);
            let answer = choice.answer;
            div()
                .id(("choice", index))
                .min_h(px(56.))
                .px(px(14.))
                .py(px(8.))
                .flex()
                .items_center()
                .gap(px(14.))
                .rounded(px(14.))
                .when(on, |row| row.bg(colors.selected))
                .active(|style| style.bg(colors.selected))
                .child(
                    div()
                        .size(px(20.))
                        .flex_none()
                        .rounded_full()
                        .border(px(if on { 6. } else { 2. }))
                        .border_color(if on {
                            colors.accent
                        } else {
                            colors.line_strong
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_size(px(15.)).child(choice.label.clone()))
                        .children(choice.detail.clone().map(|detail| {
                            div()
                                .text_size(px(13.))
                                .text_color(colors.muted)
                                .child(detail)
                        })),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.choice = Some(answer);
                    cx.notify();
                }))
        });
        let answer = ui::button("answer", Button::Primary, None, "Answer", false, colors).flex_1();
        let answer = if self.choice.is_some() {
            answer.on_click(cx.listener(move |this, _, window, cx| this.answer(id, window, cx)))
        } else {
            ui::disabled(answer, colors)
        };
        div()
            .pb(px(10.))
            .flex()
            .flex_col()
            .child(
                div()
                    .mt(px(10.))
                    .text_size(px(20.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(question.title.clone()),
            )
            .child(
                ui::hint(question.body.clone(), colors)
                    .mt(px(2.))
                    .text_size(px(14.)),
            )
            .child(
                div()
                    .mt(px(14.))
                    .px(px(14.))
                    .py(px(12.))
                    .rounded(px(12.))
                    .bg(colors.panel)
                    .font_family(MONO)
                    .text_size(px(13.))
                    .text_color(colors.plain)
                    .child(question.command.clone()),
            )
            .child(
                div()
                    .mt(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .children(choices),
            )
            .child(
                div()
                    .mt(px(16.))
                    .flex()
                    .gap(px(8.))
                    .child(
                        ui::button("later", Button::Plain, None, "Later", false, colors)
                            .flex_1()
                            .on_click(cx.listener(|this, _, _, cx| this.close_sheet(cx))),
                    )
                    .child(answer),
            )
    }

    fn details_sheet(&self, id: SessionId, colors: &Theme, cx: &Context<Self>) -> Div {
        let Some(session) = self.session(id) else {
            return div();
        };
        let details = &session.details;
        let prefs = self.prefs(cx);
        let disclosure = |key: &'static str, title: &'static str, value: AnyElement, more: Div| {
            let open = self.expanded.contains(key);
            div()
                .border_t_1()
                .border_color(colors.line)
                .child(
                    div()
                        .id(key)
                        .h(px(52.))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(div().flex_1().text_size(px(15.)).child(title))
                        .child(value)
                        .child(icon(
                            if open { "chev_d" } else { "chev_r" },
                            16.,
                            colors.muted,
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.expanded.remove(key) {
                                this.expanded.insert(key);
                            }
                            cx.notify();
                        })),
                )
                .when(open, |row| row.child(more.pb(px(14.))))
        };
        let session_file = details.session_file.clone();
        div()
            .pb(px(8.))
            .child(ui::label("Context", colors).mt(px(8.)))
            .child(
                div()
                    .mt(px(2.))
                    .flex()
                    .items_baseline()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(28.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{}%", details.context_percent)),
                    )
                    .child(ui::hint(details.context_tokens.clone(), colors)),
            )
            .child(
                div().mt(px(10.)).h(px(8.)).rounded(px(4.)).bg(colors.raised).child(
                    div()
                        .h_full()
                        .w(relative(details.context_percent.min(100) as f32 / 100.))
                        .rounded(px(4.))
                        .bg(colors.accent),
                ),
            )
            .child(ui::hint("Auto-compaction is on.", colors).mt(px(8.)))
            .child(ui::label("Run history", colors).mt(px(20.)).mb(px(8.)))
            .children(session.turns.iter().enumerate().map(|(index, turn)| {
                div().id(("details-turn", index)).mb(px(8.))
                    .child(ui::label(format!("Turn {} · {}", index + 1, turn.at), colors))
                    .child(self.turn_activity(id, index, turn, false, colors, cx))
            }))
            .child(
                ui::card(colors)
                    .mt(px(18.))
                    .flex()
                    .child(pair("Session cost", details.cost.clone(), colors))
                    .child(
                        pair("Observed edits", session.files.len().to_string(), colors)
                            .border_l_1()
                            .border_color(colors.line),
                    ),
            )
            .child(
                div()
                    .mt(px(18.))
                    .child(disclosure(
                        "usage",
                        "Usage",
                        ui::hint(format!("{} · {} turns", prefs.model, details.turns), colors).into_any_element(),
                        ui::hint(
                            format!(
                                "{} with {} thinking. {} turns so far, {} in all.",
                                prefs.model,
                                prefs.thinking.to_lowercase(),
                                details.turns,
                                details.cost
                            ),
                            colors,
                        ),
                    ))
                    .child(disclosure(
                        "tools",
                        "Active tools",
                        ui::hint(details.tools.len().to_string(), colors).into_any_element(),
                        div().flex().flex_wrap().gap(px(8.)).children(
                            details
                                .tools
                                .iter()
                                .enumerate()
                                .map(|(index, tool)| ui::chip(("tool", index), None, tool.clone(), colors).font_family(MONO)),
                        ),
                    ))
                    .child(disclosure(
                        "history",
                        "File history",
                        ui::hint(format!("on · {} snapshots", details.snapshots), colors).into_any_element(),
                        ui::hint(
                            "jj records each turn that edits files, so a turn can be undone on the computer.",
                            colors,
                        ),
                    ))
                    .child(disclosure(
                        "file",
                        "Session file",
                        ui::mono(shorten(&details.session_file), 12.)
                            .text_color(colors.muted)
                            .into_any_element(),
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                ui::mono(details.session_file.clone(), 12.)
                                    .flex_1()
                                    .min_w_0()
                                    .text_color(colors.secondary),
                            )
                            .child(
                                ui::button("copy-file", Button::Quiet, Some("copy"), "Copy", true, colors)
                                    .h(px(32.))
                                    .px(px(4.))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.copy(session_file.clone(), "the path", cx)
                                    })),
                            ),
                    )),
            )
    }

    fn attach_sheet(&self, target: Target, colors: &Theme, cx: &Context<Self>) -> Div {
        div().pb(px(8.)).child(
            ui::card(colors)
                .mt(px(8.))
                .child(
                    ui::row("choose-files", true, colors)
                        .child(icon("file", 20., colors.muted))
                        .child(ui::row_text(
                            "Choose files",
                            Some("Images, code and UTF-8 text files".into()),
                            colors,
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| this.attach_files(target, cx))),
                )
                .child(
                    ui::row("paste", false, colors)
                        .child(icon("copy", 20., colors.muted))
                        .child(ui::row_text(
                            "Paste",
                            Some("An image or text from the clipboard".into()),
                            colors,
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| this.paste_into(target, cx))),
                ),
        )
    }

    fn model_sheet(&self, colors: &Theme, cx: &Context<Self>) -> Div {
        let (selected_model, _) = self.model_settings(cx);
        let offered = self.offered_models();
        let unknown = offered.is_empty();
        let catalog = self.store.as_ref().and_then(|store| store.live.as_ref());
        let catalog_note = catalog.map(|live| {
            if live.models_loading { "Loading models from your computer…".to_owned() }
            else { live.models_error.clone().unwrap_or_else(|| "No models are configured yet. Sign in to a provider on the computer, then reload.".into()) }
        });
        let can_reload = catalog.is_some_and(|live| !live.models_loading);
        let query = self.model_search.read(cx).text().to_lowercase();
        let total = offered.len();
        let offered: Vec<_> = offered
            .into_iter()
            .filter(|(name, provider, id)| model_matches(&query, name, provider, id))
            .collect();
        let count = offered.len();
        let selected_identity = self
            .store
            .as_ref()
            .and_then(|store| store.live.as_ref())
            .and_then(|live| self.model_session().and_then(|id| live.current_model(id)))
            .map(|model| (model.provider.clone(), model.id.clone()))
            .or_else(|| {
                self.model_session()
                    .is_none()
                    .then(|| {
                        let prefs = self.prefs(cx);
                        prefs.model_provider.clone().zip(prefs.model_id.clone())
                    })
                    .flatten()
            });
        let models = offered
            .into_iter()
            .enumerate()
            .map(|(index, (name, detail, model_id))| {
                let on = selected_identity
                    .as_ref()
                    .map_or(selected_model == name, |(provider, id)| {
                        *provider == detail && *id == model_id
                    });
                let provider = detail.clone();
                ui::row(("model", index), index == 0, colors)
                    .debug_selector(move || format!("model-choice-{index}").into())
                    .child(ui::row_text(
                        name.clone(),
                        Some(format!("{detail} · {model_id}").into()),
                        colors,
                    ))
                    .when(on, |row| row.child(icon("check", 20., colors.accent)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.choose_model(name.clone(), provider.clone(), model_id.clone(), cx);
                        this.close_sheet(cx);
                    }))
            });
        div()
            .flex()
            .flex_col()
            .min_h_0()
            .child(
                div()
                    .px(px(20.))
                    .flex_none()
                    .child(self.sheet_title("Choose model", colors, cx))
                    .child(ui::hint(
                        if self.model_session().is_some() {
                            "For follow-ups in this session."
                        } else {
                            "Default for new sessions."
                        },
                        colors,
                    ))
                    .child(
                        ui::card(colors).my(px(10.)).px(px(12.)).py(px(14.)).child(
                            div()
                                .relative()
                                .child(crate::testing::probe("model-search"))
                                .debug_selector(|| "model-search".into())
                                .child(self.model_search.clone()),
                        ),
                    )
                    .child(ui::hint(format!("{count} of {total} models"), colors).mb(px(8.))),
            )
            .child(
                crate::scroll::vertical("model-list", &self.sheet_scroll)
                    .min_h_0()
                    .px(px(20.))
                    .pb(px(8.))
                    .when(unknown, |list| {
                        list.child(ui::hint(
                            catalog_note.unwrap_or_else(|| "No models available.".into()),
                            colors,
                        ))
                        .when(can_reload, |list| {
                            list.child(
                                ui::button(
                                    "reload-models",
                                    Button::Plain,
                                    None,
                                    "Reload models",
                                    false,
                                    colors,
                                )
                                .my(px(12.))
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        if let Some(live) = this
                                            .store
                                            .as_mut()
                                            .and_then(|store| store.live.as_mut())
                                        {
                                            live.refresh_models();
                                        }
                                        cx.notify();
                                    },
                                )),
                            )
                        })
                    })
                    .when(!unknown && count == 0, |list| {
                        list.child(
                            ui::hint(
                                "No matching models. Try a model name, provider, or ID.",
                                colors,
                            )
                            .py(px(20.)),
                        )
                    })
                    .when(count > 0, |list| {
                        list.child(ui::card(colors).children(models))
                    }),
            )
    }

    fn thinking_sheet(&self, colors: &Theme, cx: &Context<Self>) -> Div {
        let (model, selected) = self.model_settings(cx);
        let levels = self
            .store
            .as_ref()
            .and_then(|store| store.live.as_ref())
            .and_then(|live| self.model_session().and_then(|id| live.thinking_levels(id)))
            .unwrap_or_else(|| THINKING.iter().map(|level| (*level).to_owned()).collect());
        div().pb(px(8.))
            .child(ui::hint(format!("Reasoning effort for {model}"), colors).mb(px(12.)))
            .child(ui::card(colors).children(levels.into_iter().enumerate().map(|(index, level)| {
                let detail = match level.as_str() {
                    "Off" => "No extra reasoning",
                    "Minimal" => "A little reasoning, quickest replies",
                    "Low" => "Fast, light reasoning",
                    "Medium" => "Balanced speed and depth",
                    "High" => "More thorough reasoning",
                    _ => "Deepest reasoning; takes longer",
                };
                ui::row(("thinking", index), index == 0, colors)
                    .debug_selector(move || format!("thinking-choice-{index}").into())
                    .min_h(px(56.))
                    .child(ui::row_text(level.clone(), Some(detail.into()), colors))
                    .when(selected == level, |row| row.child(icon("check", 20., colors.accent)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.choose_thinking(&level, cx);
                        this.close_sheet(cx);
                    }))
            })))
            .child(ui::hint("Higher effort can use more tokens and take longer. Available levels depend on the model.", colors).mt(px(12.)))
    }

    fn project_sheet(&self, colors: &Theme, cx: &Context<Self>) -> Div {
        div().pb(px(8.)).child(self.project_picker(colors, cx))
    }

    fn more_sheet(&self, id: SessionId, colors: &Theme, cx: &Context<Self>) -> Div {
        let Some(session) = self.session(id) else {
            return div();
        };
        let prompt = session
            .turn()
            .map(|turn| turn.prompt.clone())
            .unwrap_or_default();
        let summary = session
            .turn()
            .and_then(|turn| turn.summary.as_ref())
            .map(|summary| summary.text());
        let reviewing = self.route() == Route::Review(id);
        let mut rows: Vec<(&'static str, &'static str, gpui::Hsla, Action)> = vec![(
            "copy",
            "Copy the prompt",
            colors.muted,
            Box::new(move |this, _, cx| this.copy(prompt.clone(), "the prompt", cx)),
        )];
        if let Some(summary) = summary {
            rows.push((
                "copy",
                "Copy the reply",
                colors.muted,
                Box::new(move |this, _, cx| this.copy(summary.clone(), "the reply", cx)),
            ));
        }
        if !session.files.is_empty() && !reviewing {
            rows.push((
                "diff",
                "Review changes",
                colors.muted,
                Box::new(move |this, window, cx| this.open_review(id, 0, window, cx)),
            ));
        }
        if session.state.is_running() {
            rows.push((
                "stop",
                "Stop",
                colors.coral,
                Box::new(move |this, _, cx| this.stop(id, cx)),
            ));
        }
        rows.push((
            "trash",
            "Delete session…",
            colors.coral,
            Box::new(move |this, _, cx| this.open_sheet(Sheet::Delete(id), cx)),
        ));
        div().pb(px(8.)).child(
            ui::card(colors)
                .mt(px(8.))
                .children(rows.into_iter().enumerate().map(
                    |(index, (glyph, text, color, action))| {
                        ui::row(("action", index), index == 0, colors)
                            .child(icon(glyph, 20., color))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(15.))
                                    .when(glyph == "stop", |text| text.text_color(colors.coral))
                                    .child(text),
                            )
                            .on_click(
                                cx.listener(move |this, _, window, cx| action(this, window, cx)),
                            )
                    },
                )),
        )
    }

    fn models_sheet(&self, colors: &Theme, _: &Context<Self>) -> Div {
        let computer = self.computer_name();
        div()
            .pb(px(8.))
            .child(ui::card(colors).mt(px(8.)).children(self.offered_models().into_iter().enumerate().map(
                |(index, (name, detail, _))| {
                    ui::row(("known-model", index), index == 0, colors)
                        .child(icon("spark", 20., colors.muted))
                        .child(ui::row_text(name, Some(detail.into()), colors))
                },
            )))
            .child(
                ui::hint(
                    if self.live() {
                        format!("Models available on {computer}, loaded before starting a session. Sign-ins stay on {computer}.")
                    } else {
                        format!("A sample list. Sign-ins stay on {computer}: change them in Pi Desktop or Pi’s settings there.")
                    },
                    colors,
                )
                .mt(px(14.))
                .mx(px(4.)),
            )
    }

    fn resources_sheet(&self, colors: &Theme, _: &Context<Self>) -> Div {
        let resources = [
            ("file", "AGENTS.md", "Context · ~/repos/pi"),
            ("slash", "/fix-tests", "Prompt"),
            ("slash", "/review", "Prompt"),
            ("slash", "/explain", "Prompt"),
            ("layers", "web-search", "Extension"),
        ];
        if self.live() {
            return div()
                .pb(px(8.))
                .child(
                    ui::hint(
                        "Durable sessions don't load Pi's extensions, skills or prompt templates yet, so there is nothing to list.",
                        colors,
                    )
                    .mt(px(8.))
                    .mx(px(4.)),
                );
        }
        div()
            .pb(px(8.))
            .child(
                ui::card(colors)
                    .mt(px(8.))
                    .children(resources.iter().enumerate().map(
                        |(index, (glyph, name, detail))| {
                            ui::row(("resource", index), index == 0, colors)
                                .child(icon(glyph, 20., colors.muted))
                                .child(ui::row_text(*name, Some((*detail).into()), colors))
                        },
                    )),
            )
            .child(
                ui::hint(
                    "A sample list. The real one comes from Pi on the computer.",
                    colors,
                )
                .mt(px(14.))
                .mx(px(4.)),
            )
    }
}

/// One of the two figures in the details card.
fn pair(label: &'static str, value: String, colors: &Theme) -> Div {
    div()
        .flex_1()
        .px(px(16.))
        .py(px(14.))
        .child(ui::label(label, colors))
        .child(
            div()
                .mt(px(2.))
                .text_size(px(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(value),
        )
}

/// "~/.pi/…/qwen-signatures.jsonl": the start and the file name.
fn shorten(path: &str) -> String {
    match (path.find('/'), path.rfind('/')) {
        (Some(first), Some(last)) if last > first => {
            let start = path[first + 1..]
                .find('/')
                .map_or(first, |next| first + 1 + next);
            if start >= last {
                path.to_owned()
            } else {
                format!("{}/…{}", &path[..start], &path[last..])
            }
        }
        _ => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::shorten;

    #[test]
    fn long_paths_keep_their_start_and_file() {
        assert_eq!(
            shorten("~/.pi/agent/sessions/--repos-pi--/qwen-signatures.jsonl"),
            "~/.pi/…/qwen-signatures.jsonl"
        );
        assert_eq!(shorten("~/.pi/x.jsonl"), "~/.pi/x.jsonl");
        assert_eq!(shorten("x.jsonl"), "x.jsonl");
    }
}
