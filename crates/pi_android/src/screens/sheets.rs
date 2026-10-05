//! Bottom sheets: a question to answer (05), a session's details (09), and
//! the short choices: attaching, the model, the project, more actions, and
//! what Pi has on the computer.

use crate::{
    app::{PhoneApp, Route, Sheet, Target},
    model::{Session, SessionId},
    theme::{MONO, Theme, theme},
    ui::{self, Button, icon},
};
use gpui::{
    AnyElement, Context, Div, Focusable, FontWeight, SharedString, Window, div, prelude::*, px,
    relative,
};

const MODELS: [(&str, &str); 4] = [
    ("Opus 5.5", "Anthropic · the most capable"),
    ("Sonnet 5.5", "Anthropic · fast and capable"),
    ("Fable 5.1", "Anthropic"),
    ("Haiku 4.5", "Anthropic · the fastest"),
];

const THINKING: [&str; 5] = ["Off", "Low", "Medium", "High", "Max"];

/// What a row of the More sheet does.
type Action = Box<dyn Fn(&mut PhoneApp, &mut Window, &mut Context<PhoneApp>)>;

impl PhoneApp {
    pub(crate) fn sheet_content(
        &mut self,
        sheet: Sheet,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = theme(cx);
        let content = match sheet {
            Sheet::Question(id) => self.question_sheet(id, &colors, cx),
            Sheet::Details(id) => self.details_sheet(id, &colors, cx),
            Sheet::Attach(target) => self.attach_sheet(target, &colors, cx),
            Sheet::Model => self.model_sheet(&colors, cx),
            Sheet::Project => self.project_sheet(&colors, cx),
            Sheet::More(id) => self.more_sheet(id, &colors, cx),
            Sheet::Models => self.models_sheet(&colors, cx),
            Sheet::Resources => self.resources_sheet(&colors, cx),
        };
        div()
            .id("sheet-body")
            .min_h_0()
            .overflow_y_scroll()
            .px(px(20.))
            .child(content)
            .into_any_element()
    }

    fn session(&self, id: SessionId) -> Option<&Session> {
        self.store.as_ref()?.session(id)
    }

