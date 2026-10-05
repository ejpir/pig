//! The second connection step and the reusable project picker.

use super::{heading, scroll_area};
use crate::{
    app::{PhoneApp, Route},
    theme::{Theme, theme},
    ui::{self, Button, icon},
};
use gpui::{Context, Div, Focusable, Window, div, prelude::*, px};

impl PhoneApp {
    pub(crate) fn projects_screen(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = theme(cx);
        let scroll = self.scroll(Route::Projects);
        div().flex_1().min_h_0().flex().flex_col()
            .child(ui::appbar(
                ui::tap("back-projects", "back", &colors).on_click(cx.listener(|this, _, window, cx| { this.back(window, cx); })),
                "Choose a project", None, &colors))
            .child(scroll_area("choose-project", &scroll).child(
                div().px(px(20.)).pb(px(24.))
                    .child(ui::label("Step 2 of 2 · Computer connected", &colors).mt(px(12.)))
                    .child(heading("Where shall we work?", 28.).mt(px(12.)))
                    .child(ui::hint("Choose a recent project or browse folders on your computer. Nothing starts until you send a prompt.", &colors).mt(px(10.)).mb(px(20.)))
                    .child(self.project_picker(&colors, cx))))
            .child(ui::button("view-sessions", Button::Plain, None, "Just view sessions", false, &colors)
                .mx(px(20.)).my(px(12.))
                .on_click(cx.listener(|this, _, window, cx| { this.back(window, cx); })))
    }

