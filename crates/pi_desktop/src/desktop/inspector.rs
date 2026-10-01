use super::panels::note;
use super::session::{Changes, SessionController, SessionEvent};
use super::*;
use gpui::Div;

#[derive(Clone, Default)]
pub enum InspectorPage {
    #[default]
    Thread,
    Tree(Entity<super::tree::TreeView>),
    Changes(Entity<super::changes::ChangesView>),
    Context(Entity<super::context::ContextView>),
    Files(Entity<super::files::FilesView>),
    Diagnostics(Entity<super::diagnostics::DiagnosticsView>),
}
pub struct InspectorView {
    page: InspectorPage,
    composer: Option<Entity<super::composer::ComposerView>>,
    composer_subscription: Option<gpui::Subscription>,
    page_subscription: Option<gpui::Subscription>,
    controller: Entity<SessionController>,
    scroll: ScrollHandle,
    _subscription: gpui::Subscription,
    #[cfg(test)]
    pub renders: usize,
}
impl InspectorView {
    pub fn new(controller: Entity<SessionController>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&controller, |_, _, event, cx| {
            if matches!(event, SessionEvent::Changed(c) if c.intersects(Changes::METADATA | Changes::SUMMARY | Changes::RUN | Changes::JJ | Changes::CATALOG | Changes::STATUS)) {
                cx.notify();
            }
        });
        Self {
            page: InspectorPage::Thread,
            composer: None,
            composer_subscription: None,
            page_subscription: None,
            controller,
            scroll: ScrollHandle::new(),
            _subscription: subscription,
            #[cfg(test)]
            renders: 0,
        }
    }
    /// The draft's mentions show under IN THIS PROMPT; typing alone renders nothing here.
    pub fn set_composer(
        &mut self,
        composer: &Entity<super::composer::ComposerView>,
        cx: &mut Context<Self>,
    ) {
        self.composer_subscription = Some(cx.subscribe(composer, |_, _, event, cx| {
            if matches!(event, super::composer::ComposerEvent::Mentions) {
                cx.notify();
            }
        }));
        self.composer = Some(composer.clone());
    }

    fn prompt_mentions(&self, cx: &Context<Self>, theme: Theme) -> Option<Div> {
        let mentions = self.composer.as_ref()?.read(cx).draft_mentions(cx);
        if mentions.is_empty() {
            return None;
        }
        Some(
            v_flex()
                .debug_selector(|| "inspector-prompt-mentions".into())
                .child(section("IN THIS PROMPT", "", theme).mt(px(16.)))
                .children(mentions.into_iter().map(|(kind, label, how)| {
                    h_flex()
                        .h(px(24.))
                        .gap(px(8.))
                        .child(icon(kind.icon(), theme.muted).size(px(13.)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(MONO)
                                .text_size(px(11.5))
                                .text_color(theme.secondary)
                                .child(label),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(10.5))
                                .text_color(theme.faint)
                                .child(how),
                        )
                }))
                .child(
                    note(
                        "Files and symbols go as paths and locations; terminal output, problems, turns and sessions go as text. Nothing is hidden.",
                        theme,
                    )
                    .mt(px(6.)),
                )
                .child(divider(theme)),
        )
    }

    pub fn show(&mut self, page: InspectorPage, cx: &mut Context<Self>) {
        if let InspectorPage::Files(view) = &page {
            view.update(cx, |view, cx| view.load_browser(cx));
        }
        self.page_subscription = match &page {
            InspectorPage::Thread => None,
            InspectorPage::Tree(view) => Some(cx.observe(view, |_, _, cx| cx.notify())),
            InspectorPage::Changes(view) => Some(cx.observe(view, |_, _, cx| cx.notify())),
            InspectorPage::Context(view) => Some(cx.observe(view, |_, _, cx| cx.notify())),
            InspectorPage::Files(view) => Some(cx.observe(view, |_, _, cx| cx.notify())),
            InspectorPage::Diagnostics(view) => Some(cx.observe(view, |_, _, cx| cx.notify())),
        };
        self.page = page;
        self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        cx.notify();
    }

    fn overview(&self, cx: &Context<Self>, theme: Theme) -> Div {
        let controller = self.controller.read(cx);
        let model = controller.model();
        let new = controller.not_started();
        let working = controller.working();
        let failed =
            !controller.is_connected() || controller.bootstrap_failed() || model.error.is_some();
        let animating = !failed && (working || !controller.ready());
        let color = if failed {
            theme.coral
        } else if animating {
            theme.accent
        } else if new {
            theme.faint
        } else {
            theme.green
        };
        let status = if !controller.is_connected() {
            "Disconnected".into()
        } else if controller.bootstrap_failed() {
            "Setup incomplete".into()
        } else if !controller.ready() {
            "Connecting…".into()
        } else if working {
            model
                .tools
                .iter()
                .rev()
                .find(|t| !t.finished)
                .map(|t| format!("{} · {}", model.run_label(), t.name))
                .unwrap_or_else(|| {
                    if model.busy() {
                        model.run_label().into()
                    } else {
                        "Starting…".into()
                    }
                })
        } else if failed {
            "Needs attention".into()
        } else if new {
            "Not started".into()
        } else {
            "Ready".into()
        };
        let panel = v_flex()
            .w_full()
            .flex_shrink_0()
            .px(px(20.))
            .pt(px(16.))
            .pb(px(16.))
            .text_size(px(12.))
            .child(
                div()
                    .font_family(SERIF)
                    .italic()
                    .text_size(px(22.))
                    .line_height(px(30.))
                    .truncate()
                    .child(model.title().to_owned()),
            )
            .child(
                h_flex()
                    .debug_selector(|| "inspector-run-status".into())
                    .mt(px(7.))
                    .mb(px(3.))
                    .h(px(24.))
                    .gap(px(8.))
                    .text_size(px(12.))
                    .text_color(color)
                    .child(if animating {
                        spinner("inspector-working", theme).into_any_element()
                    } else if failed {
                        icon("warning", color).size(px(13.)).into_any_element()
                    } else {
                        div()
                            .size(px(7.))
                            .rounded_full()
                            .bg(color)
                            .into_any_element()
                    })
                    .child(div().flex_1().min_w_0().truncate().child(status)),
            )
            .child(divider(theme).mt(px(8.)))
            .children(self.prompt_mentions(cx, theme));
        if new {
            panel.child(self.before_start(cx, theme))
        } else {
            panel
                .child(self.history(cx, theme))
                .child(divider(theme))
                .child(context(model, theme))
                .child(divider(theme))
                .child(self.active_tools(cx, theme))
                .child(divider(theme))
                .child(usage(model, theme))
                .child(self.session_file(cx, theme))
                .when(!model.extension_status.is_empty(), |panel| {
                    panel
                        .child(divider(theme))
                        .child(section("EXTENSION STATUS", "", theme).mt(px(10.)))
                        .children(
                            model
                                .extension_status
                                .iter()
                                .map(|(key, value)| note(format!("{key} · {value}"), theme)),
                        )
                })
        }
    }

    fn before_start(&self, cx: &Context<Self>, theme: Theme) -> Div {
        let controller = self.controller.read(cx);
        let model = controller.model();
        let selected = model.state.model.as_ref();
        v_flex()
            .debug_selector(|| "new-session-inspector".into())
            .child(section("MODEL", "", theme).mt(px(16.)))
            .child(metadata_pair(
                "Model",
                selected.map(|m| m.id.as_str()).unwrap_or("Not selected"),
                theme,
            ))
            .child(metadata_pair(
                "Provider",
                selected
                    .map(|m| m.provider.as_str())
                    .unwrap_or("Not reported"),
                theme,
            ))
            .child(pair(
                "Context window",
                selected
                    .filter(|m| m.context_window > 0)
                    .map(|m| count(m.context_window))
                    .unwrap_or_else(|| "—".into()),
                theme,
            ))
            .child(pair(
                "Thinking",
                if model.state.thinking_level.is_empty() {
                    "Not reported".into()
                } else {
                    model.state.thinking_level.clone()
                },
                theme,
            ))
            .child(divider(theme))
            .child(self.active_tools(cx, theme))
            .child(divider(theme))
            .child(section("PROJECT", "", theme).mt(px(12.)))
            .child(
                div()
                    .id("inspector-project-path")
                    .font_family(MONO)
                    .text_size(px(11.))
                    .line_height(px(18.))
                    .truncate()
                    .tooltip(ui::Tooltip::text(model.cwd.display().to_string()))
                    .child(model.cwd.display().to_string()),
            )
            .child(project_history(controller.jj().project.is_some(), theme))
            .child(self.session_file(cx, theme))
            .child(divider(theme))
            .child(section("KEYS", "", theme).mt(px(12.)))
            .child(pair("Send", "Enter".into(), theme))
            .child(pair("New line", "Shift+Enter".into(), theme))
            .child(pair("Commands / skills", "/".into(), theme))
    }

    fn active_tools(&self, cx: &Context<Self>, theme: Theme) -> Div {
        let tools = self.controller.read(cx).model().state.active_tools.as_ref();
        let hint = tools
            .map(|tools| format!("{} active", tools.len()))
            .unwrap_or_else(|| "not reported".into());
        let mut panel = v_flex()
            .debug_selector(|| "inspector-active-tools".into())
            .child(section("TOOLS", &hint, theme).mt(px(12.)));
        let Some(tools) = tools else {
            return panel.child(
                note("Pi has not reported its active tools.", theme)
                    .debug_selector(|| "active-tools-unreported".into()),
            );
        };
        if tools.is_empty() {
            return panel.child(
                note("No tools are active in this session.", theme)
                    .debug_selector(|| "active-tools-empty".into()),
            );
        }
        for (index, tool) in tools.iter().enumerate() {
            let mut details = format!(
                "{}\n{}",
                tool.name,
                tool.description
                    .as_deref()
                    .unwrap_or("Description not reported")
            );
            if let Some(source) = &tool.source_info {
                details.push_str(&format!(
                    "\n{} · {}\n{}",
                    source.source, source.scope, source.path
                ));
            }
            panel = panel.child(
                h_flex()
                    .id(("active-tool", index))
                    .debug_selector(move || format!("active-tool-{index}"))
                    .w_full()
                    .min_w_0()
                    .h(px(20.))
                    .tooltip(ui::Tooltip::text(details))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(MONO)
                            .text_size(px(11.5))
                            .text_color(theme.secondary)
                            .child(tool.name.clone()),
                    ),
            );
        }
        panel
    }

    fn history(&self, cx: &Context<Self>, theme: Theme) -> Div {
        let controller = self.controller.read(cx);
        let records = &controller.jj().records;
        let enabled = controller.jj().project.is_some() && controller.jj_idle();
        let mut body = v_flex()
            .debug_selector(|| "inspector-history".into())
            .child(
                section(
                    "HISTORY",
                    if records.is_empty() {
                        "not recorded"
                    } else if controller.is_demo() {
                        "sample · jj"
                    } else {
                        "jj"
                    },
                    theme,
                )
                .mt(px(16.)),
            );
        if records.is_empty() {
            body = body.child(note(if controller.jj().project.is_some() {
                "No recorded turns for this session. jj records new turns; it cannot reconstruct older edits."
            } else {
                "No jj turn history. Older sessions keep their conversation, but file snapshots cannot be reconstructed."
            }, theme))
                .child(note("Successful edit/write calls only; shell and external changes may be missing. No undo for unrecorded work.", theme).mt(px(4.)));
        } else {
            let current = records.iter().rposition(|r| r.undone.is_none());
            for (i, record) in records.iter().enumerate().rev().take(3) {
                body = body.child(
                    h_flex()
                        .h(px(26.))
                        .gap(px(7.))
                        .child(
                            icon(
                                "git_commit",
                                if Some(i) == current {
                                    theme.accent
                                } else {
                                    theme.faint
                                },
                            )
                            .size(px(12.)),
                        )
                        .child(
                            div()
                                .id(("history-diff", i))
                                .debug_selector(move || format!("history-diff-{i}"))
                                .flex_1()
                                .min_w_0()
                                .cursor_pointer()
                                .child(
                                    h_flex()
                                        .gap(px(8.))
                                        .child(
                                            div()
                                                .font_family(MONO)
                                                .text_size(px(10.5))
                                                .text_color(if Some(i) == current {
                                                    theme.accent
                                                } else {
                                                    theme.faint
                                                })
                                                .child(record.short.clone()),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .text_color(if record.undone.is_some() {
                                                    theme.faint
                                                } else {
                                                    theme.secondary
                                                })
                                                .child(if record.description.is_empty() {
                                                    format!("Turn {}", i + 1)
                                                } else {
                                                    record.description.clone()
                                                }),
                                        ),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.controller.update(cx, |c, cx| c.review_turn(i, cx))
                                })),
                        )
                        .child(if record.undone.is_some() {
                            primary_button(("history-redo", i), "Redo", enabled, theme)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if this.controller.read(cx).jj().project.is_some() {
                                        this.controller.update(cx, |c, cx| c.redo_turn(i, cx));
                                    }
                                }))
                                .into_any_element()
                        } else {
                            h_flex()
                                .gap(px(4.))
                                .font_family(MONO)
                                .text_size(px(10.5))
                                .child(
                                    div()
                                        .text_color(theme.green)
                                        .child(format!("+{}", record.added)),
                                )
                                .when(record.removed > 0, |v| {
                                    v.child(
                                        div()
                                            .text_color(theme.coral)
                                            .child(format!("−{}", record.removed)),
                                    )
                                })
                                .into_any_element()
                        }),
                );
            }
            body = body.child(primary_button("history-undo", "Undo last turn", enabled && current.is_some(), theme)
                    .mt(px(6.)).debug_selector(||"history-undo".into()).on_click(cx.listener(move|this,_,_,cx|{
                        if let Some(i)=current && this.controller.read(cx).jj().project.is_some(){this.controller.update(cx,|c,cx|c.undo_turn(i,cx));}
                    })))
                .child(note("Recorded turns only. Earlier or unrecorded work is not covered by these snapshots.",theme).mt(px(4.)));
        }
        body
    }

    fn session_file(&self, cx: &Context<Self>, theme: Theme) -> Div {
        let path = self
            .controller
            .read(cx)
            .model()
            .state
            .session_file
            .as_deref();
        v_flex()
            .child(divider(theme))
            .child(section("SESSION FILE", "", theme).mt(px(12.)))
            .child(
                h_flex()
                    .h(px(24.))
                    .gap(px(4.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(MONO)
                            .text_size(px(10.5))
                            .truncate()
                            .child(
                                path.map(short_path)
                                    .unwrap_or_else(|| "No path reported yet".into()),
                            ),
                    )
                    .when(path.is_some(), |v| {
                        v.child(
                            icon_button(
                                "copy-session-path",
                                "copy",
                                "Copy session file path",
                                theme,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(path) =
                                    &this.controller.read(cx).model().state.session_file
                                {
                                    cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                                }
                            })),
                        )
                    }),
            )
    }
}

