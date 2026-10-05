//! Follow mode (design/visual-workflow E): a stage beside the thread showing what
//! the agent just touched — an edit's reported diff, a command's output, the file
//! it read. Selecting a step only inspects it; nothing here re-runs or changes work.
use super::diff::{DiffSection, DiffView};
use super::panels::DocumentView;
use super::session::{Changes, SessionController, SessionEvent};
use super::*;
use gpui::{BoxShadow, Hsla, Role, Subscription, Toggled};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Step {
    pub id: String,
    pub kind: StepKind,
    pub finished: bool,
    pub failed: bool,
}

/// Tool calls by turn, in the order they were made. Turns start at user messages.
fn turns(model: &Session) -> Vec<Vec<Step>> {
    let kinds = super::transcript::call_kinds(model);
    let mut turns: Vec<Vec<Step>> = vec![vec![]];
    for message in &model.messages {
        if message["role"] == "user" {
            if turns.last().is_some_and(|turn| !turn.is_empty()) {
                turns.push(vec![]);
            }
            continue;
        }
        if message["role"] != "assistant" {
            continue;
        }
        for block in message["content"].as_array().into_iter().flatten() {
            if block["type"] != "toolCall" {
                continue;
            }
            let Some(id) = block["id"].as_str() else {
                continue;
            };
            let tool = model.tools.iter().find(|tool| tool.id == id);
            turns.last_mut().unwrap().push(Step {
                id: id.to_owned(),
                kind: kinds.get(id).copied().unwrap_or(StepKind::Other),
                finished: tool.is_some_and(|tool| tool.finished),
                failed: tool.is_some_and(|tool| tool.is_error),
            });
        }
    }
    turns.retain(|turn| !turn.is_empty());
    turns
}

pub struct FollowView {
    controller: Entity<SessionController>,
    pub open: bool,
    /// A step the person picked; `None` follows the latest call.
    pinned: Option<String>,
    /// What the stage documents currently hold, so streaming output only resets
    /// them when it changed.
    shown: Option<(String, usize, usize, bool)>,
    diff: Entity<DiffView>,
    document: Entity<DocumentView>,
    _subscription: Subscription,
}

impl FollowView {
    pub fn new(controller: Entity<SessionController>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&controller, |this, _, event, cx| match event {
            SessionEvent::Content(_) => this.sync(cx),
            SessionEvent::Changed(changes) if changes.intersects(Changes::RUN) => this.sync(cx),
            _ => {}
        });
        let diff = cx.new(DiffView::new);
        diff.update(cx, |diff, cx| diff.set_wide(false, cx));
        Self {
            controller,
            open: false,
            pinned: None,
            shown: None,
            diff,
            document: cx.new(|cx| DocumentView::new(cx).review().wrapped()),
            _subscription: subscription,
        }
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        self.open = !self.open;
        self.sync(cx);
    }

    pub fn following(&self) -> bool {
        self.pinned.is_none()
    }

    /// Shows one step and stops following until the person resumes.
    pub fn pin(&mut self, id: String, cx: &mut Context<Self>) {
        self.pinned = Some(id);
        self.sync(cx);
    }

    pub fn follow_latest(&mut self, cx: &mut Context<Self>) {
        self.pinned = None;
        self.sync(cx);
    }

    /// The step on stage: the pinned one while it exists, else the latest call.
    pub(super) fn shown_id(&self, cx: &App) -> Option<String> {
        let turns = turns(self.controller.read(cx).model());
        self.pinned
            .clone()
            .filter(|id| turns.iter().flatten().any(|step| &step.id == id))
            .or_else(|| turns.last()?.last().map(|step| step.id.clone()))
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let turns = turns(self.controller.read(cx).model());
        let Some(shown) = self.shown_id(cx) else {
            return;
        };
        let Some(turn) = turns
            .iter()
            .find(|turn| turn.iter().any(|step| step.id == shown))
        else {
            return;
        };
        let index = turn.iter().position(|step| step.id == shown).unwrap_or(0);
        let next = (index as isize + delta).clamp(0, turn.len() as isize - 1) as usize;
        self.pin(turn[next].id.clone(), cx);
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        if !self.open {
            cx.notify();
            return;
        }
        let shown = self.shown_id(cx);
        let model = self.controller.read(cx).model();
        let Some(tool) = shown.and_then(|id| model.tools.iter().find(|tool| tool.id == id)) else {
            self.shown = None;
            cx.notify();
            return;
        };
        let key = (
            tool.id.clone(),
            tool.output.len(),
            tool.diff.as_ref().map_or(0, String::len),
            tool.finished,
        );
        if self.shown.as_ref() == Some(&key) {
            cx.notify();
            return;
        }
        let tool = tool.clone();
        self.shown = Some(key);
        match tool.diff.as_deref().filter(|_| tool.name != "bash") {
            Some(patch) => {
                let preview =
                    super::diff_preview::reported(patch).unwrap_or_else(|| patch.to_owned());
                let sections: Vec<_> = DiffSection::reported(tool.target(), patch)
                    .into_iter()
                    .collect();
                self.diff
                    .update(cx, |diff, cx| diff.set(preview, &sections, cx));
            }
            None => {
                let (text, language) = match tool.name.as_str() {
                    "write" => (
                        text(&tool.args, "content"),
                        super::diff_preview::language(&tool.target()),
                    ),
                    "read" => (
                        tool.output.clone(),
                        super::diff_preview::language(&tool.target()),
                    ),
                    _ => (tool.output.clone(), "text"),
                };
                let language = if language == "Not identified" {
                    "text"
                } else {
                    language
                };
                self.document
                    .update(cx, |document, cx| document.set(text, Some(language), cx));
            }
        }
        cx.notify();
    }
}

