//! 04–06 A session: each prompt, the stages Pi went through, and the hand-off.
//! The run that is going shows its stages in full; finished ones fold into a
//! row of tiles with the summary under it.

use super::scroll_area;
use crate::{
    app::{PhoneApp, Route, Sheet},
    model::{
        CheckResult, Reference, Session, SessionId, Stage, StageKind, StageStatus, State, Turn,
        duration_label,
    },
    theme::{Theme, theme},
    ui::{self, Button, icon},
};
use gpui::{AnyElement, Context, Div, FontWeight, Window, div, prelude::*, px, relative};

impl PhoneApp {
    pub(crate) fn thread_screen(
        &mut self,
        id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = theme(cx);
        let scroll = self.scroll(Route::Thread(id));
        let composer = self.thread_composer(id, window, cx);
        let back = ui::tap("back", "back", &colors).on_click(cx.listener(|this, _, window, cx| {
            this.back(window, cx);
        }));
        let Some(store) = &self.store else {
            return div().into_any_element();
        };
        let Some(session) = store.session(id) else {
            return div()
                .flex_1()
                .child(ui::appbar(back, "Session", None, &colors))
                .child(ui::hint("This session is no longer on this phone.", &colors).px(px(20.)))
                .into_any_element();
        };
        let running = session.state.is_running();
        let area = composer.read(cx).area.clone();
        area.update(cx, |area, _| {
            area.set_placeholder(if running {
                "Queue a follow-up…"
            } else {
                "Ask a follow-up…"
            })
        });
        let subtitle = match session.state {
            State::NeedsYou | State::Working => {
                format!("{} · {}", session.project, store.computer.name)
            }
            State::Done => format!(
                "{} · done in {}",
                session.project,
                duration_label(session.elapsed)
            ),
            State::Stopped => format!("{} · stopped by you", session.project),
            State::Failed => format!("{} · {}", session.project, session.status_line()),
        };
        let appbar =
            ui::appbar(back, session.title.clone(), Some(subtitle.into()), &colors)
                .child(ui::tap("details", "info", &colors).on_click(
                    cx.listener(move |this, _, _, cx| this.open_sheet(Sheet::Details(id), cx)),
                ))
                .child(ui::tap("more", "dots", &colors).on_click(
                    cx.listener(move |this, _, _, cx| this.open_sheet(Sheet::More(id), cx)),
                ));
        let last = session.turns.len().saturating_sub(1);
        let turns = session
            .turns
            .iter()
            .enumerate()
            .map(|(index, turn)| {
                let live = index == last && running;
                let ending = (index == last && !running).then(|| self.ending(session, cx));
                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(prompt(turn, &colors))
                    .child(if live {
                        rail(turn, &colors)
                    } else {
                        folded(turn, index == last, &colors)
                    })
                    .children(ending)
            })
            .collect::<Vec<_>>();
        let queued = (!session.queued.is_empty()).then(|| {
            div()
                .child(ui::label("Queued for when this run ends", &colors).mb(px(8.)))
                .child(
                    ui::card(&colors).children(session.queued.iter().enumerate().map(
                        |(index, prompt)| {
                            ui::row(("queued", index), index == 0, &colors)
                                .child(icon("queue", 16., colors.muted))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_size(px(14.))
                                        .child(prompt.clone()),
                                )
                                .child(
                                    ui::tap(("unqueue", index), "x", &colors)
                                        .size(px(40.))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(store) = &mut this.store {
                                                store.unqueue(id, index);
                                            }
                                            cx.notify();
                                        })),
                                )
                        },
                    )),
                )
        });
        let status = running.then(|| status_bar(session, &colors, cx));
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(appbar)
            .child(
                scroll_area(("thread", id.0 as usize), &scroll).child(
                    div()
                        .px(px(20.))
                        .pt(px(6.))
                        .pb(px(20.))
                        .flex()
                        .flex_col()
                        .gap(px(28.))
                        .children(turns)
                        .children(queued),
                ),
            )
            .children(status)
            .child(div().flex_none().pb(px(10.)).child(composer))
            .into_any_element()
    }

    /// Under a finished run: the changed files, the check, and Review.
    fn ending(&self, session: &Session, cx: &Context<Self>) -> Div {
        let colors = theme(cx);
        let id = session.id;
        let check = session.check.as_ref().map(|check| {
            let (color, result) = match check.result {
                CheckResult::Passed => (colors.green, "passed"),
                CheckResult::Failed => (colors.coral, "failed"),
                CheckResult::NotRun => (colors.muted, "not run"),
            };
            ui::row("check", session.files.is_empty(), &colors)
                .child(icon("shield", 16., color))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(check.command.clone()),
                )
                .child(ui::hint(result, &colors))
        });
        let files = session.files.iter().enumerate().map(|(index, file)| {
            ui::row(("file", index), index == 0, &colors)
                .child(icon("file", 16., colors.muted))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(file.name().to_owned()),
                )
                .child(ui::counts(file.added, file.removed, &colors))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.open_review(id, index, window, cx)),
                )
        });
        let review = (!session.files.is_empty()).then(|| {
            let text = match session.files.len() {
                1 => "Review the changed file".to_owned(),
                count => format!("Review {count} changed files"),
            };
            ui::button(
                "review",
                Button::Primary,
                Some("diff"),
                text,
                false,
                &colors,
            )
            .w_full()
            .on_click(cx.listener(move |this, _, window, cx| this.open_review(id, 0, window, cx)))
        });
        let failure = (session.state == State::Failed).then(|| {
            ui::card(&colors)
                .border_color(colors.coral.opacity(0.4))
                .p(px(14.))
                .flex()
                .gap(px(12.))
                .child(icon("alert", 16., colors.coral))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(14.))
                        .child(capitalized(&session.status_line()))
                        .child(ui::hint(
                            "Nothing was changed. Ask again to retry.",
                            &colors,
                        )),
                )
        });
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .children(failure)
            .when(session.state == State::Stopped, |ending| {
                ending.child(ui::hint(
                    "Stopped by you. Changes made before stopping stay on the computer.",
                    &colors,
                ))
            })
            .when(!session.files.is_empty() || check.is_some(), |ending| {
                ending.child(ui::card(&colors).children(files).children(check))
            })
            .children(review)
    }
}

