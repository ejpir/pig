use super::attachments::Attachment;
use super::mention_menu::{MentionMenu, Sources};
use super::mentions::{self, Mention};
use super::menus::{Choice, Picker};
use super::session::{Changes, SessionController, SessionEvent};
use super::*;
use gpui::{EventEmitter, Subscription};

#[derive(Clone, Copy, Debug)]
pub enum ComposerEvent {
    NewSession,
    /// The draft's mention chips changed.
    Mentions,
}

pub struct ComposerView {
    pub controller: Entity<SessionController>,
    pub input: Entity<TextInput>,
    pub attached: Option<slash::Attached>,
    pub expanded: bool,
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
        let input = cx
            .new(|cx| TextInput::new("Ask pi. Type / for commands, ! for shell…", cx).multiline(8));
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
            controller,
            input,
            attached: None,
            expanded: false,
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
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.input.focus_handle(cx).focus(window, cx);
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
            self.expanded = false;
            self.input.update(cx, |input, cx| {
                input.set_fill(false, cx);
                input.set_content("", cx);
            });
            cx.notify();
        }
    }
    fn composer(&self, window: &Window, cx: &Context<Self>, theme: Theme) -> impl IntoElement {
        let open = self.controller.read(cx);
        let model = open.model();
        let shell = self.attached.is_none() && self.input.read(cx).content().starts_with('!');
        let enabled = open.ready() && (!shell || open.can_navigate());
        let queue = model
            .steering
            .iter()
            .map(|message| ("STEER", message))
            .chain(model.follow_up.iter().map(|message| ("FOLLOW-UP", message)));
        v_flex()
            .debug_selector(|| "composer".into())
            .relative()
            .when(self.expanded, |composer| composer.h_full())
            .mx(px(20.))
            .mb(px(12.))
            .rounded(px(10.))
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
                    .gap(px(9.))
                    .px(px(12.))
                    .rounded_t(px(9.))
                    .bg(theme.queue)
                    .border_b_1()
                    .border_color(theme.queue_line)
                    .child(icon("queue", theme.amber))
                    .child(
                        label(kind, theme)
                            .text_size(px(9.5))
                            .text_color(theme.amber),
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
                    .when(index == 0, |row| {
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
                    })
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
            .children(self.attachments_row(cx, theme))
            .child(
                h_flex()
                    .relative()
                    .items_start()
                    .gap(px(8.))
                    .pl(px(16.))
                    .pr(px(40.))
                    .pt(px(13.))
                    .pb(px(6.))
                    .min_h(px(64.))
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
                    .pl(px(12.))
                    .pr(px(6.))
                    .pb(px(14.))
                    .gap(px(8.))
                    .child(
                        icon_button("attach", "attach", "Attach files or images", theme)
                            .debug_selector(|| "attach".into())
                            .w(px(18.))
                            .tooltip(ui::Tooltip::text(
                                "Attach files or images; you can also paste or drop them",
                            ))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.pick_files(window, cx)),
                            ),
                    )
                    .child(
                        icon_button("commands", "slash", "Choose a slash command", theme)
                            .w(px(18.))
                            .mr(px(5.))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_slash(window, cx)),
                            ),
                    )
                    .child(
                        chip("choose-model", "Choose model", theme)
                            .aria_expanded(self.picker == Some(menus::Picker::Model))
                            .child(icon("sparkle", theme.muted).size(px(13.)))
                            .child(
                                div().max_w(px(190.)).truncate().child(
                                    model
                                        .state
                                        .model
                                        .as_ref()
                                        .map(|model| model.id.clone())
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
                            .px(px(10.))
                            .aria_expanded(self.picker == Some(menus::Picker::Thinking))
                            .child(
                                div()
                                    .size(px(7.))
                                    .rounded_full()
                                    .bg(theme.thinking(&model.state.thinking_level)),
                            )
                            .child(if model.state.thinking_level.is_empty() {
                                "off".to_owned()
                            } else {
                                model.state.thinking_level.clone()
                            })
                            .child(icon("chevron_down", theme.faint).size(px(11.)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_picker(menus::Picker::Thinking, window, cx)
                            })),
                    )
                    .child(div().flex_1())
                    .when(model.busy() && !shell && model.shell.is_none(), |row| {
                        row.child(
                            button("follow-up", "Follow-up ⌥↵", theme)
                                .on_click(cx.listener(|this, _, _, cx| this.send_prompt(true, cx))),
                        )
                    })
                    .child(
                        primary_button(
                            "submit",
                            if shell {
                                "Run ↵"
                            } else if model.busy() {
                                "Steer ↵"
                            } else {
                                "Send ↵"
                            },
                            enabled,
                            theme,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.send_prompt(false, cx))),
                    )
                    .when(
                        model.busy() || !model.follow_up.is_empty() || !model.steering.is_empty(),
                        |row| {
                            row.child(
                                icon_button(
                                    "stop",
                                    "stop",
                                    "Stop and restore queued drafts",
                                    Theme {
                                        muted: theme.coral,
                                        ..theme
                                    },
                                )
                                .bg(theme.danger)
                                .border_1()
                                .border_color(theme.danger_line)
                                .text_color(theme.coral)
                                .on_click(cx.listener(|this, _, _, cx| this.clear_queue(true, cx))),
                            )
                        },
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
        div()
            .key_context("Composer")
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
