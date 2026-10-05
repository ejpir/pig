//! Readable Markdown replies, with intact code and explicit copy actions.

use crate::{
    theme::{MONO, SANS, Theme},
    ui::{self, Button},
};
use gpui::{
    ClipboardItem, Div, FontStyle, FontWeight, InteractiveText, StyledText, TextRun,
    UnderlineStyle, div, font, prelude::*, px, relative,
};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};

#[derive(Clone, Default)]
struct Span {
    text: String,
    bold: bool,
    italic: bool,
    code: bool,
    link: Option<String>,
}

#[derive(Default)]
struct Block {
    spans: Vec<Span>,
    heading: bool,
    code: bool,
    indent: usize,
}

impl Block {
    fn text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }
}

fn blocks(source: &str) -> Vec<Block> {
    let mut result = Vec::new();
    let mut block = Block::default();
    let mut style = Span::default();
    let mut lists: Vec<Option<u64>> = Vec::new();
    let flush = |block: &mut Block, result: &mut Vec<Block>| {
        if !block.spans.is_empty() {
            result.push(std::mem::take(block));
        }
    };
    for event in Parser::new(source) {
        match event {
            Event::Start(Tag::Heading { .. }) => {
                flush(&mut block, &mut result);
                block.heading = true;
            }
            Event::Start(Tag::CodeBlock(_)) => {
                flush(&mut block, &mut result);
                block.code = true;
            }
            Event::Start(Tag::List(start)) => {
                flush(&mut block, &mut result);
                lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                flush(&mut block, &mut result);
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush(&mut block, &mut result);
                block.indent = lists.len();
                let prefix = match lists.last_mut() {
                    Some(Some(number)) => {
                        let text = format!("{number}. ");
                        *number += 1;
                        text
                    }
                    _ => "• ".into(),
                };
                block.spans.push(Span {
                    text: prefix,
                    ..Default::default()
                });
            }
            Event::Start(Tag::Strong) => style.bold = true,
            Event::End(TagEnd::Strong) => style.bold = false,
            Event::Start(Tag::Emphasis) => style.italic = true,
            Event::End(TagEnd::Emphasis) => style.italic = false,
            Event::Start(Tag::Link { dest_url, .. }) => style.link = Some(dest_url.into_string()),
            Event::End(TagEnd::Link) => style.link = None,
            Event::End(
                TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::CodeBlock | TagEnd::Item,
            ) => flush(&mut block, &mut result),
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                block.spans.push(Span {
                    text: text.into_string(),
                    ..style.clone()
                })
            }
            Event::Code(text) => block.spans.push(Span {
                text: text.into_string(),
                code: true,
                ..style.clone()
            }),
            Event::SoftBreak | Event::HardBreak => block.spans.push(Span {
                text: "\n".into(),
                ..style.clone()
            }),
            Event::Rule => {
                flush(&mut block, &mut result);
                block.spans.push(Span {
                    text: "────────".into(),
                    ..Default::default()
                });
                flush(&mut block, &mut result);
            }
            _ => {}
        }
    }
    flush(&mut block, &mut result);
    result
}

pub fn render(source: &str, colors: &Theme) -> Div {
    div().min_w_0().flex().flex_col().gap(px(12.)).children(
        blocks(source)
            .into_iter()
            .enumerate()
            .map(|(index, block)| {
                let text = block.text();
                if block.code {
                    let copied = text.clone();
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
                                .px(px(12.))
                                .child(ui::label("Code", colors))
                                .child(
                                    ui::button(
                                        ("copy-code", index),
                                        Button::Quiet,
                                        Some("copy"),
                                        "Copy",
                                        true,
                                        colors,
                                    )
                                    .h(px(48.))
                                    .on_click(
                                        move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                copied.clone(),
                                            ))
                                        },
                                    ),
                                ),
                        )
                        // Code wraps to keep every character reachable on a phone.
                        .child(
                            ui::mono(text, 12.5)
                                .px(px(12.))
                                .pb(px(12.))
                                .line_height(relative(1.55)),
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
                        if span.bold || block.heading {
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
                            strikethrough: None,
                        }
                    })
                    .collect::<Vec<_>>();
                div()
                    .min_w_0()
                    .text_size(px(if block.heading { 18. } else { 15. }))
                    .line_height(relative(1.5))
                    .pl(px(block.indent.saturating_sub(1) as f32 * 12.))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_lists_links_and_long_answers_preserve_their_content() {
        let source = format!(
            "# Result\n\n**Important** [docs](https://example.com)\n\n1. First\n2. Second\n\n```sh\nprintf 'hello'\n```\n\n{}END",
            "paragraph\n\n".repeat(200)
        );
        let blocks = blocks(&source);
        assert!(blocks[0].heading);
        assert!(
            blocks
                .iter()
                .any(|b| b.code && b.text() == "printf 'hello'\n")
        );
        assert!(blocks.iter().any(|b| b.text() == "2. Second"));
        assert_eq!(blocks.last().unwrap().text(), "END");
        assert!(
            blocks
                .iter()
                .flat_map(|b| &b.spans)
                .any(|s| s.link.as_deref() == Some("https://example.com"))
        );
    }
}
