//! The composer: the draft, what is attached to it, suggestions for the `@`
//! file or `/` command being typed, the model, and Send. Choosing a
//! suggestion or a model only edits the draft; only Send sends.

use crate::{
    prefs::{Prefs, SavedCommand},
    text_area::{TextArea, TextAreaEvent},
    theme::{MONO, theme},
    ui::{self, icon},
};
use gpui::{
    App, Context, Entity, EventEmitter, Focusable, FontWeight, Image, MouseButton, SharedString,
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
    /// The paperclip: the app offers files and the clipboard.
    Attach,
    /// Independent controls, shared by new sessions, follow-ups and reviews.
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
    ("compact", "Summarize to free context"),
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

/// A chip in the suggestion strip: its text, what it puts in the draft, its icon.
type Suggestion = (String, String, &'static str);

pub struct Composer {
    pub area: Entity<TextArea>,
    attachments: Vec<Attachment>,
    /// A session's actual model, instead of the default for new sessions.
    model_label: Option<String>,
    thinking_label: Option<String>,
    /// A strip sits on the composer's top edge (Working and Stop).
    joined: bool,
    /// The `/` button, on New session only.
    command_button: bool,
    /// The draft's least height: taller for a new task.
    draft_height: f32,
    imports: usize,
    import_generation: u64,
    /// What `@` offers: the files a session touched; `None` for the sample's
    /// built-in files.
    files: Option<Vec<String>>,
    files_loading: bool,
    files_error: Option<String>,
    /// What `/` offers. `None` selects the deterministic sample catalog;
    /// remote sessions always supply `Some`, including an authoritative empty list.
    commands: Option<Vec<SavedCommand>>,
    _area: Subscription,
}

impl EventEmitter<ComposerEvent> for Composer {}

impl Composer {
    pub fn new(placeholder: &str, cx: &mut Context<Self>) -> Self {
        let area = cx.new(|cx| TextArea::multiline(placeholder.to_owned(), 8, cx));
        let subscription = cx.subscribe(&area, |this, _, event: &TextAreaEvent, cx| match event {
            TextAreaEvent::Submit => this.send(cx),
            TextAreaEvent::Changed | TextAreaEvent::Copied => cx.notify(),
        });
        Self {
            area,
            attachments: Vec::new(),
            model_label: None,
            thinking_label: None,
            joined: false,
            command_button: false,
            draft_height: 24.,
            imports: 0,
            import_generation: 0,
            files: None,
            files_loading: false,
            files_error: None,
            commands: None,
            _area: subscription,
        }
    }

    /// The files `@` offers, or `None` for the sample's files and commands.
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
        self.commands
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

    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.area.update(cx, |area, cx| area.set_text(text, cx));
    }

    pub fn attachments(&self) -> &[Attachment] {
        &self.attachments
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

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.import_generation += 1;
        self.imports = 0;
        self.area.update(cx, |area, cx| {
            area.take(cx);
        });
        self.attachments.clear();
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
        self.imports == 0 && (!self.area.read(cx).is_empty() || !self.attachments.is_empty())
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.area.read(cx).focus_handle(cx), cx);
    }

    pub fn send(&mut self, cx: &mut Context<Self>) {
        if !self.can_send(cx) {
            return;
        }
        // The app clears only after accepting the complete payload. Validation
        // or connection errors must leave text AND images ready to retry.
        let text = self.area.read(cx).text().to_owned();
        let attachments = self.attachments.clone();
        cx.emit(ComposerEvent::Send {
            text: text.trim().to_owned(),
            attachments,
        });
        cx.notify();
    }

    /// What the `@` or `/` being typed could become.
    fn suggestions(&self, cx: &App) -> Option<(SharedString, Vec<Suggestion>)> {
        let area = self.area.read(cx);
        if let Some((range, word)) = area.token_before_caret('@') {
            let word = word.to_lowercase();
            let offered: Vec<&str> = match &self.files {
                Some(files) => files.iter().map(String::as_str).collect(),
                None => FILES.to_vec(),
            };
            let files: Vec<_> = offered
                .into_iter()
                .filter(|file| file.to_lowercase().contains(&word))
                .take(6)
                .map(|file| {
                    let glyph = if file.ends_with('/') {
                        "folder"
                    } else {
                        "file"
                    };
                    let mention = if file.chars().any(char::is_whitespace) {
                        format!("@\"{file}\" ")
                    } else {
                        format!("@{file} ")
                    };
                    (file.to_string(), mention, glyph)
                })
                .collect();
            let title = if self.files_loading {
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
            let _ = range;
            return (self.files.is_some() || !files.is_empty()).then_some((title, files));
        }
        if let Some((range, word)) = area.token_before_caret('/')
            && range.start == 0
        {
            let offered: Vec<(&str, Option<&str>)> = match &self.commands {
                Some(commands) => commands
                    .iter()
                    .map(|command| {
                        (
                            command.name.trim_start_matches('/'),
                            command.description.as_deref(),
                        )
                    })
                    .collect(),
                None => COMMANDS
                    .iter()
                    .map(|(name, description)| (*name, Some(*description)))
                    .collect(),
            };
            let commands: Vec<_> = offered
                .into_iter()
                .filter(|(name, _)| !name.is_empty() && name.starts_with(word))
                .take(6)
                .map(|(name, _)| (format!("/{name}"), format!("/{name} "), "slash"))
                .collect();
            return (!commands.is_empty()).then_some(("Commands".into(), commands));
        }
        None
    }

    fn accept(&mut self, replacement: String, cx: &mut Context<Self>) {
        self.area.update(cx, |area, cx| {
            let token = area
                .token_before_caret('@')
                .or_else(|| area.token_before_caret('/'))
                .map(|(range, _)| range);
            if let Some(range) = token {
                area.replace(range, &replacement, cx);
            }
        });
    }

    fn start_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.area.update(cx, |area, cx| {
            if !area.text().starts_with('/') {
                let text = format!("/{}", area.text());
                area.set_text(text, cx);
                area.replace(1..1, "", cx);
            }
        });
        self.focus(window, cx);
    }
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme(cx);
        let prefs = cx.global::<Prefs>().clone();
        let can_send = self.can_send(cx);
        let suggestions = self.suggestions(cx);
        let area = self.area.clone();
        // Leave room for the conversation and Send even with a tall keyboard
        // or in landscape. The rest of a long draft scrolls inside the field.
        let lines = (f32::from(window.fully_visible_bounds().size.height) * 0.3 / 22.) as usize;
        area.update(cx, |area, _| area.set_max_lines(lines.clamp(2, 8)));
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
        let has_body_attachments = self
            .attachments
            .iter()
            .any(|attachment| !matches!(attachment, Attachment::Lines(_)));
        let attachments = has_body_attachments.then(|| {
            div()
                .flex()
                .flex_wrap()
                .gap(px(12.))
                .pt(px(4.))
                .pb(px(10.))
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
                                            .child(crate::testing::probe(format!(
                                                "attachment-{index}"
                                            )))
                                            .on_click(cx.listener(move |_, _, _, cx| {
                                                cx.emit(ComposerEvent::PreviewImage(index))
                                            }))
                                            .child(
                                                img(image.clone())
                                                    .size(px(56.))
                                                    .rounded(px(10.))
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
                        .children(items.into_iter().enumerate().map(
                            |(index, (text, replacement, glyph))| {
                                let probe = crate::testing::probe(format!("suggestion-{text}"));
                                ui::chip(("suggestion", index), Some(glyph), text, &colors)
                                    .child(probe)
                                    .when(index == 0, |chip| {
                                        chip.border_color(colors.accent).text_color(colors.text)
                                    })
                                    .when(glyph == "slash", |chip| chip.font_family(MONO))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.accept(replacement.clone(), cx)
                                    }))
                            },
                        )),
                )
        });
        let attached = strip.is_some() || lines_strip.is_some() || self.joined;
        // A press on the box around the draft types into the draft; stop the
        // app, which also takes focus on a press, from taking it back.
        let focus_area = cx.listener(|this, _, window, cx| {
            this.focus(window, cx);
            window.request_virtual_keyboard();
            window.prevent_default();
        });
        let body = div()
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
                    .min_h(px(self.draft_height))
                    .pr(px(8.))
                    .on_mouse_down(MouseButton::Left, focus_area)
                    .child(area),
            )
            .child(
                div()
                    .mt(px(4.))
                    .ml(px(-12.))
                    .flex()
                    .items_center()
                    .child(clip)
                    .when(self.command_button && self.has_commands(), |row| {
                        row.child(ui::tap("command", "slash", &colors).w(px(40.)).on_click(
                            cx.listener(|this, _, window, cx| this.start_command(window, cx)),
                        ))
                    })
                    .child(model)
                    .child(thinking)
                    .child(div().flex_1())
                    .when(self.imports > 0, |row| {
                        row.child(ui::hint("Preparing…", &colors))
                    })
                    .child(send),
            );
        div()
            .flex()
            .flex_col()
            .children(strip)
            .children(lines_strip)
            .child(
                div()
                    .id("composer")
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

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
            assert_eq!(
                items[0],
                ("src/main.rs".into(), "@src/main.rs ".into(), "file")
            );
        });

        composer.update(cx, |composer, cx| {
            composer.set_text("Read @中文", cx);
        });
        composer.read_with(cx, |composer, cx| {
            let (_, items) = composer.suggestions(cx).expect("unicode suggestion");
            assert_eq!(
                items[0],
                (
                    "docs/中文 guide.md".into(),
                    "@\"docs/中文 guide.md\" ".into(),
                    "file"
                )
            );
        });

        composer.update(cx, |composer, cx| {
            composer.load_files();
            composer.set_text("@", cx);
        });
        composer.read_with(cx, |composer, cx| {
            let (title, items) = composer.suggestions(cx).expect("loading state");
            assert_eq!(title, "Loading project files…");
            assert!(items.is_empty());
        });

        composer.update(cx, |composer, _| composer.fail_files("offline".into()));
        composer.read_with(cx, |composer, cx| {
            let (title, items) = composer.suggestions(cx).expect("error state");
            assert_eq!(title, "Project files unavailable");
            assert!(items.is_empty());
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
            assert_eq!(items[0].0, "/review");
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
}
