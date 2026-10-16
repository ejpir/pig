//! The phone's building blocks, after design/android/screens/phone.css: 48 dp
//! touch targets, 56 dp app bars, pill buttons, chips, cards and stage tiles.

use crate::{
    assets::icon_path,
    model::{DiffLine, LineKind, Mark, StageKind, StageStatus, Stretch, StretchKind},
    theme::{MONO, SERIF, Theme},
};
use gpui::{
    Animation, AnimationExt, AnyElement, Div, ElementId, FontWeight, Hsla, IntoElement,
    ParentElement, SharedString, Stateful, Styled, Svg, div, prelude::*, px, relative, svg,
};

pub fn working_indicator(colors: &Theme) -> impl IntoElement {
    div()
        .size(px(10.))
        .flex_none()
        .rounded_full()
        .bg(colors.read)
        .with_animation(
            "working-pulse",
            Animation::new(std::time::Duration::from_millis(1400)).repeat(),
            |dot, delta| dot.opacity(0.4 + 0.6 * (delta * std::f32::consts::PI).sin()),
        )
}

pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(icon_path(name))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// A status row sharing the composer's top border, or a standalone pill when
/// there is no editable composer. Parent and subagent docks use the same shell.
pub fn composer_status_strip(joined: bool, colors: &Theme) -> Div {
    div()
        .mx(px(12.))
        .min_w_0()
        .min_h(px(48.))
        .pr(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
        .bg(colors.panel)
        .border_1()
        .border_color(colors.line_strong)
        .map(|strip| {
            if joined {
                strip.mb(px(-1.)).border_b_0().rounded_t(px(24.))
            } else {
                strip.rounded(px(24.))
            }
        })
}

/// A 48 dp round touch target around a 20 dp icon.
pub fn tap(id: impl Into<ElementId>, glyph: &str, colors: &Theme) -> Stateful<Div> {
    let id = id.into();
    let probe = crate::testing::probe(format!("{id:?}"));
    div()
        .id(id)
        .relative()
        .child(probe)
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
        .px(px(4.))
        .child(leading)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .px(px(4.))
                .child(
                    div()
                        .text_size(px(18.))
                        .line_height(px(24.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate()
                        .child(title.into()),
                )
                .children(subtitle.map(|subtitle| {
                    div()
                        .text_size(px(12.5))
                        .line_height(px(16.))
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

/// A pill button, 48 dp tall (40 dp when small), with a 20 dp icon (16 dp when small).
pub fn button(
    id: impl Into<ElementId>,
    kind: Button,
    glyph: Option<&str>,
    text: impl Into<SharedString>,
    small: bool,
    colors: &Theme,
) -> Stateful<Div> {
    button_glyph(
        id,
        kind,
        glyph,
        if small { 16. } else { 20. },
        text,
        small,
        colors,
    )
}

/// A pill button with an icon of the given size.
pub fn button_glyph(
    id: impl Into<ElementId>,
    kind: Button,
    glyph: Option<&str>,
    glyph_size: f32,
    text: impl Into<SharedString>,
    small: bool,
    colors: &Theme,
) -> Stateful<Div> {
    let id = id.into();
    let probe = crate::testing::probe(format!("{id:?}"));
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
        .relative()
        .child(probe)
        .h(px(if small { 40. } else { 48. }))
        .px(px(if small { 16. } else { 24. }))
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
        .children(glyph.map(|glyph| icon(glyph, glyph_size, foreground)))
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
    let id = id.into();
    let probe = crate::testing::probe(format!("{id:?}"));
    div()
        .id(id)
        .relative()
        .child(probe)
        .h(px(32.))
        .px(px(12.))
        .flex()
        .flex_none()
        .max_w_full()
        .min_w_0()
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
        .child(div().min_w_0().truncate().child(text.into()))
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
    let id = id.into();
    let probe = crate::testing::probe(format!("{id:?}"));
    div()
        .id(id)
        .relative()
        .child(probe)
        .min_h(px(56.))
        .px(px(16.))
        .py(px(8.))
        .flex()
        .items_center()
        .gap(px(14.))
        .when(!first, |row| row.border_t_1().border_color(colors.line))
        // A card's corners don't clip what is in it: the press stays inside them.
        .active(|style| style.bg(colors.selected).rounded(px(15.)))
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
        StageKind::Verify => colors.checking,
        StageKind::HandOff => colors.accent,
    }
}

/// A run line stretch's colour: blue looking, orange changing, grey checking
/// and amber waiting for you. Green and coral are left for results.
pub fn stretch_hue(kind: StretchKind, colors: &Theme) -> Hsla {
    match kind {
        StretchKind::Look => colors.read,
        StretchKind::Change => colors.edit,
        StretchKind::Check => colors.checking,
        StretchKind::Wait => colors.wait,
    }
}

/// A mark's colour and glyph: a pass, a failure, a question you answered.
pub fn mark_style(mark: Mark, colors: &Theme) -> (Hsla, &'static str) {
    match mark {
        Mark::Passed => (colors.green, "check"),
        Mark::Failed => (colors.coral, "x"),
        Mark::Answered => (colors.wait, "hand"),
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

const KEYWORDS: &[&str] = &[
    "as",
    "async",
    "await",
    "break",
    "case",
    "class",
    "const",
    "continue",
    "crate",
    "def",
    "do",
    "dyn",
    "elif",
    "else",
    "enum",
    "export",
    "extends",
    "fn",
    "for",
    "from",
    "function",
    "if",
    "impl",
    "import",
    "in",
    "interface",
    "let",
    "loop",
    "match",
    "mod",
    "mut",
    "new",
    "of",
    "pub",
    "raise",
    "return",
    "self",
    "static",
    "struct",
    "super",
    "throw",
    "trait",
    "try",
    "type",
    "use",
    "var",
    "where",
    "while",
    "yield",
];

const LITERALS: &[&str] = &["true", "false", "null", "None", "Some", "Ok", "Err"];

/// A line of code colored like the desktop: keywords and strings.
pub fn code_text(text: &str, colors: &Theme) -> AnyElement {
    code_text_language(text, None, colors)
}

/// Native Zed tree-sitter highlighting for fenced code. The bounded lexical
/// pass remains a fallback for missing/unknown language labels.
pub fn code_text_language(text: &str, language: Option<&str>, colors: &Theme) -> AnyElement {
    let native_highlights = language
        .map(|language| pi_markdown::highlight(text, language, colors.syntax_palette()))
        .unwrap_or_default();
    let mut spans: Vec<(String, Hsla)> = Vec::new();
    let mut push = |text: &str, color: Hsla| match spans.last_mut() {
        Some((last, last_color)) if *last_color == color => last.push_str(text),
        _ => spans.push((text.to_owned(), color)),
    };
    let mut rest = text;
    let language = language.unwrap_or_default().to_ascii_lowercase();
    let hash_comments = matches!(
        language.as_str(),
        "bash" | "sh" | "shell" | "zsh" | "python" | "py" | "yaml" | "yml" | "toml"
    );
    while !rest.is_empty() {
        if rest.starts_with("//")
            || rest.starts_with("/*")
            || (hash_comments && rest.starts_with('#'))
        {
            let end = if rest.starts_with("/*") {
                rest.find("*/").map_or(rest.len(), |index| index + 2)
            } else {
                rest.find('\n').unwrap_or(rest.len())
            };
            push(&rest[..end], colors.muted);
            rest = &rest[end..];
            continue;
        }
        let quote = rest
            .chars()
            .next()
            .filter(|c| matches!(c, '"' | '\'' | '`'));
        if let Some(quote) = quote {
            let mut escaped = false;
            let mut end = rest.len();
            for (index, character) in rest[quote.len_utf8()..].char_indices() {
                if character == quote && !escaped {
                    end = quote.len_utf8() + index + character.len_utf8();
                    break;
                }
                escaped = character == '\\' && !escaped;
                if character != '\\' {
                    escaped = false;
                }
            }
            push(&rest[..end], colors.string);
            rest = &rest[end..];
            continue;
        }
        if rest.as_bytes()[0].is_ascii_digit() {
            let end = rest
                .find(|character: char| {
                    !(character.is_ascii_alphanumeric()
                        || matches!(character, '.' | '_' | '+' | '-'))
                })
                .unwrap_or(rest.len());
            push(&rest[..end], colors.amber);
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
                } else if LITERALS.contains(&word) {
                    colors.amber
                } else if word.chars().next().is_some_and(char::is_uppercase) {
                    colors.read
                } else {
                    colors.plain
                },
            );
            rest = &rest[word_end..];
        } else {
            let length = rest.chars().next().map_or(1, char::len_utf8);
            let punctuation = &rest[..length];
            push(
                punctuation,
                if punctuation.chars().all(|character| {
                    matches!(
                        character,
                        '=' | '+' | '-' | '*' | '/' | '%' | '&' | '|' | '!' | '<' | '>' | ':'
                    )
                }) {
                    colors.muted
                } else {
                    colors.plain
                },
            );
            rest = &rest[length..];
        }
    }
    let styled_text: SharedString = spans
        .iter()
        .map(|(text, _)| text.as_str())
        .collect::<String>()
        .into();
    let mut offset = 0;
    let fallback_highlights: Vec<_> = spans
        .iter()
        .map(|(span, color)| {
            let range = offset..offset + span.len();
            offset += span.len();
            (range, gpui::HighlightStyle::color(*color))
        })
        .collect();
    gpui::StyledText::new(styled_text)
        .with_highlights(if native_highlights.is_empty() {
            fallback_highlights
        } else {
            native_highlights
        })
        .into_any_element()
}

/// Compact diff rows share a gutter sized for the largest line number. Keep
/// 12 dp inside the card's edge even when numbers grow past three digits.
pub fn diff_lines<'a>(lines: &'a [DiffLine], colors: &'a Theme) -> impl Iterator<Item = Div> + 'a {
    let digits = lines
        .iter()
        .map(|line| line.number)
        .max()
        .unwrap_or(0)
        .to_string()
        .len();
    // Commit Mono at 12 dp has a 7.2 dp character advance.
    let gutter = (12. + (digits as f32 * 7.2).ceil() + 10.).max(38.);
    lines
        .iter()
        .map(move |line| diff_line(line, false, gutter, colors))
}

/// A diff line: number, then code; added and removed lines are tinted.
fn diff_line(line: &DiffLine, selected: bool, gutter: f32, colors: &Theme) -> Div {
    let number = line.number;
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
                .w(px(gutter))
                .flex_none()
                .flex()
                .items_start()
                .justify_end()
                .pl(px(12.))
                .pr(px(10.))
                .whitespace_nowrap()
                .text_color(colors.muted)
                .child(
                    div()
                        .flex_none()
                        .debug_selector(move || format!("diff-number-{number}"))
                        .child(if number == 0 {
                            String::new()
                        } else {
                            number.to_string()
                        }),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .pr(px(8.))
                .debug_selector(move || format!("diff-code-{number}"))
                .child(code_text(&line.text, colors)),
        )
}

/// A soft ring around a live mark, in its hue.
pub fn ring(hue: Hsla, spread: f32, opacity: f32) -> Vec<gpui::BoxShadow> {
    vec![gpui::BoxShadow {
        color: hue.opacity(opacity),
        offset: gpui::point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(spread),
        inset: false,
    }]
}

/// An 8 dp status dot with a 4 dp ring: a working run.
pub fn ring_dot(hue: Hsla) -> Div {
    div()
        .size(px(16.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.))
        .bg(hue.opacity(0.22))
        .child(div().size(px(8.)).rounded(px(4.)).bg(hue))
}

/// A stretch's bar: solid in its colour, or a row of dots for a wait for you,
/// so a wait reads as one without its colour. `live` rings the stretch under way.
fn stretch_bar(kind: StretchKind, live: bool, colors: &Theme) -> Div {
    let hue = stretch_hue(kind, colors);
    let bar = div().size_full().rounded(px(2.));
    if kind == StretchKind::Wait {
        bar.overflow_hidden()
            .flex()
            .gap(px(2.))
            .children((0..96).map(|_| div().w(px(4.)).h_full().flex_none().rounded(px(1.)).bg(hue)))
    } else {
        bar.bg(hue)
            .when(live, |bar| bar.shadow(ring(hue, 2., 0.28)))
    }
}

/// Where a stretch ended: a disc in the result's colour, ringed in the canvas
/// so it stands off the line, with its glyph when there is room for one.
fn end_mark(mark: Mark, size: f32, colors: &Theme) -> Div {
    let (hue, glyph) = mark_style(mark, colors);
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(size / 2.))
        .bg(hue)
        .shadow(ring(colors.canvas, (size / 7.).clamp(1., 2.), 1.))
        .when(size > 12., |disc| {
            disc.child(icon(glyph, size - 4., colors.composer))
        })
}

/// The mark a stretch ends in at this size: a small mark leaves out the
/// answered question, which the dotted wait already shows.
fn shown_mark(stretch: &Stretch, live: bool, size: f32) -> Option<Mark> {
    let mark = stretch
        .mark
        .or((live && stretch.kind == StretchKind::Wait).then_some(Mark::Answered));
    mark.filter(|mark| size >= 10. || *mark != Mark::Answered)
}

/// A stretch on a line, 4 dp tall: its bar, and how it ended at its right
/// end. A question still waiting shows its hand there already.
fn stretch_piece(stretch: &Stretch, live: bool, mark: f32, colors: &Theme) -> Div {
    let end = shown_mark(stretch, live, mark);
    div()
        .relative()
        .h(px(4.))
        .child(stretch_bar(stretch.kind, live, colors))
        .children(end.map(|end| {
            div()
                .absolute()
                .top(px(2. - mark / 2.))
                .right(px(-1. - mark / 2.))
                .child(end_mark(end, mark, colors))
        }))
}

/// One run on Home's shared track: each stretch as long as it took, in the
/// order it happened, on a track `span` wide, so a run that has gone on
/// longer shows longer. The stretch under way is ringed.
pub fn track(run: &[Stretch], live: bool, span: std::time::Duration, colors: &Theme) -> Div {
    let mut left = 1f32;
    let count = run.len();
    div().h(px(10.)).w_full().flex().items_center().child(
        div()
            .w_full()
            .h(px(4.))
            .flex()
            .gap(px(2.))
            .rounded(px(2.))
            .bg(colors.raised)
            .children(run.iter().enumerate().map(|(index, stretch)| {
                let fraction = (stretch.time.as_secs_f32() / span.as_secs_f32().max(1.))
                    .max(0.02)
                    .min(left.max(0.));
                left -= fraction;
                stretch_piece(stretch, live && index + 1 == count, 10., colors)
                    .w(relative(fraction))
                    .flex_none()
            })),
    )
}

/// A run as one line: each stretch in its colour, as long as it took and in
/// the order it happened, each check ending in its result. A run whose times
/// weren't reported gets equal stretches. `mark` is the size of the marks:
/// 14 dp with glyphs on a full line, 10 dp folded into the working strip.
pub fn run_line(run: &[Stretch], live: bool, mark: f32, colors: &Theme) -> Div {
    run_line_named(run, live, mark, None, colors)
}

/// The run line with each stretch named for tests: `{name}-{index}`, and the
/// one under way also `{name}-live-{kind}`, by its place in `StretchKind`.
pub fn run_line_named(
    run: &[Stretch],
    live: bool,
    mark: f32,
    name: Option<&'static str>,
    colors: &Theme,
) -> Div {
    let total: f32 = run.iter().map(|stretch| stretch.time.as_secs_f32()).sum();
    let count = run.len();
    div()
        .h(px(mark.max(10.)))
        .w_full()
        .flex()
        .items_center()
        .gap(px(2.))
        .children(run.iter().enumerate().map(|(index, stretch)| {
            let share = if total > 0. {
                stretch.time.as_secs_f32() / total
            } else {
                1. / count as f32
            };
            let now = live && index + 1 == count;
            let kind = stretch.kind as usize;
            // A stretch that ends in a mark is wide enough to show it whole.
            let least = if shown_mark(stretch, now, mark).is_some() {
                mark + 4.
            } else {
                4.
            };
            let piece = stretch_piece(stretch, now, mark, colors)
                .when_some(name.filter(|_| now), |piece, name| {
                    piece.debug_selector(move || format!("{name}-live-{kind}"))
                });
            div()
                .h(px(4.))
                .w(relative(share))
                .min_w(px(least))
                .when_some(name, |outer, name| {
                    outer.debug_selector(move || format!("{name}-{index}"))
                })
                .child(piece.w_full())
        }))
}

/// The key under a finished line: each kind's icon, its name and the time it
/// took in all. Times are left out when the computer gave none.
pub fn run_key(run: &[Stretch], colors: &Theme) -> Div {
    let timed = run.iter().any(|stretch| !stretch.time.is_zero());
    div()
        .mt(px(10.))
        .flex()
        .flex_wrap()
        .gap_x(px(16.))
        .gap_y(px(4.))
        .children(
            [
                StretchKind::Look,
                StretchKind::Change,
                StretchKind::Check,
                StretchKind::Wait,
            ]
            .into_iter()
            // Each kind there was; when the run is timed, each that took time.
            .filter(|kind| {
                run.iter()
                    .any(|stretch| stretch.kind == *kind && (!timed || !stretch.time.is_zero()))
            })
            .map(|kind| {
                let time: std::time::Duration = run
                    .iter()
                    .filter(|stretch| stretch.kind == kind)
                    .map(|stretch| stretch.time)
                    .sum();
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .whitespace_nowrap()
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .text_color(colors.secondary)
                    .child(icon(kind.glyph(), 16., stretch_hue(kind, colors)))
                    .child(match kind {
                        StretchKind::Wait => "Waiting",
                        kind => kind.title(),
                    })
                    .when(timed, |entry| {
                        entry.child(
                            div()
                                .text_size(px(12.5))
                                .text_color(colors.muted)
                                .child(crate::model::duration_label(time)),
                        )
                    })
            }),
        )
}

/// A run line station: the stage's icon in a 28 dp circle; tinted when done,
/// ringed while live, dashed ahead. `hue` overrides the stage's (waiting).
pub fn station(glyph: &str, hue: Hsla, status: StageStatus, colors: &Theme) -> Div {
    let base = div()
        .size(px(28.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(14.));
    match status {
        StageStatus::Done => base
            .bg(colors.canvas.blend(hue.opacity(0.16)))
            .child(icon(glyph, 14., hue)),
        StageStatus::Live => base
            .bg(colors.composer)
            .border_2()
            .border_color(hue)
            .shadow(ring(hue, 4., 0.16))
            .child(icon(glyph, 14., hue)),
        StageStatus::Planned | StageStatus::Skipped => base
            .border(px(1.5))
            .border_dashed()
            .border_color(colors.line_strong)
            .child(icon(glyph, 14., colors.faint)),
    }
}

/// Change size as five blocks, split between added and removed.
pub fn blocks(added: u32, removed: u32, colors: &Theme) -> Div {
    let total = added + removed;
    let filled = total.min(5);
    let removed_blocks = if total == 0 {
        0
    } else {
        ((removed as f32 / total as f32) * filled as f32).round() as u32
    }
    .min(filled);
    let added_blocks = filled - removed_blocks;
    div()
        .flex()
        .flex_none()
        .gap(px(2.))
        .children((0..5).map(|index| {
            div()
                .size(px(7.))
                .rounded(px(2.))
                .bg(if index < added_blocks {
                    colors.green
                } else if index < filled {
                    colors.coral
                } else {
                    colors.raised
                })
        }))
}

/// A diff line as the thread and Review draw it: 22 dp lines of 12.5 dp mono,
/// numbers in a `gutter` wide column; selected lines carry an accent edge.
/// Long lines wrap, or run on past the edge for a scrolling parent to show.
pub fn code_line(line: &DiffLine, selected: bool, gutter: f32, wrap: bool, colors: &Theme) -> Div {
    code_line_in(line, selected, gutter, wrap, None, colors)
}

/// A diff line highlighted in its file's language: "typescript", "rust".
pub fn code_line_in(
    line: &DiffLine,
    selected: bool,
    gutter: f32,
    wrap: bool,
    language: Option<&str>,
    colors: &Theme,
) -> Div {
    let background = match line.kind {
        LineKind::Added => Some(colors.added),
        LineKind::Removed => Some(colors.removed),
        LineKind::Context => None,
    };
    div()
        .relative()
        .flex()
        .w_full()
        .min_h(px(22.))
        .font_family(MONO)
        .text_size(px(12.5))
        .line_height(px(22.))
        .text_color(colors.plain)
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
                .w(px(gutter))
                .flex_none()
                .pr(px(12.))
                .whitespace_nowrap()
                .text_right()
                .text_color(colors.faint)
                .child(if line.number == 0 {
                    String::new()
                } else {
                    line.number.to_string()
                }),
        )
        .child(
            div()
                .pr(px(8.))
                .map(|text| {
                    if wrap {
                        text.flex_1().min_w_0()
                    } else {
                        text.flex_none().whitespace_nowrap()
                    }
                })
                .child(code_text_language(&line.text, language, colors)),
        )
}
