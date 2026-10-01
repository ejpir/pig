use crate::theme::{MONO, Theme};
use gpui::{prelude::*, *};
use std::time::Duration;

// Zed's ui::h_flex/v_flex pattern, without pulling in the editor/workspace graph.
pub fn h_flex() -> Div {
    div().flex().flex_row().items_center()
}
pub fn v_flex() -> Div {
    div().flex().flex_col()
}

pub fn label(text: impl Into<SharedString>, theme: Theme) -> Div {
    // GPUI has no letter-spacing style. Individual mono glyphs preserve the
    // study's 1.2px tracking without adding copyable whitespace to the label.
    let text = text.into();
    h_flex()
        .gap(px(1.2))
        .font_family(MONO)
        .text_size(px(10.))
        .line_height(px(16.))
        .text_color(theme.faint)
        .flex_shrink_0()
        .children(text.chars().map(|ch| div().child(ch.to_string())))
}

pub fn icon(name: &'static str, color: Hsla) -> Svg {
    svg()
        .path(format!("icons/{name}.svg"))
        .size(px(15.))
        .flex_shrink_0()
        .text_color(color)
}

pub fn icon_button(
    id: impl Into<ElementId>,
    name: &'static str,
    description: &'static str,
    theme: Theme,
) -> Stateful<Div> {
    h_flex()
        .id(id)
        .role(Role::Button)
        .aria_label(description)
        .size(px(24.))
        .justify_center()
        .rounded(px(5.))
        .text_color(theme.muted)
        .cursor_pointer()
        .hover(move |style| style.bg(theme.hover))
        .child(icon(name, theme.muted))
}

pub fn chip(id: &'static str, description: &'static str, theme: Theme) -> Stateful<Div> {
    h_flex()
        .id(id)
        .role(Role::Button)
        .aria_label(description)
        .h(px(24.))
        .px(px(8.))
        .gap(px(6.))
        .rounded(px(5.))
        .bg(if theme.light { theme.hover } else { theme.chip })
        .text_size(px(11.))
        .font_family(MONO)
        .text_color(theme.secondary)
        .cursor_pointer()
        .hover(move |style| style.bg(theme.selected))
}

pub fn button(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    theme: Theme,
) -> Stateful<Div> {
    let text = text.into();
    h_flex()
        .id(id)
        .role(Role::Button)
        .aria_label(text.clone())
        .gap(px(6.))
        .h(px(24.))
        .px(px(9.))
        .rounded(px(5.))
        .border_1()
        .border_color(theme.chip_line)
        .bg(theme.chip)
        .text_color(theme.secondary)
        .text_size(px(11.))
        .line_height(px(18.))
        .cursor_pointer()
        .hover(move |style| style.bg(theme.hover))
        .child(text)
}

/// The accent-filled action (Send, Steer). Its hover lightens the accent, where
/// [`button`]'s neutral hover would hide the label.
pub fn primary_button(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    enabled: bool,
    theme: Theme,
) -> Stateful<Div> {
    let text = text.into();
    let (fill, label) = if enabled {
        (theme.accent, theme.on_accent)
    } else {
        (theme.raised, theme.faint)
    };
    h_flex()
        .id(id)
        .role(Role::Button)
        .aria_label(text.clone())
        .gap(px(6.))
        .h(px(24.))
        .px(px(12.))
        .rounded(px(5.))
        .border_1()
        .border_color(if enabled { theme.accent } else { theme.line })
        .bg(fill)
        .text_color(label)
        .text_size(px(11.))
        .line_height(px(18.))
        .font_weight(FontWeight::SEMIBOLD)
        .when(enabled, |button| {
            button.cursor_pointer().hover(move |style| {
                style
                    .bg(theme.accent_hover)
                    .border_color(theme.accent_hover)
            })
        })
        .child(text)
}