/// "You · 09:41" and what was asked.
fn prompt(turn: &Turn, colors: &Theme) -> Div {
    div()
        .child(ui::label(format!("You · {}", turn.at), colors))
        .child(div().mt(px(4.)).child(turn.prompt.clone()))
        .when(!turn.attachments.is_empty(), |prompt| {
            prompt.child(
                div().mt(px(8.)).flex().flex_wrap().gap(px(8.)).children(
                    turn.attachments
                        .iter()
                        .enumerate()
                        .map(|(index, attachment)| {
                            ui::chip(("sent", index), Some("clip"), attachment.clone(), colors)
                        }),
                ),
            )
        })
}

/// A run in progress: every stage, with what it read and its latest edit.
fn rail(turn: &Turn, colors: &Theme) -> Div {
    let count = turn.stages.len();
    div().flex().flex_col().gap(px(16.)).children(
        turn.stages
            .iter()
            .enumerate()
            .map(|(index, stage)| stage_row(stage, index + 1 < count, colors)),
    )
}

fn stage_row(stage: &Stage, line_below: bool, colors: &Theme) -> Div {
    let ahead = matches!(stage.status, StageStatus::Planned | StageStatus::Skipped);
    let references = match (stage.kind, stage.status) {
        (StageKind::Change, StageStatus::Live) if !stage.diff.is_empty() => Vec::new(),
        _ => stage.references.clone(),
    };
    let diff = (stage.kind == StageKind::Change
        && stage.status == StageStatus::Live
        && !stage.diff.is_empty())
    .then(|| {
        ui::card(colors)
            .mt(px(10.))
            .py(px(6.))
            .rounded(px(12.))
            .children(
                stage
                    .diff
                    .iter()
                    .map(|line| ui::diff_line(line, false, colors)),
            )
    });
    div()
        .relative()
        .flex()
        .items_start()
        .gap(px(12.))
        .when(line_below, |row| {
            let line = div()
                .absolute()
                .left(px(15.))
                .top(px(36.))
                .bottom(px(-14.))
                .w(px(1.))
                .overflow_hidden();
            row.child(if stage.status == StageStatus::Done {
                line.bg(colors.line)
            } else {
                // Dashed ahead; a one-sided dashed border does not draw this thin.
                line.flex()
                    .flex_col()
                    .gap(px(3.))
                    .children((0..80).map(|_| div().flex_none().h(px(3.)).bg(colors.line_strong)))
            })
        })
        .child(ui::tile(stage.kind, stage.status, colors))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .pt(px(5.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(14.))
                                .font_weight(if ahead {
                                    FontWeight::NORMAL
                                } else {
                                    FontWeight::SEMIBOLD
                                })
                                .text_color(if ahead { colors.muted } else { colors.text })
                                .child(stage.kind.name(stage.status)),
                        )
                        .child(ui::counts(stage.added, stage.removed, colors)),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(colors.muted)
                        .child(stage.what.clone()),
                )
                .when(!references.is_empty(), |body| {
                    body.child(
                        div().mt(px(8.)).flex().flex_wrap().gap(px(8.)).children(
                            references
                                .into_iter()
                                .enumerate()
                                .map(|(index, reference)| {
                                    let (glyph, text) = match reference {
                                        Reference::File(file) => ("file", file),
                                        Reference::Search(search) => ("search", search),
                                    };
                                    ui::chip(("reference", index), Some(glyph), text, colors)
                                }),
                        ),
                    )
                })
                .children(diff),
        )
}

