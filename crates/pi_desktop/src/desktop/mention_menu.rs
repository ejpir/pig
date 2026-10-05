//! The composer's `@` menu (design study 08): one grouped list of files, directories, symbols,
//! jj turns, sessions, terminal output and problems, filtered by what follows the
//! `@`. A choice becomes a chip in the draft; `mentions::prompt` turns the chips
//! into the text pi receives.
use super::composer::ComposerView;
use super::mentions::{Body, Kind, Mention};
use super::*;
use gpui::{HighlightStyle, StyledText, Task, WeakEntity, deferred};
use pi_core::clock::{parse_timestamp, short_age};
use std::{collections::HashMap, ops::Range, path::Path, time::Duration};

#[path = "mention_matching.rs"]
mod matching;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const MENU_WIDTH: f32 = 360.;
const PREVIEW_WIDTH: f32 = 300.;
const FILES: usize = 5;
const OTHERS: usize = 3;
const TERMINAL_LINES: usize = 40;
const SYMBOL_DELAY: Duration = Duration::from_millis(150);

/// What the menu lists besides the session's own history.
#[derive(Default)]
pub struct Sources {
    pub files: Option<WeakEntity<super::files::FilesView>>,
    pub terminal: Option<WeakEntity<super::terminal::TerminalDrawer>>,
}

#[derive(Clone)]
struct Item {
    kind: Kind,
    name: String,
    /// Byte positions in `name` that match the query.
    matched: Vec<usize>,
    detail: String,
    filter: Option<String>,
    choice: Choice,
}

#[derive(Clone)]
enum Choice {
    Ready(Mention),
    /// The problems' text is read from the buffer when chosen.
    Problems(pi_editor::ProblemFile, Mention),
}

impl Choice {
    fn mention(&self) -> &Mention {
        match self {
            Self::Ready(mention) | Self::Problems(_, mention) => mention,
        }
    }
}

/// First lines and size of a file, shown beside the highlighted file.
#[derive(Clone)]
struct Preview {
    path: String,
    summary: String,
    lines: Vec<String>,
}

#[derive(Default)]
pub struct MentionMenu {
    /// The `@query` range in the draft and the query, while the menu is open.
    open: Option<(Range<usize>, String)>,
    /// Where a dismissed `@` starts; the menu stays closed for it.
    dismissed: Option<usize>,
    items: Vec<Item>,
    index: usize,
    scroll: gpui::ScrollHandle,
    match_task: Option<Task<()>>,
    match_cancel: Arc<AtomicBool>,
    match_generation: u64,
    searching: bool,
    symbols: Vec<pi_editor::SymbolHit>,
    symbols_for: Option<String>,
    symbols_task: Option<Task<()>>,
    previews: HashMap<String, Preview>,
    preview_task: Option<Task<()>>,
    /// Mention ids whose text is still being read.
    pending: HashMap<usize, Task<()>>,
    /// A send waiting for them, and whether it was a follow-up.
    send_when_ready: Option<bool>,
    /// The chip ids last reported, to tell the inspector when they change.
    chips: Vec<usize>,
}

impl Drop for MentionMenu {
    fn drop(&mut self) {
        self.match_cancel.store(true, Ordering::Release);
    }
}

impl MentionMenu {
    fn cancel_matching(&mut self) {
        self.match_cancel.store(true, Ordering::Release);
        self.match_generation = self.match_generation.wrapping_add(1);
        self.match_task = None;
        self.searching = false;
    }
    pub(super) fn index(&self) -> usize {
        self.index
    }
}

impl ComposerView {
    pub fn set_sources(&mut self, sources: Sources, cx: &mut Context<Self>) {
        if let Some(files) = sources.files.as_ref().and_then(WeakEntity::upgrade) {
            // The file list arrives after the project scan starts.
            self._subscriptions.push(cx.observe(&files, |this, _, cx| {
                if this.mention.open.is_some() {
                    this.refresh_mentions(cx);
                }
            }));
        }
        self.sources = sources;
    }

