//! `@` mentions (design study 08): what a composer chip stands for, the text pi
//! receives for it, and how a sent message shows its chips again.
//!
//! pi gets plain text and nothing is hidden. Files, directories and symbols go as project paths
//! that pi reads with its own tools: `@path` is pi's convention, and its file tools
//! strip the `@`. Terminal output, problems, turns and sessions go as text: a
//! `[kind: title]` reference in the sentence and a `<mention>` block after the
//! message, which the transcript folds back into the chip.
use crate::input::Chip;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    File,
    Directory,
    Symbol,
    Turn,
    Session,
    Terminal,
    Problems,
}

impl Kind {
    /// Menu order.
    pub const ALL: [Self; 7] = [
        Self::File,
        Self::Directory,
        Self::Symbol,
        Self::Turn,
        Self::Session,
        Self::Terminal,
        Self::Problems,
    ];

    pub fn icon(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "folder",
            Self::Symbol => "list_tree",
            Self::Turn => "undo",
            Self::Session => "chat",
            Self::Terminal => "terminal",
            Self::Problems => "warning",
        }
    }

    pub fn group(self) -> &'static str {
        match self {
            Self::File => "Files",
            Self::Directory => "Directories",
            Self::Symbol => "Symbols",
            Self::Turn => "Turns",
            Self::Session => "Sessions",
            Self::Terminal => "Terminal",
            Self::Problems => "Problems",
        }
    }

    fn word(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Symbol => "symbol",
            Self::Turn => "turn",
            Self::Session => "session",
            Self::Terminal => "terminal",
            Self::Problems => "problems",
        }
    }

    fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.word() == word)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    /// pi reads it: a project-relative path, and a line for a symbol.
    Path { path: String, line: Option<u32> },
    /// Inlined as text.
    Text(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mention {
    pub kind: Kind,
    /// The chip's text.
    pub label: String,
    pub body: Body,
}

impl Mention {
    /// How the mention reads inside the sentence pi receives.
    pub fn inline(&self) -> String {
        match &self.body {
            // pi's `@path`; a path with spaces is quoted so it stays one path.
            Body::Path { path, line: None } if path.contains(char::is_whitespace) => {
                format!("`{path}`")
            }
            Body::Path { path, line: None } => format!("@{path}"),
            Body::Path {
                path,
                line: Some(line),
            } => format!("{} (@{path}:{line})", self.label),
            Body::Text(_) => format!("[{}: {}]", self.kind.word(), self.label),
        }
    }

    /// The text appended after the message, for a text mention.
    pub fn block(&self) -> Option<String> {
        let Body::Text(text) = &self.body else {
            return None;
        };
        Some(format!(
            "<mention kind=\"{}\" title=\"{}\">\n{}\n</mention>",
            self.kind.word(),
            self.label.replace('"', "'"),
            text.trim_end()
        ))
    }

    /// For the inspector: what pi gets for it.
    pub fn how(&self) -> String {
        match &self.body {
            Body::Path { line: None, .. } if self.kind == Kind::Directory => {
                "directory path · no contents attached".into()
            }
            Body::Path { line: None, .. } => "path · pi reads it".into(),
            Body::Path { line: Some(_), .. } => "location · pi reads it".into(),
            Body::Text(text) => match text.lines().count() {
                1 => "text · 1 line".into(),
                lines => format!("text · {lines} lines"),
            },
        }
    }
}

/// The prompt pi receives for a draft: each chip replaced by its inline form, and
/// text mentions appended in the order they appear.
pub fn prompt(content: &str, chips: &[Chip], mentions: &HashMap<usize, Mention>) -> String {
    let mut out = String::new();
    let mut blocks = Vec::new();
    let mut last = 0;
    for chip in chips {
        out.push_str(&content[last..chip.range.start]);
        match mentions.get(&chip.id) {
            Some(mention) => {
                out.push_str(&mention.inline());
                blocks.extend(mention.block());
            }
            None => out.push_str(&label_of(&content[chip.range.clone()])),
        }
        last = chip.range.end;
    }
    out.push_str(&content[last..]);
    let mut out = out.trim().to_owned();
    for block in blocks {
        out.push_str("\n\n");
        out.push_str(&block);
    }
    out
}

fn label_of(chip_text: &str) -> String {
    chip_text
        .replace(crate::input::CHIP_LEAD, "")
        .replace('\u{a0}', " ")
}

/// A sent message with its mentions found again.
#[derive(Debug, PartialEq)]
pub struct Sent {
    /// Markdown in which each mention is a `pi-mention:` link around inline code,
    /// which the Markdown renderer draws as a rounded chip.
    pub markdown: String,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub kind: Kind,
    pub title: String,
    pub body: String,
}

/// Where a chip link in a sent message leads.
#[derive(Debug, PartialEq)]
pub enum Link {
    File { path: String, line: Option<u32> },
    Directory { path: String },
    Block(usize),
}

pub const LINK_SCHEME: &str = "pi-mention:";

pub fn parse_link(url: &str) -> Option<Link> {
    let rest = url.strip_prefix(LINK_SCHEME)?;
    if let Some(index) = rest.strip_prefix("block:") {
        return index.parse().ok().map(Link::Block);
    }
    if let Some(path) = rest.strip_prefix("directory:") {
        return Some(Link::Directory {
            path: path.to_owned(),
        });
    }
    let location = rest.strip_prefix("file:")?;
    Some(match location.rsplit_once(':') {
        Some((path, line)) if line.parse::<u32>().is_ok() => Link::File {
            path: path.to_owned(),
            line: line.parse().ok(),
        },
        _ => Link::File {
            path: location.to_owned(),
            line: None,
        },
    })
}

/// Finds a sent message's mentions; `None` when it has none.
pub fn parse_sent(text: &str) -> Option<Sent> {
    let (body, blocks) = split_blocks(text);
    let mut used = vec![false; blocks.len()];
    let mut markdown = String::new();
    let mut found = false;
    // Only prose: code spans and fences keep their text.
    for (index, piece) in body.split("```").enumerate() {
        if index > 0 {
            markdown.push_str("```");
        }
        if index % 2 == 1 {
            markdown.push_str(piece);
            continue;
        }
        for (index, span) in piece.split('`').enumerate() {
            if index > 0 {
                markdown.push('`');
            }
            if index % 2 == 1 {
                markdown.push_str(span);
            } else {
                found |= link_mentions(span, &blocks, &mut used, &mut markdown);
            }
        }
    }
    (found || !blocks.is_empty()).then_some(Sent { markdown, blocks })
}

/// Splits trailing `<mention>` blocks off a message.
fn split_blocks(text: &str) -> (&str, Vec<Block>) {
    let mut search = 0;
    while let Some(at) = text[search..].find("\n\n<mention kind=\"") {
        let start = search + at;
        if let Some(blocks) = parse_blocks(&text[start + 2..]) {
            return (&text[..start], blocks);
        }
        search = start + 2;
    }
    (text, Vec::new())
}

fn parse_blocks(mut rest: &str) -> Option<Vec<Block>> {
    let mut blocks = Vec::new();
    loop {
        let after = rest.strip_prefix("<mention kind=\"")?;
        let (kind, after) = after.split_once('"')?;
        let after = after.strip_prefix(" title=\"")?;
        let (title, after) = after.split_once('"')?;
        let after = after.strip_prefix(">\n")?;
        let (body, after) = after.split_once("\n</mention>")?;
        blocks.push(Block {
            kind: Kind::from_word(kind)?,
            title: title.to_owned(),
            body: body.to_owned(),
        });
        if after.trim().is_empty() {
            return Some(blocks);
        }
        rest = after.strip_prefix("\n\n")?;
    }
}

/// Appends `text` with its mentions as links; true when it had any.
fn link_mentions(text: &str, blocks: &[Block], used: &mut [bool], out: &mut String) -> bool {
    let mut found = false;
    let mut rest = text;
    while !rest.is_empty() {
        // `[kind: title]` for a text mention with its block.
        if let Some(inner) = rest.strip_prefix('[')
            && let Some((reference, after)) = inner.split_once(']')
            && let Some((kind, title)) = reference.split_once(": ")
            && let Some(kind) = Kind::from_word(kind)
            && let Some(index) = (0..blocks.len())
                .find(|&i| !used[i] && blocks[i].kind == kind && blocks[i].title == title)
        {
            used[index] = true;
            out.push_str(&chip_link(title, &format!("block:{index}")));
            rest = after;
            found = true;
            continue;
        }
        let at_word_start =
            out.is_empty() || out.ends_with(|c: char| c.is_whitespace() || c == '(');
        // `@path`, and `name (@path:line)` for a symbol.
        if at_word_start && let Some((path, line, after)) = path_token(rest) {
            let name = path
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or(path);
            match line {
                None if path.ends_with('/') => out.push_str(&chip_link(
                    &format!("{name}/"),
                    &format!("directory:{path}"),
                )),
                Some(line) => out.push_str(&chip_link(name, &format!("file:{path}:{line}"))),
                None => out.push_str(&chip_link(name, &format!("file:{path}"))),
            }
            rest = after;
            found = true;
            continue;
        }
        if let Some((name, path, line, after)) = symbol_token(rest)
            && (out.is_empty() || out.ends_with(char::is_whitespace))
        {
            out.push_str(&chip_link(name, &format!("file:{path}:{line}")));
            rest = after;
            found = true;
            continue;
        }
        let next = rest.chars().next().map_or(1, char::len_utf8);
        out.push_str(&rest[..next]);
        rest = &rest[next..];
    }
    found
}

fn is_path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '/')
}

