//! The phone's building blocks, after design/android/screens/phone.css: 48 dp
//! touch targets, 56 dp app bars, pill buttons, chips, cards and stage tiles.

use crate::{
    assets::icon_path,
    model::{DiffLine, LineKind, StageKind, StageStatus},
    theme::{MONO, SERIF, Theme},
};
use gpui::{
    AnyElement, Div, ElementId, FontWeight, Hsla, IntoElement, ParentElement, SharedString,
    Stateful, Styled, Svg, div, prelude::*, px, relative, svg,
};

pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(icon_path(name))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// A 48 dp round touch target around a 20 dp icon.
pub fn tap(id: impl Into<ElementId>, glyph: &str, colors: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(48.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .active(|style| style.bg(colors.selected))
        .child(icon(glyph, 20., colors.secondary))
}

/// The top app bar: 56 dp, with a title and an optional subtitle.
pub fn appbar(
    leading: impl IntoElement,
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
    colors: &Theme,
) -> Div {
    div()
        .h(px(56.))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(4.))
        .px(px(4.))
        .child(leading)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .px(px(4.))
                .child(
                    div()
                        .text_size(px(17.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate()
                        .child(title.into()),
                )
                .children(subtitle.map(|subtitle| {
                    div()
                        .mt(px(-2.))
                        .text_size(px(12.5))
                        .text_color(colors.muted)
                        .truncate()
                        .child(subtitle)
                })),
        )
}

pub fn label(text: impl Into<SharedString>, colors: &Theme) -> Div {
    div()
        .text_size(px(12.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors.secondary)
        .child(text.into())
}

pub fn hint(text: impl Into<SharedString>, colors: &Theme) -> Div {
    div()
        .text_size(px(13.))
        .line_height(relative(1.4))
        .text_color(colors.muted)
        .child(text.into())
}

/// The serif italic of the desktop's landing and hand-offs.
pub fn serif(text: impl Into<SharedString>, size: f32) -> Div {
    div()
        .font_family(SERIF)
        .italic()
        .font_weight(FontWeight::NORMAL)
        .text_size(px(size))
        .line_height(relative(1.2))
        .child(text.into())
}

pub fn mono(text: impl Into<SharedString>, size: f32) -> Div {
    div()
        .font_family(MONO)
        .text_size(px(size))
        .child(text.into())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Primary,
    Plain,
    Quiet,
}

/// A pill button, 48 dp tall (40 dp when small).
pub fn button(
    id: impl Into<ElementId>,
    kind: Button,
    glyph: Option<&str>,
    text: impl Into<SharedString>,
    small: bool,
    colors: &Theme,
) -> Stateful<Div> {
    let (background, foreground, border) = match kind {
        Button::Primary => (colors.accent, colors.on_accent, colors.accent),
        Button::Plain => (colors.chip, colors.text, colors.line),
        Button::Quiet => (
            gpui::transparent_black(),
            colors.accent,
            gpui::transparent_black(),
        ),
    };
    let pressed = match kind {
        Button::Primary => colors.accent.opacity(0.85),
        _ => colors.selected,
    };
    div()
        .id(id)
        .h(px(if small { 40. } else { 48. }))
        .px(px(if small { 16. } else { 20. }))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .rounded_full()
        .bg(background)
        .border_1()
        .border_color(border)
        .text_color(foreground)
        .text_size(px(if small { 14. } else { 15. }))
        .font_weight(FontWeight::SEMIBOLD)
        .whitespace_nowrap()
        .active(move |style| style.bg(pressed))
        .children(glyph.map(|glyph| icon(glyph, 16., foreground)))
        .child(text.into())
}

/// A disabled look for a button that cannot act yet.
pub fn disabled(button: Stateful<Div>, colors: &Theme) -> Stateful<Div> {
    button
        .bg(colors.raised)
        .border_color(colors.raised)
        .text_color(colors.muted)
}

/// A 32 dp chip, for files, searches and choices.
pub fn chip(
    id: impl Into<ElementId>,
    glyph: Option<&str>,
    text: impl Into<SharedString>,
    colors: &Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(32.))
        .px(px(12.))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .rounded_full()
        .bg(colors.chip)
        .border_1()
        .border_color(colors.line)
        .text_size(px(13.))
        .text_color(colors.secondary)
        .whitespace_nowrap()
        .active(|style| style.bg(colors.selected))
        .children(glyph.map(|glyph| icon(glyph, 14., colors.muted)))
        .child(text.into())
}

pub fn card(colors: &Theme) -> Div {
    div()
        .bg(colors.composer)
        .border_1()
        .border_color(colors.line)
        .rounded(px(16.))
        .overflow_hidden()
}

/// A row in a card: at least 56 dp, separated from the one above.
pub fn row(id: impl Into<ElementId>, first: bool, colors: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .min_h(px(56.))
        .px(px(16.))
        .py(px(8.))
        .flex()
        .items_center()
        .gap(px(14.))
        .when(!first, |row| row.border_t_1().border_color(colors.line))
        .active(|style| style.bg(colors.selected))
}

/// A row's title and optional second line.
pub fn row_text(
    title: impl Into<SharedString>,
    detail: Option<SharedString>,
    colors: &Theme,
) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .truncate()
                .child(title.into()),
        )
        .children(detail.map(|detail| {
            div()
                .text_size(px(13.))
                .text_color(colors.muted)
                .truncate()
                .child(detail)
        }))
}

