//! A first screen for GPUI on Android: the window fills the display and keeps
//! clear of the system bars and the keyboard, a text field takes the on-screen
//! keyboard, buttons try the clipboard (text and images), file pickers and
//! links, and a long list scrolls with Android's fling. Build and install it as
//! described in the README.
//!
//! The UI compiles everywhere; only `android_main` is Android's.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

mod text_field;

use gpui::{
    App, ClickEvent, ClipboardEntry, ClipboardItem, Context, Entity, FontWeight, Hsla, Image,
    ImageFormat, KeyBinding, PathPromptOptions, ScrollHandle, SharedString, Subscription, Window,
    WindowAppearance, div, img, prelude::*, px, rgb,
};
use std::{path::Path, sync::Arc};
use text_field::{TextField, TextFieldEvent};

/// What "Copy image" copies.
const STRIPES: &[u8] = include_bytes!("stripes.png");

struct Touch {
    taps: usize,
    list: ScrollHandle,
    field: Entity<TextField>,
    sent: Vec<String>,
    notice: Option<SharedString>,
    /// The last pasted image.
    image: Option<Arc<Image>>,
    /// Names and sizes of the last attached files.
    attached: Vec<String>,
    _sent: Subscription,
}

struct Palette {
    background: Hsla,
    text: Hsla,
    muted: Hsla,
    accent: Hsla,
    row: Hsla,
}

fn palette(appearance: WindowAppearance) -> Palette {
    match appearance {
        WindowAppearance::Dark | WindowAppearance::VibrantDark => Palette {
            background: rgb(0x16181d).into(),
            text: rgb(0xe8e6e3).into(),
            muted: rgb(0x9a9a9a).into(),
            accent: rgb(0x6f8fbf).into(),
            row: rgb(0x20232a).into(),
        },
        _ => Palette {
            background: rgb(0xf7f6f3).into(),
            text: rgb(0x1f1f1f).into(),
            muted: rgb(0x6b6b6b).into(),
            accent: rgb(0x3d5a80).into(),
            row: rgb(0xffffff).into(),
        },
    }
}

