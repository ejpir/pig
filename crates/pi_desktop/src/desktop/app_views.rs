//! App views (design study 02): All Sessions and Settings take the middle column and
//! the inspector instead of a session. The sidebar's app rows open them; selecting a
//! session leaves them. They read pi's session list and settings files, and the
//! header's search field filters them.
mod catalog;
mod sessions;
mod settings;
pub use catalog::CatalogScreen;

use super::*;
use gpui::{Div, ElementId, Stateful};
pub use sessions::SessionsView;
pub use settings::SettingsView;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppView {
    Sessions,
    Settings,
    Models,
    Resources,
}

impl AppView {
    pub fn title(self) -> &'static str {
        match self {
            Self::Sessions => "All Sessions",
            Self::Settings => "Settings",
            Self::Models => "Models",
            Self::Resources => "Resources",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Sessions => "list_tree",
            Self::Settings => "settings",
            Self::Models => "sparkle",
            Self::Resources => "box",
        }
    }
}

/// A row of segments; each one gets its click handler from the caller.
pub(super) fn segments(children: impl IntoIterator<Item = Stateful<Div>>, theme: Theme) -> Div {
    h_flex()
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(6.))
        .border_1()
        .border_color(theme.chip_line)
        .bg(theme.hover)
        .children(children)
}

pub(super) fn segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    theme: Theme,
) -> Stateful<Div> {
    let label = label.into();
    div()
        .id(id)
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .aria_selected(selected)
        .h(px(22.))
        .px(px(10.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .text_size(px(11.))
        .cursor_pointer()
        .when(selected, |segment| {
            segment
                .bg(theme.canvas)
                .border_1()
                .border_color(theme.chip_line)
                .text_color(theme.text)
        })
        .when(!selected, |segment| {
            segment
                .text_color(theme.secondary)
                .hover(move |style| style.text_color(theme.text))
        })
        .child(label)
}

/// An on/off switch.
fn toggle(id: impl Into<ElementId>, on: bool, theme: Theme) -> Stateful<Div> {
    div()
        .id(id)
        .role(gpui::Role::Switch)
        .aria_toggled(if on {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .w(px(30.))
        .h(px(18.))
        .flex_shrink_0()
        .rounded_full()
        .p(px(2.))
        .cursor_pointer()
        .bg(if on { theme.accent } else { theme.chip_line })
        .child(
            div()
                .size(px(14.))
                .rounded_full()
                .bg(theme.canvas)
                .when(on, |knob| knob.ml(px(12.))),
        )
}

/// An element id from a name and a key, such as a setting's.
fn keyed(name: &str, key: &str) -> ElementId {
    ElementId::Name(format!("{name}-{key}").into())
}

/// A value shown as a small button that opens its editor.
fn value_chip(id: ElementId, theme: Theme) -> Stateful<Div> {
    h_flex()
        .id(id)
        .h(px(26.))
        .px(px(10.))
        .gap(px(6.))
        .rounded(px(5.))
        .border_1()
        .border_color(theme.chip_line)
        .bg(theme.chip)
        .font_family(MONO)
        .text_size(px(11.))
        .text_color(theme.secondary)
        .cursor_pointer()
        .hover(move |style| style.bg(theme.hover))
}

/// A heading and value on one inspector line.
fn detail(name: &str, value: impl Into<SharedString>, mono: bool, theme: Theme) -> Div {
    h_flex()
        .h(px(24.))
        .gap(px(12.))
        .text_size(px(12.))
        .child(
            div()
                .flex_shrink_0()
                .text_color(theme.secondary)
                .child(name.to_owned()),
        )
        .child(div().flex_1())
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_color(theme.text)
                .when(mono, |value| value.font_family(MONO).text_size(px(11.5)))
                .child(value.into()),
        )
}

/// The inspector's big title.
fn inspector_title(text: impl Into<SharedString>, serif: bool) -> Div {
    div()
        .when(serif, |title| {
            title.font_family(SERIF).italic().text_size(px(22.))
        })
        .when(!serif, |title| title.font_family(MONO).text_size(px(18.)))
        .line_height(px(30.))
        .truncate()
        .child(text.into())
}
