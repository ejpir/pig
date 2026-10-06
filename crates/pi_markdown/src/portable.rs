use gpui::{HighlightStyle, Hsla};
use std::{collections::HashMap, ops::Range, sync::Mutex};
use streaming_iterator::StreamingIterator as _;
use tree_sitter::{Language, Parser, Query, QueryCursor};

#[derive(Clone, Copy)]
pub struct SyntaxPalette {
    pub text: Hsla,
    pub plain: Hsla,
    pub muted: Hsla,
    pub faint: Hsla,
    pub accent: Hsla,
    pub steel: Hsla,
    pub amber: Hsla,
    pub coral: Hsla,
    pub green: Hsla,
    pub keyword: Hsla,
    pub string: Hsla,
    pub added: Hsla,
    pub removed: Hsla,
    pub selected: Hsla,
}

struct Highlighter {
    language: Language,
    query: Query,
}

static HIGHLIGHTERS: std::sync::OnceLock<Mutex<HashMap<&'static str, Option<Highlighter>>>> =
    std::sync::OnceLock::new();

/// Highlight fenced code with the same native grammars and `highlights.scm`
/// queries used by Zed. Unsupported language labels simply return no ranges.
pub fn highlight(
    source: &str,
    language: &str,
    palette: SyntaxPalette,
) -> Vec<(Range<usize>, HighlightStyle)> {
    // Bound synchronous work during streaming and for generated files. The
    // caller still renders all source text, just without colors past this size.
    if source.is_empty() || source.len() > 128 * 1024 {
        return Vec::new();
    }
    let Some((grammar_name, query_name)) = language_names(language) else {
        return Vec::new();
    };
    let highlighters = HIGHLIGHTERS.get_or_init(Default::default);
    let mut highlighters = highlighters
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let highlighter = highlighters
        .entry(query_name)
        .or_insert_with(|| build_highlighter(grammar_name, query_name));
    let Some(highlighter) = highlighter.as_ref() else {
        return Vec::new();
    };

    let mut parser = Parser::new();
    if parser.set_language(&highlighter.language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let capture_names = highlighter.query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut captures = cursor.captures(&highlighter.query, tree.root_node(), source.as_bytes());
    let mut byte_colors = vec![None; source.len()];
    while let Some((query_match, capture_index)) = captures.next() {
        let capture = query_match.captures[*capture_index];
        let Some(color) = capture_color(capture_names[capture.index as usize], palette) else {
            continue;
        };
        for byte in &mut byte_colors[capture.node.byte_range()] {
            *byte = Some(color);
        }
    }
    drop(captures);
    drop(highlighters);

    let mut result = Vec::new();
    let mut start = 0;
    while start < byte_colors.len() {
        let Some(color) = byte_colors[start] else {
            start += 1;
            continue;
        };
        let mut end = start + 1;
        while end < byte_colors.len() && byte_colors[end] == Some(color) {
            end += 1;
        }
        result.push((start..end, HighlightStyle::color(color)));
        start = end;
    }
    result
}

fn build_highlighter(grammar_name: &str, query_name: &'static str) -> Option<Highlighter> {
    let language = grammars::native_grammars()
        .into_iter()
        .find_map(|(name, language)| (name == grammar_name).then_some(language))?;
    let query_source = grammars::load_queries(query_name).highlights?;
    let query = Query::new(&language, &query_source).ok()?;
    Some(Highlighter { language, query })
}

fn language_names(label: &str) -> Option<(&'static str, &'static str)> {
    Some(match label.trim().to_ascii_lowercase().as_str() {
        "bash" | "sh" | "shell" | "zsh" => ("bash", "bash"),
        "c" | "h" => ("c", "c"),
        "cpp" | "c++" | "cc" | "cxx" | "hpp" => ("cpp", "cpp"),
        "css" => ("css", "css"),
        "diff" | "patch" => ("diff", "diff"),
        "go" | "golang" => ("go", "go"),
        "gomod" | "go.mod" => ("gomod", "gomod"),
        "gowork" | "go.work" => ("gowork", "gowork"),
        "javascript" | "js" | "jsx" | "mjs" | "cjs" | "node" => ("tsx", "javascript"),
        "json" => ("json", "json"),
        "jsonc" => ("json", "jsonc"),
        "python" | "py" => ("python", "python"),
        "regex" | "regexp" => ("regex", "regex"),
        "rust" | "rs" => ("rust", "rust"),
        "tsx" => ("tsx", "tsx"),
        "typescript" | "ts" => ("typescript", "typescript"),
        "yaml" | "yml" => ("yaml", "yaml"),
        _ => return None,
    })
}

fn capture_color(name: &str, palette: SyntaxPalette) -> Option<Hsla> {
    let root = name.split('.').next().unwrap_or(name);
    Some(match root {
        "comment" => palette.faint,
        "string" => palette.string,
        "keyword" | "tag" | "boolean" | "variable.special" => palette.keyword,
        "function" => palette.accent,
        "type" | "constructor" => palette.steel,
        "number" | "constant" | "attribute" | "label" => palette.amber,
        "operator" | "punctuation" => palette.muted,
        "property" | "variable" | "embedded" => palette.plain,
        _ => return None,
    })
}

/// Wrap arbitrary tool output in a Markdown fence while preserving an exact
/// copy range. The fence is always longer than any backtick run in the source.
pub fn code_source(text: &str, language: &str) -> (String, Range<usize>) {
    let longest = text.split(|ch| ch != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    let language: String = language
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        .take(32)
        .collect();
    let prefix = format!("{fence}{language}\n");
    let range = prefix.len()..prefix.len() + text.len();
    (format!("{prefix}{text}\n{fence}"), range)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> SyntaxPalette {
        let color = |rgb: u32| gpui::rgb(rgb).into();
        SyntaxPalette {
            text: color(0xffffff),
            plain: color(0xeeeeee),
            muted: color(0x888888),
            faint: color(0x777777),
            accent: color(0x00aaff),
            steel: color(0x5599aa),
            amber: color(0xffaa00),
            coral: color(0xff6655),
            green: color(0x55cc77),
            keyword: color(0xaa77ff),
            string: color(0xddaa55),
            added: color(0x003300),
            removed: color(0x330000),
            selected: color(0x333333),
        }
    }

    #[test]
    fn native_queries_highlight_common_languages() {
        for (language, source) in [
            ("rust", "pub fn answer() -> bool { true }"),
            ("typescript", "const answer: boolean = true;"),
            ("python", "def answer():\n    return True"),
            ("bash", "if true; then echo ok; fi"),
        ] {
            assert!(
                !highlight(source, language, palette()).is_empty(),
                "{language}"
            );
        }
    }

    #[test]
    fn unknown_languages_and_huge_sources_are_safe() {
        assert!(highlight("hello", "made-up", palette()).is_empty());
        assert!(highlight(&"x".repeat(128 * 1024 + 1), "rust", palette()).is_empty());
    }

    #[test]
    fn code_fences_preserve_exact_source() {
        let source = "  π🐈\n```text\nbody\n```\n";
        let (wrapped, range) = code_source(source, "text\n`bad");
        assert!(wrapped.starts_with("````textbad\n"));
        assert_eq!(&wrapped[range], source);
    }
}
