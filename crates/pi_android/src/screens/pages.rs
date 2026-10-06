//! 10 A page Pi made, as a card under its turn: how the page looks, with
//! Open and its source. Opening shows it full screen in a web view (11).

use crate::{
    app::PhoneApp,
    model::{SessionId, ToolImage, Turn},
    pages::Page,
    theme::{Theme, theme},
    ui::{self, Button, icon},
};
use gpui::{Context, Div, ElementId, FontWeight, ObjectFit, div, img, prelude::*, px, rgb, rgba};
use std::{
    hash::{Hash, Hasher},
    path::PathBuf,
};

/// The poster's height: the card shows the top of the page at 412 by 232.
const POSTER: f32 = 232.;

impl PhoneApp {
    pub(crate) fn page_cards(
        &self,
        index: usize,
        turn: &Turn,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Option<Div> {
        (!turn.pages.is_empty()).then(|| {
            div().flex().flex_col().gap(px(12.)).children(
                (0..turn.pages.len()).filter_map(|n| self.page_card(index, turn, n, colors, cx)),
            )
        })
    }

    /// Page `n` of a turn: how it looks, its name, Source and Open.
    pub(crate) fn page_card(
        &self,
        index: usize,
        turn: &Turn,
        n: usize,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Option<Div> {
        let page = turn.pages.get(n)?;
        Some({
            {
                let detail = match &page.html {
                    Some(html) => format!("{} · {}", page.file_name(), size_label(html.len())),
                    None => format!("{} · changed on the computer", page.file_name()),
                };
                let poster = self.poster_path(page).filter(|path| path.exists());
                if poster.is_none() {
                    self.draw_poster(page, cx);
                }
                let (opened, source) = (page.clone(), page.clone());
                let picture = div()
                    .id(ElementId::Name(format!("page-poster-{index}-{n}").into()))
                    .relative()
                    .h(px(POSTER))
                    .overflow_hidden()
                    // Inside the card's border, so one less than its 16.
                    .rounded_t(px(15.))
                    .bg(rgb(0x05070d))
                    .map(|picture| match poster {
                        Some(path) => picture.child(
                            img(path)
                                .size_full()
                                .rounded_t(px(15.))
                                .object_fit(ObjectFit::Cover),
                        ),
                        None => picture.child(
                            div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .gap(px(8.))
                                .text_size(px(12.5))
                                .text_color(rgba(0xe8edf2a0))
                                .child(ui::working_indicator(colors))
                                .child("Drawing the page…"),
                        ),
                    })
                    .child(
                        div()
                            .absolute()
                            .left(px(12.))
                            .top(px(12.))
                            .h(px(24.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .rounded(px(12.))
                            .bg(rgba(0x05070d8c))
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0xe8edf2))
                            .child(icon("spark", 12., rgb(0xe8edf2).into()))
                            .child("Page"),
                    )
                    .on_click({
                        let page = page.clone();
                        cx.listener(move |this, _, _, cx| this.open_page(&page, false, cx))
                    });
                ui::card(colors).child(picture).child(
                    div()
                        .pl(px(16.))
                        .pr(px(12.))
                        .py(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .truncate()
                                        .child(page.title()),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.5))
                                        .line_height(px(16.))
                                        .text_color(colors.muted)
                                        .truncate()
                                        .child(detail),
                                ),
                        )
                        .child(
                            ui::button(
                                ElementId::Name(format!("page-source-{index}-{n}").into()),
                                Button::Plain,
                                Some("code"),
                                "",
                                true,
                                colors,
                            )
                            .px(px(12.))
                            .gap(px(0.))
                            .bg(colors.tint(colors.accent))
                            .border_color(gpui::transparent_black())
                            .aria_label("Source")
                            .on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.open_page(&source, true, cx)
                                }),
                            ),
                        )
                        .child(
                            ui::button(
                                ElementId::Name(format!("page-{index}-{n}").into()),
                                Button::Primary,
                                Some("open"),
                                "Open",
                                true,
                                colors,
                            )
                            .on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.open_page(&opened, false, cx)
                                }),
                            ),
                        ),
                )
            }
        })
    }

    /// Pictures Pi looked at, such as a screenshot it took and read, as
    /// cards under the turn; tapping one shows it whole.
    pub(crate) fn image_cards(
        &self,
        id: SessionId,
        index: usize,
        turn: &Turn,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Option<Div> {
        (!turn.images.is_empty()).then(|| {
            div().flex().flex_col().gap(px(12.)).children(
                (0..turn.images.len())
                    .filter_map(|n| self.image_card(id, index, turn, n, colors, cx)),
            )
        })
    }

    /// Image `n` of a turn, as a card; tapping it shows it whole.
    pub(crate) fn image_card(
        &self,
        id: SessionId,
        index: usize,
        turn: &Turn,
        n: usize,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Option<gpui::Stateful<Div>> {
        let image = turn.images.get(n)?;
        Some({
            {
                let picture = match self.tool_image(id, image) {
                    Ok(Some(shown)) => div()
                        .w_full()
                        .max_h(px(420.))
                        .aspect_ratio(shown.ratio)
                        .rounded_t(px(15.))
                        .overflow_hidden()
                        .bg(colors.panel)
                        .child(
                            img(shown.image)
                                .size_full()
                                .rounded_t(px(15.))
                                .object_fit(ObjectFit::Contain),
                        ),
                    Ok(None) => div()
                        .h(px(160.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(8.))
                        .bg(colors.panel)
                        .rounded_t(px(15.))
                        .child(ui::working_indicator(colors))
                        .child(ui::hint("Getting the image from the computer…", colors)),
                    Err(error) => div()
                        .p(px(16.))
                        .bg(colors.panel)
                        .rounded_t(px(15.))
                        .child(ui::hint(error, colors).text_color(colors.coral)),
                };
                ui::card(colors)
                    .id(ElementId::Name(format!("tool-image-{index}-{n}").into()))
                    .relative()
                    .child(crate::testing::probe(format!("tool-image-{index}-{n}")))
                    .child(picture)
                    .child(
                        div()
                            .pl(px(16.))
                            .pr(px(12.))
                            .py(px(12.))
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .border_t_1()
                            .border_color(colors.line)
                            .child(icon("image", 18., colors.muted))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(image.name.clone()),
                            )
                            .child(icon("open", 18., colors.muted)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_sheet(crate::app::Sheet::ToolImage(id, index, n), cx)
                    }))
            }
        })
    }

    /// A tool image ready to draw, `None` while it comes from the computer.
    pub(crate) fn tool_image(
        &self,
        id: SessionId,
        image: &ToolImage,
    ) -> Result<Option<ShownImage>, String> {
        if let Some(shown) = self.tool_images.borrow().get(&image.key) {
            return Ok(Some(shown.clone()));
        }
        let (mime, bytes) = match &image.inline {
            Some(data) => {
                use base64::Engine as _;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map_err(|_| "This image could not be read.".to_owned())?;
                (image.mime.clone(), bytes)
            }
            None => {
                let live = self.store.as_ref().and_then(|store| store.live.as_ref());
                match live.and_then(|live| live.image(id, &image.key)) {
                    None => return Ok(None),
                    Some(Err(error)) => return Err(error.clone()),
                    Some(Ok(fetched)) => (fetched.mime.clone(), fetched.bytes.to_vec()),
                }
            }
        };
        let format = gpui::ImageFormat::from_mime_type(&mime)
            .ok_or_else(|| format!("The phone can't show {mime} images."))?;
        let (width, height) = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .ok()
            .and_then(|reader| reader.into_dimensions().ok())
            .ok_or_else(|| "This image could not be read.".to_owned())?;
        let shown = ShownImage {
            image: std::sync::Arc::new(gpui::Image::from_bytes(format, bytes)),
            ratio: width.max(1) as f32 / height.max(1) as f32,
        };
        self.tool_images
            .borrow_mut()
            .insert(image.key.clone(), shown.clone());
        Ok(Some(shown))
    }

    /// Asks for a page's poster once, and shows it when it is saved.
    fn draw_poster(&self, page: &Page, cx: &Context<Self>) {
        let (Some(html), Some(path)) = (&page.html, self.poster_path(page)) else {
            return;
        };
        if !self.posters_drawing.borrow_mut().insert(path.clone()) {
            return;
        }
        if !gpui_android::activity::render_poster(html, &path.display().to_string()) {
            return;
        }
        cx.spawn(async move |this, cx| {
            for _ in 0..40 {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(250))
                    .await;
                if path.exists() {
                    this.update(cx, |_, cx| cx.notify()).ok();
                    return;
                }
            }
        })
        .detach();
    }

    /// Where the viewer keeps a picture of the page, by what the page is.
    fn poster_path(&self, page: &Page) -> Option<PathBuf> {
        let html = page.html.as_ref()?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        html.hash(&mut hasher);
        Some(
            self.data_dir
                .join("posters")
                .join(format!("{:016x}.png", hasher.finish())),
        )
    }

    fn open_page(&mut self, page: &Page, source: bool, cx: &mut Context<Self>) {
        let Some(html) = &page.html else {
            self.notify_user(
                "Pi changed this page in a way the phone can't follow. Ask Pi to write it again.",
                cx,
            );
            return;
        };
        let poster = self
            .poster_path(page)
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        if gpui_android::activity::show_page(&page.title(), html, theme(cx).dark, source, &poster) {
            return;
        }
        if cfg!(target_os = "android") {
            self.notify_user("The page could not be opened.", cx);
            return;
        }
        // The desktop preview has no web view of its own: the browser shows it.
        let file = self.data_dir.join("pages").join(page.file_name());
        let written = std::fs::create_dir_all(self.data_dir.join("pages"))
            .and_then(|()| std::fs::write(&file, html));
        match written {
            Ok(()) => cx.open_url(&format!("file://{}", file.display())),
            Err(error) => self.notify_user(format!("The page could not be saved: {error}"), cx),
        }
    }
}

/// "2.1 KB", as a file's size reads.
fn size_label(bytes: usize) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f32 / 1024.),
        _ => format!("{:.1} MB", bytes as f32 / 1_048_576.),
    }
}

/// A decoded tool image and its width over its height.
#[derive(Clone)]
pub(crate) struct ShownImage {
    pub image: std::sync::Arc<gpui::Image>,
    pub ratio: f32,
}
