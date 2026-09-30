//! The composer's paperclip: files added with the file picker, a paste or a drop.
//! pi's prompt takes text and images, so images go with the prompt (pi resizes
//! them for the model). Any other file becomes a file chip whose path pi reads
//! with its own tools, inside the project or not.
use super::composer::ComposerView;
use super::mentions::{Body, Kind, Mention};
use super::*;
use base64::Engine as _;
use gpui::{ClipboardEntry, ExternalPaths, Image, ImageFormat, ObjectFit, PathPromptOptions, img};
use pi_core::protocol::ImageContent;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// Larger images are refused rather than sent through the RPC pipe.
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

/// An image that goes with the next prompt.
pub struct Attachment {
    pub name: String,
    pub size: usize,
    pub content: ImageContent,
    pub preview: Arc<Image>,
}

impl Attachment {
    /// PNG, JPEG, GIF and WebP: what the model providers accept.
    pub fn new(name: String, format: ImageFormat, bytes: Vec<u8>) -> Option<Self> {
        let mime = mime(format)?;
        Some(Self {
            name,
            size: bytes.len(),
            content: ImageContent::new(
                base64::engine::general_purpose::STANDARD.encode(&bytes),
                mime,
            ),
            preview: Arc::new(Image::from_bytes(format, bytes)),
        })
    }

    /// An image pi gave back with a draft it did not take.
    fn restored(content: ImageContent, number: usize) -> Option<Self> {
        let format = format_of_mime(&content.mime_type)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&content.data)
            .ok()?;
        Some(Self {
            name: format!("image {number}"),
            size: bytes.len(),
            preview: Arc::new(Image::from_bytes(format, bytes)),
            content,
        })
    }
}

fn mime(format: ImageFormat) -> Option<&'static str> {
    Some(match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Gif => "image/gif",
        ImageFormat::Webp => "image/webp",
        _ => return None,
    })
}

pub fn format_of_mime(mime: &str) -> Option<ImageFormat> {
    Some(match mime {
        "image/png" => ImageFormat::Png,
        "image/jpeg" => ImageFormat::Jpeg,
        "image/gif" => ImageFormat::Gif,
        "image/webp" => ImageFormat::Webp,
        _ => return None,
    })
}

/// Images by extension; other files go as paths.
fn image_format(path: &Path) -> Option<ImageFormat> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => ImageFormat::Png,
        "jpg" | "jpeg" => ImageFormat::Jpeg,
        "gif" => ImageFormat::Gif,
        "webp" => ImageFormat::Webp,
        _ => return None,
    })
}

fn size_label(bytes: usize) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.0} KB", bytes as f64 / 1024.),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.),
    }
}

