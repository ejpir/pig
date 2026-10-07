//! Pictures and pages Pi's tools made, under the reply that called the tool: a
//! screenshot it read, an HTML page it wrote. `pi_markdown` finds them, as it
//! does for Android; this draws them. A page opens in the browser.

use super::*;
use pi_markdown::{Decoded, Page, Pages, ToolImage};
use std::path::{Path, PathBuf};

/// A card's corner radius inside its border. `overflow_hidden` clips to a
/// rectangle, so a picture at the card's top rounds its own corners.
const INNER_RADIUS: gpui::Pixels = px(7.);

/// The page each tool call left, by call id, with the row that made the call.
/// A page shows once per turn, where it last changed.
pub(super) fn follow_pages(model: &Session) -> HashMap<String, (usize, Page)> {
    let cwd = model.cwd.to_string_lossy();
    let tools: HashMap<&str, &Tool> = model
        .tools
        .iter()
        .map(|tool| (tool.id.as_str(), tool))
        .collect();
    let mut pages = Pages::default();
    let mut shown: HashMap<(usize, String), (String, usize, Page)> = HashMap::new();
    let mut turn = 0;
    for (row, message) in model.messages.iter().enumerate() {
        if message["role"] == "user" {
            turn += 1;
        }
        if message["role"] != "assistant" {
            continue;
        }
        let calls = message["content"].as_array().into_iter().flatten();
        for call in calls.filter(|block| block["type"] == "toolCall") {
            let Some(tool) = call["id"].as_str().and_then(|id| tools.get(id)) else {
                continue;
            };
            let Some(path) = tool.args["path"].as_str() else {
                continue;
            };
            if !tool.finished || tool.is_error {
                continue;
            }
            let path = path
                .strip_prefix(cwd.as_ref())
                .and_then(|path| path.strip_prefix('/'))
                .unwrap_or(path);
            if let Some(page) = pages.follow(&tool.name, &tool.args, path) {
                shown.insert((turn, page.path.clone()), (tool.id.clone(), row, page));
            }
        }
    }
    shown
        .into_values()
        .map(|(id, row, page)| (id, (row, page)))
        .collect()
}

impl TranscriptView {
    /// The images and pages of a reply's tool calls, in call order.
    pub(super) fn tool_media(
        &mut self,
        blocks: &[Value],
        controller: &SessionController,
        theme: Theme,
    ) -> Option<AnyElement> {
        let mut cards = Vec::new();
        for id in blocks
            .iter()
            .filter(|block| block["type"] == "toolCall")
            .filter_map(|block| block["id"].as_str())
        {
            if let Some(tool) = controller.model().tools.iter().find(|tool| tool.id == id) {
                let images = pi_markdown::tool_images(&tool.images, tool.args["path"].as_str());
                for (n, image) in images.iter().enumerate() {
                    cards.push(self.image_card(id, n, image, controller, theme));
                }
            }
            if let Some((_, page)) = self.pages.get(id) {
                let poster = page
                    .html
                    .as_deref()
                    .and_then(|html| super::super::poster::path(html, &page.path))
                    .and_then(|path| match self.posters.get(&path) {
                        Some(Poster::Shown(poster)) => Some(poster.clone()),
                        _ => None,
                    });
                cards.push(page_card(id, page, poster, controller, theme));
            }
        }
        (!cards.is_empty()).then(|| {
            v_flex()
                .w_full()
                .max_w(px(760.))
                .gap(px(10.))
                .mt(px(6.))
                .mb(px(14.))
                .children(cards)
                .into_any_element()
        })
    }

