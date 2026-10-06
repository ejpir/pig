//! 09 File history: points named by the prompts that made them, the current
//! point marked, and Restore on each. A full screen, as restoring files
//! deserves context and an explicit confirmation.

use super::scroll_area;
use crate::{
    app::{JjHistoryState, PhoneApp, Route, Sheet},
    model::SessionId,
    remote::JjOperation,
    theme::theme,
    ui::{self, Button, icon},
};
use gpui::{AnyElement, Context, FontWeight, Window, div, prelude::*, px};
use std::time::{SystemTime, UNIX_EPOCH};

impl PhoneApp {
    pub(crate) fn history_screen(
        &mut self,
        id: SessionId,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = theme(cx);
        let scroll = self.scroll(Route::History(id));
        let back = ui::tap("back", "back", &colors).on_click(cx.listener(|this, _, window, cx| {
            this.back(window, cx);
        }));
        let (folder, computer) = self
            .store
            .as_ref()
            .and_then(|store| {
                let session = store.session(id)?;
                Some((session.folder.clone(), store.computer.name.clone()))
            })
            .unwrap_or_else(|| ("Project".into(), "Computer".into()));
        let state = self.jj_histories.get(&id).cloned();
        let appbar = ui::appbar(
            back,
            "File history",
            Some(format!("{folder} on {computer}").into()),
            &colors,
        );
        let running = self.project_is_running(id);
        let body = match state {
            None | Some(JjHistoryState::Loading) => div()
                .py(px(48.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(12.))
                .child(ui::working_indicator(&colors))
                .child(ui::hint("Loading jj history from the computer…", &colors)),
            Some(JjHistoryState::Failed(error)) => div()
                .child(
                    ui::card(&colors)
                        .p(px(16.))
                        .border_color(colors.coral.opacity(0.55))
                        .child(ui::label("History could not be loaded", &colors))
                        .child(ui::hint(error, &colors).mt(px(6.))),
                )
                .child(
                    ui::button(
                        "retry-history",
                        Button::Plain,
                        Some("clock"),
                        "Try again",
                        false,
                        &colors,
                    )
                    .mt(px(14.))
                    .on_click(cx.listener(move |this, _, _, cx| this.load_jj_history(id, cx))),
                ),
            Some(JjHistoryState::Loaded(history)) if !history.available => {
                let can_enable = history.root.is_some();
                div().child(
                    ui::card(&colors)
                        .p(px(16.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(10.))
                                .child(icon("info", 18., colors.amber))
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("jj history is not enabled"),
                                ),
                        )
                        .child(
                            ui::hint(
                                history.reason.unwrap_or_else(|| {
                                    "No jj workspace was found for this project.".into()
                                }),
                                &colors,
                            )
                            .mt(px(8.)),
                        ),
                ).child(
                    ui::hint(
                        if can_enable {
                            "Pi does not alter a repository just by opening this screen. Turning on jj is an explicit, confirmed action."
                        } else {
                            "File history can be enabled after this folder is initialized as a Git project."
                        },
                        &colors,
                    )
                    .mt(px(14.)),
                ).when(can_enable, |body| {
                    body.child(
                        ui::button(
                            "offer-enable-jj",
                            Button::Primary,
                            Some("shield"),
                            "Turn on jj",
                            false,
                            &colors,
                        )
                        .mt(px(16.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_sheet(Sheet::EnableJj(id), cx)
                        })),
                    )
                })
            }
            Some(JjHistoryState::Loaded(history)) => {
                let empty = history.operations.is_empty();
                let points =
                    history
                        .operations
                        .into_iter()
                        .enumerate()
                        .map(|(index, operation)| {
                            point(
                                id,
                                index,
                                operation,
                                running,
                                self.restoring_history == Some((id, index)),
                                &colors,
                                cx,
                            )
                            .into_any_element()
                        });
                div()
                    .child(
                        div()
                            .pt(px(8.))
                            .pb(px(16.))
                            .text_size(px(14.))
                            .line_height(px(20.))
                            .text_color(colors.secondary)
                            .child("Each turn that edits files leaves a point to return to. Restoring saves the current files first, so you can undo it."),
                    )
                    .child(
                        div()
                            .relative()
                            .when(!empty, |points| {
                                points.child(
                                    div()
                                        .absolute()
                                        .left(px(7.))
                                        .top(px(24.))
                                        .bottom(px(36.))
                                        .w(px(2.))
                                        .bg(colors.line),
                                )
                            })
                            .child(
                                timeline_row(node(Node::Here, &colors))
                                    .child(
                                        div()
                                            .flex_1()
                                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Now"))
                                            .child(meta("The project's files as they are", &colors)),
                                    ),
                            )
                            .children(points),
                    )
                    .when(empty, |body| {
                        body.child(ui::hint("No points yet: Pi leaves one each time a turn edits files.", &colors).mt(px(12.)))
                    })
            }
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(appbar)
            .child(
                scroll_area(("history", id.0 as usize), &scroll)
                    .child(div().px(px(20.)).pt(px(8.)).pb(px(24.)).child(body)),
            )
            .when(running, |screen| {
                screen.child(
                    meta(
                        "Restore is off while Pi is working in this project.",
                        &colors,
                    )
                    .flex_none()
                    .px(px(20.))
                    .pt(px(8.))
                    .pb(px(16.)),
                )
            })
            .into_any_element()
    }
}

