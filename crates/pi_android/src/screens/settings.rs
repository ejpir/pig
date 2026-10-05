//! 12 Settings and tools: computers, appearance, notifications, typing, and
//! what Pi has on the computer.

use super::{scroll_area, section, switch};
use crate::{
    app::{PhoneApp, Route, Sheet},
    prefs::Prefs,
    theme::{Appearance, Theme, theme},
    ui::{self, Button, icon},
};
use gpui::{
    Context, Div, ElementId, FontWeight, Hsla, SharedString, Stateful, Window, div, prelude::*, px,
    relative, rgb,
};
use gpui_android::activity;

impl PhoneApp {
    pub(crate) fn settings_screen(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = theme(cx);
        let scroll = self.scroll(Route::Settings);
        let prefs = self.prefs(cx).clone();
        let computer = self.store.as_ref().map_or_else(
            || "the computer".to_owned(),
            |store| store.computer.name.clone(),
        );
        let live = self.store.as_ref().is_some_and(|store| !store.is_sample());
        let fingerprint = self
            .store
            .as_ref()
            .and_then(|store| prefs.host_keys.get(&store.computer.address))
            .cloned();
        let computers = self
            .store
            .iter()
            .flat_map(|store| store.computers.iter())
            .enumerate()
            .map(|(index, computer)| {
                let detail = match &computer.pi_version {
                    Some(version) if computer.connected => {
                        format!("{} · {version}", computer.address)
                    }
                    None if computer.connected && live => {
                        format!("{} · connected", computer.address)
                    }
                    None if live && index == 0 => format!("{} · reconnecting…", computer.address),
                    _ => format!("{} · not connected", computer.address),
                };
                let address = computer.address.clone();
                let connected = computer.connected;
                ui::row(("computer", index), index == 0, &colors)
                    .child(icon(
                        if index == 0 { "computer" } else { "server" },
                        20.,
                        colors.muted,
                    ))
                    .child(ui::row_text(
                        computer.name.clone(),
                        Some(detail.into()),
                        &colors,
                    ))
                    .child(if connected {
                        div()
                            .size(px(8.))
                            .rounded_full()
                            .bg(colors.green)
                            .into_any_element()
                    } else {
                        icon("chev_r", 16., colors.muted).into_any_element()
                    })
                    .when(!connected && !live, |row| {
                        row.on_click(cx.listener(move |this, _, window, cx| {
                            this.address
                                .update(cx, |field, cx| field.set_text(address.clone(), cx));
                            this.push(Route::Connect, window, cx);
                        }))
                    })
            });
        let add = ui::row("add-computer", false, &colors)
            .text_color(colors.accent)
            .child(icon("plus", 20., colors.accent))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Add a computer"),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.address.update(cx, |address, cx| {
                    address.take(cx);
                });
                this.push(Route::Connect, window, cx);
            }));
        let themes = [
            (Appearance::System, "System"),
            (Appearance::Evening, "Evening"),
            (Appearance::Moonstone, "Moonstone"),
        ]
        .map(|(appearance, name)| {
            let on = prefs.appearance == appearance;
            div()
                .id(name)
                .flex_1()
                .p(px(if on { 7. } else { 8. }))
                .rounded(px(14.))
                .border(px(if on { 2. } else { 1. }))
                .border_color(if on { colors.accent } else { colors.line })
                .bg(colors.composer)
                .child(theme_preview(appearance))
                .child(
                    div()
                        .mt(px(6.))
                        .text_center()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(name),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.update_prefs(cx, |prefs| prefs.appearance = appearance);
                    this.apply_theme(window, cx);
                }))
        });
        let system_off = cfg!(target_os = "android")
            && (prefs.notify_questions || prefs.notify_finished || prefs.notify_working)
            && !activity::notifications_enabled();
        let notifications = ui::card(&colors)
            .when(system_off, |card| {
                card.child(
                    div()
                        .px(px(16.))
                        .py(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .bg(colors.tint(colors.wait))
                        .child(icon("bell", 16., colors.amber))
                        .child(ui::hint("Android has notifications off for Pi.", &colors).flex_1())
                        .child(
                            ui::button("allow", Button::Quiet, None, "Allow", true, &colors)
                                .h(px(32.))
                                .px(px(4.))
                                .on_click(|_, _, _| activity::request_notification_permission()),
                        ),
                )
            })
            .child(toggle_row(
                "notify-questions",
                !system_off,
                "When Pi needs you",
                Some("Questions and permissions, with an answer button"),
                prefs.notify_questions,
                &colors,
                cx,
                |prefs| &mut prefs.notify_questions,
                ask_to_notify,
            ))
            .child(toggle_row(
                "notify-finished",
                false,
                "When a session finishes",
                None,
                prefs.notify_finished,
                &colors,
                cx,
                |prefs| &mut prefs.notify_finished,
                ask_to_notify,
            ))
            .child(toggle_row(
                "notify-working",
                false,
                "While sessions work",
                Some("A quiet ongoing notification"),
                prefs.notify_working,
                &colors,
                cx,
                |prefs| &mut prefs.notify_working,
                ask_to_notify,
            ));
        let typing = ui::card(&colors).child(toggle_row(
            "return-sends",
            true,
            "Return sends",
            Some(if prefs.return_sends {
                "Return or the send button sends. Shift+Return adds a line."
            } else {
                "Off: return adds a line, send with the button"
            }),
            prefs.return_sends,
            &colors,
            cx,
            |prefs| &mut prefs.return_sends,
            |this, _, cx| this.apply_return_sends(cx),
        ));
        let tools = ui::card(&colors)
            .child(
                ui::row("models", true, &colors)
                    .child(icon("spark", 20., colors.muted))
                    .child(div().flex_1().child("Models"))
                    .child(icon("chev_r", 16., colors.muted))
                    .on_click(cx.listener(|this, _, _, cx| this.open_sheet(Sheet::Models, cx))),
            )
            .child(
                ui::row("resources", false, &colors)
                    .child(icon("layers", 20., colors.muted))
                    .child(div().flex_1().child("Resources"))
                    .child(icon("chev_r", 16., colors.muted))
                    .on_click(cx.listener(|this, _, _, cx| this.open_sheet(Sheet::Resources, cx))),
            );
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(ui::appbar(
                ui::tap("back", "back", &colors).on_click(cx.listener(|this, _, window, cx| {
                    this.back(window, cx);
                })),
                "Settings and tools",
                None,
                &colors,
            ))
            .child(
                scroll_area("settings", &scroll).child(
                    div()
                        .px(px(16.))
                        .pb(px(24.))
                        .child(section("Computers", &colors).mt(px(4.)))
                        .child(
                            ui::card(&colors)
                                .children(computers)
                                .child(add)
                                .when(live, |card| {
                                    card.child(
                                        ui::row("forget", false, &colors)
                                            .text_color(colors.coral)
                                            .child(icon("x", 20., colors.coral))
                                            .child(div().text_size(px(15.)).child("Forget this computer"))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.forget_computer(window, cx)
                                            })),
                                    )
                                }),
                        )
                        .child(section("Appearance", &colors))
                        .child(div().flex().gap(px(10.)).children(themes))
                        .child(section("Notifications", &colors))
                        .child(notifications)
                        .child(section("Typing", &colors))
                        .child(typing)
                        .child(
                            ui::label(SharedString::from(format!("On {computer}")), &colors)
                                .mt(px(20.))
                                .mb(px(8.))
                                .mx(px(4.)),
                        )
                        .child(tools)
                        .child(
                            ui::hint(
                                match (&fingerprint, live) {
                                    (Some(fingerprint), true) => format!(
                                        "Connected over SSH with this phone's own key. {computer}'s host key is {fingerprint}; a different one is refused."
                                    ),
                                    _ => "These are sample sessions: nothing runs on a computer. Add one above to follow its sessions."
                                        .to_owned(),
                                },
                                &colors,
                            )
                            .mx(px(4.))
                            .mt(px(20.)),
                        ),
                ),
            )
    }
}