    fn image_card(
        &mut self,
        tool: &str,
        n: usize,
        image: &ToolImage,
        controller: &SessionController,
        theme: Theme,
    ) -> AnyElement {
        let shown: Result<Option<Decoded>, String> = match &image.inline {
            Some(data) => self
                .tool_images
                .entry(image.key.clone())
                .or_insert_with(|| {
                    pi_markdown::decode_base64(&image.mime, data).map_err(|error| match error {
                        pi_markdown::DecodeError::Unsupported(mime) => {
                            format!("Pi Desktop can't show {mime} images.")
                        }
                        pi_markdown::DecodeError::Unreadable => {
                            "This image could not be read.".to_owned()
                        }
                    })
                })
                .clone()
                .map(Some),
            None => match controller.fetched_image(&image.key) {
                Some(Some(fetched)) => fetched.clone().map(Some),
                _ => Ok(None),
            },
        };
        let picture = match &shown {
            // Never larger than the image itself, so a small one stays sharp.
            Ok(Some(decoded)) => h_flex()
                .w_full()
                .justify_center()
                .rounded_t(INNER_RADIUS)
                .bg(theme.canvas)
                .child(
                    gpui::img(decoded.image.clone())
                        .w(px((decoded.width as f32).min(560.)))
                        .max_w_full()
                        .max_h(px(360.))
                        .aspect_ratio(decoded.ratio)
                        // As wide as the card, it reaches the card's corners.
                        .when(decoded.width >= 560, |image| image.rounded_t(INNER_RADIUS))
                        .object_fit(gpui::ObjectFit::Contain),
                ),
            Ok(None) => h_flex()
                .h(px(96.))
                .justify_center()
                .text_size(px(12.))
                .text_color(theme.muted)
                .child("Getting the image from the computer…"),
            Err(error) => div()
                .p(px(12.))
                .text_size(px(12.))
                .text_color(theme.coral)
                .child(error.clone()),
        };
        let selector = format!("tool-image-{tool}-{n}");
        let opened = shown.ok().flatten();
        let (name, mime) = (image.name.clone(), image.mime.clone());
        v_flex()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector)
            .w_full()
            .max_w(px(560.))
            .rounded(px(8.))
            .border_1()
            .border_color(theme.line)
            .bg(theme.panel)
            .overflow_hidden()
            .child(picture)
            .child(
                h_flex()
                    .h(px(32.))
                    .px(px(10.))
                    .gap(px(8.))
                    .border_t_1()
                    .border_color(theme.line)
                    .text_size(px(12.))
                    .text_color(theme.secondary)
                    .child(icon("image", theme.muted).size(px(13.)))
                    .child(div().flex_1().min_w_0().truncate().child(name.clone()))
                    .when(opened.is_some(), |row| {
                        row.child(icon("open", theme.faint).size(px(13.)))
                    }),
            )
            .when_some(opened, |card, decoded| {
                card.cursor_pointer()
                    .hover(move |style| style.border_color(theme.line_strong))
                    .on_click(move |_, _, cx| {
                        let extension = mime
                            .strip_prefix("image/")
                            .map_or("png", |kind| kind.split('+').next().unwrap_or(kind));
                        let file = Path::new(&name).with_extension(extension);
                        match open_copy(&file, &decoded.image.bytes) {
                            Ok(path) => cx.open_with_system(&path),
                            Err(error) => log::warn!("couldn't open {name}: {error}"),
                        }
                    })
            })
            .into_any_element()
    }
}

/// A page's poster, by where it is kept.
pub(super) enum Poster {
    Drawing,
    Shown(Decoded),
    /// It couldn't be drawn, or nothing here draws posters.
    Missing,
}

