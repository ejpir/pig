use gpui::{HighlightStyle, Hsla};
use std::{cmp::Reverse, collections::HashMap, ops::Range, sync::Mutex};
use streaming_iterator::StreamingIterator as _;
use tree_sitter::{InputEdit, Language, Node, Parser, Point, Query, QueryCursor, Tree};

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

/// The largest source that Pi parses synchronously for syntax services.
///
/// A [`SyntaxDocument`] still remembers larger source so that its state never
/// becomes stale, but drops its tree and returns no syntax-derived results.
/// Returning below this limit causes the next update to do a fresh parse.
pub const MAX_SYNTAX_DOCUMENT_BYTES: usize = 128 * 1024;

/// The changed byte ranges in the source before and after one text edit.
///
/// Both ranges use UTF-8 byte offsets. Their starts must be equal, all four
/// bounds must be character boundaries, and text outside the two ranges must
/// be unchanged. If an editor supplies an invalid descriptor,
/// [`SyntaxDocument::update_source`] safely falls back to a full parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxEdit {
    pub old_range: Range<usize>,
    pub new_range: Range<usize>,
}

impl SyntaxEdit {
    pub fn new(old_range: Range<usize>, new_range: Range<usize>) -> Self {
        Self {
            old_range,
            new_range,
        }
    }

    fn to_input_edit(&self, old_source: &str, new_source: &str) -> Option<InputEdit> {
        let old = &self.old_range;
        let new = &self.new_range;
        if old.start > old.end
            || new.start > new.end
            || old.end > old_source.len()
            || new.end > new_source.len()
            || old.start != new.start
            || !old_source.is_char_boundary(old.start)
            || !old_source.is_char_boundary(old.end)
            || !new_source.is_char_boundary(new.start)
            || !new_source.is_char_boundary(new.end)
            || old_source.get(..old.start)? != new_source.get(..new.start)?
            || old_source.get(old.end..)? != new_source.get(new.end..)?
        {
            return None;
        }

        Some(InputEdit {
            start_byte: old.start,
            old_end_byte: old.end,
            new_end_byte: new.end,
            start_position: point_at(old_source, old.start),
            old_end_position: point_at(old_source, old.end),
            new_end_position: point_at(new_source, new.end),
        })
    }
}

/// One symbol produced by the bundled Zed `outline.scm` query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxOutlineItem {
    /// Display text assembled from the query's `@context` and `@name` captures.
    pub label: String,
    /// The Tree-sitter kind of the query's `@item` node.
    pub kind: String,
    /// Full byte range of the item.
    pub range: Range<usize>,
    /// Byte range spanning its `@name` captures.
    pub selection_range: Range<usize>,
    /// Nesting depth among outline items (top-level is zero).
    pub depth: usize,
}

/// A named Tree-sitter node in a syntax breadcrumb.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxAncestor {
    pub label: String,
    pub kind: String,
    pub range: Range<usize>,
}

/// A retained, incrementally updated Tree-sitter document.
///
/// The parser, tree, and compiled Zed highlight/outline queries are all kept
/// between edits. All public positions and ranges are UTF-8 byte offsets.
pub struct SyntaxDocument {
    parser: Parser,
    tree: Option<Tree>,
    source: String,
    // Keep the language alive alongside its compiled queries. Native grammar
    // languages are static today, but this also makes ownership explicit.
    _language: Language,
    grammar_name: &'static str,
    query_name: &'static str,
    highlight_query: Option<Query>,
    outline_query: Option<Query>,
}

