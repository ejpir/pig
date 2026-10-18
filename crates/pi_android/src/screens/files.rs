//! A retained mobile source workspace: tabs, Tree-sitter breadcrumbs and
//! structure, an IME-correct editor, and explicit optimistic saves.

use crate::{
    app::{PhoneApp, Sheet, size_label},
    projects::{FileRecoveryState, FileSaveState, Load, language_for_path},
    theme::{MONO, Theme, theme},
    ui::{self, Button},
};
use gpui::{
    Context, Div, FontWeight, IntoElement, Role, SharedString, Window, div, prelude::*, px,
};
use pi_core::remote_files::RecoveryArtifact;

/// Shared by change/review rendering.
pub(crate) fn language(name: &str) -> Option<&'static str> {
    language_for_path(name)
}

impl PhoneApp {
    pub(crate) fn file_screen(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = theme(cx);
        if self.editor_root.is_none() {
            self.editor_root = self.file_view.as_ref().map(|view| view.root.clone());
        }
        let Some(view) = &self.file_view else {
            let subtitle = self
                .editor_root
                .as_ref()
                .map(|root| self.short_path(root).into());
            return div()
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
                        "Editor",
                        subtitle,
                        &colors,
                    )
                    .child(
                        ui::tap("file-drawer", "folder", &colors)
                            .debug_selector(|| "file-drawer".into())
                            .aria_label("Open files in the editor folder")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_sheet(Sheet::EditorFiles, cx)
                            })),
                    ),
                )
                .child(
                    div()
                        .id("empty-editor")
                        .debug_selector(|| "empty-editor".into())
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(10.))
                        .px(px(32.))
                        .text_center()
                        .child(ui::icon("folder", 28., colors.muted))
                        .child(
                            div()
                                .text_size(px(17.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Choose a file"),
                        )
                        .child(ui::hint(
                            "Open a file from this editor folder to begin.",
                            &colors,
                        )),
                );
        };

        let root = view.root.clone();
        let path = view.path.clone();
        let name = view.name().to_owned();
        let project = root.rsplit('/').next().unwrap_or_default().to_owned();
        let editor = view.editor.clone();
        let dirty = view.dirty(cx);
        let editable = view.editable;
        let blocked = view.saving_blocked();
        let reason = view.read_only_reason.clone();
        let save_state = view.save.clone();
        let load_error = match &view.text {
            Load::Failed(error) => Some(error.clone()),
            _ => None,
        };
        let loading = matches!(view.text, Load::Loading);
        let (line, column) = editor.read(cx).line_column();
        let lines = editor.read(cx).line_count();
        let language = language_for_path(&path).unwrap_or("Plain text");
        let bytes = view.size.unwrap_or(editor.read(cx).text().len() as u64);
        let caret = editor.read(cx).selection().end;
        let breadcrumbs = view.syntax.as_ref().map_or_else(Vec::new, |syntax| {
            syntax
                .breadcrumbs(caret)
                .into_iter()
                .filter(|node| node.kind != "program" && node.kind != "source_file")
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(|node| node.label)
                .collect()
        });

        let mut tabs = self
            .file_tabs
            .iter()
            .map(|tab| {
                (
                    tab.root.clone(),
                    tab.path.clone(),
                    tab.name().to_owned(),
                    tab.dirty(cx),
                    false,
                    tab.request,
                )
            })
            .collect::<Vec<_>>();
        tabs.push((
            root.clone(),
            path.clone(),
            name.clone(),
            dirty,
            true,
            view.request,
        ));
        tabs.sort_by_key(|(_, _, _, _, _, opened)| *opened);

        let (save_icon, save_label, save_attention, save_color) = match &save_state {
            _ if loading => ("refresh", "Loading file", false, colors.muted),
            _ if !editable => ("eye", "Read-only file", false, colors.muted),
            FileSaveState::Saving => ("refresh", "Saving file", false, colors.muted),
            FileSaveState::Failed(_) => ("alert", "Retry failed save", true, colors.coral),
            FileSaveState::Uncertain(_) | FileSaveState::Conflict(_) => {
                ("alert", "Save needs attention", true, colors.amber)
            }
            _ if dirty => ("save", "Save unsaved changes", false, colors.accent),
            _ => ("check", "File saved", false, colors.green),
        };
        let mut save_button = ui::tap_colored("save-file", save_icon, save_color, &colors)
            .debug_selector(|| "save-file".into())
            .aria_label(save_label);
        if dirty && !blocked {
            save_button = save_button.bg(colors.selected);
        } else if save_attention {
            save_button = save_button.bg(colors.raised);
        }
        let save_button = if editable && dirty && !blocked {
            save_button.on_click(cx.listener(|this, _, _, cx| this.save_active_file(cx)))
        } else {
            ui::disabled(save_button, &colors)
        };
        let subtitle: SharedString = if breadcrumbs.is_empty() {
            format!("{project} · {}", self.short_path(&root)).into()
        } else {
            breadcrumbs.join(" › ").into()
        };
        let recovery_button = ui::tap("file-recoveries", "history", &colors)
            .debug_selector(|| "file-recoveries".into())
            .aria_label("File recovery records");
        let recovery_button = if editable {
            recovery_button.on_click(cx.listener(|this, _, _, cx| this.open_file_recoveries(cx)))
        } else {
            ui::disabled(recovery_button, &colors)
        };
        let file_actions = div()
            .flex()
            .items_center()
            .child(
                ui::tap("file-drawer", "folder", &colors)
                    .debug_selector(|| "file-drawer".into())
                    .aria_label("Open files in the editor folder")
                    .on_click(
                        cx.listener(|this, _, _, cx| this.open_sheet(Sheet::EditorFiles, cx)),
                    ),
            )
            .child(
                ui::tap("expand-syntax", "maximize", &colors)
                    .debug_selector(|| "expand-syntax".into())
                    .aria_label("Expand syntax selection")
                    .on_click(cx.listener(|this, _, _, cx| this.expand_file_selection(cx))),
            )
            .child(
                ui::tap("file-structure", "layers", &colors)
                    .debug_selector(|| "file-structure".into())
                    .aria_label("File structure")
                    .on_click(
                        cx.listener(|this, _, _, cx| this.open_sheet(Sheet::FileStructure, cx)),
                    ),
            )
            .child(recovery_button)
            .child(save_button);

        let notice = self.file_save_notice(&save_state, reason, load_error, &colors, cx);

        let editor_body = if loading {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .child(ui::working_indicator(&colors))
                .child(ui::hint("Reading the file…", &colors))
                .into_any_element()
        } else {
            div()
                .id("file-editor")
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .px(px(12.))
                .py(px(8.))
                .bg(colors.panel)
                .font_family(MONO)
                .text_size(px(13.))
                .line_height(px(21.))
                .child(editor)
                .into_any_element()
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
                    name,
                    Some(subtitle),
                    &colors,
                )
                .child(file_actions),
            )
            .child(
                div()
                    .id("file-tabs")
                    .h(px(48.))
                    .flex_none()
                    .flex()
                    .items_end()
                    .overflow_x_scroll()
                    .border_b_1()
                    .border_color(colors.line)
                    .children(tabs.into_iter().map(
                        |(tab_root, tab_path, tab_name, tab_dirty, active, _)| {
                            let click_root = tab_root.clone();
                            let click_path = tab_path.clone();
                            div()
                                .id(SharedString::from(format!(
                                    "file-tab:{tab_root}:{tab_path}"
                                )))
                                .role(Role::Button)
                                .aria_label(SharedString::from(format!(
                                    "{} file tab{}{}",
                                    tab_name,
                                    if active { ", selected" } else { "" },
                                    if tab_dirty { ", unsaved" } else { "" }
                                )))
                                .h(px(47.))
                                .px(px(14.))
                                .flex()
                                .items_center()
                                .gap(px(7.))
                                .border_b_2()
                                .border_color(if active {
                                    colors.accent
                                } else {
                                    colors.line.opacity(0.)
                                })
                                .text_size(px(12.5))
                                .font_weight(if active {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(if active { colors.text } else { colors.muted })
                                .child(tab_name)
                                .children(
                                    tab_dirty.then(|| {
                                        div().size(px(6.)).rounded_full().bg(colors.amber)
                                    }),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.activate_file_tab(&click_root, &click_path, cx)
                                }))
                        },
                    )),
            )
            .children(notice)
            .child(editor_body)
            .child(
                div()
                    .h(px(36.))
                    .flex_none()
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .border_t_1()
                    .border_color(colors.line)
                    .text_size(px(11.5))
                    .text_color(colors.muted)
                    .child(format!("Ln {line}, Col {column}"))
                    .child(format!("{lines} lines"))
                    .child(div().flex_1())
                    .child(language)
                    .child(size_label(bytes))
                    .child(if editable { "UTF-8" } else { "Read-only" }),
            )
    }

    fn file_save_notice(
        &self,
        state: &FileSaveState,
        read_only: Option<String>,
        load_error: Option<String>,
        colors: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        if let Some(error) = load_error {
            return Some(
                file_banner(error, colors.coral, colors).child(
                    ui::button(
                        "retry-file-read",
                        Button::Plain,
                        Some("refresh"),
                        "Retry",
                        true,
                        colors,
                    )
                    .flex_none()
                    .on_click(cx.listener(|this, _, _, cx| this.retry_active_file(cx))),
                ),
            );
        }
        if let Some(reason) = read_only {
            return Some(file_banner(reason, colors.amber, colors));
        }
        match state {
            FileSaveState::Idle | FileSaveState::Saving => None,
            FileSaveState::Failed(error) => Some(file_banner(error.clone(), colors.coral, colors)),
            FileSaveState::Uncertain(error) => Some(
                file_banner(error.clone(), colors.amber, colors).child(
                    ui::button(
                        "check-file-save",
                        Button::Plain,
                        None,
                        "Check",
                        true,
                        colors,
                    )
                    .flex_none()
                    .on_click(cx.listener(|this, _, _, cx| this.check_active_file_save(cx))),
                ),
            ),
            FileSaveState::Conflict(error) => Some(
                file_banner(error.clone(), colors.coral, colors)
                    .child(
                        ui::button(
                            "reload-conflict",
                            Button::Plain,
                            None,
                            "Reload",
                            true,
                            colors,
                        )
                        .flex_none()
                        .on_click(cx.listener(|this, _, _, cx| this.reload_conflicted_file(cx))),
                    )
                    .child(
                        ui::button(
                            "keep-local-conflict",
                            Button::Plain,
                            None,
                            "Keep mine",
                            true,
                            colors,
                        )
                        .flex_none()
                        .on_click(cx.listener(|this, _, _, cx| this.keep_local_file(cx))),
                    ),
            ),
        }
    }

    pub(crate) fn file_recoveries_sheet(&mut self, colors: &Theme, cx: &mut Context<Self>) -> Div {
        let records = match &self.file_recoveries {
            FileRecoveryState::Idle | FileRecoveryState::Loading => {
                return div()
                    .pb(px(20.))
                    .child(ui::working_indicator(colors))
                    .child(ui::hint("Loading recovery records…", colors));
            }
            FileRecoveryState::Failed(error) => {
                return div()
                    .pb(px(20.))
                    .child(ui::hint(error.clone(), colors))
                    .child(
                        ui::button(
                            "retry-file-recoveries",
                            Button::Plain,
                            Some("refresh"),
                            "Retry",
                            true,
                            colors,
                        )
                        .mt(px(12.))
                        .on_click(cx.listener(|this, _, _, cx| this.open_file_recoveries(cx))),
                    );
            }
            FileRecoveryState::Loaded(records) => records.clone(),
        };
        if records.is_empty() {
            return div().pb(px(20.)).child(ui::hint(
                "No retained saves for this editor folder.",
                colors,
            ));
        }
        div()
            .pb(px(16.))
            .child(ui::hint(
                "Each save retains the original, commit-boundary, installed and phone-draft files. Review every useful artifact before deleting its record; deletion cannot be undone.",
                colors,
            ))
            .children(records.into_iter().enumerate().map(|(index, record)| {
                let armed = self.file_recovery_delete_armed.as_deref() == Some(&record.id);
                let mut row = div()
                    .id(("file-recovery", index))
                    .mt(px(12.))
                    .p(px(10.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(colors.line)
                    .bg(colors.panel)
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(12.5))
                            .text_color(colors.text)
                            .child(record.path.clone()),
                    )
                    .child(
                        div()
                            .mt(px(3.))
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .child(format!("{} · {}", record.state, record.id)),
                    );
                let artifacts = [
                    ("Original", RecoveryArtifact::Original),
                    ("Boundary", RecoveryArtifact::Boundary),
                    ("Installed", RecoveryArtifact::Installed),
                    ("Draft", RecoveryArtifact::Draft),
                ];
                row = row.child(
                    div().mt(px(8.)).flex().flex_wrap().gap(px(6.)).children(
                        artifacts
                            .into_iter()
                            .enumerate()
                            .map(|(artifact_index, (label, artifact))| {
                            let id = record.id.clone();
                            ui::button(
                                format!("file-recovery-read-{index}-{artifact_index}"),
                                Button::Plain,
                                None,
                                label,
                                true,
                                colors,
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.read_file_recovery(id.clone(), artifact, cx)
                            }))
                        }),
                    ),
                );
                let id = record.id;
                row.child(
                    ui::button(
                        ("file-recovery-delete", index),
                        Button::Plain,
                        None,
                        if armed { "Confirm delete" } else { "Delete record…" },
                        true,
                        colors,
                    )
                    .mt(px(8.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.delete_file_recovery(id.clone(), cx)
                    })),
                )
            }))
    }

    pub(crate) fn file_structure_sheet(&mut self, colors: &Theme, cx: &mut Context<Self>) -> Div {
        let Some(view) = &self.file_view else {
            return div();
        };
        let editor = view.editor.clone();
        let items = view
            .syntax
            .as_ref()
            .map_or_else(Vec::new, |syntax| syntax.outline());
        if items.is_empty() {
            return div().pb(px(20.)).child(ui::hint(
                "This language has no structure items in the current file.",
                colors,
            ));
        }
        div()
            .pb(px(12.))
            .children(items.into_iter().enumerate().map(move |(index, item)| {
                let range = item.selection_range.clone();
                let editor = editor.clone();
                div()
                    .id(("file-symbol", index))
                    .role(Role::Button)
                    .aria_label(SharedString::from(format!("{} {}", item.kind, item.label)))
                    .min_h(px(48.))
                    .pl(px(8. + item.depth.min(6) as f32 * 14.))
                    .pr(px(8.))
                    .py(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(10.))
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(12.5))
                            .text_color(colors.text)
                            .child(item.label),
                    )
                    .child(
                        div()
                            .ml_auto()
                            .text_size(px(11.))
                            .text_color(colors.faint)
                            .child(item.kind),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        editor.update(cx, |editor, cx| editor.select_range(range.clone(), cx));
                        this.close_sheet(cx);
                    }))
            }))
    }
}

fn file_banner(text: String, accent: gpui::Hsla, colors: &Theme) -> Div {
    div()
        .flex_none()
        .min_h(px(44.))
        .px(px(12.))
        .py(px(8.))
        .flex()
        .items_center()
        .gap(px(10.))
        .border_b_1()
        .border_color(colors.line)
        .bg(colors.panel)
        .text_size(px(12.))
        .text_color(accent)
        .child(div().flex_1().min_w_0().child(text))
}
