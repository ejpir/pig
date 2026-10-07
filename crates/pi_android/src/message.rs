//! Readable Markdown replies, with intact code and explicit copy actions.

use crate::{
    theme::{MONO, SANS, Theme},
    ui,
};
use gpui::{
    ClipboardItem, Div, FontStyle, FontWeight, InteractiveText, ObjectFit, StrikethroughStyle,
    StyledText, TextRun, UnderlineStyle, div, font, img, prelude::*, px, relative,
};

pub fn render(source: &str, colors: &Theme) -> Div {
    div().min_w_0().flex().flex_col().gap(px(12.)).children(
        pi_markdown::blocks(source)
            .into_iter()
            .enumerate()
            .map(|(index, block)| {
                let text = block.text();
                if let Some(pi_markdown::Media::Image(image)) = block.media.as_ref() {
                    return div()
                        .id(("markdown-image", index))
                        .relative()
                        .child(crate::testing::probe(format!("markdown-image-{index}")))
                        .debug_selector(|| format!("markdown-image-{index}"))
                        .aria_label(if text.is_empty() {
                            "Embedded image".into()
                        } else {
                            text.clone()
                        })
                        .min_w_0()
                        .w_full()
                        .max_h(px(360.))
                        .rounded(px(12.))
                        .overflow_hidden()
                        .bg(colors.panel)
                        .child(
                            img(image.clone())
                                .w_full()
                                .max_h(px(360.))
                                .rounded(px(12.))
                                .object_fit(ObjectFit::Contain),
                        )
                        .into_any_element();
                }
                if let Some(pi_markdown::Media::Mermaid(source)) = block.media.as_ref()
                    && let Some(image) = pi_markdown::mermaid_image(
                        source,
                        pi_markdown::DiagramPalette {
                            dark: colors.dark,
                            background: colors.canvas,
                            panel: colors.panel,
                            raised: colors.raised,
                            line: colors.line,
                            line_strong: colors.line_strong,
                            text: colors.text,
                            muted: colors.muted,
                            accent: colors.accent,
                            amber: colors.amber,
                            coral: colors.coral,
                            green: colors.green,
                            steel: colors.read,
                        },
                    )
                {
                    let copied = source.clone();
                    return div()
                        .id(("markdown-mermaid", index))
                        .relative()
                        .child(crate::testing::probe(format!("markdown-mermaid-{index}")))
                        .debug_selector(|| format!("markdown-mermaid-{index}"))
                        .min_w_0()
                        .rounded(px(12.))
                        .bg(colors.panel)
                        .border_1()
                        .border_color(colors.line)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .pl(px(12.))
                                .child(ui::label("Mermaid", colors).flex_1())
                                .child(
                                    ui::tap(("copy-mermaid", index), "copy", colors)
                                        .size(px(40.))
                                        .aria_label("Copy Mermaid source")
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                copied.clone(),
                                            ))
                                        }),
                                ),
                        )
                        .child(
                            img(image)
                                .w_full()
                                .max_h(px(360.))
                                // Inside the 12 dp border: its corners clip nothing.
                                .rounded_b(px(11.))
                                .object_fit(ObjectFit::Contain),
                        )
                        .into_any_element();
                }
                if block.code {
                    let copied = text.clone();
                    let language = block.language.clone();
                    let code_label = language
                        .as_deref()
                        .filter(|language| !language.is_empty())
                        .unwrap_or("Code")
                        .to_owned();
                    return div()
                        .min_w_0()
                        .rounded(px(12.))
                        .bg(colors.panel)
                        .border_1()
                        .border_color(colors.line)
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .items_center()
                                .pl(px(12.))
                                .child(ui::label(code_label, colors))
                                .child(
                                    ui::tap(("copy-code", index), "copy", colors)
                                        .size(px(40.))
                                        .aria_label("Copy code")
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                copied.clone(),
                                            ))
                                        }),
                                ),
                        )
                        // Code wraps to keep every character reachable on a phone.
                        .child(
                            div()
                                .px(px(12.))
                                .pb(px(12.))
                                .font_family(MONO)
                                .text_size(px(12.5))
                                .line_height(relative(1.55))
                                .text_color(colors.plain)
                                .child(ui::code_text_language(&text, language.as_deref(), colors)),
                        )
                        .into_any_element();
                }
                let mut offset = 0;
                let mut ranges = Vec::new();
                let mut urls = Vec::new();
                let runs = block
                    .spans
                    .into_iter()
                    .map(|span| {
                        let mut face = font(if span.code { MONO } else { SANS });
                        if span.bold || block.heading.is_some() {
                            face.weight = FontWeight::SEMIBOLD;
                        }
                        if span.italic {
                            face.style = FontStyle::Italic;
                        }
                        let link = span.link.filter(|url| {
                            url.starts_with("https://")
                                || url.starts_with("http://")
                                || url.starts_with("mailto:")
                        });
                        let color = if link.is_some() {
                            colors.accent
                        } else {
                            colors.text
                        };
                        if let Some(url) = link.as_ref() {
                            ranges.push(offset..offset + span.text.len());
                            urls.push(url.clone());
                        }
                        offset += span.text.len();
                        TextRun {
                            len: span.text.len(),
                            font: face,
                            color,
                            background_color: span.code.then_some(colors.panel),
                            underline: link.map(|_| UnderlineStyle {
                                color: Some(color),
                                thickness: px(1.),
                                wavy: false,
                            }),
                            strikethrough: span.strikethrough.then_some(StrikethroughStyle {
                                thickness: px(1.),
                                color: Some(color),
                            }),
                        }
                    })
                    .collect::<Vec<_>>();
                div()
                    .min_w_0()
                    .text_size(px(match block.heading {
                        Some(1) => 20.,
                        Some(2) => 18.,
                        Some(3) => 16.,
                        Some(_) => 15.,
                        None => 15.,
                    }))
                    .line_height(relative(1.5))
                    .pl(px(block.indent.saturating_sub(1) as f32 * 12.))
                    .when(block.quote, |text| {
                        text.pl(px(12.))
                            .border_l_2()
                            .border_color(colors.line_strong)
                            .text_color(colors.muted)
                    })
                    .child(
                        InteractiveText::new(
                            ("message-block", index),
                            StyledText::new(text).with_runs(runs),
                        )
                        .on_click(ranges, move |index, _, cx| cx.open_url(&urls[index])),
                    )
                    .into_any_element()
            }),
    )
}
