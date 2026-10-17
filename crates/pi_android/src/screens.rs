//! The screens of design/android, as `PhoneApp` methods: one file each, and
//! the bottom sheets together.

pub(crate) mod command;
mod connect;
mod diagnostics;
mod files;
pub(crate) use files::language;
mod history;
mod pages;
pub(crate) use pages::ShownImage;
pub(crate) use thread::is_yes_no;
mod projects;
mod review;
mod schedules;
mod sessions;
mod settings;
pub(crate) mod sheets;
mod start;
mod stream;
mod subagents;
pub(crate) mod swarm;
mod swarm_map;
mod thread;

use crate::{app::PhoneApp, theme::Theme, ui};
use gpui::{
    AnyElement, Context, Div, ElementId, Pixels, ScrollHandle, Stateful, div, point, prelude::*, px,
};

/// A screen's scrolling middle.
fn scroll_area(id: impl Into<ElementId>, handle: &ScrollHandle) -> crate::scroll::ScrollArea {
    crate::scroll::vertical(id, handle).flex_1()
}

/// A clipped horizontal viewport whose offset can be shared by many virtual rows.
/// Each mounted row gets its own measurement handle so one wheel event is not
/// applied once per row to the shared handle.
fn shared_horizontal_pan(
    id: impl Into<ElementId>,
    shared: ScrollHandle,
    content_width: Pixels,
    child: AnyElement,
    cx: &mut Context<PhoneApp>,
) -> Stateful<Div> {
    let measurement = ScrollHandle::new();
    let offset = shared.offset().x;
    let clamp_measurement = measurement.clone();
    let clamp_shared = shared.clone();
    div()
        .on_children_prepainted(move |_, window, _| {
            let max = (content_width - clamp_measurement.bounds().size.width).max(px(0.));
            let current = clamp_shared.offset().x;
            let clamped = current.clamp(-max, px(0.));
            if clamped != current {
                clamp_shared.set_offset(point(clamped, px(0.)));
                window.refresh();
            }
        })
        .id(id)
        .overflow_hidden()
        .track_scroll(&measurement)
        .on_scroll_wheel(cx.listener({
            let measurement = measurement.clone();
            move |_, event: &gpui::ScrollWheelEvent, _, cx| {
                let delta = event.delta.pixel_delta(px(20.));
                if delta.x.abs() <= delta.y.abs() {
                    return;
                }
                let max = (content_width - measurement.bounds().size.width).max(px(0.));
                let x = (shared.offset().x + delta.x).clamp(-max, px(0.));
                shared.set_offset(point(x, px(0.)));
                crate::scroll::note_input(event.touch_phase);
                cx.stop_propagation();
                cx.notify();
            }
        }))
        .child(div().relative().left(offset).child(child))
}

/// A section's heading above a card.
fn section(text: &'static str, colors: &Theme) -> Div {
    ui::label(text, colors).mt(px(20.)).mb(px(8.)).mx(px(4.))
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