    /// The models to offer, with a detail: the computer's once a session
    /// reported them, the sample's otherwise.
    fn offered_models(&self) -> Vec<(String, String)> {
        match self.store.as_ref().and_then(|store| store.live.as_ref()) {
            Some(live) => live
                .models
                .iter()
                .map(|model| {
                    let name = model.name.clone().unwrap_or_else(|| model.id.clone());
                    (name, model.provider.clone())
                })
                .collect(),
            None => MODELS
                .iter()
                .map(|(name, detail)| (name.to_string(), detail.to_string()))
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
            return div()
                .pb(px(8.))
                .child(self.sheet_title("Already answered", colors, cx))
                .child(
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
                    .flex()
                    .child(ui::badge("Needs you", colors.wait, colors.amber, colors)),
            )
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
            .child(self.sheet_title("Details", colors, cx))
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
        div()
            .pb(px(8.))
            .child(self.sheet_title("Add to the message", colors, cx))
            .child(
                ui::card(colors)
                    .mt(px(8.))
                    .child(
                        ui::row("choose-files", true, colors)
                            .child(icon("file", 20., colors.muted))
                            .child(ui::row_text(
                                "Choose files",
                                Some("Photos, documents and logs on this phone".into()),
                                colors,
                            ))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.attach_files(target, cx)),
                            ),
                    )
                    .child(
                        ui::row("paste", false, colors)
                            .child(icon("copy", 20., colors.muted))
                            .child(ui::row_text(
                                "Paste",
                                Some("An image or text from the clipboard".into()),
                                colors,
                            ))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.paste_into(target, cx)),
                            ),
                    ),
            )
    }

    fn model_sheet(&self, colors: &Theme, cx: &Context<Self>) -> Div {
        let prefs = self.prefs(cx);
        let offered = self.offered_models();
        let unknown = offered.is_empty();
        let models = offered
            .into_iter()
            .enumerate()
            .map(|(index, (name, detail))| {
                let on = prefs.model == name;
                ui::row(("model", index), index == 0, colors)
                    .child(ui::row_text(name.clone(), Some(detail.into()), colors))
                    .when(on, |row| row.child(icon("check", 20., colors.accent)))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.choose_model(name.clone(), cx)),
                    )
            });
        let levels = THINKING.iter().enumerate().map(|(index, level)| {
            let on = prefs.thinking == *level;
            ui::chip(("thinking", index), None, *level, colors)
                .when(on, |chip| {
                    chip.bg(colors.selected)
                        .border_color(colors.selected)
                        .text_color(colors.text)
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.update_prefs(cx, |prefs| prefs.thinking = level.to_string());
                    if let Some(live) = this.store.as_mut().and_then(|store| store.live.as_mut()) {
                        live.thinking = level.to_string();
                    }
                }))
        });
        div()
            .pb(px(8.))
            .child(self.sheet_title("Model", colors, cx))
            .when(unknown, |sheet| {
                sheet.child(
                    ui::hint(
                        format!(
                            "{}'s models show once a session there has started.",
                            self.computer_name()
                        ),
                        colors,
                    )
                    .mt(px(8.))
                    .mx(px(4.)),
                )
            })
            .when(!unknown, |sheet| {
                sheet.child(ui::card(colors).mt(px(8.)).children(models))
            })
            .child(
                ui::label("Thinking", colors)
                    .mt(px(18.))
                    .mb(px(8.))
                    .mx(px(4.)),
            )
            .child(div().flex().flex_wrap().gap(px(8.)).children(levels))
            .child(
                ui::hint(
                    format!(
                        "Models and sign-ins come from Pi on {}.",
                        self.computer_name()
                    ),
                    colors,
                )
                .mt(px(14.))
                .mx(px(4.)),
            )
            .child(
                ui::button("done", Button::Primary, None, "Done", false, colors)
                    .mt(px(16.))
                    .w_full()
                    .on_click(cx.listener(|this, _, _, cx| this.close_sheet(cx))),
            )
    }

    fn project_sheet(&self, colors: &Theme, cx: &Context<Self>) -> Div {
        let computer = self.computer_name();
        let projects = self
            .store
            .as_ref()
            .map(|store| store.projects.clone())
            .unwrap_or_default();
        let live = self.store.as_ref().is_some_and(|store| !store.is_sample());
        let field = div()
            .id("folder")
            .mt(px(8.))
            .h(px(48.))
            .px(px(14.))
            .flex()
            .items_center()
            .rounded(px(12.))
            .border_1()
            .border_color(colors.line_strong)
            .bg(colors.canvas)
            .child(div().flex_1().min_w_0().child(self.folder.clone()))
            .on_click(cx.listener(|this, _, window, cx| {
                let focus = this.folder.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }));
        div()
            .pb(px(8.))
            .child(self.sheet_title(format!("Projects on {computer}"), colors, cx))
            .when(projects.is_empty(), |sheet| {
                sheet.child(
                    ui::hint(
                        "No sessions on this computer yet. Type a project folder below.",
                        colors,
                    )
                    .mt(px(8.))
                    .mx(px(4.)),
                )
            })
            .when(!projects.is_empty(), |sheet| {
                sheet.child(ui::card(colors).mt(px(8.)).children(
                    projects.into_iter().enumerate().map(|(index, project)| {
                        ui::row(("project", index), index == 0, colors)
                            .child(icon("folder", 20., colors.muted))
                            .child(ui::row_text(
                                project.name,
                                Some(project.folder.into()),
                                colors,
                            ))
                            .when(self.project == index, |row| {
                                row.child(icon("check", 20., colors.accent))
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.project = index;
                                this.close_sheet(cx);
                            }))
                    }),
                ))
            })
            .when(live, |sheet| {
                sheet
                    .child(ui::label("Another folder", colors).mt(px(18.)).mx(px(4.)))
                    .child(field)
                    .child(
                        ui::hint(format!("Its full path on {computer}."), colors)
                            .mt(px(6.))
                            .mx(px(4.)),
                    )
            })
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
            .map(|summary| format!("{}\n\n{}", summary.headline, summary.body));
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
                "Copy the summary",
                colors.muted,
                Box::new(move |this, _, cx| this.copy(summary.clone(), "the summary", cx)),
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
        div()
            .pb(px(8.))
            .child(self.sheet_title(session.title.clone(), colors, cx))
            .child(
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
                                    cx.listener(move |this, _, window, cx| {
                                        action(this, window, cx)
                                    }),
                                )
                        },
                    )),
            )
    }

    fn models_sheet(&self, colors: &Theme, cx: &Context<Self>) -> Div {
        let computer = self.computer_name();
        div()
            .pb(px(8.))
            .child(self.sheet_title(format!("Models on {computer}"), colors, cx))
            .child(ui::card(colors).mt(px(8.)).children(self.offered_models().into_iter().enumerate().map(
                |(index, (name, detail))| {
                    ui::row(("known-model", index), index == 0, colors)
                        .child(icon("spark", 20., colors.muted))
                        .child(ui::row_text(name, Some(detail.into()), colors))
                },
            )))
            .child(
                ui::hint(
                    if self.live() {
                        format!("What durable sessions on {computer} can use; they show once a session there has started. Sign-ins stay on {computer}.")
                    } else {
                        format!("A sample list. Sign-ins stay on {computer}: change them in Pi Desktop or Pi’s settings there.")
                    },
                    colors,
                )
                .mt(px(14.))
                .mx(px(4.)),
            )
    }

    fn resources_sheet(&self, colors: &Theme, cx: &Context<Self>) -> Div {
        let computer = self.computer_name();
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
                .child(self.sheet_title(format!("Resources on {computer}"), colors, cx))
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
            .child(self.sheet_title(format!("Resources on {computer}"), colors, cx))
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
