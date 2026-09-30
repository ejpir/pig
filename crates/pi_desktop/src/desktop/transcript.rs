mod activity;
use activity::{Activity, Block, Group};
use serde_json::Value;
use std::borrow::Cow;

use super::session::{Changes, ContentChange, SessionController, SessionEvent};
use super::*;
use gpui::{Div, Role};
use gpui::{FollowMode, ListAlignment, ListState, Subscription, list};
use markdown::MarkdownStyle;
use pi_core::skill::{SkillBlock, parse_skill_block};
use ui::{ContextMenu, right_click_menu};

use crate::markdown_view::{self, Source};

pub struct TranscriptView {
    controller: Entity<SessionController>,
    pub documents: markdown_view::Documents,
    pub list: ListState,
    expanded: HashSet<String>,
    activity: Activity,
    activity_choices: HashMap<Block, bool>,
    dirty: HashSet<usize>,
    row_keys: HashMap<usize, HashSet<SharedString>>,
    document_subscriptions: HashMap<SharedString, Subscription>,
    /// Images of sent messages, decoded once: by message, block and data length.
    images: HashMap<(usize, usize, usize), std::sync::Arc<gpui::Image>>,
    _subscription: Subscription,
    #[cfg(test)]
    pub renders: usize,
    #[cfg(test)]
    pub rows_rendered: usize,
    #[cfg(test)]
    pub rows_synced: usize,
}
impl TranscriptView {
    pub fn new(controller: Entity<SessionController>, cx: &mut Context<Self>) -> Self {
        let count = controller.read(cx).model().messages.len();
        let list = ListState::new(count.max(1) + 1, ListAlignment::Top, px(180.));
        if !controller.read(cx).is_demo() {
            list.set_follow_mode(FollowMode::Tail);
        }
        let subscription = cx.subscribe(&controller, |this, _, event, cx| match event {
            SessionEvent::Content(change) => this.content_changed(change, cx),
            SessionEvent::Changed(changes)
                if changes.intersects(Changes::RUN | Changes::METADATA | Changes::JJ) =>
            {
                this.refresh_activity(cx);
                // Run status, thinking-level colors and whether jj actions are
                // available affect already measured rows.
                this.list.remeasure();
                cx.notify();
            }
            SessionEvent::RevealTool(id) => {
                let row = this
                    .controller
                    .read(cx)
                    .model()
                    .messages
                    .iter()
                    .position(|m| {
                        m["content"].as_array().is_some_and(|blocks| {
                            blocks.iter().any(|b| b["id"].as_str() == Some(id))
                        })
                    });
                if let Some(row) = row {
                    this.pin_activity(row, true);
                    this.expanded.insert(id.clone());
                    this.dirty.insert(row);
                    this.list.pause_following_tail();
                    this.list.remeasure_items(row..row + 1);
                    this.list.scroll_to_reveal_item(row);
                    cx.notify();
                }
            }
            SessionEvent::RevealMessage(row) => {
                this.list.pause_following_tail();
                this.list.scroll_to_reveal_item(*row);
                cx.notify();
            }
            SessionEvent::RevealTail => {
                this.list.set_follow_mode(FollowMode::Tail);
                cx.notify();
            }
            _ => {}
        });
        Self {
            controller: controller.clone(),
            documents: Default::default(),
            list,
            expanded: HashSet::new(),
            activity: project_activity(controller.read(cx)),
            activity_choices: HashMap::new(),
            dirty: (0..count).collect(),
            row_keys: HashMap::new(),
            document_subscriptions: HashMap::new(),
            images: HashMap::new(),
            _subscription: subscription,
            #[cfg(test)]
            renders: 0,
            #[cfg(test)]
            rows_rendered: 0,
            #[cfg(test)]
            rows_synced: 0,
        }
    }
    fn content_changed(&mut self, change: &ContentChange, cx: &mut Context<Self>) {
        let model = self.controller.read(cx).model();
        let count = model.messages.len();
        if matches!(change, ContentChange::Reset) {
            self.documents.sync(vec![], cx);
            self.row_keys.clear();
            self.document_subscriptions.clear();
            self.expanded.clear();
            self.activity_choices.clear();
            self.dirty = (0..count).collect();
            self.list.reset(count.max(1) + 1);
        } else {
            let wanted = count.max(1) + 1;
            let previous = self.list.item_count();
            if previous != wanted {
                self.list.splice(
                    previous.saturating_sub(1)..previous,
                    wanted - previous.saturating_sub(1),
                );
            }
            let index = match change {
                ContentChange::Message(index) => Some(*index),
                ContentChange::Append(index) => {
                    self.dirty.extend(*index..count);
                    self.list.remeasure_items(*index..count);
                    None
                }
                ContentChange::Tool(id) => model.messages.iter().rposition(|message| {
                    message["content"].as_array().is_some_and(|blocks| {
                        blocks.iter().any(|block| block["id"].as_str() == Some(id))
                    })
                }),
                ContentChange::Reset => None,
            };
            if let Some(index) = index.filter(|index| *index < count) {
                self.dirty.insert(index);
                self.list.remeasure_items(index..index + 1);
            }
        }
        self.refresh_activity(cx);
        cx.notify();
    }
    fn refresh_activity(&mut self, cx: &mut Context<Self>) {
        let next = project_activity(self.controller.read(cx));
        let old: HashMap<_, _> = self.activity.groups.iter().map(|g| (g.key, g)).collect();
        let new: HashMap<_, _> = next.groups.iter().map(|g| (g.key, g)).collect();
        let mut rows = HashSet::new();
        for group in self.activity.groups.iter().chain(&next.groups) {
            if old.get(&group.key) != new.get(&group.key) {
                rows.extend(group.blocks.iter().map(|b| b.row));
            }
        }
        self.activity_choices.retain(|key, _| new.contains_key(key));
        self.activity = next;
        for row in rows {
            self.dirty.insert(row);
            self.list.remeasure_items(row..row + 1);
        }
    }
    #[cfg(test)]
    pub(super) fn disclose_activity_for_tool(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(group) = self
            .activity
            .groups
            .iter()
            .find(|g| g.tools.iter().any(|tool| tool == id))
            && !self.activity_open(group)
        {
            self.toggle_activity(group.key, cx);
        }
    }
    fn activity_open(&self, group: &Group) -> bool {
        self.activity_choices
            .get(&group.key)
            .copied()
            .unwrap_or_else(|| group.auto_open())
    }
    fn pin_activity(&mut self, row: usize, reveal: bool) {
        for group in &self.activity.groups {
            if group.blocks.iter().any(|b| b.row == row)
                && (reveal || self.activity_open(group))
                && self.activity_choices.insert(group.key, true) != Some(true)
            {
                for block in &group.blocks {
                    self.dirty.insert(block.row);
                    self.list.remeasure_items(block.row..block.row + 1);
                }
            }
        }
    }
    fn toggle_activity(&mut self, key: Block, cx: &mut Context<Self>) {
        let Some(group) = self.activity.group(key) else {
            return;
        };
        let open = !self.activity_open(group);
        let rows: HashSet<_> = group.blocks.iter().map(|b| b.row).collect();
        self.activity_choices.insert(key, open);
        self.list.pause_following_tail();
        for row in rows {
            self.dirty.insert(row);
            self.list.remeasure_items(row..row + 1);
            if !open && self.row_keys.contains_key(&row) {
                self.sync_row(row, cx);
            }
        }
        cx.notify();
    }
    fn toggle(&mut self, id: &str, row: usize, cx: &mut Context<Self>) {
        self.pin_activity(row, false);
        if !self.expanded.remove(id) {
            self.expanded.insert(id.to_owned());
        }
        self.dirty.insert(row);
        self.list.remeasure_items(row..row + 1);
        cx.notify();
    }
    fn sync_row(&mut self, row: usize, cx: &mut Context<Self>) {
        if !self.dirty.remove(&row) {
            return;
        }
        #[cfg(test)]
        {
            self.rows_synced += 1;
        }
        // An entity loan lets us borrow sources while updating separate Markdown entities.
        // No controller mutation or notification is needed for this projection.
        let keys: HashSet<_> = self.controller.clone().update(cx, |controller, cx| {
            let visible = |block: usize| {
                self.activity
                    .group(Block { row, index: block })
                    .is_none_or(|g| self.activity_open(g))
            };
            let sources = markdown_sources(controller.model(), row, &self.expanded, visible);
            let keys = sources.iter().map(|(key, _)| key.clone()).collect();
            self.documents.update(sources, cx);
            keys
        });
        if let Some(old) = self.row_keys.insert(row, keys.clone()) {
            for key in old.difference(&keys) {
                self.documents.remove(key);
                self.document_subscriptions.remove(key);
            }
        }
        for key in keys {
            if !self.document_subscriptions.contains_key(&key)
                && let Some(document) = self.documents.get(&key)
            {
                let subscription = cx.observe(document, move |this, document, cx| {
                    if document.read(cx).has_selection() {
                        this.list.pause_following_tail();
                        this.pin_activity(row, false);
                    }
                    if row < this.list.item_count() {
                        this.list.remeasure_items(row..row + 1);
                        cx.notify();
                    }
                });
                self.document_subscriptions.insert(key, subscription);
            }
        }
    }
}

