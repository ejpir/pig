//! Main-area review: the inspector is supplementary, never required for actions.
use super::*;

impl ChangesView {
    fn review_toolbar(&self, cx: &Context<Self>, theme: Theme) -> AnyElement {
        let Some(file) = self.file() else {
            return div().into_any_element();
        };
        let path = PathBuf::from(&file.path);
        let split = self.diff.read(cx).is_split();
        let unified = self.diff.read(cx).unified.clone();
        let wrapped = unified.read(cx).is_wrapped();
        v_flex().gap(px(16.)).mb(px(26.))
            .child(h_flex().relative().top(px(-6.)).items_start().gap(px(12.)).flex_wrap()
                .child(v_flex().flex_1().min_w(px(160.)).gap(px(4.))
                    .child(div().truncate().text_size(px(19.)).font_weight(FontWeight::SEMIBOLD).child(
                        path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned()))
                    .child(div().truncate().font_family(MONO).text_size(px(12.)).text_color(theme.muted)
                        .child(path.parent().unwrap_or(std::path::Path::new("")).display().to_string())))
                .when(self.diff.read(cx).can_split(), |v| v.child(
                    work_button("changes-diff-mode", if split { "Split diff ⌄" } else { "Unified diff ⌄" }, theme)
                        .w(px(101.))
                        .justify_center()
                        .debug_selector(|| "changes-diff-mode".into())
                        .tooltip(ui::Tooltip::text("Switch between aligned split and unified diff"))
                        .on_click(cx.listener(|this,_,_,cx| this.diff.update(cx, |d,cx| d.toggle(cx))))))
                .when(!split, |v| v.child(
                    work_button("changes-wrap", if wrapped { "Wrap: on" } else { "Wrap: off" }, theme)
                        .debug_selector(|| "changes-wrap".into())
                        .on_click(move |_,_,cx| unified.update(cx, |d,cx| d.toggle_wrap(cx)))))
                .child(work_button("changes-open", "Open file", theme)
                    .w(px(105.))
                    .justify_center()
                    .debug_selector(|| "changes-open".into())
                    .on_click(cx.listener(|this,_,_,cx| this.open(cx)))))
            .child(work_notice(if file.turn.is_none() {
                    "Observed edit, already on disk · not the full working tree · no snapshot to restore".to_owned()
                } else {
                    format!("{} · immutable snapshot, not today's working tree", file.source)
                }, file.turn.is_none(), theme).debug_selector(|| "tool-reported-explanation".into()))
            .into_any_element()
    }
    fn review_actions(&self, cx: &Context<Self>, theme: Theme) -> AnyElement {
        let Some(file) = self.file() else {
            return div().into_any_element();
        };
        if self
            .composer
            .read(cx)
            .revision
            .as_ref()
            .is_some_and(|revision| revision.path == file.path)
        {
            return div().into_any_element();
        }
        let patch = file.patch.clone();
        h_flex()
            .gap(px(8.))
            .flex_wrap()
            .mt(px(12.))
            .child(
                work_button("changes-request-revision", "Request revision", theme)
                    .debug_selector(|| "changes-request-revision".into())
                    .tooltip(ui::Tooltip::text(
                        "Attach this file to the draft below. Nothing is sent yet.",
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.request_revision(cx))),
            )
            .child(
                work_button("changes-copy", "Copy diff", theme)
                    .debug_selector(|| "changes-copy".into())
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(patch.clone()))
                    }),
            )
            .when(file.turn.is_some(), |v| {
                let (action, enabled) = self.turn_action(cx);
                v.child(
                    work_primary(
                        "changes-restore",
                        "Restore file",
                        self.can_restore(cx),
                        theme,
                    )
                    .debug_selector(|| "changes-restore".into())
                    .on_click(cx.listener(|this, _, window, cx| this.restore_file(window, cx))),
                )
                .child(
                    work_primary("changes-undo", action, enabled, theme)
                        .on_click(cx.listener(|this, _, _, cx| this.undo(cx))),
                )
            })
            .into_any_element()
    }
    fn compact_picker(&self, cx: &Context<Self>, theme: Theme) -> AnyElement {
        v_flex()
            .relative()
            .mx(WORK_GUTTER)
            .mt(px(14.))
            .mb(px(4.))
            .child(
                work_button(
                    "changes-file-picker",
                    self.file()
                        .map(|f| f.path.as_str())
                        .unwrap_or("Choose a file"),
                    theme,
                )
                .debug_selector(|| "changes-file-picker".into())
                .min_w_0()
                .overflow_hidden()
                .aria_expanded(self.picker_open)
                .child(icon("chevron_down", theme.muted).size(px(12.)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.picker_open = !this.picker_open;
                    cx.notify();
                })),
            )
            .when(self.picker_open, |v| {
                v.child(
                    v_flex()
                        .id("changes-file-options")
                        .max_h(px(180.))
                        .overflow_y_scroll()
                        .border_1()
                        .border_color(theme.line)
                        .bg(theme.panel)
                        .children(self.files.iter().enumerate().map(|(index, file)| {
                            changed_file_row(
                                ("review-file-option", index),
                                &file.path,
                                file.added,
                                file.removed,
                                self.selected == Some(index),
                                theme,
                            )
                            .debug_selector(move || format!("review-file-option-{index}"))
                            .on_click(cx.listener(move |this, _, _, cx| this.select(index, cx)))
                        })),
                )
            })
            .into_any_element()
    }
}
impl Render for ChangesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        if self.tab == Tab::Operations {
            let can = self.controller.read(cx).jj_idle() && !pi_editor::has_unsaved_buffers(cx);
            return v_flex()
                .size_full()
                .min_h_0()
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .child(self.operations_view(window, cx)),
                )
                .when(self.operation.is_some(), |v| {
                    v.child(
                        work_primary(
                            "restore-operation-main",
                            "Restore project to selected operation…",
                            can,
                            theme,
                        )
                        .mx(WORK_GUTTER)
                        .mb(px(12.))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.restore_operation(window, cx)),
                        ),
                    )
                })
                .into_any_element();
        }
        let weak = cx.entity().downgrade();
        let rows = gpui::list(self.file_list.clone(), move |index, _, cx| {
            weak.update(cx, |this, cx| this.row(index, cx, theme))
                .unwrap_or_else(|_| div().into_any_element())
        })
        .size_full();
        let has_operations = self.controller.read(cx).jj().project.is_some();
        let main_folder = self
            .controller
            .read(cx)
            .main_folder()
            .map(|p| p.display().to_string());
        let workbench_demo =
            self.controller.read(cx).model().state.session_id.as_deref() == Some("demo-workbench");
        let revision_attached = self.file().is_some_and(|file| {
            self.composer
                .read(cx)
                .revision
                .as_ref()
                .is_some_and(|r| r.path == file.path)
        });
        let content = v_flex().id("changes-scroll").debug_selector(||"changes-scroll".into())
            .track_scroll(&self.scroll).size_full().overflow_y_scroll().p(WORK_GUTTER)
            .child(self.review_toolbar(cx,theme))
            .when(self.file().is_some(), |v|v.child(
                div().debug_selector(|| "changes-document".into()).w_full().min_w_0().child(self.diff.clone())))
            .when_some(self.file(), |v,file|v
                .child(
                    v_flex()
                        .mt(px(24.))
                        .gap(px(11.))
                        .text_color(theme.muted)
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(theme.secondary)
                                .child(if workbench_demo {
                                    "Only the provider guard changes. The test file covers the empty-signature case."
                                } else if file.turn.is_some() {
                                    "Old/new positions come from the recorded hunks."
                                } else {
                                    "Only the selected file's observed report is shown. Other edits may be missing."
                                }),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .child(if workbench_demo {
                                    "Select lines to include them in a revision request."
                                } else if file.turn.is_some() {
                                    "Select lines or attach the file to request a revision; its snapshot can also be restored."
                                } else {
                                    "Select lines or attach the file to request a revision. Nothing is sent until you submit."
                                }),
                        ),
                )
                .child(self.review_actions(cx,theme))
                .when(!revision_attached,|v|v.children(file.touches.iter().enumerate().map(|(index,(id,name))| {
                    let tool_id = id.clone();
                    work_button(("review-tool",index),format!("{name} · {}",data::short_call_id(id)),theme)
                        .debug_selector(move || format!("change-tool-{index}"))
                        .mt(px(8.)).min_w_0().overflow_hidden()
                        .tooltip(ui::Tooltip::text(id.clone()))
                        .on_click(cx.listener(move |this,_,_,cx| this.controller.update(cx,|c,cx|c.reveal_tool(tool_id.clone(),cx))))
                }))))
            .when(self.files.is_empty(), |v|v.child(empty("No recorded changes",
                "Future snapshots and successful edit/write calls will appear here. Earlier work cannot be reconstructed without a snapshot.",theme)));
        h_flex()
            .id("changes-view")
            .debug_selector(|| "changes-view".into())
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &NextChoice, window, cx| {
                if this.focus.is_focused(window) {
                    cx.stop_propagation();
                    this.move_file(1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PreviousChoice, window, cx| {
                if this.focus.is_focused(window) {
                    cx.stop_propagation();
                    this.move_file(-1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Submit, window, cx| {
                if this.focus.is_focused(window) {
                    cx.stop_propagation();
                    this.request_revision(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Stop, _, cx| {
                if this.picker_open {
                    cx.stop_propagation();
                    this.picker_open = false;
                    cx.notify();
                }
            }))
            .size_full()
            .items_stretch()
            .min_h_0()
            .when(self.wide, |v| {
                v.child(
                    v_flex()
                        .w(px(200.))
                        .h_full()
                        .flex_shrink_0()
                        .p(px(8.))
                        .gap(px(3.))
                        .border_r_1()
                        .border_color(theme.line)
                        .child(
                            div()
                                .px(px(12.))
                                .py(px(10.))
                                .text_size(px(12.))
                                .text_color(theme.muted)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!(
                                    "{} files · {}",
                                    if self.files.iter().all(|f| f.turn.is_none()) {
                                        "Observed"
                                    } else {
                                        "Review"
                                    },
                                    self.files.len()
                                )),
                        )
                        .child(
                            div().flex_1().min_h_0().child(rows).custom_scrollbars(
                                Scrollbars::new(ScrollAxes::Vertical)
                                    .id("review-files-scrollbar")
                                    .tracked_scroll_handle(&self.file_list),
                                window,
                                cx,
                            ),
                        )
                        .child(
                            v_flex()
                                .p(px(12.))
                                .gap(px(8.))
                                .when(has_operations, |v| {
                                    v.child(
                                        work_button("changes-tab-operations", "Operations…", theme)
                                            .debug_selector(|| "changes-tab-operations".into())
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.show_tab(Tab::Operations, cx)
                                            })),
                                    )
                                })
                                .when_some(main_folder.clone(), |v, main| {
                                    v.child(
                                        work_button(
                                            "changes-bring-in",
                                            format!("Bring turns into {main}"),
                                            theme,
                                        )
                                        .debug_selector(|| "changes-bring-in".into())
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.bring_in(window, cx)
                                            }),
                                        ),
                                    )
                                })
                                .child(change_counts(
                                    self.files.iter().map(|f| f.added).sum(),
                                    self.files.iter().map(|f| f.removed).sum(),
                                    theme,
                                ))
                                .child(work_notice(
                                    if self.files.iter().all(|f| f.turn.is_none()) {
                                        "Tool-reported changes"
                                    } else {
                                        "Recorded and observed changes"
                                    },
                                    false,
                                    theme,
                                )),
                        ),
                )
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .when(!self.wide, |v| v.child(self.compact_picker(cx, theme)))
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .child(content)
                            .custom_scrollbars(
                                scrollbar("changes-scrollbar", &self.scroll, None),
                                window,
                                cx,
                            ),
                    )
                    .child(self.composer.clone()),
            )
            .into_any_element()
    }
}
