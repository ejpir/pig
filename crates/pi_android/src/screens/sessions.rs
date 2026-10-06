//! 02 Home: the computer as the title, Pi's question first, working runs
//! drawn to time on one shared track, today's results, and the start bar.

use super::scroll_area;
use crate::{
    app::{PhoneApp, Route, Sheet},
    model::{Session, State, duration_label},
    motion::SwipeMotion,
    theme::{Theme, theme},
    ui::{self, Button, icon},
};
use gpui::{
    Context, Div, Focusable, FontWeight, SharedString, Stateful, TouchPhase, Window, div,
    prelude::*, px,
};

impl PhoneApp {
    pub(crate) fn sessions_screen(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = theme(cx);
        let row_width = (window.viewport_size().width - px(34.)).max(px(1.));
        let scroll = self.scroll(Route::Sessions);
        let query = self.search.read(cx).text().trim().to_lowercase();
        let appbar = if self.searching {
            div()
                .h(px(56.))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(4.))
                .px(px(4.))
                .child(
                    ui::tap("close-search", "back", &colors).on_click(cx.listener(
                        |this, _, window, cx| {
                            this.back(window, cx);
                        },
                    )),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .mr(px(12.))
                        .h(px(44.))
                        .px(px(14.))
                        .flex()
                        .items_center()
                        .rounded_full()
                        .bg(colors.panel)
                        .child(div().flex_1().min_w_0().child(self.search.clone()))
                        .when(!query.is_empty(), |field| {
                            field.child(ui::tap("clear-search", "x", &colors).on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.search.update(cx, |area, cx| area.set_text("", cx));
                                    window.focus(&this.search.read(cx).focus_handle(cx), cx);
                                    cx.notify();
                                }),
                            ))
                        }),
                )
        } else {
            let (name, status, status_hue) = match &self.store {
                Some(store) if store.is_sample() => (
                    store.computer.name.clone(),
                    "Sample sessions",
                    colors.green,
                ),
                Some(store) if store.computer.connected => {
                    (store.computer.name.clone(), "Connected", colors.green)
                }
                Some(store) => (store.computer.name.clone(), "Reconnecting…", colors.wait),
                None => ("Pi".into(), "Not connected", colors.muted),
            };
            div()
                .h(px(56.))
                .flex_none()
                .flex()
                .items_center()
                .pl(px(12.))
                .pr(px(4.))
                .child(
                    div()
                        .id("computers")
                        .relative()
                        .child(crate::testing::probe("computers"))
                        .flex_1()
                        .min_w_0()
                        .h(px(48.))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .rounded(px(12.))
                        .child(
                            ui::tile_box(40., 12., colors.accent, &colors)
                                .child(icon("pi", 16., colors.accent)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(4.))
                                        .child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .text_size(px(18.))
                                                .line_height(px(24.))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(name),
                                        )
                                        .child(icon("chev_d", 16., colors.muted)),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(8.))
                                        .text_size(px(12.5))
                                        .line_height(px(16.))
                                        .text_color(colors.muted)
                                        .child(div().size(px(6.)).rounded_full().bg(status_hue))
                                        .child(status),
                                ),
                        )
                        .on_click(
                            cx.listener(|this, _, _, cx| this.open_sheet(Sheet::Computers, cx)),
                        ),
                )
                .child(ui::tap("search", "search", &colors).on_click(cx.listener(
                    |this, _, window, cx| {
                        this.searching = true;
                        let focus = this.search.read(cx).focus_handle(cx);
                        window.focus(&focus, cx);
                        cx.notify();
                    },
                )))
                .child(ui::tap("settings", "settings", &colors).on_click(
                    cx.listener(|this, _, window, cx| this.push(Route::Settings, window, cx)),
                ))
        };
        let Some(store) = &self.store else {
            return div().flex_1().child(appbar);
        };
        let matches = |session: &&Session| {
            query.is_empty()
                || session.title.to_lowercase().contains(&query)
                || session.project.to_lowercase().contains(&query)
        };
        let [needs, working, finished] = store
            .grouped()
            .map(|group| group.into_iter().filter(matches).collect::<Vec<_>>());
        let found = !(needs.is_empty() && working.is_empty() && finished.is_empty());
        // Working runs share one track: five minutes wide, or the longest run.
        let span = working
            .iter()
            .filter_map(|session| session.turn())
            .map(|turn| turn.times.iter().sum::<std::time::Duration>())
            .max()
            .unwrap_or_default()
            .max(std::time::Duration::from_secs(300));
        let label = |text: &'static str| {
            div()
                .text_size(px(12.5))
                .line_height(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors.muted)
                .child(text)
        };
        let earlier = (store.earlier > 0 && !self.searching).then(|| {
            div()
                .min_h(px(56.))
                .flex()
                .items_center()
                .gap(px(12.))
                .border_t_1()
                .border_color(colors.line)
                .text_color(colors.accent)
                .child(
                    div()
                        .w(px(32.))
                        .flex_none()
                        .flex()
                        .justify_center()
                        .child(icon("clock", 16., colors.accent)),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Earlier sessions"),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(colors.muted)
                        .child(store.earlier.to_string()),
                )
                .child(icon("chev_r", 16., colors.accent))
        });
        let list = div()
            .px(px(20.))
            .pt(px(12.))
            .pb(px(24.))
            .flex()
            .flex_col()
            .children(needs.iter().enumerate().map(|(index, session)| {
                self.home_question(session, &colors, cx)
                    .when(index > 0, |card| card.mt(px(12.)))
            }))
            .when(!working.is_empty(), |list| {
                list.child(label("Working").mt(px(if needs.is_empty() { 4. } else { 24. })))
                    .children(working.iter().map(|session| {
                        self.session_row(session, true, span, row_width, &colors, cx)
                    }))
            })
            .when(!finished.is_empty() || earlier.is_some(), |list| {
                list.child(
                    label("Today").mt(px(if needs.is_empty() && working.is_empty() {
                        4.
                    } else {
                        16.
                    })),
                )
                .children(finished.iter().enumerate().map(|(index, session)| {
                    self.session_row(session, index == 0, span, row_width, &colors, cx)
                }))
                .children(earlier)
            })
            .when(!found || !store.is_sample() && !self.searching, |list| {
                list.child(
                    ui::hint(
                        match found {
                            true => format!(
                                "Sessions on {} that Pi Desktop or this phone started over SSH",
                                store.computer.name
                            ),
                            false if !query.is_empty() => {
                                "No matching sessions. Try a different title or project.".into()
                            }
                            false => format!(
                                "No sessions on {} yet. Start one below.",
                                store.computer.name
                            ),
                        },
                        &colors,
                    )
                    .my(px(14.)),
                )
            });
        let project = store
            .projects
            .get(self.project)
            .map(|project| project.name.clone());
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(appbar)
            .child(scroll_area("sessions", &scroll).child(list))
            .when(!self.searching, |screen| {
                screen.child(self.start_bar(project, &colors, cx))
            })
    }

    /// The start bar at the bottom of Home: tapping it starts a new session.
    fn start_bar(
        &self,
        project: Option<String>,
        colors: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div().flex_none().px(px(12.)).pt(px(8.)).pb(px(12.)).child(
            div()
                .id("new-session")
                .child(crate::testing::probe("new-session"))
                .debug_selector(|| "new-session".into())
                .h(px(56.))
                .pl(px(20.))
                .pr(px(8.))
                .flex()
                .items_center()
                .rounded(px(28.))
                .bg(colors.composer)
                .border_1()
                .border_color(colors.line_strong)
                .active(|style| style.bg(colors.selected))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(colors.faint)
                        .child("What should we change?"),
                )
                .children(project.map(|project| {
                    div()
                        .id("start-project")
                        .mr(px(4.))
                        .h(px(28.))
                        .px(px(10.))
                        .max_w(px(140.))
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(6.))
                        .rounded_full()
                        .bg(colors.chip)
                        .border_1()
                        .border_color(colors.line)
                        .text_size(px(12.5))
                        .text_color(colors.secondary)
                        .child(icon("folder", 14., colors.secondary))
                        .child(div().min_w_0().truncate().child(project))
                        .on_click(cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.push(Route::Start, window, cx);
                            this.open_sheet(Sheet::Project, cx);
                        }))
                }))
                .child(
                    div()
                        .size(px(40.))
                        .m(px(4.))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(colors.accent)
                        .child(icon("plus", 20., colors.on_accent)),
                )
                .on_click(cx.listener(|this, _, window, cx| this.push(Route::Start, window, cx))),
        )
    }

    /// Pi's question first on Home, with its command and Answer.
    fn home_question(&self, session: &Session, colors: &Theme, cx: &mut Context<Self>) -> Stateful<Div> {
        let id = session.id;
        let question = session.question.as_ref();
        let open = cx.listener(move |this, _, window, cx| this.show_session(id, window, cx));
        div()
            .id(("question-card", id.0 as usize))
            .relative()
            .child(crate::testing::probe(format!("question-card-{}", id.0)))
            .p(px(16.))
            .rounded(px(16.))
            .bg(colors.composer)
            .border_1()
            .border_color(colors.line.blend(colors.wait.opacity(0.45)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(ui::tile_box(32., 10., colors.wait, colors).child(icon("hand", 16., colors.wait)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(session.title.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .line_height(px(16.))
                                    .text_color(colors.muted)
                                    .truncate()
                                    .child(format!("{} · waiting for you", session.project)),
                            ),
                    ),
            )
            .child(
                div()
                    .mt(px(12.))
                    .line_height(px(22.))
                    .child(question.map_or_else(|| "Pi is waiting for you".to_owned(), |q| q.title.clone())),
            )
            .child(
                div()
                    .mt(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .children(question.filter(|q| !q.command.is_empty()).map(|question| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(px(40.))
                            .px(px(12.))
                            .flex()
                            .items_center()
                            .rounded(px(12.))
                            .bg(colors.panel)
                            .child(
                                ui::mono(question.command.clone(), 12.5)
                                    .min_w_0()
                                    .truncate()
                                    .text_color(colors.secondary),
                            )
                    }))
                    .when(question.is_none_or(|q| q.command.is_empty()), |row| row.child(div().flex_1()))
                    .child(
                        ui::button(("answer", id.0 as usize), Button::Primary, None, "Answer", true, colors)
                            .on_click(cx.listener(move |this, _, window, cx| this.show_session(id, window, cx))),
                    ),
            )
            .on_click(open)
    }

    fn session_row(
        &self,
        session: &Session,
        first: bool,
        span: std::time::Duration,
        width: gpui::Pixels,
        colors: &Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = session.id;
        let working = session.state == State::Working;
        let leading = match session.state {
            State::NeedsYou => ui::ring_dot(colors.wait).into_any_element(),
            State::Working => ui::ring_dot(colors.read).into_any_element(),
            State::Done => icon("check", 16., colors.green).into_any_element(),
            State::Stopped => icon("stop", 16., colors.muted).into_any_element(),
            State::Failed => icon("alert", 16., colors.coral).into_any_element(),
        };
        let time = if working {
            duration_label(session.elapsed)
        } else {
            session.finished_at.clone().unwrap_or_default()
        };
        let detail: SharedString = if working {
            let stage = session
                .turn()
                .and_then(|turn| turn.live_stage())
                .map(|kind| kind.name(crate::model::StageStatus::Live));
            match stage {
                Some(stage) if !session.activity.contains(stage) => {
                    format!("{} · {stage} · {}", session.project, session.activity)
                }
                _ => format!("{} · {}", session.project, session.activity),
            }
        } else {
            format!("{} · {}", session.project, session.status_line())
        }
        .into();
        let meta = |text: SharedString| {
            div()
                .text_size(px(12.5))
                .line_height(px(16.))
                .text_color(colors.muted)
                .truncate()
                .child(text)
        };
        let offset = self
            .swiping_session
            .as_ref()
            .filter(|(session, _)| *session == id)
            .map_or(px(0.), |(_, motion)| width * motion.position());
        let swiping = offset > px(0.);
        div()
            .id(("swipe-session", id.0 as usize))
            .child(crate::testing::probe(format!("session-row-{}", id.0)))
            .debug_selector(move || format!("session-row-{}", id.0).into())
            .relative()
            .overflow_hidden()
            .when(!working && !first, |row| row.border_t_1().border_color(colors.line))
            .when(swiping, |row| row.bg(colors.coral).rounded(px(12.)))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .pl(px(20.))
                    .text_color(colors.canvas)
                    .child(icon("trash", 20., colors.canvas))
                    .child(if offset >= (width * 0.42).max(px(110.)) {
                        "Release to delete"
                    } else {
                        "Delete"
                    }),
            )
            .child(
                div()
                    .id(("session", id.0 as usize))
                    .relative()
                    .left(offset)
                    .min_h(px(64.))
                    .py(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .bg(colors.canvas)
                    .active(|style| style.bg(colors.selected))
                    .child(div().w(px(32.)).flex_none().flex().justify_center().child(leading))
                    .child(if working {
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(session.title.clone()),
                                    )
                                    .child(meta(time.into())),
                            )
                            .children(session.turn().map(|turn| {
                                ui::track(&turn.times, turn.live_stage(), span, colors).mt(px(8.))
                            }))
                            .child(meta(detail).mt(px(6.)))
                    } else {
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .truncate()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(session.title.clone()),
                                    )
                                    .child(meta(detail).mt(px(2.))),
                            )
                            .child(meta(time.into()).flex_none())
                    })
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.show_session(id, window, cx)),
                    ),
            )
            .on_scroll_wheel(
                cx.listener(move |this, event: &gpui::ScrollWheelEvent, _, cx| {
                    let delta = event.delta.pixel_delta(px(20.));
                    if event.touch_phase == TouchPhase::Started {
                        if delta.x <= px(0.) || delta.x.abs() < delta.y.abs() {
                            return;
                        }
                        if !this
                            .swiping_session
                            .as_ref()
                            .is_some_and(|(session, _)| *session == id)
                        {
                            this.swiping_session = Some((id, SwipeMotion::at(0.)));
                        }
                        this.swiping_session.as_mut().unwrap().1.begin_drag();
                    }
                    let Some((session, motion)) = this.swiping_session.as_mut() else {
                        return;
                    };
                    if *session != id || !motion.dragging() {
                        return;
                    }
                    cx.stop_propagation();
                    motion.drag_by(delta.x / width);
                    if event.touch_phase == TouchPhase::Cancelled {
                        motion.settle(0.);
                    } else if event.touch_phase == TouchPhase::Ended {
                        let confirm = width * motion.position() >= (width * 0.42).max(px(110.));
                        motion.settle(0.);
                        if confirm {
                            this.open_sheet(Sheet::Delete(id), cx);
                        }
                    }
                    cx.notify();
                }),
            )
    }
}
