//! 01 Connect: where Pi runs, in three steps.

use super::{heading, scroll_area};
use crate::{
    app::{PhoneApp, Route},
    theme::{MONO, SANS, Theme, theme},
    ui::{self, Button, icon},
};
use gpui::{
    AnyElement, Context, Div, Focusable, FontWeight, StyledText, TextRun, Window, div, prelude::*,
    px, relative,
};

impl PhoneApp {
    pub(crate) fn connect_screen(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = theme(cx);
        let scroll = self.scroll(Route::Connect);
        let entered = !self.address.read(cx).text().trim().is_empty();
        let field = div()
            .id("address")
            .mt(px(8.))
            .h(px(48.))
            .px(px(14.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(12.))
            .border_1()
            .border_color(if self.connect_error.is_some() && !self.key_refused {
                colors.coral
            } else {
                colors.line_strong
            })
            .bg(colors.canvas)
            .child(div().flex_1().min_w_0().child(self.address.clone()))
            .child(icon("computer", 16., colors.muted))
            .on_click(cx.listener(|this, _, window, cx| {
                let focus = this.address.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }));
        let key = match &self.phone_key {
            Some(key) => div()
                .mt(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .px(px(12.))
                        .py(px(10.))
                        .rounded(px(12.))
                        .bg(colors.panel)
                        .child(
                            ui::mono(key.clone(), 12.)
                                .text_color(colors.secondary)
                                .truncate(),
                        ),
                )
                .child({
                    let key = key.clone();
                    ui::button(
                        "copy-key",
                        Button::Plain,
                        Some("copy"),
                        "Copy",
                        true,
                        &colors,
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.copy(key.clone(), "the key", cx)),
                    )
                }),
            None => div()
                .mt(px(8.))
                .px(px(12.))
                .py(px(10.))
                .rounded(px(12.))
                .bg(colors.panel)
                .text_size(px(13.))
                .text_color(colors.muted)
                .child("Made on this phone when you first connect"),
        };
        let steps = ui::card(&colors)
            .mt(px(24.))
            .child(
                step(Some(entered && (self.connect_error.is_none() || self.key_refused)), 1, &colors).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(ui::label("Computer", &colors))
                        .child(field)
                        .children(self.connect_error.clone().filter(|_| !self.key_refused).map(|error| {
                            ui::hint(error, &colors).mt(px(8.)).text_color(colors.coral)
                        })),
                ),
            )
            .child(
                step(None, 2, &colors).border_t_1().child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(ui::label("This phone’s key", &colors))
                        .child(key)
                        .when(self.key_refused, |step| {
                            step.child(
                                ui::hint("The computer turned this key down. Copy it, add it there as below, then connect again.", &colors)
                                    .mt(px(8.))
                                    .text_color(colors.coral),
                            )
                        })
                        .child(
                            div()
                                .mt(px(8.))
                                .text_size(px(13.))
                                .line_height(relative(1.4))
                                .child(key_hint(&colors)),
                        ),
                ),
            )
            .child(
                step(None, 3, &colors).border_t_1().child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(ui::label("Pi on the computer", &colors))
                        .child(ui::hint(
                            "The phone uses Pi Desktop’s helper there, built with durable sessions. Sign-ins and settings stay on the computer.",
                            &colors,
                        ).mt(px(4.))),
                ),
            );
        let connect = if self.connecting {
            ui::disabled(
                ui::button(
                    "connect",
                    Button::Primary,
                    None,
                    "Connecting…",
                    false,
                    &colors,
                ),
                &colors,
            )
        } else {
            ui::button("connect", Button::Primary, None, "Connect", false, &colors)
                .on_click(cx.listener(|this, _, window, cx| this.connect(window, cx)))
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                scroll_area("connect", &scroll).child(
                    div()
                        .px(px(20.))
                        .pt(px(44.))
                        .pb(px(12.))
                        .flex()
                        .flex_col()
                        .child(
                            ui::tile_box(44., 12., colors.accent, &colors).child(icon("pi", 22., colors.accent)),
                        )
                        .child(heading("Where does Pi run?", 30.).mt(px(20.)))
                        .child(ui::label("Step 1 of 2 · Connect a computer", &colors).mt(px(12.)))
                        .child(
                            ui::hint(
                                "Scan the code printed by Pi on your computer. It securely authorizes this phone through SSH—no address or key copying.",
                                &colors,
                            )
                            .mt(px(10.))
                            .text_size(px(15.))
                            .line_height(relative(1.5)),
                        )
                        .child({
                            let button = ui::button(
                                "scan-computer",
                                Button::Primary,
                                Some("scan"),
                                if self.connecting && self.pairing_status.is_some() {
                                    "Pairing…"
                                } else {
                                    "Scan computer QR"
                                },
                                false,
                                &colors,
                            )
                            .mt(px(20.))
                            .w_full();
                            if self.connecting {
                                ui::disabled(button, &colors)
                            } else {
                                button.on_click(cx.listener(|this, _, window, cx| {
                                    this.scan_computer(window, cx)
                                }))
                            }
                        })
                        .children(self.pairing_status.clone().map(|status| {
                            div()
                                .mt(px(10.))
                                .px(px(12.))
                                .py(px(10.))
                                .rounded(px(12.))
                                .bg(colors.tint(colors.accent))
                                .text_center()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors.accent)
                                .child(status)
                        }))
                        .child(
                            div()
                                .mt(px(24.))
                                .flex()
                                .items_center()
                                .gap(px(12.))
                                .child(div().h(px(1.)).flex_1().bg(colors.line))
                                .child(ui::label("Or connect manually", &colors))
                                .child(div().h(px(1.)).flex_1().bg(colors.line)),
                        )
                        .child(steps),
                ),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(20.))
                    .pt(px(12.))
                    .pb(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(connect.w_full())
                    .child(
                        div()
                            .id("sample")
                            .py(px(6.))
                            .text_center()
                            .text_size(px(14.))
                            .text_color(colors.accent)
                            .child("Look around with sample sessions")
                            .on_click(cx.listener(|this, _, window, cx| this.open_sample(window, cx))),
                    ),
            )
    }
}

/// Where the key goes, with the file in mono, wrapping as one paragraph.
fn key_hint(colors: &Theme) -> StyledText {
    let (before, file, after) = (
        "You add it to ",
        "~/.ssh/authorized_keys",
        " on the computer. The private key never leaves this phone.",
    );
    let run = |len: usize, family: &'static str, color| TextRun {
        len,
        font: gpui::font(family),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    StyledText::new(format!("{before}{file}{after}")).with_runs(vec![
        run(before.len(), SANS, colors.muted),
        run(file.len(), MONO, colors.secondary),
        run(after.len(), SANS, colors.muted),
    ])
}

/// A numbered step; `done` shows a check instead of the number.
fn step(done: Option<bool>, number: u32, colors: &Theme) -> Div {
    let done = done.unwrap_or(false);
    let badge: AnyElement = if done {
        icon("check", 14., colors.green).into_any_element()
    } else {
        div().child(number.to_string()).into_any_element()
    };
    div()
        .flex()
        .gap(px(14.))
        .p(px(16.))
        .border_color(colors.line)
        .child(
            div()
                .size(px(28.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(if done {
                    colors.green.opacity(0.16)
                } else {
                    colors.selected
                })
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors.secondary)
                .child(badge),
        )
}
