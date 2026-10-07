//! 10 A page Pi made, as a card under its turn: how the page looks, with
//! Open and its source. Opening shows it full screen in a web view (11).

use crate::{
    app::PhoneApp,
    model::{SessionId, ToolImage, Turn},
    theme::{Theme, theme},
    ui::{self, Button, icon},
};
use gpui::{Context, Div, ElementId, FontWeight, ObjectFit, div, img, prelude::*, px, rgb, rgba};
use pi_markdown::Page;
use std::{
    collections::BTreeMap,
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
                    .debug_selector(|| format!("page-poster-{index}-{n}"))
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
                    .debug_selector(|| format!("tool-image-{index}-{n}"))
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
        let decoded = match &image.inline {
            Some(data) => pi_markdown::decode_base64(&image.mime, data),
            None => {
                let live = self.store.as_ref().and_then(|store| store.live.as_ref());
                // A subagent's images come through its session's connection.
                let id = self
                    .subagent_ids
                    .iter()
                    .find(|(_, own)| **own == id)
                    .map_or(id, |((session, _), _)| *session);
                match live.and_then(|live| live.image(id, &image.key)) {
                    None => return Ok(None),
                    Some(Err(error)) => return Err(error.clone()),
                    Some(Ok(fetched)) => pi_markdown::decode(&fetched.mime, fetched.bytes.to_vec()),
                }
            }
        };
        let shown = decoded.map_err(|error| match error {
            pi_markdown::DecodeError::Unsupported(mime) => {
                format!("The phone can't show {mime} images.")
            }
            pi_markdown::DecodeError::Unreadable => "This image could not be read.".to_owned(),
        })?;
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
        let Some(html) = page.html.clone() else {
            self.notify_user(
                "Pi changed this page in a way the phone can't follow. Ask Pi to write it again.",
                cx,
            );
            return;
        };
        let pages = self.site(page);
        let id = match self.route() {
            crate::app::Route::Thread(id) => Some(id),
            _ => None,
        };
        let live = self.store.as_ref().and_then(|store| {
            let live = store.live.as_ref()?;
            let cwd = live.session_cwd(id?)?;
            Some((
                live.connection.clone(),
                live.helper.clone(),
                store.computer.address.clone(),
                cwd.trim_end_matches('/').to_owned(),
            ))
        });
        let page = page.clone();
        let Some((connection, helper, host, cwd)) = live else {
            self.show_page(&page, &html, source, &pages, cx);
            return;
        };
        // Pages it links to that Pi wrote some other way, as with a shell
        // command, come from the computer.
        let base = if page.path.starts_with('/') {
            "/".to_owned()
        } else {
            format!("{cwd}/")
        };
        cx.spawn(async move |this, cx| {
            let pages = linked_pages(pages, key(&page.path), |path| {
                let full = format!("{base}{path}");
                let (connection, helper, host) = (connection.clone(), helper.clone(), host.clone());
                async move {
                    let (folder, name) = full.rsplit_once('/')?;
                    let folder = if folder.is_empty() { "/" } else { folder };
                    crate::remote::read_file(&connection, &helper, &host, folder, name)
                        .await
                        .ok()
                }
            })
            .await;
            this.update(cx, |this, cx| {
                this.show_page(&page, &html, source, &pages, cx)
            })
            .ok();
        })
        .detach();
    }

    fn show_page(
        &mut self,
        page: &Page,
        html: &str,
        source: bool,
        pages: &BTreeMap<String, String>,
        cx: &mut Context<Self>,
    ) {
        let poster = self
            .poster_path(page)
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let site = serde_json::json!({"path": key(&page.path), "pages": pages}).to_string();
        if gpui_android::activity::show_page(
            &page.title(),
            html,
            theme(cx).dark,
            source,
            &poster,
            &site,
        ) {
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

    /// The page and every other page Pi wrote in the open session, as each
    /// was last left, by [`key`].
    fn site(&self, page: &Page) -> BTreeMap<String, String> {
        let session = match self.route() {
            crate::app::Route::Thread(id) => {
                self.store.as_ref().and_then(|store| store.session(id))
            }
            // A subagent's pages, from its screen.
            crate::app::Route::Subagent(id, pick) => self
                .store
                .as_ref()
                .and_then(|store| store.session(id))
                .and_then(|session| session.turns.get(pick.turn))
                .and_then(|turn| turn.handoffs.get(pick.handoff))
                .and_then(|handoff| handoff.subagents.get(pick.index))
                .and_then(|subagent| subagent.conversation_id.clone())
                .and_then(|conversation| self.subagent_ids.get(&(id, conversation)))
                .and_then(|own| self.subagent_sessions.get(own)),
            _ => None,
        };
        site(
            session
                .into_iter()
                .flat_map(|session| &session.turns)
                .flat_map(|turn| &turn.pages)
                .chain([page]),
        )
    }
}

/// How many pages a page's links may bring from the computer.
const LINKED: usize = 24;

/// A page's path as the viewer looks it up: without a leading slash.
fn key(path: &str) -> String {
    path.trim_start_matches('/').to_owned()
}

/// Pages by [`key`]; later ones replace earlier ones at a path.
fn site<'a>(pages: impl IntoIterator<Item = &'a Page>) -> BTreeMap<String, String> {
    pages
        .into_iter()
        .filter_map(|page| Some((key(&page.path), page.html.clone()?)))
        .collect()
}

