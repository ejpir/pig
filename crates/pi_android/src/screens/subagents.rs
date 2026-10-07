//! Work Pi handed to subagents (design/android/subagents): a card in the
//! turn, one row per subagent with what it is doing, its time and cost; and a
//! subagent's own screen, opened from its row. Only Pi talks to a subagent, so
//! that screen is read-only, with Stop.

use super::{scroll_area, thread::stage_row};
use crate::{
    app::{PhoneApp, Pick, Route},
    model::{SessionId, StageStatus, Turn, duration_label},
    projection,
    theme::{Theme, theme},
    ui::{self, icon},
};
use gpui::{AnyElement, Context, Div, ElementId, FontWeight, Hsla, Window, div, prelude::*, px};
use pi_core::subagent::{Handoff, Mode, Status, Subagent};
use std::time::Duration;

/// The look of a well-known agent: its icon and hue. Others get Pi's.
pub(crate) fn agent_look(agent: &str, colors: &Theme) -> (&'static str, Hsla) {
    match agent {
        "scout" => ("search", colors.read),
        "planner" => ("plan", colors.accent),
        "worker" => ("pencil", colors.edit),
        "reviewer" => ("eye", colors.check),
        _ => ("spark", colors.accent),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |now| now.as_millis() as u64)
}

fn cost_label(cost: f64) -> String {
    if cost < 0.005 {
        "<$0.01".into()
    } else {
        format!("${cost:.2}")
    }
}

/// "3 scouts in parallel", "A chain of 2", "scout".
fn shape(handoff: &Handoff) -> String {
    let first = handoff.subagents.first().map_or("", |s| s.agent.as_str());
    let same = handoff.subagents.iter().all(|s| s.agent == first);
    let count = handoff.subagents.len();
    match handoff.mode {
        Mode::Single => first.to_owned(),
        Mode::Parallel if same => format!("{count} {first}s side by side"),
        Mode::Parallel => format!("{count} subagents side by side"),
        Mode::Chain => format!(
            "A chain of {count} · {}",
            handoff
                .subagents
                .iter()
                .map(|s| s.agent.as_str())
                .collect::<Vec<_>>()
                .join(", then ")
        ),
    }
}

/// How many subagents a card shows before Show all.
const SHOWN: usize = 5;

/// The subagents a card shows: every one, or the first few with those at work
/// first, so a hundred side by side still show what is happening. A chain
/// keeps its steps in order.
fn shown(handoff: &Handoff, all: bool) -> Vec<usize> {
    let count = handoff.subagents.len();
    if all || count <= SHOWN {
        return (0..count).collect();
    }
    if handoff.mode == Mode::Chain {
        let current = handoff
            .subagents
            .iter()
            .position(|subagent| !subagent.status.finished())
            .unwrap_or(count);
        let start = current.saturating_sub(SHOWN - 1).min(count - SHOWN);
        return (start..start + SHOWN).collect();
    }
    let mut picked: Vec<usize> = (0..count)
        .filter(|&index| handoff.subagents[index].status == Status::Running)
        .chain((0..count).filter(|&index| handoff.subagents[index].status != Status::Running))
        .take(SHOWN)
        .collect();
    picked.sort_unstable();
    picked
}

/// "8 running · 30 done · 62 waiting".
fn tally(handoff: &Handoff) -> String {
    let count = |wanted: &[Status]| {
        handoff
            .subagents
            .iter()
            .filter(|subagent| wanted.contains(&subagent.status))
            .count()
    };
    [
        (count(&[Status::Running]), "running"),
        (count(&[Status::Done]), "done"),
        (count(&[Status::Failed]), "failed"),
        (count(&[Status::Stopped]), "stopped"),
        (count(&[Status::Waiting]), "waiting"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, said)| format!("{count} {said}"))
    .collect::<Vec<_>>()
    .join(" · ")
}

fn first_line(text: &str) -> String {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_owned()
}

/// What a subagent's row says under its task.
fn status_line(subagent: &Subagent, mixed: bool) -> String {
    let said = match (subagent.status, &subagent.now) {
        (Status::Waiting, _) => "Waiting its turn".to_owned(),
        (Status::Stopped, None) => "Stopped".to_owned(),
        (Status::Failed, None) => "Failed".to_owned(),
        (_, Some(now)) => first_line(now),
        (Status::Running, None) => "Starting".to_owned(),
        (Status::Done, None) => "Done".to_owned(),
    };
    if mixed {
        format!("{} · {said}", subagent.agent)
    } else {
        said
    }
}

