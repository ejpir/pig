//! Platform-neutral Markdown semantics used by the Android view. The desktop
//! view uses Zed's `MarkdownElement`, while both paths share the same native
//! syntax and media engines from this crate.

use base64::Engine as _;
use gpui::{Hsla, Image, ImageFormat};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{Arc, Mutex, OnceLock},
};

const MAX_EMBEDDED_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_MERMAID_SOURCE_BYTES: usize = 64 * 1024;
const MAX_CACHED_MARKDOWN_SOURCE_BYTES: usize = 256 * 1024;
const MAX_MARKDOWN_CACHE_SOURCE_BYTES: usize = 2 * 1024 * 1024;
const MAX_MARKDOWN_CACHE_ENTRIES: usize = 128;

#[derive(Clone, Default)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub code: bool,
    pub link: Option<String>,
}

#[derive(Clone)]
pub enum Media {
    /// With its size, to draw it no larger than it is.
    Image(crate::Decoded),
    Mermaid(String),
}

#[derive(Default)]
pub struct Block {
    pub spans: Vec<Span>,
    pub heading: Option<u8>,
    pub code: bool,
    pub language: Option<String>,
    pub indent: usize,
    pub quote: bool,
    pub media: Option<Media>,
}

impl Block {
    pub fn text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }
}

#[derive(Default)]
struct MarkdownCache {
    source_bytes: usize,
    entries: HashMap<String, Arc<[Block]>>,
}

static MARKDOWN_BLOCKS: OnceLock<Mutex<MarkdownCache>> = OnceLock::new();

/// Reuse immutable Markdown semantics when a retained conversation is painted
/// again after navigating away or replacing Android's native surface. The
/// source is kept as the collision-proof key, with strict bounds so image data
/// and unusually large generated replies cannot accumulate here.
pub fn blocks_cached(source: &str) -> Arc<[Block]> {
    if source.len() <= MAX_CACHED_MARKDOWN_SOURCE_BYTES {
        let cache = MARKDOWN_BLOCKS.get_or_init(Default::default);
        if let Some(blocks) = cache
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entries
            .get(source)
            .cloned()
        {
            return blocks;
        }
    }

    let parsed: Arc<[Block]> = blocks(source).into();
    if source.len() > MAX_CACHED_MARKDOWN_SOURCE_BYTES {
        return parsed;
    }

    let cache = MARKDOWN_BLOCKS.get_or_init(Default::default);
    let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(blocks) = cache.entries.get(source).cloned() {
        return blocks;
    }
    if cache.entries.len() >= MAX_MARKDOWN_CACHE_ENTRIES
        || cache.source_bytes.saturating_add(source.len()) > MAX_MARKDOWN_CACHE_SOURCE_BYTES
    {
        cache.entries.clear();
        cache.source_bytes = 0;
    }
    cache.source_bytes += source.len();
    cache.entries.insert(source.to_owned(), parsed.clone());
    parsed
}