/// Shared session/catalog notice presentation. Dismissal stays with the owner.
pub fn notification_card(
    id: &'static str,
    message: &str,
    error: bool,
    close: AnyElement,
    theme: Theme,
) -> Stateful<Div> {
    h_flex()
        .id(id)
        .debug_selector(move || id.into())
        .w(px(440.))
        .max_w_full()
        .items_start()
        .gap(px(10.))
        .p(px(14.))
        .rounded(px(9.))
        .border_1()
        .border_color(if error { theme.coral } else { theme.amber })
        .bg(theme.canvas)
        .text_color(theme.text)
        .shadow_lg()
        .occlude()
        .tooltip(ui::Tooltip::text(message.to_owned()))
        .child(icon(
            if error { "warning" } else { "info" },
            if error { theme.coral } else { theme.amber },
        ))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(4.))
                .child(
                    div()
                        .id("notification-message")
                        .max_h(px(190.))
                        .overflow_y_scroll()
                        .text_size(px(13.))
                        .line_height(px(19.))
                        .child(message.chars().take(600).collect::<String>()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .line_height(px(17.))
                        .text_color(theme.faint)
                        .child(if error {
                            "Pi · error · Ctrl+Shift+D for diagnostics"
                        } else {
                            "Pi · notification"
                        }),
                ),
        )
        .child(close)
}
pub fn notification_overlay(card: AnyElement, top: Pixels) -> AnyElement {
    gpui::deferred(
        h_flex()
            .absolute()
            .top(top)
            .left(px(16.))
            .right(px(16.))
            .justify_end()
            .child(card),
    )
    .with_priority(2)
    .into_any_element()
}

pub fn divider(theme: Theme) -> Div {
    div()
        .h(px(1.))
        .w_full()
        .bg(theme.line)
        .mt(px(12.))
        .flex_shrink_0()
}

pub fn section(name: &str, hint: &str, theme: Theme) -> Div {
    h_flex()
        .h(px(20.))
        .justify_between()
        .child(label(name.to_owned(), theme))
        .child(
            div()
                .text_size(px(10.5))
                .text_color(theme.faint)
                .child(hint.to_owned()),
        )
}

pub fn pair(name: &str, value: String, theme: Theme) -> Div {
    h_flex()
        .justify_between()
        .gap(px(8.))
        .h(px(24.))
        .text_size(px(12.))
        .child(div().text_color(theme.muted).child(name.to_owned()))
        .child(
            div()
                .font_family(MONO)
                .text_color(theme.secondary)
                .child(value),
        )
}

pub fn count(value: u64) -> String {
    if value >= 1000 {
        format!(
            "{}k",
            format!("{:.1}", value as f64 / 1000.).trim_end_matches(".0")
        )
    } else {
        value.to_string()
    }
}

pub fn short_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let tail = normalized
        .rsplit('/')
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("/");
    format!("…/{tail}")
}

/// The three-colour pi mark from the supplied study, not a substitute π glyph.
pub fn brand_mark_sized(size: f32) -> Div {
    let scale = size / 18.;
    let block = |x, y, w, h, color| {
        div()
            .absolute()
            .left(px(x * scale))
            .top(px(y * scale))
            .w(px(w * scale))
            .h(px(h * scale))
            .bg(rgb(color))
    };
    div()
        .relative()
        .size(px(size))
        .flex_shrink_0()
        .child(block(0., 0., 13.5, 4.5, 0xf09082))
        .child(block(9., 4.5, 4.5, 4.5, 0xf09082))
        .child(block(0., 4.5, 4.5, 13.5, 0x4d9abf))
        .child(block(4.5, 9., 4.5, 4.5, 0x4d9abf))
        .child(block(13.5, 9., 4.5, 9., 0xf1be58))
}

/// The study's running mark: a faint track with a turning accent arc.
pub fn spinner(id: impl Into<ElementId>, theme: Theme) -> Div {
    div()
        .relative()
        .flex_shrink_0()
        .size(px(12.))
        .child(
            svg()
                .path("icons/spinner_track.svg")
                .absolute()
                .size(px(12.))
                .text_color(theme.line_strong),
        )
        .child(
            svg()
                .path("icons/spinner_arc.svg")
                .absolute()
                .size(px(12.))
                .text_color(theme.accent)
                .with_animation(
                    id,
                    Animation::new(Duration::from_millis(900)).repeat(),
                    |arc, delta| arc.with_transformation(Transformation::rotate(percentage(delta))),
                ),
        )
}