    /// The `@query` just before the cursor, unless dismissed or another menu is open.
    pub(super) fn mention_query(&self, cx: &App) -> Option<(Range<usize>, String)> {
        if self.picker.is_some() || self.slash_query(cx).is_some() {
            return None;
        }
        let input = self.input.read(cx);
        let content = input.content();
        let cursor = input.cursor();
        let start = content[..cursor].rfind(char::is_whitespace).map_or(0, |i| {
            i + content[i..].chars().next().map_or(1, char::len_utf8)
        });
        let token = &content[start..cursor];
        let query = token.strip_prefix('@')?;
        (!query.contains('@') && self.mention.dismissed != Some(start))
            .then(|| (start..cursor, query.to_owned()))
    }

    pub(super) fn mention_open(&self) -> bool {
        self.mention.open.is_some()
    }

    /// Called when the draft changes: opens, filters or closes the menu.
    pub(super) fn update_mentions(&mut self, cx: &mut Context<Self>) {
        // Mentions whose chips were deleted are gone.
        let chips: Vec<usize> = self.input.read(cx).chips().iter().map(|c| c.id).collect();
        if chips != self.mention.chips {
            self.mention.chips = chips.clone();
            cx.emit(super::composer::ComposerEvent::Mentions);
        }
        self.mentions.retain(|id, _| chips.contains(id));
        self.mention.pending.retain(|id, _| chips.contains(id));
        if let Some(start) = self.mention.dismissed {
            let content = self.input.read(cx).content();
            if content.get(start..start + 1) != Some("@") {
                self.mention.dismissed = None;
            }
        }
        let open = self.mention_query(cx);
        if open.as_ref().map(|(_, q)| q) != self.mention.open.as_ref().map(|(_, q)| q) {
            self.mention.cancel_matching();
            self.mention.items.clear();
            self.mention.index = 0;
            self.mention.scroll.set_offset(gpui::point(px(0.), px(0.)));
        }
        let was_open = self.mention.open.is_some();
        if !was_open && open.is_some() && self.controller.read(cx).is_remote() {
            // Remote files may have changed since the previous menu opening.
            self.mention.previews.clear();
            self.mention.preview_task = None;
        }
        self.mention.open = open;
        if self.mention.open.is_some() {
            if !was_open && let Some(files) = self.files() {
                // The project's file list loads on first use.
                files.update(cx, |files, cx| files.load_browser(cx));
            }
            self.refresh_mentions(cx);
        } else {
            self.mention.cancel_matching();
            self.mention.items.clear();
        }
    }

    fn files(&self) -> Option<Entity<super::files::FilesView>> {
        self.sources.files.as_ref().and_then(WeakEntity::upgrade)
    }

    /// The folder mentioned paths are relative to: the project, else the session's folder.
    pub(super) fn mention_root(&self, cx: &App) -> std::path::PathBuf {
        self.files()
            .map(|files| files.read(cx).root().clone())
            .unwrap_or_else(|| self.controller.read(cx).model().cwd.clone())
    }

