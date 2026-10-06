//! Chat Markdown through Zed's `markdown` crate, styled with the Evening/Moonstone
//! palette. Zed components inside it (code blocks, tables, copy buttons, tooltips)
//! read Zed's global theme, so that theme mirrors the app palette.
use std::sync::Arc;

use gpui::{
    AnyElement, App, ClipboardItem, ElementId, Entity, FontWeight, Hsla, IntoElement as _,
    KeyBinding, Refineable as _, SharedString, StyleRefinement, Styled as _, TextStyleRefinement,
    UnderlineStyle, Window, point, px, relative, rgb,
};
use markdown::{BlockQuoteKindColors, HeadingLevelStyles, Markdown, MarkdownStyle};
use ui::{ContextMenu, prelude::FluentBuilder as _, right_click_menu};
use zed_theme::{ActiveTheme as _, Appearance, GlobalTheme};

pub use pi_markdown::{Documents, LinkHandler, Source, element};

use crate::theme::{MONO, SANS, Theme};

/// Requires the app `Theme` global.
pub fn init(cx: &mut App) {
    pi_markdown::init(cx);
    let menu = Some("menu");
    cx.bind_keys([
        // Zed's context menus act on `menu::*` actions; Zed binds them in its keymap.
        KeyBinding::new("up", menu::SelectPrevious, menu),
        KeyBinding::new("down", menu::SelectNext, menu),
        KeyBinding::new("home", menu::SelectFirst, menu),
        KeyBinding::new("end", menu::SelectLast, menu),
        KeyBinding::new("enter", menu::Confirm, menu),
        KeyBinding::new("escape", menu::Cancel, menu),
    ]);
    sync_theme(cx);
}