impl SyntaxDocument {
    /// Creates an empty document for a language label, or `None` when the label
    /// is not one of the native grammars bundled for Android and desktop.
    pub fn new(language: &str) -> Option<Self> {
        let (grammar_name, query_name) = language_names(language)?;
        let grammar = native_language(grammar_name)?;
        let queries = grammars::load_queries(query_name);
        let highlight_query = queries
            .highlights
            .as_deref()
            .and_then(|source| Query::new(&grammar, source).ok());
        let outline_query = queries
            .outline
            .as_deref()
            .and_then(|source| Query::new(&grammar, source).ok())
            .filter(|query| {
                query.capture_index_for_name("item").is_some()
                    && query.capture_index_for_name("name").is_some()
            });

        let mut parser = Parser::new();
        parser.set_language(&grammar).ok()?;
        let tree = parser.parse("", None);
        Some(Self {
            parser,
            tree,
            source: String::new(),
            _language: grammar,
            grammar_name,
            query_name,
            highlight_query,
            outline_query,
        })
    }

    /// Creates a document and parses its initial source.
    pub fn with_source(language: &str, source: impl Into<String>) -> Option<Self> {
        let mut document = Self::new(language)?;
        document.set_source(source);
        Some(document)
    }

    /// Alias for [`SyntaxDocument::with_source`].
    pub fn from_source(language: &str, source: impl Into<String>) -> Option<Self> {
        Self::with_source(language, source)
    }

    /// The canonical native grammar name (for example, `tsx` for `jsx`).
    pub fn grammar_name(&self) -> &str {
        self.grammar_name
    }

