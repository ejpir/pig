//! Context usage and compaction controls. Values are authoritative RPC metadata;
//! unavailable prompt internals are explicitly unknown, not filesystem guesses.
use super::*;
use super::{
    panels::*,
    session::{Changes, SessionController, SessionEvent},
};
use pi_core::history::{self, ResponseUsage};

pub struct ContextView {
    controller: Entity<SessionController>,
    instructions: Entity<TextInput>,
    summary: Entity<DocumentView>,
    show_summary: bool,
    /// The reported-sources disclosure.
    sources_open: bool,
    scroll: ScrollHandle,
    usage: Vec<ResponseUsage>,
    _subscription: gpui::Subscription,
}
impl ContextView {
    pub fn new(controller: Entity<SessionController>, cx: &mut Context<Self>) -> Self {
        let subscription=cx.subscribe(&controller,|this,_,event,cx|{if matches!(event,SessionEvent::Changed(c) if c.intersects(Changes::HISTORY | Changes::METADATA | Changes::CONTEXT | Changes::RUN | Changes::CATALOG | Changes::STATUS)) {this.refresh(cx);}});
        let mut this = Self {
            controller,
            instructions: cx.new(|cx| {
                TextInput::new("Keep the provider decisions and the test plan…", cx).multiline(4)
            }),
            summary: cx.new(DocumentView::new),
            show_summary: false,
            sources_open: false,
            scroll: ScrollHandle::new(),
            usage: vec![],
            _subscription: subscription,
        };
        this.refresh(cx);
        this
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let history = self.controller.read(cx).model().history.as_ref();
        self.usage = history
            .map(|h| history::response_usage(h))
            .unwrap_or_default();
        let text = history
            .and_then(|h| h.path().filter(|e| e["type"] == "compaction").last())
            .and_then(|e| e["summary"].as_str())
            .unwrap_or("")
            .to_string();
        self.summary.update(cx, |d, cx| d.set(text, None, cx));
        cx.notify();
    }
    pub fn inspector(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme(cx);
        let c = self.controller.read(cx);
        let model = c.model();
        let enabled = c.can_navigate();
        let toggle = c.ready();
        let auto = model.state.auto_compaction_enabled;
        let percent = model
            .stats
            .context_usage
            .as_ref()
            .and_then(|u| u.percent)
            .map(|p| format!("{p:.0}% of window used"))
            .unwrap_or_else(|| "Context usage not yet reported".into());
        let settings = model.settings.as_ref();
        v_flex().p(px(20.)).gap(px(8.)).child(heading("Context",percent,theme))
            .child(h_flex().mt(px(8.)).h(px(28.)).gap(px(12.))
                .child(div().flex_1().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).child("Auto-compaction"))
                .child(super::app_views::toggle("context-auto", auto, theme).debug_selector(||"context-auto".into())
                    .role(gpui::Role::Switch).aria_toggled(if auto { gpui::Toggled::True } else { gpui::Toggled::False })
                    .when(!toggle, |switch| switch.opacity(0.5))
                    .on_click(cx.listener(move|this,_,_,cx|{if toggle{this.controller.update(cx,|c,cx|c.command(Command::SetAutoCompaction {enabled:!auto},cx));}}))))
            .child(note("Saved in Pi's settings, so future runs use it too.",theme)).child(pair("Reserve for reply","Not exposed by RPC".into(),theme)).child(pair("Keep recent","Not exposed by RPC".into(),theme))
            .child(divider(theme)).child(label("Compact now",theme).mt(px(8.))).child(note("Optional instructions",theme)).child(input_box(self.instructions.clone(),theme))
            .child(primary_button("context-compact","Compact…",enabled,theme).w_full().justify_center().debug_selector(||"context-compact".into()).on_click(cx.listener(move|this,_,_,cx|{if enabled{let text=this.instructions.read(cx).content().trim().to_owned();this.controller.update(cx,|c,cx|c.command(Command::Compact {custom_instructions:(!text.is_empty()).then_some(text)},cx));}})))
            .child(note("Uses the selected model to summarize. Entries stay in the file. Wait for active runs and file recording to finish.",theme))
            .child(divider(theme)).child(section("RETRIES",if settings.and_then(|s|s["autoRetry"].as_bool())==Some(true){"auto-retry on"}else{""},theme))
            .when(model.retries.is_empty(),|v|v.child(note("No retry events observed in this connection.",theme)))
            .children(model.retries.iter().rev().take(6).map(|r|{
                let s=if r["type"]=="auto_retry_end" {if r["success"]==true {"Recovered".to_owned()}else{format!("Retry ended · {}",r["finalError"].as_str().unwrap_or("no further details"))}}else{format!("Attempt {} · {}",r["attempt"].as_u64().unwrap_or(1),r["errorMessage"].as_str().or(r["error"].as_str()).unwrap_or("Retry scheduled"))};
                note(s,theme)
            }))
            .child(divider(theme)).child(section("PROMPT CACHE","",theme)).child(pair("Warming",settings.and_then(|s|s["cacheWarming"].as_str()).unwrap_or("Not reported").into(),theme))
            .child(pair("Cache misses","Not exposed by RPC".into(),theme)).into_any_element()
    }
}
/// One figure in the summary row: plain, with a hairline between figures.
fn tile(title: &str, value: String, detail: String, theme: Theme) -> gpui::Div {
    v_flex()
        .debug_selector({
            let title = title.to_lowercase();
            move || format!("context-tile-{title}")
        })
        .flex_1()
        .min_w_0()
        .flex_shrink_0()
        .px(px(16.))
        .py(px(12.))
        .gap(px(2.))
        // Hairlines between figures, not a box around the first.
        .when(title != "CONTEXT", |tile| {
            tile.border_l_1().border_color(theme.line)
        })
        .child(
            div()
                .text_size(px(12.5))
                .text_color(theme.muted)
                .child(sentence_case(title)),
        )
        .child(
            div()
                .text_size(px(26.))
                .line_height(px(34.))
                .flex_shrink_0()
                .font_weight(FontWeight::SEMIBOLD)
                .child(value),
        )
        .child(
            div()
                .debug_selector({
                    let title = title.to_lowercase();
                    move || format!("context-detail-{title}")
                })
                .text_size(px(12.))
                .line_height(px(18.))
                .flex_shrink_0()
                .text_color(theme.muted)
                .child(detail),
        )
}
impl Render for ContextView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let model = self.controller.read(cx).model();
        let stats = &model.stats;
        let usage = stats.context_usage.as_ref();
        let tokens = usage
            .and_then(|u| u.tokens)
            .map(count)
            .unwrap_or_else(|| "—".into());
        let total = stats
            .tokens
            .input
            .saturating_add(stats.tokens.cache_read)
            .saturating_add(stats.tokens.cache_write);
        let cache = if total > 0 {
            format!(
                "{:.0}%",
                stats.tokens.cache_read as f64 / total as f64 * 100.
            )
        } else {
            "—".into()
        };
        let compactions: Vec<_> = model
            .history
            .as_ref()
            .map(|h| h.path().filter(|e| e["type"] == "compaction").collect())
            .unwrap_or_default();
        let skills = model
            .commands
            .iter()
            .filter(|c| c.source == "skill")
            .map(|c| c.name.trim_start_matches("skill:"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut names: Vec<_> = model.tools.iter().map(|t| t.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        let visible = &self.usage[self.usage.len().saturating_sub(64)..];
        let max = visible
            .iter()
            .map(|u| u.input.saturating_add(u.output).saturating_add(u.cache))
            .max()
            .unwrap_or(1)
            .max(1) as f32;
        // Sparse samples stay narrow bars on a scale, not wide slabs.
        const CHART: f32 = 150.;
        let mut bars = h_flex()
            .debug_selector(|| "context-bars".into())
            .items_end()
            .h(px(CHART))
            .gap(px(if visible.len() > 24 { 4. } else { 22. }));
        for (i, u) in visible.iter().enumerate() {
            let segment = |n, color| div().w_full().h(px(n as f32 / max * CHART)).bg(color);
            bars = bars.child(
                v_flex()
                    .id(("usage-bar", i))
                    .debug_selector(move || format!("usage-bar-{i}"))
                    .flex_1()
                    .max_w(px(28.))
                    .min_w(px(4.))
                    .h_full()
                    .justify_end()
                    .rounded_t(px(2.))
                    .overflow_hidden()
                    .when(u.compaction_before, |v| {
                        v.border_l_2().border_color(theme.amber)
                    })
                    .child(segment(u.output, theme.orange))
                    .child(segment(u.input, theme.secondary))
                    .child(segment(u.cache, theme.steel)),
            );
        }
        let gridline = |fraction: f32| {
            h_flex()
                .absolute()
                .left_0()
                .right_0()
                .top(px(CHART * (1. - fraction)))
                .gap(px(8.))
                .child(
                    div()
                        .w(px(40.))
                        .text_right()
                        .text_size(px(11.))
                        .text_color(theme.muted)
                        .child(count((max * fraction) as u64)),
                )
                .child(div().flex_1().h(px(1.)).bg(theme.line))
        };
        let legend = |name: &'static str, color| {
            h_flex()
                .gap(px(6.))
                .child(div().size(px(9.)).rounded(px(2.)).bg(color))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.secondary)
                        .child(name),
                )
        };
        let chart =
            v_flex()
                .gap(px(10.))
                .child(
                    h_flex()
                        .child(label("Tokens per response", theme))
                        .child(div().flex_1())
                        .child(
                            h_flex()
                                .gap(px(16.))
                                .child(legend("Cache read", theme.steel))
                                .child(legend("Input", theme.secondary))
                                .child(legend("Output", theme.orange)),
                        ),
                )
                .child(
                    div()
                        .relative()
                        .h(px(CHART))
                        .child(gridline(0.))
                        .child(gridline(0.5))
                        .child(gridline(1.))
                        .child(
                            div()
                                .absolute()
                                .left(px(56.))
                                .right_0()
                                .top_0()
                                .bottom_0()
                                .child(bars),
                        ),
                )
                .when(!visible.is_empty() && visible.len() <= 24, |chart| {
                    chart.child(h_flex().pl(px(56.)).gap(px(22.)).children(
                        (1..=visible.len()).map(|n| {
                            div()
                                .flex_1()
                                .max_w(px(28.))
                                .text_center()
                                .text_size(px(11.))
                                .text_color(theme.muted)
                                .child(n.to_string())
                        }),
                    ))
                })
                .when(visible.is_empty(), |v| {
                    v.child(note("No assistant usage reported on this path yet.", theme))
                })
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.muted)
                        .child(format!(
                            "{} response{} · current branch{}",
                            self.usage.len(),
                            if self.usage.len() == 1 { "" } else { "s" },
                            if self.usage.len() > 64 {
                                " · latest 64"
                            } else {
                                ""
                            }
                        )),
                );
        let content=v_flex().id("context-scroll").track_scroll(&self.scroll).size_full().overflow_y_scroll().px(px(24.)).py(px(16.)).gap(px(20.))
            .child(h_flex().items_stretch().pb(px(4.)).border_b_1().border_color(theme.line)
                .child(tile("CONTEXT",tokens,usage.map(|u|format!("of {} · {}",count(u.context_window),u.percent.map(|p|format!("{p:.0}%")).unwrap_or_else(||"pending".into()))).unwrap_or_else(||"Not reported yet".into()),theme))
                .child(tile("COST",stats.cost.map(|c|format!("${c:.2}")).unwrap_or_else(||"—".into()),"whole session".into(),theme))
                .child(tile("CACHE READ",cache,"of all input tokens".into(),theme))
                .child(tile("COMPACTIONS",model.history.as_ref().map(|_|compactions.len().to_string()).unwrap_or_else(||"—".into()),"on current path".into(),theme)))
            .child(chart)
            .child(v_flex().gap(px(6.)).pt(px(4.)).border_t_1().border_color(theme.line)
                .child(h_flex().id("context-sources").debug_selector(||"context-sources".into()).h(px(36.)).gap(px(8.)).cursor_pointer()
                    .child(icon(if self.sources_open {"chevron_down"} else {"chevron_right"}, theme.muted).size(px(12.)))
                    .child(div().text_size(px(13.5)).child("Reported context sources"))
                    .child(div().flex_1())
                    .child(div().text_size(px(12.)).text_color(theme.muted).child("Skills, observed tools and history"))
                    .on_click(cx.listener(|this,_,_,cx|{this.sources_open = !this.sources_open;cx.notify();})))
                .child(note("The exact request payload is not exposed by Pi. These are reported sources, not a prompt manifest.",theme).pl(px(20.)))
                .when(self.sources_open, |v| v.child(v_flex().pl(px(20.)).child(pair("System prompt","Not exposed by RPC".into(),theme)).child(pair("Context files","Not exposed by RPC".into(),theme))
                .child(pair("Skills available",if skills.is_empty(){"None reported".into()}else{skills},theme))
                .child(pair("Tools observed",if names.is_empty(){"None yet".into()}else{names.join(", ")},theme))
                .child(pair("History",model.history.as_ref().map(|h|format!("{} entries on current path",h.active.len())).unwrap_or_else(||"Loading…".into()),theme)))))
            .when_some(compactions.last(),|v,e|v.child(v_flex().p(px(14.)).gap(px(8.)).bg(theme.panel).border_1().border_color(theme.line).rounded(px(7.)).child(h_flex().gap(px(8.)).child(div().text_color(theme.amber).child("◆")).child(div().flex_1().child(format!("Compacted at {}",history::time(e)))).child(button("context-summary",if self.show_summary{"Hide summary"}else{"View summary"},theme).debug_selector(||"context-summary".into()).on_click(cx.listener(|this,_,_,cx|{this.show_summary = !this.show_summary;cx.notify();}))))
                .child(note(format!("{} tokens before compaction. Original entries remain in the session file.",e["tokensBefore"].as_u64().map(count).unwrap_or_else(||"Unreported".into())),theme))
                .when(self.show_summary,|v|v.child(self.summary.clone()))));
        div()
            .debug_selector(|| "context-view".into())
            .relative()
            .size_full()
            .child(content)
            .custom_scrollbars(
                scrollbar("context-scrollbar", &self.scroll, None),
                window,
                cx,
            )
    }
}
