//! SVG written into a reply as it is, without base64: a ```` ```svg ```` block,
//! `<svg>…</svg>` markup on its own, or a `data:image/svg+xml,<svg…>` image.
//! [`embed_svg`] turns each into the base64 image both apps already draw, so
//! a reply needs no encoding from Pi. An SVG that doesn't parse stays as it
//! was, and is shown as code or text.

use base64::Engine as _;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::{borrow::Cow, ops::Range};

const MAX_SVG_BYTES: usize = 2 * 1024 * 1024;

/// `source` with its raw SVG embedded as base64 images; unchanged, and not
/// copied, when it has none.
pub fn embed_svg(source: &str) -> Cow<'_, str> {
    if !source.contains("<svg") && !source.contains("image/svg+xml") {
        return Cow::Borrowed(source);
    }
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut code: Option<(Range<usize>, String)> = None;
    // A block of markup, or a paragraph: CommonMark doesn't list `svg` as a
    // block tag, so `<svg …>` sharing its line with more is a paragraph.
    let mut markup: Option<Range<usize>> = None;
    let options = Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info)))
                if info.split_whitespace().next() == Some("svg") =>
            {
                code = Some((range, String::new()));
            }
            Event::Start(Tag::HtmlBlock | Tag::Paragraph) => markup = Some(range),
            Event::Text(text) if code.is_some() => code.as_mut().unwrap().1.push_str(&text),
            Event::End(TagEnd::CodeBlock) => {
                if let Some((range, svg)) = code.take()
                    && let Some(image) = image_markdown(&svg)
                {
                    edits.push((range, image));
                }
            }
            Event::End(TagEnd::HtmlBlock | TagEnd::Paragraph) => {
                if let Some(range) = markup.take() {
                    let text = source[range.clone()].trim();
                    if text.starts_with("<svg")
                        && text.ends_with("</svg>")
                        && let Some(image) = image_markdown(text)
                    {
                        edits.push((range, image));
                    }
                }
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                if let Some(svg) = plain_data_url(&dest_url)
                    && let Some(encoded) = encode(&svg)
                    && let Some(at) = source[range.clone()].find(dest_url.as_ref())
                {
                    let start = range.start + at;
                    edits.push((start..start + dest_url.len(), encoded));
                }
            }
            _ => {}
        }
    }
    if edits.is_empty() {
        return Cow::Borrowed(source);
    }
    let mut result = String::with_capacity(source.len());
    let mut at = 0;
    for (range, replacement) in edits {
        result.push_str(&source[at..range.start]);
        result.push_str(&replacement);
        // A block's range takes its line break; keep it, so the next block
        // still starts on a line of its own.
        if source[range.clone()].ends_with('\n') && !replacement.ends_with('\n') {
            result.push('\n');
        }
        at = range.end;
    }
    result.push_str(&source[at..]);
    Cow::Owned(result)
}

/// An image line for a whole SVG block.
fn image_markdown(svg: &str) -> Option<String> {
    Some(format!("![SVG image]({})", encode(svg)?))
}

/// `svg` as a base64 `data:` URL, when it is an SVG gpui can draw.
fn encode(svg: &str) -> Option<String> {
    let svg = svg.trim();
    if !svg.starts_with("<svg") && !svg.starts_with("<?xml") || svg.len() > MAX_SVG_BYTES {
        return None;
    }
    let svg = with_namespaces(svg);
    usvg::Tree::from_str(&svg, &usvg::Options::default()).ok()?;
    Some(format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(svg.as_bytes())
    ))
}

/// Markup in a web page may leave out `xmlns`, which an SVG file needs.
fn with_namespaces(svg: &str) -> Cow<'_, str> {
    let Some(open) = svg.find("<svg") else {
        return Cow::Borrowed(svg);
    };
    let Some(close) = svg[open..].find('>').map(|end| open + end) else {
        return Cow::Borrowed(svg);
    };
    let tag = &svg[open..close];
    let mut added = String::new();
    if !tag.contains("xmlns=") {
        added.push_str(r#" xmlns="http://www.w3.org/2000/svg""#);
    }
    if svg.contains("xlink:") && !tag.contains("xmlns:xlink") {
        added.push_str(r#" xmlns:xlink="http://www.w3.org/1999/xlink""#);
    }
    if added.is_empty() {
        return Cow::Borrowed(svg);
    }
    let at = open + "<svg".len();
    Cow::Owned(format!("{}{added}{}", &svg[..at], &svg[at..]))
}

/// The SVG in a `data:image/svg+xml` URL that isn't base64, percent-decoded.
fn plain_data_url(url: &str) -> Option<String> {
    let (metadata, data) = url.strip_prefix("data:")?.split_once(',')?;
    let mut fields = metadata.split(';');
    if !fields.next()?.eq_ignore_ascii_case("image/svg+xml")
        || fields.any(|field| field.eq_ignore_ascii_case("base64"))
    {
        return None;
    }
    let bytes = data.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
                decoded.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            byte => {
                decoded.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Media, blocks};

    const SVG: &str = r##"<svg width="40" height="20" viewBox="0 0 40 20"><rect width="40" height="20" fill="#80bf95"/></svg>"##;

    fn svg_images(source: &str) -> usize {
        blocks(source)
            .iter()
            .filter(|block| {
                matches!(&block.media, Some(Media::Image(image)) if image.image.format == gpui::ImageFormat::Svg)
            })
            .count()
    }

    #[test]
    fn raw_svg_in_any_of_its_forms_becomes_a_picture() {
        let fenced = format!("Before.\n\n```svg\n{SVG}\n```\n\nAfter.");
        let markup = format!("Before.\n\n{SVG}\n\nAfter.");
        let url = format!(
            "![dot](data:image/svg+xml;utf8,{})",
            SVG.replace('"', "%22")
                .replace('#', "%23")
                .replace(' ', "%20")
        );
        for source in [&fenced, &markup, &url] {
            let embedded = embed_svg(source);
            assert!(embedded.contains("data:image/svg+xml;base64,"), "{source}");
            assert_eq!(svg_images(source), 1, "{source}");
        }
        // The words around a block stay where they were.
        let embedded = embed_svg(&fenced);
        assert!(embedded.starts_with("Before.\n\n![SVG image](data:"));
        assert!(embedded.ends_with(")\n\nAfter."));
    }

    #[test]
    fn other_text_and_broken_svg_stay_as_they_are() {
        let plain = "No pictures here, only `<svg>` in code.";
        assert!(matches!(embed_svg(plain), Cow::Borrowed(_)));
        // Unclosed, as while a reply streams in: shown as code until it closes.
        let streaming = "```svg\n<svg width=\"4\" height=\"4\"><rect";
        assert!(matches!(embed_svg(streaming), Cow::Borrowed(_)));
        let broken = "```svg\n<svg><rect></svg>\n```";
        assert!(matches!(embed_svg(broken), Cow::Borrowed(_)));
        // SVG in an HTML code block is code someone wants to read.
        let html = format!("```html\n{SVG}\n```");
        assert!(matches!(embed_svg(&html), Cow::Borrowed(_)));
        assert_eq!(svg_images(&html), 0);
    }

    #[test]
    fn markup_without_a_namespace_gets_one() {
        assert!(with_namespaces(SVG).contains(r#"<svg xmlns="http://www.w3.org/2000/svg" width"#));
        let declared = r#"<svg xmlns="http://www.w3.org/2000/svg"></svg>"#;
        assert!(matches!(with_namespaces(declared), Cow::Borrowed(_)));
        assert!(with_namespaces(r##"<svg><use xlink:href="#a"/></svg>"##).contains("xmlns:xlink="));
    }
}
