//! Shared card chrome; hosts supply a palette and standard copy/refresh icons.
use crate::theme::Theme;
use gpui::{Div, ElementId, Hsla, Role, Stateful, div, prelude::*, px};
pub fn label(text: impl Into<gpui::SharedString>, c: &Theme) -> Div {
    div()
        .text_color(c.text)
        .text_size(px(14.))
        .child(text.into())
}
pub fn hint(text: impl Into<gpui::SharedString>, c: &Theme) -> Div {
    div()
        .text_color(c.muted)
        .text_size(px(12.))
        .child(text.into())
}
pub fn icon(_: &str, size: f32, color: Hsla) -> Div {
    div().text_size(px(size)).text_color(color).child("✦")
}
pub fn tap(id: impl Into<ElementId>, kind: &str, c: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .size(px(48.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .active(|style| style.bg(c.selected))
        .cursor_pointer()
        .child(
            gpui::svg()
                .path(format!("icons/{kind}.svg"))
                .size(px(20.))
                .text_color(c.secondary),
        )
}

pub enum Button {
    Quiet,
}
pub fn button(
    id: impl Into<ElementId>,
    _: Button,
    _: Option<&str>,
    label: &str,
    _: bool,
    c: &Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.to_owned())
        .min_h(px(48.))
        .px(px(12.))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.))
        .text_color(c.muted)
        .cursor_pointer()
        .child(label.to_owned())
}