fn metadata_pair(name: &str, value: &str, theme: Theme) -> Div {
    h_flex()
        .h(px(24.))
        .gap(px(8.))
        .text_size(px(12.))
        .child(div().text_color(theme.muted).child(name.to_owned()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_right()
                .truncate()
                .font_family(MONO)
                .text_color(theme.secondary)
                .child(value.to_owned()),
        )
}
fn context(model: &Session, theme: Theme) -> Div {
    let usage = model.stats.context_usage.as_ref();
    let percent = usage.and_then(|u| u.percent);
    v_flex()
        .child(
            section(
                "CONTEXT",
                if model.state.auto_compaction_enabled {
                    "auto-compact on"
                } else {
                    "auto-compact off"
                },
                theme,
            )
            .mt(px(12.)),
        )
        .child(
            h_flex()
                .h(px(30.))
                .gap(px(7.))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(20.))
                        .child(
                            usage
                                .and_then(|u| u.tokens)
                                .map(count)
                                .unwrap_or_else(|| "—".into()),
                        ),
                )
                .child(
                    div().text_color(theme.muted).child(
                        usage
                            .map(|u| format!("/ {} tokens", count(u.context_window)))
                            .unwrap_or_else(|| "Not reported yet".into()),
                    ),
                ),
        )
        .child(div().h(px(6.)).rounded(px(3.)).bg(theme.track).child(
            div().h_full().rounded(px(3.)).bg(theme.accent).w(relative(
                (percent.unwrap_or(0.).clamp(0., 100.) / 100.) as f32,
            )),
        ))
        .child(
            div()
                .mt(px(4.))
                .text_size(px(11.))
                .text_color(theme.faint)
                .child(
                    percent
                        .map(|p| format!("{p:.0}% of window"))
                        .unwrap_or_else(|| "Waiting for usage from Pi".into()),
                ),
        )
}
fn project_history(recording: bool, theme: Theme) -> Div {
    v_flex()
        .mt(px(6.))
        .gap(px(4.))
        .debug_selector(|| "project-history".into())
        .child(
            h_flex()
                .justify_between()
                .gap(px(12.))
                .h(px(24.))
                .child(div().text_color(theme.muted).child("File history"))
                .child(
                    h_flex()
                        .gap(px(5.))
                        .text_size(px(11.))
                        .text_color(if recording { theme.accent } else { theme.faint })
                        .child(
                            icon(
                                "git_commit",
                                if recording { theme.accent } else { theme.faint },
                            )
                            .size(px(12.)),
                        )
                        .child(if recording {
                            "jj enabled"
                        } else {
                            "Not recording"
                        }),
                ),
        )
        .child(note(
            if recording {
                "New turns are recorded with jj. Enabling it does not reconstruct earlier edits."
            } else {
                "No file snapshots. Enable jj to record future turns."
            },
            theme,
        ))
}

