//! A left navigation drawer with the connected computer and recent work.

use crate::{
    app::{PhoneApp, Route},
    model::State,
    motion::SwipeMotion,
    theme::theme,
    ui::{self, Button, icon},
};
use gpui::{AnyElement, Context, FontWeight, Window, div, prelude::*, px};

impl PhoneApp {
    pub(crate) fn open_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sheet = None;
        self.closing_sheet = None;
        self.sheet_motion = SwipeMotion::at(1.);
        self.drawer_open = true;
        self.drawer_motion.settle(0.);
        window.dismiss_virtual_keyboard();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(crate) fn drawer(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let colors = theme(cx);
        let insets = Self::insets(window);
        let sessions = self
            .store
            .as_ref()
            .map(|store| {
                store
                    .grouped()
                    .into_iter()
                    .flatten()
                    .take(30)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let computer = self
            .store
            .as_ref()
            .map(|store| store.computer.name.clone())
            .unwrap_or_else(|| "Connect a computer".into());
        let active = self.route();
        let width = px(320.).min(window.viewport_size().width - px(48.));
        let progress = self.drawer_motion.position();
        div()
            .absolute()
            .inset_0()
            .child(
                div()
                    .id("drawer-scrim")
                    .occlude()
                    .absolute()
                    .inset_0()
                    .bg(colors.scrim)
                    .opacity(1. - progress)
                    .on_click(cx.listener(|this, _, _, cx| this.close_drawer(cx))),
            )
            .child(
                div()
                    .id("navigation-drawer")
                    .child(crate::testing::probe("drawer-panel"))
                    .debug_selector(|| "navigation-drawer".into())
                    .occlude()
                    .absolute()
                    .left(-width * progress)
                    .top_0()
                    .bottom_0()
                    .w(width)
                    .bg(colors.canvas)
                    .border_r_1()
                    .border_color(colors.line)
                    .pt(insets.top)
                    .pb(insets.bottom)
                    .flex()
                    .flex_col()
                    .child(self.dismiss_gesture(true, cx))
                    .child(
                        div()
                            .flex_none()
                            .px(px(16.))
                            .py(px(12.))
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(ui::tile_box(40., 12., colors.accent, &colors).child(icon(
                                "pi",
                                24.,
                                colors.accent,
                            )))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_size(px(20.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("Pi"),
                                    )
                                    .child(ui::hint(computer, &colors).truncate()),
                            )
                            .child(
                                ui::tap("close-drawer", "x", &colors)
                                    .on_click(cx.listener(|this, _, _, cx| this.close_drawer(cx))),
                            ),
                    )
                    .child(
                        div().flex_none().px(px(16.)).pb(px(12.)).child(
                            ui::button(
                                "drawer-new",
                                Button::Primary,
                                Some("plus"),
                                "New session",
                                false,
                                &colors,
                            )
                            .w_full()
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.routes = vec![Route::Sessions];
                                    this.push(Route::Start, window, cx);
                                },
                            )),
                        ),
                    )
                    .child(
                        ui::row("all-sessions", true, &colors)
                            .mx(px(8.))
                            .rounded(px(12.))
                            .when(active == Route::Sessions, |row| row.bg(colors.selected))
                            .child(icon("layers", 20., colors.accent))
                            .child("All sessions")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_drawer(cx);
                                this.routes = vec![Route::Sessions];
                                window.focus(&this.focus, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        ui::label("Recent sessions", &colors)
                            .mx(px(20.))
                            .mt(px(20.))
                            .mb(px(8.)),
                    )
                    .child(
                        crate::scroll::vertical("drawer-sessions", &self.drawer_scroll)
                            .flex_1()
                            .min_h_0()
                            .px(px(8.))
                            .children(sessions.iter().map(|session| {
                                let id = session.id;
                                let color = match session.state {
                                    State::NeedsYou => colors.wait,
                                    State::Working => colors.read,
                                    State::Failed => colors.coral,
                                    _ => colors.muted,
                                };
                                ui::row(("drawer-session", id.0 as usize), true, &colors)
                                    .rounded(px(12.))
                                    .when(
                                        active == Route::Thread(id) || active == Route::Review(id),
                                        |row| row.bg(colors.selected),
                                    )
                                    .child(icon(
                                        if session.state == State::Working {
                                            "clock"
                                        } else {
                                            "chat"
                                        },
                                        18.,
                                        color,
                                    ))
                                    .child(ui::row_text(
                                        session.title.clone(),
                                        Some(
                                            format!(
                                                "{} · {}",
                                                session.project,
                                                session.status_line()
                                            )
                                            .into(),
                                        ),
                                        &colors,
                                    ))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.show_session(id, window, cx)
                                    }))
                            })),
                    )
                    .child(
                        ui::row("drawer-settings", false, &colors)
                            .flex_none()
                            .child(icon("settings", 20., colors.muted))
                            .child("Settings and tools")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.routes = vec![Route::Sessions];
                                this.push(Route::Settings, window, cx);
                            })),
                    ),
            )
            .when(!self.drawer_open, |overlay| {
                overlay.child(
                    div()
                        .id("closing-drawer-blocker")
                        .absolute()
                        .inset_0()
                        .occlude(),
                )
            })
            .into_any_element()
    }
}