impl TranscriptView {
    fn activity_header(&self, group: &Group, cx: &Context<Self>, theme: Theme) -> impl IntoElement {
        let key = group.key;
        let expanded = self.activity_open(group);
        h_flex()
            .id(SharedString::from(key.selector()))
            .debug_selector(move || key.selector())
            .role(Role::Button)
            .aria_label(group.label())
            .aria_expanded(expanded)
            .w_full()
            .h(px(28.))
            .my(px(4.))
            .px(px(8.))
            .gap(px(8.))
            .rounded(px(4.))
            .cursor_pointer()
            .hover(move |v| v.bg(theme.hover))
            .text_size(px(12.))
            .text_color(if group.failed > 0 {
                theme.coral
            } else {
                theme.muted
            })
            .child(
                icon(
                    if expanded {
                        "chevron_down"
                    } else {
                        "chevron_right"
                    },
                    theme.faint,
                )
                .size(px(12.)),
            )
            .when(
                !group.tools.is_empty()
                    && (group.failed > 0 || (!group.live && group.pending == 0)),
                |row| {
                    let failed = group.failed > 0;
                    row.child(
                        icon(
                            if failed { "warning" } else { "check" },
                            if failed { theme.coral } else { theme.green },
                        )
                        .id(SharedString::from(format!("{}-status", key.selector())))
                        .debug_selector(move || {
                            format!(
                                "{}-{}",
                                key.selector(),
                                if failed { "failed" } else { "passed" }
                            )
                        })
                        .size(px(13.)),
                    )
                },
            )
            .child(group.label())
            .tooltip(ui::Tooltip::text(
                "Show or hide original tool calls and reasoning. No generated summary.",
            ))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_activity(key, cx)))
    }
    /// What a finished `bash` call changed, when jj took snapshots around it and
    /// it changed anything, and whether that was restored since.
    fn command_files(&self, tool: &Tool, cx: &App) -> Option<(Vec<pi_jj::FileChange>, bool)> {
        if tool.name != "bash" {
            return None;
        }
        let command = self.controller.read(cx).jj().commands.get(&tool.id)?;
        let files = command.files.clone().filter(|files| !files.is_empty())?;
        Some((files, command.restored))
    }
    /// Restore to before (design study 05, 03), after a confirmation.
    fn confirm_restore_command(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let controller = self.controller.read(cx);
        let Some(files) = controller
            .jj()
            .commands
            .get(&id)
            .and_then(|command| command.files.clone())
        else {
            return;
        };
        let command = controller
            .model()
            .tools
            .iter()
            .find(|tool| tool.id == id)
            .map(|tool| tool.target())
            .unwrap_or_default();
        let first = command.lines().next().unwrap_or_default().to_owned();
        let mut list: Vec<String> = files.iter().take(8).map(|file| file.path.clone()).collect();
        if files.len() > 8 {
            list.push(format!("and {} more", files.len() - 8));
        }
        let answer = window.prompt(
            gpui::PromptLevel::Info,
            "Restore to before this command?",
            Some(&format!(
                "Brings back {} file{} as they were just before `{first}`:\n{}",
                files.len(),
                if files.len() == 1 { "" } else { "s" },
                list.join("\n")
            )),
            &[
                gpui::PromptButton::cancel("Cancel"),
                gpui::PromptButton::ok("Restore to before"),
            ],
            cx,
        );
        let controller = self.controller.clone();
        cx.spawn(async move |_, cx| {
            if answer.await == Ok(1) {
                controller.update(cx, |c, cx| c.restore_command(id, cx));
            }
        })
        .detach();
    }
    fn tool(
        &self,
        message: usize,
        tool: &Tool,
        output: &MarkdownStyle,
        cx: &Context<Self>,
        theme: Theme,
    ) -> AnyElement {
        let id = tool.id.clone();
        let expanded = self.expanded.contains(&id);
        let busy = self.controller.read(cx).model().busy();
        let color = if tool.is_error {
            theme.coral
        } else if tool.finished {
            theme.green
        } else {
            theme.accent
        };
        let verb = match tool.name.as_str() {
            "read" => "Read",
            "grep" => "Grep",
            "edit" => "Edit",
            "bash" => "Bash",
            "write" => "Write",
            name => name,
        };
        let target = if tool.name == "grep" {
            format!(
                "\"{}\" in {}",
                text(&tool.args, "pattern"),
                text(&tool.args, "path")
            )
        } else {
            tool.target()
        };
        // A multi-line command (a heredoc or script) keeps the header to one line.
        let (first_line, more_lines) = match target.split_once('\n') {
            Some((first, rest)) => (first.trim_end().to_owned(), rest.lines().count()),
            None => (target.clone(), 0),
        };
        let group = SharedString::from(format!("tool-row-{}", tool.id));
        let header = h_flex()
            .id(SharedString::from(format!("tool-{}", tool.id)))
            .group(group.clone())
            .role(Role::Button)
            .aria_expanded(expanded)
            .aria_label(format!("{verb} {first_line}"))
            .debug_selector({
                let id = tool.id.clone();
                move || format!("tool-header-{id}")
            })
            .h(px(24.))
            .gap(px(8.))
            .px(px(8.))
            .rounded(px(4.))
            .hover(move |row| row.bg(theme.hover))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle(&id, message, cx);
            }))
            .child(icon(
                match tool.name.as_str() {
                    "bash" => "terminal",
                    "edit" | "write" => "pencil",
                    "grep" => "magnifying_glass",
                    _ => "file",
                },
                theme.muted,
            ))
            .child(
                div()
                    .w(px(32.))
                    .text_size(px(12.))
                    .text_color(theme.muted)
                    .child(verb.to_owned()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .font_family(MONO)
                    .text_size(px(11.5))
                    .text_color(theme.secondary)
                    .debug_selector({
                        let id = tool.id.clone();
                        move || format!("tool-target-{id}")
                    })
                    .child(first_line),
            )
            .when(more_lines > 0, |row| {
                row.child(
                    div()
                        .flex_shrink_0()
                        .font_family(MONO)
                        .text_size(px(10.5))
                        .text_color(theme.faint)
                        .child(format!(
                            "+{more_lines} line{}",
                            if more_lines == 1 { "" } else { "s" }
                        )),
                )
            })
            // A finished command can be typed into a new terminal, to rerun or edit it.
            .when(tool.name == "bash" && tool.finished, |row| {
                let controller = self.controller.downgrade();
                let command = tool.target();
                let group = group.clone();
                row.child(
                    h_flex()
                        .id(SharedString::from(format!("open-in-terminal-{}", tool.id)))
                        .debug_selector({
                            let id = tool.id.clone();
                            move || format!("open-in-terminal-{id}")
                        })
                        .role(Role::Button)
                        .aria_label("Open in terminal")
                        .invisible()
                        .group_hover(group, |button| button.visible())
                        .flex_shrink_0()
                        .h(px(20.))
                        .px(px(8.))
                        .gap(px(5.))
                        .rounded(px(5.))
                        .border_1()
                        .border_color(theme.chip_line)
                        .bg(theme.chip)
                        .text_size(px(10.5))
                        .text_color(theme.secondary)
                        .hover(move |button| button.bg(theme.hover))
                        .child(icon("terminal", theme.secondary).size(px(12.)))
                        .child("Open in terminal")
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            controller
                                .update(cx, |controller, cx| {
                                    controller.open_in_terminal(command.clone(), cx)
                                })
                                .ok();
                        }),
                )
            })
            // What the command changed on disk, from jj snapshots around it.
            .when_some(self.command_files(tool, cx), |row, (files, restored)| {
                let count = files.len();
                let list = files
                    .iter()
                    .map(|file| format!("{}  +{} −{}", file.path, file.added, file.removed))
                    .collect::<Vec<_>>()
                    .join("\n");
                let id = tool.id.clone();
                row.when(!restored, |row| {
                    row.child(
                        h_flex()
                            .id(SharedString::from(format!("restore-command-{}", tool.id)))
                            .debug_selector({
                                let id = tool.id.clone();
                                move || format!("restore-command-{id}")
                            })
                            .role(Role::Button)
                            .aria_label("Restore to before")
                            .invisible()
                            .group_hover(group.clone(), |button| button.visible())
                            .flex_shrink_0()
                            .h(px(20.))
                            .px(px(8.))
                            .gap(px(5.))
                            .rounded(px(5.))
                            .border_1()
                            .border_color(theme.chip_line)
                            .bg(theme.chip)
                            .text_size(px(10.5))
                            .text_color(theme.secondary)
                            .hover(move |button| button.bg(theme.hover))
                            .child(icon("undo", theme.secondary).size(px(12.)))
                            .child("Restore to before")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.confirm_restore_command(id.clone(), window, cx);
                            })),
                    )
                })
                .child(
                    div()
                        .id(SharedString::from(format!("command-files-{}", tool.id)))
                        .debug_selector({
                            let id = tool.id.clone();
                            move || format!("command-files-{id}")
                        })
                        .flex_shrink_0()
                        .h(px(18.))
                        .px(px(6.))
                        .flex()
                        .items_center()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(if restored {
                            theme.chip_line
                        } else {
                            theme.amber.opacity(0.4)
                        })
                        .text_size(px(10.))
                        .text_color(if restored { theme.faint } else { theme.amber })
                        .tooltip(ui::Tooltip::text(list))
                        .child(format!(
                            "{} {count} file{}",
                            if restored { "restored" } else { "changed" },
                            if count == 1 { "" } else { "s" }
                        )),
                )
            })
            .when(tool.finished && tool.diff.is_some(), |row| {
                let (added, removed) =
                    crate::presentation::diff_counts(tool.diff.as_deref().unwrap_or(""));
                row.child(
                    h_flex()
                        .gap(px(5.))
                        .font_family(MONO)
                        .text_size(px(11.))
                        .child(div().text_color(theme.green).child(format!("+{added}")))
                        .child(div().text_color(theme.coral).child(format!("−{removed}"))),
                )
            })
            .when(
                tool.finished && tool.diff.is_none() && !tool.is_error,
                |row| row.child(icon("check", theme.green).size(px(13.))),
            )
            .when(!tool.finished || tool.is_error, |row| {
                row.child(
                    div()
                        .font_family(MONO)
                        .text_size(px(10.5))
                        .text_color(color)
                        .child(if tool.is_error {
                            "failed"
                        } else if busy {
                            "running"
                        } else {
                            "not completed"
                        }),
                )
            });
        let header = header.child(
            icon(
                if expanded {
                    "chevron_down"
                } else {
                    "chevron_right"
                },
                theme.faint,
            )
            .size(px(12.)),
        );
        let header = self.tool_menu(tool, header.into_any_element());
        // Every tool is a quiet, single-line call. Output is retained for explicit
        // disclosure/copy, never a card or a second result row by default.
        if !expanded {
            return header;
        }
        let mut body = v_flex().w_full().child(header);
        let mut details = v_flex()
            .debug_selector({
                let id = tool.id.clone();
                move || format!("tool-details-{id}")
            })
            .w_full()
            .bg(theme.deep)
            .rounded(px(4.))
            .mx(px(8.))
            .px(px(12.))
            .py(px(8.))
            .gap(px(8.));
        for field in tool_texts(tool) {
            let shown = shown_output(field.text);
            let selector = format!("tool-{}-{}", tool.id, field.part);
            details = details.child(v_flex().w_full().gap(px(4.))
                .child(label(field.label, theme).text_color(theme.faint))
                .child(div().w_full().debug_selector(move || selector)
                    .child(self.document(tool_text_key(&tool.id, field.part), shown, output, field.copy, None)))
                .when(shown.len() < field.text.len(), |body| body.child(div().text_size(px(10.)).text_color(theme.faint)
                    .child("Preview limited to 500 lines / 64 KiB. The tool header's copy menu has the full text."))));
        }
        if tool.name == "bash" && !tool.finished && busy {
            details = details.child(div().w(px(7.)).h(px(13.)).rounded(px(1.)).bg(theme.accent));
        }
        body = body.child(details);
        body.into_any_element()
    }

    /// Copy actions for a tool row. The menu reads the tool when it opens, so rendering
    /// never clones large outputs.
    fn tool_menu(&self, tool: &Tool, header: AnyElement) -> AnyElement {
        let controller = self.controller.downgrade();
        let id = tool.id.clone();
        right_click_menu(SharedString::from(format!("tool-menu-{}", tool.id)))
            .trigger(move |_, _, _| header)
            .menu(move |window, cx| {
                let tool = controller.upgrade().and_then(|controller| {
                    let model = controller.read(cx).model();
                    model.tools.iter().find(|tool| tool.id == id).cloned()
                });
                ContextMenu::build(window, cx, move |menu, _, _| {
                    let Some(tool) = tool else {
                        return menu;
                    };
                    let target = tool.target();
                    let label = match tool.name.as_str() {
                        "bash" => "Copy Command",
                        "grep" => "Copy Pattern",
                        _ => "Copy Path",
                    };
                    menu.when(!target.is_empty(), |menu| {
                        menu.entry(label, None, move |_, cx| markdown_view::copy(&target, cx))
                    })
                    .map(|mut menu| {
                        for field in tool_texts(&tool)
                            .into_iter()
                            .filter(|field| field.part != "command")
                        {
                            let text = field.text.to_owned();
                            menu = menu.entry(field.copy, None, move |_, cx| {
                                markdown_view::copy(&text, cx)
                            });
                        }
                        menu
                    })
                })
            })
            .into_any_element()
    }

    /// A selectable transcript block with a right-click copy menu.
    fn document(
        &self,
        key: SharedString,
        fallback: &str,
        style: &MarkdownStyle,
        copy_all: &'static str,
        on_link: Option<markdown_view::LinkHandler>,
    ) -> AnyElement {
        let document = self.documents.get(&key).cloned();
        let body = match &document {
            Some(document) => {
                let element = markdown_view::element(document, style.clone());
                match on_link {
                    Some(handler) => element.on_link(handler).into_any_element(),
                    None => element.into_any_element(),
                }
            }
            None => div()
                .text_color(style.base_text_style.color)
                .child(fallback.to_owned())
                .into_any_element(),
        };
        markdown_view::with_menu(
            SharedString::from(format!("menu-{key}")),
            document,
            copy_all,
            self.documents.copy_range(&key),
            body,
        )
    }

    fn markdown(&self, key: SharedString, fallback: &str, style: &MarkdownStyle) -> AnyElement {
        self.document(key, fallback, style, "Copy Message", None)
    }

    /// A sent message; its `@` mentions show as chips (see `mentions`). A file chip
    /// opens the file; a text chip shows or hides the text pi received.
    /// A sent message's images, decoded once.
    fn user_images(&mut self, index: usize, content: &Value) -> Vec<std::sync::Arc<gpui::Image>> {
        use base64::Engine as _;
        content
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter(|(_, block)| block["type"] == "image")
            .filter_map(|(block, image)| {
                let data = image["data"].as_str()?;
                let key = (index, block, data.len());
                if let Some(image) = self.images.get(&key) {
                    return Some(image.clone());
                }
                let format = super::attachments::format_of_mime(image["mimeType"].as_str()?)?;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .ok()?;
                let image = std::sync::Arc::new(gpui::Image::from_bytes(format, bytes));
                self.images.insert(key, image.clone());
                Some(image)
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn user_bubble(
        &self,
        index: usize,
        request: &str,
        images: Vec<std::sync::Arc<gpui::Image>>,
        timestamp: Option<u64>,
        style: &MarkdownStyle,
        cx: &Context<Self>,
        theme: Theme,
    ) -> Div {
        let group = SharedString::from(format!("user-message-{index}"));
        let sent = mentions::parse_sent(request);
        let view = cx.entity().downgrade();
        let on_link: markdown_view::LinkHandler =
            std::rc::Rc::new(move |url, _, cx| match mentions::parse_link(&url) {
                Some(mentions::Link::Block(block)) => {
                    view.update(cx, |this, cx| {
                        this.toggle(&format!("mention-{index}-{block}"), index, cx)
                    })
                    .ok();
                }
                Some(mentions::Link::File { path, .. }) => {
                    view.update(cx, |this, cx| {
                        this.controller
                            .update(cx, |controller, cx| controller.open_file(path, cx))
                    })
                    .ok();
                }
                Some(mentions::Link::Directory { path }) => {
                    view.update(cx, |this, cx| {
                        this.controller
                            .update(cx, |_, cx| cx.emit(SessionEvent::OpenDirectory(path)))
                    })
                    .ok();
                }
                None => cx.open_url(&url),
            });
        let shown: Vec<(usize, mentions::Block)> = sent
            .as_ref()
            .map(|sent| {
                sent.blocks
                    .iter()
                    .cloned()
                    .enumerate()
                    .filter(|(block, _)| {
                        self.expanded.contains(&format!("mention-{index}-{block}"))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let text = sent.as_ref().map_or(request, |sent| sent.markdown.as_str());
        div()
            .relative()
            .group(group.clone())
            .rounded(px(8.))
            .border_1()
            .border_color(theme.user_line)
            .bg(theme.user)
            .px(px(16.))
            .py(px(9.))
            .mb(px(2.))
            .pr(px(64.))
            // The time stays out of the way until the message is pointed at.
            .when_some(timestamp, |bubble, timestamp| {
                bubble.child(
                    div()
                        .id(("message-time", index))
                        .absolute()
                        .right(px(12.))
                        .top(px(9.))
                        .invisible()
                        .group_hover(group, |time| time.visible())
                        .font_family(MONO)
                        .text_size(px(10.))
                        .text_color(theme.faint)
                        .aria_label("Message time in UTC")
                        .child(clock(timestamp)),
                )
            })
            .child(self.document(
                doc_key(index, "user"),
                text,
                style,
                "Copy Message",
                Some(on_link),
            ))
            .when(!images.is_empty(), |bubble| {
                bubble.child(h_flex().flex_wrap().gap(px(8.)).mt(px(8.)).children(
                    images.into_iter().enumerate().map(|(n, image)| {
                        div()
                            .debug_selector(move || format!("user-image-{index}-{n}"))
                            .child(
                                gpui::img(image)
                                    .max_h(px(160.))
                                    .max_w(px(260.))
                                    .rounded(px(6.)),
                            )
                    }),
                ))
            })
            .children(shown.into_iter().map(|(block, mention)| {
                v_flex()
                    .debug_selector(move || format!("mention-block-{index}-{block}"))
                    .mt(px(8.))
                    .mr(px(-48.))
                    .p(px(10.))
                    .gap(px(4.))
                    .rounded(px(6.))
                    .bg(theme.deep)
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .text_size(px(11.))
                            .text_color(theme.muted)
                            .child(icon(mention.kind.icon(), theme.muted).size(px(12.)))
                            .child(mention.title)
                            .child(div().text_color(theme.faint).child("· as pi received it")),
                    )
                    .child(
                        v_flex()
                            .font_family(MONO)
                            .text_size(px(11.))
                            .line_height(px(17.))
                            .text_color(theme.secondary)
                            .children(
                                mention
                                    .body
                                    .lines()
                                    .map(|line| div().child(line.to_owned())),
                            ),
                    )
            }))
    }

    /// pi expands `/skill:name` into the skill file; show that as one collapsed card.
    fn skill_card(
        &self,
        index: usize,
        skill: &SkillBlock,
        timestamp: Option<u64>,
        style: &MarkdownStyle,
        cx: &Context<Self>,
        theme: Theme,
    ) -> AnyElement {
        let id = format!("skill-{index}");
        let expanded = self.expanded.contains(&id);
        let header = h_flex()
            .id(SharedString::from(id.clone()))
            .debug_selector(move || format!("skill-card-{index}"))
            .role(Role::Button)
            .aria_label(format!("Skill {}", skill.name))
            .aria_expanded(expanded)
            .h(px(32.))
            .px(px(12.))
            .gap(px(8.))
            .bg(theme.panel)
            .cursor_pointer()
            .hover(move |row| row.bg(theme.hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle(&id, index, cx);
            }))
            .child(icon("sparkle", theme.accent).size(px(14.)))
            .child(label("SKILL", theme).text_color(theme.accent))
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(skill.name.to_owned()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .text_color(theme.faint)
                    .child("added to context"),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_right()
                    .font_family(MONO)
                    .text_size(px(10.5))
                    .text_color(theme.faint)
                    .child(short_path(skill.location)),
            )
            .when_some(timestamp, |row, timestamp| {
                row.child(
                    div()
                        .font_family(MONO)
                        .text_size(px(10.))
                        .text_color(theme.faint)
                        .child(clock(timestamp)),
                )
            })
            .child(icon(
                if expanded {
                    "chevron_down"
                } else {
                    "chevron_right"
                },
                theme.faint,
            ));
        v_flex()
            .rounded(px(8.))
            .overflow_hidden()
            .border_1()
            .border_color(theme.line)
            .mb(px(6.))
            .child(header)
            .when(expanded, |card| {
                card.child(
                    div()
                        .debug_selector(move || format!("skill-body-{index}"))
                        .border_t_1()
                        .border_color(theme.line)
                        .bg(theme.canvas)
                        .px(px(16.))
                        .py(px(10.))
                        .child(self.markdown(doc_key(index, "skill"), skill.content, style)),
                )
            })
            .into_any_element()
    }

    fn render_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        #[cfg(test)]
        {
            self.rows_rendered += 1;
        }
        self.sync_row(index, cx);
        let model = self.controller.read(cx).model();
        let theme = theme(cx);
        let mut prose = markdown_view::style(theme, theme.secondary, window, cx);
        prose.base_text_style.line_height = px(22.).into();
        prose.container_style = prose.container_style.line_height(px(22.));
        prose.paragraph_line_height = px(22.).into();
        prose.paragraph_spacing = px(12.);
        let quiet = markdown_view::style(theme, theme.muted, window, cx);
        let mut request = markdown_view::style(theme, theme.text, window, cx);
        // Mention chips: accent text on Markdown's inline-code chip, not underlined.
        request.link_callback = Some(std::rc::Rc::new(move |url: &str, _: &App| {
            url.starts_with(mentions::LINK_SCHEME)
                .then(|| gpui::TextStyleRefinement {
                    color: Some(theme.accent),
                    ..Default::default()
                })
        }));
        let output = markdown_view::output_style(theme, window, cx);
        let mut content = v_flex().w_full();
        if index >= model.messages.len().max(1) {
            // pi confirms a run with `agent_start`; show work from the moment Enter is pressed.
            let open = self.controller.read(cx);
            if open.working() {
                content = content.child(
                    h_flex()
                        .debug_selector(|| "working".into())
                        .h(px(28.))
                        .mt(px(4.))
                        .px(px(8.))
                        .gap(px(9.))
                        .text_size(px(12.))
                        .italic()
                        .text_color(theme.muted)
                        .child(spinner("transcript-working", theme))
                        .child(match model.run {
                            pi_core::session::RunState::Compacting => "Compacting…",
                            pi_core::session::RunState::Retrying => "Retrying…",
                            _ if model.shell_running() => "Running shell command…",
                            _ if self.activity.groups.iter().any(|g| g.live && g.pending > 0) => {
                                "Running tools…"
                            }
                            _ if model
                                .messages
                                .last()
                                .and_then(|m| m["content"].as_array())
                                .and_then(|blocks| blocks.last())
                                .is_some_and(|b| b["type"] == "thinking") =>
                            {
                                "Thinking…"
                            }
                            _ => "Working…",
                        }),
                );
            }
            return content.into_any_element();
        }
        if model.messages.is_empty() {
            content = content.child(
                v_flex()
                    .py(px(65.))
                    .gap(px(14.))
                    .items_center()
                    .child(
                        div()
                            .text_size(px(30.))
                            .font_family(SERIF)
                            .italic()
                            .child("A quiet place to work."),
                    )
                    .child(
                        div()
                            .text_color(theme.muted)
                            .child("Start a conversation in this project."),
                    )
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(11.))
                            .text_color(theme.faint)
                            .child(model.cwd.display().to_string()),
                    ),
            );
        }
        let Some(message) = model.messages.get(index) else {
            return content.into_any_element();
        };
        let timestamp = message["timestamp"].as_u64();
        match message["role"].as_str() {
            Some("user") => {
                content = content.mt(px(if index == 0 { 0. } else { 24. })).mb(px(8.));
                let body = content_text(&message["content"]);
                let images = self.user_images(index, &message["content"]);
                match parse_skill_block(&body) {
                    Some(skill) => {
                        content = content.child(self.skill_card(
                            index,
                            &skill,
                            timestamp.filter(|_| skill.user_message.is_none()),
                            &quiet,
                            cx,
                            theme,
                        ));
                        if let Some(message) = skill.user_message {
                            content = content.child(self.user_bubble(
                                index, message, images, timestamp, &request, cx, theme,
                            ));
                        }
                    }
                    None => {
                        content = content.child(
                            self.user_bubble(index, &body, images, timestamp, &request, cx, theme),
                        )
                    }
                }
            }
            Some("assistant") => {
                // How the reply ended, if not normally, stays next to it.
                let ending = message["errorMessage"].as_str().map(|error| {
                    let stopped = message["stopReason"] == "aborted";
                    h_flex()
                        .h(px(24.))
                        .px(px(8.))
                        .gap(px(7.))
                        .text_size(px(12.))
                        .text_color(if stopped { theme.faint } else { theme.coral })
                        .child(
                            icon(
                                if stopped { "stop" } else { "warning" },
                                if stopped { theme.faint } else { theme.coral },
                            )
                            .size(px(12.)),
                        )
                        .child(if stopped {
                            "Stopped".to_owned()
                        } else {
                            error.to_owned()
                        })
                });
                if let Some(blocks) = message["content"].as_array() {
                    for (block_index, block) in blocks.iter().enumerate() {
                        if let Some(group) = self.activity.group(Block {
                            row: index,
                            index: block_index,
                        }) {
                            if group.key
                                == (Block {
                                    row: index,
                                    index: block_index,
                                })
                            {
                                content = content.child(self.activity_header(group, cx, theme));
                            }
                            if !self.activity_open(group) {
                                continue;
                            }
                            // Automatic live expansion shows calls, not repeated thinking rows.
                            if block["type"] == "thinking"
                                && !self
                                    .activity_choices
                                    .get(&group.key)
                                    .copied()
                                    .unwrap_or(false)
                            {
                                continue;
                            }
                        }
                        match block["type"].as_str() {
                            Some("text") if text(block, "text").trim().is_empty() => {}
                            Some("text") => {
                                content = content.child(
                                    div()
                                        .w_full()
                                        .debug_selector(move || {
                                            format!("assistant-prose-{index}-{block_index}")
                                        })
                                        .px(px(8.))
                                        .mt(px(8.))
                                        .mb(px(12.))
                                        .child(self.markdown(
                                            doc_key(index, &format!("text-{block_index}")),
                                            &text(block, "text"),
                                            &prose,
                                        )),
                                )
                            }
                            // Some providers send empty or redacted thinking; there is nothing to show.
                            Some("thinking")
                                if block["thinking"]
                                    .as_str()
                                    .is_none_or(|thinking| thinking.trim().is_empty()) => {}
                            Some("thinking") => {
                                let id = format!("thinking-{index}-{block_index}");
                                let expanded = self.expanded.contains(&id);
                                let mut thought = v_flex().child(
                                    h_flex()
                                        .id(SharedString::from(id.clone()))
                                        .gap(px(8.))
                                        .px(px(8.))
                                        .h(px(25.))
                                        .text_color(theme.muted)
                                        .italic()
                                        .cursor_pointer()
                                        .text_size(px(12.))
                                        .child(
                                            icon(
                                                "sparkle",
                                                theme.thinking(&model.state.thinking_level),
                                            )
                                            .size(px(14.)),
                                        )
                                        .child("Thinking")
                                        .child(icon(
                                            if expanded {
                                                "chevron_down"
                                            } else {
                                                "chevron_right"
                                            },
                                            theme.faint,
                                        ))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.toggle(&id, index, cx);
                                        })),
                                );
                                if expanded {
                                    thought = thought.child(div().w_full().px(px(16.)).child(
                                        self.markdown(
                                            doc_key(index, &format!("thinking-{block_index}")),
                                            &text(block, "thinking"),
                                            &quiet,
                                        ),
                                    ));
                                }
                                content = content.child(thought);
                            }
                            Some("toolCall") => {
                                if let Some(tool) = model
                                    .tools
                                    .iter()
                                    .find(|tool| Some(tool.id.as_str()) == block["id"].as_str())
                                {
                                    content =
                                        content.child(self.tool(index, tool, &output, cx, theme));
                                }
                            }
                            Some("image") => {
                                content = content.child(
                                    div().text_color(theme.muted).child("[Image attachment]"),
                                )
                            }
                            _ => {}
                        }
                    }
                }
                content = content.children(ending);
            }
            Some("bashExecution") => {
                let command = text(message, "command");
                let result = text(message, "output");
                let excluded = message["excludeFromContext"] == true;
                let status = if message["cancelled"] == true {
                    "Cancelled".to_owned()
                } else {
                    message["exitCode"]
                        .as_i64()
                        .map(|code| format!("Exit {code}"))
                        .unwrap_or_else(|| "Exit not reported".into())
                };
                let copied_command = command.clone();
                let copied_output = result.clone();
                content = content.child(
                    v_flex()
                        .debug_selector(move || format!("shell-result-{index}"))
                        .p(px(12.))
                        .gap(px(8.))
                        .rounded(px(7.))
                        .bg(theme.deep)
                        .child(
                            h_flex()
                                .gap(px(8.))
                                .child(icon("terminal", theme.muted))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .font_family(MONO)
                                        .text_size(px(12.))
                                        .child(format!("$ {command}")),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(
                                            if message["exitCode"]
                                                .as_i64()
                                                .is_some_and(|code| code != 0)
                                            {
                                                theme.coral
                                            } else {
                                                theme.muted
                                            },
                                        )
                                        .child(status),
                                )
                                .child(
                                    icon_button(
                                        ("copy-shell-command", index),
                                        "copy",
                                        "Copy command",
                                        theme,
                                    )
                                    .debug_selector(move || format!("copy-shell-command-{index}"))
                                    .size(px(20.))
                                    .tooltip(ui::Tooltip::text("Copy command"))
                                    .on_click(
                                        move |_, _, cx| markdown_view::copy(&copied_command, cx),
                                    ),
                                )
                                .child(
                                    icon_button(
                                        ("copy-shell-output", index),
                                        "copy",
                                        "Copy output",
                                        theme,
                                    )
                                    .debug_selector(move || format!("copy-shell-output-{index}"))
                                    .size(px(20.))
                                    .tooltip(ui::Tooltip::text("Copy output"))
                                    .on_click(
                                        move |_, _, cx| markdown_view::copy(&copied_output, cx),
                                    ),
                                ),
                        )
                        .child(self.document(
                            doc_key(index, "shell-output"),
                            &result,
                            &output,
                            "Copy Output",
                            None,
                        ))
                        .child(div().text_size(px(10.)).text_color(theme.faint).child(
                            if excluded {
                                "!! · excluded from model context"
                            } else {
                                "! · included in model context on the next prompt"
                            },
                        ))
                        .when(message["truncated"] == true, |card| {
                            card.child(div().text_size(px(11.)).text_color(theme.amber).child(
                                format!(
                                        "Pi truncated this output. Full output: {}",
                                        message["fullOutputPath"]
                                            .as_str()
                                            .unwrap_or("path not reported")
                                    ),
                            ))
                        }),
                );
            }
            Some("compactionSummary" | "branchSummary") => {
                content = content.child(div().p(px(12.)).child(self.markdown(
                    doc_key(index, "summary"),
                    &text(message, "summary"),
                    &quiet,
                )))
            }
            Some("custom") if message["display"] == true => {
                content = content.child(div().p(px(12.)).child(self.markdown(
                    doc_key(index, "custom"),
                    &content_text(&message["content"]),
                    &quiet,
                )))
            }
            _ => {}
        }
        let controller = self.controller.read(cx);
        let idle = controller.jj_idle() && controller.jj().project.is_some();
        for (record_index, record) in controller.jj().records.iter().enumerate() {
            if record.anchored && record.after_message == index {
                content = content.child(turn_line(record_index, record, idle, cx, theme));
            }
        }
        content.into_any_element()
    }
}

