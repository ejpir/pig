//! 02 Sessions: what needs you, what is working, and what finished today.

use super::{scroll_area, section};
use crate::{
    app::{PhoneApp, Route, Sheet},
    model::{Session, State, duration_label},
    motion::SwipeMotion,
    theme::{Theme, theme},
    ui::{self, icon},
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
            div()
                .h(px(56.))
                .flex_none()
                .flex()
                .items_center()
                .px(px(4.))
                .child(
                    ui::tap("navigation", "menu", &colors)
                        .on_click(cx.listener(|this, _, window, cx| this.open_drawer(window, cx))),
                )
                .child(
                    div()
                        .flex_1()
                        .pl(px(4.))
                        .text_size(px(22.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Sessions"),
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
        let list = div()
            .px(px(16.))
            .pb(px(96.))
            .when(!self.searching, |list| {
                list.child(
                    div().flex().child(
                        div()
                            .h(px(32.))
                            .max_w_full()
                            .min_w_0()
                            .pl(px(10.))
                            .pr(px(12.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .rounded_full()
                            .bg(colors.panel)
                            .text_size(px(13.))
                            .text_color(colors.secondary)
                            .child(div().size(px(8.)).rounded_full().bg(
                                if store.is_sample() || store.computer.connected {
                                    colors.green
                                } else {
                                    colors.wait
                                },
                            ))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .child(store.computer.name.clone()),
                            )
                            .child(div().flex_none().text_color(colors.muted).child(
                                if store.is_sample() {
                                    "· sample sessions"
                                } else if store.computer.connected {
                                    "· connected"
                                } else {
                                    "· reconnecting…"
                                },
                            )),
                    ),
                )
            })
            .when(!needs.is_empty(), |list| {
                list.child(section("Needs you", &colors).mt(px(18.))).child(
                    ui::card(&colors)
                        .border_color(colors.wait.opacity(0.45))
                        .children(needs.iter().enumerate().map(|(index, session)| {
                            self.session_row(
                                session,
                                index == 0,
                                index + 1 == needs.len(),
                                row_width,
                                &colors,
                                cx,
                            )
                        })),
                )
            })
            .when(!working.is_empty(), |list| {
                list.child(section("Working", &colors).mt(px(18.))).child(
                    ui::card(&colors).children(working.iter().enumerate().map(
                        |(index, session)| {
                            self.session_row(
                                session,
                                index == 0,
                                index + 1 == working.len(),
                                row_width,
                                &colors,
                                cx,
                            )
                        },
                    )),
                )
            })
            .when(!finished.is_empty(), |list| {
                list.child(section("Today", &colors).mt(px(18.)))
                    .child(ui::card(&colors).children(finished.iter().enumerate().map(
                        |(index, session)| {
                            self.session_row(
                                session,
                                index == 0,
                                index + 1 == finished.len(),
                                row_width,
                                &colors,
                                cx,
                            )
                        },
                    )))
            })
            .child(
                ui::hint(
                    match (found, store.is_sample()) {
                        (true, true) => format!(
                            "Earlier sessions on {} · {}",
                            store.computer.name, store.earlier
                        ),
                        (true, false) => format!(
                            "Sessions on {} that Pi Desktop or this phone started over SSH",
                            store.computer.name
                        ),
                        (false, _) if !query.is_empty() => {
                            "No matching sessions. Try a different title or project.".into()
                        }
                        (false, _) => format!(
                            "No sessions on {} yet. Start one with New session.",
                            store.computer.name
                        ),
                    },
                    &colors,
                )
                .mx(px(4.))
                .my(px(14.)),
            );
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(appbar)
            .child(scroll_area("sessions", &scroll).child(list))
            .when(!self.searching, |screen| {
                screen.child(
                    div()
                        .id("new-session")
                        .child(crate::testing::probe("new-session"))
                        .debug_selector(|| "new-session".into())
                        .occlude()
                        .absolute()
                        .right(px(16.))
                        .bottom(px(16.))
                        .h(px(56.))
                        .pl(px(18.))
                        .pr(px(22.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .rounded(px(18.))
                        .bg(colors.accent)
                        .text_color(colors.on_accent)
                        .font_weight(FontWeight::SEMIBOLD)
                        .shadow(vec![gpui::BoxShadow {
                            color: colors.shadow,
                            offset: gpui::point(px(0.), px(6.)),
                            blur_radius: px(16.),
                            spread_radius: px(0.),
                            inset: false,
                        }])
                        .active(|style| style.opacity(0.9))
                        .child(icon("plus", 20., colors.on_accent))
                        .child("New session")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.push(Route::Start, window, cx)),
                        ),
                )
            })
    }

    fn session_row(
        &self,
        session: &Session,
        first: bool,
        last: bool,
        width: gpui::Pixels,
        colors: &Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = session.id;
        let leading = match session.state {
            State::NeedsYou => ui::dot(colors.wait, true, colors).into_any_element(),
            State::Working => ui::working_indicator(colors).into_any_element(),
            State::Done => icon("check", 16., colors.green).into_any_element(),
            State::Stopped => icon("stop", 16., colors.muted).into_any_element(),
            State::Failed => icon("alert", 16., colors.coral).into_any_element(),
        };
        let trailing = match session.state {
            State::NeedsYou => {
                ui::badge("Answer", colors.wait, colors.amber, colors).into_any_element()
            }
            State::Working => ui::hint(duration_label(session.elapsed), colors).into_any_element(),
            _ => {
                ui::hint(session.finished_at.clone().unwrap_or_default(), colors).into_any_element()
            }
        };
        let detail: SharedString =
            format!("{} · {}", session.project, session.status_line()).into();
        let offset = self
            .swiping_session
            .as_ref()
            .filter(|(session, _)| *session == id)
            .map_or(px(0.), |(_, motion)| width * motion.position());
        div()
            .id(("swipe-session", id.0 as usize))
            .child(crate::testing::probe(format!("session-row-{}", id.0)))
            .debug_selector(move || format!("session-row-{}", id.0).into())
            .relative()
            .overflow_hidden()
            .bg(colors.coral)
            .when(first, |row| row.rounded_t(px(15.)))
            .when(last, |row| row.rounded_b(px(15.)))
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
                ui::row(("session", id.0 as usize), first, colors)
                    .relative()
                    .left(offset)
                    .bg(colors.panel)
                    .when(first, |row| row.rounded_t(px(15.)))
                    .when(last, |row| row.rounded_b(px(15.)))
                    .child(leading)
                    .child(ui::row_text(session.title.clone(), Some(detail), colors))
                    .child(trailing)
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
