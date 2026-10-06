//! 01 Connect: where Pi runs, in three steps.

use super::scroll_area;
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
    pub(crate) fn connect_screen(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
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
                    ui::tap("copy-key", "copy", &colors).on_click(
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
            .mt(px(20.))
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
        if self.manual_setup {
            return div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(ui::appbar(
                    ui::tap("manual-back", "back", &colors).on_click(cx.listener(
                        |this, _, window, cx| {
                            this.back(window, cx);
                        },
                    )),
                    "Set up with an SSH key",
                    None,
                    &colors,
                ))
                .child(
                    scroll_area("connect", &scroll).child(
                        div()
                            .px(px(20.))
                            .pt(px(8.))
                            .pb(px(12.))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .line_height(px(22.))
                                    .text_color(colors.secondary)
                                    .child("Enter the computer’s SSH address, then add this phone’s key there."),
                            )
                            .child(steps),
                    ),
                )
                .child(
                    div()
                        .flex_none()
                        .px(px(20.))
                        .pt(px(12.))
                        .pb(px(16.))
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(connect.w_full())
                        .child(
                            ui::button(
                                "sample",
                                Button::Quiet,
                                None,
                                "Look around with sample sessions",
                                false,
                                &colors,
                            )
                            .w_full()
                            .on_click(cx.listener(|this, _, window, cx| this.open_sample(window, cx))),
                        ),
                )
                .into_any_element();
        }
        let scan = {
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
            .w_full();
            if self.connecting {
                ui::disabled(button, &colors)
            } else {
                button.on_click(cx.listener(|this, _, window, cx| this.scan_computer(window, cx)))
            }
        };
        let note = match self.pairing_status.clone() {
            Some(status) => div()
                .mb(px(4.))
                .px(px(12.))
                .py(px(10.))
                .rounded(px(12.))
                .bg(colors.tint(colors.accent))
                .text_center()
                .text_size(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors.accent)
                .child(status),
            None => div()
                .mb(px(4.))
                .flex()
                .items_start()
                .gap(px(8.))
                .text_size(px(12.5))
                .line_height(px(16.))
                .text_color(colors.muted)
                .child(icon("shield", 16., colors.green))
                .child(
                    div().flex_1().min_w_0().child(
                        "Uses the computer’s own SSH. The code works once, for two minutes, and pins the computer’s key.",
                    ),
                ),
        };
        let pair = ui::card(&colors)
            .mt(px(8.))
            .child(
                pair_step(1, &colors).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .pt(px(2.))
                        .child("On the computer, run")
                        .child(
                            div()
                                .mt(px(8.))
                                .h(px(40.))
                                .pl(px(12.))
                                .flex()
                                .items_center()
                                .rounded(px(12.))
                                .bg(colors.panel)
                                .child(
                                    ui::mono(PAIR_COMMAND, 12.5)
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(colors.text),
                                )
                                .child(
                                    div()
                                        .id("copy-pair-command")
                                        .relative()
                                        .child(crate::testing::probe("copy-pair-command"))
                                        .size(px(40.))
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(12.))
                                        .active(|style| style.bg(colors.selected))
                                        .child(icon("copy", 16., colors.muted))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.copy(PAIR_COMMAND_FULL.into(), "the command", cx)
                                        })),
                                ),
                        ),
                ),
            )
            .child(
                pair_step(2, &colors).border_t_1().child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .pt(px(2.))
                        .child("Scan the code it prints"),
                ),
            )
            .child(
                pair_step(3, &colors).border_t_1().child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .pt(px(2.))
                        .child("Check both screens show the same six digits"),
                ),
            );
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                scroll_area("connect", &scroll).child(
                    div()
                        .px(px(20.))
                        .pt(px(64.))
                        .pb(px(12.))
                        .flex()
                        .flex_col()
                        .child(
                            ui::tile_box(48., 14., colors.accent, &colors)
                                .child(icon("pi", 24., colors.accent)),
                        )
                        .child(
                            ui::serif("Where does Pi run?", 31.)
                                .mt(px(24.))
                                .line_height(px(36.)),
                        )
                        .child(
                            div()
                                .mt(px(12.))
                                .text_size(px(15.))
                                .line_height(px(22.))
                                .text_color(colors.secondary)
                                .child("On your computer. This phone follows its sessions, answers its questions and reviews what it changed."),
                        )
                        .child(
                            div()
                                .mt(px(32.))
                                .text_size(px(12.5))
                                .line_height(px(16.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors.muted)
                                .child("Pair in under a minute"),
                        )
                        .child(pair),
                ),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(20.))
                    .pt(px(12.))
                    .pb(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(note)
                    .child(scan)
                    .child(
                        ui::button(
                            "manual-setup",
                            Button::Quiet,
                            None,
                            "Set up with an SSH key instead",
                            false,
                            &colors,
                        )
                        .w_full()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.manual_setup = true;
                            cx.notify();
                        })),
                    ),
            )
            .into_any_element()
    }
}

/// What the computer runs to show its pairing code.
const PAIR_COMMAND: &str = "pi-desktop-remote pair";
/// The same command where the helper is installed, which works without a PATH entry.
const PAIR_COMMAND_FULL: &str = "~/.pi/desktop/bin/pi-desktop-remote pair";

/// A pairing step: a 24 dp number, then its text.
fn pair_step(number: u32, colors: &Theme) -> Div {
    div()
        .flex()
        .gap(px(12.))
        .p(px(16.))
        .border_color(colors.line)
        .text_size(px(15.))
        .line_height(px(20.))
        .child(
            div()
                .size(px(24.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(colors.selected)
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors.secondary)
                .child(number.to_string()),
        )
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
