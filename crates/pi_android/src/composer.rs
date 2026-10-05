//! The composer: the draft, what is attached to it, suggestions for the `@`
//! file or `/` command being typed, the model, and Send. Choosing a
//! suggestion or a model only edits the draft; only Send sends.

use crate::{
    prefs::Prefs,
    text_area::{TextArea, TextAreaEvent},
    theme::{MONO, theme},
    ui::{self, icon},
};
use gpui::{
    App, Context, Entity, EventEmitter, Focusable, FontWeight, Image, MouseButton, SharedString,
    Subscription, Window, div, img, prelude::*, px,
};
use std::sync::Arc;

#[derive(Clone)]
pub enum Attachment {
    Image {
        name: String,
        image: Arc<Image>,
    },
    File {
        name: String,
        size: String,
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
        attachments: Vec<String>,
    },
    /// The paperclip: the app offers files and the clipboard.
    Attach,
    /// The model chip: the app offers models and thinking levels.
    ChooseModel,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// The landing's composer: tools on their own row.
    Full,
    /// A follow-up docked at the bottom: one row.
    Compact,
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

/// Commands the `/` suggestions offer.
const COMMANDS: &[(&str, &str)] = &[
    ("fix-tests", "Fix the failing tests"),
    ("review", "Review the local changes"),
    ("explain", "Explain this project"),
    ("compact", "Summarize to free context"),
];

/// A chip in the suggestion strip: its text, what it puts in the draft, its icon.
type Suggestion = (String, String, &'static str);

pub struct Composer {
    pub area: Entity<TextArea>,
    attachments: Vec<Attachment>,
    layout: Layout,
    /// What `@` offers: the files a session touched; `None` for the sample's
    /// files and commands.
    files: Option<Vec<String>>,
    _area: Subscription,
}

impl EventEmitter<ComposerEvent> for Composer {}

impl Composer {
    pub fn new(placeholder: &str, layout: Layout, cx: &mut Context<Self>) -> Self {
        let lines = if layout == Layout::Full { 8 } else { 5 };
        let area = cx.new(|cx| TextArea::multiline(placeholder.to_owned(), lines, cx));
        let subscription = cx.subscribe(&area, |this, _, event: &TextAreaEvent, cx| match event {
            TextAreaEvent::Submit => this.send(cx),
            TextAreaEvent::Changed => cx.notify(),
        });
        Self {
            area,
            attachments: Vec::new(),
            layout,
            files: None,
            _area: subscription,
        }
    }

    /// The files `@` offers, or `None` for the sample's files and commands.
    pub fn use_files(&mut self, files: Option<Vec<String>>) {
        self.files = files;
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
        !self.area.read(cx).is_empty() || !self.attachments.is_empty()
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.area.read(cx).focus_handle(cx), cx);
    }

    pub fn send(&mut self, cx: &mut Context<Self>) {
        if !self.can_send(cx) {
            return;
        }
        let text = self.area.update(cx, |area, cx| area.take(cx));
        let attachments = self.attachments.drain(..).map(|a| a.label()).collect();
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
                    (file.to_string(), format!("@{file} "), glyph)
                })
                .collect();
            let title = if word.is_empty() {
                "Files".into()
            } else {
                format!("Files matching “{word}”").into()
            };
            let _ = range;
            return (!files.is_empty()).then_some((title, files));
        }
        if let Some((range, word)) = area.token_before_caret('/')
            && range.start == 0
            && self.files.is_none()
        {
            let commands: Vec<_> = COMMANDS
                .iter()
                .filter(|(name, _)| name.starts_with(word))
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = theme(cx);
        let prefs = cx.global::<Prefs>().clone();
        let can_send = self.can_send(cx);
        let suggestions = self.suggestions(cx);
        let area = self.area.clone();
        let send = div()
            .id("send")
            .size(px(40.))
            .flex_none()
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
                    colors.muted
                },
            ))
            .when(can_send, |send| {
                send.active(|style| style.opacity(0.85))
                    .on_click(cx.listener(|this, _, _, cx| this.send(cx)))
            });
        let clip = ui::tap("attach", "clip", &colors)
            .size(px(40.))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposerEvent::Attach)));
        let attachments = (!self.attachments.is_empty()).then(|| {
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
                        .map(|(index, attachment)| {
                            let remove = cx.listener(move |this, _, _, cx| {
                                this.attachments.remove(index);
                                cx.notify();
                            });
                            match attachment {
                                Attachment::Image { image, .. } => div()
                                    .id(("attachment", index))
                                    .relative()
                                    .size(px(56.))
                                    .flex_none()
                                    .child(
                                        img(image.clone())
                                            .size(px(56.))
                                            .rounded(px(10.))
                                            .object_fit(gpui::ObjectFit::Cover),
                                    )
                                    .child(
                                        div()
                                            .id(("remove", index))
                                            .absolute()
                                            .top(px(-6.))
                                            .right(px(-6.))
                                            .size(px(22.))
                                            .rounded_full()
                                            .bg(colors.text)
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(icon("x", 12., colors.canvas))
                                            .on_click(remove),
                                    )
                                    .into_any_element(),
                                Attachment::File { name, size } => ui::chip(
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
                                Attachment::Lines(lines) => ui::chip(
                                    ("attachment", index),
                                    Some("file"),
                                    lines.clone(),
                                    &colors,
                                )
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(icon("x", 14., colors.muted))
                                .on_click(remove)
                                .into_any_element(),
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
                                ui::chip(("suggestion", index), Some(glyph), text, &colors)
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
        let attached = strip.is_some();
        // A press on the box around the draft types into the draft; stop the
        // app, which also takes focus on a press, from taking it back.
        let focus_area = cx.listener(|this, _, window, cx| {
            this.focus(window, cx);
            window.prevent_default();
        });
        let body = match self.layout {
            Layout::Full => div()
                .flex()
                .flex_col()
                .pl(px(16.))
                .pr(px(8.))
                .pt(px(12.))
                .pb(px(8.))
                .children(attachments)
                .child(div().min_h(px(48.)).pr(px(8.)).child(area))
                .child(
                    div()
                        .mt(px(6.))
                        .ml(px(-8.))
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .child(clip)
                        // Durable sessions have no commands to start.
                        .when(self.files.is_none(), |row| {
                            row.child(ui::tap("command", "slash", &colors).size(px(40.)).on_click(
                                cx.listener(|this, _, window, cx| this.start_command(window, cx)),
                            ))
                        })
                        .child(
                            div()
                                .id("model")
                                .h(px(40.))
                                .px(px(8.))
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .rounded_full()
                                .text_size(px(13.))
                                .text_color(colors.secondary)
                                .active(|style| style.bg(colors.selected))
                                .child(format!("{} · {}", prefs.model, prefs.thinking))
                                .child(icon("chev_d", 14., colors.muted))
                                .on_click(
                                    cx.listener(|_, _, _, cx| cx.emit(ComposerEvent::ChooseModel)),
                                ),
                        )
                        .child(div().flex_1())
                        .child(send),
                ),
            Layout::Compact => div()
                .flex()
                .flex_col()
                .pl(px(16.))
                .pr(px(6.))
                .py(px(6.))
                .children(attachments.map(|row| row.pt(px(6.)).pb(px(2.))))
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap(px(2.))
                        .child(div().flex_1().min_w_0().py(px(10.)).child(area))
                        .child(clip)
                        .child(send),
                ),
        };
        div().flex().flex_col().children(strip).child(
            div()
                .id("composer")
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
                        composer.rounded_b(px(20.))
                    } else {
                        composer.rounded(px(20.))
                    }
                })
                .on_mouse_down(MouseButton::Left, focus_area)
                .child(body),
        )
    }
}
