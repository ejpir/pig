//! 03 New session: a sheet from Home's start bar with the project, the
//! composer, and starting points that fill the draft but never send it.

use super::scroll_area;
use crate::{
    app::{PhoneApp, Route, Sheet},
    theme::{MONO, theme},
    ui::{self, icon},
};
use gpui::{Context, Focusable, Window, div, prelude::*, px};

/// Starting points: what they put in the draft, and a command they use.
const STARTERS: [(&str, Option<&str>); 3] = [
    ("Review local changes", None),
    ("Explain this project", None),
    ("Fix the failing tests", Some("/fix-tests")),
];

impl PhoneApp {
    pub(crate) fn start_screen(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = theme(cx);
        let keyboard =
            window.viewport_size().height - window.fully_visible_bounds().bottom() > px(120.);
        let scroll = self.scroll(Route::Start);
        let visible_height = window.fully_visible_bounds().size.height;
        let resized = self.start_visible_height.replace(visible_height) != Some(visible_height);
        let focused = self
            .start
            .read(cx)
            .area
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        let can_use_starter = !focused && self.start.read(cx).area.read(cx).is_empty();
        if keyboard && resized && focused {
            // Keep the same content during the IME animation. Only scroll the
            // minimum needed to keep the complete composer (including Send)
            // visible, and stop following once the keyboard settles.
            scroll.scroll_to_item(1);
        }
        let live = self.store.as_ref().is_some_and(|store| !store.is_sample());
        let commands = self.command_catalog_for_project(self.project, cx);
        self.start
            .update(cx, |composer, _| composer.use_commands(commands));
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
        let home = self.sessions_screen(window, cx).into_any_element();
        let project_picker = div().flex().child(
            ui::chip(
                "project",
                Some("folder"),
                if folder.is_empty() {
                    project
                } else {
                    format!("{project} on {computer}")
                },
                &colors,
            )
            .max_w_full()
            .child(icon("chev_d", 14., colors.muted))
            .on_click(cx.listener(|this, _, _, cx| this.open_sheet(Sheet::Project, cx))),
        );
        let starters = STARTERS.iter().enumerate().map(|(index, (text, command))| {
            // Durable sessions have no commands: the words go instead.
            let command = command.filter(|_| !live);
            let draft = command.map_or_else(|| text.to_string(), |command| format!("{command} "));
            ui::chip(("starter", index), None, command.unwrap_or(text).to_owned(), &colors)
                .debug_selector(move || format!("starter-{index}").into())
                .when(command.is_some(), |chip| chip.font_family(MONO).text_size(px(12.5)))
                .when(!can_use_starter, |chip| chip.opacity(0.55))
                .when(can_use_starter, |chip| {
                    chip.on_click(cx.listener(move |this, _, window, cx| {
                        if !this.start.read(cx).area.read(cx).is_empty() {
                            return;
                        }
                        this.start.update(cx, |composer, cx| {
                            composer.set_text(&draft, cx);
                            composer.focus(window, cx);
                        });
                    }))
                })
        }).collect::<Vec<_>>();
        let sheet = div()
            .id("start-sheet")
            .child(crate::testing::probe("start-sheet"))
            .occlude()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .max_h_full()
            .flex()
            .flex_col()
            .bg(colors.canvas)
            .rounded_t(px(24.))
            .shadow(vec![gpui::BoxShadow {
                color: colors.shadow,
                offset: gpui::point(px(0.), px(-8.)),
                blur_radius: px(32.),
                spread_radius: px(0.),
                inset: false,
            }])
            .pt(px(8.))
            .pb(px(12.))
            .child(
                div()
                    .flex_none()
                    .mx_auto()
                    .mt(px(4.))
                    .mb(px(16.))
                    .w(px(32.))
                    .h(px(4.))
                    .rounded_full()
                    .bg(colors.line_strong),
            )
            .child(
                scroll_area("start", &scroll)
                    .child(div().px(px(20.)).child(project_picker))
                    .child(div().mt(px(12.)).child(self.start.clone()))
                    .child(
                        div()
                            .id("starters")
                            .relative()
                            .child(crate::testing::probe("starters"))
                            .mt(px(12.))
                            .px(px(20.))
                            .flex()
                            .gap(px(8.))
                            .overflow_x_scroll()
                            .children(starters),
                    ),
            );
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(home)
            .child(
                div()
                    .id("start-scrim")
                    .occlude()
                    .absolute()
                    .inset_0()
                    .bg(colors.scrim)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.back(window, cx);
                    })),
            )
            .child(sheet)
    }
}