impl Touch {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let accent = palette(window.appearance()).accent;
        let field = cx.new(|cx| TextField::new("Type a message", accent, cx));
        let sent = cx.subscribe(&field, |this, _, event: &TextFieldEvent, cx| {
            let TextFieldEvent::Submit(text) = event;
            this.sent.insert(0, text.clone());
            this.notice = None;
            cx.notify();
        });
        Self {
            taps: 0,
            list: ScrollHandle::new(),
            field,
            sent: Vec::new(),
            notice: None,
            image: None,
            attached: Vec::new(),
            _sent: sent,
        }
    }

    fn paste(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            self.notice = Some("The clipboard is empty".into());
            cx.notify();
            return;
        };
        let image = item.entries.iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image.clone()),
            _ => None,
        });
        self.notice = Some(match (image, item.text()) {
            (Some(image), _) => {
                let notice = format!(
                    "Pasted a {} image, {}",
                    image.format.extension().to_uppercase(),
                    size(image.bytes.len() as u64)
                );
                self.image = Some(Arc::new(image));
                notice.into()
            }
            (None, Some(text)) => {
                self.field.update(cx, |field, cx| {
                    let joined = format!("{}{}", field.text(), text.replace('\n', " "));
                    field.set_text(joined, cx);
                });
                "Pasted from the clipboard".into()
            }
            (None, None) => "The clipboard has nothing to paste".into(),
        });
        cx.notify();
    }

    fn copy_image(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem {
            entries: vec![ClipboardEntry::Image(Image::from_bytes(
                ImageFormat::Png,
                STRIPES.to_vec(),
            ))],
        });
        self.notice = Some("Copied an image; paste it here or in another app".into());
        cx.notify();
    }

    fn attach(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let picked = picked.await;
            this.update(cx, |this, cx| {
                this.notice = Some(match picked {
                    Ok(Ok(Some(paths))) => {
                        this.attached = paths.iter().map(|path| describe(path)).collect();
                        format!("Attached {} file(s)", paths.len()).into()
                    }
                    Ok(Ok(None)) => "Nothing attached".into(),
                    Ok(Err(error)) => format!("{error:#}").into(),
                    Err(_) => "The picker closed".into(),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn save(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let mut text = self
            .sent
            .iter()
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        if text.is_empty() {
            text = "Sent messages go here.".into();
        }
        let picked = cx.prompt_for_new_path(Path::new("/"), Some("messages.txt"));
        cx.spawn(async move |this, cx| {
            let notice: SharedString = match picked.await {
                Ok(Ok(Some(path))) => match std::fs::write(&path, format!("{text}\n")) {
                    Ok(()) => format!("Saved to {}", file_name(&path)).into(),
                    Err(error) => format!("Could not save: {error}").into(),
                },
                Ok(Ok(None)) => "Not saved".into(),
                Ok(Err(error)) => format!("{error:#}").into(),
                Err(_) => "The picker closed".into(),
            };
            this.update(cx, |this, cx| {
                this.notice = Some(notice);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn copy(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.field.read(cx).text().to_string();
        self.notice = Some(if text.is_empty() {
            "Nothing to copy".into()
        } else {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            "Copied to the clipboard".into()
        });
        cx.notify();
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn size(bytes: u64) -> String {
    match bytes {
        0..1_000 => format!("{bytes} B"),
        1_000..1_000_000 => format!("{:.0} KB", bytes as f64 / 1e3),
        _ => format!("{:.1} MB", bytes as f64 / 1e6),
    }
}

/// "name · size" for an attached file, read back from its path.
fn describe(path: &Path) -> String {
    match std::fs::metadata(path) {
        Ok(metadata) => format!("{} · {}", file_name(path), size(metadata.len())),
        Err(error) => format!("{} · {error}", file_name(path)),
    }
}

fn button(id: &'static str, label: &'static str, colors: &Palette) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(40.))
        .px(px(14.))
        .flex()
        .items_center()
        .rounded(px(10.))
        .bg(colors.row)
        .text_color(colors.text)
        .text_size(px(14.))
        .child(label)
}

impl Render for Touch {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = palette(window.appearance());
        let viewport = window.viewport_size();
        // Clear of the status bar, navigation bar, cutouts and the keyboard.
        let safe = window.fully_visible_bounds();
        let field = self.field.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.background)
            .text_color(colors.text)
            .pt(safe.top())
            .pb(viewport.height - safe.bottom())
            .pl(safe.left() + px(20.))
            .pr(viewport.width - safe.right() + px(20.))
            .child(
                div()
                    .pt(px(24.))
                    .text_size(px(28.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("GPUI on Android"),
            )
            .child(
                div()
                    .pt(px(4.))
                    .text_size(px(14.))
                    .text_color(colors.muted)
                    .child(format!(
                        "{:.0} × {:.0} at {}× · {:?}",
                        f32::from(viewport.width),
                        f32::from(viewport.height),
                        window.scale_factor(),
                        window.appearance(),
                    )),
            )
            .child(
                div()
                    .mt(px(20.))
                    .h(px(52.))
                    .px(px(14.))
                    .rounded(px(12.))
                    .bg(colors.row)
                    .text_size(px(17.))
                    .child(self.field.clone()),
            )
            .child(
                div()
                    .mt(px(10.))
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        button("send", "Send", &colors)
                            .bg(colors.accent)
                            .text_color(gpui::white())
                            .on_click(cx.listener(move |_, _, _, cx| {
                                field.update(cx, |field, cx| field.submit(cx));
                            })),
                    )
                    .child(button("paste", "Paste", &colors).on_click(cx.listener(Self::paste)))
                    .child(button("copy", "Copy", &colors).on_click(cx.listener(Self::copy)))
                    .child(
                        button("copy-image", "Copy image", &colors)
                            .on_click(cx.listener(Self::copy_image)),
                    )
                    .child(button("attach", "Attach…", &colors).on_click(cx.listener(Self::attach)))
                    .child(button("save", "Save…", &colors).on_click(cx.listener(Self::save)))
                    .child(button("link", "Open a link", &colors).on_click(|_, _, cx| {
                        cx.open_url("https://developer.android.com/");
                    }))
                    .child(button("tap", "Tap me", &colors).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.taps += 1;
                            cx.notify();
                        },
                    ))),
            )
            .child(
                div()
                    .pt(px(8.))
                    .text_size(px(13.))
                    .text_color(colors.muted)
                    .child(match (&self.notice, self.taps) {
                        (Some(notice), _) => notice.clone(),
                        (None, 0) => "Sent messages appear here".into(),
                        (None, 1) => "Tapped once".into(),
                        (None, taps) => format!("Tapped {taps} times").into(),
                    }),
            )
            .children(self.sent.iter().take(3).map(|message| {
                div()
                    .pt(px(6.))
                    .text_size(px(15.))
                    .truncate()
                    .child(message.clone())
            }))
            .children(self.attached.iter().take(3).map(|file| {
                div()
                    .pt(px(6.))
                    .text_size(px(14.))
                    .text_color(colors.muted)
                    .truncate()
                    .child(format!("📎 {file}"))
            }))
            .children(self.image.clone().map(|image| {
                div()
                    .mt(px(8.))
                    .size(px(96.))
                    .rounded(px(8.))
                    .overflow_hidden()
                    .child(img(image).size_full())
            }))
            .child(
                div()
                    .pt(px(16.))
                    .pb(px(8.))
                    .text_size(px(13.))
                    .text_color(colors.muted)
                    .child("Drag or fling the list"),
            )
            .child(
                div()
                    .id("rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.list)
                    .children((1..=200).map(|row| {
                        div()
                            .mb(px(8.))
                            .px(px(16.))
                            .h(px(52.))
                            .flex()
                            .items_center()
                            .rounded(px(10.))
                            .bg(colors.row)
                            .text_size(px(16.))
                            .child(format!("Row {row}"))
                    })),
            )
    }
}

fn bind_keys(cx: &mut App) {
    use text_field::*;
    let context = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("left", Left, context),
        KeyBinding::new("right", Right, context),
        KeyBinding::new("home", Home, context),
        KeyBinding::new("end", End, context),
        KeyBinding::new("enter", Enter, context),
    ]);
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: gpui_android::AndroidApp) {
    use gpui::{Application, WindowOptions};
    use std::rc::Rc;

    gpui_android::init_logging("gpui-touch", log::LevelFilter::Info);
    let platform = match gpui_android::AndroidPlatform::new(app) {
        Ok(platform) => platform,
        Err(error) => {
            log::error!("Could not start GPUI: {error:#}");
            return;
        }
    };
    Application::with_platform(Rc::new(platform)).run(|cx: &mut App| {
        bind_keys(cx);
        let opened = cx.open_window(WindowOptions::default(), |window, cx| {
            cx.new(|cx| Touch::new(window, cx))
        });
        if let Err(error) = opened {
            log::error!("Could not open the window: {error:#}");
        }
    });
    // The activity is gone. Android may call `android_main` again in this
    // process for the next launch; start that from a fresh process instead
    // of a second GPUI application.
    std::process::exit(0);
}