impl TranscriptView {
    /// Finds or starts the poster of each page that has one to show.
    pub(super) fn draw_posters(&mut self, cx: &mut Context<Self>) {
        let controller = self.controller.read(cx);
        let remote = controller.is_remote();
        let cwd = controller.model().cwd.clone();
        let wanted: Vec<(Page, PathBuf)> = self
            .pages
            .values()
            .filter_map(|(_, page)| {
                let out = super::super::poster::path(page.html.as_deref()?, &page.path)?;
                (!self.posters.contains_key(&out)).then(|| (page.clone(), out))
            })
            .collect();
        for (page, out) in wanted {
            if let Ok(bytes) = std::fs::read(&out)
                && let Ok(poster) = pi_markdown::decode("image/png", bytes)
            {
                self.posters.insert(out, Poster::Shown(poster));
                continue;
            }
            let local = (!remote).then(|| cwd.join(&page.path));
            self.posters.insert(out.clone(), Poster::Missing);
            #[cfg(target_os = "macos")]
            if let Ok(file) = page_file(&page, local.as_deref())
                && let Some(folder) = file.parent()
                && let Some(html) = page.html.as_deref()
                && let Some(drawing) = super::super::poster::Drawing::start(&file, folder, html)
            {
                self.posters.insert(out.clone(), Poster::Drawing);
                cx.spawn(async move |this, cx| {
                    let pause = |ms| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(ms))
                    };
                    // Up to ten seconds to load, then a moment for scripts and fonts.
                    for _ in 0..100 {
                        if !drawing.loading() {
                            break;
                        }
                        pause(100).await;
                    }
                    pause(600).await;
                    let (sender, receiver) = async_channel::bounded(1);
                    drawing.snapshot(out.clone(), move |saved| {
                        sender.try_send(saved).ok();
                    });
                    let saved = receiver.recv().await.unwrap_or(false);
                    let poster = std::fs::read(&out)
                        .ok()
                        .filter(|_| saved)
                        .and_then(|bytes| pi_markdown::decode("image/png", bytes).ok());
                    this.update(cx, |this, cx| {
                        let rows: Vec<usize> = this.pages.values().map(|(row, _)| *row).collect();
                        this.posters
                            .insert(out, poster.map_or(Poster::Missing, Poster::Shown));
                        for row in rows {
                            this.list.remeasure_items(row..row + 1);
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (local, &cx);
        }
    }
}

fn page_card(
    tool: &str,
    page: &Page,
    poster: Option<Decoded>,
    controller: &SessionController,
    theme: Theme,
) -> AnyElement {
    let local = (!controller.is_remote()).then(|| controller.model().cwd.join(&page.path));
    let detail = match &page.html {
        Some(html) => format!("{} · {}", page.path, size_label(html.len())),
        None => format!("{} · changed on the computer", page.path),
    };
    let openable = page.html.is_some() || local.as_ref().is_some_and(|path| path.is_file());
    let selector = format!("page-card-{tool}");
    let page = page.clone();
    let open = {
        let page = page.clone();
        let local = local.clone();
        move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut App| match page_file(
            &page,
            local.as_deref(),
        ) {
            Ok(path) => cx.open_with_system(&path),
            Err(error) => log::warn!("couldn't open {}: {error}", page.path),
        }
    };
    let row = h_flex()
        .w_full()
        .gap(px(12.))
        .px(px(12.))
        .py(px(10.))
        .child(
            div()
                .flex_none()
                .size(px(32.))
                .rounded(px(6.))
                .bg(theme.chip)
                .flex()
                .items_center()
                .justify_center()
                .child(icon("file", theme.accent).size(px(16.))),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .truncate()
                        .child(page.title()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .font_family(MONO)
                        .text_color(theme.muted)
                        .truncate()
                        .child(detail),
                ),
        )
        .when(openable, |row| {
            row.child(
                button(
                    SharedString::from(format!("open-page-{tool}")),
                    "Open in browser",
                    theme,
                )
                .debug_selector({
                    let tool = tool.to_owned();
                    move || format!("open-page-{tool}")
                })
                .child(icon("open", theme.muted).size(px(12.)))
                .on_click(open.clone()),
            )
        });
    v_flex()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector)
        .w_full()
        .max_w(px(560.))
        .rounded(px(8.))
        .border_1()
        .border_color(theme.line)
        .bg(theme.panel)
        .overflow_hidden()
        // The top of the page as it looks; opening it shows the rest.
        .when_some(poster, |card, poster| {
            let tool = tool.to_owned();
            card.child(
                div()
                    .id(SharedString::from(format!("page-poster-{tool}")))
                    .debug_selector(move || format!("page-poster-{tool}"))
                    .w_full()
                    .aspect_ratio(poster.ratio)
                    .border_b_1()
                    .border_color(theme.line)
                    .rounded_t(INNER_RADIUS)
                    .bg(theme.canvas)
                    .when(openable, |poster| poster.cursor_pointer().on_click(open))
                    .child(
                        gpui::img(poster.image)
                            .size_full()
                            .rounded_t(INNER_RADIUS)
                            .object_fit(gpui::ObjectFit::Cover),
                    ),
            )
        })
        .child(row)
        .into_any_element()
}

/// The file to open for a page: the project's own when it still holds what
/// the turn left, so the page's styles and images next to it load too;
/// otherwise a copy of the page as the turn left it.
fn page_file(page: &Page, local: Option<&Path>) -> std::io::Result<PathBuf> {
    if let Some(path) = local.filter(|path| path.is_file())
        && page
            .html
            .as_ref()
            .is_none_or(|html| std::fs::read_to_string(path).is_ok_and(|now| &now == html))
    {
        return Ok(path.to_owned());
    }
    let html = page.html.as_ref().ok_or_else(|| {
        std::io::Error::other("the page changed in a way Pi Desktop can't follow")
    })?;
    open_copy(Path::new(page.file_name()), html.as_bytes())
}

/// Writes `bytes` to a temporary file named `name`, for the system to open.
/// Each content gets its own folder, so earlier copies stay as they were.
fn open_copy(name: &Path, bytes: &[u8]) -> std::io::Result<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    let folder = std::env::temp_dir()
        .join("pi-desktop-open")
        .join(format!("{:016x}", hasher.finish()));
    std::fs::create_dir_all(&folder)?;
    let path = folder.join(name.file_name().unwrap_or(name.as_os_str()));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

fn size_label(bytes: usize) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f32 / 1024.),
        _ => format!("{:.1} MB", bytes as f32 / 1_048_576.),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_opened_from_the_project_only_while_it_holds_the_turns_version() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("page.html");
        std::fs::write(&file, "<p>now</p>").unwrap();
        let page = |html: Option<&str>| Page {
            path: "page.html".into(),
            html: html.map(str::to_owned),
        };
        assert_eq!(
            page_file(&page(Some("<p>now</p>")), Some(&file)).unwrap(),
            file
        );
        assert_eq!(page_file(&page(None), Some(&file)).unwrap(), file);
        let copy = page_file(&page(Some("<p>then</p>")), Some(&file)).unwrap();
        assert_ne!(copy, file);
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "<p>then</p>");
        assert_eq!(copy.file_name().unwrap(), "page.html");
        // A remote session's page is always a copy, and can't be opened unknown.
        assert!(page_file(&page(Some("<p>then</p>")), None).is_ok());
        assert!(page_file(&page(None), None).is_err());
    }
}