impl ComposerView {
    /// The paperclip: a native picker for any number of files.
    pub(super) fn pick_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            this.update_in(cx, |this, window, cx| this.add_paths(paths, window, cx))
                .ok();
        })
        .detach();
    }

    /// Images are read off the UI thread and attached; other files become chips.
    pub fn add_paths(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let (images, files): (Vec<_>, Vec<_>) = paths
            .into_iter()
            .partition(|path| image_format(path).is_some());
        for file in files {
            self.insert_file_chip(&file, cx);
        }
        if !images.is_empty() {
            let read = cx.background_executor().spawn(async move {
                images
                    .into_iter()
                    .map(|path| {
                        let bytes = std::fs::metadata(&path).and_then(|meta| {
                            if meta.len() as usize > MAX_IMAGE_BYTES {
                                Err(std::io::Error::other("larger than 20 MB"))
                            } else {
                                std::fs::read(&path)
                            }
                        });
                        (path, bytes)
                    })
                    .collect::<Vec<_>>()
            });
            cx.spawn(async move |this, cx| {
                let read = read.await;
                this.update(cx, |this, cx| {
                    for (path, bytes) in read {
                        let name = path.file_name().map_or_else(
                            || path.display().to_string(),
                            |n| n.to_string_lossy().into_owned(),
                        );
                        let attachment = bytes.map_err(|e| e.to_string()).and_then(|bytes| {
                            let format = image_format(&path).ok_or("not an image")?;
                            Attachment::new(name.clone(), format, bytes)
                                .ok_or("unsupported image".into())
                        });
                        match attachment {
                            Ok(attachment) => this.attachments.push(attachment),
                            Err(error) => this.controller.update(cx, |controller, cx| {
                                controller.notice(format!("{name} was not attached: {error}."), cx)
                            }),
                        }
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        self.input.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    /// A file chip at the cursor: the path from the project root, or the whole path
    /// for a file elsewhere.
    fn insert_file_chip(&mut self, path: &Path, cx: &mut Context<Self>) {
        let root = self.mention_root(cx);
        let shown = match path.strip_prefix(&root) {
            Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
            Err(_) => path.display().to_string(),
        };
        let label = path
            .file_name()
            .map_or_else(|| shown.clone(), |name| name.to_string_lossy().into_owned());
        let id = self.next_mention;
        self.next_mention += 1;
        self.input.update(cx, |input, cx| {
            let at = input.cursor();
            if at > 0 && !input.content()[..at].ends_with(char::is_whitespace) {
                input.replace(at..at, " ", cx);
            }
            let at = input.cursor();
            input.insert_chip(at..at, &label, Kind::File.icon(), id, cx);
        });
        self.mentions.insert(
            id,
            Mention {
                kind: Kind::File,
                label,
                body: Body::Path {
                    path: shown,
                    line: None,
                },
            },
        );
    }

    /// A pasted image or copied files attach instead of pasting text.
    pub(super) fn paste_attachments(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let mut paths = Vec::new();
        let mut pasted = false;
        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(image) => {
                    let number = self.attachments.len() + 1;
                    let extension = mime(image.format).map_or("image", |m| &m["image/".len()..]);
                    match Attachment::new(
                        format!("pasted image {number}.{extension}"),
                        image.format,
                        image.bytes.clone(),
                    ) {
                        Some(attachment) => self.attachments.push(attachment),
                        None => self.controller.update(cx, |controller, cx| {
                            controller
                                .notice("Only PNG, JPEG, GIF and WebP images can be attached.", cx)
                        }),
                    }
                    pasted = true;
                }
                ClipboardEntry::ExternalPaths(external) => {
                    paths.extend(external.paths().iter().cloned())
                }
                ClipboardEntry::String(_) => {}
            }
        }
        if !paths.is_empty() {
            self.add_paths(paths, window, cx);
            pasted = true;
        }
        if pasted {
            cx.notify();
        }
        pasted
    }

    pub(super) fn drop_paths(
        &mut self,
        paths: &ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_paths(paths.paths().to_vec(), window, cx);
    }

    pub(super) fn restore_images(&mut self, images: &[ImageContent], cx: &mut Context<Self>) {
        let start = self.attachments.len();
        self.attachments.extend(
            images
                .iter()
                .enumerate()
                .filter_map(|(n, image)| Attachment::restored(image.clone(), start + n + 1)),
        );
        cx.notify();
    }

    pub(super) fn attachments_row(&self, cx: &Context<Self>, theme: Theme) -> Option<AnyElement> {
        if self.attachments.is_empty() {
            return None;
        }
        let model = &self.controller.read(cx).model().state.model;
        let blind = model
            .as_ref()
            .filter(|model| {
                !model.input.is_empty() && !model.input.iter().any(|input| input == "image")
            })
            .map(|model| model.id.clone());
        Some(
            v_flex()
                .px(px(14.))
                .pt(px(12.))
                .gap(px(6.))
                .child(
                    h_flex().flex_wrap().gap(px(8.)).children(
                        self.attachments
                            .iter()
                            .enumerate()
                            .map(|(index, attachment)| {
                                h_flex()
                                    .id(("composer-attachment", index))
                                    .debug_selector(move || format!("composer-attachment-{index}"))
                                    .h(px(42.))
                                    .pl(px(5.))
                                    .pr(px(4.))
                                    .gap(px(8.))
                                    .rounded(px(7.))
                                    .border_1()
                                    .border_color(theme.chip_line)
                                    .bg(theme.chip)
                                    .child(
                                        img(attachment.preview.clone())
                                            .size(px(32.))
                                            .rounded(px(4.))
                                            .object_fit(ObjectFit::Cover),
                                    )
                                    .child(
                                        v_flex()
                                            .max_w(px(170.))
                                            .child(
                                                div()
                                                    .truncate()
                                                    .font_family(MONO)
                                                    .text_size(px(11.))
                                                    .text_color(theme.secondary)
                                                    .child(attachment.name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(10.))
                                                    .text_color(theme.faint)
                                                    .child(format!(
                                                        "image · {}",
                                                        size_label(attachment.size)
                                                    )),
                                            ),
                                    )
                                    .child(
                                        icon_button(
                                            ("remove-attachment", index),
                                            "close",
                                            "Remove image",
                                            theme,
                                        )
                                        .size(px(18.))
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                if index < this.attachments.len() {
                                                    this.attachments.remove(index);
                                                    cx.notify();
                                                }
                                            }),
                                        ),
                                    )
                            }),
                    ),
                )
                .when_some(blind, |row, model| {
                    row.child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.amber)
                            .child(format!(
                                "{model} does not accept images; it will not see these."
                            )),
                    )
                })
                .into_any_element(),
        )
    }
}
