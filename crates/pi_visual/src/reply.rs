//! Markdown-aware gating. Only top-level, explicitly closed reserved fences run.
use crate::MAX_BUNDLE;
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag};

#[derive(Debug)]
pub enum Part<'a> {
    Markdown(&'a str),
    App {
        source: &'a str,
        raw: &'a str,
        ordinal: usize,
    },
}

/// Immutable parsed offsets, so clients can cache fence parsing without borrowing
/// a live session or executing any app. All ranges come from parts().
type PartRange = (
    std::ops::Range<usize>,
    Option<(std::ops::Range<usize>, usize)>,
);
pub struct Parsed {
    source: std::sync::Arc<str>,
    ranges: Vec<PartRange>,
}
impl Parsed {
    pub fn new(source: &str) -> Self {
        let owned: std::sync::Arc<str> = source.into();
        let base = owned.as_ptr() as usize;
        let range = |s: &str| {
            let start = s.as_ptr() as usize - base;
            start..start + s.len()
        };
        let ranges = parts(&owned)
            .into_iter()
            .map(|part| match part {
                Part::Markdown(text) => (range(text), None),
                Part::App {
                    source,
                    raw,
                    ordinal,
                } => (range(raw), Some((range(source), ordinal))),
            })
            .collect();
        Self {
            source: owned,
            ranges,
        }
    }
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn has_app(&self) -> bool {
        self.ranges.iter().any(|(_, app)| app.is_some())
    }
    pub fn parts(&self) -> impl Iterator<Item = Part<'_>> {
        self.ranges.iter().map(|(raw, app)| match app {
            Some((body, ordinal)) => Part::App {
                source: &self.source[body.clone()],
                raw: &self.source[raw.clone()],
                ordinal: *ordinal,
            },
            None => Part::Markdown(&self.source[raw.clone()]),
        })
    }
}

fn fence(line: &str) -> Option<(u8, usize, &str)> {
    let line = line.trim_end_matches(['\r', '\n']);
    let spaces = line.bytes().take_while(|b| *b == b' ').count();
    if spaces > 3 {
        return None;
    }
    let line = &line[spaces..];
    let ch = *line.as_bytes().first()?;
    if !matches!(ch, b'`' | b'~') {
        return None;
    }
    let n = line.bytes().take_while(|b| *b == ch).count();
    (n >= 3).then(|| (ch, n, line[n..].trim()))
}

pub fn parts(source: &str) -> Vec<Part<'_>> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    let mut ordinal = 0;
    for (event, range) in Parser::new(source).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info)))
                if depth == 0 && matches!(info.as_ref(), "javascript pi-visual" | "pi-visual") =>
            {
                let raw = &source[range.clone()];
                let body = raw.find('\n').map(|n| n + 1);
                let trimmed = raw.trim_end_matches(['\n', '\r']);
                let closing = trimmed.rfind('\n').map(|n| n + 1);
                if let (Some(body), Some(closing), Some((ch, length, _))) =
                    (body, closing, fence(raw.lines().next().unwrap_or_default()))
                    && closing >= body
                    && fence(&trimmed[closing..]).is_some_and(|(end, count, rest)| {
                        end == ch && count >= length && rest.is_empty()
                    })
                    && closing - body <= MAX_BUNDLE
                {
                    if range.start > start {
                        result.push(Part::Markdown(&source[start..range.start]));
                    }
                    result.push(Part::App {
                        source: &raw[body..closing],
                        raw,
                        ordinal,
                    });
                    ordinal += 1; // One ordinal vocabulary: only emitted candidates count.
                    start = range.end;
                }
                depth += 1;
            }
            Event::Start(_) => depth += 1,
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    if start < source.len() {
        result.push(Part::Markdown(&source[start..]));
    }
    result
}

pub fn has_app(source: &str) -> bool {
    parts(source)
        .iter()
        .any(|part| matches!(part, Part::App { .. }))
}