/// Token reuse, not a fabricated count of cache-hit requests.
fn cache_reuse(tokens: &pi_core::protocol::Tokens) -> Option<f64> {
    let total = tokens.input as f64 + tokens.cache_read as f64 + tokens.cache_write as f64;
    (total > 0.).then(|| 100. * tokens.cache_read as f64 / total)
}
pub(super) fn usage(model: &Session, theme: Theme) -> Div {
    let tokens = &model.stats.tokens;
    v_flex()
        .debug_selector(|| "inspector-usage".into())
        .child(section("USAGE", "this session", theme).mt(px(12.)))
        .child(pair("Input", count(tokens.input), theme))
        .child(pair("Output", count(tokens.output), theme))
        .child(pair("Cache read", count(tokens.cache_read), theme))
        .child(pair("Cache write", count(tokens.cache_write), theme))
        .child(
            pair(
                "Cache hits (tokens)",
                cache_reuse(tokens)
                    .map(|p| format!("{p:.1}%"))
                    .unwrap_or_else(|| "—".into()),
                theme,
            )
            .id("cache-reuse")
            .tooltip(ui::Tooltip::text(
                "Cache read ÷ (input + cache read + cache write). Not a count of requests.",
            )),
        )
        .child(pair(
            "Cost",
            model
                .stats
                .cost
                .map(|c| format!("${c:.2}"))
                .unwrap_or_else(|| "—".into()),
            theme,
        ))
}
impl Render for InspectorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.renders += 1;
        }
        let theme = theme(cx);
        let content = match self.page.clone() {
            InspectorPage::Thread => self.overview(cx, theme).into_any_element(),
            InspectorPage::Tree(view) => view.update(cx, |v, cx| v.inspector(window, cx)),
            InspectorPage::Changes(view) => view.update(cx, |v, cx| v.inspector(window, cx)),
            InspectorPage::Context(view) => view.update(cx, |v, cx| v.inspector(window, cx)),
            InspectorPage::Files(view) => view.update(cx, |v, cx| v.inspector(window, cx)),
            InspectorPage::Diagnostics(view) => view.into_any_element(),
        };
        div()
            .relative()
            .size_full()
            .bg(theme.panel)
            .border_l_1()
            .border_color(theme.edge)
            .child(
                div()
                    .id("inspector")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(content),
            )
            .custom_scrollbars(
                scrollbar("inspector-scrollbar", &self.scroll, None),
                window,
                cx,
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_hit_fraction_includes_cache_writes_and_unknown_is_not_zero() {
        assert_eq!(cache_reuse(&Default::default()), None);
        let tokens = pi_core::protocol::Tokens {
            input: 10,
            output: 900,
            cache_read: 60,
            cache_write: 30,
        };
        assert_eq!(cache_reuse(&tokens), Some(60.));
    }
}