/// One quiet line under a turn that changed files: its jj change and counts,
/// with Undo on hover; once undone, what was restored and Redo.
fn turn_line(
    index: usize,
    record: &super::jj::TurnRecord,
    idle: bool,
    cx: &Context<TranscriptView>,
    theme: Theme,
) -> AnyElement {
    let group = SharedString::from(format!("jj-turn-{index}"));
    let row = h_flex()
        .group(group.clone())
        .debug_selector(move || format!("jj-turn-{index}"))
        .h(px(26.))
        .mt(px(2.))
        .mb(px(12.))
        .rounded(px(5.))
        .hover(move |row| row.bg(theme.hover))
        .px(px(8.))
        .gap(px(8.))
        .font_family(MONO)
        .text_size(px(10.5))
        .text_color(theme.faint);
    let action = |id: &'static str, name: &'static str, glyph: &'static str| {
        button((id, index), name, theme)
            .h(px(20.))
            .child(icon(glyph, theme.muted).size(px(12.)))
            .flex_row_reverse()
    };
    match &record.undone {
        None => row
            .child(icon("git_commit", theme.faint).size(px(12.)))
            .child(record.short.clone())
            .child(record.files_label())
            .child(
                div()
                    .text_color(theme.green)
                    .child(format!("+{}", record.added)),
            )
            .child(
                div()
                    .text_color(theme.coral)
                    .child(format!("−{}", record.removed)),
            )
            .child(div().flex_1())
            .child(
                h_flex()
                    .gap(px(6.))
                    .invisible()
                    .group_hover(group, |style| style.visible())
                    .child(
                        action("jj-diff", "Diff", "git_commit").on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.controller
                                    .update(cx, |controller, cx| controller.review_turn(index, cx));
                            },
                        )),
                    )
                    .when(idle, |v| {
                        v.child(action("jj-undo", "Undo turn", "undo").on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.controller
                                    .update(cx, |controller, cx| controller.undo_turn(index, cx))
                            },
                        )))
                    }),
            )
            .into_any_element(),
        Some(_) => row
            .child(icon("undo", theme.muted).size(px(12.)))
            .child(
                div()
                    .font_family(SANS)
                    .text_size(px(12.))
                    .text_color(theme.muted)
                    .child("Turn undone"),
            )
            .child(format!(
                "· {} restored, change {} abandoned",
                record.files_label(),
                record.short
            ))
            .child(div().flex_1())
            .when(idle, |row| {
                row.child(action("jj-redo", "Redo", "redo").on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.controller
                            .update(cx, |controller, cx| controller.redo_turn(index, cx))
                    },
                )))
            })
            .into_any_element(),
    }
}

