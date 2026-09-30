//! Display-only, two-column line numbers from immutable jj hunks. Raw copy
//! remains the original patch; no current-file reads or inferred line counts.
use pi_jj::{FileChange, LineKind};

pub fn numbered(file: &FileChange) -> String {
    if file.binary {
        return "Binary file changed — no text diff available.".into();
    }
    let width = file
        .hunks
        .iter()
        .map(|h| {
            h.old_start
                .max(h.new_start)
                .saturating_add(h.lines.len())
                .to_string()
                .len()
        })
        .max()
        .unwrap_or(4)
        .max(4);
    let mut text = String::new();
    for hunk in &file.hunks {
        let old_count = hunk
            .lines
            .iter()
            .filter(|(k, _)| *k != LineKind::Added)
            .count();
        let new_count = hunk
            .lines
            .iter()
            .filter(|(k, _)| *k != LineKind::Removed)
            .count();
        text.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            hunk.old_start, old_count, hunk.new_start, new_count
        ));
        let (mut old, mut new) = (hunk.old_start, hunk.new_start);
        for (kind, line) in &hunk.lines {
            match kind {
                LineKind::Context => {
                    text.push_str(&format!(" {old:>width$} {new:>width$}   {line}\n"));
                    old += 1;
                    new += 1;
                }
                LineKind::Added => {
                    text.push_str(&format!("+{:>width$} {new:>width$}   {line}\n", ""));
                    new += 1;
                }
                LineKind::Removed => {
                    text.push_str(&format!("-{old:>width$} {:>width$}   {line}\n", ""));
                    old += 1;
                }
            }
        }
    }
    text.trim_end_matches('\n').to_owned()
}

/// Pi's edit tool reports `-old`, `+new` and context with old line numbers.
/// Skips are unchanged spans, so the accumulated delta still applies afterward.
/// Reject unnumbered/unified patches rather than guessing file positions.
pub fn reported(text: &str) -> Option<String> {
    let mut rows = Vec::new();
    let mut delta = 0isize;
    for line in text.lines() {
        let (prefix, rest) = line.split_at_checked(1)?;
        let rest = rest.trim_start_matches(' ');
        if prefix == " " && rest == "..." {
            rows.push((' ', None, None, "…"));
            continue;
        }
        let end = rest.bytes().take_while(u8::is_ascii_digit).count();
        if end == 0 || rest.as_bytes().get(end) != Some(&b' ') {
            return None;
        }
        let n: usize = rest[..end].parse().ok()?;
        if n == 0 {
            return None;
        }
        let content = &rest[end + 1..];
        rows.push(match prefix {
            "+" => {
                delta = delta.checked_add(1)?;
                ('+', None, Some(n), content)
            }
            "-" => {
                delta = delta.checked_sub(1)?;
                ('-', Some(n), None, content)
            }
            " " => (' ', Some(n), Some(n.checked_add_signed(delta)?), content),
            _ => return None,
        });
    }
    if rows.is_empty() {
        return None;
    }
    let width = rows
        .iter()
        .flat_map(|(_, a, b, _)| [*a, *b])
        .flatten()
        .max()
        .unwrap_or(0)
        .to_string()
        .len()
        .max(4);
    Some(
        rows.into_iter()
            .map(|(marker, old, new, text)| {
                format!(
                    "{marker}{:>width$} {:>width$}   {text}",
                    old.map(|n| n.to_string()).unwrap_or_default(),
                    new.map(|n| n.to_string()).unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

pub fn language(path: &str) -> &'static str {
    match std::path::Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
    {
        "ts" | "tsx" => "TypeScript",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "rs" => "Rust",
        "py" => "Python",
        "go" => "Go",
        "json" | "jsonc" => "JSON",
        "md" => "Markdown",
        "yaml" | "yml" => "YAML",
        "toml" => "TOML",
        "sh" | "bash" => "Shell",
        "css" => "CSS",
        "html" => "HTML",
        _ => "Not identified",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reported_pi_positions_keep_delta_across_context_skips() {
        let text = reported(" 10 same\n-11 old\n+11 new\n+12 extra\n 12 after\n    ...\n 40 later")
            .unwrap();
        assert!(text.contains("   12   13   after"));
        assert!(text.contains("   40   41   later"));
        assert!(reported("-old\n+new").is_none());
        assert!(reported("@@ -10,1 +10,1 @@\n-old\n+new").is_none());
    }
    #[test]
    fn numbers_follow_each_side_and_reset_at_hunks() {
        let file = FileChange {
            path: "a.rs".into(),
            status: pi_jj::FileStatus::Modified,
            added: 1,
            removed: 1,
            binary: false,
            hunks: vec![pi_jj::Hunk {
                old_start: 10,
                new_start: 20,
                lines: vec![
                    (LineKind::Context, "same".into()),
                    (LineKind::Removed, "old".into()),
                    (LineKind::Added, "new".into()),
                    (LineKind::Context, "end".into()),
                ],
            }],
        };
        let text = numbered(&file);
        assert!(text.contains("   10   20   same\n"));
        assert!(text.contains("-  11        old\n"));
        assert!(text.contains("+       21   new\n"));
        assert!(text.ends_with("   12   22   end"));
    }
}