/// Status dots: live and waiting ones carry a soft ring.
pub fn dot(color: Hsla, ring: bool, colors: &Theme) -> Div {
    div()
        .size(px(16.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .when(ring, |dot| {
            dot.bg(colors
                .tint(color)
                .opacity(if colors.dark { 0.3 } else { 0.25 }))
        })
        .child(div().size(px(8.)).rounded_full().bg(color))
}

pub fn badge(text: impl Into<SharedString>, hue: Hsla, foreground: Hsla, colors: &Theme) -> Div {
    div()
        .h(px(24.))
        .px(px(10.))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .rounded_full()
        .bg(colors.tint(hue))
        .text_color(foreground)
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(div().size(px(6.)).rounded_full().bg(hue))
        .child(text.into())
}

pub fn stage_hue(kind: StageKind, colors: &Theme) -> Hsla {
    match kind {
        StageKind::Understand => colors.read,
        StageKind::Change => colors.edit,
        StageKind::Verify => colors.check,
        StageKind::HandOff => colors.accent,
    }
}

/// A square tinted with a hue, for an icon: the run rail's tiles, the app mark.
pub fn tile_box(size: f32, radius: f32, hue: Hsla, colors: &Theme) -> Div {
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(radius))
        .bg(colors.tint(hue))
}

/// A run rail tile: tinted when done, outlined while live, dashed ahead.
pub fn tile(kind: StageKind, status: StageStatus, colors: &Theme) -> Div {
    let hue = stage_hue(kind, colors);
    let base = div()
        .size(px(32.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(9.));
    match status {
        StageStatus::Done => base
            .bg(colors.tint(hue))
            .child(icon(kind.glyph(), 16., hue)),
        StageStatus::Live => base
            .bg(colors.composer)
            .border(px(1.5))
            .border_color(hue.opacity(0.6))
            .shadow(vec![gpui::BoxShadow {
                color: colors.tint(hue),
                offset: gpui::point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(4.),
                inset: false,
            }])
            .child(icon(kind.glyph(), 16., hue)),
        StageStatus::Planned | StageStatus::Skipped => base
            .border(px(1.5))
            .border_dashed()
            .border_color(colors.line_strong)
            .child(icon(kind.glyph(), 16., colors.muted)),
    }
}

/// "+3 −1" in mono, green and coral.
pub fn counts(added: u32, removed: u32, colors: &Theme) -> Div {
    div()
        .flex()
        .flex_none()
        .gap(px(6.))
        .font_family(MONO)
        .text_size(px(12.))
        .when(added > 0, |counts| {
            counts.child(div().text_color(colors.green).child(format!("+{added}")))
        })
        .when(removed > 0, |counts| {
            counts.child(div().text_color(colors.coral).child(format!("−{removed}")))
        })
}

const KEYWORDS: &[&str] = &[
    "const", "let", "if", "for", "of", "return", "function", "export", "throw", "new", "continue",
    "import", "from", "await", "async", "fn", "pub", "use", "impl", "else",
];

/// A line of code colored like the desktop: keywords and strings.
pub fn code_text(text: &str, colors: &Theme) -> AnyElement {
    let mut spans: Vec<(String, Hsla)> = Vec::new();
    let mut push = |text: &str, color: Hsla| match spans.last_mut() {
        Some((last, last_color)) if *last_color == color => last.push_str(text),
        _ => spans.push((text.to_owned(), color)),
    };
    let mut rest = text;
    while !rest.is_empty() {
        let quote = rest
            .chars()
            .next()
            .filter(|c| matches!(c, '"' | '\'' | '`'));
        if let Some(quote) = quote {
            let end = rest[1..].find(quote).map_or(rest.len(), |index| index + 2);
            push(&rest[..end], colors.string);
            rest = &rest[end..];
            continue;
        }
        let word_end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if word_end > 0 {
            let word = &rest[..word_end];
            push(
                word,
                if KEYWORDS.contains(&word) {
                    colors.keyword
                } else {
                    colors.plain
                },
            );
            rest = &rest[word_end..];
        } else {
            let length = rest.chars().next().map_or(1, char::len_utf8);
            push(&rest[..length], colors.plain);
            rest = &rest[length..];
        }
    }
    let text: SharedString = spans
        .iter()
        .map(|(text, _)| text.as_str())
        .collect::<String>()
        .into();
    let mut offset = 0;
    let highlights: Vec<_> = spans
        .iter()
        .map(|(span, color)| {
            let range = offset..offset + span.len();
            offset += span.len();
            (range, gpui::HighlightStyle::color(*color))
        })
        .collect();
    gpui::StyledText::new(text)
        .with_highlights(highlights)
        .into_any_element()
}

/// A diff line: number, then code; added and removed lines are tinted.
pub fn diff_line(line: &DiffLine, selected: bool, colors: &Theme) -> Div {
    let background = match line.kind {
        LineKind::Added => Some(colors.added),
        LineKind::Removed => Some(colors.removed),
        LineKind::Context => None,
    };
    div()
        .relative()
        .flex()
        .w_full()
        .min_h(px(20.))
        .font_family(MONO)
        .text_size(px(12.))
        .line_height(px(20.))
        .when_some(background, |line, background| line.bg(background))
        .when(selected, |line| {
            line.child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(px(3.))
                    .bg(colors.accent),
            )
        })
        .child(
            div()
                .w(px(38.))
                .flex_none()
                .pr(px(10.))
                .whitespace_nowrap()
                .text_right()
                .text_color(colors.muted)
                .child(line.number.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .pr(px(8.))
                .child(code_text(&line.text, colors)),
        )
}
