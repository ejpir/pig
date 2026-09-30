//! Numbered tool diffs retain their source text; selectable rendering lives in Markdown.

pub fn diff_parts(line: &str) -> (&str, &str, &str) {
    let (sign, rest) = if let Some(rest) = line.strip_prefix('+') {
        ("+", rest)
    } else if let Some(rest) = line.strip_prefix('-') {
        ("−", rest)
    } else {
        ("", line.trim_start())
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && rest[digits..].starts_with(' ') {
        (&rest[..digits], sign, &rest[digits + 1..])
    } else {
        ("", sign, rest)
    }
}

pub fn diff_counts(diff: &str) -> (usize, usize) {
    diff.lines()
        .fold((0, 0), |(added, removed), line| match diff_parts(line).1 {
            "+" => (added + 1, removed),
            "−" => (added, removed + 1),
            _ => (added, removed),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diff_gutters_preserve_source_indentation_and_unicode() {
        assert_eq!(
            diff_parts("+212       if (🐈) {"),
            ("212", "+", "      if (🐈) {")
        );
        assert_eq!(diff_parts(" 211     context"), ("211", "", "    context"));
        assert_eq!(diff_parts("-not numbered"), ("", "−", "not numbered"));
        assert_eq!(diff_counts(" old\n-new\n+new\n+more"), (2, 1));
    }
}