/// `pages`, with the pages their links reach from `start`, read with `read`
/// by key; at most [`LINKED`] more.
async fn linked_pages<F, R>(
    mut pages: BTreeMap<String, String>,
    start: String,
    read: F,
) -> BTreeMap<String, String>
where
    F: Fn(String) -> R,
    R: std::future::Future<Output = Option<String>>,
{
    let mut seen = std::collections::HashSet::from([start.clone()]);
    let mut queue = vec![start];
    let mut read_count = 0;
    while let Some(from) = queue.pop() {
        let Some(html) = pages.get(&from).cloned() else {
            continue;
        };
        for link in page_links(&html) {
            let Some(path) = resolve(&from, &link) else {
                continue;
            };
            if !seen.insert(path.clone()) {
                continue;
            }
            if !pages.contains_key(&path) {
                if read_count == LINKED {
                    return pages;
                }
                read_count += 1;
                let Some(text) = read(path.clone()).await else {
                    continue;
                };
                pages.insert(path.clone(), text);
            }
            queue.push(path);
        }
    }
    pages
}

/// Where a page's links lead to other pages beside it: `href`s with no
/// scheme or host, ending in `.html`, without their query or fragment.
fn page_links(html: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("href=") {
        rest = &rest[at + 5..];
        let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            continue;
        };
        let Some(end) = rest[1..].find(quote) else {
            break;
        };
        let link = &rest[1..1 + end];
        let link = link.split(['#', '?']).next().unwrap_or("");
        let lower = link.to_ascii_lowercase();
        if (lower.ends_with(".html") || lower.ends_with(".htm"))
            && !link.contains(':')
            && !link.starts_with('/')
        {
            links.push(link.to_owned());
        }
    }
    links
}

/// `link` from the page at `from`, as a key; `None` above the top.
fn resolve(from: &str, link: &str) -> Option<String> {
    let mut parts: Vec<&str> = from.split('/').collect();
    parts.pop();
    for part in link.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
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
pub(crate) type ShownImage = pi_markdown::Decoded;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_opens_with_the_pages_its_links_reach() {
        let page = |path: &str, html: &str| Page {
            path: path.into(),
            html: Some(html.into()),
        };
        let lost = Page {
            path: "demo/lost.html".into(),
            html: None,
        };
        let pages = site([
            &page("demo/index.html", "old"),
            &page("/home/me/orbit.html", "orbit"),
            &lost,
            &page("demo/index.html", "new"),
        ]);
        assert_eq!(
            pages.into_iter().collect::<Vec<_>>(),
            [
                ("demo/index.html".to_owned(), "new".to_owned()),
                ("home/me/orbit.html".to_owned(), "orbit".to_owned())
            ]
        );
    }

    #[test]
    fn links_between_pages_resolve_beside_the_page() {
        let html = r##"<a href="neon-orbit.html">a</a> <a href='games/drive.html#top'>b</a>
            <a href="https://example.com/x.html">c</a> <a href="#top">d</a> <a href="/abs.html">e</a>
            <a href="../up.htm?x=1">f</a> <link href="style.css">"##;
        assert_eq!(
            page_links(html),
            ["neon-orbit.html", "games/drive.html", "../up.htm"]
        );
        assert_eq!(
            resolve("demo/index.html", "a.html").as_deref(),
            Some("demo/a.html")
        );
        assert_eq!(
            resolve("demo/index.html", "../a.html").as_deref(),
            Some("a.html")
        );
        assert_eq!(resolve("index.html", "../a.html"), None);
    }

    #[test]
    fn linked_pages_come_from_the_computer_once() {
        let pages = site([&Page {
            path: "index.html".into(),
            html: Some(
                r#"<a href="a.html"></a><a href="b.html"></a><a href="gone.html"></a>"#.into(),
            ),
        }]);
        let asked = std::cell::RefCell::new(Vec::new());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let found = runtime.block_on(linked_pages(pages, "index.html".into(), |path| {
            asked.borrow_mut().push(path.clone());
            async move {
                match path.as_str() {
                    "a.html" => {
                        Some(r#"<a href="index.html"></a><a href="b.html"></a>"#.to_owned())
                    }
                    "b.html" => Some(String::from("b")),
                    _ => None,
                }
            }
        }));
        assert_eq!(
            found.keys().collect::<Vec<_>>(),
            ["a.html", "b.html", "index.html"]
        );
        assert_eq!(asked.into_inner(), ["a.html", "b.html", "gone.html"]);
    }
}