/// An agent's tile: tinted when done, ringed while it works, dashed while it waits.
fn tile(subagent: &Subagent, colors: &Theme) -> Div {
    let (glyph, hue) = agent_look(&subagent.agent, colors);
    let base = div()
        .size(px(32.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(10.));
    match subagent.status {
        Status::Waiting => base
            .border(px(1.5))
            .border_dashed()
            .border_color(colors.line_strong)
            .child(icon(glyph, 16., colors.faint)),
        Status::Running => base
            .bg(colors.composer)
            .border(px(1.5))
            .border_color(hue.opacity(0.6))
            .shadow(ui::ring(hue, 4., 0.14))
            .child(icon(glyph, 16., hue)),
        Status::Failed => {
            base.bg(colors.tint(colors.coral))
                .child(icon("alert", 16., colors.coral))
        }
        Status::Stopped => {
            base.bg(colors.tint(colors.muted))
                .child(icon("stop", 16., colors.muted))
        }
        Status::Done => base.bg(colors.tint(hue)).child(icon(glyph, 16., hue)),
    }
}

/// A note in the card's width, such as a command a restart cut off.
fn note(glyph: &str, hue: Hsla, text: String, colors: &Theme) -> Div {
    div()
        .mx(px(16.))
        .mb(px(12.))
        .px(px(12.))
        .py(px(10.))
        .flex()
        .items_start()
        .gap(px(10.))
        .rounded(px(12.))
        .bg(colors.canvas.blend(hue.opacity(0.10)))
        .child(icon(glyph, 16., hue).mt(px(1.)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(12.5))
                .line_height(px(18.))
                .text_color(colors.secondary)
                .child(text),
        )
}

fn interrupted_note(subagent: &Subagent) -> Option<String> {
    let commands = subagent.interrupted.join(", ");
    (!commands.is_empty()).then(|| {
        format!(
            "A restart cut off {commands}. It wasn't repeated by itself; the {} was told.",
            subagent.agent
        )
    })
}

/// A working session's subagents on Home: the first few as small overlapping
/// tiles, live ones ringed. The line beside them says how many are at work.
pub(crate) fn crew(handoff: &Handoff, colors: &Theme) -> Div {
    div().flex().flex_none().items_center().children(
        shown(handoff, false)
            .into_iter()
            .enumerate()
            .map(|(n, index)| {
                let subagent = &handoff.subagents[index];
                let (glyph, hue) = agent_look(&subagent.agent, colors);
                div()
                    .size(px(22.))
                    .when(n > 0, |tile| tile.ml(px(-6.)))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(7.))
                    .border_2()
                    .border_color(colors.canvas)
                    .bg(colors.canvas.blend(hue.opacity(0.22)))
                    .when(subagent.status == Status::Running, |tile| {
                        tile.shadow(ui::ring(hue, 1.5, 1.))
                    })
                    .when(subagent.status == Status::Waiting, |tile| {
                        tile.opacity(0.45)
                    })
                    .child(icon(glyph, 12., hue))
            }),
    )
}

/// The latest hand-off of a turn whose subagents are still at work.
pub(crate) fn waiting_on(turn: &Turn) -> Option<&Handoff> {
    turn.handoffs.iter().rev().find(|handoff| {
        handoff
            .subagents
            .iter()
            .any(|subagent| !subagent.status.finished())
    })
}

/// Subagents still at work anywhere in a session: they carry on after Pi's turn ends.
pub(crate) fn at_work(turns: &[Turn]) -> Option<&Handoff> {
    turns.iter().rev().find_map(waiting_on)
}

