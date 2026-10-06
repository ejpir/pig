//! 07 Review: one file at a time with arrows between files, an edge-to-edge
//! diff, and tapped lines attached to the follow-up, which goes back to the
//! session.

use super::scroll_area;
use crate::{
    app::{PhoneApp, Route},
    composer::Attachment,
    model::{FileChange, SessionId},
    theme::{MONO, theme},
    ui::{self, icon},
};
use gpui::{AnyElement, Context, Window, div, prelude::*, px};
use std::collections::BTreeSet;

impl PhoneApp {
    pub(crate) fn review_screen(
        &mut self,
        id: SessionId,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = theme(cx);
        let scroll = self.scroll(Route::Review(id));
        let commands = self.command_catalog_for_session(id, cx);
        self.review
            .update(cx, |composer, _| composer.use_commands(commands));
        let (model, thinking) = self.model_settings(cx);
        self.review
            .update(cx, |composer, _| composer.set_model_label(model, thinking));
        // Removing the lines chip from the draft lets go of the lines.
        let attached = self
            .review
            .read(cx)
            .attachments()
            .iter()
            .any(|attachment| matches!(attachment, Attachment::Lines(_)));
        if !attached {
            self.review_lines.clear();
        }
        let back = ui::tap("back", "back", &colors).on_click(cx.listener(|this, _, window, cx| {
            this.back(window, cx);
        }));
        let Some(session) = self.store.as_ref().and_then(|store| store.session(id)) else {
            return div().into_any_element();
        };
        let count = session.files.len();
        let shown = self.review_file.min(count.saturating_sub(1));
        let file = session.files.get(shown);
        let (name, folder) = file.map_or_else(
            || ("Changes".to_owned(), session.title.clone()),
            |file| {
                let folder = file
                    .path
                    .rsplit_once('/')
                    .map_or_else(String::new, |(folder, _)| folder.to_owned());
                (file.name().to_owned(), folder)
            },
        );
        let step = |id: &'static str, glyph: &'static str, to: Option<usize>| {
            let tap = ui::tap(id, glyph, &colors);
            match to {
                Some(to) => tap.on_click(cx.listener(move |this, _, _, cx| {
                    this.review_file = to;
                    this.review_lines.clear();
                    this.review
                        .update(cx, |review, cx| review.set_lines(None, cx));
                    cx.notify();
                })),
                None => tap.opacity(0.45),
            }
        };
        let wrap = self.wrap_lines(cx);
        let appbar = ui::appbar(back, name, Some(folder.into()), &colors)
            .child(self.wrap_toggle("wrap-review", cx))
            .child(step("previous-file", "chev_l", shown.checked_sub(1)))
            .child(step(
                "next-file",
                "chev_r",
                (shown + 1 < count).then_some(shown + 1),
            ));
        let progress = div()
            .flex_none()
            .flex()
            .gap(px(4.))
            .px(px(20.))
            .py(px(12.))
            .children((0..count).map(|index| {
                div()
                    .flex_1()
                    .h(px(3.))
                    .rounded(px(2.))
                    .bg(if index == shown {
                        colors.accent
                    } else {
                        colors.raised
                    })
            }));
        let summary = file.map(|file| {
            div()
                .flex_none()
                .flex()
                .items_center()
                .px(px(20.))
                .pb(px(12.))
                .text_size(px(12.5))
                .line_height(px(16.))
                .text_color(colors.muted)
                .child(div().flex_1().child("Tap a line to ask Pi about it"))
                .when(file.added > 0, |row| {
                    row.child(
                        div()
                            .text_color(colors.green)
                            .child(format!("+{}", file.added)),
                    )
                })
                .child(
                    div()
                        .w(px(28.))
                        .flex()
                        .justify_end()
                        .text_color(colors.coral)
                        .when(file.removed > 0, |removed| {
                            removed.child(format!("−{}", file.removed))
                        }),
                )
        });
        let diff = file.map(|file| {
            let hunks = &file.hunks;
            div().children(hunks.iter().enumerate().map(|(hunk_index, hunk)| {
                // The lines between this hunk and the one before stay folded.
                let gap = hunk_index
                    .checked_sub(1)
                    .and_then(|before| hunks[before].lines.last())
                    .zip(hunk.lines.first())
                    .map(|(last, first)| first.number.saturating_sub(last.number + 1))
                    .filter(|gap| *gap > 0);
                div()
                    .children(gap.map(|gap| {
                        div()
                            .h(px(32.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .border_t_1()
                            .border_b_1()
                            .border_dashed()
                            .border_color(colors.line_strong)
                            .text_size(px(12.5))
                            .text_color(colors.muted)
                            .child(match gap {
                                1 => "1 unchanged line".to_owned(),
                                gap => format!("{gap} unchanged lines"),
                            })
                    }))
                    .child(
                        div()
                            .h(px(32.))
                            .px(px(20.))
                            .flex()
                            .items_center()
                            .bg(colors.panel)
                            .font_family(MONO)
                            .text_size(px(12.))
                            .text_color(colors.faint)
                            .child(hunk_title(&hunk.header)),
                    )
                    .child(
                        div()
                            .id(("hunk", hunk_index))
                            .py(px(4.))
                            .when(!wrap, |lines| lines.overflow_x_scroll())
                            .children(hunk.lines.iter().enumerate().map(|(index, line)| {
                                let selected = self.review_lines.contains(&(hunk_index, index));
                                div()
                                    .id(("line", hunk_index * 10_000 + index))
                                    .relative()
                                    .child(ui::code_line(line, selected, 48., wrap, &colors))
                                    .child(self.copyable(line.text.clone(), cx))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.toggle_line(id, hunk_index, index, cx)
                                    }))
                            })),
                    )
            }))
        });
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(appbar)
            .when(count > 1, |screen| screen.child(progress))
            .when(count <= 1, |screen| screen.child(div().h(px(12.))))
            .children(summary)
            .child(scroll_area(("review", id.0 as usize), &scroll).child(
                div().pb(px(12.)).children(diff).when(count == 0, |list| {
                    list.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .px(px(20.))
                            .child(icon("info", 16., colors.muted))
                            .child(ui::hint("Pi changed no files in this session.", &colors)),
                    )
                }),
            ))
            .child(
                div()
                    .flex_none()
                    .pt(px(8.))
                    .pb(px(12.))
                    .child(self.review.clone()),
            )
            .into_any_element()
    }

    pub(crate) fn toggle_line(
        &mut self,
        id: SessionId,
        hunk: usize,
        line: usize,
        cx: &mut Context<Self>,
    ) {
        if !self.review_lines.remove(&(hunk, line)) {
            self.review_lines.insert((hunk, line));
        }
        let label = self
            .store
            .as_ref()
            .and_then(|store| store.session(id))
            .and_then(|session| session.files.get(self.review_file))
            .and_then(|file| lines_label(file, &self.review_lines));
        self.review
            .update(cx, |review, cx| review.set_lines(label, cx));
        cx.notify();
    }
}