/// Call after replacing the app `Theme` global.
pub fn sync_theme(cx: &mut App) {
    let palette = *cx.global::<Theme>();
    let mut theme = (**cx.theme()).clone();
    theme.id = if palette.light {
        "pi-moonstone"
    } else {
        "pi-evening"
    }
    .into();
    theme.name = if palette.light {
        "Moonstone"
    } else {
        "Evening"
    }
    .into();
    theme.appearance = if palette.light {
        Appearance::Light
    } else {
        Appearance::Dark
    };
    let transparent = gpui::transparent_black();
    let colors = &mut theme.styles.colors;
    colors.text = palette.text;
    colors.text_muted = palette.muted;
    colors.text_placeholder = palette.faint;
    colors.text_disabled = palette.faint;
    colors.text_accent = palette.accent;
    colors.icon = palette.muted;
    colors.icon_muted = palette.faint;
    colors.icon_disabled = palette.faint;
    colors.icon_placeholder = palette.faint;
    colors.icon_accent = palette.accent;
    colors.border = palette.line;
    colors.border_variant = palette.line;
    colors.border_focused = palette.focus;
    colors.border_selected = palette.accent;
    colors.border_transparent = transparent;
    colors.border_disabled = palette.line;
    colors.background = palette.canvas;
    colors.surface_background = palette.panel;
    colors.elevated_surface_background = if palette.light {
        palette.chip
    } else {
        palette.bar
    };
    colors.panel_background = palette.panel;
    colors.title_bar_background = palette.bar;
    colors.editor_background = palette.canvas;
    colors.editor_foreground = palette.plain;
    colors.editor_gutter_background = palette.canvas;
    colors.editor_active_line_background = palette.hover;
    colors.editor_highlighted_line_background = palette.selected;
    colors.editor_line_number = palette.faint;
    colors.editor_active_line_number = palette.muted;
    colors.editor_invisible = palette.line_strong;
    colors.editor_indent_guide = palette.line;
    colors.editor_indent_guide_active = palette.line_strong;
    colors.editor_document_highlight_read_background = palette.selected;
    colors.editor_document_highlight_write_background = palette.selected;
    colors.editor_document_highlight_bracket_background = palette.selected;
    colors.element_background = palette.chip;
    colors.element_hover = palette.hover;
    colors.element_active = palette.selected;
    colors.element_selected = palette.selected;
    colors.element_selection_background = palette.selection();
    colors.element_disabled = palette.raised;
    colors.ghost_element_background = transparent;
    colors.ghost_element_hover = palette.hover;
    colors.ghost_element_active = palette.selected;
    colors.ghost_element_selected = palette.selected;
    colors.ghost_element_disabled = transparent;
    colors.scrollbar_thumb_background = palette.chip_line.opacity(0.7);
    colors.scrollbar_thumb_hover_background = palette.chip_line;
    colors.scrollbar_thumb_active_background = palette.focus;
    colors.scrollbar_thumb_border = transparent;
    colors.scrollbar_track_background = transparent;
    colors.scrollbar_track_border = transparent;
    // Terminals sit on the deep surface, with ANSI colors from the palette.
    colors.terminal_background = palette.deep;
    colors.terminal_ansi_background = palette.deep;
    colors.terminal_foreground = palette.plain;
    colors.terminal_bright_foreground = palette.text;
    colors.terminal_dim_foreground = palette.muted;
    let ansi =
        |dark: u32, light: u32| -> Hsla { rgb(if palette.light { light } else { dark }).into() };
    colors.terminal_ansi_black = ansi(0x1f2630, 0x3a4453);
    colors.terminal_ansi_red = palette.coral;
    colors.terminal_ansi_green = palette.green;
    colors.terminal_ansi_yellow = palette.amber;
    colors.terminal_ansi_blue = palette.accent;
    colors.terminal_ansi_magenta = ansi(0xc49bd6, 0x8a4f9e);
    colors.terminal_ansi_cyan = palette.steel;
    colors.terminal_ansi_white = ansi(0xd5d8db, 0xe4ded8);
    colors.terminal_ansi_bright_black = palette.faint;
    colors.terminal_ansi_bright_red = ansi(0xf5a99d, 0xd0705c);
    colors.terminal_ansi_bright_green = ansi(0x7fcf98, 0x3a9f65);
    colors.terminal_ansi_bright_yellow = ansi(0xf6d185, 0xc98d2a);
    colors.terminal_ansi_bright_blue = palette.code;
    colors.terminal_ansi_bright_magenta = ansi(0xd6b6e3, 0x9d64b0);
    colors.terminal_ansi_bright_cyan = ansi(0x7fb8d6, 0x3f93bd);
    colors.terminal_ansi_bright_white = ansi(0xebe7e4, 0xfaf9f7);
    for (dim, color) in [
        (
            &mut colors.terminal_ansi_dim_black,
            colors.terminal_ansi_black,
        ),
        (&mut colors.terminal_ansi_dim_red, colors.terminal_ansi_red),
        (
            &mut colors.terminal_ansi_dim_green,
            colors.terminal_ansi_green,
        ),
        (
            &mut colors.terminal_ansi_dim_yellow,
            colors.terminal_ansi_yellow,
        ),
        (
            &mut colors.terminal_ansi_dim_blue,
            colors.terminal_ansi_blue,
        ),
        (
            &mut colors.terminal_ansi_dim_magenta,
            colors.terminal_ansi_magenta,
        ),
        (
            &mut colors.terminal_ansi_dim_cyan,
            colors.terminal_ansi_cyan,
        ),
        (
            &mut colors.terminal_ansi_dim_white,
            colors.terminal_ansi_white,
        ),
    ] {
        *dim = color.opacity(0.7);
    }
    cx.set_global(pi_terminal::TerminalStyle {
        font_family: MONO.into(),
        font_size: px(11.5),
        line_height: px(19.),
        cursor: palette.accent,
        selection: palette.selection(),
        link: palette.code,
    });
    let status = &mut theme.styles.status;
    status.success = palette.green;
    status.created = palette.green;
    status.warning = palette.amber;
    status.modified = palette.amber;
    status.error = palette.coral;
    status.error_background = palette.danger;
    status.error_border = palette.danger_line;
    status.warning_background = palette.queue;
    status.warning_border = palette.queue_line;
    status.info_background = palette.selected;
    status.info_border = palette.focus;
    status.hint_background = palette.panel;
    status.hint_border = palette.line;
    status.deleted = palette.coral;
    status.info = palette.steel;
    status.hint = palette.muted;
    theme.styles.syntax = Arc::new(pi_markdown::syntax(pi_markdown::SyntaxPalette {
        text: palette.text,
        plain: palette.plain,
        muted: palette.muted,
        faint: palette.faint,
        accent: palette.code,
        steel: palette.steel,
        amber: palette.amber,
        coral: palette.coral,
        green: palette.green,
        keyword: palette.keyword,
        string: palette.string,
        added: palette.added,
        removed: palette.removed,
        selected: palette.selected,
    }));
    let theme = Arc::new(theme);
    // Languages resolve highlight captures against the theme they were given.
    pi_markdown::set_language_theme(theme.clone(), cx);
    GlobalTheme::update_theme(cx, theme);
}

