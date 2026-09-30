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
        v_flex().p(px(20.)).gap(px(8.)).child(heading("Context",percent,theme)).child(section("AUTO-COMPACTION","",theme))
            .child(primary_button("context-auto",if auto{"Enabled · turn off"}else{"Disabled · turn on"},toggle,theme).debug_selector(||"context-auto".into()).on_click(cx.listener(move|this,_,_,cx|{if toggle{this.controller.update(cx,|c,cx|c.command(Command::SetAutoCompaction {enabled:!auto},cx));}})))
            .child(note("This changes Pi’s saved auto-compaction setting.",theme)).child(pair("Reserve for reply","Not exposed by RPC".into(),theme)).child(pair("Keep recent","Not exposed by RPC".into(),theme))
            .child(divider(theme)).child(section("COMPACT NOW","",theme)).child(input_box(self.instructions.clone(),theme))
            .child(primary_button("context-compact","Compact",enabled,theme).debug_selector(||"context-compact".into()).on_click(cx.listener(move|this,_,_,cx|{if enabled{let text=this.instructions.read(cx).content().trim().to_owned();this.controller.update(cx,|c,cx|c.command(Command::Compact {custom_instructions:(!text.is_empty()).then_some(text)},cx));}})))
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
fn tile(title: &str, value: String, detail: String, theme: Theme) -> gpui::Div {
    v_flex()
        .debug_selector({
            let title = title.to_lowercase();
            move || format!("context-tile-{title}")
        })
        .flex_1()
        .min_w_0()
        .min_h(px(92.))
        .flex_shrink_0()
        .p(px(12.))
        .gap(px(3.))
        .bg(theme.panel)
        .border_1()
        .border_color(theme.line)
        .rounded(px(7.))
        .child(label(title.to_owned(), theme))
        .child(
            div()
                .text_size(px(22.))
                .line_height(px(28.))
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
                .text_size(px(10.))
                .line_height(px(16.))
                .flex_shrink_0()
                .text_color(theme.faint)
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
        let mut bars = h_flex()
            .items_end()
            .h(px(170.))
            .w_full()
            .gap(px(8.))
            .border_b_1()
            .border_color(theme.line);
        for (i, u) in visible.iter().enumerate() {
            let segment = |n, color| div().w_full().h(px(n as f32 / max * 155.)).bg(color);
            bars = bars.child(
                v_flex()
                    .id(("usage-bar", i))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .justify_end()
                    .gap(px(0.))
                    .when(u.compaction_before, |v| {
                        v.border_l_1().border_color(theme.amber)
                    })
                    .child(segment(u.output, theme.amber))
                    .child(segment(u.input, theme.accent))
                    .child(segment(u.cache, theme.steel)),
            );
        }
        let content=v_flex().id("context-scroll").track_scroll(&self.scroll).size_full().overflow_y_scroll().px(px(24.)).py(px(16.)).gap(px(20.))
            .child(h_flex().gap(px(10.)).items_stretch()
                .child(tile("CONTEXT",tokens,usage.map(|u|format!("of {} · {}",count(u.context_window),u.percent.map(|p|format!("{p:.0}%")).unwrap_or_else(||"pending".into()))).unwrap_or_else(||"Not reported yet".into()),theme))
                .child(tile("COST",stats.cost.map(|c|format!("${c:.2}")).unwrap_or_else(||"—".into()),"whole session".into(),theme))
                .child(tile("CACHE READ",cache,"of all input tokens".into(),theme))
                .child(tile("COMPACTIONS",model.history.as_ref().map(|_|compactions.len().to_string()).unwrap_or_else(||"—".into()),"on current path".into(),theme)))
            .child(v_flex().gap(px(12.)).child(section("TOKENS PER RESPONSE","cache read  ·  input + cache write  ·  output",theme)).child(bars)
                .when(visible.is_empty(),|v|v.child(note("No assistant usage reported on this path yet.",theme)))
                .child(section(&format!("{} RESPONSES",self.usage.len()),&format!("{} max · latest 64 · amber marker = compaction",count(max as u64)),theme)))
            .child(v_flex().gap(px(8.)).child(section("IN CONTEXT / REPORTED SOURCES","",theme))
                .child(pair("System prompt","Not exposed by RPC".into(),theme)).child(pair("Context files","Not exposed by RPC".into(),theme))
                .child(pair("Skills available",if skills.is_empty(){"None reported".into()}else{skills},theme))
                .child(pair("Tools observed",if names.is_empty(){"None yet".into()}else{names.join(", ")},theme))
                .child(pair("History",model.history.as_ref().map(|h|format!("{} entries on current path",h.active.len())).unwrap_or_else(||"Loading…".into()),theme))
                .child(note("Available skills and observed tools are not an exact prompt manifest. Pi assembles request-specific context; this RPC does not expose that payload.",theme)))
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