/// What the stage calls a step: "Just edited", "Running", "Read".
fn verb(tool: &Tool, kind: StepKind, latest: bool) -> &'static str {
    if tool.is_error {
        return "Failed";
    }
    match (kind, tool.finished) {
        (StepKind::Change, false) => "Editing",
        (StepKind::Change, true) if latest => "Just edited",
        (StepKind::Change, true) => "Edited",
        (StepKind::Check, false) => "Checking",
        (StepKind::Check, true) => "Checked",
        (StepKind::Run, false) => "Running",
        (StepKind::Run, true) => "Ran",
        (StepKind::Explore, false) => "Reading",
        (StepKind::Explore, true) => "Read",
        (StepKind::Search, false) => "Searching",
        (StepKind::Search, true) => "Searched",
        (_, false) => "Calling",
        (_, true) => "Called",
    }
}

impl Render for FollowView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let model = self.controller.read(cx).model();
        let turns = turns(model);
        let shown = self.shown_id(cx);
        let tool = shown
            .as_ref()
            .and_then(|id| model.tools.iter().find(|tool| &tool.id == id))
            .cloned();
        let turn = shown
            .as_ref()
            .and_then(|id| turns.iter().find(|turn| turn.iter().any(|s| &s.id == id)))
            .cloned()
            .unwrap_or_default();
        let index = shown
            .as_ref()
            .and_then(|id| turn.iter().position(|step| &step.id == id));
        let latest = turns.last().and_then(|turn| turn.last()).map(|s| &s.id) == shown.as_ref();
        // While the shown turn runs, the stages it has not reached follow its
        // steps as dashed ticks. A new turn without calls is not on the scrubber.
        let ahead: Vec<Stage> = if model.run == pi_core::session::RunState::Running
            && turns.last().is_some_and(|last| last == &turn)
        {
            let ahead = super::transcript::stages_ahead(model);
            if ahead.iter().any(|(_, state)| *state == NodeState::Live) {
                vec![]
            } else {
                ahead.into_iter().map(|(stage, _)| stage).collect()
            }
        } else {
            vec![]
        };
        let following = self.following();
        let hue = |kind: StepKind, failed: bool| if failed { theme.coral } else { kind.hue(theme) };
        let kind_of = |tool: &Tool| {
            turns
                .iter()
                .flatten()
                .find(|step| step.id == tool.id)
                .map_or_else(
                    || StepKind::of_call(&tool.name, &tool.args, &[]),
                    |step| step.kind,
                )
        };

        let header = h_flex()
            .h(px(48.))
            .flex_shrink_0()
            .px(px(18.))
            .gap(px(10.))
            .border_b_1()
            .border_color(theme.line)
            .bg(theme.canvas)
            .when_some(tool.as_ref(), |header, tool| {
                let kind = kind_of(tool);
                header
                    .child(
                        div()
                            .debug_selector(|| "follow-verb".into())
                            .flex_shrink_0()
                            .font_family(MONO)
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(hue(kind, tool.is_error))
                            .child(verb(tool, kind, latest).to_uppercase()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(MONO)
                            .text_size(px(13.))
                            .child(tool.target().lines().next().unwrap_or_default().to_owned()),
                    )
            })
            .when(tool.is_none(), |header| {
                header.child(div().flex_1().text_color(theme.muted).child("Follow"))
            })
            .child(
                h_flex()
                    .id("follow-latest")
                    .debug_selector(|| "follow-latest".into())
                    .role(Role::Switch)
                    .aria_label("Follow the agent")
                    .aria_toggled(if following {
                        Toggled::True
                    } else {
                        Toggled::False
                    })
                    .flex_shrink_0()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .text_color(theme.secondary)
                    .cursor_pointer()
                    .child("Follow the agent")
                    .child(
                        div()
                            .relative()
                            .w(px(30.))
                            .h(px(18.))
                            .rounded_full()
                            .bg(if following {
                                theme.orange
                            } else {
                                theme.line_strong
                            })
                            .child(
                                div()
                                    .absolute()
                                    .top(px(2.))
                                    .when(following, |knob| knob.right(px(2.)))
                                    .when(!following, |knob| knob.left(px(2.)))
                                    .size(px(14.))
                                    .rounded_full()
                                    .bg(gpui::white())
                                    .shadow(lift(theme)),
                            ),
                    )
                    .tooltip(ui::Tooltip::text(
                        "Show each call as the agent makes it. Picking a step pauses following.",
                    ))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.following() {
                            if let Some(id) = this.shown_id(cx) {
                                this.pin(id, cx);
                            }
                        } else {
                            this.follow_latest(cx);
                        }
                    })),
            )
            .when_some(
                tool.as_ref()
                    .filter(|tool| matches!(tool.name.as_str(), "edit" | "write") && tool.finished),
                |header, tool| {
                    let path = tool.target();
                    header.child(
                        work_button("follow-review", "Review", theme)
                            .debug_selector(|| "follow-review".into())
                            .tooltip(ui::Tooltip::text("Open this file in Changes"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let path = path.clone();
                                this.controller.update(cx, |_, cx| {
                                    cx.emit(SessionEvent::ReviewFile(path, None))
                                });
                            })),
                    )
                },
            )
            .child(
                icon_button("follow-close", "close", "Close follow mode", theme)
                    .debug_selector(|| "follow-close".into())
                    .on_click(cx.listener(|this, _, _, cx| this.toggle(cx))),
            );

        let body = match &tool {
            None => v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .p(px(32.))
                .child(icon("eye", theme.faint).size(px(22.)))
                .child(
                    div()
                        .font_family(SERIF)
                        .italic()
                        .text_size(px(20.))
                        .child("Nothing to follow yet."),
                )
                .child(
                    div()
                        .max_w(px(360.))
                        .text_center()
                        .text_size(px(13.))
                        .text_color(theme.muted)
                        .child("Each file the agent reads or edits and each command it runs appears here as it happens."),
                )
                .into_any_element(),
            Some(tool) => {
                let reported_diff = tool.diff.is_some() && tool.name != "bash";
                let status = if tool.is_error {
                    "failed".to_owned()
                } else if !tool.finished {
                    "waiting for a result".to_owned()
                } else if reported_diff {
                    "observed edit · already on disk".to_owned()
                } else if tool.name == "bash" {
                    "finished".to_owned()
                } else {
                    "tool result".to_owned()
                };
                let name = std::path::Path::new(&tool.target())
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| tool.name.clone());
                let empty = !reported_diff
                    && tool.output.trim().is_empty()
                    && !(tool.name == "write" && !text(&tool.args, "content").is_empty());
                div()
                    .id("follow-body")
                    .debug_selector(|| "follow-body".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(18.))
                    .child(
                        v_flex()
                            .debug_selector(|| "follow-card".into())
                            .w_full()
                            .overflow_hidden()
                            .rounded(px(10.))
                            .border_1()
                            .border_color(theme.line)
                            .bg(theme.composer)
                            .shadow(vec![
                                BoxShadow::new(
                                    px(0.),
                                    px(8.),
                                    gpui::black().opacity(if theme.light { 0.06 } else { 0.3 }),
                                )
                                .blur_radius(px(30.)),
                            ])
                            .child(
                                h_flex()
                                    .h(px(34.))
                                    .px(px(14.))
                                    .gap(px(8.))
                                    .border_b_1()
                                    .border_color(theme.line)
                                    .font_family(MONO)
                                    .text_size(px(11.5))
                                    .text_color(theme.muted)
                                    .child(
                                        icon(
                                            kind_of(tool).glyph(),
                                            hue(kind_of(tool), tool.is_error),
                                        )
                                        .size(px(13.)),
                                    )
                                    .child(div().text_color(theme.secondary).child(name))
                                    .child(div().flex_1())
                                    .child(status),
                            )
                            .child(div().w_full().p(px(10.)).child(if reported_diff {
                                self.diff.clone().into_any_element()
                            } else if empty {
                                div()
                                    .p(px(8.))
                                    .text_size(px(13.))
                                    .text_color(theme.muted)
                                    .child(if tool.finished {
                                        "No output was reported."
                                    } else {
                                        "Waiting for output…"
                                    })
                                    .into_any_element()
                            } else {
                                self.document.clone().into_any_element()
                            })),
                    )
                    .into_any_element()
            }
        };

        let scrubber = h_flex()
            .debug_selector(|| "follow-steps".into())
            .h(px(52.))
            .flex_shrink_0()
            .px(px(18.))
            .gap(px(14.))
            .border_t_1()
            .border_color(theme.line)
            .bg(theme.canvas)
            .child(
                div()
                    .flex_shrink_0()
                    .font_family(MONO)
                    .text_size(px(11.))
                    .text_color(theme.muted)
                    .child(match index {
                        Some(index) => format!("STEP {} / {}", index + 1, turn.len()),
                        None => "NO STEPS".into(),
                    }),
            )
            .child({
                let first = turn.first().map(|s| hue(s.kind, s.failed));
                let current = index
                    .and_then(|i| turn.get(i))
                    .map(|s| hue(s.kind, s.failed));
                let progress = match (index, turn.len() + ahead.len()) {
                    (Some(index), len) if len > 1 => index as f32 / (len - 1) as f32,
                    _ => 0.,
                };
                h_flex()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h(px(24.))
                    .justify_between()
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(px(11.))
                            .h(px(2.))
                            .rounded(px(1.))
                            .bg(theme.line),
                    )
                    .when_some(first.zip(current), |track, (first, current)| {
                        track.child(
                            div()
                                .absolute()
                                .left_0()
                                .top(px(11.))
                                .w(relative(progress))
                                .h(px(2.))
                                .rounded(px(1.))
                                .bg(gpui::linear_gradient(
                                    90.,
                                    gpui::linear_color_stop(first, 0.),
                                    gpui::linear_color_stop(current, 1.),
                                )),
                        )
                    })
                    .children(turn.iter().enumerate().map(|(i, step)| {
                        let tint: Hsla = hue(step.kind, step.failed);
                        let current = Some(i) == index;
                        let id = step.id.clone();
                        div()
                            .id(("follow-step", i))
                            .debug_selector(move || format!("follow-step-{i}"))
                            .role(Role::Button)
                            .aria_label(format!("Show step {}", i + 1))
                            .aria_selected(current)
                            .size(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .child(
                                div()
                                    .size(px(if current { 18. } else { 12. }))
                                    // Steps are dots; the one on stage is a soft square, as on the rail.
                                    .rounded(px(6.))
                                    .border_2()
                                    .border_color(if step.finished || step.failed {
                                        tint
                                    } else {
                                        theme.line_strong
                                    })
                                    .when(!step.finished && !step.failed && !current, |dot| {
                                        dot.border_dashed()
                                    })
                                    .bg(if current { tint } else { theme.canvas })
                                    .when(current, |dot| dot.shadow(halo(tint, 0.5))),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| this.pin(id.clone(), cx)))
                    }))
                    .children(ahead.iter().map(|stage| {
                        let stage = *stage;
                        div()
                            .id(SharedString::from(format!("follow-ahead-{}", stage.slug())))
                            .debug_selector(move || format!("follow-ahead-{}", stage.slug()))
                            .size(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .tooltip(ui::Tooltip::text(format!("Ahead: {}", stage.title())))
                            .child(
                                div()
                                    .size(px(12.))
                                    .rounded(px(6.))
                                    .border_2()
                                    .border_dashed()
                                    .border_color(theme.line_strong)
                                    .bg(theme.canvas),
                            )
                    }))
            })
            .child(
                work_button("follow-previous", "←", theme)
                    .debug_selector(|| "follow-previous".into())
                    .aria_label("Previous step")
                    .on_click(cx.listener(|this, _, _, cx| this.step(-1, cx))),
            )
            .child(
                work_button("follow-next", "→", theme)
                    .debug_selector(|| "follow-next".into())
                    .aria_label("Next step")
                    .on_click(cx.listener(|this, _, _, cx| this.step(1, cx))),
            );

        v_flex()
            .id("follow-stage")
            .debug_selector(|| "follow-stage".into())
            .size_full()
            .min_w_0()
            .bg(theme.panel)
            .child(header)
            .child(body)
            .child(scrubber)
    }
}

#[cfg(test)]
mod tests {
    use super::{StepKind, turns};
    use pi_core::session::Session;
    use serde_json::json;

    #[test]
    fn steps_follow_call_order_and_turns_start_at_user_messages() {
        let mut model = Session::new("/demo".into());
        model.messages = vec![
            json!({"role":"user","content":"First"}),
            json!({"role":"assistant","content":[{"type":"toolCall","id":"a","name":"read"},{"type":"toolCall","id":"b","name":"edit"}]}),
            json!({"role":"user","content":"Second, no tools yet"}),
            json!({"role":"assistant","content":[{"type":"text","text":"Thinking about it"}]}),
        ];
        let turns = turns(&model);
        assert_eq!(
            turns.len(),
            1,
            "a turn without calls keeps the previous one on stage"
        );
        assert_eq!(turns[0][1].kind, StepKind::Change);
        assert!(
            !turns[0][0].finished,
            "a call without a result is not finished"
        );
    }
}
