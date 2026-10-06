//! 03 A file, opened read-only from the project browser: edge to edge as in
//! Review, with its file history and a new session that asks about it.

use super::scroll_area;
use crate::{
    app::{PhoneApp, Route, size_label},
    projects::Load,
    theme::{MONO, theme},
    ui::{self, Button},
};
use gpui::{Context, Window, div, prelude::*, px};

/// More than this many lines are left for the computer to show.
const LINES: usize = 3_000;

/// The language a file's extension names, for highlighting.
fn language(name: &str) -> Option<&'static str> {
    Some(match name.rsplit_once('.')?.1 {
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "py" => "python",
        "json" => "json",
        "toml" => "toml",
        "md" => "markdown",
        "sh" | "bash" | "zsh" => "bash",
        "yml" | "yaml" => "yaml",
        "go" => "go",
        "css" => "css",
        "html" | "htm" => "html",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "cpp",
        _ => return None,
    })
}

impl PhoneApp {
    pub(crate) fn file_screen(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = theme(cx);
        let scroll = self.scroll(Route::File);
        let Some(view) = &self.file_view else {
            return div();
        };
        let project = view.root.rsplit('/').next().unwrap_or_default().to_owned();
        let subtitle = if view.folder().is_empty() {
            project.clone()
        } else {
            format!("{project} · {}", view.folder())
        };
        let folder = self.short_path(&view.root);
        // The newest session in this project that changed it, else any.
        let sessions = self.store.as_ref().map_or(&[][..], |store| &store.sessions);
        let in_project: Vec<_> = sessions.iter().filter(|s| s.folder == folder).collect();
        let edited = in_project
            .iter()
            .find(|session| session.files.iter().any(|file| file.path == view.path));
        let edited_note = edited.map(|session| {
            if session.state.is_running() {
                "Pi is editing it".to_owned()
            } else {
                session.finished_at.as_ref().map_or_else(
                    || "Edited by Pi".into(),
                    |at| format!("Edited by Pi at {at}"),
                )
            }
        });
        let history = edited.or(in_project.first()).map(|session| session.id);
        let text = match &view.text {
            Load::Ready(text) => Some(text.clone()),
            _ => None,
        };
        let lines = text.as_ref().map(|text| text.lines().count());
        let meta = [
            lines.map(|lines| match lines {
                1 => "1 line".to_owned(),
                lines => format!("{lines} lines"),
            }),
            view.size
                .or_else(|| text.as_ref().map(|text| text.len() as u64))
                .map(size_label),
            Some("read-only".into()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let mention = if view.path.contains(char::is_whitespace) {
            format!("@\"{}\" ", view.path)
        } else {
            format!("@{} ", view.path)
        };
        let root = view.root.clone();
        let wrap = self.wrap_lines(cx);
        let body = match &view.text {
            Load::Loading => div()
                .p(px(20.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(ui::working_indicator(&colors))
                .child(ui::hint("Reading the file…", &colors)),
            Load::Failed(error) => div()
                .p(px(20.))
                .child(ui::hint(error.clone(), &colors).text_color(colors.coral)),
            Load::Ready(text) => {
                let shown: Vec<&str> = text.lines().take(LINES).collect();
                let more = text.lines().count().saturating_sub(LINES);
                let digits = shown.len().max(1).to_string().len();
                let gutter = (digits as f32 * 7.6 + 16.).max(40.);
                let numbers = (1..=shown.len().max(1))
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                let code = shown.join("\n");
                let language = language(view.name());
                // Wrapped, each line keeps its number beside it.
                let wrapped = wrap.then(|| {
                    div()
                        .relative()
                        .font_family(MONO)
                        .text_size(px(12.5))
                        .line_height(px(22.))
                        .text_color(colors.plain)
                        .children(shown.iter().enumerate().map(|(index, line)| {
                            div()
                                .flex()
                                .child(
                                    div()
                                        .w(px(gutter))
                                        .flex_none()
                                        .pr(px(12.))
                                        .text_right()
                                        .text_color(colors.faint)
                                        .child((index + 1).to_string()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .pr(px(20.))
                                        .child(ui::code_text_language(line, language, &colors)),
                                )
                        }))
                        .child(self.copyable(text.clone(), cx))
                });
                div()
                    .py(px(8.))
                    .children(wrapped)
                    .when(!wrap, |body| {
                        body.child(
                            div()
                                .relative()
                                .flex()
                                .font_family(MONO)
                                .text_size(px(12.5))
                                .line_height(px(22.))
                                .text_color(colors.plain)
                                .child(
                                    div()
                                        .w(px(gutter))
                                        .flex_none()
                                        .pr(px(12.))
                                        .text_right()
                                        .text_color(colors.faint)
                                        .child(numbers),
                                )
                                .child(
                                    div()
                                        .id("file-code")
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_x_scroll()
                                        .child(div().pr(px(20.)).whitespace_nowrap().child(
                                            ui::code_text_language(&code, language, &colors),
                                        )),
                                )
                                .child(self.copyable(text.clone(), cx)),
                        )
                    })
                    .when(more > 0, |body| {
                        body.child(
                            ui::hint(format!("{more} more lines are on the computer."), &colors)
                                .px(px(20.))
                                .pt(px(12.)),
                        )
                    })
            }
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                ui::appbar(
                    ui::tap("back-file", "back", &colors).on_click(cx.listener(
                        |this, _, window, cx| {
                            this.back(window, cx);
                        },
                    )),
                    view.name().to_owned(),
                    Some(subtitle.into()),
                    &colors,
                )
                .child(self.wrap_toggle("wrap-file", cx))
                .children(text.clone().map(|text| {
                    ui::tap("copy-file", "copy", &colors)
                        .aria_label("Copy the file")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy(text.clone(), "the file", cx)
                        }))
                })),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(20.))
                    .pt(px(4.))
                    .pb(px(12.))
                    .flex()
                    .gap(px(12.))
                    .text_size(px(12.5))
                    .line_height(px(16.))
                    .text_color(colors.muted)
                    .child(div().flex_1().min_w_0().truncate().child(meta))
                    .children(edited_note.map(|note| div().flex_none().child(note))),
            )
            .child(
                scroll_area("file", &scroll)
                    .border_t_1()
                    .border_color(colors.line)
                    .child(body),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(12.))
                    .pt(px(8.))
                    .pb(px(12.))
                    .flex()
                    .gap(px(8.))
                    .child({
                        let button = ui::button(
                            "file-history",
                            Button::Plain,
                            Some("restore"),
                            "File history",
                            false,
                            &colors,
                        )
                        .flex_1();
                        match history {
                            Some(id) => button.on_click(cx.listener(move |this, _, window, cx| {
                                this.open_history(id, window, cx)
                            })),
                            None => ui::disabled(button, &colors),
                        }
                    })
                    .child(
                        ui::button(
                            "ask-about-file",
                            Button::Primary,
                            Some("plus"),
                            "Ask about it",
                            false,
                            &colors,
                        )
                        .flex_1()
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.use_folder(&root, cx);
                                let mention = mention.clone();
                                this.start.update(cx, |composer, cx| {
                                    composer.set_text(&mention, cx);
                                    composer.focus(window, cx);
                                });
                            },
                        )),
                    ),
            )
    }
}
