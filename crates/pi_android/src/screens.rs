//! The screens of design/android, as `PhoneApp` methods: one file each, and
//! the bottom sheets together.

mod connect;
mod drawer;
mod projects;
mod review;
mod sessions;
mod settings;
mod sheets;
mod start;
mod thread;

use crate::{theme::Theme, ui};
use gpui::{Div, ElementId, ScrollHandle, div, prelude::*, px};

/// A screen's scrolling middle.
fn scroll_area(id: impl Into<ElementId>, handle: &ScrollHandle) -> crate::scroll::ScrollArea {
    crate::scroll::vertical(id, handle).flex_1()
}

/// A section's heading above a card.
fn section(text: &'static str, colors: &Theme) -> Div {
    ui::label(text, colors).mt(px(20.)).mb(px(8.)).mx(px(4.))
}

/// The serif italic headline of the connect and start screens.
fn heading(text: &'static str, size: f32) -> Div {
    ui::serif(text, size).line_height(gpui::relative(1.15))
}

/// An on/off switch, 44 by 26.
fn switch(on: bool, colors: &Theme) -> Div {
    div()
        .w(px(44.))
        .h(px(26.))
        .flex_none()
        .p(px(3.))
        .flex()
        .when(on, |switch| switch.justify_end())
        .rounded_full()
        .bg(if on { colors.accent } else { colors.raised })
        .child(
            div()
                .size(px(20.))
                .rounded_full()
                .bg(colors.composer)
                .shadow(vec![gpui::BoxShadow {
                    color: colors.shadow,
                    offset: gpui::point(px(0.), px(1.)),
                    blur_radius: px(2.),
                    spread_radius: px(0.),
                    inset: false,
                }]),
        )
}