    /// The canonical Zed query-set name.
    pub fn language_name(&self) -> &str {
        self.query_name
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn is_parsed(&self) -> bool {
        self.tree.is_some()
    }

    /// Replaces all source and performs a fresh parse.
    ///
    /// Returns whether a syntax tree is available. Sources over
    /// [`MAX_SYNTAX_DOCUMENT_BYTES`] are retained but deliberately not parsed.
    pub fn set_source(&mut self, source: impl Into<String>) -> bool {
        self.source = source.into();
        if self.source.len() > MAX_SYNTAX_DOCUMENT_BYTES {
            self.tree = None;
            return false;
        }
        self.parser.reset();
        self.tree = self.parser.parse(&self.source, None);
        self.tree.is_some()
    }

    /// Replaces the source, incrementally reparsing from `edit` when valid.
    ///
    /// Invalid descriptors and documents that currently have no tree use a
    /// fresh parse, so callers cannot leave this object with a stale tree.
    /// Returns whether a syntax tree is available.
    pub fn update_source(&mut self, new_source: impl Into<String>, edit: SyntaxEdit) -> bool {
        let new_source = new_source.into();
        if new_source.len() > MAX_SYNTAX_DOCUMENT_BYTES {
            self.source = new_source;
            self.tree = None;
            return false;
        }

        // Do not even scan an unparsed (and potentially very large) old
        // source to calculate points; no old tree exists to edit in that case.
        let input_edit = self
            .tree
            .as_ref()
            .and_then(|_| edit.to_input_edit(&self.source, &new_source));
        let parsed = match (self.tree.take(), input_edit) {
            (Some(mut old_tree), Some(input_edit)) => {
                old_tree.edit(&input_edit);
                self.parser.parse(&new_source, Some(&old_tree))
            }
            _ => {
                self.parser.reset();
                self.parser.parse(&new_source, None)
            }
        };
        self.source = new_source;
        self.tree = parsed;
        self.tree.is_some()
    }

    /// Short alias for [`SyntaxDocument::update_source`].
    pub fn update(&mut self, new_source: impl Into<String>, edit: SyntaxEdit) -> bool {
        self.update_source(new_source, edit)
    }

    /// Edit-first form of [`SyntaxDocument::update_source`].
    pub fn apply_update(&mut self, edit: SyntaxEdit, new_source: impl Into<String>) -> bool {
        self.update_source(new_source, edit)
    }

    /// Returns non-overlapping byte ranges and styles for the current tree.
    pub fn highlights(&self, palette: SyntaxPalette) -> Vec<(Range<usize>, HighlightStyle)> {
        let (Some(tree), Some(query)) = (&self.tree, &self.highlight_query) else {
            return Vec::new();
        };
        highlight_tree(&self.source, tree, query, palette)
    }

    /// Alias emphasizing that all returned highlight positions are byte ranges.
    pub fn highlight_ranges(&self, palette: SyntaxPalette) -> Vec<(Range<usize>, HighlightStyle)> {
        self.highlights(palette)
    }

    /// Runs the language's bundled Zed outline query against the retained tree.
    pub fn outline(&self) -> Vec<SyntaxOutlineItem> {
        let (Some(tree), Some(query)) = (&self.tree, &self.outline_query) else {
            return Vec::new();
        };
        outline_items(&self.source, tree, query)
    }

    /// Named syntax nodes at `byte_offset`, ordered innermost to outermost.
    pub fn named_ancestors(&self, byte_offset: usize) -> Vec<SyntaxAncestor> {
        let Some(mut node) = self.named_node_at(byte_offset) else {
            return Vec::new();
        };
        let mut result = Vec::new();
        loop {
            if node.is_named() {
                result.push(SyntaxAncestor {
                    label: node_label(node, &self.source),
                    kind: node.kind().to_owned(),
                    range: node.byte_range(),
                });
            }
            let Some(parent) = node.parent() else {
                break;
            };
            node = parent;
        }
        result
    }

    /// Byte ranges from [`SyntaxDocument::named_ancestors`], innermost first.
    pub fn ancestor_ranges(&self, byte_offset: usize) -> Vec<Range<usize>> {
        self.named_ancestors(byte_offset)
            .into_iter()
            .map(|ancestor| ancestor.range)
            .collect()
    }

    /// Labels from [`SyntaxDocument::named_ancestors`], innermost first.
    pub fn ancestor_labels(&self, byte_offset: usize) -> Vec<String> {
        self.named_ancestors(byte_offset)
            .into_iter()
            .map(|ancestor| ancestor.label)
            .collect()
    }

    /// Named syntax nodes suitable for UI breadcrumbs, outermost to innermost.
    pub fn breadcrumbs(&self, byte_offset: usize) -> Vec<SyntaxAncestor> {
        let mut result = self.named_ancestors(byte_offset);
        result.reverse();
        result
    }

    /// Singular alias for [`SyntaxDocument::breadcrumbs`].
    pub fn breadcrumb(&self, byte_offset: usize) -> Vec<SyntaxAncestor> {
        self.breadcrumbs(byte_offset)
    }

    /// Returns the smallest named syntax range at a caret byte offset.
    ///
    /// A caret in root-level trivia has downstream affinity: the nearest named
    /// node to its right is preferred, with the nearest node to its left used
    /// at EOF. Empty and whitespace-only documents have no structural range.
    pub fn structural_selection(&self, byte_offset: usize) -> Option<Range<usize>> {
        self.expand_selection(byte_offset..byte_offset)
    }

    /// Expands a byte selection to the next containing named syntax node.
    ///
    /// A collapsed selection first chooses the caret's smallest meaningful
    /// node, including the trivia affinity described by
    /// [`SyntaxDocument::structural_selection`]. If a nonempty `range` already
    /// exactly selects a node, its nearest larger named parent is returned.
    /// Repeated calls therefore grow a structural selection up to the root and
    /// then return `None`.
    pub fn expand_selection(&self, range: Range<usize>) -> Option<Range<usize>> {
        if range.start > range.end
            || range.end > self.source.len()
            || !self.source.is_char_boundary(range.start)
            || !self.source.is_char_boundary(range.end)
        {
            return None;
        }
        if range.is_empty() {
            let mut node = self.named_node_at(range.start)?;
            loop {
                let node_range = node.byte_range();
                if node.is_named() && !node_range.is_empty() {
                    return Some(node_range);
                }
                node = node.parent()?;
            }
        }

        let tree = self.tree.as_ref()?;
        let mut node = tree
            .root_node()
            .named_descendant_for_byte_range(range.start, range.end.saturating_sub(1))?;
        loop {
            let node_range = node.byte_range();
            if node.is_named()
                && node_range.start <= range.start
                && node_range.end >= range.end
                && node_range != range
            {
                return Some(node_range);
            }
            node = node.parent()?;
        }
    }

    /// Finds a named node at a caret. Tree-sitter returns the parse root for a
    /// zero-width query in top-level whitespace and at EOF. In that case walk
    /// visible source bytes downstream, then upstream, until a real descendant
    /// is found instead of treating Expand as Select all.
    fn named_node_at(&self, byte_offset: usize) -> Option<Node<'_>> {
        if byte_offset > self.source.len() || !self.source.is_char_boundary(byte_offset) {
            return None;
        }
        let root = self.tree.as_ref()?.root_node();
        let direct = root.named_descendant_for_byte_range(byte_offset, byte_offset)?;
        if direct.id() != root.id() {
            return Some(direct);
        }

        let descendant_at = |start: usize, character: char| {
            root.named_descendant_for_byte_range(
                start,
                start + character.len_utf8().saturating_sub(1),
            )
            .filter(|node| node.id() != root.id())
        };
        for (relative, character) in self.source[byte_offset..].char_indices() {
            if !character.is_whitespace()
                && let Some(node) = descendant_at(byte_offset + relative, character)
            {
                return Some(node);
            }
        }
        for (start, character) in self.source[..byte_offset].char_indices().rev() {
            if !character.is_whitespace()
                && let Some(node) = descendant_at(start, character)
            {
                return Some(node);
            }
        }

        self.source
            .chars()
            .any(|character| !character.is_whitespace())
            .then_some(direct)
    }
}