impl Render for TranscriptView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.renders += 1;
        }
        let view = cx.entity().downgrade();
        let content = list(self.list.clone(), move |index, window, cx| {
            view.update(cx, |this, cx| this.render_row(index, window, cx))
                .unwrap_or_else(|_| div().into_any_element())
        })
        .size_full()
        .pt(px(14.))
        .pb(px(12.));
        div()
            .id("transcript")
            .size_full()
            .pl(px(24.))
            .pr(px(20.))
            .child(content)
            .custom_scrollbars(
                Scrollbars::always_visible(ScrollAxes::Vertical)
                    .id("transcript-scrollbar")
                    .tracked_scroll_handle(&self.list),
                window,
                cx,
            )
    }
}

fn project_activity(controller: &SessionController) -> Activity {
    let anchors = controller
        .jj()
        .records
        .iter()
        .filter(|r| r.anchored)
        .map(|r| r.after_message)
        .collect();
    Activity::project(controller.model(), controller.working(), &anchors)
}

/// Documents the transcript shows, keyed like `transcript` looks them up. Text is
/// borrowed from the session so unchanged blocks cost no copies.
fn markdown_sources<'a>(
    model: &'a Session,
    row: usize,
    expanded: &HashSet<String>,
    visible: impl Fn(usize) -> bool,
) -> Vec<(SharedString, Source<'a>)> {
    let mut sources = Vec::new();
    for (index, message) in model.messages.iter().enumerate().skip(row).take(1) {
        match message["role"].as_str() {
            Some("user") => match message_text(&message["content"]) {
                Cow::Borrowed(body) => user_sources(index, body, Cow::Borrowed, &mut sources),
                Cow::Owned(body) => user_sources(
                    index,
                    &body,
                    |text| Cow::Owned(text.to_owned()),
                    &mut sources,
                ),
            },
            Some("assistant") => {
                let blocks = message["content"].as_array().into_iter().flatten();
                for (block_index, block) in blocks.enumerate() {
                    let (part, field) = match block["type"].as_str() {
                        Some("text") => ("text", "text"),
                        Some("thinking")
                            if visible(block_index)
                                && expanded
                                    .contains(&format!("thinking-{index}-{block_index}")) =>
                        {
                            ("thinking", "thinking")
                        }
                        _ => continue,
                    };
                    sources.push((
                        doc_key(index, &format!("{part}-{block_index}")),
                        Source::Markdown(Cow::Borrowed(block[field].as_str().unwrap_or(""))),
                    ));
                }
            }
            Some("bashExecution") => sources.push((
                doc_key(index, "shell-output"),
                Source::Text(message["output"].as_str().unwrap_or("")),
            )),
            Some("compactionSummary" | "branchSummary") => sources.push((
                doc_key(index, "summary"),
                Source::Markdown(Cow::Borrowed(message["summary"].as_str().unwrap_or(""))),
            )),
            Some("custom") if message["display"] == true => sources.push((
                doc_key(index, "custom"),
                Source::Markdown(message_text(&message["content"])),
            )),
            _ => {}
        }
    }
    let ids: HashSet<_> = model
        .messages
        .get(row)
        .and_then(|m| m["content"].as_array())
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(index, _)| visible(*index))
        .filter_map(|(_, block)| block["id"].as_str())
        .collect();
    for tool in model
        .tools
        .iter()
        .filter(|tool| ids.contains(tool.id.as_str()) && expanded.contains(&tool.id))
    {
        for field in tool_texts(tool) {
            let text = shown_output(field.text);
            sources.push((
                tool_text_key(&tool.id, field.part),
                match field.language {
                    Some(language) => Source::Code { text, language },
                    None => Source::Text(text),
                },
            ));
        }
    }
    sources
}