    fn refresh_mentions(&mut self, cx: &mut Context<Self>) {
        let Some((range, query)) = self.mention.open.clone() else {
            return;
        };
        let open = (range, query.clone());
        let files = self.file_items(Kind::File, cx);
        let directories = self.file_items(Kind::Directory, cx);
        let directory_first = query.ends_with('/')
            || directories
                .iter()
                .any(|item| item.name.trim_end_matches('/').eq_ignore_ascii_case(&query));
        let mut groups = if directory_first {
            vec![directories, files]
        } else {
            vec![files, directories]
        };
        groups.extend([
            self.symbol_items(&query, cx),
            self.turn_items(cx),
            self.session_items(cx),
            self.terminal_items(cx),
            self.problem_items(cx),
        ]);
        self.mention.cancel_matching();
        self.mention.searching = true;
        let generation = self.mention.match_generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.mention.match_cancel = cancel.clone();
        let executor = cx.background_executor().clone();
        let task = executor.spawn({
            let executor = executor.clone();
            async move { matching::filter(groups, &query, &cancel, executor).await }
        });
        self.mention.match_task = Some(cx.spawn(async move |this, cx| {
            let items = task.await;
            this.update(cx, |this, cx| {
                if this.mention.match_generation != generation
                    || this.mention.open.as_ref() != Some(&open)
                {
                    return;
                }
                this.mention.searching = false;
                this.mention.items = items;
                this.mention.index = this
                    .mention
                    .index
                    .min(this.mention.items.len().saturating_sub(1));
                this.load_preview(cx);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn file_items(&self, kind: Kind, cx: &App) -> Vec<Item> {
        let Some(files) = self.files() else {
            return Vec::new();
        };
        let changed = self.controller.read(cx).model().changed_files();
        let mut scored: Vec<(i64, Item)> = files
            .read(cx)
            .file_entries()
            .iter()
            .filter(|entry| entry.directory == (kind == Kind::Directory))
            .map(|entry| {
                let relative = entry.relative.as_str();
                let (name, folder) = match relative.rsplit_once('/') {
                    Some((folder, name)) => (name, folder),
                    None => (relative, "."),
                };
                let label = if entry.directory {
                    format!("{name}/")
                } else {
                    name.to_owned()
                };
                let path = if entry.directory {
                    format!("{relative}/")
                } else {
                    relative.to_owned()
                };
                // Files pi changed in this session come first, then shallow ones.
                let boost = if changed.iter().any(|c| c.ends_with(relative)) {
                    5_000
                } else {
                    0
                };
                let depth = relative.matches('/').count() as i64;
                (
                    boost - depth,
                    Item {
                        kind,
                        name: label.clone(),
                        matched: Vec::new(),
                        detail: folder.to_owned(),
                        filter: None,
                        choice: Choice::Ready(Mention {
                            kind,
                            label,
                            body: Body::Path { path, line: None },
                        }),
                    },
                )
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.detail.cmp(&b.1.detail)));
        scored.into_iter().map(|(_, item)| item).collect()
    }

    fn language_project(&self, cx: &App) -> Option<Entity<pi_editor::Project>> {
        // Remote LSP is still deferred: even a matching local cache entry must
        // never provide symbols/diagnostics for a remote project.
        if self.controller.read(cx).is_remote() {
            return None;
        }
        let root = self.files()?.read(cx).root().clone();
        pi_editor::language_project(&root, cx)
    }

    /// Workspace symbols need running language servers; they arrive after a short pause.
    fn symbol_items(&mut self, query: &str, cx: &mut Context<Self>) -> Vec<Item> {
        if query.chars().count() < 2 {
            return Vec::new();
        }
        let Some(project) = self.language_project(cx) else {
            return Vec::new();
        };
        if self.mention.symbols_for.as_deref() != Some(query) {
            self.mention.symbols_for = Some(query.to_owned());
            let query = query.to_owned();
            self.mention.symbols_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SYMBOL_DELAY).await;
                let hits = cx
                    .update(|cx| pi_editor::workspace_symbols(&project, &query, cx))
                    .await;
                this.update(cx, |this, cx| {
                    if this.mention.symbols_for.as_deref() == Some(query.as_str()) {
                        this.mention.symbols = hits;
                        this.refresh_mentions(cx);
                    }
                })
                .ok();
            }));
        }
        self.mention
            .symbols
            .iter()
            .map(|hit| {
                let file = hit.path.rsplit('/').next().unwrap_or(&hit.path);
                Item {
                    kind: Kind::Symbol,
                    name: hit.name.clone(),
                    matched: Vec::new(),
                    filter: None,
                    detail: format!("{file}:{} · {}", hit.line, hit.kind),
                    choice: Choice::Ready(Mention {
                        kind: Kind::Symbol,
                        label: hit.name.clone(),
                        body: Body::Path {
                            path: hit.path.clone(),
                            line: Some(hit.line),
                        },
                    }),
                }
            })
            .collect()
    }

    fn turn_items(&self, cx: &App) -> Vec<Item> {
        let records = &self.controller.read(cx).jj().records;
        records
            .iter()
            .rev()
            .map(|record| {
                let description = record.description.lines().next().unwrap_or("").trim();
                let mut text = format!(
                    "jj change {} (`jj diff -r {}` shows it): {description}\n",
                    record.short, record.short
                );
                for file in &record.diff {
                    text.push_str(&format!(
                        "{} +{} −{}\n",
                        file.path, file.added, file.removed
                    ));
                }
                Item {
                    kind: Kind::Turn,
                    name: record.short.clone(),
                    matched: Vec::new(),
                    filter: Some(format!("{} {description}", record.short)),
                    detail: format!("{} · jj", truncate(description, 28)),
                    choice: Choice::Ready(Mention {
                        kind: Kind::Turn,
                        label: record.short.clone(),
                        body: Body::Text(text),
                    }),
                }
            })
            .collect()
    }

    fn session_items(&self, cx: &App) -> Vec<Item> {
        let model = self.controller.read(cx).model();
        let current = model.state.session_id.as_deref();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_millis() as u64);
        model
            .saved
            .iter()
            .filter(|saved| Some(saved.id.as_str()) != current)
            .map(|saved| {
                let title = saved.title().to_owned();
                let age = saved
                    .modified
                    .as_deref()
                    .and_then(parse_timestamp)
                    .map(|then| short_age(then, now));
                let mut text = format!("pi session “{title}”, saved in {}", saved.path);
                if !saved.first_message.is_empty() {
                    text.push_str(&format!("\nIt began: {}", saved.first_message.trim()));
                }
                Item {
                    kind: Kind::Session,
                    name: title.clone(),
                    matched: Vec::new(),
                    filter: None,
                    detail: match age {
                        Some(age) if age == "now" => "pi · now".into(),
                        Some(age) => format!("pi · {age} ago"),
                        None => "pi".into(),
                    },
                    choice: Choice::Ready(Mention {
                        kind: Kind::Session,
                        label: title,
                        body: Body::Text(text),
                    }),
                }
            })
            .collect()
    }

    fn terminal_items(&self, cx: &App) -> Vec<Item> {
        let Some(terminal) = self.sources.terminal.as_ref().and_then(WeakEntity::upgrade) else {
            return Vec::new();
        };
        terminal
            .read(cx)
            .outputs(TERMINAL_LINES, cx)
            .into_iter()
            .filter(|(_, lines)| !lines.is_empty())
            .map(|(label, lines)| {
                let count = lines.len();
                let chip = format!(
                    "{label} · {count} {}",
                    if count == 1 { "line" } else { "lines" }
                );
                Item {
                    kind: Kind::Terminal,
                    name: label,
                    matched: Vec::new(),
                    filter: None,
                    detail: format!(
                        "last output · {count} {}",
                        if count == 1 { "line" } else { "lines" }
                    ),
                    choice: Choice::Ready(Mention {
                        kind: Kind::Terminal,
                        label: chip,
                        body: Body::Text(lines.join("\n")),
                    }),
                }
            })
            .collect()
    }

    fn problem_items(&self, cx: &App) -> Vec<Item> {
        let Some(project) = self.language_project(cx) else {
            return Vec::new();
        };
        pi_editor::problem_files(&project, cx)
            .into_iter()
            .map(|file| {
                let name = file
                    .relative
                    .rsplit('/')
                    .next()
                    .unwrap_or(&file.relative)
                    .to_owned();
                let count = file.errors + file.warnings;
                let title = format!("{count} in {name}");
                let label = format!(
                    "{count} {} in {name}",
                    if count == 1 { "problem" } else { "problems" }
                );
                Item {
                    kind: Kind::Problems,
                    name: title,
                    matched: Vec::new(),
                    filter: None,
                    detail: format!("from {}", file.servers.join(", ")),
                    choice: Choice::Problems(
                        file,
                        Mention {
                            kind: Kind::Problems,
                            label,
                            body: Body::Text(String::new()),
                        },
                    ),
                }
            })
            .collect()
    }

    pub(super) fn move_mention(&mut self, direction: isize, cx: &mut Context<Self>) {
        let count = self.mention.items.len();
        if count == 0 {
            return;
        }
        self.mention.index =
            (self.mention.index as isize + direction).rem_euclid(count as isize) as usize;
        let groups = self.mention.items[..=self.mention.index]
            .windows(2)
            .filter(|items| items[0].kind != items[1].kind)
            .count();
        self.mention
            .scroll
            .scroll_to_item(self.mention.index + 1 + groups * 2);
        self.load_preview(cx);
        cx.notify();
    }

    pub(super) fn dismiss_mentions(&mut self, cx: &mut Context<Self>) {
        if let Some((range, _)) = self.mention.open.take() {
            self.mention.dismissed = Some(range.start);
        }
        self.mention.cancel_matching();
        self.mention.items.clear();
        cx.notify();
    }

    /// ⏎: the highlighted item becomes a chip.
    pub(super) fn choose_mention(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some((range, _)), Some(item)) = (
            self.mention.open.clone(),
            self.mention.items.get(index).cloned(),
        ) else {
            return;
        };
        let id = self.next_mention;
        self.next_mention += 1;
        let mention = item.choice.mention().clone();
        self.input.update(cx, |input, cx| {
            input.insert_chip(range, &mention.label, mention.kind.icon(), id, cx)
        });
        self.mentions.insert(id, mention);
        if let Choice::Problems(file, _) = item.choice
            && let Some(project) = self.language_project(cx)
        {
            let text = pi_editor::problems_text(&project, &file, cx);
            let task = cx.spawn(async move |this, cx| {
                let text = text.await;
                this.update(cx, |this, cx| {
                    if let Some(mention) = this.mentions.get_mut(&id) {
                        mention.body = Body::Text(text);
                    }
                    this.mention.pending.remove(&id);
                    if this.mention.pending.is_empty()
                        && let Some(follow_up) = this.mention.send_when_ready.take()
                    {
                        this.send_prompt(follow_up, cx);
                    }
                    cx.notify();
                })
                .ok();
            });
            self.mention.pending.insert(id, task);
        }
        self.mention.open = None;
        self.mention.cancel_matching();
        self.mention.items.clear();
        self.input.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    /// ⇥: a file, directory or symbol is written out as text (`@path`) instead of a chip.
    pub(super) fn complete_mention(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.mention.index;
        let (Some((range, _)), Some(item)) = (
            self.mention.open.clone(),
            self.mention.items.get(index).cloned(),
        ) else {
            return;
        };
        match &item.choice {
            Choice::Ready(
                mention @ Mention {
                    body: Body::Path { .. },
                    ..
                },
            ) => {
                let text = format!("{} ", mention.inline());
                self.input
                    .update(cx, |input, cx| input.replace(range, &text, cx));
                self.input.focus_handle(cx).focus(window, cx);
                self.mention.open = None;
                self.mention.cancel_matching();
                self.mention.items.clear();
                cx.notify();
            }
            _ => self.choose_mention(index, window, cx),
        }
    }

    /// Whether sending must wait for mentioned text still being read.
    pub(super) fn mentions_pending(&mut self, follow_up: bool) -> bool {
        if self.mention.pending.is_empty() {
            return false;
        }
        self.mention.send_when_ready = Some(follow_up);
        true
    }

    /// Chips in the draft, for the inspector: kind, label and what pi gets.
    pub fn draft_mentions(&self, cx: &App) -> Vec<(Kind, String, String)> {
        self.input
            .read(cx)
            .chips()
            .iter()
            .filter_map(|chip| self.mentions.get(&chip.id))
            .map(|mention| (mention.kind, mention.label.clone(), mention.how()))
            .collect()
    }

    fn load_preview(&mut self, cx: &mut Context<Self>) {
        let Some(Item {
            choice:
                Choice::Ready(Mention {
                    kind: Kind::File,
                    body: Body::Path { path, .. },
                    ..
                }),
            ..
        }) = self.mention.items.get(self.mention.index)
        else {
            return;
        };
        if self.mention.previews.contains_key(path) {
            return;
        }
        let Some(files) = self.files() else {
            return;
        };
        let path = path.clone();
        let read = if self.controller.read(cx).is_remote() {
            let Some(read) = files.read(cx).read_remote(path.clone(), cx) else {
                return; // No local fallback for SSH sessions.
            };
            cx.background_executor().spawn({
                let path = path.clone();
                async move {
                    match read.await {
                        Ok(document) => text_preview(&path, &document.text),
                        Err(error) => Preview {
                            path,
                            summary: format!("Remote preview unavailable: {error:#}"),
                            lines: Vec::new(),
                        },
                    }
                }
            })
        } else {
            let root = files.read(cx).root().clone();
            cx.background_executor().spawn({
                let path = path.clone();
                async move { read_preview(&root, &path) }
            })
        };
        self.mention.preview_task = Some(cx.spawn(async move |this, cx| {
            let preview = read.await;
            this.update(cx, |this, cx| {
                this.mention.previews.insert(path, preview);
                cx.notify();
            })
            .ok();
        }));
    }

    pub(super) fn mention_view(
        &self,
        window: &Window,
        cx: &Context<Self>,
        theme: Theme,
    ) -> Option<AnyElement> {
        let (_, query) = self.mention.open.as_ref()?;
        let surface = if theme.light { theme.chip } else { theme.bar };
        let panel = |element: gpui::Div| {
            element
                .rounded(px(8.))
                .border_1()
                .border_color(theme.chip_line)
                .bg(surface)
                .shadow_lg()
        };
        let mut list = v_flex()
            .id("mention-list")
            .max_h((window.viewport_size().height - px(190.)).max(px(100.)))
            .overflow_y_scroll()
            .track_scroll(&self.mention.scroll)
            .py(px(4.));
        let mut previous = None;
        let mut selected_top = 0f32;
        let mut top = 4f32;
        for (index, item) in self.mention.items.iter().enumerate() {
            if previous != Some(item.kind) {
                if previous.is_some() {
                    list = list.child(div().my(px(4.)).h(px(1.)).bg(theme.line));
                    top += 9.;
                }
                list = list.child(
                    div()
                        .h(px(24.))
                        .px(px(14.))
                        .pt(px(5.))
                        .text_size(px(12.))
                        .text_color(theme.muted)
                        .child(item.kind.group()),
                );
                top += 24.;
                previous = Some(item.kind);
            }
            let selected = index == self.mention.index;
            if selected {
                selected_top = top;
            }
            top += 26.;
            list = list.child(
                div().px(px(6.)).child(
                    h_flex()
                        .id(("mention-item", index))
                        .debug_selector(move || format!("mention-item-{index}"))
                        .h(px(26.))
                        .px(px(10.))
                        .gap(px(9.))
                        .rounded(px(5.))
                        .cursor_pointer()
                        .when(selected, |row| row.bg(theme.selected))
                        .hover(move |row| row.bg(theme.hover))
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            if *hovered && this.mention.index != index {
                                this.mention.index = index;
                                this.load_preview(cx);
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.choose_mention(index, window, cx)
                        }))
                        .child(
                            icon(
                                item.kind.icon(),
                                if selected { theme.accent } else { theme.muted },
                            )
                            .size(px(14.)),
                        )
                        .child(matched(&item.name, &item.matched, theme))
                        .child(div().flex_1())
                        .child(
                            div()
                                .flex_shrink(1.)
                                .min_w_0()
                                .truncate()
                                .text_size(px(10.5))
                                .text_color(theme.faint)
                                .child(item.detail.clone()),
                        ),
                ),
            );
        }
        let empty = self.mention.items.is_empty();
        let file_status = self
            .files()
            .and_then(|files| files.read(cx).browser_status().map(str::to_owned));
        let preview = self
            .mention
            .items
            .get(self.mention.index)
            .and_then(|item| match &item.choice {
                Choice::Ready(Mention {
                    kind: Kind::File,
                    body: Body::Path { path, .. },
                    ..
                }) => self.mention.previews.get(path),
                _ => None,
            })
            .cloned();
        let menu = div()
            .id("mention-menu")
            .debug_selector(|| "mention-menu".into())
            .absolute()
            .left(px(8.))
            .map(|menu| {
                if self.menus_below() {
                    menu.top(relative(1.)).mt(px(6.))
                } else {
                    menu.bottom(relative(1.)).mb(px(6.))
                }
            })
            .child(
                panel(v_flex().w(px(MENU_WIDTH)).occlude())
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss_mentions(cx)))
                    .child(if empty {
                        div()
                            .px(px(14.))
                            .py(px(9.))
                            .text_size(px(12.))
                            .text_color(theme.faint)
                            .child(if self.mention.searching {
                                "Searching…".to_owned()
                            } else if let Some(status) = file_status {
                                status
                            } else if query.is_empty() {
                                "Nothing to mention yet: open the project's files first.".to_owned()
                            } else {
                                format!("Nothing matches @{query}")
                            })
                            .into_any_element()
                    } else {
                        list.into_any_element()
                    })
                    .child(
                        div()
                            .border_t_1()
                            .border_color(theme.line)
                            .px(px(14.))
                            .py(px(5.))
                            .font_family(MONO)
                            .text_size(px(10.))
                            .text_color(theme.faint)
                            .child("↑↓ select · ⏎ insert · ⇥ complete · esc dismiss"),
                    ),
            )
            .when_some(preview, |menu, preview| {
                menu.child(
                    panel(
                        v_flex()
                            .absolute()
                            .left(px(MENU_WIDTH + 8.))
                            .top(px((selected_top - 26.).max(0.)))
                            .w(px(PREVIEW_WIDTH)),
                    )
                    .debug_selector(|| "mention-preview".into())
                    .p(px(12.))
                    .gap(px(4.))
                    .child(
                        div()
                            .truncate()
                            .font_family(MONO)
                            .text_size(px(11.))
                            .text_color(theme.secondary)
                            .child(preview.path),
                    )
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(theme.faint)
                            .child(preview.summary),
                    )
                    .when(!preview.lines.is_empty(), |panel| {
                        panel.child(
                            v_flex()
                                .mt(px(4.))
                                .p(px(10.))
                                .rounded(px(5.))
                                .bg(theme.deep)
                                .font_family(MONO)
                                .text_size(px(10.5))
                                .line_height(px(16.))
                                .text_color(theme.code)
                                .children(
                                    preview.lines.into_iter().map(|line| {
                                        div().truncate().whitespace_nowrap().child(line)
                                    }),
                                ),
                        )
                    }),
                )
            });
        Some(deferred(menu).with_priority(1).into_any_element())
    }
}