/// Text style for one transcript block. `color` is the body color; headings use `text`.
pub fn style(palette: Theme, color: Hsla, window: &Window, cx: &App) -> MarkdownStyle {
    let mut base_text_style = window.text_style();
    base_text_style.refine(&TextStyleRefinement {
        font_family: Some(SANS.into()),
        font_size: Some(px(14.).into()),
        line_height: Some(px(20.).into()),
        color: Some(color),
        ..Default::default()
    });
    let heading = |size: f32| {
        Some(TextStyleRefinement {
            font_size: Some(px(size).into()),
            font_weight: Some(FontWeight::SEMIBOLD),
            line_height: Some(relative(1.35)),
            color: Some(palette.text),
            ..Default::default()
        })
    };
    MarkdownStyle {
        base_text_style,
        // Text runs take fonts from `base_text_style`, but size from the container.
        container_style: StyleRefinement::default()
            .text_size(px(14.))
            .line_height(px(20.)),
        code_block: StyleRefinement::default()
            .mt(px(4.))
            .mb(px(10.))
            .px(px(12.))
            .py(px(10.))
            .rounded(px(6.))
            .border_1()
            .border_color(palette.line)
            .bg(palette.deep)
            .font_family(MONO)
            .text_size(px(12.))
            .line_height(px(19.))
            .text_color(palette.plain),
        code_block_overflow_x_scroll: true,
        inline_code: TextStyleRefinement {
            font_family: Some(MONO.into()),
            font_size: Some(px(12.).into()),
            color: Some(palette.code),
            background_color: Some(palette.hover),
            ..Default::default()
        },
        block_quote: TextStyleRefinement {
            color: Some(palette.muted),
            ..Default::default()
        },
        link: TextStyleRefinement {
            color: Some(palette.accent),
            underline: Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(palette.accent.opacity(0.5)),
                ..Default::default()
            }),
            ..Default::default()
        },
        rule_color: palette.line,
        block_quote_border_color: palette.chip_line,
        block_quote_kind_colors: BlockQuoteKindColors {
            note: palette.steel,
            tip: palette.green,
            important: palette.accent,
            warning: palette.amber,
            caution: palette.coral,
        },
        syntax: cx.theme().syntax().clone(),
        selection_background_color: palette.selection(),
        heading_level_styles: Some(HeadingLevelStyles {
            h1: heading(19.),
            h2: heading(17.),
            h3: heading(15.),
            h4: heading(14.),
            h5: heading(13.),
            h6: heading(13.),
        }),
        paragraph_spacing: px(10.),
        paragraph_line_height: px(20.).into(),
        list_spacing: px(4.),
        table_cell_padding: point(px(10.), px(5.)),
        // Chat text follows pi's TUI: a newline in a message is a line break.
        soft_break_as_hard_break: true,
        ..Default::default()
    }
}

/// Tool output: selectable plain text in the card's mono face.
pub fn output_style(palette: Theme, window: &Window, cx: &App) -> MarkdownStyle {
    let mut base_text_style = window.text_style();
    base_text_style.refine(&TextStyleRefinement {
        font_family: Some(MONO.into()),
        font_size: Some(px(11.).into()),
        line_height: Some(px(18.).into()),
        color: Some(palette.secondary),
        ..Default::default()
    });
    MarkdownStyle {
        base_text_style,
        // Text runs take fonts from `base_text_style`, but size from the container.
        container_style: StyleRefinement::default()
            .w_full()
            .font_family(MONO)
            .text_size(px(11.))
            .line_height(px(18.)),
        link: TextStyleRefinement {
            color: Some(palette.accent),
            ..Default::default()
        },
        selection_background_color: palette.selection(),
        paragraph_spacing: px(0.),
        paragraph_line_height: px(18.).into(),
        code_block: StyleRefinement::default()
            .font_family(MONO)
            .text_size(px(11.))
            .line_height(px(18.))
            .p(px(6.))
            .rounded(px(4.))
            .bg(palette.deep),
        code_block_overflow_x_scroll: true,
        syntax: cx.theme().syntax().clone(),
        ..Default::default()
    }
}

