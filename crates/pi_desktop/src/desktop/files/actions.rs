//! File-browser interactions. Mutations are executed by pi_editor; this module
//! owns no filesystem access and never writes through the session RPC process.
use super::*;
use pi_editor::FileAction;
use ui::{ContextMenu, Tooltip, right_click_menu};

gpui::actions!(file_browser, [ConfirmMutation, CancelMutation]);
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MutationKind {
    File,
    Folder,
    Rename,
    Trash,
}
impl MutationKind {
    fn title(self) -> &'static str {
        match self {
            Self::File => "New File",
            Self::Folder => "New Folder",
            Self::Rename => "Rename",
            Self::Trash => "Move to Trash",
        }
    }
}
#[derive(Clone)]
pub(super) struct Mutation {
    kind: MutationKind,
    path: PathBuf,
}
impl FilesView {
    pub(super) fn mutation_enabled(&self) -> bool {
        !self.demo && self.host.is_some() && !self.browser_loading && !self.mutating
    }
    fn parent_for_new(&self) -> PathBuf {
        self.selected_entry
            .as_ref()
            .map(|path| {
                if self.entries.iter().any(|e| &e.path == path && e.directory) {
                    path.clone()
                } else {
                    path.parent().unwrap_or(&self.root).to_owned()
                }
            })
            .unwrap_or_else(|| self.root.clone())
    }
    pub(super) fn begin_mutation(
        &mut self,
        kind: MutationKind,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.mutation_enabled() {
            return;
        }
        let path = match kind {
            MutationKind::File | MutationKind::Folder => {
                path.unwrap_or_else(|| self.parent_for_new())
            }
            _ => {
                let Some(path) = path.or_else(|| self.selected_entry.clone()) else {
                    return;
                };
                path
            }
        };
        if matches!(kind, MutationKind::Rename | MutationKind::Trash)
            && pi_editor::has_unsaved_buffers(cx)
        {
            self.error =
                Some("Save or close unsaved editor buffers before renaming or deleting.".into());
            cx.notify();
            return;
        }
        let name = if kind == MutationKind::Rename {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        } else {
            String::new()
        };
        self.name_input
            .update(cx, |input, cx| input.set_content(name, cx));
        if matches!(kind, MutationKind::Rename | MutationKind::Trash) {
            self.selected_entry = Some(path.clone());
        }
        self.mutation = Some(Mutation { kind, path });
        self.error = None;
        if kind != MutationKind::Trash {
            self.name_input.focus_handle(cx).focus(window, cx);
        } else {
            self.mutation_focus.focus(window, cx);
        }
        cx.notify();
    }
    fn cancel_mutation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mutating {
            return;
        }
        self.mutation = None;
        self.error = None;
        self.filter.focus_handle(cx).focus(window, cx);
        cx.notify();
    }
    fn apply_mutation(&mut self, cx: &mut Context<Self>) {
        if !self.mutation_enabled() {
            return;
        }
        let Some(mutation) = self.mutation.clone() else {
            return;
        };
        let name = self.name_input.read(cx).content().trim().to_owned();
        let action = match mutation.kind {
            MutationKind::File | MutationKind::Folder => FileAction::Create {
                parent: mutation.path.clone(),
                name,
                directory: mutation.kind == MutationKind::Folder,
            },
            MutationKind::Rename => FileAction::Rename {
                path: mutation.path.clone(),
                name,
            },
            MutationKind::Trash => FileAction::Trash {
                path: mutation.path.clone(),
            },
        };
        let task = self.host.as_ref().unwrap().file_action(action, cx);
        self.mutating = true;
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.mutating = false;
                match result {
                    Ok(path) => {
                        this.mutation = None;
                        this.error = None;
                        match mutation.kind {
                            MutationKind::File => {
                                this.selected_entry = Some(path.clone());
                                this.open(path, cx);
                            }
                            MutationKind::Folder => {
                                this.collapsed.remove(&mutation.path);
                                this.selected_entry = Some(path);
                            }
                            MutationKind::Rename => this.entry_renamed(&mutation.path, &path, cx),
                            MutationKind::Trash => {
                                // Closed buffers were clean at confirmation. Keep a buffer
                                // if it became dirty while the filesystem task was running.
                                this.tabs.retain(|t| {
                                    !t.path.starts_with(&path) || t.buffer.read(cx).is_dirty()
                                });
                                if this.active.as_ref().is_some_and(|p| p.starts_with(&path)) {
                                    this.active = this.tabs.last().map(|t| t.path.clone());
                                }
                                this.selected_entry = None;
                                cx.emit(FileEvent::Tabs);
                                // Keep FILES visible after deleting the last open file.
                            }
                        }
                        this.refresh_entries(cx);
                    }
                    Err(error) => {
                        this.error = Some(format!("{error:#}"));
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
    pub(super) fn entry_renamed(
        &mut self,
        old: &std::path::Path,
        new: &std::path::Path,
        cx: &mut Context<Self>,
    ) {
        for tab in &mut self.tabs {
            if let Ok(relative) = tab.path.strip_prefix(old) {
                tab.path = if relative.as_os_str().is_empty() {
                    new.to_owned()
                } else {
                    new.join(relative)
                };
            }
        }
        for path in [&mut self.active, &mut self.selected_entry] {
            if let Some(old_path) = path.as_ref()
                && let Ok(relative) = old_path.strip_prefix(old)
            {
                *path = Some(if relative.as_os_str().is_empty() {
                    new.to_owned()
                } else {
                    new.join(relative)
                });
            }
        }
        self.collapsed = self
            .collapsed
            .iter()
            .map(|p| {
                p.strip_prefix(old)
                    .map(|r| {
                        if r.as_os_str().is_empty() {
                            new.to_owned()
                        } else {
                            new.join(r)
                        }
                    })
                    .unwrap_or_else(|_| p.clone())
            })
            .collect();
        cx.emit(FileEvent::Tabs);
        cx.notify();
    }
    pub(super) fn mutation_form(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(mutation) = self.mutation.clone() else {
            return div().into_any_element();
        };
        let theme = theme(cx);
        v_flex().id("file-mutation").track_focus(&self.mutation_focus).key_context("FileMutation").debug_selector(||"file-mutation".into()).p(px(10.)).gap(px(8.)).rounded(px(6.)).border_1().border_color(theme.focus).bg(theme.canvas)
            .on_action(cx.listener(|this,_:&ConfirmMutation,_,cx|{this.apply_mutation(cx);cx.stop_propagation();}))
            .on_action(cx.listener(|this,_:&CancelMutation,window,cx|{this.cancel_mutation(window,cx);cx.stop_propagation();}))
            .child(label(mutation.kind.title().to_uppercase(),theme))
            .child(div().font_family(MONO).text_size(px(10.)).child(mutation.path.display().to_string()))
            .when(mutation.kind==MutationKind::Trash,|v|v.child(note("Move this item and any contents to the system Trash? It can be restored there. Unsaved buffers must be saved or closed first.",theme)))
            .when(mutation.kind!=MutationKind::Trash,|v|v.child(input_box(self.name_input.clone(),theme).when(self.mutating,|input|input.opacity(0.5))))
            .child(h_flex().gap(px(8.))
                .child(primary_button("apply-file-mutation",if self.mutating{"Working…"}else if mutation.kind==MutationKind::Trash{"Move to Trash"}else{"Save"},!self.mutating,theme).debug_selector(||"apply-file-mutation".into()).on_click(cx.listener(|this,_,_,cx|this.apply_mutation(cx))))
                .child(button("cancel-file-mutation","Cancel",theme).on_click(cx.listener(|this,_,window,cx|this.cancel_mutation(window,cx)))))
            .into_any_element()
    }
    pub(super) fn file_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme(cx);
        let enabled = self.mutation_enabled();
        h_flex()
            .gap(px(3.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(26.))
                    .px(px(8.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme.chip_line)
                    .bg(theme.canvas)
                    .child(self.filter.clone()),
            )
            .children(
                [
                    ("new-file", "file_add", "New file", MutationKind::File),
                    (
                        "new-folder",
                        "folder_add",
                        "New folder",
                        MutationKind::Folder,
                    ),
                ]
                .into_iter()
                .map(|(id, icon_name, title, kind)| {
                    icon_button(id, icon_name, title, theme)
                        .debug_selector(move || id.into())
                        .tooltip(Tooltip::text(if self.demo {
                            "Unavailable in offline preview"
                        } else {
                            title
                        }))
                        .when(!enabled, |b| b.opacity(0.4).cursor_default())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.begin_mutation(kind, None, window, cx)
                        }))
                }),
            )
            .child(
                icon_button("collapse-files", "list_collapse", "Collapse folders", theme)
                    .tooltip(Tooltip::text("Collapse all folders"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.collapsed = this
                            .entries
                            .iter()
                            .filter(|e| e.directory)
                            .map(|e| e.path.clone())
                            .collect();
                        this.filter_rows(cx);
                    })),
            )
            .into_any_element()
    }
    pub(super) fn file_row(
        &self,
        index: usize,
        cx: &mut Context<Self>,
        theme: Theme,
    ) -> AnyElement {
        let entry = self.rows[index].clone();
        let menu_entry = entry.clone();
        let weak = cx.entity().downgrade();
        let depth = entry.relative.matches('/').count().min(8);
        let collapsed = self.collapsed.contains(&entry.path);
        let row = h_flex()
            .id(("project-file", index))
            .debug_selector(move || format!("project-file-{index}"))
            .h(px(26.))
            .w_full()
            .pl(px(depth as f32 * 12.))
            .pr(px(6.))
            .gap(px(4.))
            .cursor_pointer()
            .rounded(px(3.))
            .hover(move |s| s.bg(theme.hover))
            .when(self.selected_entry.as_ref() == Some(&entry.path), |v| {
                v.bg(theme.selected)
            })
            .child(div().w(px(12.)).flex_shrink_0().when(entry.directory, |v| {
                v.child(
                    icon(
                        if collapsed {
                            "chevron_right"
                        } else {
                            "chevron_down"
                        },
                        theme.faint,
                    )
                    .size(px(10.)),
                )
            }))
            .child(icon(if entry.directory { "folder" } else { "file" }, theme.muted).size(px(13.)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.))
                    .truncate()
                    .child(
                        entry
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                    ),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected_entry = Some(entry.path.clone());
                if entry.directory {
                    if !this.collapsed.remove(&entry.path) {
                        this.collapsed.insert(entry.path.clone());
                    }
                    this.filter_rows(cx);
                } else {
                    this.open(entry.path.clone(), cx);
                }
            }))
            .into_any_element();
        let enabled = self.mutation_enabled();
        right_click_menu(("file-menu", index))
            .trigger(move |_, _, _| row)
            .menu(move |window, cx| {
                // Focus only after RightClickMenu accepts the hit. Changing focus
                // in the trigger's mouse-down can invalidate its hover hitbox.
                weak.update(cx, |this, cx| this.browser_focus.focus(window, cx))
                    .ok();
                let entry = menu_entry.clone();
                let weak = weak.clone();
                ContextMenu::build(window, cx, move |mut menu, _, _| {
                    let parent = if entry.directory {
                        entry.path.clone()
                    } else {
                        entry.path.parent().unwrap().to_owned()
                    };
                    if enabled {
                        for kind in [
                            MutationKind::File,
                            MutationKind::Folder,
                            MutationKind::Rename,
                        ] {
                            let weak = weak.clone();
                            let path = if kind == MutationKind::Rename {
                                entry.path.clone()
                            } else {
                                parent.clone()
                            };
                            menu = menu.entry(kind.title(), None, move |window, cx| {
                                weak.update(cx, |this, cx| {
                                    this.begin_mutation(kind, Some(path.clone()), window, cx)
                                })
                                .ok();
                            });
                        }
                        menu = menu.separator();
                    }
                    let path = entry.path.clone();
                    menu = menu.entry("Copy Path", None, move |_, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(path.display().to_string()))
                    });
                    let path = entry.path.clone();
                    menu = menu.entry("Reveal in file manager", None, move |_, cx| {
                        cx.reveal_path(&path)
                    });
                    if enabled {
                        menu = menu.separator().entry("Delete…", None, move |window, cx| {
                            weak.update(cx, |this, cx| {
                                this.begin_mutation(
                                    MutationKind::Trash,
                                    Some(entry.path.clone()),
                                    window,
                                    cx,
                                )
                            })
                            .ok();
                        });
                    }
                    menu
                })
            })
            .into_any_element()
    }
}