fn matched(name: &str, positions: &[usize], theme: Theme) -> impl IntoElement {
    let highlights = positions.iter().filter_map(|&at| {
        let end = at + name[at..].chars().next()?.len_utf8();
        Some((
            at..end,
            HighlightStyle {
                color: Some(theme.accent),
                ..Default::default()
            },
        ))
    });
    div()
        .flex_shrink_0()
        .max_w(px(210.))
        .truncate()
        .font_family(MONO)
        .text_size(px(12.))
        .text_color(theme.text)
        .child(StyledText::new(name.to_owned()).with_highlights(highlights.collect::<Vec<_>>()))
}

fn truncate(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

/// Off the UI thread: size, language and the first lines of a text file.
fn read_preview(root: &Path, relative: &str) -> Preview {
    let path = root.join(relative);
    let text = std::fs::read(&path)
        .ok()
        .filter(|bytes| {
            bytes.len() <= 2 * 1024 * 1024 && !bytes[..bytes.len().min(8192)].contains(&0)
        })
        .and_then(|bytes| String::from_utf8(bytes).ok());
    match text {
        Some(text) => text_preview(relative, &text),
        None => Preview {
            path: relative.to_owned(),
            summary: "Not a text file".into(),
            lines: Vec::new(),
        },
    }
}

fn text_preview(path: &str, text: &str) -> Preview {
    let language = language_name(path);
    Preview {
        path: path.to_owned(),
        summary: match text.lines().count() {
            1 => format!("1 line · {language}"),
            lines => format!("{lines} lines · {language}"),
        },
        lines: text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .take(3)
            .map(|line| line.replace('\t', "    "))
            .collect(),
    }
}

fn language_name(path: &str) -> String {
    let extension = path.rsplit_once('.').map_or("", |(_, e)| e);
    match extension {
        "ts" | "tsx" | "mts" | "cts" => "TypeScript",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "rs" => "Rust",
        "py" => "Python",
        "go" => "Go",
        "json" => "JSON",
        "md" => "Markdown",
        "toml" => "TOML",
        "yaml" | "yml" => "YAML",
        "sh" | "bash" => "Shell",
        "css" => "CSS",
        "html" => "HTML",
        "" => "Plain text",
        other => return other.to_uppercase(),
    }
    .to_owned()
}

#[cfg(test)]
#[path = "mention_remote_tests.rs"]
mod remote_tests;