    pub(crate) fn project_picker(&self, colors: &Theme, cx: &Context<Self>) -> Div {
        let browser = &self.project_browser;
        let projects = self
            .store
            .as_ref()
            .map(|store| store.projects.clone())
            .unwrap_or_default();
        let live = self.store.as_ref().and_then(|store| store.live.as_ref());
        let short =
            |path: &str| live.map_or_else(|| path.to_owned(), |live| live.helper.short(path));
        let directory = browser.directory.clone();
        let field = div()
            .id("folder")
            .h(px(48.))
            .px(px(14.))
            .flex()
            .items_center()
            .rounded(px(12.))
            .border_1()
            .border_color(colors.line_strong)
            .bg(colors.canvas)
            .child(div().flex_1().min_w_0().child(self.folder.clone()))
            .on_click(cx.listener(|this, _, window, cx| {
                window.focus(&this.folder.read(cx).focus_handle(cx), cx)
            }));
        div().flex().flex_col().gap(px(10.))
            .when(!projects.is_empty(), |body| body
                .child(ui::label("Recent projects", colors))
                .child(ui::card(colors).children(projects.into_iter().enumerate().map(|(index, project)| {
                    ui::row(("project", index), index == 0, colors)
                        .debug_selector(move || format!("project-choice-{index}").into())
                        .child(icon("folder", 20., colors.muted))
                        .child(ui::row_text(project.name, Some(project.folder.into()), colors))
                        .when(self.project == index, |row| row.child(icon("check", 20., colors.accent)))
                        .on_click(cx.listener(move |this, _, _, cx| this.select_project(index, cx)))
                }))))
            .child(div().mt(px(8.)).flex().items_center().gap(px(8.))
                .child(ui::label("Browse remote folders", colors).flex_1())
                .child(ui::button("folders-home", Button::Plain, Some("folder"), "Home", true, colors)
                    .on_click(cx.listener(|this, _, _, cx| this.browse_projects(None, cx)))))
            .when(browser.loading, |body| body.child(
                div().flex().items_center().gap(px(8.)).py(px(8.))
                    .child(ui::working_indicator(colors)).child(ui::hint("Loading folders…", colors))))
            .children(browser.error.clone().map(|error| div().p(px(12.)).rounded(px(12.)).bg(colors.coral.opacity(0.08))
                .child(ui::hint(error, colors).text_color(colors.coral))
                .child(ui::button("retry-folders", Button::Plain, None, "Retry", true, colors).mt(px(8.))
                    .on_click(cx.listener(|this, _, _, cx| this.browse_projects(this.project_browser.requested.clone(), cx))))))
            .children(directory.map(|directory| {
                let can_use = !browser.loading && browser.error.is_none();
                let path = folder_label(&short(&directory.path));
                let full_path = directory.path.clone();
                div().flex().flex_col().gap(px(10.))
                    .child(div().flex().items_center().gap(px(8.))
                        .children(directory.parent.map(|parent| ui::tap("folder-up", "back", colors)
                            .on_click(cx.listener(move |this, _, _, cx| this.browse_projects(Some(parent.clone()), cx)))))
                        .child(ui::mono(path, 13.).flex_1().min_w_0().text_color(colors.secondary))
                        .child(ui::tap("copy-folder-path", "copy", colors).on_click(cx.listener(move |this, _, _, cx| this.copy(full_path.clone(), "the folder path", cx)))))
                    .child(ui::button("use-folder", Button::Primary, Some("check"), "Use this folder", false, colors)
                        .debug_selector(|| "use-folder".into()).w_full()
                        .when(!can_use, |button| button.opacity(0.45))
                        .when(can_use, |button| button.on_click(cx.listener(|this, _, _, cx| this.use_browsed_project(cx)))))
                    .when(directory.entries.is_empty() && !browser.loading, |body| body.child(ui::hint("No subfolders. You can use this folder as the project.", colors)))
                    .when(!directory.entries.is_empty(), |body| body.child(ui::card(colors).children(
                        directory.entries.into_iter().enumerate().map(|(index, entry)| {
                            ui::row(("remote-folder", index), index == 0, colors)
                                .debug_selector(move || format!("remote-folder-{index}").into())
                                .child(icon("folder", 20., if entry.project { colors.accent } else { colors.muted }))
                                .child(ui::row_text(entry.name, entry.project.then(|| "Project folder".into()), colors))
                                .child(icon("chev_r", 16., colors.muted))
                                .on_click(cx.listener(move |this, _, _, cx| this.browse_projects(Some(entry.path.clone()), cx)))
                        })
                    )))
                    .when(directory.truncated, |body| body.child(ui::hint("This folder is very large; only part of its contents is shown. Enter a specific path to open another folder.", colors)))
            }))
            .child(ui::button("toggle-hidden-folders", Button::Plain, None, if browser.show_hidden { "Hide hidden folders" } else { "Show hidden folders" }, true, colors)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_browser.show_hidden = !this.project_browser.show_hidden;
                    let path = this.project_browser.directory.as_ref().map(|directory| directory.path.clone());
                    this.browse_projects(path, cx);
                })))
            .child(ui::button("manual-folder", Button::Plain, None, if browser.manual { "Hide path entry" } else { "Enter a path instead" }, true, colors)
                .on_click(cx.listener(|this, _, _, cx| { this.project_browser.manual = !this.project_browser.manual; cx.notify(); })))
            .when(browser.manual, |body| body.child(field)
                .child(ui::hint("An absolute path or ~/folder on the computer.", colors))
                .child(ui::button("open-folder-path", Button::Plain, None, "Open folder", false, colors)
                    .on_click(cx.listener(|this, _, window, cx| this.add_folder(window, cx)))))
    }
}

/// Keep navigation and selection visible even hundreds of components deep.
/// The adjacent Copy control always copies the complete, unmodified path.
fn folder_label(path: &str) -> String {
    if path.chars().count() <= 44 {
        return path.to_owned();
    }
    let beginning: String = path.chars().take(12).collect();
    let ending: String = path
        .chars()
        .rev()
        .take(28)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{beginning}…{ending}")
}

#[cfg(test)]
mod tests {
    use super::folder_label;

    #[test]
    fn deep_paths_have_a_bounded_unicode_safe_label() {
        assert_eq!(folder_label("~/repos/pi"), "~/repos/pi");
        let path = format!("/Users/me/{}/日本語-project", "very-long-name/".repeat(30));
        let label = folder_label(&path);
        assert!(label.chars().count() <= 44);
        assert!(label.ends_with("日本語-project"));
    }
}
