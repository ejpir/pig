//! Conversation tree: retained selection/documents, virtual rows and native rails.
use super::*;
use super::{
    panels::*,
    session::{Changes, SessionController, SessionEvent},
    tree_graph::{self, Graph},
};
use pi_core::history::{self, Filter, History, Row};
use serde_json::Value;
use std::sync::Arc;

struct EntryRow {
    id: String,
    kind: String,
    body: String,
    tool: Option<String>,
    time: String,
    label: Option<String>,
    result: Option<bool>,
    active: bool,
    leaf: bool,
}
fn project_entry(e: &Value, h: &History, active: bool) -> EntryRow {
    let id = e["id"].as_str().unwrap_or("").to_owned();
    let kind = history::kind(e).to_owned();
    let m = &e["message"];
    let tool = if matches!(kind.as_str(), "tool" | "tool_call") {
        Some(m["toolName"].as_str().unwrap_or("tool").to_owned())
    } else {
        None
    };
    let call = m["toolCallId"].as_str().and_then(|id| {
        h.entries
            .iter()
            .filter_map(|e| e["message"]["content"].as_array())
            .flatten()
            .find(|b| b["type"] == "toolCall" && b["id"] == id)
    });
    let body = if kind == "compaction" {
        e["tokensBefore"]
            .as_u64()
            .map(|n| format!("{} tokens before", count(n)))
            .unwrap_or_else(|| "Context compacted".into())
    } else if let Some(call) = call {
        let args = &call["arguments"];
        args["path"]
            .as_str()
            .map(|p| {
                PathBuf::from(p)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .or_else(|| args["command"].as_str().map(str::to_owned))
            .unwrap_or_else(|| history::preview(e))
    } else {
        let text = history::preview(e);
        tool.as_ref()
            .and_then(|name| text.strip_prefix(&format!("{name} ")))
            .unwrap_or(&text)
            .to_owned()
    };
    EntryRow {
        id: id.clone(),
        kind,
        body,
        tool,
        time: history::time(e),
        label: h.labels.get(&id).cloned(),
        result: (m["role"] == "toolResult").then(|| m["isError"] != true),
        active,
        leaf: h.leaf.as_deref() == Some(&id),
    }
}
pub struct TreeView {
    controller: Entity<SessionController>,
    history: Option<Arc<History>>,
    rows: Vec<Row>,
    entries: Vec<EntryRow>,
    graph: Graph,
    pub selected: Option<String>,
    filter: Filter,
    summarize: bool,
    label_open: bool,
    summary_open: bool,
    label_input: Entity<TextInput>,
    document: Entity<DocumentView>,
    summary_document: Entity<DocumentView>,
    scroll: gpui::UniformListScrollHandle,
    focus: gpui::FocusHandle,
    _subscription: gpui::Subscription,
}
impl TreeView {
    pub fn new(controller: Entity<SessionController>, cx: &mut Context<Self>) -> Self {
        let subscription=cx.subscribe(&controller,|this,controller,event,cx| {
            if matches!(event,SessionEvent::Changed(c) if c.intersects(Changes::HISTORY)) {this.refresh(controller.read(cx).model().history.clone(),cx);}
            if matches!(event,SessionEvent::Changed(c) if c.intersects(Changes::RUN|Changes::STATUS|Changes::JJ)) {cx.notify();}
        });
        let mut this = Self {
            controller,
            history: None,
            rows: vec![],
            entries: vec![],
            graph: Graph::new(vec![], vec![]),
            selected: None,
            filter: Filter::Default,
            summarize: false,
            label_open: false,
            summary_open: false,
            label_input: cx.new(|cx| TextInput::new("Entry label…", cx).compact()),
            document: cx.new(|cx| DocumentView::new(cx).compact()),
            summary_document: cx.new(|cx| DocumentView::new(cx).compact()),
            scroll: gpui::UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            _subscription: subscription,
        };
        this.refresh(this.controller.read(cx).model().history.clone(), cx);
        // The study's selected abandoned branch is an offline preview only.
        if this.controller.read(cx).is_demo()
            && this
                .history
                .as_ref()
                .is_some_and(|h| h.entry("live-answer").is_some())
        {
            this.select("live-answer".into(), cx);
        }
        this
    }
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }
    fn refresh(&mut self, history: Option<Arc<History>>, cx: &mut Context<Self>) {
        self.history = history;
        self.rebuild(cx);
    }
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.rows = self
            .history
            .as_ref()
            .map(|h| h.rows(self.filter))
            .unwrap_or_default();
        self.entries = self
            .history
            .as_ref()
            .map(|h| {
                self.rows
                    .iter()
                    .map(|r| project_entry(&h.entries[r.index], h, r.active))
                    .collect()
            })
            .unwrap_or_default();
        self.graph = Graph::new(
            self.rows.clone(),
            self.history
                .as_ref()
                .map(|h| h.connections(&self.rows))
                .unwrap_or_default(),
        );
        let selected = self
            .selected
            .clone()
            .filter(|id| self.entries.iter().any(|e| &e.id == id))
            .or_else(|| {
                self.entries
                    .iter()
                    .rfind(|e| e.active)
                    .or(self.entries.first())
                    .map(|e| e.id.clone())
            });
        if selected != self.selected || !self.label_open {
            if let Some(id) = selected {
                self.select(id, cx);
            } else {
                self.selected = None;
            }
        }
        cx.notify();
    }
    pub fn select(&mut self, id: String, cx: &mut Context<Self>) {
        if self.selected.as_ref() != Some(&id) {
            self.label_open = false;
            self.summary_open = false;
        }
        self.selected = Some(id.clone());
        if let Some(h) = &self.history {
            if let Some(e) = h.entry(&id) {
                self.document
                    .update(cx, |d, cx| d.set(history::entry_text(e), None, cx));
            }
            let label = h.labels.get(&id).cloned().unwrap_or_default();
            self.label_input
                .update(cx, |input, cx| input.set_content(label, cx));
        }
        let summary = self
            .branch_summary()
            .map(history::entry_text)
            .unwrap_or_default();
        self.summary_document
            .update(cx, |d, cx| d.set(summary, None, cx));
        cx.notify();
    }
    fn branch_summary(&self) -> Option<&Value> {
        let h = self.history.as_ref()?;
        let id = self.selected.as_deref()?;
        if h.active.contains(id) {
            return None;
        }
        h.entries.iter().rev().find(|e| {
            if e["type"] != "branch_summary" {
                return false;
            }
            let mut cursor = e["fromId"].as_str();
            let mut remaining = h.entries.len();
            while let Some(current) = cursor {
                if current == id {
                    return true;
                }
                if remaining == 0 {
                    break;
                }
                remaining -= 1;
                cursor = h.entry(current).and_then(|p| p["parentId"].as_str());
            }
            false
        })
    }
    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.entries.is_empty() {
            return;
        }
        let pos = self
            .entries
            .iter()
            .position(|e| Some(&e.id) == self.selected.as_ref())
            .unwrap_or(0)
            .saturating_add_signed(delta)
            .min(self.entries.len() - 1);
        self.select(self.entries[pos].id.clone(), cx);
        self.scroll
            .scroll_to_item(pos, gpui::ScrollStrategy::Nearest);
    }
    fn row(&self, index: usize, cx: &mut Context<Self>, theme: Theme) -> AnyElement {
        let entry = &self.entries[index];
        let selected = self.selected.as_ref() == Some(&entry.id);
        let id = entry.id.clone();
        let color = if !entry.active {
            theme.muted
        } else if entry.kind == "you" {
            theme.text
        } else {
            theme.secondary
        };
        let marker_size = if entry.kind == "compaction" {
            10.
        } else if entry.kind == "tool" {
            7.
        } else {
            9.
        };
        div()
            .id(("tree-entry", index))
            .debug_selector(move || format!("tree-entry-{index}"))
            .role(gpui::Role::TreeItem)
            .aria_selected(selected)
            .aria_label(entry.body.clone())
            .relative()
            .w_full()
            .h(px(36.))
            .cursor_pointer()
            .child(
                div()
                    .absolute()
                    .top(px(1.))
                    .bottom(px(1.))
                    .left(px(2.))
                    .right(px(2.))
                    .rounded(px(5.))
                    .when(selected, |d| d.bg(theme.selected)),
            )
            .when(selected, |d| {
                d.child(
                    div()
                        .absolute()
                        .left(px(2.))
                        .top(px(7.))
                        .w(px(2.))
                        .h(px(22.))
                        .rounded(px(1.))
                        .bg(theme.accent),
                )
            })
            .child(self.graph.element(index, theme))
            .child(
                div()
                    .absolute()
                    .left(px(self.graph.rail_x(index) - marker_size / 2.))
                    .top(px(tree_graph::CENTER - marker_size / 2.))
                    .child(tree_graph::node(&entry.kind, entry.active, theme)),
            )
            .child(
                h_flex()
                    .absolute()
                    .left(px(self.graph.text_left))
                    .right(px(12.))
                    .top(px(0.))
                    .h(px(32.))
                    .gap(px(12.))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(6.))
                            .when(entry.kind == "compaction", |v| {
                                v.child(
                                    div()
                                        .flex_shrink_0()
                                        .text_size(px(12.5))
                                        .text_color(theme.amber)
                                        .child("Compacted"),
                                )
                            })
                            .when_some(entry.tool.clone(), |v, tool| {
                                v.child(
                                    div()
                                        .flex_shrink_0()
                                        .font_family(MONO)
                                        .text_size(px(11.5))
                                        .text_color(theme.muted)
                                        .child(tool),
                                )
                            })
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(color)
                                    .text_size(px(if entry.tool.is_some() { 11.5 } else { 12.5 }))
                                    .when(entry.tool.is_some(), |d| d.font_family(MONO))
                                    .child(entry.body.clone()),
                            )
                            .when_some(entry.label.clone(), |v, text| {
                                v.child(chip(
                                    text,
                                    theme.amber,
                                    theme.amber.opacity(0.08),
                                    theme.amber.opacity(0.35),
                                ))
                            })
                            .when(entry.leaf, |v| {
                                v.child(chip(
                                    "current leaf".into(),
                                    theme.accent,
                                    theme.selected,
                                    theme.focus,
                                ))
                            }),
                    )
                    .child(
                        h_flex()
                            .w(px(42.))
                            .justify_end()
                            .flex_shrink_0()
                            .when_some(entry.result, |v, success| {
                                v.child(
                                    icon(
                                        if success { "check" } else { "close" },
                                        if success { theme.green } else { theme.coral },
                                    )
                                    .size(px(12.)),
                                )
                            })
                            .when(entry.result.is_none(), |v| {
                                v.child(
                                    div()
                                        .font_family(MONO)
                                        .text_size(px(10.5))
                                        .text_color(theme.faint)
                                        .child(entry.time.clone()),
                                )
                            }),
                    ),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select(id.clone(), cx);
                this.focus(window, cx);
            }))
            .into_any_element()
    }
    pub fn inspector(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme(cx);
        let Some((h, id)) = self.history.as_ref().zip(self.selected.as_deref()) else {
            return empty(
                "Tree",
                "Select an entry to inspect its message and branch.",
                theme,
            )
            .into_any_element();
        };
        let Some(e) = h.entry(id) else {
            return div().into_any_element();
        };
        let active = h.active.contains(id);
        let enabled = self.controller.read(cx).can_navigate();
        let forkable = enabled && active && history::kind(e) == "you";
        let id = id.to_owned();
        let continue_id = id.clone();
        let fork_id = id.clone();
        let tools = e["message"]["content"]
            .as_array()
            .map(|a| a.iter().filter(|b| b["type"] == "toolCall").count())
            .unwrap_or(0);
        let summary = self.branch_summary();
        let has_summary = summary.is_some();
        let summary_note=summary.map(|s|format!("Written when you left this branch at {}.\nAttached to the continuing conversation.",history::time(s))).unwrap_or_else(||"No branch summary recorded for this entry.\nSummaries appear after leaving with summarization.".into());
        v_flex().w_full().min_h_full().px(px(20.)).pt(px(16.)).pb(px(12.)).text_size(px(12.))
            .child(div().font_family(SERIF).italic().text_size(px(22.)).line_height(px(30.)).child(h.labels.get(&id).cloned().unwrap_or_else(||format!("{} entry",history::kind(e)))))
            .child(h_flex().h(px(20.)).gap(px(8.)).child(div().size(px(7.)).rounded_full().bg(if active{theme.accent}else{theme.faint})).child(div().text_color(theme.secondary).child(if active{"On the current path"}else{"Not on the current path"})))
            .child(section("ENTRY","",theme).mt(px(24.)).h(px(28.)))
            .child(pair("Role",history::kind(e).into(),theme)).child(pair("Time",history::time(e),theme))
            .child(pair("Output",e["message"]["usage"]["output"].as_u64().map(|n|format!("{} tokens",count(n))).unwrap_or_else(||"—".into()),theme))
            .child(pair("Tool calls",tools.to_string(),theme)).child(divider(theme).mt(px(10.)))
            .child(section("MESSAGE","",theme).mt(px(12.)))
            .child(div().id("tree-message-preview").mt(px(6.)).max_h(px(160.)).overflow_y_scroll().px(px(12.)).py(px(8.)).rounded(px(6.)).border_1().border_color(theme.line).relative().bg(theme.canvas).child(div().absolute().left_0().top(px(6.)).bottom(px(6.)).w(px(2.)).bg(theme.accent)).child(self.document.clone()))
            .child(h_flex().mt(px(13.)).h(px(20.)).justify_between().child(label("BRANCH SUMMARY",theme))
                .child(div().id("view-branch-summary").text_size(px(10.)).font_family(MONO).text_color(if has_summary{theme.accent}else{theme.faint}).when(has_summary,|d|d.cursor_pointer()).child("[ VIEW ]").on_click(cx.listener(move|this,_,_,cx|{if has_summary{this.summary_open = !this.summary_open;cx.notify();}}))))
            .child(note(summary_note,theme).mt(px(8.)).line_height(px(18.)))
            .when(self.summary_open,|v|v.child(div().id("tree-summary-preview").max_h(px(160.)).overflow_y_scroll().mt(px(8.)).p(px(10.)).bg(theme.canvas).child(self.summary_document.clone())))
            .child(divider(theme).mt(px(10.))).child(section("ACTIONS","",theme).mt(px(12.)))
            .child(primary_button("tree-continue","Continue from here",enabled,theme).debug_selector(||"tree-continue".into()).mt(px(6.)).justify_center().on_click(cx.listener(move|this,_,_,cx|{if enabled{this.controller.update(cx,|c,cx|c.command(Command::NavigateTree {target_id:continue_id.clone(),summarize:this.summarize},cx));}})))
            .child(h_flex().mt(px(8.)).gap(px(8.))
                .child(button("tree-fork","Fork to new session",theme).tooltip(ui::Tooltip::text("Fork is available from user messages on the current path; the original session stays open.")).flex_1().justify_center().when(!forkable,|d|d.text_color(theme.faint).cursor_default()).on_click(cx.listener(move|this,_,_,cx|{if forkable{this.controller.update(cx,|c,cx|c.request_fork(fork_id.clone(),cx));}})))
                .child(button("tree-label-toggle","Label…",theme).w(px(96.)).justify_center().on_click(cx.listener(|this,_,window,cx|{this.label_open = !this.label_open;if this.label_open{this.label_input.focus_handle(cx).focus(window,cx);}cx.notify();}))))
            .when(self.label_open,|v|v.child(v_flex().mt(px(8.)).gap(px(6.)).child(input_box(self.label_input.clone(),theme)).child(primary_button("tree-label","Save label (empty clears)",enabled,theme).on_click(cx.listener(move|this,_,_,cx|{if enabled{let label=this.label_input.read(cx).content().to_owned();this.controller.update(cx,|c,cx|c.command(Command::SetLabel {target_id:id.clone(),label},cx));this.label_open=false;cx.notify();}})))))
            .child(h_flex().id("tree-summarize").role(gpui::Role::CheckBox).aria_label("Summarize the branch I leave (makes a model call)").mt(px(18.)).h(px(18.)).gap(px(8.)).cursor_pointer()
                .child(div().size(px(14.)).rounded(px(3.)).border_1().border_color(if self.summarize{theme.accent}else{theme.chip_line}).bg(if self.summarize{theme.accent}else{theme.canvas}).when(self.summarize,|d|d.child(icon("check",theme.on_accent).size(px(12.)))))
                .child(div().text_color(theme.secondary).child("Summarize the branch I leave"))
                .on_click(cx.listener(|this,_,_,cx|{this.summarize = !this.summarize;cx.notify();})))
            .child(h_flex().items_start().gap(px(7.)).mt(px(28.)).child(icon("info",theme.faint).size(px(13.))).child(note("The tree moves inside this session file.\nSummaries call the model; files stay unchanged.",theme)))
            .child(div().flex_1().min_h(px(24.)))
            .child(div().text_size(px(10.)).text_color(theme.faint).child(if active{"Inspecting an entry on the current path"}else{"Inspecting an entry on an inactive branch"}))
            .into_any_element()
    }
}
fn chip(text: String, color: gpui::Hsla, background: gpui::Hsla, border: gpui::Hsla) -> gpui::Div {
    div()
        .flex_shrink_0()
        .h(px(16.))
        .px(px(5.))
        .rounded(px(3.))
        .border_1()
        .border_color(border)
        .bg(background)
        .font_family(MONO)
        .text_size(px(9.5))
        .line_height(px(14.))
        .text_color(color)
        .child(text)
}
impl Render for TreeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let weak = cx.entity().downgrade();
        let filters = h_flex()
            .h(px(24.))
            .p(px(2.))
            .rounded(px(5.))
            .border_1()
            .border_color(theme.chip_line)
            .bg(theme.hover)
            .children(Filter::ALL.into_iter().enumerate().map(|(i, filter)| {
                h_flex()
                    .id(("tree-filter", i))
                    .debug_selector(move || format!("tree-filter-{i}"))
                    .h(px(20.))
                    .px(px(12.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_size(px(11.))
                    .line_height(px(16.))
                    .text_color(if self.filter == filter {
                        theme.text
                    } else {
                        theme.muted
                    })
                    .when(self.filter == filter, |v| {
                        v.bg(theme.chip).border_1().border_color(theme.chip_line)
                    })
                    .child(filter.label())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.filter = filter;
                        this.rebuild(cx);
                        this.focus(window, cx);
                    }))
            }));
        let legend = h_flex()
            .gap(px(22.))
            .children(
                [
                    ("you", "you"),
                    ("pi", "pi"),
                    ("tool", "tool call"),
                    ("compaction", "compaction"),
                ]
                .into_iter()
                .map(|(kind, title)| {
                    h_flex()
                        .gap(px(6.))
                        .child(tree_graph::node(kind, true, theme))
                        .child(note(title, theme))
                }),
            )
            .child(
                h_flex()
                    .gap(px(6.))
                    .child(div().w(px(12.)).h(px(1.6)).bg(theme.accent))
                    .child(note("current path", theme)),
            );
        v_flex().id("tree-view").debug_selector(||"tree-view".into()).role(gpui::Role::Tree).track_focus(&self.focus).size_full().min_h_0().px(px(8.)).pt(px(12.))
            .on_action(cx.listener(|this,_:&PreviousChoice,_,cx|{this.move_selection(-1,cx);cx.stop_propagation();}))
            .on_action(cx.listener(|this,_:&NextChoice,_,cx|{this.move_selection(1,cx);cx.stop_propagation();}))
            .child(h_flex().mx(px(16.)).h(px(24.)).gap(px(8.)).child(filters).child(div().flex_1()).child(note(self.history.as_ref().map(|h|format!("{} entries · {} branches",h.entries.len(),h.branch_count())).unwrap_or_else(||"Loading history…".into()),theme)))
            .when(self.rows.is_empty(),|v|v.child(empty("No matching entries","History includes abandoned branches and pre-compaction entries.",theme)))
            .child(gpui::uniform_list("tree-list",self.rows.len(),move|range,_,cx|weak.update(cx,|this,cx|range.map(|i|this.row(i,cx,theme)).collect::<Vec<_>>()).unwrap_or_default()).track_scroll(&self.scroll).mt(px(18.)).flex_1().min_h_0().w_full())
            .child(v_flex().mx(px(24.)).pt(px(12.)).pb(px(8.)).gap(px(8.)).child(legend.ml(px(-4.5))).child(note("Select a message to continue from it. Branches you leave stay in the file.",theme)))
    }
}
