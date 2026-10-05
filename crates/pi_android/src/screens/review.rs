//! 07 Review: the edits Pi made, file by file. Tapping lines attaches them to
//! a follow-up, which goes back to the session.

use super::scroll_area;
use crate::{
    app::{PhoneApp, Route, Sheet},
    composer::Attachment,
    model::{FileChange, SessionId},
    theme::theme,
    ui::{self, icon},
};
use gpui::{AnyElement, Context, FontWeight, Window, div, prelude::*, px};
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
        let stopping = self
            .store
            .as_ref()
            .and_then(|store| store.live.as_ref())
            .is_some_and(|live| live.is_stopping(id));
        self.review.update(cx, |composer, _| {
            composer.set_running(session.state.is_running(), stopping)
        });
        let computer = self
            .store
            .as_ref()
            .map(|store| store.computer.name.clone())
            .unwrap_or_default();
        let count = session.files.len();
        let subtitle = match count {
            1 => format!("{} · 1 file", session.title),
            count => format!("{} · {count} files", session.title),
        };
        let appbar = ui::appbar(back, "Changes", Some(subtitle.into()), &colors).child(
            ui::tap("more", "dots", &colors)
                .on_click(cx.listener(move |this, _, _, cx| this.open_sheet(Sheet::More(id), cx))),
        );
        let shown = self.review_file.min(count.saturating_sub(1));
        let files = div()
            .id("files")
            .flex_none()
            .flex()
            .gap(px(8.))
            .px(px(16.))
            .pt(px(2.))
            .pb(px(12.))
            .overflow_x_scroll()
            .children(session.files.iter().enumerate().map(|(index, file)| {
                ui::chip(
                    ("file", index),
                    Some("file"),
                    file.name().to_owned(),
                    &colors,
                )
                .when(index == shown, |chip| {
                    chip.bg(colors.selected)
                        .border_color(colors.selected)
                        .text_color(colors.text)
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .child(ui::counts(file.added, file.removed, &colors))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.review_file = index;
                    this.review_lines.clear();
                    this.review
                        .update(cx, |review, cx| review.set_lines(None, cx));
                    cx.notify();
                }))
            }));
        let diff = session.files.get(shown).map(|file| {
            ui::card(&colors)
                .rounded(px(14.))
                .child(
                    div()
                        .px(px(12.))
                        .py(px(10.))
                        .border_b_1()
                        .border_color(colors.line)
                        .child(
                            ui::mono(file.path.clone(), 12.)
                                .text_color(colors.secondary)
                                .truncate(),
                        ),
                )
                .children(file.hunks.iter().enumerate().map(|(hunk_index, hunk)| {
                    div()
                        .child(
                            ui::mono(hunk.header.clone(), 11.5)
                                .px(px(12.))
                                .py(px(6.))
                                .bg(colors.panel)
                                .text_color(colors.muted),
                        )
                        .child(div().py(px(2.)).children(hunk.lines.iter().enumerate().map(
                            |(index, line)| {
                                let selected = self.review_lines.contains(&(hunk_index, index));
                                div()
                                    .id(("line", hunk_index * 10_000 + index))
                                    .child(
                                        ui::diff_line(line, selected, &colors)
                                            .text_size(px(12.5))
                                            .line_height(px(22.)),
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.toggle_line(id, hunk_index, index, cx)
                                    }))
                            },
                        )))
                }))
        });
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(appbar)
            .child(files)
            .child(
                ui::hint(
                    format!("Edits Pi made in this session, already on {computer}. Tap a line to ask about it."),
                    &colors,
                )
                .px(px(20.))
                .pb(px(10.)),
            )
            .child(
                scroll_area(("review", id.0 as usize), &scroll)
                    .child(div().px(px(12.)).pb(px(12.)).children(diff).when(count == 0, |list| {
                        list.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(10.))
                                .px(px(8.))
                                .child(icon("info", 16., colors.muted))
                                .child(ui::hint("Pi changed no files in this session.", &colors)),
                        )
                    })),
            )
            .child(div().flex_none().pt(px(10.)).pb(px(10.)).child(self.review.clone()))
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

/// "line 211" or "lines 211–212", from the tapped lines' numbers.
fn lines_label(file: &FileChange, lines: &BTreeSet<(usize, usize)>) -> Option<String> {
    let numbers: Vec<u32> = lines
        .iter()
        .filter_map(|(hunk, line)| file.hunks.get(*hunk)?.lines.get(*line))
        .map(|line| line.number)
        .collect();
    let first = *numbers.iter().min()?;
    let last = *numbers.iter().max()?;
    Some(if first == last {
        format!("line {first}")
    } else {
        format!("lines {first}–{last}")
    })
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
}