fn point_at(source: &str, byte_offset: usize) -> Point {
    let prefix = &source.as_bytes()[..byte_offset];
    let row = prefix.iter().filter(|byte| **byte == b'\n').count();
    let column = prefix
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(prefix.len(), |newline| prefix.len() - newline - 1);
    Point::new(row, column)
}

fn native_language(grammar_name: &str) -> Option<Language> {
    grammars::native_grammars()
        .into_iter()
        .find_map(|(name, language)| (name == grammar_name).then_some(language))
}

fn highlight_tree(
    source: &str,
    tree: &Tree,
    query: &Query,
    palette: SyntaxPalette,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let capture_names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut captures = cursor.captures(query, tree.root_node(), source.as_bytes());
    let mut byte_colors = vec![None; source.len()];
    while let Some((query_match, capture_index)) = captures.next() {
        let capture = query_match.captures[*capture_index];
        let Some(color) = capture_color(capture_names[capture.index as usize], palette) else {
            continue;
        };
        let Some(bytes) = byte_colors.get_mut(capture.node.byte_range()) else {
            continue;
        };
        for byte in bytes {
            *byte = Some(color);
        }
    }
    drop(captures);

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

fn outline_items(source: &str, tree: &Tree, query: &Query) -> Vec<SyntaxOutlineItem> {
    let Some(item_capture) = query.capture_index_for_name("item") else {
        return Vec::new();
    };
    let Some(name_capture) = query.capture_index_for_name("name") else {
        return Vec::new();
    };
    let context_capture = query.capture_index_for_name("context");
    let extra_context_capture = query.capture_index_for_name("context.extra");

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());
    let mut items = Vec::new();
    while let Some(query_match) = matches.next() {
        let Some(item_node) = query_match
            .captures
            .iter()
            .find(|capture| capture.index == item_capture)
            .map(|capture| capture.node)
        else {
            continue;
        };

        let mut label_ranges = Vec::new();
        let mut name_ranges = Vec::new();
        for capture in query_match.captures {
            if capture.index == name_capture {
                let range = first_line_range(source, capture.node.byte_range());
                if !range.is_empty() {
                    name_ranges.push(range.clone());
                    label_ranges.push(range);
                }
            } else if Some(capture.index) == context_capture
                || Some(capture.index) == extra_context_capture
            {
                let range = first_line_range(source, capture.node.byte_range());
                if !range.is_empty() {
                    label_ranges.push(range);
                }
            }
        }
        if name_ranges.is_empty() {
            continue;
        }
        label_ranges.sort_by_key(|range| (range.start, range.end));
        label_ranges.dedup();
        name_ranges.sort_by_key(|range| (range.start, range.end));

        let mut label = String::new();
        let mut previous_end: Option<usize> = None;
        for range in label_ranges {
            let append_start = previous_end.map_or(range.start, |end| end.max(range.start));
            if append_start >= range.end {
                continue;
            }
            if !label.is_empty() && previous_end.is_some_and(|end| end < range.start) {
                label.push(' ');
            }
            let Some(part) = source.get(append_start..range.end) else {
                continue;
            };
            label.push_str(part);
            previous_end = Some(previous_end.map_or(range.end, |end| end.max(range.end)));
        }
        let label = label.trim().to_owned();
        if label.is_empty() {
            continue;
        }
        let selection_range = name_ranges.first().unwrap().start..name_ranges.last().unwrap().end;
        items.push(SyntaxOutlineItem {
            label,
            kind: item_node.kind().to_owned(),
            range: item_node.byte_range(),
            selection_range,
            depth: 0,
        });
    }
    drop(matches);

    items.sort_by_key(|item| (item.range.start, Reverse(item.range.end)));
    items.dedup_by(|left, right| {
        left.range == right.range
            && left.selection_range == right.selection_range
            && left.label == right.label
    });

    let mut enclosing_ends = Vec::new();
    for item in &mut items {
        while enclosing_ends
            .last()
            .is_some_and(|end| *end < item.range.end)
        {
            enclosing_ends.pop();
        }
        item.depth = enclosing_ends.len();
        enclosing_ends.push(item.range.end);
    }
    items
}