/// Parse the full Markdown block vocabulary once, independently of the view
/// toolkit. Embedded data images are decoded here; network-backed images stay
/// as accessible alt text and never cause an implicit request from a reply.
pub fn blocks(source: &str) -> Vec<Block> {
    let source = crate::embed_svg(source);
    let source = source.as_ref();
    let mut result = Vec::new();
    let mut block = Block::default();
    let mut style = Span::default();
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut quote_depth = 0usize;
    let flush = |block: &mut Block, result: &mut Vec<Block>| {
        if !block.spans.is_empty() || block.media.is_some() {
            if block.code
                && block.language.as_deref() == Some("mermaid")
                && !block.text().trim().is_empty()
            {
                block.media = Some(Media::Mermaid(block.text()));
            }
            result.push(std::mem::take(block));
        }
    };
    let options = Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;

    for event in Parser::new_ext(source, options) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut block, &mut result);
                block.heading = Some(match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                });
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush(&mut block, &mut result);
                block.code = true;
                block.language = match kind {
                    CodeBlockKind::Fenced(language) => language
                        .split_whitespace()
                        .next()
                        .filter(|language| !language.is_empty())
                        .map(|language| language.to_ascii_lowercase()),
                    CodeBlockKind::Indented => None,
                };
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                flush(&mut block, &mut result);
                block.media = embedded_image(&dest_url).map(Media::Image);
            }
            Event::End(TagEnd::Image) => flush(&mut block, &mut result),
            Event::Start(Tag::BlockQuote(_)) => {
                flush(&mut block, &mut result);
                quote_depth += 1;
                block.quote = true;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                flush(&mut block, &mut result);
                quote_depth = quote_depth.saturating_sub(1);
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
                block.quote = quote_depth > 0;
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
            Event::Start(Tag::Strikethrough) => style.strikethrough = true,
            Event::End(TagEnd::Strikethrough) => style.strikethrough = false,
            Event::Start(Tag::Link { dest_url, .. }) => style.link = Some(dest_url.into_string()),
            Event::End(TagEnd::Link) => style.link = None,
            Event::End(
                TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::CodeBlock | TagEnd::Item,
            ) => flush(&mut block, &mut result),
            Event::End(TagEnd::TableRow | TagEnd::TableHead) => flush(&mut block, &mut result),
            Event::End(TagEnd::TableCell) => block.spans.push(Span {
                text: "  ".into(),
                ..style.clone()
            }),
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                block.quote = quote_depth > 0;
                block.spans.push(Span {
                    text: text.into_string(),
                    ..style.clone()
                });
            }
            Event::Code(text) | Event::InlineMath(text) | Event::DisplayMath(text) => {
                block.spans.push(Span {
                    text: text.into_string(),
                    code: true,
                    ..style.clone()
                })
            }
            Event::SoftBreak | Event::HardBreak => block.spans.push(Span {
                text: "\n".into(),
                ..style.clone()
            }),
            Event::TaskListMarker(checked) => block.spans.push(Span {
                text: if checked { "☑ " } else { "☐ " }.into(),
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

fn embedded_image(url: &str) -> Option<crate::Decoded> {
    let data = url.strip_prefix("data:")?;
    let (metadata, encoded) = data.split_once(',')?;
    let mut fields = metadata.split(';');
    let format = ImageFormat::from_mime_type(fields.next()?)?;
    if !fields.any(|field| field.eq_ignore_ascii_case("base64"))
        || encoded.len() > MAX_EMBEDDED_IMAGE_BYTES.saturating_mul(4) / 3 + 8
    {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    if bytes.is_empty() || bytes.len() > MAX_EMBEDDED_IMAGE_BYTES {
        return None;
    }
    if format == ImageFormat::Svg {
        let svg = std::str::from_utf8(&bytes).ok()?.trim();
        if !svg.contains("<svg") || !svg.contains("</svg>") {
            return None;
        }
    }
    crate::images::decode_as(format, bytes).ok()
}

#[derive(Clone, Copy)]
pub struct DiagramPalette {
    pub dark: bool,
    pub background: Hsla,
    pub panel: Hsla,
    pub raised: Hsla,
    pub line: Hsla,
    pub line_strong: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub amber: Hsla,
    pub coral: Hsla,
    pub green: Hsla,
    pub steel: Hsla,
}

static MERMAID_IMAGES: OnceLock<Mutex<HashMap<u64, Option<Arc<Image>>>>> = OnceLock::new();

/// Render Mermaid through the same pure-Rust `mermaid_render` and SVG pipeline
/// used by Zed. Failures fall back to the original fenced source in the view.
pub fn mermaid_image(source: &str, palette: DiagramPalette) -> Option<Arc<Image>> {
    if source.is_empty() || source.len() > MAX_MERMAID_SOURCE_BYTES {
        return None;
    }
    let key = diagram_key(source, palette);
    let cache = MERMAID_IMAGES.get_or_init(Default::default);
    if let Some(image) = cache.lock().ok()?.get(&key).cloned() {
        return image;
    }
    let branch = [
        palette.accent,
        palette.amber,
        palette.green,
        palette.coral,
        palette.steel,
        palette.muted,
        palette.line_strong,
        palette.text,
    ];
    let labels = branch.map(mermaid_render::text_color_for_background);
    let theme = mermaid_render::MermaidTheme {
        dark_mode: palette.dark,
        font_family: "IBM Plex Sans, sans-serif".into(),
        background: palette.background,
        primary_color: palette.panel,
        primary_text_color: palette.text,
        primary_border_color: palette.line_strong,
        secondary_color: palette.raised,
        tertiary_color: palette.background,
        line_color: palette.muted,
        text_color: palette.text,
        edge_label_background: palette.background,
        cluster_background: palette.panel,
        cluster_border: palette.line,
        note_background: palette.raised,
        note_border: palette.amber,
        actor_background: palette.panel,
        actor_border: palette.line_strong,
        activation_background: palette.raised,
        activation_border: palette.accent,
        git_branch_colors: branch,
        git_branch_label_colors: labels,
        er_attr_bg_odd: palette.panel,
        er_attr_bg_even: palette.raised,
        error_color: palette.coral,
        warning_color: palette.amber,
        accent_colors: branch
            .into_iter()
            .map(|background| mermaid_render::AccentColor {
                foreground: mermaid_render::text_color_for_background(background),
                background,
            })
            .collect(),
    };
    let rendered = mermaid_render::render_to_svg(source, &theme)
        .ok()
        .map(|svg| Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes())));
    if let Ok(mut cache) = cache.lock() {
        if cache.len() >= 64 {
            cache.clear();
        }
        cache.insert(key, rendered.clone());
    }
    rendered
}

fn diagram_key(source: &str, palette: DiagramPalette) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    palette.dark.hash(&mut hasher);
    for color in [
        palette.background,
        palette.panel,
        palette.raised,
        palette.line,
        palette.line_strong,
        palette.text,
        palette.muted,
        palette.accent,
        palette.amber,
        palette.coral,
        palette.green,
        palette.steel,
    ] {
        color.h.to_bits().hash(&mut hasher);
        color.s.to_bits().hash(&mut hasher);
        color.l.to_bits().hash(&mut hasher);
        color.a.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rich_blocks_embedded_svg_and_mermaid() {
        let svg = base64::engine::general_purpose::STANDARD.encode(
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><path d="M0 0h40v20H0z"/></svg>"#,
        );
        let source = format!(
            "## Result\n\n~~old~~ **new**\n\n![diagram](data:image/svg+xml;base64,{svg})\n\n```mermaid\ngraph TD; A-->B\n```"
        );
        let blocks = blocks(&source);
        assert!(blocks.iter().any(|block| block.heading == Some(2)));
        assert!(
            blocks
                .iter()
                .flat_map(|block| &block.spans)
                .any(|span| span.strikethrough)
        );
        assert!(
            blocks
                .iter()
                .any(|block| matches!(block.media, Some(Media::Image(_))))
        );
        assert!(
            blocks
                .iter()
                .any(|block| matches!(block.media, Some(Media::Mermaid(_))))
        );
    }

    #[test]
    fn external_oversized_and_invalid_images_remain_alt_text() {
        let source = format!(
            "![remote](https://example.com/a.svg)\n\n![invalid](data:image/svg+xml;base64,nope)\n\n![huge](data:image/png;base64,{})",
            "A".repeat(MAX_EMBEDDED_IMAGE_BYTES * 4 / 3 + 16)
        );
        let blocks = blocks(&source);
        assert!(blocks.iter().all(|block| block.media.is_none()));
        assert_eq!(
            blocks.iter().map(Block::text).collect::<Vec<_>>(),
            ["remote", "invalid", "huge"]
        );
    }

    #[test]
    fn code_lists_links_and_long_answers_preserve_their_content() {
        let source = format!(
            "# Result\n\n**Important** [docs](https://example.com)\n\n1. First\n2. Second\n\n```sh\nprintf 'hello'\n```\n\n{}END",
            "paragraph\n\n".repeat(200)
        );
        let blocks = blocks(&source);
        assert_eq!(blocks[0].heading, Some(1));
        assert!(
            blocks
                .iter()
                .any(|block| block.code && block.text() == "printf 'hello'\n")
        );
        assert!(blocks.iter().any(|block| block.text() == "2. Second"));
        assert_eq!(blocks.last().unwrap().text(), "END");
        assert!(
            blocks
                .iter()
                .flat_map(|block| &block.spans)
                .any(|span| span.link.as_deref() == Some("https://example.com"))
        );
    }

    #[test]
    fn cached_blocks_are_reused_for_a_retained_reply() {
        let first = blocks_cached("A **retained** reply.");
        let second = blocks_cached("A **retained** reply.");
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn fenced_language_quotes_tables_and_tasks_keep_structure() {
        let blocks = blocks(
            "> quoted\n\n- [x] done\n\n| A | B |\n| - | - |\n| 1 | 2 |\n\n```rust\nlet answer = true;\n```",
        );
        assert!(
            blocks
                .iter()
                .any(|block| block.quote && block.text() == "quoted")
        );
        assert!(blocks.iter().any(|block| block.text().contains("☑ done")));
        assert!(blocks.iter().any(|block| block.text().contains("A  B")));
        assert!(blocks.iter().any(|block| {
            block.code
                && block.language.as_deref() == Some("rust")
                && block.text().contains("let answer")
        }));
    }
}