fn user_sources<'a, 'b>(
    index: usize,
    body: &'b str,
    to_source: impl Fn(&'b str) -> Cow<'a, str>,
    sources: &mut Vec<(SharedString, Source<'a>)>,
) {
    match parse_skill_block(body) {
        Some(skill) => {
            sources.push((
                doc_key(index, "skill"),
                Source::Markdown(to_source(skill.content)),
            ));
            if let Some(message) = skill.user_message {
                sources.push((
                    doc_key(index, "user"),
                    Source::Markdown(user_text(message, &to_source)),
                ));
            }
        }
        None => sources.push((
            doc_key(index, "user"),
            Source::Markdown(user_text(body, &to_source)),
        )),
    }
}

/// A sent message's Markdown, with its mentions as chip links.
fn user_text<'a, 'b>(body: &'b str, to_source: impl Fn(&'b str) -> Cow<'a, str>) -> Cow<'a, str> {
    match mentions::parse_sent(body) {
        Some(sent) => Cow::Owned(sent.markdown),
        None => to_source(body),
    }
}

/// `content_text`, borrowed when the content is a single string or text block.
fn message_text(content: &Value) -> Cow<'_, str> {
    if let Some(text) = content.as_str() {
        return Cow::Borrowed(text);
    }
    let mut texts = content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| block["text"].as_str());
    match (texts.next(), texts.next()) {
        (Some(text), None) => Cow::Borrowed(text),
        _ => Cow::Owned(content_text(content)),
    }
}