/// A finished run: its stages as a row of tiles, then Pi's summary.
fn folded(turn: &Turn, latest: bool, colors: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .children(
                    turn.stages
                        .iter()
                        .map(|stage| ui::tile(stage.kind, stage.status, colors)),
                )
                .child(
                    ui::hint(turn.digest(), colors)
                        .ml(px(4.))
                        .flex_1()
                        .min_w_0()
                        .truncate(),
                ),
        )
        .children(turn.summary.as_ref().map(|summary| {
            div()
                .child(
                    ui::serif(summary.headline.clone(), if latest { 21. } else { 18. })
                        .line_height(relative(1.3)),
                )
                .child(
                    div()
                        .mt(px(10.))
                        .text_color(colors.secondary)
                        .child(summary.body.clone()),
                )
        }))
}

/// Above the composer while a run goes: what it does, and Stop or Answer.
fn status_bar(session: &Session, colors: &Theme, cx: &Context<PhoneApp>) -> Div {
    let id = session.id;
    let waiting = session.state == State::NeedsYou;
    let (title, detail) = if waiting {
        (
            "Needs you",
            session
                .question
                .as_ref()
                .map_or_else(String::new, |question| question.title.clone()),
        )
    } else {
        (
            "Working",
            format!("{} · {}", session.activity, duration_label(session.elapsed)),
        )
    };
    let action: AnyElement = if waiting {
        ui::button("answer", Button::Primary, None, "Answer", true, colors)
            .on_click(cx.listener(move |this, _, _, cx| this.open_sheet(Sheet::Question(id), cx)))
            .into_any_element()
    } else {
        ui::button("stop", Button::Plain, Some("stop"), "Stop", true, colors)
            .text_color(colors.coral)
            .on_click(cx.listener(move |this, _, _, cx| this.stop(id, cx)))
            .into_any_element()
    };
    div()
        .flex_none()
        .px(px(16.))
        .pt(px(10.))
        .pb(px(8.))
        .flex()
        .items_center()
        .gap(px(12.))
        .border_t_1()
        .border_color(colors.line)
        .child(ui::dot(
            if waiting { colors.wait } else { colors.read },
            true,
            colors,
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(ui::hint(detail, colors).mt(px(-2.)).truncate()),
        )
        .child(action)
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}
