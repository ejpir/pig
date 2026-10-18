//! Readable Markdown replies, with intact code and explicit copy actions.

use crate::{
    app::{PhoneApp, Route},
    text_area::TextArea,
    theme::{MONO, SANS, Theme},
    ui,
};
use gpui::{
    ClipboardItem, Div, Entity, Focusable, FontStyle, FontWeight, InteractiveText, ObjectFit,
    StrikethroughStyle, StyledText, TextRun, UnderlineStyle, WeakEntity, div, font, img,
    prelude::*, px, relative,
};
use gpui_android::activity;

#[derive(Clone)]
struct InlineSelector {
    active: Option<String>,
    area: Entity<TextArea>,
    app: WeakEntity<PhoneApp>,
}

pub fn render(source: &str, colors: &Theme) -> Div {
    render_blocks(source, colors, "", None)
}

/// Markdown in a chat thread remains rich and link-aware until it is held. The
/// one block under the finger then becomes the app's retained read-only text
/// surface in exactly the same place, rather than opening a selection drawer.
pub fn render_selectable(
    source: &str,
    owner: impl Into<String>,
    colors: &Theme,
    app: &PhoneApp,
    cx: &gpui::Context<PhoneApp>,
) -> Div {
    let owner = owner.into();
    render_blocks(
        source,
        colors,
        &owner,
        Some(InlineSelector {
            active: app.inline_selection.clone(),
            area: app.inline_selectable.clone(),
            app: cx.entity().downgrade(),
        }),
    )
}

