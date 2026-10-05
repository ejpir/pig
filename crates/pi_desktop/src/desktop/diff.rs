//! Selectable, aligned split diffs. Projections use recorded hunks or tool reports,
//! never today's file to fill missing history. Unified copy stays with the owner.
use super::panels::{DocumentView, note};
use super::*;
use pi_jj::{FileChange, LineKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DiffLine {
    pub kind: LineKind,
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub text: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct DiffSection {
    pub label: String,
    pub lines: Vec<DiffLine>,
}
impl DiffSection {
    pub fn recorded(file: &FileChange) -> Vec<Self> {
        file.hunks
            .iter()
            .map(|h| {
                let (mut old, mut new) = (h.old_start, h.new_start);
                let lines = h
                    .lines
                    .iter()
                    .map(|(kind, text)| {
                        let line = DiffLine {
                            kind: *kind,
                            old: (*kind != LineKind::Added).then_some(old),
                            new: (*kind != LineKind::Removed).then_some(new),
                            text: text.clone(),
                        };
                        old += usize::from(*kind != LineKind::Added);
                        new += usize::from(*kind != LineKind::Removed);
                        line
                    })
                    .collect();
                Self {
                    label: format!("@@ −{} +{} @@", h.old_start, h.new_start),
                    lines,
                }
            })
            .collect()
    }
    pub fn reported(label: String, patch: &str) -> Option<Self> {
        let mut lines = vec![];
        let mut delta = 0isize;
        for line in patch.lines() {
            let (marker, rest) = line.split_at_checked(1)?;
            let rest = rest.trim_start_matches(' ');
            if marker == " " && rest == "..." {
                lines.push(DiffLine {
                    kind: LineKind::Context,
                    old: None,
                    new: None,
                    text: "… unchanged span …".into(),
                });
                continue;
            }
            let end = rest.bytes().take_while(u8::is_ascii_digit).count();
            let positioned = end > 0 && rest.as_bytes().get(end) == Some(&b' ');
            let (kind, old, new, text) = if positioned {
                let n: usize = rest[..end].parse().ok()?;
                if n == 0 {
                    return None;
                }
                let (kind, old, new) = match marker {
                    "+" => {
                        delta = delta.checked_add(1)?;
                        (LineKind::Added, None, Some(n))
                    }
                    "-" => {
                        delta = delta.checked_sub(1)?;
                        (LineKind::Removed, Some(n), None)
                    }
                    " " => (
                        LineKind::Context,
                        Some(n),
                        Some(n.checked_add_signed(delta)?),
                    ),
                    _ => return None,
                };
                (kind, old, new, rest[end + 1..].to_owned())
            } else {
                let kind = match marker {
                    "+" => LineKind::Added,
                    "-" => LineKind::Removed,
                    " " => LineKind::Context,
                    _ => return None,
                };
                // Tool-reported writes and replacements often omit positions.
                // A blank line-number gutter is honest; guessing positions is not.
                (kind, None, None, rest.to_owned())
            };
            lines.push(DiffLine {
                kind,
                old,
                new,
                text,
            });
        }
        (!lines.is_empty()).then_some(Self { label, lines })
    }
}

/// Consecutive removed/added runs share rows. Extra lines get blank counterparts.
/// A shared byte budget keeps both previews aligned, including truncation.
fn split(sections: &[DiffSection]) -> (String, String, bool) {
    let mut rows: Vec<(String, String)> = vec![];
    let format_line = |line: &DiffLine, before: bool| {
        let number = if before { line.old } else { line.new };
        let marker = match (before, line.kind) {
            (true, LineKind::Removed) => '-',
            (false, LineKind::Added) => '+',
            _ => ' ',
        };
        format!(
            "{marker}{:>5}  {}",
            number.map(|n| n.to_string()).unwrap_or_default(),
            line.text
        )
    };
    for section in sections {
        if sections.len() > 1 {
            rows.push((section.label.clone(), section.label.clone()));
        }
        let mut index = 0;
        while index < section.lines.len() {
            let line = &section.lines[index];
            if line.kind == LineKind::Context {
                rows.push((format_line(line, true), format_line(line, false)));
                index += 1;
                continue;
            }
            let start = index;
            while index < section.lines.len() && section.lines[index].kind != LineKind::Context {
                index += 1;
            }
            let removed: Vec<_> = section.lines[start..index]
                .iter()
                .filter(|l| l.kind == LineKind::Removed)
                .collect();
            let added: Vec<_> = section.lines[start..index]
                .iter()
                .filter(|l| l.kind == LineKind::Added)
                .collect();
            for i in 0..removed.len().max(added.len()) {
                rows.push((
                    removed
                        .get(i)
                        .map(|l| format_line(l, true))
                        .unwrap_or_else(|| " ".into()),
                    added
                        .get(i)
                        .map(|l| format_line(l, false))
                        .unwrap_or_else(|| " ".into()),
                ));
            }
        }
    }
    let (mut before, mut after, mut bytes) = (String::new(), String::new(), 0);
    for (left, right) in rows {
        bytes += left.len() + right.len() + 2;
        if bytes > 60 * 1024 {
            before.push_str("… preview truncated …\n");
            after.push_str("… preview truncated …\n");
            return (before, after, true);
        }
        before.push_str(&left);
        before.push('\n');
        after.push_str(&right);
        after.push('\n');
    }
    (before, after, false)
}

pub(super) struct DiffView {
    pub unified: Entity<DocumentView>,
    before: Entity<DocumentView>,
    after: Entity<DocumentView>,
    can_split: bool,
    split_requested: bool,
    wide: bool,
    truncated: bool,
    removed_rows: Vec<usize>,
    added_rows: Vec<usize>,
}
impl DiffView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            unified: cx.new(|cx| DocumentView::new(cx).review().wrapped()),
            before: cx.new(|cx| DocumentView::new(cx).review().unwrapped()),
            after: cx.new(|cx| DocumentView::new(cx).review().unwrapped()),
            can_split: false,
            split_requested: true,
            wide: true,
            truncated: false,
            removed_rows: vec![],
            added_rows: vec![],
        }
    }
    pub fn set(&mut self, unified: String, sections: &[DiffSection], cx: &mut Context<Self>) {
        self.unified
            .update(cx, |d, cx| d.set(unified, Some("diff"), cx));
        let (before, after, truncated) = split(sections);
        self.removed_rows = before
            .lines()
            .enumerate()
            .filter_map(|(i, l)| l.starts_with('-').then_some(i))
            .collect();
        self.added_rows = after
            .lines()
            .enumerate()
            .filter_map(|(i, l)| l.starts_with('+').then_some(i))
            .collect();
        let unmarked = |text: &str| {
            text.lines()
                .map(|line| {
                    if line.starts_with([' ', '+', '-']) {
                        &line[1..]
                    } else {
                        line
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        self.before
            .update(cx, |d, cx| d.set(unmarked(&before), Some("diff"), cx));
        self.after
            .update(cx, |d, cx| d.set(unmarked(&after), Some("diff"), cx));
        self.can_split = !sections.is_empty();
        self.truncated = truncated;
        cx.notify();
    }
    pub fn set_wide(&mut self, wide: bool, cx: &mut Context<Self>) {
        if self.wide != wide {
            self.wide = wide;
            cx.notify();
        }
    }
    pub fn is_split(&self) -> bool {
        self.wide && self.can_split && self.split_requested
    }
    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        self.split_requested = !self.split_requested;
        cx.notify();
    }
    pub fn can_split(&self) -> bool {
        self.wide && self.can_split
    }
    pub fn selected_source(&self, cx: &App) -> Option<String> {
        if self.is_split() {
            self.before
                .read(cx)
                .selected_source(cx)
                .or_else(|| self.after.read(cx).selected_source(cx))
        } else {
            self.unified.read(cx).selected_source(cx)
        }
    }
}
impl Render for DiffView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let side = |name: &'static str,
                    selector: &'static str,
                    doc: Entity<DocumentView>,
                    rows: &[usize],
                    color: gpui::Hsla| {
            v_flex()
                .debug_selector(move || selector.into())
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .h(px(30.))
                        .px(px(12.))
                        .py(px(5.))
                        .bg(theme.work_code)
                        .rounded(px(4.))
                        .text_size(px(12.))
                        .text_color(theme.muted)
                        .child(name),
                )
                .child(
                    div()
                        .relative()
                        .mt(px(0.))
                        .w_full()
                        .min_w_0()
                        .border_b_1()
                        .border_color(theme.line)
                        .children(rows.iter().map(|row| {
                            div()
                                .absolute()
                                .left(px(0.))
                                .top(px(1. + *row as f32 * 24.))
                                .w_full()
                                .h(px(24.))
                                .bg(color)
                        }))
                        .child(doc),
                )
        };
        v_flex()
            .w_full()
            .min_w_0()
            .child(if self.is_split() {
                h_flex()
                    .items_start()
                    .gap(px(16.))
                    .w_full()
                    .min_w_0()
                    .child(side(
                        "Before",
                        "diff-before",
                        self.before.clone(),
                        &self.removed_rows,
                        theme.removed,
                    ))
                    .child(side(
                        "After",
                        "diff-after",
                        self.after.clone(),
                        &self.added_rows,
                        theme.added,
                    ))
                    .into_any_element()
            } else {
                v_flex()
                    .debug_selector(|| "diff-unified".into())
                    .w_full()
                    .min_w_0()
                    .child(
                        div()
                            .h(px(30.))
                            .px(px(12.))
                            .py(px(5.))
                            .bg(theme.work_code)
                            .rounded(px(4.))
                            .text_size(px(12.))
                            .text_color(theme.muted)
                            .child("Unified diff"),
                    )
                    .child(self.unified.clone())
                    .into_any_element()
            })
            .when(self.is_split() && self.truncated, |v| {
                v.child(note(
                    "Preview limited to 60 KiB. Copy diff retains the complete patch.",
                    theme,
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn asymmetric_hunks_align_without_guessing_missing_lines() {
        let section = DiffSection {
            label: "hunk".into(),
            lines: vec![
                DiffLine {
                    kind: LineKind::Removed,
                    old: Some(8),
                    new: None,
                    text: "old".into(),
                },
                DiffLine {
                    kind: LineKind::Added,
                    old: None,
                    new: Some(9),
                    text: "new".into(),
                },
                DiffLine {
                    kind: LineKind::Added,
                    old: None,
                    new: Some(10),
                    text: "extra".into(),
                },
                DiffLine {
                    kind: LineKind::Context,
                    old: Some(9),
                    new: Some(11),
                    text: "  end".into(),
                },
            ],
        };
        let (before, after, truncated) = split(&[section]);
        assert!(!truncated);
        assert_eq!(before.lines().count(), after.lines().count());
        assert_eq!(before.lines().nth(1), Some(" "));
        assert!(before.contains("    9    end"));
        assert!(after.contains("   11    end"));
    }
    #[test]
    fn unpositioned_reports_still_have_an_honest_split_projection() {
        let section = DiffSection::reported("write · call".into(), "+hello\n+world").unwrap();
        assert_eq!(section.lines.len(), 2);
        assert!(
            section
                .lines
                .iter()
                .all(|line| line.old.is_none() && line.new.is_none())
        );
        let (before, after, truncated) = split(&[section]);
        assert!(!truncated);
        assert_eq!(before.lines().count(), after.lines().count());
        assert!(after.contains("hello"));
        assert!(!before.contains("hello"));
    }

    #[test]
    fn reported_numbers_and_unicode_are_preserved() {
        let section = DiffSection::reported(
            "edit · call".into(),
            " 10 same\n-11 old\n+11 新\n+12 extra\n 12 after\n    ...\n 40 later",
        )
        .unwrap();
        let (before, after, _) = split(&[section]);
        assert!(before.contains("   40  later"));
        assert!(after.contains("   41  later"));
        assert!(after.contains("新"));
        assert!(DiffSection::reported("".into(), "-old\n+new").is_some());
    }
    #[test]
    fn oversized_sides_truncate_together_at_a_row_boundary() {
        let section = DiffSection {
            label: "hunk".into(),
            lines: (0..1000)
                .map(|i| DiffLine {
                    kind: LineKind::Added,
                    old: None,
                    new: Some(i),
                    text: "界".repeat(100),
                })
                .collect(),
        };
        let (before, after, truncated) = split(&[section]);
        assert!(truncated);
        assert_eq!(before.lines().count(), after.lines().count());
        assert!(before.len() < 65536 && after.len() < 65536);
    }
}
