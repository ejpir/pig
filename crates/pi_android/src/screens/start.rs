//! 03 New session: the task first, then files and details as needed.

use super::{heading, scroll_area};
use crate::{
    app::{PhoneApp, Route, Sheet},
    theme::theme,
    ui::{self, icon},
};
use gpui::{Context, Window, div, prelude::*, px};

/// Starting points: what they put in the draft, and a command they use.
const STARTERS: [(&str, Option<&str>); 3] = [
    ("Review the local changes", None),
    ("Explain this project", None),
    ("Fix the failing tests", Some("/fix-tests")),
];

impl PhoneApp {
    pub(crate) fn start_screen(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = theme(cx);
        let scroll = self.scroll(Route::Start);
        let live = self.store.as_ref().is_some_and(|store| !store.is_sample());
        let computer = self
            .store
            .as_ref()
            .map(|store| store.computer.name.clone())
            .unwrap_or_default();
        let (project, folder) = self
            .store
            .as_ref()
            .and_then(|store| store.projects.get(self.project))
            .map_or_else(
                || ("Choose a project".to_owned(), String::new()),
                |project| (project.name.clone(), project.folder.clone()),
            );
        let appbar = ui::appbar(
            ui::tap("close", "x", &colors).on_click(cx.listener(|this, _, window, cx| {
                this.back(window, cx);
            })),
            "New session",
            None,
            &colors,
        )
        .child(
            ui::chip(
                "project",
                Some("folder"),
                format!("{project} · {computer}"),
                &colors,
            )
            .mr(px(12.))
            .child(icon("chev_d", 14., colors.muted))
            .on_click(cx.listener(|this, _, _, cx| this.open_sheet(Sheet::Project, cx))),
        );
        let starters = STARTERS.iter().enumerate().map(|(index, (text, command))| {
            // Durable sessions have no commands: the words go instead.
            let command = command.filter(|_| !live);
            let draft = command.map_or_else(|| text.to_string(), |command| format!("{command} "));
            div()
                .id(("starter", index))
                .h(px(52.))
                .flex()
                .items_center()
                .gap(px(12.))
                .border_b_1()
                .border_color(colors.line)
                .active(|style| style.bg(colors.selected))
                .child(icon("chat", 16., colors.muted))
                .child(div().flex_1().min_w_0().truncate().child(*text))
                .children(command.map(|command| ui::mono(command, 12.).text_color(colors.muted)))
                .child(icon("chev_r", 16., colors.muted))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.start.update(cx, |composer, cx| {
                        composer.set_text(&draft, cx);
                        composer.focus(window, cx);
                    });
                }))
        });
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(appbar)
            .child(
                scroll_area("start", &scroll).child(
                    div()
                        .pb(px(20.))
                        .child(
                            div()
                                .px(px(20.))
                                .pt(px(28.))
                                .child(heading("What should we change?", 32.))
                                .child(
                                    ui::hint("Start with the task. Bring in files and details as you need them.", &colors)
                                        .mt(px(10.))
                                        .text_size(px(15.)),
                                ),
                        )
                        .child(div().mt(px(22.)).child(self.start.clone()))
                        .child(
                            ui::hint(
                                if live {
                                    "Name files by their path in the project; Pi reads them on the computer."
                                } else {
                                    "Type / for commands or @ to include a file."
                                },
                                &colors,
                            )
                                .px(px(24.))
                                .mt(px(10.)),
                        )
                        .child(
                            div()
                                .px(px(20.))
                                .pt(px(34.))
                                .child(ui::label("A starting point", &colors).mb(px(4.)))
                                .children(starters)
                                .child(
                                    div()
                                        .mt(px(30.))
                                        .flex()
                                        .flex_col()
                                        .gap(px(8.))
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(12.))
                                                .child(ui::label("Project", &colors))
                                                .child(
                                                    ui::mono(format!("{folder} on {computer}"), 12.5)
                                                        .text_color(colors.muted),
                                                ),
                                        )
                                        .child(ui::hint(
                                            if live {
                                                "A durable session: its state is kept on the computer, so it carries on and picks up where it was."
                                            } else {
                                                "File history is on. jj records each turn that edits files."
                                            },
                                            &colors,
                                        )),
                                ),
                        ),
                ),
            )
    }
}