/// Once a notification setting is on, Android has to allow notifying too.
fn ask_to_notify(_: &mut PhoneApp, on: bool, _: &mut Context<PhoneApp>) {
    if on {
        activity::request_notification_permission();
    }
}

/// A setting with a switch; the whole row toggles it, then `changed` runs.
#[allow(clippy::too_many_arguments)]
fn toggle_row(
    id: impl Into<ElementId>,
    first: bool,
    title: &'static str,
    detail: Option<&'static str>,
    on: bool,
    colors: &Theme,
    cx: &Context<PhoneApp>,
    field: fn(&mut Prefs) -> &mut bool,
    changed: fn(&mut PhoneApp, bool, &mut Context<PhoneApp>),
) -> Stateful<Div> {
    ui::row(id, first, colors)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(div().text_size(px(15.)).child(title))
                .children(detail.map(|detail| {
                    div()
                        .text_size(px(13.))
                        .line_height(relative(1.35))
                        .text_color(colors.muted)
                        .child(detail)
                })),
        )
        .child(switch(on, colors))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.update_prefs(cx, |prefs| {
                let value = field(prefs);
                *value = !*value;
            });
            let on = *field(cx.global_mut::<Prefs>());
            changed(this, on, cx);
        }))
}

/// A small picture of a theme: its canvas and three lines of its colors.
fn theme_preview(appearance: Appearance) -> Div {
    let color = |hex: u32| -> Hsla { rgb(hex).into() };
    let (light, dark) = (color(0xfaf9f7), color(0x161d27));
    let bars = match appearance {
        Appearance::System => [0x4b607c, 0xcbc3bb, 0x8caecb],
        Appearance::Evening => [0x8caecb, 0x424954, 0x363d46],
        Appearance::Moonstone => [0x4b607c, 0xddd7d0, 0xcbc3bb],
    };
    let half = |background: Hsla, left: bool| {
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .w(relative(0.5))
            .map(|half| if left { half.left_0() } else { half.right_0() })
            .bg(background)
    };
    div()
        .relative()
        .h(px(58.))
        .p(px(8.))
        .flex()
        .flex_col()
        .gap(px(5.))
        .rounded(px(8.))
        .overflow_hidden()
        .map(|preview| match appearance {
            Appearance::System => preview.child(half(light, true)).child(half(dark, false)),
            Appearance::Evening => preview.bg(dark),
            Appearance::Moonstone => preview.bg(light).border_1().border_color(color(0xddd7d0)),
        })
        .children(bars.into_iter().zip([0.6, 0.8, 0.45]).map(|(bar, width)| {
            div()
                .h(px(6.))
                .w(relative(width))
                .rounded(px(3.))
                .bg(color(bar))
        }))
}
