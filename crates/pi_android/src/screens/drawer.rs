//! A left navigation drawer with the connected computer and recent work.

use crate::{
    app::{PhoneApp, Route},
    model::State,
    motion::SwipeMotion,
    theme::theme,
    ui::{self, icon},
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
        // Leave enough of the current screen visible that this still reads as
        // navigation, rather than a second full-screen destination.
        let width = px(300.).min(window.viewport_size().width - px(56.));
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
                            .h(px(64.))
                            .px(px(12.))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(ui::tile_box(32., 10., colors.accent, &colors).child(icon(
                                "pi",
                                20.,
                                colors.accent,
                            )))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_size(px(17.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("Pi"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(colors.muted)
                                            .truncate()
                                            .child(computer),
                                    ),
                            )
                            .child(
                                ui::tap("close-drawer", "x", &colors)
                                    .on_click(cx.listener(|this, _, _, cx| this.close_drawer(cx))),
                            ),
                    )
                    .child(
                        div()
                            .id("drawer-new")
                            .relative()
                            .child(crate::testing::probe("drawer-new"))
                            .mx(px(8.))
                            .mb(px(4.))
                            .h(px(48.))
                            .px(px(12.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .rounded(px(12.))
                            .bg(colors.tint(colors.accent))
                            .text_color(colors.accent)
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .active(|style| style.bg(colors.selected))
                            .child(icon("plus", 18., colors.accent))
                            .child("New session")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.routes = vec![Route::Sessions];
                                this.push(Route::Start, window, cx);
                            })),
                    )
                    .child(
                        ui::row("all-sessions", true, &colors)
                            .mx(px(8.))
                            .min_h(px(48.))
                            .py(px(4.))
                            .gap(px(12.))
                            .rounded(px(12.))
                            .when(active == Route::Sessions, |row| row.bg(colors.selected))
                            .child(icon("layers", 18., colors.accent))
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
                            .mx(px(16.))
                            .mt(px(16.))
                            .mb(px(6.)),
                    )
                    .child(
                        crate::scroll::vertical("drawer-sessions", &self.drawer_scroll)
                            .flex_1()
                            .min_h_0()
                            .px(px(8.))
                            .children(sessions.iter().map(|session| {
                                let id = session.id;
                                let is_active =
                                    active == Route::Thread(id) || active == Route::Review(id);
                                let color = match session.state {
                                    State::NeedsYou => colors.wait,
                                    State::Working => colors.read,
                                    State::Failed => colors.coral,
                                    _ => colors.muted,
                                };
                                ui::row(("drawer-session", id.0 as usize), true, &colors)
                                    .min_h(px(52.))
                                    .px(px(12.))
                                    .py(px(5.))
                                    .gap(px(10.))
                                    .rounded(px(12.))
                                    .when(is_active, |row| row.bg(colors.selected))
                                    .child(ui::dot(
                                        color,
                                        matches!(session.state, State::NeedsYou | State::Working),
                                        &colors,
                                    ))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .text_size(px(14.))
                                                    .font_weight(if is_active {
                                                        FontWeight::SEMIBOLD
                                                    } else {
                                                        FontWeight::NORMAL
                                                    })
                                                    .truncate()
                                                    .child(session.title.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(12.))
                                                    .text_color(colors.muted)
                                                    .truncate()
                                                    .child(format!(
                                                        "{} · {}",
                                                        session.project,
                                                        session.status_line()
                                                    )),
                                            ),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.show_session(id, window, cx)
                                    }))
                            })),
                    )
                    .child(
                        ui::row("drawer-settings", false, &colors)
                            .flex_none()
                            .min_h(px(52.))
                            .px(px(16.))
                            .py(px(4.))
                            .gap(px(12.))
                            .border_t_1()
                            .border_color(colors.line)
                            .text_size(px(14.))
                            .child(icon("settings", 18., colors.muted))
                            .child("Settings")
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