impl PhoneApp {
    /// Hand-off `n` of a turn: who Pi handed work to, and how each is doing.
    pub(crate) fn handoff_card(
        &self,
        id: SessionId,
        turn_index: usize,
        turn: &Turn,
        n: usize,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Option<Div> {
        let handoff = turn.handoffs.get(n)?;
        let now = now_ms();
        let first = handoff.subagents.first().map(|s| s.agent.as_str());
        let mixed = handoff
            .subagents
            .iter()
            .any(|s| Some(s.agent.as_str()) != first);
        let cost = handoff.cost();
        let key = (id, turn_index, n);
        let folded = self.folded_handoffs.contains(&key);
        let all = self.all_subagents.contains(&key);
        let probe = format!("handoff-{turn_index}-{n}");
        let header = div()
            .id(ElementId::Name(probe.clone().into()))
            .debug_selector({
                let probe = probe.clone();
                move || probe
            })
            .relative()
            .child(crate::testing::probe(probe))
            .min_h(px(56.))
            .px(px(16.))
            .py(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .when(!folded, |header| {
                header.border_b_1().border_color(colors.line)
            })
            .active(|style| style.bg(colors.selected).rounded(px(15.)))
            .child(icon("fork", 16., colors.muted))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Handed off"),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.5))
                            .text_color(colors.muted)
                            .child(tally(handoff)),
                    ),
            )
            .when(handoff.resumed, |header| {
                header.child(ui::badge("Resumed", colors.accent, colors.accent, colors))
            })
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.5))
                    .text_color(colors.muted)
                    .child(if cost > 0. {
                        cost_label(cost)
                    } else {
                        String::new()
                    }),
            )
            .child(icon(
                if folded { "chev_d" } else { "chev_u" },
                16.,
                colors.faint,
            ))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.folded_handoffs.remove(&key) {
                    this.folded_handoffs.insert(key);
                }
                cx.notify();
            }));
        let visible = if folded {
            Vec::new()
        } else {
            shown(handoff, all)
        };
        let hidden = handoff.subagents.len() - shown(handoff, false).len();
        let more = (!folded && hidden > 0).then(|| {
            let probe = format!("handoff-all-{turn_index}-{n}");
            div()
                .id(ElementId::Name(probe.clone().into()))
                .debug_selector({
                    let probe = probe.clone();
                    move || probe
                })
                .relative()
                .child(crate::testing::probe(probe))
                .min_h(px(48.))
                .px(px(16.))
                .flex()
                .items_center()
                .gap(px(12.))
                .border_t_1()
                .border_color(colors.line)
                .text_size(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors.accent)
                .active(|style| style.bg(colors.selected).rounded(px(15.)))
                .child(icon(
                    if all { "chev_u" } else { "chev_d" },
                    16.,
                    colors.accent,
                ))
                .child(if all {
                    "Show fewer".to_owned()
                } else {
                    format!("Show all {}", handoff.subagents.len())
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.all_subagents.remove(&key) {
                        this.all_subagents.insert(key);
                    }
                    cx.notify();
                }))
        });
        let rows =
            visible
                .into_iter()
                .enumerate()
                .flat_map(|(position, index)| {
                    let subagent = &handoff.subagents[index];
                    let pick = Pick {
                        turn: turn_index,
                        handoff: n,
                        index,
                    };
                    let opens = subagent.conversation_id.is_some();
                    let waiting = subagent.status == Status::Waiting;
                    let time = subagent
                        .elapsed_ms(now)
                        .map(|ms| duration_label(Duration::from_millis(ms)));
                    let row = ui::row(
                        ElementId::Name(format!("subagent-{turn_index}-{n}-{index}").into()),
                        position == 0,
                        colors,
                    )
                    .debug_selector(move || format!("subagent-{turn_index}-{n}-{index}"))
                    .min_h(px(64.))
                    .gap(px(12.))
                    .when(handoff.mode == Mode::Chain, |row| {
                        row.child(
                            div()
                                .w(px(14.))
                                .flex_none()
                                .text_size(px(12.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors.muted)
                                .child(format!("{}", index + 1)),
                        )
                    })
                    .child(tile(subagent, colors))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(if waiting { colors.faint } else { colors.text })
                                    .child(first_line(&subagent.task)),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.5))
                                    .text_color(match subagent.status {
                                        Status::Failed => colors.coral,
                                        _ if waiting => colors.faint,
                                        _ => colors.muted,
                                    })
                                    .child(status_line(subagent, mixed)),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .flex_col()
                            .items_end()
                            .gap(px(4.))
                            .text_size(px(12.5))
                            .text_color(colors.muted)
                            .children(time)
                            .children(subagent.cost.filter(|cost| *cost > 0.).map(cost_label)),
                    )
                    .when(opens, |row| {
                        row.child(icon("chev_r", 16., colors.faint).mr(px(-4.)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_subagent(id, pick, window, cx)
                            }))
                    });
                    std::iter::once(row.into_any_element())
                        .chain(interrupted_note(subagent).map(|text| {
                            note("alert", colors.amber, text, colors).into_any_element()
                        }))
                        .chain(subagent.model_note.clone().map(|text| {
                            note("info", colors.muted, text, colors).into_any_element()
                        }))
                        .collect::<Vec<_>>()
                });
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(ui::label(shape(handoff), colors))
                .child(ui::card(colors).child(header).children(rows).children(more)),
        )
    }

    pub(crate) fn open_subagent(
        &mut self,
        id: SessionId,
        pick: Pick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let conversation = self.picked(id, pick).and_then(|s| s.conversation_id);
        self.follow_subagent(id, conversation);
        self.push(Route::Subagent(id, pick), window, cx);
    }

    /// Keeps one subagent's messages coming while its screen is open.
    pub(crate) fn follow_subagent(&mut self, id: SessionId, conversation: Option<String>) {
        if let Some(live) = self.store.as_mut().and_then(|store| store.live.as_mut()) {
            live.follow_subagent(id, conversation);
        }
    }

    fn picked(&self, id: SessionId, pick: Pick) -> Option<Subagent> {
        let session = self.store.as_ref()?.session(id)?;
        session
            .turns
            .get(pick.turn)?
            .handoffs
            .get(pick.handoff)?
            .subagents
            .get(pick.index)
            .cloned()
    }

    /// One subagent: what Pi asked it, and its own run, read-only.
    pub(crate) fn subagent_screen(
        &mut self,
        id: SessionId,
        pick: Pick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _ = window;
        let colors = theme(cx);
        let back = ui::tap("back", "back", &colors).on_click(cx.listener(|this, _, window, cx| {
            this.back(window, cx);
        }));
        let parent = self
            .store
            .as_ref()
            .and_then(|store| store.session(id))
            .map(|session| session.title.clone())
            .unwrap_or_default();
        let Some(subagent) = self.picked(id, pick) else {
            return div()
                .flex_1()
                .child(ui::appbar(back, "Subagent", None, &colors))
                .child(ui::hint("This subagent is no longer in the session.", &colors).px(px(20.)))
                .into_any_element();
        };
        let (_, hue) = agent_look(&subagent.agent, &colors);
        let subtitle = match &subagent.model {
            Some(model) => format!(
                "{} for {parent} · {}",
                subagent.agent,
                model.rsplit('/').next().unwrap_or(model)
            ),
            None => format!("{} for {parent}", subagent.agent),
        };
        let appbar = ui::appbar(
            back,
            first_line(&subagent.task),
            Some(subtitle.into()),
            &colors,
        );
        let fetched = subagent
            .conversation_id
            .as_deref()
            .and_then(|conversation| {
                let store = self.store.as_ref()?;
                match &store.live {
                    Some(live) => live.subagent(id, conversation).cloned(),
                    None => store.sample_subagents.get(conversation).cloned().map(Ok),
                }
            });
        // What Pi wrote to it in full; the call's summary keeps a shorter task.
        let asked = match &fetched {
            Some(Ok(pi)) => pi
                .messages
                .iter()
                .find(|message| message["role"] == "user")
                .map(|message| pi_core::session::content_text(&message["content"]))
                .filter(|text| !text.trim().is_empty()),
            _ => None,
        }
        .unwrap_or_else(|| subagent.task.clone());
        let run = match &fetched {
            Some(Ok(pi)) => {
                let shown = projection::project(
                    pi,
                    projection::Facts {
                        id,
                        cwd: &pi.cwd.to_string_lossy(),
                        folder: String::new(),
                        question: None,
                        outbox: &[],
                        key: "",
                    },
                );
                let turn = shown.turns.last().cloned();
                turn.map(|turn| {
                    let stages = turn
                        .stages
                        .iter()
                        .filter(|stage| {
                            !(subagent.status.finished() && stage.status == StageStatus::Planned)
                        })
                        .collect::<Vec<_>>();
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(24.))
                        .child(div().flex().flex_col().gap(px(24.)).children(
                            stages.iter().enumerate().map(|(n, stage)| {
                                stage_row(stage, &turn, pick.turn, n, n + 1 < stages.len(), &colors)
                            }),
                        ))
                        .children(
                            turn.summary
                                .as_ref()
                                .filter(|_| subagent.status.finished())
                                .map(|summary| crate::message::render(&summary.text(), &colors)),
                        )
                        .into_any_element()
                })
                .unwrap_or_else(|| ui::hint("Not started yet.", &colors).into_any_element())
            }
            Some(Err(error)) => ui::hint(error.clone(), &colors).into_any_element(),
            None if subagent.conversation_id.is_none() => {
                ui::hint("It starts when its turn comes.", &colors).into_any_element()
            }
            None => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(13.))
                .text_color(colors.muted)
                .child(ui::working_indicator(&colors))
                .child("Loading what it did…")
                .into_any_element(),
        };
        let brief = div()
            .pl(px(16.))
            .py(px(4.))
            .border_l(px(3.))
            .border_color(colors.accent)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon("pi", 14., colors.accent))
                    .child(ui::label("Pi asked", &colors)),
            )
            .child(
                div()
                    .mt(px(4.))
                    .text_size(px(15.))
                    .line_height(px(22.))
                    .text_color(colors.secondary)
                    .child(asked),
            );
        let scroll = self.scroll(Route::Subagent(id, pick));
        let running = subagent.status == Status::Running;
        let label = match subagent.status {
            Status::Waiting => "Waiting its turn",
            Status::Running => "Working",
            Status::Done => "Done",
            Status::Failed => "Failed",
            Status::Stopped => "Stopped",
        };
        let spent = [
            subagent
                .elapsed_ms(now_ms())
                .map(|ms| duration_label(Duration::from_millis(ms))),
            subagent.cost.filter(|cost| *cost > 0.).map(cost_label),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let conversation = subagent.conversation_id.clone();
        let strip = div()
            .min_h(px(48.))
            .pl(px(16.))
            .pr(px(8.))
            .flex()
            .items_center()
            .gap(px(12.))
            .rounded(px(24.))
            .bg(colors.panel)
            .border_1()
            .border_color(colors.line_strong)
            .child(if running {
                ui::ring_dot(hue).into_any_element()
            } else {
                icon(
                    if subagent.status == Status::Done {
                        "check"
                    } else {
                        "stop"
                    },
                    16.,
                    colors.muted,
                )
                .into_any_element()
            })
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(label),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.5))
                    .text_color(colors.muted)
                    .child(spent),
            )
            .when_some(conversation.filter(|_| running), |strip, conversation| {
                strip.child(
                    div()
                        .id("stop-subagent")
                        .debug_selector(|| "stop-subagent".into())
                        .relative()
                        .child(crate::testing::probe("stop-subagent"))
                        .h(px(40.))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .rounded(px(20.))
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors.coral)
                        .active(|style| style.bg(colors.selected))
                        .child(icon("stop", 14., colors.coral))
                        .child(format!("Stop this {}", subagent.agent))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let stopped = this
                                .store
                                .as_mut()
                                .and_then(|store| store.live.as_mut())
                                .map(|live| live.stop_subagent(id, &conversation));
                            if let Some(Err(error)) = stopped {
                                this.notify_user(error, cx);
                            }
                            cx.notify();
                        })),
                )
            });
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(appbar)
            .child(
                scroll_area(("subagent", pick.index), &scroll).child(
                    div()
                        .px(px(20.))
                        .pt(px(8.))
                        .pb(px(16.))
                        .flex()
                        .flex_col()
                        .gap(px(24.))
                        .child(brief)
                        .children(interrupted_note(&subagent).map(|text| {
                            note("alert", colors.amber, text, &colors).mx(px(0.)).mb(px(0.))
                        }))
                        .child(run),
                ),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(12.))
                    .pt(px(8.))
                    .pb(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .px(px(8.))
                            .text_size(px(12.5))
                            .line_height(px(18.))
                            .text_color(colors.muted)
                            .child(format!(
                                "Only Pi talks to a subagent. Its answer goes back to {parent} when it finishes."
                            )),
                    )
                    .child(strip),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crew(statuses: &[Status], mode: Mode) -> Handoff {
        let mut handoff = Handoff::from_args(&serde_json::json!({"tasks": statuses
            .iter()
            .map(|_| serde_json::json!({"agent":"scout","task":"look"}))
            .collect::<Vec<_>>()}));
        handoff.mode = mode;
        for (subagent, status) in handoff.subagents.iter_mut().zip(statuses) {
            subagent.status = *status;
        }
        handoff
    }

    #[test]
    fn a_big_crew_shows_those_at_work_first() {
        use Status::*;
        let handoff = crew(
            &[Done, Done, Done, Done, Done, Running, Waiting, Running],
            Mode::Parallel,
        );
        assert_eq!(shown(&handoff, false), [0, 1, 2, 5, 7]);
        assert_eq!(shown(&handoff, true).len(), 8);
        assert_eq!(tally(&handoff), "2 running · 5 done · 1 waiting");
        // Few enough: every one, in order.
        assert_eq!(
            shown(&crew(&[Done, Running], Mode::Parallel), false),
            [0, 1]
        );
    }

    #[test]
    fn a_long_chain_shows_the_steps_up_to_the_current_one() {
        use Status::*;
        let chain = crew(
            &[Done, Done, Done, Done, Done, Done, Running, Waiting],
            Mode::Chain,
        );
        assert_eq!(shown(&chain, false), [2, 3, 4, 5, 6]);
        let started = crew(
            &[Running, Waiting, Waiting, Waiting, Waiting, Waiting],
            Mode::Chain,
        );
        assert_eq!(shown(&started, false), [0, 1, 2, 3, 4]);
        let finished = crew(&[Done; 7], Mode::Chain);
        assert_eq!(shown(&finished, false), [2, 3, 4, 5, 6]);
    }
}