/// Tool cards show at most this much output; the full text stays in the session.
fn shown_output(output: &str) -> &str {
    let mut end = output
        .match_indices('\n')
        .nth(499)
        .map_or(output.len(), |(end, _)| end)
        .min(64 * 1024);
    while !output.is_char_boundary(end) {
        end -= 1;
    }
    &output[..end]
}

struct ToolText<'a> {
    part: &'static str,
    label: &'static str,
    copy: &'static str,
    text: &'a str,
    language: Option<&'a str>,
}
fn tool_texts(tool: &Tool) -> Vec<ToolText<'_>> {
    let mut fields = Vec::new();
    let path = tool.args["path"].as_str().unwrap_or("");
    let language = std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    match tool.name.as_str() {
        "edit" => {
            if let Some(diff) = tool.diff.as_deref().filter(|diff| !diff.is_empty()) {
                fields.push(ToolText {
                    part: "diff",
                    label: "DIFF",
                    copy: "Copy Diff",
                    text: diff,
                    language: Some("diff"),
                });
            } else {
                for (part, label, copy) in [
                    ("oldText", "BEFORE", "Copy Before"),
                    ("newText", "AFTER", "Copy After"),
                ] {
                    if let Some(text) = tool.args[part].as_str() {
                        fields.push(ToolText {
                            part,
                            label,
                            copy,
                            text,
                            language: Some(language),
                        });
                    }
                }
            }
        }
        "write" => {
            if let Some(text) = tool.args["content"].as_str() {
                fields.push(ToolText {
                    part: "content",
                    label: "CONTENT",
                    copy: "Copy Content",
                    text,
                    language: Some(language),
                });
            }
        }
        "bash" => {
            if let Some(text) = tool.args["command"].as_str() {
                fields.push(ToolText {
                    part: "command",
                    label: "COMMAND",
                    copy: "Copy Command",
                    text,
                    language: Some("bash"),
                });
            }
        }
        _ => {}
    }
    if !tool.output.is_empty() {
        fields.push(ToolText {
            part: "output",
            label: "OUTPUT",
            copy: "Copy Output",
            text: &tool.output,
            language: None,
        });
    }
    fields
}
fn tool_text_key(id: &str, part: &str) -> SharedString {
    format!("tool:{id}:{part}").into()
}

fn doc_key(message: usize, part: &str) -> SharedString {
    format!("{message}:{part}").into()
}

/// Message times are shown in UTC; pi records epoch milliseconds.
fn clock(timestamp: u64) -> String {
    format!(
        "{:02}:{:02}",
        timestamp / 3_600_000 % 24,
        timestamp / 60_000 % 60
    )
}