/// Right-click menu for one transcript block. The Markdown element records the
/// clicked link and the selection before the menu is built.
pub fn with_menu(
    id: impl Into<ElementId>,
    document: Option<Entity<Markdown>>,
    copy_all: &'static str,
    copy_range: Option<std::ops::Range<usize>>,
    body: AnyElement,
) -> AnyElement {
    right_click_menu(id)
        .trigger(move |_, _, _| body)
        .menu(move |window, cx| {
            let document = document.clone();
            let copy_range = copy_range.clone();
            ContextMenu::build(window, cx, move |menu, _, cx| {
                let markdown = document.as_ref().map(|document| document.read(cx));
                let link = markdown.and_then(|markdown| markdown.context_menu_link().cloned());
                let text =
                    markdown.and_then(|markdown| markdown.context_menu_selected_text().cloned());
                let source = markdown
                    .and_then(|markdown| markdown.context_menu_selected_markdown().cloned());
                let whole = markdown.and_then(|markdown| match &copy_range {
                    Some(range) => markdown.source().get(range.clone()).map(SharedString::from),
                    None => Some(markdown.source().clone()),
                });
                menu.when_some(link, |menu, url| {
                    menu.entry("Copy Link", None, move |_, cx| copy(&url, cx))
                        .separator()
                })
                .when_some(text, |menu, text| {
                    menu.entry("Copy", Some(Box::new(markdown::Copy)), move |_, cx| {
                        copy(&text, cx)
                    })
                })
                .when_some(source, |menu, source| {
                    menu.entry(
                        "Copy as Markdown",
                        Some(Box::new(markdown::CopyAsMarkdown)),
                        move |_, cx| copy(&source, cx),
                    )
                })
                .when_some(whole, |menu, whole| {
                    menu.entry(copy_all, None, move |_, cx| copy(&whole, cx))
                })
            })
        })
        .into_any_element()
}

pub fn copy(text: &str, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use language::Rope;

    #[gpui::test]
    fn code_previews_keep_fences_literal_and_copy_exact_unicode_source(cx: &mut TestAppContext) {
        let raw = "  π🐈\n```text\n![literal](https://example.invalid/image)\n```\n";
        let (source, range) = pi_markdown::code_source(raw, "text\n`bad");
        assert!(source.starts_with("````textbad\n"));
        assert_eq!(&source[range], raw);
        cx.update(|cx| {
            cx.set_global(Theme::new(false));
            init(cx);
        });
        let mut documents = Documents::default();
        cx.update(|cx| {
            documents.update(
                vec![(
                    "code".into(),
                    Source::Code {
                        text: raw,
                        language: "text",
                    },
                )],
                cx,
            )
        });
        cx.read(|cx| assert_eq!(documents.copy_source("code", cx), Some(raw)));
        let identity = documents.get("code").unwrap().entity_id();
        cx.update(|cx| {
            documents.update(
                vec![(
                    "code".into(),
                    Source::Code {
                        text: raw,
                        language: "text",
                    },
                )],
                cx,
            )
        });
        assert_eq!(documents.get("code").unwrap().entity_id(), identity);
    }

    #[gpui::test]
    async fn code_block_languages_resolve_by_name_or_extension_and_highlight(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            cx.set_global(Theme::new(false));
            init(cx);
        });
        let languages = cx.update(|cx| pi_markdown::languages(cx));
        let typescript = languages
            .language_for_name_or_extension("ts")
            .await
            .unwrap();
        assert_eq!(typescript.name().as_ref(), "TypeScript");
        let rust = languages
            .language_for_name_or_extension("rust")
            .await
            .unwrap();
        let code = "fn main() { let answer = \"42\"; }";
        let highlights = rust.highlight_text_resolved(&Rope::from(code), 0..code.len());
        // Captures resolve to the app palette through the theme the registry was given.
        let (palette, syntax) =
            cx.update(|cx| (*cx.global::<Theme>(), cx.theme().syntax().clone()));
        let color_of = |text: &str| {
            let start = code.find(text).unwrap();
            highlights
                .runs
                .iter()
                .find(|(range, _)| range.start == start)
                .and_then(|(_, id)| syntax.get(*id))
                .and_then(|style| style.color)
        };
        assert_eq!(color_of("fn"), Some(palette.keyword));
        assert_eq!(color_of("\"42\""), Some(palette.string));
    }

    #[gpui::test]
    fn fenced_code_blocks_get_their_language(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Theme::new(false));
            init(cx);
        });
        let mut documents = Documents::default();
        cx.update(|cx| {
            documents.sync(
                vec![(
                    "doc".into(),
                    Source::Markdown("```ts\nconst answer = \"42\";\n```".into()),
                )],
                cx,
            )
        });
        cx.run_until_parked();
        let document = documents.get("doc").unwrap().clone();
        cx.read(|cx| {
            let parsed = document.read(cx).parsed_markdown();
            let language = parsed.languages_by_name.get(&SharedString::from("ts"));
            assert_eq!(language.unwrap().name().as_ref(), "TypeScript");
        });
    }
}
