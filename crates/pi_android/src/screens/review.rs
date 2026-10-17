//! 07 Review: one file at a time with arrows between files, an edge-to-edge
//! diff, and tapped lines attached to the follow-up, which goes back to the
//! session.

use crate::{
    app::{PhoneApp, ReviewListShape, Route},
    composer::Attachment,
    model::{DiffLine, FileChange, LineKind, SessionId, StageKind},
    theme::{MONO, theme},
    ui::{self, Button, icon},
};
use gpui::{AnyElement, Context, Window, div, list, prelude::*, px};
use std::{
    collections::{BTreeSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::Arc,
};

#[derive(Clone, Copy, Debug)]
enum ReviewRow {
    Gap(u32),
    Header(usize),
    Spacer,
    Line(usize, usize),
}

fn file_content_key(file: &FileChange) -> u64 {
    let mut hasher = DefaultHasher::new();
    file.path.hash(&mut hasher);
    for hunk in &file.hunks {
        hunk.header.hash(&mut hasher);
        for line in &hunk.lines {
            std::mem::discriminant(&line.kind).hash(&mut hasher);
            line.number.hash(&mut hasher);
            line.text.hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn call_content_key(path: &str, lines: &[DiffLine]) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    for line in lines {
        std::mem::discriminant(&line.kind).hash(&mut hasher);
        line.number.hash(&mut hasher);
        line.text.hash(&mut hasher);
    }
    hasher.finish()
}

fn review_rows(file: &FileChange) -> Vec<ReviewRow> {
    let mut rows = Vec::with_capacity(file.hunks.iter().map(|hunk| hunk.lines.len() + 3).sum());
    for (hunk_index, hunk) in file.hunks.iter().enumerate() {
        let gap = hunk_index
            .checked_sub(1)
            .and_then(|before| file.hunks[before].lines.last())
            .zip(hunk.lines.first())
            .map(|(last, first)| first.number.saturating_sub(last.number + 1))
            .filter(|gap| *gap > 0);
        rows.extend(gap.map(ReviewRow::Gap));
        rows.push(ReviewRow::Header(hunk_index));
        rows.push(ReviewRow::Spacer);
        rows.extend(
            hunk.lines
                .iter()
                .enumerate()
                .map(|(line, _)| ReviewRow::Line(hunk_index, line)),
        );
        rows.push(ReviewRow::Spacer);
    }
    rows
}

impl PhoneApp {
    pub(crate) fn review_screen(
        &mut self,
        id: SessionId,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = theme(cx);
        if let Some((turn, kind, tool)) = self.review_call {
            if let Some(screen) = self.call_diff_review_screen(id, turn, kind, tool, &colors, cx) {
                return screen;
            }
            self.review_call = None;
        }
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
        let step = |selector: &'static str, glyph: &'static str, to: Option<usize>| {
            let tap = ui::tap(selector, glyph, &colors);
            match to {
                Some(to) => tap.on_click(cx.listener(move |this, _, _, cx| {
                    this.review_file = to;
                    this.review_lines.clear();
                    this.review_list_shape = None;
                    this.review_list_offset = px(0.);
                    this.scroll(Route::Review(id))
                        .set_offset(gpui::point(px(0.), px(0.)));
                    this.review
                        .update(cx, |review, cx| review.set_lines(None, cx));
                    cx.notify();
                })),
                None => tap.opacity(0.45),
            }
        };
        let wrap = self.wrap_lines(cx);
        let (rows, diff_width, added, removed, content_key) = file.map_or_else(
            || (Vec::new(), 0., 0, 0, 0),
            |file| {
                (
                    review_rows(file),
                    file.hunks
                        .iter()
                        .flat_map(|hunk| &hunk.lines)
                        .map(|line| ui::unwrapped_code_line_width(line, 48.))
                        .fold(0., f32::max),
                    file.added,
                    file.removed,
                    file_content_key(file),
                )
            },
        );
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
        let row_count = rows.len();
        let shape = ReviewListShape {
            session: id,
            call: None,
            file: shown,
            rows: row_count,
            wrap,
            added,
            removed,
            content_key,
        };
        if self.review_list_shape != Some(shape) {
            self.review_list
                .reset_with_uniform_height(row_count, px(24.));
            self.review_list_shape = Some(shape);
            self.review_list_offset = px(0.);
        }
        let list_state = self.review_list.clone();
        let offset = list_state.scroll_px_offset_for_scrollbar().y;
        crate::scroll::note_movement(offset - self.review_list_offset);
        self.review_list_offset = offset;
        let rows = Arc::new(rows);
        let list_rows = rows.clone();
        let row_colors = colors;
        let row_scroll = scroll.clone();
        let diff_list = list(
            list_state.clone(),
            cx.processor(move |this, row_index, _, cx| {
                let Some(file) = this
                    .session(id)
                    .and_then(|session| session.files.get(shown))
                else {
                    return div().into_any_element();
                };
                let row = match list_rows[row_index] {
                    ReviewRow::Gap(gap) => div()
                        .h(px(32.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .border_t_1()
                        .border_b_1()
                        .border_dashed()
                        .border_color(row_colors.line_strong)
                        .text_size(px(12.5))
                        .text_color(row_colors.muted)
                        .child(match gap {
                            1 => "1 unchanged line".to_owned(),
                            gap => format!("{gap} unchanged lines"),
                        })
                        .into_any_element(),
                    ReviewRow::Header(hunk) => div()
                        .h(px(32.))
                        .px(px(20.))
                        .flex()
                        .items_center()
                        .bg(row_colors.panel)
                        .font_family(MONO)
                        .text_size(px(12.))
                        .text_color(row_colors.faint)
                        .child(hunk_title(&file.hunks[hunk].header))
                        .into_any_element(),
                    ReviewRow::Spacer => div().h(px(4.)).into_any_element(),
                    ReviewRow::Line(hunk, line_index) => {
                        let line = &file.hunks[hunk].lines[line_index];
                        let selected = this.review_lines.contains(&(hunk, line_index));
                        div()
                            .id(("line", hunk * 10_000 + line_index))
                            .debug_selector(move || format!("review-line-{hunk}-{line_index}"))
                            .relative()
                            .child(ui::code_line(line, selected, 48., wrap, &row_colors))
                            .child(this.copyable(line.text.clone(), cx))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_line(id, hunk, line_index, cx)
                            }))
                            .into_any_element()
                    }
                };
                if wrap {
                    row
                } else {
                    div()
                        .id(("review-row-pan", row_index))
                        .w_full()
                        .overflow_x_scroll()
                        .restrict_scroll_to_axis()
                        .track_scroll(&row_scroll)
                        .child(div().min_w(px(diff_width)).child(row))
                        .into_any_element()
                }
            }),
        )
        .size_full();
        let diff_body = if count == 0 {
            div()
                .flex_1()
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(20.))
                .child(icon("info", 16., colors.muted))
                .child(ui::hint("Pi changed no files in this session.", &colors))
                .into_any_element()
        } else {
            crate::scroll::virtual_list(("review", id.0 as usize), &list_state)
                .flex_1()
                .min_h_0()
                .child(diff_list)
                .into_any_element()
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(appbar)
            .when(count > 1, |screen| screen.child(progress))
            .when(count <= 1, |screen| screen.child(div().h(px(12.))))
            .children(summary)
            .child(diff_body)
            .child(
                div()
                    .flex_none()
                    .pt(px(8.))
                    .pb(px(12.))
                    .child(self.review.clone()),
            )
            .into_any_element()
    }

    /// A single edit/write call as a full-screen file diff. This deliberately
    /// leaves command output and neighboring calls behind in the thread.
    fn call_diff_review_screen(
        &mut self,
        id: SessionId,
        turn_index: usize,
        kind: StageKind,
        tool_index: usize,
        colors: &crate::theme::Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let tool = self
            .session(id)?
            .turns
            .get(turn_index)?
            .tool(kind, tool_index)?;
        if tool.diff.is_empty() {
            return None;
        }
        let path = tool.target.clone();
        let line_count = tool.diff.len();
        let diff_width = tool
            .diff
            .iter()
            .map(|line| ui::unwrapped_code_line_width(line, 48.))
            .fold(0., f32::max);
        let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
        let folder = path
            .rsplit_once('/')
            .map_or_else(String::new, |(folder, _)| folder.to_owned());
        let subtitle: Option<gpui::SharedString> = (!folder.is_empty()).then(|| folder.into());
        let wrap = self.wrap_lines(cx);
        let language = super::language(&path);
        let line_count_u32 = u32::try_from(line_count).unwrap_or(u32::MAX);
        let shape = ReviewListShape {
            session: id,
            call: Some((turn_index, kind, tool_index)),
            file: 0,
            rows: line_count,
            wrap,
            added: line_count_u32,
            removed: 0,
            content_key: call_content_key(&path, &tool.diff),
        };
        if self.review_list_shape != Some(shape) {
            self.review_list
                .reset_with_uniform_height(line_count, px(22.));
            self.review_list_shape = Some(shape);
            self.review_list_offset = px(0.);
        }
        let list_state = self.review_list.clone();
        let offset = list_state.scroll_px_offset_for_scrollbar().y;
        crate::scroll::note_movement(offset - self.review_list_offset);
        self.review_list_offset = offset;
        let scroll = self.scroll(Route::Review(id));
        let row_colors = *colors;
        let row_scroll = scroll.clone();
        let diff_list = list(
            list_state.clone(),
            cx.processor(move |this, index, _, _| {
                let Some(line) = this
                    .session(id)
                    .and_then(|session| session.turns.get(turn_index))
                    .and_then(|turn| turn.tool(kind, tool_index))
                    .and_then(|tool| tool.diff.get(index))
                else {
                    return div().into_any_element();
                };
                let row = div()
                    .id(("call-diff-review-row", index))
                    .when(index == 0, |row| row.pt(px(8.)))
                    .when(index + 1 == line_count, |row| row.pb(px(8.)))
                    .child(
                        ui::code_line_in(line, false, 48., wrap, language, &row_colors)
                            .debug_selector(move || format!("call-diff-review-line-{index}")),
                    );
                if wrap {
                    row.into_any_element()
                } else {
                    div()
                        .id(("call-diff-review-pan", index))
                        .w_full()
                        .overflow_x_scroll()
                        .restrict_scroll_to_axis()
                        .track_scroll(&row_scroll)
                        .child(div().min_w(px(diff_width)).child(row))
                        .into_any_element()
                }
            }),
        )
        .size_full();
        let diff_body =
            crate::scroll::virtual_list(("call-diff-review", id.0 as usize), &list_state)
                .flex_1()
                .min_h_0()
                .child(diff_list)
                .into_any_element();
        let can_ask = self
            .store
            .as_ref()
            .is_some_and(|store| store.session(id).is_some());
        let back = ui::tap("back", "back", colors).on_click(cx.listener(|this, _, window, cx| {
            this.back(window, cx);
        }));
        let appbar = ui::appbar(back, name, subtitle, colors)
            .debug_selector(|| "call-diff-review-file".into())
            .child(self.wrap_toggle("wrap-call-diff-review", cx));
        Some(
            div()
                .debug_selector(|| "call-diff-review".into())
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(appbar)
                .child(diff_body)
                .when(can_ask, |screen| {
                    screen.child(
                        div().flex_none().px(px(20.)).pt(px(8.)).pb(px(12.)).child(
                            ui::button(
                                "ask-about-diff",
                                Button::Plain,
                                Some("clip"),
                                "Ask about this",
                                false,
                                colors,
                            )
                            .w_full()
                            .bg(colors.tint(colors.accent))
                            .border_color(gpui::transparent_black())
                            .text_color(colors.accent)
                            .debug_selector(|| "ask-about-diff".into())
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    let Some((attachment_name, attachment_text)) = this
                                        .session(id)
                                        .and_then(|session| session.turns.get(turn_index))
                                        .and_then(|turn| turn.tool(kind, tool_index))
                                        .map(|tool| {
                                            (
                                                format!(
                                                    "{} diff",
                                                    tool.target
                                                        .rsplit('/')
                                                        .next()
                                                        .unwrap_or(&tool.target)
                                                ),
                                                call_diff_text(&tool.target, &tool.diff),
                                            )
                                        })
                                    else {
                                        return;
                                    };
                                    let composer = this.thread_composer(id, window, cx);
                                    let attachment = Attachment::File {
                                        name: attachment_name,
                                        size: crate::app::size_label(attachment_text.len() as u64),
                                        contents: attachment_text,
                                    };
                                    match composer.update(cx, |composer, cx| {
                                        composer.try_attach(attachment, cx)
                                    }) {
                                        Ok(()) => {
                                            this.review_call = None;
                                            this.back(window, cx);
                                            composer.update(cx, |composer, cx| {
                                                composer.focus(window, cx)
                                            });
                                        }
                                        Err(error) => this.notify_user(error, cx),
                                    }
                                },
                            )),
                        ),
                    )
                })
                .into_any_element(),
        )
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

fn call_diff_text(path: &str, lines: &[DiffLine]) -> String {
    let mut text = format!("File: {path}\n\n```diff\n");
    for line in lines {
        text.push(match line.kind {
            LineKind::Context => ' ',
            LineKind::Added => '+',
            LineKind::Removed => '-',
        });
        text.push_str(&line.text);
        text.push('\n');
    }
    text.push_str("```\n");
    text
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