/// Plain chat text uses the same retained in-place selection surface as rich
/// Markdown, while inheriting its size and line height from the bubble.
pub fn plain_selectable(
    text: impl Into<String>,
    owner: impl Into<String>,
    color: gpui::Hsla,
    app: &PhoneApp,
    cx: &gpui::Context<PhoneApp>,
) -> Div {
    let text = text.into();
    let key = format!("{}:plain", owner.into());
    let selector = InlineSelector {
        active: app.inline_selection.clone(),
        area: app.inline_selectable.clone(),
        app: cx.entity().downgrade(),
    };
    let selected = selector.active.as_deref() == Some(key.as_str());
    let runs = vec![TextRun {
        len: text.len(),
        font: font(SANS),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    div()
        .relative()
        .debug_selector(|| "plain-chat-text".into())
        .min_w_0()
        .child(if selected {
            selector.area.clone().into_any_element()
        } else {
            text.clone().into_any_element()
        })
        .children(
            (!selected && !text.is_empty()).then(|| selection_target(key, text, runs, selector)),
        )
}

fn selection_target(
    key: String,
    text: String,
    runs: Vec<TextRun>,
    selector: InlineSelector,
) -> impl IntoElement {
    gpui::canvas(
        |bounds, window, _| window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
        move |_, hitbox, window, _| {
            let key = key.clone();
            let text = text.clone();
            let runs = runs.clone();
            let selector = selector.clone();
            window.on_mouse_event(move |event: &gpui::LongPressEvent, phase, window, cx| {
                if phase != gpui::DispatchPhase::Bubble
                    || event.phase != gpui::TouchPhase::Started
                    || !hitbox.is_hovered(window)
                {
                    return;
                }
                window.prevent_default();
                cx.stop_propagation();
                activity::long_press_feedback();
                let key = key.clone();
                let text = text.clone();
                let runs = runs.clone();
                let area = selector.area.clone();
                selector
                    .app
                    .update(cx, |app, cx| {
                        if let Route::Thread(id)
                        | Route::Subagent(id, _)
                        | Route::SwarmPeer(id, _, _) = app.route()
                            && let Some(list) = app.thread_lists.get(&id)
                        {
                            list.pause_following_tail();
                        }
                        app.inline_selection = Some(key);
                        area.update(cx, |area, cx| {
                            area.show_presented(text, runs, event.start_position, cx)
                        });
                        // The overlay disappears as soon as selection starts.
                        // Keep ownership of this same hold-and-drag stream so
                        // its Moved/Ended events reach the replacement area.
                        window.capture_long_press(&area);
                        window.focus(&area.read(cx).focus_handle(cx), cx);
                        cx.notify();
                    })
                    .ok();
            });
        },
    )
    .absolute()
    .inset_0()
}

fn selectable_prose_block(block: &pi_markdown::Block) -> bool {
    !block.code && block.media.is_none() && block.heading.is_none()
}

fn same_selectable_prose_run(left: &pi_markdown::Block, right: &pi_markdown::Block) -> bool {
    selectable_prose_block(left)
        && selectable_prose_block(right)
        && left.quote == right.quote
        // A flat text layout cannot retain a nested block's full left margin
        // on wrapped lines. Keep depth changes as honest visual boundaries.
        && (left.indent <= 1 && right.indent <= 1 || left.indent == right.indent)
}

fn render_prose(
    blocks: &[pi_markdown::Block],
    index: usize,
    owner: &str,
    colors: &Theme,
    selector: Option<InlineSelector>,
) -> gpui::AnyElement {
    let first = &blocks[0];
    let mut text = String::new();
    let mut ranges = Vec::new();
    let mut urls = Vec::new();
    let mut runs = Vec::new();
    for (block_index, block) in blocks.iter().enumerate() {
        if block_index > 0 {
            let previous = &blocks[block_index - 1];
            // List items are consecutive lines. Other Markdown blocks retain
            // a blank line, but all remain in one selectable text surface so
            // a handle can cross the paragraph/list boundary.
            let separator = if block.list_group.is_some() && block.list_group == previous.list_group
            {
                "\n"
            } else {
                "\n\n"
            };
            text.push_str(separator);
            runs.push(TextRun {
                len: separator.len(),
                font: font(SANS),
                color: colors.text,
                background_color: None,
                underline: None,
                strikethrough: None,
            });
        }
        // Ordinary spaces preserve nested-list structure in both the rendered
        // text and copied plaintext while retaining one selectable layout.
        let prefix = "  ".repeat(block.indent.saturating_sub(first.indent.max(1)));
        if !prefix.is_empty() {
            text.push_str(&prefix);
            runs.push(TextRun {
                len: prefix.len(),
                font: font(SANS),
                color: colors.text,
                background_color: None,
                underline: None,
                strikethrough: None,
            });
        }
        for span in &block.spans {
            let offset = text.len();
            text.push_str(&span.text);
            let mut face = font(if span.code { MONO } else { SANS });
            if span.bold || block.heading.is_some() {
                face.weight = FontWeight::SEMIBOLD;
            }
            if span.italic {
                face.style = FontStyle::Italic;
            }
            let link = span.link.as_deref().filter(|url| {
                url.starts_with("https://")
                    || url.starts_with("http://")
                    || url.starts_with("mailto:")
            });
            let color = if link.is_some() {
                colors.accent
            } else {
                colors.text
            };
            if let Some(url) = link {
                ranges.push(offset..offset + span.text.len());
                urls.push(url.to_owned());
            }
            runs.push(TextRun {
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
            });
        }
    }
    let key = format!("{owner}:markdown:{index}");
    let selected = selector
        .as_ref()
        .is_some_and(|selector| selector.active.as_deref() == Some(key.as_str()));
    let content = if selected {
        selector.as_ref().unwrap().area.clone().into_any_element()
    } else {
        InteractiveText::new(
            ("message-block", index),
            StyledText::new(text.clone()).with_runs(runs.clone()),
        )
        .on_click(ranges, move |index, _, cx| cx.open_url(&urls[index]))
        .into_any_element()
    };
    div()
        .relative()
        .debug_selector(move || format!("markdown-text-block-{index}"))
        .min_w_0()
        .text_size(px(match first.heading {
            Some(1) => 20.,
            Some(2) => 18.,
            Some(3) => 16.,
            Some(_) => 15.,
            None => 15.,
        }))
        .line_height(relative(1.5))
        .pl(px(first.indent.saturating_sub(1) as f32 * 12.))
        .when(first.quote, |text| {
            text.pl(px(12.))
                .border_l_2()
                .border_color(colors.line_strong)
                .text_color(colors.muted)
        })
        .child(content)
        .children(
            selector
                .filter(|_| !selected && !text.is_empty())
                .map(|selector| selection_target(key, text, runs, selector)),
        )
        .into_any_element()
}

fn render_blocks(
    source: &str,
    colors: &Theme,
    owner: &str,
    selector: Option<InlineSelector>,
) -> Div {
    let blocks = pi_markdown::blocks_cached(source);
    let children = (0..blocks.len())
        .filter(|index| {
            *index == 0 || !same_selectable_prose_run(&blocks[*index - 1], &blocks[*index])
        })
        .map(|index| {
            let block = &blocks[index];
            if selectable_prose_block(block) {
                let mut end = index + 1;
                while end < blocks.len() && same_selectable_prose_run(block, &blocks[end]) {
                    end += 1;
                }
                if end > index + 1 {
                    return render_prose(
                        &blocks[index..end],
                        index,
                        owner,
                        colors,
                        selector.clone(),
                    );
                }
            }
            {
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
                        // No larger than it is, and never wider than the reply.
                        .child(
                            img(image.image.clone())
                                .w(px(image.width as f32))
                                .max_w_full()
                                .max_h(px(360.))
                                .aspect_ratio(image.ratio)
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
                let key = format!("{owner}:markdown:{index}");
                let selected = selector
                    .as_ref()
                    .is_some_and(|selector| selector.active.as_deref() == Some(key.as_str()));
                if block.code {
                    let copied = text.clone();
                    let language = block.language.clone();
                    let code_label = language
                        .as_deref()
                        .filter(|language| !language.is_empty())
                        .unwrap_or("Code")
                        .to_owned();
                    let runs = vec![TextRun {
                        len: text.len(),
                        font: font(MONO),
                        color: colors.plain,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }];
                    let body = div()
                        .relative()
                        .debug_selector(move || format!("markdown-code-block-{index}"))
                        .px(px(12.))
                        .pb(px(12.))
                        .font_family(MONO)
                        .text_size(px(12.5))
                        .line_height(relative(1.55))
                        .text_color(colors.plain)
                        .child(if selected {
                            selector.as_ref().unwrap().area.clone().into_any_element()
                        } else {
                            ui::code_text_language(&text, language.as_deref(), colors)
                        })
                        .children(
                            selector
                                .clone()
                                .filter(|_| !selected && !text.is_empty())
                                .map(|selector| {
                                    selection_target(key, text.clone(), runs, selector)
                                }),
                        );
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
                        .child(body)
                        .into_any_element();
                }
                render_prose(
                    std::slice::from_ref(block),
                    index,
                    owner,
                    colors,
                    selector.clone(),
                )
            }
        });
    div()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(12.))
        .children(children)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prose_selection_crosses_lists_but_keeps_honest_visual_boundaries() {
        let blocks = pi_markdown::blocks("Before.\n\n- One\n- Two\n\nAfter.");
        assert!(
            blocks
                .windows(2)
                .all(|pair| same_selectable_prose_run(&pair[0], &pair[1]))
        );

        let quoted = pi_markdown::blocks("> - One\n> - Two");
        assert_eq!(quoted.len(), 2);
        assert!(same_selectable_prose_run(&quoted[0], &quoted[1]));

        let nested = pi_markdown::blocks("- Root\n  - Nested\n- Root again");
        assert_eq!(nested.len(), 3);
        assert!(!same_selectable_prose_run(&nested[0], &nested[1]));
        assert!(!same_selectable_prose_run(&nested[1], &nested[2]));
    }
}