/// `@path` or `@path:line` at the start of `text`.
fn path_token(text: &str) -> Option<(&str, Option<u32>, &str)> {
    let rest = text.strip_prefix('@')?;
    let end = rest.find(|c| !is_path_char(c)).unwrap_or(rest.len());
    let path = rest[..end].trim_end_matches('.');
    if path.is_empty() || !path.contains(['.', '/']) {
        return None;
    }
    let after = &rest[path.len()..];
    if let Some(digits) = after.strip_prefix(':') {
        let end = digits
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(digits.len());
        if end > 0 {
            return Some((path, digits[..end].parse().ok(), &digits[end..]));
        }
    }
    Some((path, None, after))
}

/// `name (@path:line)` at the start of `text`.
fn symbol_token(text: &str) -> Option<(&str, &str, u32, &str)> {
    let end = text.find(|c: char| c.is_whitespace()).unwrap_or(text.len());
    let name = &text[..end];
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
    {
        return None;
    }
    let inner = text[end..].strip_prefix(" (")?;
    let (path, line, after) = path_token(inner)?;
    let after = after.strip_prefix(')')?;
    Some((name, path, line?, after))
}

/// A link around inline code: Markdown draws the code as a chip, the link makes it
/// clickable. Code spans take precedence over the link's brackets, so only
/// backticks need replacing.
fn chip_link(name: &str, target: &str) -> String {
    format!("[`{}`]({LINK_SCHEME}{target})", name.replace('`', "'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> Mention {
        Mention {
            kind: Kind::File,
            label: path.rsplit('/').next().unwrap().into(),
            body: Body::Path {
                path: path.into(),
                line: None,
            },
        }
    }

    #[test]
    fn chips_become_paths_and_text_blocks_pi_can_read() {
        let lead = crate::input::CHIP_LEAD;
        let content = format!(
            "Why does {lead}readThinking reject what {lead}zsh\u{a0}·\u{a0}2\u{a0}lines shows? "
        );
        let first = content.find(lead).unwrap();
        let first_end = first + lead.len() + "readThinking".len();
        let second = content[first_end..].find(lead).unwrap() + first_end;
        let second_end = second + format!("{lead}zsh\u{a0}·\u{a0}2\u{a0}lines").len();
        let chips = vec![
            Chip {
                range: first..first_end,
                icon: "list_tree",
                id: 1,
            },
            Chip {
                range: second..second_end,
                icon: "terminal",
                id: 2,
            },
        ];
        let mentions = HashMap::from([
            (
                1,
                Mention {
                    kind: Kind::Symbol,
                    label: "readThinking".into(),
                    body: Body::Path {
                        path: "packages/ai/src/openai-completions.ts".into(),
                        line: Some(205),
                    },
                },
            ),
            (
                2,
                Mention {
                    kind: Kind::Terminal,
                    label: "zsh · 2 lines".into(),
                    body: Body::Text("$ npm test\nfailed".into()),
                },
            ),
        ]);
        let sent = prompt(&content, &chips, &mentions);
        assert_eq!(
            sent,
            "Why does readThinking (@packages/ai/src/openai-completions.ts:205) reject what \
             [terminal: zsh · 2 lines] shows?\n\n\
             <mention kind=\"terminal\" title=\"zsh · 2 lines\">\n$ npm test\nfailed\n</mention>"
        );

        // The transcript finds both again, and the block leaves the visible text.
        let parsed = parse_sent(&sent).unwrap();
        assert_eq!(
            parsed.markdown,
            "Why does [`readThinking`](pi-mention:file:packages/ai/src/openai-completions.ts:205) \
             reject what [`zsh · 2 lines`](pi-mention:block:0) shows?"
        );
        assert_eq!(
            parsed.blocks,
            [Block {
                kind: Kind::Terminal,
                title: "zsh · 2 lines".into(),
                body: "$ npm test\nfailed".into()
            }]
        );
    }

    #[test]
    fn file_paths_become_chips_but_code_and_emails_do_not() {
        let parsed =
            parse_sent("Compare @packages/ai/src/opencode.ts with `@not/a.chip` and mail a@b.io.")
                .unwrap();
        assert_eq!(
            parsed.markdown,
            "Compare [`opencode.ts`](pi-mention:file:packages/ai/src/opencode.ts) with \
             `@not/a.chip` and mail a@b.io."
        );
        assert_eq!(parse_sent("No mentions @here or @ all."), None);
        assert_eq!(
            parse_link("pi-mention:file:a/b.ts:14"),
            Some(Link::File {
                path: "a/b.ts".into(),
                line: Some(14)
            })
        );
        assert_eq!(parse_link("pi-mention:block:2"), Some(Link::Block(2)));
        assert_eq!(file("a/b.ts").inline(), "@a/b.ts");
        assert_eq!(file("a/b.ts").how(), "path · pi reads it");
    }

    #[test]
    fn directories_are_references_not_content_attachments() {
        let mention = Mention {
            kind: Kind::Directory,
            label: "设计/".into(),
            body: Body::Path {
                path: "docs/设计/".into(),
                line: None,
            },
        };
        assert_eq!(mention.inline(), "@docs/设计/");
        assert!(mention.block().is_none());
        assert_eq!(mention.how(), "directory path · no contents attached");
        let sent = parse_sent("Inspect @docs/设计/").unwrap();
        assert_eq!(
            sent.markdown,
            "Inspect [`设计/`](pi-mention:directory:docs/设计/)"
        );
        assert_eq!(
            parse_link("pi-mention:directory:docs/设计/"),
            Some(Link::Directory {
                path: "docs/设计/".into()
            })
        );
        let spaced = Mention {
            body: Body::Path {
                path: "docs/release notes/".into(),
                line: None,
            },
            ..mention
        };
        assert_eq!(spaced.inline(), "`docs/release notes/`");
        assert!(spaced.block().is_none());
    }
}
