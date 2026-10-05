use super::attachments::Attachment;
use super::mention_menu::{MentionMenu, Sources};
use super::mentions::{self, Mention};
use super::menus::{Choice, Picker};
use super::session::{Changes, SessionController, SessionEvent};
use super::*;
use gpui::{EventEmitter, Subscription};

mod status;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RevisionContext {
    pub path: String,
    pub source: String,
}
impl RevisionContext {
    fn prompt(&self, draft: &str) -> String {
        format!(
            "Revision request for file {} ({}). The review is historical/observed; inspect the current file before editing.\n\n{draft}",
            serde_json::to_string(&self.path).expect("string serialization"),
            self.source
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ComposerEvent {
    NewSession,
    /// The draft's mention chips changed.
    Mentions,
}

pub struct ComposerView {
    /// Where the input card was drawn last, so its menus open where they fit.
    card_top: std::rc::Rc<std::cell::Cell<Option<Pixels>>>,
    pub controller: Entity<SessionController>,
    pub input: Entity<TextInput>,
    pub attached: Option<slash::Attached>,
    pub expanded: bool,
    pub(super) revision: Option<RevisionContext>,
    pub(super) waiting: bool,
    /// Thread renders a settled tool failure beside its turn result; Changes
    /// keeps the shared status beside the composer instead.
    pub(super) contextual_issue: bool,
    pub(super) picker_filter: Entity<TextInput>,
    pub(super) picker: Option<Picker>,
    pub(super) picker_pending: Option<String>,
    pub(super) picker_index: usize,
    pub(super) picker_scroll: gpui::UniformListScrollHandle,
    pub(super) choices: Vec<Choice>,
    pub(super) _subscriptions: Vec<Subscription>,
    pub(super) slash_index: usize,
    pub(super) slash_menu: slash::Menu,
    pub(super) slash_dismissed: bool,
    pub(super) slash_seen: Option<String>,
    pub(super) slash_stash: Option<String>,
    /// What each chip in the draft stands for, by chip id.
    pub(super) mentions: std::collections::HashMap<usize, Mention>,
    pub(super) next_mention: usize,
    pub(super) mention: MentionMenu,
    pub(super) sources: Sources,
    /// Images that go with the next prompt.
    pub attachments: Vec<Attachment>,
    #[cfg(test)]
    pub renders: usize,
}
impl EventEmitter<ComposerEvent> for ComposerView {}
impl ComposerView {
    pub fn new(
        controller: Entity<SessionController>,
        initial_draft: &str,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            TextInput::new("Describe a change, ask a question, or drop a file…", cx)
                .multiline(8)
                .font_size(px(15.))
        });
        input.update(cx, |input, cx| input.set_content(initial_draft, cx));
        let picker_filter = cx.new(|cx| TextInput::new("Filter…", cx).compact());
        let subscriptions = vec![
            cx.observe(&input, |this, _, cx| this.composer_changed(cx)),
            cx.observe(&picker_filter, |this, _, cx| {
                this.refresh_choices(cx);
                this.picker_index = 0;
                this.picker_scroll
                    .scroll_to_item(0, gpui::ScrollStrategy::Top);
                cx.notify();
            }),
            cx.subscribe(&controller, |this, _, event, cx| match event {
                SessionEvent::Changed(changes)
                    if changes.intersects(
                        Changes::RUN
                            | Changes::METADATA
                            | Changes::QUEUE
                            | Changes::CATALOG
                            | Changes::STATUS,
                    ) =>
                {
                    if changes.intersects(Changes::CATALOG) {
                        this.refresh_choices(cx);
                        this.refresh_slash(cx);
                    }
                    cx.notify();
                }
                SessionEvent::RequestFinished(id) if this.picker_pending.as_ref() == Some(id) => {
                    this.picker_pending = None;
                    cx.notify();
                }
                SessionEvent::RecoverDraft(text) => {
                    this.input.update(cx, |input, cx| {
                        let content = if input.content().is_empty() {
                            text.clone()
                        } else {
                            format!("{}\n{text}", input.content())
                        };
                        input.set_content(content, cx);
                    });
                }
                SessionEvent::RecoverImages(images) => this.restore_images(images, cx),
                _ => {}
            }),
        ];
        Self {
            card_top: Default::default(),
            controller,
            input,
            attached: None,
            expanded: false,
            revision: None,
            waiting: false,
            contextual_issue: false,
            picker_filter,
            picker: None,
            picker_pending: None,
            picker_index: 0,
            picker_scroll: Default::default(),
            choices: vec![],
            _subscriptions: subscriptions,
            slash_index: 0,
            slash_menu: Default::default(),
            slash_dismissed: false,
            slash_seen: None,
            slash_stash: None,
            mentions: Default::default(),
            next_mention: 0,
            mention: Default::default(),
            sources: Default::default(),
            attachments: Vec::new(),
            #[cfg(test)]
            renders: 0,
        }
    }
    pub(super) fn set_revision(&mut self, context: RevisionContext, cx: &mut Context<Self>) {
        self.revision = Some(context);
        cx.notify();
    }
    pub(super) fn set_waiting(&mut self, waiting: bool, cx: &mut Context<Self>) {
        if self.waiting != waiting {
            self.waiting = waiting;
            cx.notify();
        }
    }
    pub(super) fn set_contextual_issue(&mut self, contextual: bool, cx: &mut Context<Self>) {
        if self.contextual_issue != contextual {
            self.contextual_issue = contextual;
            cx.notify();
        }
    }
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.input.focus_handle(cx).focus(window, cx);
    }
    /// Menus open above a docked composer; one high on the page, as on the
    /// landing, has no room above, so they open below it.
    pub(super) fn menus_below(&self) -> bool {
        self.card_top.get().is_some_and(|top| top < px(420.))
    }
    /// A starting point's text: the draft when it is empty, else added below it.
    /// Never submitted.
    pub fn offer_draft(&mut self, text: &str, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            let current = input.content().trim_end().to_owned();
            input.set_content(
                if current.is_empty() {
                    text.to_owned()
                } else {
                    format!("{current}\n\n{text}")
                },
                cx,
            )
        });
        cx.notify();
    }
    /// Choose a landing-page starting point without submitting or replacing the draft.
    pub fn use_command(&mut self, command: SlashCommand, cx: &mut Context<Self>) {
        self.attached = Some(slash::Attached::Pi(command));
        self.slash_dismissed = true;
        cx.notify();
    }
    pub fn deactivate(&mut self, cx: &mut Context<Self>) {
        self.picker = None;
        self.dismiss_slash(cx);
    }
    pub fn command(&mut self, command: Command, cx: &mut Context<Self>) -> Option<String> {
        self.controller
            .update(cx, |controller, cx| controller.command(command, cx))
    }
    pub fn clear_queue(&mut self, abort: bool, cx: &mut Context<Self>) {
        self.controller
            .update(cx, |controller, cx| controller.clear_queue(abort, cx));
    }
    pub fn toggle_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.expanded = !self.expanded;
        self.input
            .update(cx, |input, cx| input.set_fill(self.expanded, cx));
        self.focus(window, cx);
        cx.notify();
    }
    pub fn send_prompt(&mut self, follow_up: bool, cx: &mut Context<Self>) {
        let raw = self.input.read(cx).content().to_owned();
        if self.waiting
            || (raw.trim().is_empty() && self.attachments.is_empty() && self.attached.is_none())
        {
            return;
        }
        if self.revision.is_some() && (self.attached.is_some() || raw.starts_with(['!', '/'])) {
            self.controller.update(cx, |c, cx| {
                c.notice(
                    "Remove the revision attachment before running a shell or slash command.",
                    cx,
                )
            });
            return;
        }
        if self.attached.is_none() && raw.starts_with('!') {
            if follow_up || !self.attachments.is_empty() || !self.input.read(cx).chips().is_empty()
            {
                self.controller.update(cx, |controller, cx| controller.notice(
                    "Run shell commands explicitly with Enter, without attachments or mention chips. Use !! to exclude output from model context.", cx));
                return;
            }
            if self
                .controller
                .update(cx, |controller, cx| controller.submit_shell(raw, cx))
            {
                self.input.update(cx, |input, cx| {
                    input.set_content("", cx);
                    input.set_fill(false, cx);
                });
                self.expanded = false;
                cx.notify();
            }
            return;
        }
        // Problems chosen a moment ago may still be read from their files.
        if self.mentions_pending(follow_up) {
            return;
        }
        let draft = {
            let input = self.input.read(cx);
            mentions::prompt(input.content(), input.chips(), &self.mentions)
        };
        let builtin = match &self.attached {
            Some(slash::Attached::BuiltIn(builtin)) => Some((*builtin, draft.clone())),
            Some(slash::Attached::Pi(_)) => None,
            None => self.typed_builtin(&draft, cx),
        };
        if let Some((builtin, text)) = builtin {
            if !matches!(builtin, slash::BuiltIn::Copy | slash::BuiltIn::New)
                && !self.controller.read(cx).ready()
            {
                return;
            }
            self.attached = None;
            self.input.update(cx, |input, cx| input.set_content("", cx));
            self.run_builtin(builtin, text, cx);
            return;
        }
        let content = match &self.attached {
            Some(slash::Attached::Pi(command)) if draft.is_empty() => format!("/{}", command.name),
            Some(slash::Attached::Pi(command)) => format!("/{} {draft}", command.name),
            _ => draft,
        };
        let content = match &self.revision {
            Some(context) if !content.trim().is_empty() => context.prompt(&content),
            _ => content,
        };
        let images = self
            .attachments
            .iter()
            .map(|attachment| attachment.content.clone())
            .collect();
        if self.controller.update(cx, |controller, cx| {
            controller.submit_with(content, images, follow_up, cx)
        }) {
            self.attachments.clear();
            self.attached = None;
            self.revision = None;
            self.expanded = false;
            self.input.update(cx, |input, cx| {
                input.set_fill(false, cx);
                input.set_content("", cx);
            });
            cx.notify();
        }
    }
    fn compact_idle(&self, cx: &App) -> bool {
        let controller = self.controller.read(cx);
        let model = controller.model();
        !self.expanded
            && !self.waiting
            && !controller.working()
            && self.input.read(cx).content().trim().is_empty()
            && self.attached.is_none()
            && self.attachments.is_empty()
            && self.revision.is_none()
            && model.steering.is_empty()
            && model.follow_up.is_empty()
            && model.shell.is_none()
    }
    fn composer(&self, window: &Window, cx: &Context<Self>, theme: Theme) -> impl IntoElement {
        let open = self.controller.read(cx);
        let model = open.model();
        let compact = self.compact_idle(cx);
        let shell = self.attached.is_none() && self.input.read(cx).content().starts_with('!');
        let enabled = open.ready()
            && !self.waiting
            && (!shell || open.can_navigate())
            && (!self.input.read(cx).content().trim().is_empty()
                || !self.attachments.is_empty()
                || self.attached.is_some());
        let queue = model
            .steering
            .iter()
            .map(|message| ("Steer", message))
            .chain(model.follow_up.iter().map(|message| ("Next", message)));
        let card_top = self.card_top.clone();
        v_flex()
            .debug_selector(|| "composer".into())
            .relative()
            .child(
                gpui::canvas(
                    move |bounds, _, _| card_top.set(Some(bounds.top())),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .when(!self.expanded, |composer| {
                composer.h(px(if compact { 112. } else { 142. }))
            })
            .when(self.expanded, |composer| composer.flex_1().min_h_0())
            .mx(WORK_GUTTER)
            .mb(WORK_GUTTER)
            .rounded(px(8.))
            .bg(theme.composer)
            .border_1()
            .border_color(if shell { theme.amber } else { theme.focus })
            .flex_shrink_0()
            .when(shell, |view| {
                view.child(
                    h_flex()
                        .debug_selector(|| "shell-mode".into())
                        .px(px(16.))
                        .pt(px(9.))
                        .gap(px(8.))
                        .text_size(px(11.))
                        .text_color(theme.amber)
                        .child(icon("terminal", theme.amber).size(px(13.)))
                        .child(if self.input.read(cx).content().starts_with("!!") {
                            "SHELL · output excluded from model context"
                        } else {
                            "SHELL · output included on the next prompt · !! excludes it"
                        }),
                )
            })
            // Files dropped anywhere on the composer attach.
            .on_drop(
                cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| {
                    this.drop_paths(paths, window, cx)
                }),
            )
            .drag_over::<gpui::ExternalPaths>(move |style, _, _, _| {
                style.border_color(theme.accent)
            })
            .children(queue.enumerate().map(|(index, (kind, message))| {
                h_flex()
                    .h(px(30.))
                    .gap(px(12.))
                    .px(px(14.))
                    .rounded_t(px(9.))
                    .bg(theme.queue)
                    .border_b_1()
                    .border_color(theme.queue_line)
                    .child(
                        div()
                            .font_family(MONO)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(12.))
                            .text_color(theme.amber)
                            .child(kind),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .child(message.clone()),
                    )
                    .when(
                        index == 0 && model.steering.len() + model.follow_up.len() > 1,
                        |row| {
                            row.child(
                                div()
                                    .text_size(px(10.5))
                                    .font_family(MONO)
                                    .text_color(theme.faint)
                                    .child(format!(
                                        "{} queued",
                                        model.steering.len() + model.follow_up.len()
                                    )),
                            )
                        },
                    )
                    .child(
                        icon_button(
                            ("clear-queue", index),
                            "close",
                            "Clear queued messages and restore drafts",
                            theme,
                        )
                        .size(px(20.))
                        .on_click(cx.listener(|this, _, _, cx| this.clear_queue(false, cx))),
                    )
            }))
            .when_some(self.revision.as_ref(), |v, context| {
                v.child(
                    h_flex()
                        .id("revision-attachment")
                        .px(px(12.))
                        .h(px(30.))
                        .gap(px(8.))
                        .text_size(px(12.))
                        .text_color(theme.muted)
                        .debug_selector(|| "revision-context".into())
                        .tooltip(ui::Tooltip::text(format!(
                            "{}\n{}",
                            context.path, context.source
                        )))
                        .child(icon("file", theme.muted).size(px(13.)))
                        .child(
                            div().flex_1().min_w_0().truncate().child(
                                std::path::Path::new(&context.path)
                                    .file_name()
                                    .unwrap_or_else(|| std::ffi::OsStr::new(&context.path))
                                    .to_string_lossy()
                                    .into_owned(),
                            ),
                        )
                        .child(
                            icon_button(
                                "remove-revision",
                                "close",
                                "Remove revision attachment",
                                theme,
                            )
                            .debug_selector(|| "remove-revision".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.revision = None;
                                cx.notify();
                            })),
                        ),
                )
            })
            .children(self.attachments_row(cx, theme))
            .child(
                h_flex()
                    .relative()
                    .items_start()
                    .gap(px(8.))
                    .pl(px(16.))
                    .pr(px(44.))
                    .pt(px(13.))
                    .pb(px(6.))
                    .min_h(px(if compact {
                        66.
                    } else if model.steering.is_empty()
                        && model.follow_up.is_empty()
                        && self.revision.is_none()
                    {
                        96.
                    } else {
                        66.
                    }))
                    .when(self.expanded, |area| area.flex_1().min_h_0())
                    .when_some(self.attached.as_ref(), |row, attached| {
                        row.child(command_chip(attached, cx, theme))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .when(self.expanded, |input| input.h_full())
                            .child(self.input.clone()),
                    )
                    .child({
                        let (glyph, title) = if self.expanded {
                            ("minimize", "Collapse Message Editor")
                        } else {
                            ("maximize", "Expand Message Editor")
                        };
                        icon_button("expand-composer", glyph, title, theme)
                            .debug_selector(|| "expand-composer".into())
                            .absolute()
                            .top(px(9.))
                            .right(px(10.))
                            .tooltip(move |_, cx| {
                                ui::Tooltip::for_action(title, &ExpandMessageEditor, cx)
                            })
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_composer(window, cx)),
                            )
                    }),
            )
            .child(
                h_flex()
                    .pl(px(8.))
                    .pr(px(16.))
                    .pb(px(14.))
                    .gap(px(8.))
                    .child(
                        icon_button("attach", "attach", "Attach files or images", theme)
                            .debug_selector(|| "attach".into())
                            .w(px(28.))
                            .tooltip(ui::Tooltip::text(
                                "Attach files or images; you can also paste or drop them",
                            ))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.pick_files(window, cx)),
                            ),
                    )
                    // Beside the paperclip, so the menu opens in the column it was asked from.
                    .child(
                        icon_button("commands", "slash", "Choose a slash command", theme)
                            .debug_selector(|| "slash-button".into())
                            .aria_expanded(self.slash_query(cx).is_some())
                            .tooltip(ui::Tooltip::text("Choose a slash command"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_slash(window, cx)),
                            ),
                    )
                    .child(
                        chip("choose-model", "Choose model", theme)
                            .bg(gpui::transparent_black())
                            .aria_expanded(self.picker == Some(menus::Picker::Model))
                            .h(px(30.))
                            .font_family(SANS)
                            .text_size(px(13.))
                            .child(
                                div().max_w(px(190.)).truncate().child(
                                    model
                                        .state
                                        .model
                                        .as_ref()
                                        .map(|model| {
                                            model.name.as_ref().unwrap_or(&model.id).clone()
                                        })
                                        .unwrap_or_else(|| "Choose model".into()),
                                ),
                            )
                            .child(icon("chevron_down", theme.faint).size(px(11.)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_picker(menus::Picker::Model, window, cx)
                            })),
                    )
                    .child(
                        chip("choose-thinking", "Choose thinking level", theme)
                            .bg(gpui::transparent_black())
                            .px(px(10.))
                            .h(px(30.))
                            .font_family(SANS)
                            .text_size(px(13.))
                            .aria_expanded(self.picker == Some(menus::Picker::Thinking))
                            .child(if model.state.thinking_level.is_empty() {
                                "Off".to_owned()
                            } else {
                                let level = &model.state.thinking_level;
                                format!("{}{}", level[..1].to_uppercase(), &level[1..])
                            })
                            .child(icon("chevron_down", theme.faint).size(px(11.)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_picker(menus::Picker::Thinking, window, cx)
                            })),
                    )
                    .child(div().flex_1())
                    .when(model.busy() && !shell && model.shell.is_none(), |row| {
                        row.child(
                            work_button("steer-now", "Steer now", theme)
                                .w(px(114.))
                                .justify_center()
                                .debug_selector(|| "steer-now".into())
                                .tooltip(|_, cx| ui::Tooltip::for_action("Steer now", &Steer, cx))
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.send_prompt(false, cx)),
                                ),
                        )
                    })
                    .child(
                        work_primary(
                            "submit",
                            if shell {
                                "Run ↵"
                            } else if model.busy() {
                                "Queue follow-up ↵"
                            } else if self.revision.is_some() {
                                "Send revision ↵"
                            } else {
                                "Send ↵"
                            },
                            enabled,
                            theme,
                        )
                        .w(px(if model.busy() {
                            150.
                        } else if self.revision.is_some() {
                            148.
                        } else {
                            98.
                        }))
                        .justify_center()
                        .flex_shrink_0()
                        .debug_selector(|| "submit".into())
                        .on_click(cx.listener(|this, _, _, cx| {
                            let follow_up = this.controller.read(cx).model().busy();
                            this.send_prompt(follow_up, cx);
                        })),
                    ),
            )
            .children(self.picker_view(cx, theme))
            .children(self.slash_view(cx, theme))
            .children(self.mention_view(window, cx, theme))
    }
}
impl Render for ComposerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.renders += 1;
        }
        let compact = self.compact_idle(cx);
        v_flex()
            .key_context("Composer")
            .min_h(px(if compact { 136. } else { 190. }))
            .flex_shrink_0()
            .when(self.expanded, |view| view.h(relative(0.62)))
            .on_action(cx.listener(|this, _: &Submit, window, cx| {
                if this.picker.is_some() {
                    this.choose(this.picker_index, window, cx);
                } else if this.mention_open() {
                    this.choose_mention(this.mention.index(), window, cx);
                } else if this.slash_query(cx).is_some() {
                    this.choose_slash(this.slash_index, window, cx);
                } else if this.input.focus_handle(cx).is_focused(window) {
                    let follow_up = this.controller.read(cx).model().busy()
                        && !this.input.read(cx).content().starts_with('!');
                    this.send_prompt(follow_up, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Steer, window, cx| {
                if this.picker.is_none()
                    && !this.mention_open()
                    && this.slash_query(cx).is_none()
                    && this.input.focus_handle(cx).is_focused(window)
                {
                    this.send_prompt(false, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Complete, window, cx| {
                if this.mention_open() {
                    this.complete_mention(window, cx);
                } else if this.slash_query(cx).is_some() {
                    this.choose_slash(this.slash_index, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &FollowUp, window, cx| {
                if this.picker.is_none()
                    && !this.mention_open()
                    && this.slash_query(cx).is_none()
                    && this.input.focus_handle(cx).is_focused(window)
                {
                    this.send_prompt(true, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Stop, window, cx| {
                cx.stop_propagation();
                if this.picker.take().is_some() {
                    this.input.focus_handle(cx).focus(window, cx);
                    cx.notify();
                } else if this.mention_open() {
                    this.dismiss_mentions(cx);
                } else if this.slash_query(cx).is_some() {
                    this.dismiss_slash(cx);
                } else {
                    this.clear_queue(true, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PreviousChoice, _, cx| {
                if this.mention_open() {
                    this.move_mention(-1, cx)
                } else if this.slash_query(cx).is_some() {
                    this.move_slash(-1, cx)
                } else {
                    this.move_choice(-1, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &NextChoice, _, cx| {
                if this.mention_open() {
                    this.move_mention(1, cx)
                } else if this.slash_query(cx).is_some() {
                    this.move_slash(1, cx)
                } else {
                    this.move_choice(1, cx)
                }
            }))
            // Up and Down belong to an open menu before the input's own line movement.
            .capture_action(cx.listener(|this, _: &crate::input::Up, _, cx| {
                if this.picker.is_some() {
                    this.move_choice(-1, cx);
                    cx.stop_propagation();
                } else if this.mention_open() {
                    this.move_mention(-1, cx);
                    cx.stop_propagation();
                } else if this.slash_query(cx).is_some() {
                    this.move_slash(-1, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &crate::input::Down, _, cx| {
                if this.picker.is_some() {
                    this.move_choice(1, cx);
                    cx.stop_propagation();
                } else if this.mention_open() {
                    this.move_mention(1, cx);
                    cx.stop_propagation();
                } else if this.slash_query(cx).is_some() {
                    this.move_slash(1, cx);
                    cx.stop_propagation();
                }
            }))
            // A pasted image or copied files attach; text pastes as usual.
            .capture_action(cx.listener(|this, _: &crate::input::Paste, window, cx| {
                if this.paste_attachments(window, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &ExpandMessageEditor, window, cx| {
                cx.stop_propagation();
                this.toggle_composer(window, cx)
            }))
            .child(self.work_status(cx, theme(cx)))
            .child(self.composer(window, cx, theme(cx)))
    }
}
/// The `/` menu's choice, shown as a token rather than `/skill:name` text in the draft.
fn command_chip(
    attached: &slash::Attached,
    cx: &Context<ComposerView>,
    theme: Theme,
) -> impl IntoElement {
    h_flex()
        .id("composer-command")
        .debug_selector(|| "composer-command".into())
        .flex_shrink_0()
        .h(px(22.))
        .mt(px(-1.))
        .pl(px(7.))
        .pr(px(2.))
        .gap(px(5.))
        .rounded(px(5.))
        .bg(theme.selected)
        .border_1()
        .border_color(theme.focus)
        .font_family(MONO)
        .text_size(px(11.))
        .text_color(theme.accent)
        .child(icon(attached.icon(), theme.accent).size(px(12.)))
        .when(attached.is_skill(), |chip| {
            chip.child(div().text_color(theme.faint).child("skill"))
        })
        .child(attached.label())
        .child(
            icon_button("remove-command", "close", "Remove command", theme)
                .size(px(16.))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.attached = None;
                    cx.notify();
                })),
        )
}