/// Bare tokens only in prose, never inline/fenced code, HTML, quotes or links.
pub fn has_directive(source: &str, token: &str) -> bool {
    let mut excluded = Vec::new();
    let mut inline_html = Vec::<String>::new();
    for (event, range) in Parser::new(source).into_offset_iter() {
        match event {
            Event::Start(tag) => excluded.push(matches!(
                tag,
                Tag::CodeBlock(_)
                    | Tag::BlockQuote(_)
                    | Tag::HtmlBlock
                    | Tag::Link { .. }
                    | Tag::Image { .. }
            )),
            Event::End(_) => {
                excluded.pop();
            }
            Event::InlineHtml(html) => {
                let tag = html.trim();
                if let Some(rest) = tag.strip_prefix("</") {
                    let name = rest
                        .split(|c: char| !c.is_ascii_alphanumeric())
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if let Some(index) = inline_html.iter().rposition(|t| t == &name) {
                        inline_html.truncate(index);
                    }
                } else if let Some(rest) = tag.strip_prefix('<') {
                    let name = rest
                        .split(|c: char| !c.is_ascii_alphanumeric())
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if !name.is_empty()
                        && !tag.ends_with("/>")
                        && !matches!(
                            name.as_str(),
                            "br" | "hr"
                                | "img"
                                | "input"
                                | "meta"
                                | "link"
                                | "wbr"
                                | "area"
                                | "base"
                                | "embed"
                                | "param"
                                | "source"
                                | "track"
                                | "col"
                        )
                    {
                        inline_html.push(name);
                    }
                }
            }
            Event::Text(_) if inline_html.is_empty() && !excluded.iter().any(|v| *v) => {
                let raw = &source[range.clone()];
                for (local, _) in raw.char_indices() {
                    let offset = range.start + local;
                    if offset > 0
                        && !source[..offset]
                            .chars()
                            .next_back()
                            .is_some_and(char::is_whitespace)
                    {
                        continue;
                    }
                    if source[offset..]
                        .get(..token.len())
                        .is_some_and(|s| s.eq_ignore_ascii_case(token))
                    {
                        let rest = &source[offset + token.len()..];
                        if rest.is_empty()
                            || rest
                                .chars()
                                .next()
                                .is_some_and(|c| c.is_whitespace() || matches!(c, ',' | '!' | '?'))
                        {
                            return true;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_a_real_matching_closing_fence() {
        for text in [
            "```pi-visual\n{}",
            "```pi-visual\n{}\n~~~\n",
            "````pi-visual\n{}\n```\n",
        ] {
            assert!(!has_app(text));
        }
        let text = "Before\n\n```pi-visual\n{}\n```\n\nAfter";
        assert!(matches!(
            parts(text).as_slice(),
            [
                Part::Markdown(_),
                Part::App {
                    source: "{}\n",
                    ordinal: 0,
                    ..
                },
                Part::Markdown(_)
            ]
        ));
        assert!(has_app("~~~pi-visual\n{}\n~~~~\n"));
    }
    #[test]
    fn ignores_containers_html_and_nested_code() {
        for text in [
            "    ```pi-visual\n    {}\n    ```",
            "> ```pi-visual\n> {}\n> ```",
            "- item\n\n  ```pi-visual\n  {}\n  ```",
            "<pre>\n```pi-visual\n{}\n```\n</pre>",
            "````text\n```pi-visual\n{}\n```\n````",
        ] {
            assert!(!has_app(text), "{text}");
        }
        assert!(!has_app(&format!(
            "```pi-visual\n{}\n```",
            " ".repeat(MAX_BUNDLE + 1)
        )));
    }
    #[test]
    fn native_javascript_requires_reserved_info_and_closed_top_level_fence() {
        assert!(has_app("```javascript pi-visual\napp({});\n```"));
        for source in [
            "```javascript\napp({});\n```",
            "```javascript pi-visual\napp({});",
            "> ```javascript pi-visual\n> app({});\n> ```",
        ] {
            assert!(!has_app(source));
        }
        let source = "α prose\n\n```javascript pi-visual\napp({});\n```";
        let parsed = Parsed::new(source);
        assert!(parsed.has_app());
        assert_eq!(parsed.source(), source);
        assert!(matches!(
            parsed.parts().last(),
            Some(Part::App {
                source: "app({});\n",
                ..
            })
        ));
    }
    #[test]
    fn ordinal_only_counts_emitted_apps() {
        let a = "```pi-visual\n{}\n```\n";
        let b = format!(
            "```pi-visual\n{}\n```\n\n{a}more prose",
            " ".repeat(MAX_BUNDLE + 1)
        );
        assert!(matches!(parts(a)[0], Part::App { ordinal: 0, .. }));
        assert!(matches!(parts(&b)[1], Part::App { ordinal: 0, .. }));
    }
    #[test]
    fn directives_ignore_multiline_code_and_container_fences() {
        for text in [
            "`example\n@Visualize\nexample`",
            "> quote\n> ```text\n> @Visualize\n> ```",
            "- item\n\n  ```text\n  @Visualize\n  ```",
            "<pre>\n@Visualize\n</pre>",
            "Example HTML: <code> @Visualize </code>",
            "Example HTML: <span><b> @Visualize </b></span>",
            "[@Visualize](https://example.com)",
        ] {
            assert!(!has_directive(text, "@Visualize"), "{text}");
        }
        assert!(has_directive(
            "Please @Visualize a calculator",
            "@Visualize"
        ));
    }
}
