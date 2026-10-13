//! One recent HTML artifact inline in the native thread. The running revision
//! is pinned until an explicit update. Native view state is a bounded cache,
//! not persistence across eviction or process death.

use crate::{
    app::{PhoneApp, Route, Sheet},
    model::{SessionId, Turn},
    theme::{MONO, Theme, theme},
    ui::{self, Button},
};
use gpui::{Bounds, Context, Div, FontWeight, Pixels, Window, canvas, div, prelude::*, px};
use gpui_android::activity;
use pi_markdown::Page;
use std::{
    cell::RefCell,
    collections::HashMap,
    hash::{Hash, Hasher},
    rc::Rc,
    sync::Arc,
};

const HEIGHT: f32 = 376.;
const MAX_HTML: usize = 2 * 1024 * 1024;
const MAX_PINNED: usize = 8;

#[derive(Clone)]
struct Running {
    session: SessionId,
    turn: usize,
    page: usize,
    title: String,
    html: Arc<str>,
    revision: String,
    version: u32,
    saved: bool,
}

#[derive(Default)]
pub(crate) struct InlinePages {
    running: HashMap<String, Running>,
    pub(crate) expanded: Option<String>,
    origin: Option<Route>,
    selected: Option<String>,
    pub(crate) slot: Rc<RefCell<Option<String>>>,
}

fn digest(value: impl Hash) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
fn identity(id: SessionId, index: usize, turn: &Turn, page: &Page) -> String {
    format!(
        "{}-{index}-{}",
        id.0,
        digest((&turn.prompt, &turn.at, &page.path))
    )
}
fn document_title(page: &Page) -> String {
    page.title().chars().take(256).collect()
}

fn rectangle(bounds: Bounds<Pixels>, scale: f32) -> [f32; 4] {
    [
        f32::from(bounds.origin.x) * scale,
        f32::from(bounds.origin.y) * scale,
        f32::from(bounds.size.width) * scale,
        f32::from(bounds.size.height) * scale,
    ]
}

impl PhoneApp {
    pub(crate) fn reset_inline_pages(&mut self) {
        *self.inline_pages.borrow_mut() = InlinePages::default();
        activity::clear_inline_pages();
    }

    /// Start with the most recently changed page. Older artifacts keep the
    /// existing poster/viewer fallback, avoiding a transcript full of browsers.
    pub(crate) fn uses_inline_page(&self, id: SessionId, index: usize, n: usize) -> bool {
        if !activity::supports_inline_pages() {
            return false;
        }
        self.session(id).and_then(|session| {
            session
                .turns
                .iter()
                .enumerate()
                .rev()
                .find_map(|(i, turn)| {
                    let last = turn
                        .flow
                        .iter()
                        .rev()
                        .find_map(|step| match step {
                            crate::model::Flow::Page(n) => Some(*n),
                            _ => None,
                        })
                        .or_else(|| turn.pages.len().checked_sub(1))?;
                    Some((i, last))
                })
        }) == Some((index, n))
    }