fn first_line_range(source: &str, range: Range<usize>) -> Range<usize> {
    let Some(text) = source.get(range.clone()) else {
        return range.start..range.start;
    };
    let mut end = range.start + text.find('\n').unwrap_or(text.len());
    if end > range.start && source.as_bytes()[end - 1] == b'\r' {
        end -= 1;
    }
    range.start..end
}

fn node_label(node: Node<'_>, source: &str) -> String {
    if let Some(name) = node.child_by_field_name("name")
        && let Some(text) = source.get(name.byte_range())
    {
        let label = short_label(text);
        if !label.is_empty() {
            return label;
        }
    }
    if node.named_child_count() == 0
        && let Some(text) = source.get(node.byte_range())
    {
        let label = short_label(text);
        if !label.is_empty() {
            return label;
        }
    }
    node.kind().replace('_', " ")
}

fn short_label(text: &str) -> String {
    const MAX_LABEL_CHARS: usize = 80;
    let line = text.split(['\r', '\n']).next().unwrap_or("").trim();
    let mut chars = line.chars();
    let mut label: String = chars.by_ref().take(MAX_LABEL_CHARS).collect();
    if chars.next().is_some() {
        label.push('…');
    }
    label
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
    if source.is_empty() || source.len() > MAX_SYNTAX_DOCUMENT_BYTES {
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
    let result = highlight_tree(source, &tree, &highlighter.query, palette);
    drop(highlighters);
    result
}

fn build_highlighter(grammar_name: &str, query_name: &'static str) -> Option<Highlighter> {
    let language = native_language(grammar_name)?;
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
        "jsdoc" => ("jsdoc", "jsdoc"),
        "json" => ("json", "json"),
        "jsonc" => ("jsonc", "jsonc"),
        "markdown" | "md" => ("markdown", "markdown"),
        "markdown-inline" => ("markdown-inline", "markdown-inline"),
        "python" | "py" => ("python", "python"),
        "regex" | "regexp" => ("regex", "regex"),
        "rust" | "rs" => ("rust", "rust"),
        "tsx" => ("tsx", "tsx"),
        "typescript" | "ts" => ("typescript", "typescript"),
        "yaml" | "yml" => ("yaml", "yaml"),
        "gitcommit" | "git-commit" => ("gitcommit", "gitcommit"),
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

    #[test]
    fn unicode_multiline_edit_reparses_incrementally() {
        let old_source = "fn café() {\n    let animal = \"🐈\";\n}\n";
        let old_text = "let animal = \"🐈\";";
        let new_text = "let animal = \"猫\";\n    let λ = \"🐕\";";
        let start = old_source.find(old_text).unwrap();
        let old_range = start..start + old_text.len();
        let new_range = start..start + new_text.len();
        let mut new_source = old_source.to_owned();
        new_source.replace_range(old_range.clone(), new_text);
        let edit = SyntaxEdit::new(old_range, new_range);

        let input_edit = edit.to_input_edit(old_source, &new_source).unwrap();
        assert_eq!(input_edit.start_position, Point::new(1, 4));
        assert_eq!(
            input_edit.old_end_position,
            Point::new(1, 4 + old_text.len())
        );
        assert_eq!(
            input_edit.new_end_position,
            Point::new(2, "    let λ = \"🐕\";".len())
        );

        let mut document = SyntaxDocument::with_source("rust", old_source).unwrap();
        assert!(document.update_source(new_source.clone(), edit));
        assert_eq!(document.source(), new_source);

        // Incremental and fresh parses must expose the same syntax structure.
        let fresh = SyntaxDocument::with_source("rust", new_source.clone()).unwrap();
        assert_eq!(document.outline(), fresh.outline());
        let lambda = new_source.find('λ').unwrap();
        assert_eq!(
            document
                .named_ancestors(lambda)
                .into_iter()
                .map(|ancestor| (ancestor.kind, ancestor.range))
                .collect::<Vec<_>>(),
            fresh
                .named_ancestors(lambda)
                .into_iter()
                .map(|ancestor| (ancestor.kind, ancestor.range))
                .collect::<Vec<_>>()
        );
        assert!(
            document
                .ancestor_labels(lambda)
                .iter()
                .any(|label| label == "café")
        );
        assert!(document.highlights(palette()).iter().all(|(range, _)| {
            new_source.is_char_boundary(range.start) && new_source.is_char_boundary(range.end)
        }));
    }

    #[test]
    fn zed_outline_query_produces_labels_kinds_and_depths() {
        let source = r#"pub struct Widget {
    value: i32,
}

impl Widget {
    pub fn run(&self) -> i32 {
        self.value
    }
}
"#;
        let document = SyntaxDocument::with_source("rs", source).unwrap();
        let outline = document.outline();

        let structure = outline
            .iter()
            .find(|item| item.kind == "struct_item")
            .unwrap();
        assert!(structure.label.contains("struct Widget"));
        assert_eq!(&source[structure.selection_range.clone()], "Widget");

        let implementation = outline
            .iter()
            .find(|item| item.kind == "impl_item")
            .unwrap();
        let function = outline
            .iter()
            .find(|item| item.kind == "function_item")
            .unwrap();
        assert!(implementation.label.contains("impl Widget"));
        assert!(function.label.contains("fn run"));
        assert!(implementation.range.start <= function.range.start);
        assert!(implementation.range.end >= function.range.end);
        assert!(function.depth > implementation.depth);
    }

    #[test]
    fn ancestors_breadcrumbs_and_structural_selections_are_named_nodes() {
        let source = "fn outer() {\n    fn inner() {\n        let value = 1;\n    }\n}\n";
        let document = SyntaxDocument::with_source("rust", source).unwrap();
        let value = source.find("value").unwrap()..source.find("value").unwrap() + 5;

        let ancestors = document.named_ancestors(value.start + 1);
        assert_eq!(ancestors.first().unwrap().kind, "identifier");
        assert_eq!(ancestors.first().unwrap().range, value);
        let labels = document.ancestor_labels(value.start + 1);
        assert!(labels.iter().any(|label| label == "inner"));
        assert!(labels.iter().any(|label| label == "outer"));

        let breadcrumbs = document.breadcrumbs(value.start + 1);
        assert_eq!(breadcrumbs.first().unwrap().kind, "source_file");
        assert_eq!(breadcrumbs.last().unwrap().label, "value");
        assert_eq!(
            document.structural_selection(value.start + 1),
            Some(value.clone())
        );

        let expanded = document.expand_selection(value.clone()).unwrap();
        assert!(expanded.start <= value.start && expanded.end >= value.end);
        assert_ne!(expanded, value);
        assert!(document.expand_selection(0..source.len()).is_none());
        assert!(document.named_ancestors(source.len() + 1).is_empty());
    }

    #[test]
    fn caret_in_trivia_expands_to_nearby_syntax_before_the_root() {
        let source = "fn first() {}\n\nfn last() {\n    let λ = 1;\n}\n\n";
        let document = SyntaxDocument::with_source("rust", source).unwrap();
        let last = source.find("fn last").unwrap();

        let eof = document.structural_selection(source.len()).unwrap();
        assert_ne!(eof, 0..source.len(), "EOF must not begin with Select all");
        assert!(eof.start >= last);
        assert!(eof.end <= source.trim_end().len());
        assert!(
            document
                .named_ancestors(source.len())
                .iter()
                .any(|ancestor| ancestor.kind == "function_item")
        );

        let between = source.find("\n\n").unwrap() + 1;
        let downstream = document.structural_selection(between).unwrap();
        assert!(
            downstream.start >= last,
            "blank lines prefer syntax to the right"
        );
        assert!(downstream.end <= source.trim_end().len());

        let lambda = source.find('λ').unwrap();
        assert!(document.structural_selection(lambda).is_some());
        assert!(
            document.structural_selection(lambda + 1).is_none(),
            "syntax offsets must remain UTF-8 boundaries"
        );

        let whitespace = SyntaxDocument::with_source("rust", " \n\t\n").unwrap();
        assert!(whitespace.structural_selection(2).is_none());
        assert!(whitespace.named_ancestors(2).is_empty());
    }

    #[test]
    fn repeated_caret_expansion_grows_to_the_root_then_stops() {
        let source = "fn first() {}\nfn last() {}\n";
        let document = SyntaxDocument::with_source("rust", source).unwrap();
        let mut selection = source.len()..source.len();
        let mut expansions = 0;
        while let Some(next) = document.expand_selection(selection.clone()) {
            if !selection.is_empty() {
                assert!(next.start <= selection.start && next.end >= selection.end);
                assert_ne!(next, selection);
            }
            selection = next;
            expansions += 1;
            assert!(expansions < 32, "structural expansion must terminate");
        }
        assert!(expansions >= 2);
        assert_eq!(selection, 0..source.len());
    }

    #[test]
    fn retained_documents_observe_the_synchronous_size_bound() {
        let mut document = SyntaxDocument::new("rust").unwrap();
        let huge = "x".repeat(MAX_SYNTAX_DOCUMENT_BYTES + 1);
        assert!(!document.set_source(huge.clone()));
        assert_eq!(document.source(), huge);
        assert!(!document.is_parsed());
        assert!(document.highlights(palette()).is_empty());
        assert!(document.outline().is_empty());

        assert!(document.set_source("fn recovered() {}"));
        assert!(document.is_parsed());
        assert!(!document.outline().is_empty());
        assert!(SyntaxDocument::new("made-up").is_none());
    }
}