#[derive(Clone, Copy)]
enum Node {
    /// Where the files are now.
    Here,
    /// A point Pi's turn left.
    Turn,
    /// A snapshot of edits made on the computer, or where git started.
    Quiet,
}

fn node(kind: Node, colors: &crate::theme::Theme) -> gpui::Div {
    let node = div().flex_none().rounded_full();
    match kind {
        Node::Here => div()
            .size(px(20.))
            .mt(px(0.))
            .ml(px(-2.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(10.))
            .bg(colors.canvas.blend(colors.accent.opacity(0.2)))
            .child(node.size(px(12.)).rounded(px(6.)).bg(colors.accent)),
        Node::Turn => node
            .size(px(12.))
            .mt(px(4.))
            .mx(px(2.))
            .rounded(px(6.))
            .bg(colors.canvas)
            .border_2()
            .border_color(colors.text),
        Node::Quiet => node
            .size(px(8.))
            .mt(px(6.))
            .mx(px(4.))
            .rounded(px(4.))
            .bg(colors.canvas)
            .border_2()
            .border_color(colors.line_strong),
    }
}

/// A point on the line: its node in a 16 dp column, then what it is.
fn timeline_row(node: gpui::Div) -> gpui::Div {
    div()
        .relative()
        .flex()
        .items_start()
        .gap(px(16.))
        .py(px(12.))
        .child(div().w(px(16.)).flex_none().child(node))
}

fn meta(text: impl Into<gpui::SharedString>, colors: &crate::theme::Theme) -> gpui::Div {
    div()
        .text_size(px(12.5))
        .line_height(px(16.))
        .text_color(colors.muted)
        .child(text.into())
}

fn point(
    id: SessionId,
    index: usize,
    operation: JjOperation,
    running: bool,
    restoring: bool,
    colors: &crate::theme::Theme,
    cx: &Context<PhoneApp>,
) -> gpui::Div {
    let (kind, label) = match operation.kind.as_str() {
        "pi" => (Node::Turn, "Pi’s turn"),
        "snapshot" => (Node::Quiet, "Saved"),
        "git" => (Node::Quiet, "From git"),
        _ => (Node::Turn, "jj"),
    };
    let quiet = matches!(kind, Node::Quiet);
    let title = operation
        .description
        .strip_prefix("pi: ")
        .unwrap_or(&operation.description)
        .to_owned();
    let restore = div()
        .id(("restore-operation", index))
        .relative()
        .child(crate::testing::probe(format!(
            "{:?}",
            gpui::ElementId::from(("restore-operation", index))
        )))
        .mt(px(-10.))
        .h(px(40.))
        .px(px(8.))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(20.))
        .text_size(px(14.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors.accent)
        .active(|style| style.bg(colors.selected))
        .child(if restoring { "Restoring…" } else { "Restore" })
        .when(running || restoring, |button| button.opacity(0.35))
        .when(!running && !restoring, |button| {
            button.on_click(cx.listener(move |this, _, _, cx| {
                this.open_sheet(Sheet::RestoreHistory(id, index), cx)
            }))
        });
    timeline_row(node(kind, colors))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .when(quiet, |title| title.text_color(colors.muted))
                        .child(title),
                )
                .child(meta(
                    format!("{label}, {}", relative_time(operation.time)),
                    colors,
                )),
        )
        .child(restore)
}

fn relative_time(milliseconds: i64) -> String {
    if milliseconds <= 0 {
        return "a while ago".into();
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64);
    match (now - milliseconds / 1_000).max(0) {
        0..60 => "just now".into(),
        seconds @ 60..3_600 => format!("{} min ago", seconds / 60),
        seconds @ 3_600..86_400 => format!("{} h ago", seconds / 3_600),
        seconds @ 86_400..604_800 => format!("{} d ago", seconds / 86_400),
        seconds => format!("{} w ago", seconds / 604_800),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_history_dates_stay_compact() {
        assert_eq!(relative_time(0), "a while ago");
        assert!(relative_time(1).ends_with(" w ago"));
    }
}