    #[allow(clippy::too_many_arguments)] // Native host/render context and artifact identity are explicit.
    pub(crate) fn inline_page_card(
        &self,
        id: SessionId,
        index: usize,
        turn: &Turn,
        n: usize,
        expanded: bool,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Option<Div> {
        let page = turn.pages.get(n)?;
        let key = identity(id, index, turn, page);
        let running = {
            let mut state = self.inline_pages.borrow_mut();
            if !state.running.contains_key(&key)
                && let Some(html) = page.html.as_ref().filter(|html| html.len() <= MAX_HTML)
            {
                if state.running.len() >= MAX_PINNED {
                    let old = state
                        .running
                        .keys()
                        .find(|old| {
                            state.expanded.as_ref() != Some(old)
                                && state.selected.as_ref() != Some(old)
                        })
                        .cloned();
                    if let Some(old) = old {
                        state.running.remove(&old);
                    }
                }
                state.running.insert(
                    key.clone(),
                    Running {
                        session: id,
                        turn: index,
                        page: n,
                        title: document_title(page),
                        html: Arc::from(html.as_str()),
                        revision: digest(html.as_str()),
                        version: 1,
                        saved: false,
                    },
                );
            }
            if let Some(running) = state.running.get_mut(&key)
                && page.html.as_deref() == Some(running.html.as_ref())
            {
                running.saved = false;
            }
            state.running.get(&key).cloned()
        };
        let failed = !activity::inline_pages_available();
        let oversized = page.html.as_ref().is_some_and(|html| html.len() > MAX_HTML);
        let unavailable =
            oversized || page.html.is_none() && !running.as_ref().is_some_and(|r| r.saved);
        let changed = running.as_ref().is_some_and(|r| {
            page.html
                .as_deref()
                .is_some_and(|html| html != r.html.as_ref())
        });
        let saved = running.as_ref().is_some_and(|r| r.saved);
        let title = running
            .as_ref()
            .map_or_else(|| document_title(page), |r| r.title.clone());
        let subtitle = if unavailable {
            "Current revision unavailable".to_owned()
        } else if saved {
            "Saved snapshot · not the current revision".to_owned()
        } else {
            format!(
                "{} · v{}",
                page.file_name(),
                running.as_ref().map_or(1, |r| r.version)
            )
        };
        let header = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.))
            .pl(px(14.))
            .pr(px(4.))
            .min_h(px(58.))
            .child(ui::icon("code", 20., colors.accent))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(14.))
                            .truncate()
                            .child(title),
                    )
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(10.))
                            .text_color(if saved { colors.amber } else { colors.muted })
                            .truncate()
                            .child(subtitle),
                    ),
            )
            .when(!unavailable && !failed, |header| {
                header.child(
                    ui::tap(
                        format!("page-{index}-{n}"),
                        if expanded { "back" } else { "open" },
                        colors,
                    )
                    .aria_label(if expanded {
                        "Return to chat"
                    } else {
                        "Expand page"
                    })
                    .on_click({
                        let key = key.clone();
                        cx.listener(move |this, _, window, cx| {
                            window.dismiss_virtual_keyboard();
                            window.focus(&this.focus, cx);
                            let mut state = this.inline_pages.borrow_mut();
                            state.expanded = if expanded { None } else { Some(key.clone()) };
                            state.origin = Some(this.route());
                            cx.notify();
                        })
                    }),
                )
            });
        let body = if unavailable || running.is_none() {
            div().p(px(24.)).min_h(px(240.)).flex().flex_col().gap(px(16.))
                .child(ui::hint(if oversized { "This page is too large for the inline preview (2 MiB limit). Open it separately." }
                    else { "This revision can’t be reconstructed. Ask Pi to write the complete HTML file again." }, colors))
                .when(running.is_some() && !oversized, |body| body.child(ui::button("inline-saved", Button::Plain, None, "View saved version instead", false, colors)
                    .on_click({ let key = key.clone(); cx.listener(move |this, _, _, cx| {
                        if let Some(running) = this.inline_pages.borrow_mut().running.get_mut(&key) { running.saved = true; }
                        this.remeasure_thread_turn(id, index);
                        cx.notify();
                    }) })))
        } else if failed {
            div().relative().p(px(24.)).flex().flex_col().gap(px(16.))
                .when(!expanded, |body| body.h(px(HEIGHT)).flex_none())
                .when(expanded, |body| body.flex_1().min_h_0())
                .child(crate::testing::probe("inline-failed"))
                .child(ui::hint("Interactive preview stopped or could not start. Your conversation is safe. Source and Open separately are still available.", colors))
                .child(ui::hint("Retry starts a fresh page and clears cached page inputs.", colors))
                .child(ui::button("inline-retry", Button::Plain, None, "Retry preview", false, colors)
                    .on_click(cx.listener(move |this, _, _, cx| { this.reset_inline_pages(); this.remeasure_thread_turn(id, index); cx.notify(); })))
        } else {
            let running = running.as_ref()?;
            let html = running.html.clone();
            let revision = running.revision.clone();
            let slot = self.inline_pages.borrow().slot.clone();
            let key = key.clone();
            let dark = colors.dark;
            div()
                .relative()
                .w_full()
                .min_h(px(80.))
                .bg(colors.composer)
                .when(!expanded, |body| body.h(px(HEIGHT)).flex_none())
                .when(expanded, |body| body.flex_1().min_h_0())
                .child(crate::testing::probe(format!("inline-page-{index}-{n}")))
                .child(
                    canvas(
                        |bounds, window, _| {
                            bounds
                                .intersect(&window.content_mask().bounds)
                                .intersect(&Bounds::new(
                                    gpui::Point::default(),
                                    window.viewport_size(),
                                ))
                        },
                        move |bounds, clip, window, cx| {
                            #[cfg(feature = "ui-test")]
                            if cx.has_global::<crate::testing::State>() {
                                let state = cx.global_mut::<crate::testing::State>();
                                state
                                    .bounds
                                    .insert("inline-page-clip".into(), rectangle(clip, 1.));
                            }
                            #[cfg(not(feature = "ui-test"))]
                            let _ = cx;
                            if clip.is_empty()
                                || f32::from(clip.size.width) * window.scale_factor() < 1.
                                || f32::from(clip.size.height) * window.scale_factor() < 1.
                            {
                                return;
                            }
                            if !activity::prepare_inline_page(&key, &revision, &html) {
                                return;
                            }
                            *slot.borrow_mut() = Some(
                                serde_json::json!({ "id": key, "revision": revision,
                            "bounds": rectangle(bounds, window.scale_factor()),
                            "clip": rectangle(clip, window.scale_factor()), "dark": dark })
                                .to_string(),
                            );
                        },
                    )
                    .absolute()
                    .size_full(),
                )
        };
        let separate = page.clone();
        let footer = div()
            .flex_none()
            .min_h(px(48.))
            .border_t_1()
            .border_color(colors.line)
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(4.))
            .bg(colors.canvas)
            .child(
                div()
                    .id("inline-permissions")
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h(px(48.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(6.))
                    .child(crate::testing::probe("inline-permissions"))
                    .text_size(px(11.))
                    .text_color(colors.muted)
                    .child(ui::icon(
                        if failed { "info" } else { "check" },
                        14.,
                        if failed { colors.amber } else { colors.green },
                    ))
                    .child(if failed {
                        "Preview unavailable"
                    } else {
                        "Local · no network"
                    })
                    .on_click(
                        cx.listener(|this, _, _, cx| this.open_sheet(Sheet::InlinePermissions, cx)),
                    ),
            )
            .when(page.html.is_some(), |footer| {
                footer.child(
                    ui::tap(format!("inline-open-{index}-{n}"), "open", colors)
                        .aria_label("Open separately")
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.open_page(&separate, false, cx)),
                        ),
                )
            })
            .when(!unavailable, |footer| {
                footer.child(
                    ui::tap(format!("page-source-{index}-{n}"), "code", colors)
                        .aria_label("Source")
                        .on_click({
                            let key = key.clone();
                            cx.listener(move |this, _, _, cx| {
                                this.open_inline_source(&key, cx);
                            })
                        }),
                )
            });
        Some(
            ui::card(colors)
                .min_w_0()
                .flex()
                .flex_col()
                .overflow_hidden()
                .when(expanded, |card| card.flex_1().min_h_0())
                .when(!expanded, |card| card.flex_none())
                .child(header)
                .when(changed && !unavailable && !failed, |card| {
                    card.child(
                        div()
                            .flex_none()
                            .px(px(12.))
                            .min_h(px(54.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .bg(colors.tint(colors.amber))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(12.))
                                    .text_color(colors.amber)
                                    .child("Update ready · current page kept"),
                            )
                            .child(
                                ui::button(
                                    "inline-update",
                                    Button::Plain,
                                    None,
                                    "Update",
                                    false,
                                    colors,
                                )
                                .on_click({
                                    let key = key.clone();
                                    cx.listener(move |this, _, _, cx| {
                                        this.inline_pages.borrow_mut().selected = Some(key.clone());
                                        this.open_sheet(Sheet::InlineUpdate, cx);
                                    })
                                }),
                            ),
                    )
                })
                .child(body)
                .child(footer),
        )
    }

    fn open_inline_source(&mut self, key: &str, cx: &mut Context<Self>) {
        let html = self
            .inline_pages
            .borrow()
            .running
            .get(key)
            .map(|r| r.html.clone());
        if let Some(html) = html {
            self.inline_pages.borrow_mut().selected = Some(key.to_owned());
            self.selectable
                .update(cx, |area, cx| area.show_selected(html.to_string(), cx));
            self.open_sheet(Sheet::InlineSource, cx);
        }
    }

    pub(crate) fn inline_expanded_screen(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<Div> {
        let state = self.inline_pages.borrow();
        if state.origin != Some(self.route()) {
            return None;
        }
        let key = state.expanded.as_ref()?.clone();
        let running = state.running.get(&key)?.clone();
        drop(state);
        let session = self.session(running.session)?;
        let turn = session.turns.get(running.turn)?;
        if identity(
            running.session,
            running.turn,
            turn,
            turn.pages.get(running.page)?,
        ) != key
        {
            return None;
        }
        let colors = theme(cx);
        let card = self.inline_page_card(
            running.session,
            running.turn,
            turn,
            running.page,
            true,
            &colors,
            cx,
        )?;
        Some(
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(ui::appbar(
                    ui::tap("inline-back", "back", &colors).on_click(cx.listener(
                        |this, _, window, cx| {
                            this.inline_pages.borrow_mut().expanded = None;
                            window.dismiss_virtual_keyboard();
                            window.focus(&this.focus, cx);
                            cx.notify();
                        },
                    )),
                    running.title,
                    Some("Same running page · on this phone".into()),
                    &colors,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .p(px(12.))
                        .pb(px(8.))
                        .flex()
                        .flex_col()
                        .child(card),
                )
                .max_h(window.fully_visible_bounds().size.height),
        )
    }

    pub(crate) fn inline_page_sheet(
        &mut self,
        sheet: Sheet,
        colors: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        match sheet {
            Sheet::InlinePermissions => div()
                .child(ui::hint("Local scripts work on this phone, including offline. This page cannot access the network, files, app data, credentials or Pi’s tools. Links, downloads, JavaScript dialogs, nested frames, workers and permission requests are blocked.", colors))
                .child(ui::hint("Page state stays while its native view is cached. The cache holds three recent pages; eviction, reconnecting/changing computers or process death starts a fresh document. Expand and Source do not reload it.", colors).mt(px(16.))),
            Sheet::InlineSource => {
                let lines = (f32::from(window.viewport_size().height) * 0.5 / 22.) as usize;
                self.selectable.update(cx, |area, _| area.set_max_lines(lines.max(4)));
                let text = self.selectable.read(cx).text().to_owned();
                div().child(ui::hint("Source of the running version. Viewing it doesn’t reload the page.", colors))
                    .child(div().mt(px(12.)).p(px(12.)).bg(colors.panel).rounded(px(12.)).child(self.selectable.clone()))
                    .child(ui::button("inline-copy-source", Button::Plain, Some("copy"), "Copy source", false, colors).mt(px(12.))
                        .on_click(cx.listener(move |this, _, _, cx| this.copy(text.clone(), "page source", cx))))
            }
            Sheet::InlineUpdate => div()
                .child(ui::hint("Load the new HTML revision? This replaces the running document and resets its inputs. Your conversation and draft are kept.", colors))
                .child(div().mt(px(20.)).flex().gap(px(10.))
                    .child(ui::button("inline-keep", Button::Plain, None, "Keep current", false, colors).flex_1()
                        .on_click(cx.listener(|this, _, _, cx| this.close_sheet(cx))))
                    .child(ui::button("inline-load", Button::Primary, None, "Load update", false, colors).flex_1()
                        .on_click(cx.listener(|this, _, _, cx| this.apply_inline_update(cx))))),
            _ => div(),
        }
    }

    fn apply_inline_update(&mut self, cx: &mut Context<Self>) {
        let selected = self.inline_pages.borrow().selected.clone();
        let running = selected
            .as_ref()
            .and_then(|key| self.inline_pages.borrow().running.get(key).cloned());
        if let (Some(key), Some(running)) = (selected, running) {
            let latest = self
                .session(running.session)
                .and_then(|session| session.turns.get(running.turn))
                .and_then(|turn| {
                    turn.pages
                        .get(running.page)
                        .filter(|page| identity(running.session, running.turn, turn, page) == key)
                })
                .cloned();
            if let Some(page) = latest
                && let Some(html) = page.html.as_ref().filter(|html| html.len() <= MAX_HTML)
            {
                self.inline_pages.borrow_mut().running.insert(
                    key,
                    Running {
                        html: Arc::from(html.as_str()),
                        title: document_title(&page),
                        revision: digest(html.as_str()),
                        version: running.version + 1,
                        saved: false,
                        ..running
                    },
                );
                self.remeasure_thread_turn(running.session, running.turn);
            }
        }
        self.close_sheet(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_identity_does_not_include_the_revision() {
        let turn = Turn::new("Calculator", "9:41");
        let mut page = Page {
            path: "calculator.html".into(),
            html: Some("old".into()),
        };
        let key = identity(SessionId(1), 0, &turn, &page);
        page.html = Some("new".into());
        assert_eq!(key, identity(SessionId(1), 0, &turn, &page));
        assert_ne!(key, identity(SessionId(2), 0, &turn, &page));
        assert_ne!(
            key,
            identity(SessionId(1), 0, &Turn::new("Another turn", "9:41"), &page)
        );
        assert_ne!(key, identity(SessionId(1), 1, &turn, &page));
        page.path = "other.html".into();
        assert_ne!(key, identity(SessionId(1), 0, &turn, &page));
        assert_ne!(digest("old"), digest("new"));
    }
    #[test]
    fn host_geometry_is_in_device_pixels() {
        assert_eq!(
            rectangle(
                Bounds::new(
                    gpui::point(px(10.), px(20.)),
                    gpui::size(px(200.), px(376.))
                ),
                2.
            ),
            [20., 40., 400., 752.]
        );
    }
}