/// "@@ 204 readThinking", from "@@ 204,13 · readThinking".
fn hunk_title(header: &str) -> String {
    let (start, rest) = header.split_once(" · ").unwrap_or((header, ""));
    let start = start.split(',').next().unwrap_or(start);
    if rest.is_empty() {
        start.to_owned()
    } else {
        format!("{start} {rest}")
    }
}

/// "line 211", "lines 211–212", or "lines 1–10, 15–20" when the tapped
/// lines are apart, from their numbers.
fn lines_label(file: &FileChange, lines: &BTreeSet<(usize, usize)>) -> Option<String> {
    let numbers: BTreeSet<u32> = lines
        .iter()
        .filter_map(|(hunk, line)| file.hunks.get(*hunk)?.lines.get(*line))
        .map(|line| line.number)
        .collect();
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for number in numbers {
        match runs.last_mut() {
            Some((_, last)) if *last + 1 == number => *last = number,
            _ => runs.push((number, number)),
        }
    }
    let single = matches!(runs.as_slice(), [(first, last)] if first == last);
    let runs = runs
        .iter()
        .map(|(first, last)| {
            if first == last {
                first.to_string()
            } else {
                format!("{first}–{last}")
            }
        })
        .collect::<Vec<_>>();
    if runs.is_empty() {
        return None;
    }
    Some(format!(
        "{} {}",
        if single { "line" } else { "lines" },
        runs.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DiffLine, Hunk, LineKind};

    #[test]
    fn tapped_lines_are_named_by_their_numbers() {
        let file = FileChange {
            path: "a.ts".into(),
            added: 2,
            removed: 1,
            hunks: vec![Hunk {
                header: "@@ 211".into(),
                lines: vec![
                    DiffLine::new(LineKind::Removed, 211, "if (!signature) {"),
                    DiffLine::new(LineKind::Added, 211, "if (!signature &&"),
                    DiffLine::new(LineKind::Added, 212, "    isAnthropic(model)) {"),
                ],
            }],
        };
        let mut lines = BTreeSet::from([(0, 0), (0, 1)]);
        assert_eq!(lines_label(&file, &lines).as_deref(), Some("line 211"));
        lines.insert((0, 2));
        assert_eq!(lines_label(&file, &lines).as_deref(), Some("lines 211–212"));
        assert_eq!(lines_label(&file, &BTreeSet::new()), None);
    }

    #[test]
    fn lines_apart_are_named_as_separate_ranges() {
        let file = FileChange {
            path: "a.ts".into(),
            added: 0,
            removed: 0,
            hunks: vec![
                Hunk {
                    header: "@@ 1".into(),
                    lines: (1..=3)
                        .map(|n| DiffLine::new(LineKind::Context, n, ""))
                        .collect(),
                },
                Hunk {
                    header: "@@ 15".into(),
                    lines: (15..=20)
                        .map(|n| DiffLine::new(LineKind::Context, n, ""))
                        .collect(),
                },
            ],
        };
        let lines = BTreeSet::from([(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 5)]);
        assert_eq!(
            lines_label(&file, &lines).as_deref(),
            Some("lines 1–3, 15–16, 20")
        );
    }
}
