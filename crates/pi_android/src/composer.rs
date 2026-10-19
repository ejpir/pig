//! The composer: the draft, what is attached to it, suggestions for the `@`
//! file or `/` command being typed, the model, and Send. Choosing a
//! suggestion or a model only edits the draft; only Send sends. Direct
//! subagent input uses the same layout without the optional selection controls.

use crate::{
    prefs::{Prefs, SavedCommand},
    text_area::{TextArea, TextAreaEvent},
    theme::{MONO, theme},
    ui::{self, icon},
};
use gpui::{
    App, Context, Edges, Entity, EventEmitter, Focusable, FontWeight, Image, SharedString,
    Subscription, Window, div, img, prelude::*, px,
};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub enum Attachment {
    Image {
        name: String,
        image: Arc<Image>,
    },
    File {
        name: String,
        size: String,
        contents: String,
    },
    /// Lines chosen in a review: "lines 211–212".
    Lines(String),
}

impl Attachment {
    pub fn label(&self) -> String {
        match self {
            Self::Image { name, .. } | Self::File { name, .. } => name.clone(),
            Self::Lines(lines) => lines.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ComposerEvent {
    Send {
        text: String,
        attachments: Vec<Attachment>,
    },
    /// Stage one line-anchored section for a later batched review follow-up.
    StageReview {
        text: String,
        attachments: Vec<Attachment>,
    },
    /// Close the line-anchored review editor without discarding its draft.
    CancelReview,
    /// The paperclip: the app offers files and the clipboard.
    Attach,
    /// Independent controls, shared by new sessions, later prompts and reviews.
    ChooseModel,
    ChooseThinking,
    PreviewImage(usize),
}

/// Files the `@` suggestions offer, as on the computer's project.
const FILES: &[&str] = &[
    "deepseek.ts",
    "deepseek.test.ts",
    "deep/",
    "openai-completions.ts",
    "qwen.test.ts",
    "README.md",
    "package.json",
    "stream.ts",
    "retry.ts",
];

/// Commands the sample's `/` suggestions offer: its prompt templates and a skill.
const COMMANDS: &[(&str, &str)] = &[
    ("fix-tests", "Fix the failing tests"),
    ("review", "Review the local changes"),
    ("explain", "Explain this project"),
    ("skill:lint", "Lint the code with the project's rules"),
];

/// The sample's command catalog, as a computer sends one.
pub fn sample_commands() -> Vec<SavedCommand> {
    COMMANDS
        .iter()
        .map(|(name, description)| SavedCommand {
            name: (*name).into(),
            description: Some((*description).into()),
        })
        .collect()
}

/// Recognizes the app-owned `/compact` action. A session-local command with the
/// same exact name shadows it and remains ordinary prompt text for Pi to expand.
pub(crate) fn compact_instructions(
    draft: &str,
    remote_compact_shadows: bool,
) -> Option<Option<String>> {
    if remote_compact_shadows {
        return None;
    }
    let rest = draft.strip_prefix('/')?;
    let (name, arguments) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    (name == "compact").then(|| {
        let instructions = arguments.trim();
        (!instructions.is_empty()).then(|| instructions.to_owned())
    })
}

/// A chip in the suggestion strip. Commands attach structurally so they can be
/// chosen at the caret while still being serialized first for Pi on Send.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Suggestion {
    label: String,
    replacement: String,
    glyph: &'static str,
    command: Option<SavedCommand>,
}

pub struct Composer {
    pub area: Entity<TextArea>,
    attachments: Vec<Attachment>,
    /// A session's actual model, instead of the default for new sessions.
    model_label: Option<String>,
    thinking_label: Option<String>,
    /// A status strip shares the composer's top edge.
    joined: bool,
    /// Whether the composer exposes the `/` button.
    command_button: bool,
    /// Existing sessions add app-owned action commands to Pi's catalog.
    session_commands: bool,
    /// A child-conversation composer exposes only text and Send.
    message_only: bool,
    /// A session's composer is one line while you aren't typing in it; the
    /// model and effort show once it has focus or a draft.
    compact_when_idle: bool,
    /// Draw the review composer as a comment card inside the diff.
    inline_review: bool,
    send_blocked: bool,
    /// The exact child payload remains visible but cannot be sent twice in flight.
    sending: bool,
    /// The draft's least height: taller for a new task.
    draft_height: f32,
    imports: usize,
    import_generation: u64,
    /// What `@` offers: the files a session touched; `None` for the sample's
    /// built-in files.
    files: Option<Vec<String>>,
    workspace: bool,
    files_loading: bool,
    files_error: Option<String>,
    /// What `/` offers. `None` selects the deterministic sample catalog;
    /// remote sessions always supply `Some`, including an authoritative empty list.
    commands: Option<Vec<SavedCommand>>,
    /// A picker-selected command is independent of the draft's caret position.
    /// Send puts it first, as Pi's slash-command protocol requires.
    attached_command: Option<SavedCommand>,
    _area: Subscription,
}

impl EventEmitter<ComposerEvent> for Composer {}

impl Composer {
    pub fn new(placeholder: &str, cx: &mut Context<Self>) -> Self {
        let area = cx.new(|cx| TextArea::multiline(placeholder.to_owned(), 8, cx));
        let subscription = cx.subscribe(&area, |this, _, event: &TextAreaEvent, cx| match event {
            TextAreaEvent::Submit => this.send(cx),
            TextAreaEvent::Changed(_)
            | TextAreaEvent::SelectionChanged
            | TextAreaEvent::Copied
            | TextAreaEvent::Dismissed => cx.notify(),
        });
        Self {
            area,
            attachments: Vec::new(),
            model_label: None,
            thinking_label: None,
            joined: false,
            command_button: false,
            session_commands: false,
            message_only: false,
            compact_when_idle: false,
            inline_review: false,
            send_blocked: false,
            sending: false,
            draft_height: 24.,
            imports: 0,
            import_generation: 0,
            files: None,
            workspace: true,
            files_loading: false,
            files_error: None,
            commands: None,
            attached_command: None,
            _area: subscription,
        }
    }

    /// The files `@` offers, or `None` for the sample's files and commands.
    pub fn set_workspace(&mut self, enabled: bool) {
        self.workspace = enabled;
        if !enabled {
            self.use_files(Some(Vec::new()));
            self.use_commands(Some(Vec::new()));
        }
    }
    pub fn use_files(&mut self, files: Option<Vec<String>>) {
        self.files = files;
        self.files_loading = false;
        self.files_error = None;
    }

    pub fn load_files(&mut self) {
        self.files = Some(Vec::new());
        self.files_loading = true;
        self.files_error = None;
    }

    pub fn fail_files(&mut self, error: String) {
        self.files = Some(Vec::new());
        self.files_loading = false;
        self.files_error = Some(error);
    }

    pub fn use_commands(&mut self, commands: Option<Vec<SavedCommand>>) {
        self.commands = commands;
    }

    fn has_commands(&self) -> bool {
        self.session_commands
            || self
                .commands
                .as_ref()
                .map_or(!COMMANDS.is_empty(), |commands| !commands.is_empty())
    }

    pub fn set_model_label(&mut self, model: String, thinking: String) {
        self.model_label = Some(model);
        self.thinking_label = Some(thinking);
    }

    /// A strip on the composer's top edge shares its border.
    pub fn set_joined(&mut self, joined: bool) {
        self.joined = joined;
    }

    /// New session's composer: the `/` button and a taller draft.
    pub fn set_new_task(&mut self) {
        self.command_button = true;
        self.draft_height = 66.;
    }

    /// Existing sessions expose Pi's commands plus app-owned actions such as `/compact`.
    pub fn set_session_commands(&mut self, compact: bool) {
        self.command_button = true;
        self.session_commands = compact;
    }

    /// Direct child input deliberately has no parent model, command or attachment controls.
    pub fn set_message_only(&mut self) {
        self.message_only = true;
        self.command_button = false;
        self.session_commands = false;
        self.files = Some(Vec::new());
        self.commands = Some(Vec::new());
    }

    /// One line until you type: see `compact_when_idle`.
    pub fn set_compact_when_idle(&mut self) {
        self.compact_when_idle = true;
    }

    pub fn set_inline_review(&mut self, inline: bool) {
        self.inline_review = inline;
    }

    pub fn set_send_blocked(&mut self, blocked: bool) {
        self.send_blocked = blocked;
    }
    pub fn set_sending(&mut self, sending: bool) {
        self.sending = sending;
    }

    pub fn clear_if_text(&mut self, sent: &str, cx: &mut Context<Self>) {
        if self.area.read(cx).text().trim() == sent.trim() {
            self.clear(cx);
        }
    }

    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.area.update(cx, |area, cx| area.set_text(text, cx));
    }

    pub fn attachments(&self) -> &[Attachment] {
        &self.attachments
    }

    pub fn has_attached_command(&self) -> bool {
        self.attached_command.is_some()
    }

    pub fn attach(&mut self, attachment: Attachment, cx: &mut Context<Self>) {
        self.attachments.push(attachment);
        cx.notify();
    }

    pub fn try_attach(
        &mut self,
        attachment: Attachment,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.attachments.len() >= 8 {
            return Err("Attach up to eight files per message.".into());
        }
        if matches!(attachment, Attachment::Image { .. })
            && self
                .attachments
                .iter()
                .filter(|a| matches!(a, Attachment::Image { .. }))
                .count()
                >= crate::attachments::MAX_IMAGES
        {
            return Err("Attach up to four images per message.".into());
        }
        self.attach(attachment, cx);
        Ok(())
    }

    pub(crate) fn cancel_imports(&mut self, cx: &mut Context<Self>) {
        self.import_generation = self.import_generation.wrapping_add(1);
        self.imports = 0;
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.cancel_imports(cx);
        self.area.update(cx, |area, cx| {
            area.take(cx);
        });
        self.attachments.clear();
        self.attached_command = None;
        cx.notify();
    }

    pub fn restore_prompt(
        &mut self,
        prompt: &crate::prompt::Prompt,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        use base64::Engine;
        if self.can_send(cx) || self.imports > 0 {
            return Err("Send or clear your current draft before recovering this message.".into());
        }
        let attachments = prompt
            .images
            .iter()
            .enumerate()
            .map(|(index, content)| {
                let format = gpui::ImageFormat::from_mime_type(&content.mime_type)
                    .ok_or("Unsupported stored image type")?;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&content.data)
                    .map_err(|_| "Could not read the stored image")?;
                Ok(Attachment::Image {
                    name: format!("Recovered image {}", index + 1),
                    image: Arc::new(Image::from_bytes(format, bytes)),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.set_text(&prompt.message, cx);
        self.attachments = attachments;
        cx.notify();
        Ok(())
    }

    pub fn begin_import(&mut self, cx: &mut Context<Self>) -> u64 {
        self.imports += 1;
        cx.notify();
        self.import_generation
    }

    pub fn finish_import(&mut self, generation: u64, cx: &mut Context<Self>) -> bool {
        if generation != self.import_generation {
            return false;
        }
        self.imports = self.imports.saturating_sub(1);
        cx.notify();
        true
    }

    /// Replaces the review lines attached, if any.
    pub fn set_lines(&mut self, lines: Option<String>, cx: &mut Context<Self>) {
        self.attachments
            .retain(|attachment| !matches!(attachment, Attachment::Lines(_)));
        if let Some(lines) = lines {
            self.attachments.insert(0, Attachment::Lines(lines));
        }
        cx.notify();
    }

    pub fn can_send(&self, cx: &App) -> bool {
        let has_payload = !self.area.read(cx).is_empty()
            || self.attached_command.is_some()
            || self.attachments.iter().any(|attachment| {
                !self.inline_review || !matches!(attachment, Attachment::Lines(_))
            });
        !self.sending && !self.send_blocked && self.imports == 0 && has_payload
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.area.read(cx).focus_handle(cx), cx);
    }

    pub fn send(&mut self, cx: &mut Context<Self>) {
        if !self.can_send(cx) {
            return;
        }
        // The app clears only after accepting the complete payload. Validation
        // or connection errors must leave text, the selected command, and images
        // ready to retry. Pi expands commands only at the start of a message.
        let draft = self.area.read(cx).text().trim().to_owned();
        let text = match &self.attached_command {
            Some(command) if draft.is_empty() => format!("/{}", command.name),
            Some(command) => format!("/{} {draft}", command.name),
            None => draft,
        };
        let attachments = self.attachments.clone();
        if self.inline_review {
            cx.emit(ComposerEvent::StageReview { text, attachments });
        } else {
            cx.emit(ComposerEvent::Send { text, attachments });
        }
        cx.notify();
    }

    /// What the `@` or `/` being typed could become.
    fn suggestions(&self, cx: &App) -> Option<(SharedString, Vec<Suggestion>)> {
        if self.message_only {
            return None;
        }
        let area = self.area.read(cx);
        if self.workspace
            && let Some((_range, word)) = area.token_before_caret('@')
        {
            let word = word.to_lowercase();
            let offered: Vec<&str> = match &self.files {
                Some(files) => files.iter().map(String::as_str).collect(),
                None => FILES.to_vec(),
            };
            let mut files: Vec<_> = offered
                .into_iter()
                .filter(|file| file.to_lowercase().contains(&word))
                .take(6)
                .map(|file| {
                    let glyph = if file.ends_with('/') {
                        "folder"
                    } else {
                        "file"
                    };
                    let mention = if file.chars().any(char::is_whitespace)
                        || file.eq_ignore_ascii_case("Visualize")
                    {
                        format!("@\"{file}\" ")
                    } else {
                        format!("@{file} ")
                    };
                    Suggestion {
                        label: file.to_string(),
                        replacement: mention,
                        glyph,
                        command: None,
                    }
                })
                .collect();
            let mut title = if self.files_loading {
                "Loading project files…".into()
            } else if self.files_error.is_some() {
                "Project files unavailable".into()
            } else if files.is_empty() && word.is_empty() {
                "No project files found".into()
            } else if files.is_empty() {
                format!("No files matching “{word}”").into()
            } else if word.is_empty() {
                "Files".into()
            } else {
                format!("Files matching “{word}”").into()
            };
            if "visualize".starts_with(&word) {
                files.insert(
                    0,
                    Suggestion {
                        label: "@Visualize · Native mini-app".into(),
                        replacement: "@Visualize ".into(),
                        glyph: "spark",
                        command: None,
                    },
                );
                files.truncate(6);
                if !self.files_loading && self.files_error.is_none() {
                    title = if word.is_empty() {
                        "Visualize & files".into()
                    } else {
                        "Native mini-app".into()
                    };
                }
            }
            return (self.files.is_some() || !files.is_empty()).then_some((title, files));
        }
        if let Some((_range, word)) = area.token_before_caret('/') {
            let mut offered: Vec<SavedCommand> = match &self.commands {
                Some(commands) => commands.clone(),
                None => COMMANDS
                    .iter()
                    .map(|(name, description)| SavedCommand {
                        name: (*name).into(),
                        description: Some((*description).into()),
                    })
                    .collect(),
            };
            // Built-in actions belong to the app, not Pi's resource catalog. A
            // session-local command of the same name wins, matching desktop.
            if self.session_commands
                && !offered
                    .iter()
                    .any(|command| command.name.trim_start_matches('/') == "compact")
            {
                offered.insert(
                    0,
                    SavedCommand {
                        name: "compact".into(),
                        description: Some("Summarize to free context".into()),
                    },
                );
            }
            let commands: Vec<_> = offered
                .into_iter()
                .filter_map(|command| {
                    let name = command.name.trim_start_matches('/').to_owned();
                    let display_name = name.strip_prefix("skill:").unwrap_or(&name);
                    (!name.is_empty() && (name.starts_with(word) || display_name.starts_with(word)))
                        .then(|| Suggestion {
                            label: format!("/{name}"),
                            replacement: String::new(),
                            glyph: "slash",
                            command: Some(SavedCommand {
                                name,
                                description: command.description,
                            }),
                        })
                })
                .take(6)
                .collect();
            return (!commands.is_empty()).then_some(("Commands".into(), commands));
        }
        None
    }

    fn accept(&mut self, suggestion: Suggestion, cx: &mut Context<Self>) {
        let marker = if suggestion.command.is_some() {
            '/'
        } else {
            '@'
        };
        let replacement = suggestion.replacement;
        let accepted = self.area.update(cx, |area, cx| {
            let Some((range, _)) = area.token_at_caret(marker) else {
                return false;
            };
            area.replace(range, &replacement, cx);
            true
        });
        if accepted && let Some(command) = suggestion.command {
            self.attached_command = Some(command);
            cx.notify();
        }
    }

    fn start_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.area.update(cx, |area, cx| area.insert("/", cx));
        self.focus(window, cx);
    }
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme(cx);
        let prefs = cx.global::<Prefs>().clone();
        let can_send = self.can_send(cx);
        let area = self.area.clone();
        // Sheets move focus away from the editor. Avoid rescanning and
        // lowercasing a large project file list on every overlay animation
        // frame when no suggestion can be accepted.
        let focused = area.read(cx).focus_handle(cx).is_focused(window);
        let suggestions = focused.then(|| self.suggestions(cx)).flatten();
        // Not typing, nothing drafted: one line, the model and effort away.
        let compact = self.compact_when_idle
            && !self.message_only
            && !focused
            && area.read(cx).is_empty()
            && self.attachments.is_empty()
            && self.attached_command.is_none()
            && self.imports == 0
            && !self.sending;
        // Leave room for the conversation and Send even with a tall keyboard
        // or in landscape. The rest of a long draft scrolls inside the field.
        let lines = (f32::from(window.fully_visible_bounds().size.height) * 0.3 / 22.) as usize;
        area.update(cx, |area, _| {
            area.set_max_lines(lines.clamp(2, 8));
            // The padded editor is interactive, stopping at the action row
            // so Send and the optional selection controls keep their own targets.
            if compact {
                area.set_touch_target(
                    px(24.),
                    Edges {
                        top: px(15.),
                        right: px(8.),
                        bottom: px(15.),
                        left: px(0.),
                    },
                );
            } else {
                area.set_touch_target(
                    px(if self.inline_review {
                        88.
                    } else {
                        self.draft_height
                    }),
                    Edges {
                        top: px(12.),
                        right: px(16.),
                        bottom: px(4.),
                        left: px(16.),
                    },
                );
            }
        });
        // Keep the visual control compact while preserving Android's 48 dp
        // touch target around it.
        let send = div()
            .id("send")
            .relative()
            .child(crate::testing::probe("send"))
            .debug_selector(|| "send".into())
            .size(px(48.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .child(
                div()
                    .size(px(40.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(if can_send {
                        colors.accent
                    } else {
                        colors.raised
                    })
                    .child(icon(
                        "send",
                        20.,
                        if can_send {
                            colors.on_accent
                        } else {
                            colors.faint
                        },
                    )),
            )
            .when(can_send, |send| {
                send.active(|style| style.opacity(0.85))
                    .on_click(cx.listener(|this, _, _, cx| this.send(cx)))
            });
        let clip = ui::tap("attach", "clip", &colors)
            .debug_selector(|| "composer-attach".into())
            .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposerEvent::Attach)));
        let model = div()
            .id("model")
            .relative()
            .child(crate::testing::probe("composer-model"))
            .debug_selector(|| "composer-model".into())
            .h(px(48.))
            .min_w_0()
            .max_w(px(136.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(4.))
            .rounded(px(12.))
            .text_size(px(12.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(colors.secondary)
            .active(|style| style.bg(colors.selected))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(self.model_label.clone().unwrap_or(prefs.model)),
            )
            .child(icon("chev_d", 14., colors.secondary))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposerEvent::ChooseModel)));
        let thinking = div()
            .id("thinking")
            .relative()
            .child(crate::testing::probe("composer-thinking"))
            .debug_selector(|| "composer-thinking".into())
            .h(px(48.))
            .flex_none()
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(4.))
            .rounded(px(12.))
            .text_size(px(12.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(colors.secondary)
            .child(self.thinking_label.clone().unwrap_or(prefs.thinking))
            .child(icon("chev_d", 14., colors.secondary))
            .active(|style| style.bg(colors.selected))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposerEvent::ChooseThinking)));
        // Lines picked in Review sit in a strip on the composer's top edge.
        let lines = self
            .attachments
            .iter()
            .enumerate()
            .filter_map(|(index, attachment)| match attachment {
                Attachment::Lines(lines) => Some((index, lines.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        let lines_strip = (!lines.is_empty()).then(|| {
            div()
                .mx(px(12.))
                .mb(px(-1.))
                .min_h(px(48.))
                .pl(px(16.))
                .pr(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .bg(colors.panel)
                .border_1()
                .border_b_0()
                .border_color(colors.line_strong)
                .rounded_t(px(24.))
                .children(lines.into_iter().map(|(index, lines)| {
                    let mut label = lines;
                    if let Some(first) = label.get_mut(0..1) {
                        first.make_ascii_uppercase();
                    }
                    ui::chip(("attachment", index), Some("file"), label, &colors)
                        .h(px(28.))
                        .px(px(10.))
                        .bg(colors.selected)
                        .border_color(colors.selected)
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(icon("x", 14., colors.muted))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.attachments.remove(index);
                            cx.notify();
                        }))
                }))
        });
        let attached_command = self.attached_command.clone();
        let has_body_attachments = attached_command.is_some()
            || self
                .attachments
                .iter()
                .any(|attachment| !matches!(attachment, Attachment::Lines(_)));
        let attachments = has_body_attachments.then(|| {
            div()
                // The dock can be squeezed by the keyboard. Keep the attachment
                // row's measured height so the editor never paints through it.
                .flex_none()
                .flex()
                .flex_wrap()
                .gap(px(12.))
                .pt(px(4.))
                .pb(px(10.))
                .children(attached_command.into_iter().map(|command| {
                    ui::chip(
                        "attached-command",
                        Some("slash"),
                        format!("/{}", command.name.trim_start_matches('/')),
                        &colors,
                    )
                    .debug_selector(|| "attached-command".into())
                    .h(px(36.))
                    .font_family(MONO)
                    .child(icon("x", 14., colors.muted))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.attached_command = None;
                        cx.notify();
                    }))
                }))
                .children(
                    self.attachments
                        .iter()
                        .enumerate()
                        .filter(|(_, attachment)| !matches!(attachment, Attachment::Lines(_)))
                        .map(|(index, attachment)| {
                            let remove = cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.attachments.remove(index);
                                cx.notify();
                            });
                            match attachment {
                                Attachment::Image { image, .. } => div()
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .p(px(2.))
                                    .rounded(px(12.))
                                    .bg(colors.raised)
                                    .child(
                                        div()
                                            .id(("attachment", index))
                                            .debug_selector(move || format!("attachment-{index}"))
                                            .relative()
                                            .size(px(56.))
                                            // GPUI images preserve their intrinsic aspect ratio.
                                            // A portrait attachment must not paint below this
                                            // square thumbnail into the following draft field.
                                            .overflow_hidden()
                                            .rounded(px(10.))
                                            .child(crate::testing::probe(format!(
                                                "attachment-{index}"
                                            )))
                                            .on_click(cx.listener(move |_, _, _, cx| {
                                                cx.emit(ComposerEvent::PreviewImage(index))
                                            }))
                                            .child(
                                                img(image.clone())
                                                    .size_full()
                                                    .aspect_ratio(1.)
                                                    .object_fit(gpui::ObjectFit::Cover),
                                            ),
                                    )
                                    // Separate targets: increasing the remove hit area must
                                    // never cover the image's preview target.
                                    .child(
                                        ui::tap(("remove-attachment", index), "x", &colors)
                                            .debug_selector(move || {
                                                format!("remove-attachment-{index}")
                                            })
                                            .size(px(44.))
                                            .on_click(remove),
                                    )
                                    .into_any_element(),
                                Attachment::File { name, size, .. } => ui::chip(
                                    ("attachment", index),
                                    Some("file"),
                                    name.clone(),
                                    &colors,
                                )
                                .h(px(36.))
                                .child(div().text_color(colors.muted).child(format!("· {size}")))
                                .child(icon("x", 14., colors.muted))
                                .on_click(remove)
                                .into_any_element(),
                                Attachment::Lines(_) => div().into_any_element(),
                            }
                        }),
                )
        });
        if self.inline_review {
            let context = self
                .attachments
                .iter()
                .find_map(|attachment| match attachment {
                    Attachment::Lines(lines) => Some(lines.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "selected lines".into());
            let attach = ui::button(
                "inline-review-attach",
                ui::Button::Quiet,
                Some("clip"),
                "Add files",
                true,
                &colors,
            )
            .debug_selector(|| "inline-review-attach".into())
            .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposerEvent::Attach)));
            let cancel = ui::button(
                "inline-review-cancel",
                ui::Button::Plain,
                None,
                "Cancel",
                true,
                &colors,
            )
            .debug_selector(|| "inline-review-cancel".into())
            .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposerEvent::CancelReview)));
            let comment = ui::button(
                "inline-review-submit",
                ui::Button::Primary,
                None,
                "Add comment",
                true,
                &colors,
            )
            .debug_selector(|| "inline-review-submit".into())
            .when(can_send, |button| {
                button.on_click(cx.listener(|this, _, _, cx| this.send(cx)))
            })
            .when(!can_send, |button| button.opacity(0.45));
            return div()
                .id("inline-review-card")
                .debug_selector(|| "inline-review-card".into())
                .relative()
                .child(crate::testing::probe("inline-review-card"))
                .mx(px(12.))
                .p(px(12.))
                .flex()
                .flex_col()
                .gap(px(10.))
                .rounded(px(16.))
                .bg(colors.composer)
                .border_1()
                .border_color(colors.line_strong)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(14.))
                        .child(icon("file", 16., colors.accent))
                        .child(format!("Add a comment on {context}")),
                )
                .children(attachments)
                .child(
                    div()
                        .id("inline-review-draft")
                        .debug_selector(|| "inline-review-draft".into())
                        .relative()
                        .child(crate::testing::probe("inline-review-draft"))
                        .min_h(px(112.))
                        .p(px(12.))
                        .rounded(px(10.))
                        .bg(colors.canvas)
                        .border_1()
                        .border_color(if focused {
                            colors.accent
                        } else {
                            colors.line_strong
                        })
                        .child(area),
                )
                .child(attach)
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(cancel)
                        .child(comment),
                )
                .into_any_element();
        }

        let strip = suggestions.map(|(title, items)| {
            div()
                .mx(px(12.))
                .mb(px(-1.))
                .px(px(12.))
                .pt(px(10.))
                .pb(px(12.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .bg(colors.panel)
                .border_1()
                .border_b_0()
                .border_color(colors.line)
                .rounded_t(px(20.))
                .child(ui::label(title, &colors))
                .child(
                    div()
                        .id("suggestions")
                        .flex()
                        .gap(px(8.))
                        .overflow_x_scroll()
                        .children(items.into_iter().enumerate().map(|(index, suggestion)| {
                            let probe =
                                crate::testing::probe(format!("suggestion-{}", suggestion.label));
                            let glyph = suggestion.glyph;
                            ui::chip(
                                ("suggestion", index),
                                Some(glyph),
                                suggestion.label.clone(),
                                &colors,
                            )
                            .child(probe)
                            .when(index == 0, |chip| {
                                chip.border_color(colors.accent).text_color(colors.text)
                            })
                            .when(glyph == "slash", |chip| chip.font_family(MONO))
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.accept(suggestion.clone(), cx),
                            ))
                        })),
                )
        });
        let attached = strip.is_some() || lines_strip.is_some() || self.joined;
        // Child input is the regular composer minus its selection controls,
        // not a separate compact editor. Keep spacing and Send placement shared.
        let body = if compact {
            div()
                .debug_selector(|| "composer-compact".into())
                .min_h(px(54.))
                .px(px(4.))
                .flex()
                .items_center()
                .child(clip)
                .child(
                    div()
                        .id("draft-box")
                        .debug_selector(|| "composer-draft".into())
                        .relative()
                        .child(crate::testing::probe("draft"))
                        .flex_1()
                        .min_w_0()
                        .child(area),
                )
                .child(send)
        } else {
            div()
                .flex()
                .flex_col()
                .pl(px(16.))
                .pr(px(8.))
                .pt(px(12.))
                .pb(px(4.))
                .children(attachments)
                .child(
                    div()
                        .id("draft-box")
                        .debug_selector(|| "composer-draft".into())
                        .relative()
                        .child(crate::testing::probe("draft"))
                        .pr(px(8.))
                        .child(area),
                )
                .child(
                    div()
                        .mt(px(4.))
                        .ml(px(-12.))
                        .flex()
                        .items_center()
                        .when(!self.message_only, |row| {
                            row.child(clip)
                                .when(self.command_button && self.has_commands(), |row| {
                                    row.child(
                                        ui::tap("command", "slash", &colors)
                                            .debug_selector(|| "composer-command".into())
                                            .w(px(40.))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.start_command(window, cx)
                                            })),
                                    )
                                })
                                .child(model)
                                .child(thinking)
                        })
                        .child(div().flex_1())
                        .when(self.sending, |row| row.child(ui::hint("Sending…", &colors)))
                        .when(self.imports > 0, |row| {
                            row.child(ui::hint("Preparing…", &colors))
                        })
                        .child(send),
                )
        };
        div()
            .flex()
            .flex_col()
            .children(strip)
            .children(lines_strip)
            .child(
                div()
                    .id("composer")
                    .debug_selector(|| "composer".into())
                    .relative()
                    .child(crate::testing::probe("composer"))
                    .occlude()
                    .mx(px(12.))
                    .bg(colors.composer)
                    .border_1()
                    .border_color(if attached {
                        colors.line
                    } else {
                        colors.line_strong
                    })
                    .map(|composer| {
                        if attached {
                            composer.rounded_b(px(24.))
                        } else {
                            composer.rounded(px(24.))
                        }
                    })
                    .child(body),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn message_only_uses_regular_layout_without_selection_controls(cx: &mut TestAppContext) {
        use gpui::size;
        let long_draft = (0..20).map(|_| "A line\n").collect::<String>();
        for (width, height) in [(320., 640.), (412., 915.), (640., 360.), (320., 400.)] {
            for text in ["", "Two\nlines", long_draft.as_str()] {
                let mut regular_bounds = None;
                for message_only in [false, true] {
                    let (app, cx) = cx
                        .add_window_view(|window, cx| crate::app::PhoneApp::new(None, window, cx));
                    cx.simulate_resize(size(px(width), px(height)));
                    cx.update(|window, cx| {
                        app.update(cx, |app, cx| {
                            app.preview("start", window, cx);
                            app.start.update(cx, |composer, cx| {
                                composer.draft_height = 24.;
                                if message_only {
                                    composer.set_message_only();
                                }
                                composer.set_text(text, cx);
                            });
                        });
                    });
                    cx.run_until_parked();
                    let bounds = (
                        cx.debug_bounds("composer").unwrap(),
                        cx.debug_bounds("composer-draft").unwrap(),
                        cx.debug_bounds("send").unwrap(),
                    );
                    assert!(bounds.1.bottom() <= bounds.2.top());
                    for control in [
                        "composer-attach",
                        "composer-command",
                        "composer-model",
                        "composer-thinking",
                    ] {
                        assert_eq!(
                            cx.debug_bounds(control).is_some(),
                            !message_only,
                            "{control}"
                        );
                    }
                    if message_only {
                        assert_eq!(Some(bounds), regular_bounds, "layout at {width}×{height}");
                    } else {
                        regular_bounds = Some(bounds);
                    }
                }
            }
        }
    }

    #[gpui::test]
    fn message_only_keeps_text_only_and_pending_send_behavior(cx: &mut TestAppContext) {
        let composer = cx.new(|cx| Composer::new("Steer…", cx));
        composer.update(cx, |composer, cx| {
            composer.set_message_only();
            for text in ["@README", "@Visualize", "/review"] {
                composer.set_text(text, cx);
                assert!(composer.suggestions(cx).is_none());
            }
            assert!(!composer.has_commands());
            assert!(composer.attachments().is_empty());
            assert!(composer.can_send(cx));
            composer.set_sending(true);
            assert!(!composer.can_send(cx));
            assert_eq!(composer.area.read(cx).text(), "/review");
            composer.clear_if_text("a different draft", cx);
            assert_eq!(composer.area.read(cx).text(), "/review");
            composer.set_sending(false);
            assert!(composer.can_send(cx));
            composer.clear_if_text("/review", cx);
            assert!(!composer.can_send(cx));
        });
    }

    #[gpui::test]
    fn inline_review_requires_a_comment_not_only_its_line_anchor(cx: &mut TestAppContext) {
        let composer = cx.new(|cx| Composer::new("Add a comment…", cx));
        composer.update(cx, |composer, cx| {
            composer.set_inline_review(true);
            composer.set_lines(Some("lines 10–12".into()), cx);
            assert!(!composer.can_send(cx));
            composer.set_text("Handle the empty case", cx);
            assert!(composer.can_send(cx));
            composer.set_text("", cx);
            composer.attach(
                Attachment::File {
                    name: "note.txt".into(),
                    size: "4 B".into(),
                    contents: "note".into(),
                },
                cx,
            );
            assert!(composer.can_send(cx));
        });
    }

    #[gpui::test]
    fn message_only_first_touch_covers_the_editor_but_not_send(cx: &mut TestAppContext) {
        use gpui::{InputEvent, TouchPhase, point, size};
        for width in [320., 412.] {
            for target in ["center", "left", "top", "bottom", "right", "send"] {
                let (app, cx) =
                    cx.add_window_view(|window, cx| crate::app::PhoneApp::new(None, window, cx));
                cx.simulate_resize(size(px(width), px(640.)));
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.preview("start", window, cx);
                        app.start.update(cx, |composer, cx| {
                            composer.set_message_only();
                            composer.draft_height = 24.;
                            composer
                                .area
                                .update(cx, |area, _| area.set_placeholder("Steer this subagent…"));
                        });
                    })
                });
                cx.run_until_parked();
                let draft = cx.debug_bounds("composer-draft").unwrap();
                let send = cx.debug_bounds("send").unwrap();
                assert!(
                    draft.bottom() <= send.top(),
                    "Send sits below the full-width editor, as in the regular composer"
                );
                assert!(draft.right() >= send.right());
                let position = match target {
                    "left" => point(draft.left() - px(8.), draft.center().y),
                    "top" => point(draft.center().x, draft.top() - px(2.)),
                    "bottom" => point(draft.center().x, draft.bottom() + px(2.)),
                    "right" => point(draft.right() + px(2.), draft.center().y),
                    "send" => send.center(),
                    _ => draft.center(),
                };
                for phase in [TouchPhase::Started, TouchPhase::Ended] {
                    cx.update(|window, cx| {
                        window.dispatch_event(
                            gpui::TouchEvent {
                                id: gpui::TouchId(82),
                                position,
                                phase,
                                ..Default::default()
                            }
                            .to_platform_input(),
                            cx,
                        );
                    });
                    cx.run_until_parked();
                }
                cx.update(|window, cx| {
                    let area = app.read(cx).start.read(cx).area.read(cx);
                    assert_eq!(
                        area.focus_handle(cx).is_focused(window),
                        target != "send",
                        "first touch at {target}"
                    );
                });
                if target != "send" {
                    cx.simulate_resize(size(px(width), px(400.)));
                    cx.run_until_parked();
                    cx.simulate_input("first");
                    app.read_with(cx, |app, cx| {
                        assert_eq!(app.start.read(cx).area.read(cx).text(), "first")
                    });
                }
            }
        }
    }

    #[gpui::test]
    fn remote_project_files_drive_mentions_and_keep_states_visible(cx: &mut TestAppContext) {
        let composer = cx.new(|cx| Composer::new("", cx));
        composer.update(cx, |composer, cx| {
            composer.use_files(Some(vec![
                "src/main.rs".into(),
                "docs/中文 guide.md".into(),
                "src/components/".into(),
            ]));
            composer.set_text("Compare @main", cx);
        });
        composer.read_with(cx, |composer, cx| {
            let (title, items) = composer.suggestions(cx).expect("remote suggestions");
            assert_eq!(title, "Files matching “main”");
            assert_eq!(items[0].label, "src/main.rs");
            assert_eq!(items[0].replacement, "@src/main.rs ");
            assert_eq!(items[0].glyph, "file");
            assert!(items[0].command.is_none());
        });

        composer.update(cx, |composer, cx| {
            composer.set_text("Read @中文", cx);
        });
        composer.read_with(cx, |composer, cx| {
            let (_, items) = composer.suggestions(cx).expect("unicode suggestion");
            assert_eq!(items[0].label, "docs/中文 guide.md");
            assert_eq!(items[0].replacement, "@\"docs/中文 guide.md\" ");
            assert_eq!(items[0].glyph, "file");
        });

        composer.update(cx, |composer, cx| {
            composer.load_files();
            composer.set_text("@", cx);
        });
        composer.read_with(cx, |composer, cx| {
            let (title, items) = composer.suggestions(cx).expect("loading state");
            assert_eq!(title, "Loading project files…");
            assert_eq!(items[0].replacement, "@Visualize ");
        });

        composer.update(cx, |composer, _| composer.fail_files("offline".into()));
        composer.read_with(cx, |composer, cx| {
            let (title, items) = composer.suggestions(cx).expect("error state");
            assert_eq!(title, "Project files unavailable");
            assert_eq!(items[0].replacement, "@Visualize ");
        });
    }

    #[gpui::test]
    fn visualize_is_available_without_files_and_acceptance_only_edits_the_token(
        cx: &mut TestAppContext,
    ) {
        let composer = cx.new(|cx| Composer::new("", cx));
        composer.update(cx, |composer, cx| {
            composer.use_files(Some(vec!["Visualize".into()]));
            composer.set_text("Build @vis", cx);
            let (_, items) = composer.suggestions(cx).unwrap();
            assert_eq!(items[0].replacement, "@Visualize ");
            assert_eq!(items[1].replacement, "@\"Visualize\" ");
            composer.accept(items[0].clone(), cx);
            assert_eq!(composer.area.read(cx).text(), "Build @Visualize ");
            composer.use_files(Some(vec![]));
            composer.set_text("@V", cx);
            assert_eq!(
                composer.suggestions(cx).unwrap().1[0].replacement,
                "@Visualize "
            );
        });
    }

    #[test]
    fn compact_action_is_exact_and_remote_commands_can_shadow_it() {
        assert_eq!(compact_instructions("/compact", false), Some(None));
        assert_eq!(
            compact_instructions("/compact   keep API names ", false),
            Some(Some("keep API names".into()))
        );
        assert_eq!(compact_instructions("/compactly", false), None);
        assert_eq!(compact_instructions("x /compact", false), None);
        assert_eq!(
            compact_instructions("/compact preserve this text", true),
            None
        );
    }

    #[gpui::test]
    fn session_commands_add_compact_without_losing_or_duplicating_remote_commands(
        cx: &mut TestAppContext,
    ) {
        let composer = cx.new(|cx| Composer::new("", cx));
        composer.update(cx, |composer, cx| {
            composer.set_session_commands(true);
            composer.use_commands(Some(vec![
                SavedCommand {
                    name: "review".into(),
                    description: Some("Review changes".into()),
                },
                SavedCommand {
                    name: "skill:lint".into(),
                    description: Some("Lint".into()),
                },
            ]));
            composer.set_text("/", cx);
        });
        composer.read_with(cx, |composer, cx| {
            let (_, items) = composer.suggestions(cx).expect("session commands");
            let labels: Vec<_> = items.iter().map(|item| item.label.as_str()).collect();
            assert_eq!(labels, ["/compact", "/review", "/skill:lint"]);
            assert!(composer.has_commands());
        });

        composer.update(cx, |composer, cx| {
            composer.use_commands(Some(vec![SavedCommand {
                name: "/compact".into(),
                description: Some("Project compact command".into()),
            }]));
            composer.set_text("/", cx);
        });
        composer.read_with(cx, |composer, cx| {
            let (_, items) = composer.suggestions(cx).expect("shadowing command");
            assert_eq!(
                items.iter().filter(|item| item.label == "/compact").count(),
                1
            );
        });
    }

    #[gpui::test]
    fn remote_commands_are_independent_from_remote_file_mentions(cx: &mut TestAppContext) {
        let composer = cx.new(|cx| Composer::new("", cx));
        composer.update(cx, |composer, cx| {
            composer.use_files(Some(vec!["README.md".into()]));
            composer.use_commands(Some(vec![SavedCommand {
                name: "review".into(),
                description: Some("Review the local changes".into()),
            }]));
            composer.set_text("/rev", cx);
        });
        composer.read_with(cx, |composer, cx| {
            let (title, items) = composer.suggestions(cx).expect("remote commands");
            assert_eq!(title, "Commands");
            assert_eq!(items[0].label, "/review");
            assert!(composer.has_commands());
        });

        composer.update(cx, |composer, cx| {
            composer.use_commands(Some(Vec::new()));
            composer.set_text("/", cx);
        });
        composer.read_with(cx, |composer, cx| {
            assert!(composer.suggestions(cx).is_none());
            assert!(!composer.has_commands());
        });
    }

    #[gpui::test]
    fn slash_button_inserts_at_the_utf8_selection_instead_of_the_start(cx: &mut TestAppContext) {
        crate::theme::install_for_tests(cx);
        cx.update(|cx| cx.set_global(Prefs::default()));
        let (composer, cx) = cx.add_window_view(|_, cx| Composer::new("", cx));
        cx.update(|window, cx| {
            composer.update(cx, |composer, cx| {
                composer.set_text("前文 tail", cx);
                let caret = "前文 ".len();
                composer
                    .area
                    .update(cx, |area, cx| area.select_range(caret..caret, cx));
                composer.start_command(window, cx);
                assert_eq!(composer.area.read(cx).text(), "前文 /tail");
                assert_eq!(composer.area.read(cx).selection(), caret + 1..caret + 1);

                composer
                    .area
                    .update(cx, |area, cx| area.select_range(0.."前文".len(), cx));
                composer.start_command(window, cx);
                assert_eq!(composer.area.read(cx).text(), "/ /tail");
                assert_eq!(composer.area.read(cx).selection(), 1..1);
            });
        });
    }

    #[gpui::test]
    fn command_chosen_after_prose_attaches_and_is_serialized_first(cx: &mut TestAppContext) {
        use std::{cell::RefCell, rc::Rc};

        let composer = cx.new(|cx| Composer::new("", cx));
        let sent = Rc::new(RefCell::new(Vec::new()));
        let received = sent.clone();
        let _subscription = cx.update(|cx| {
            cx.subscribe(&composer, move |_, event: &ComposerEvent, _| {
                if let ComposerEvent::Send { text, .. } = event {
                    received.borrow_mut().push(text.clone());
                }
            })
        });
        composer.update(cx, |composer, cx| {
            composer.use_commands(Some(vec![SavedCommand {
                name: "skill:lint".into(),
                description: Some("Lint".into()),
            }]));
            // Stock Pi displays skills by their skill name even though their
            // canonical command is namespaced as `skill:<name>`. Completion
            // replaces the entire token even if the caret is inside it.
            composer.set_text("Use /lint", cx);
            let caret = "Use /lin".len();
            composer
                .area
                .update(cx, |area, cx| area.select_range(caret..caret, cx));
            let (_, items) = composer
                .suggestions(cx)
                .expect("slash completion after prose");
            assert_eq!(items[0].label, "/skill:lint");
            composer.accept(items[0].clone(), cx);
            assert_eq!(composer.area.read(cx).text(), "Use ");
            assert_eq!(
                composer
                    .attached_command
                    .as_ref()
                    .map(|command| command.name.as_str()),
                Some("skill:lint")
            );
            composer.send(cx);
        });
        assert_eq!(&*sent.borrow(), &["/skill:lint Use"]);
    }
}
