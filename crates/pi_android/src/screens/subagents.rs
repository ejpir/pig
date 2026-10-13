//! Work Pi handed to subagents (design/android/subagents): a card in the
//! turn, one row per subagent with what it is doing, its time and cost; and a
//! subagent's own screen, opened from its row. Durable children can be steered
//! directly while running when the backend advertises exact-target input support.

use super::scroll_area;
use crate::{
    app::{CrewPick, PhoneApp, Pick, Route},
    model::{SessionId, Turn, duration_label, session_subagents, subagent_summary},
    projection,
    theme::{Theme, theme},
    ui::{self, icon},
};
use gpui::{
    AnyElement, Context, Div, ElementId, Focusable, FontWeight, Hsla, Stateful, Window, div,
    prelude::*, px,
};
use pi_core::subagent::{
    Handoff, Mode, Status, Subagent, Swarm, SwarmMessage, SwarmMessageKind, SwarmMessageStatus,
};
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
    if let Some(swarm) = &handoff.swarm {
        return format!("Read-only swarm · {} peers", swarm.members.len());
    }
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

/// Swarms are small, read-only crews. Bound even malformed/older snapshots.
const SWARM_PEERS: usize = 8;
const RECENT_EXCHANGES: usize = 3;
const EXCHANGE_CHARS: usize = 180;
const RECEIVED_COPY: &str = "Received means the recipient explicitly retrieved it with a tool, not agreement or successful execution.";

fn shown_peer_count(swarm: &Swarm, all: bool) -> usize {
    swarm
        .members
        .len()
        .min(if all { SWARM_PEERS } else { SHOWN })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SwarmView {
    Observed,
    Sample,
    Offline,
}

fn swarm_lifecycle(swarm: &Swarm, view: SwarmView) -> String {
    let state = if swarm.closed { "Closed" } else { "Open" };
    match view {
        SwarmView::Observed => state.into(),
        SwarmView::Sample => format!("Sample · {state}"),
        SwarmView::Offline => format!("Last known: {state}"),
    }
}

fn peer_label(swarm: &Swarm, id: &str) -> String {
    match swarm.members.iter().find(|member| member.id == id) {
        Some(member) => format!("{} · {}", member.agent, member.id),
        None => format!("Unknown peer · {id}"),
    }
}

fn peer_status(status: Status) -> &'static str {
    match status {
        Status::Waiting => "Waiting",
        Status::Running => "Running",
        Status::Done => "Done",
        Status::Failed => "Failed",
        Status::Stopped => "Stopped",
    }
}

fn exchange_kind(kind: SwarmMessageKind) -> &'static str {
    match kind {
        SwarmMessageKind::Finding => "Finding",
        SwarmMessageKind::Question => "Question",
        SwarmMessageKind::Reply => "Reply",
    }
}

fn exchange_delivery(status: SwarmMessageStatus, view: SwarmView) -> String {
    let label = match status {
        SwarmMessageStatus::Queued => "Queued · not retrieved",
        SwarmMessageStatus::Received => "Received · retrieved by recipient",
        SwarmMessageStatus::Cancelled => "Cancelled",
    };
    match view {
        SwarmView::Observed => label.into(),
        SwarmView::Sample => format!("Sample: {label}"),
        SwarmView::Offline => format!("Last known: {label}"),
    }
}

fn exchange_reply(swarm: &Swarm, message: &SwarmMessage) -> Option<String> {
    let id = message.reply_to.as_deref()?;
    Some(match swarm.messages.iter().find(|item| item.id == id) {
        Some(original) => format!("Reply to #{} · {id}", original.sequence),
        None => format!("Reply to {id} · original not in snapshot"),
    })
}

/// Chronological tail by runtime sequence, not arrival order or wall clock.
fn recent_exchanges<'a>(swarm: &'a Swarm, peer: Option<&str>) -> Vec<&'a SwarmMessage> {
    let mut messages = swarm
        .messages
        .iter()
        .filter(|message| peer.is_none_or(|id| message.from == id || message.to == id))
        .collect::<Vec<_>>();
    messages.sort_by(|a, b| a.sequence.cmp(&b.sequence).then_with(|| a.id.cmp(&b.id)));
    let start = messages.len().saturating_sub(RECENT_EXCHANGES);
    messages.drain(..start);
    messages
}

/// Only message bodies are abbreviated; peer/message IDs remain exact.
fn exchange_excerpt(text: &str) -> String {
    // Normalize line breaks without altering case or Unicode content.
    let mut body = text
        .split_whitespace()
        .enumerate()
        .flat_map(|(index, word)| (index > 0).then_some(' ').into_iter().chain(word.chars()));
    let mut excerpt = body.by_ref().take(EXCHANGE_CHARS).collect::<String>();
    if body.next().is_some() {
        excerpt.push('…');
    }
    excerpt
}

fn swarm_scope(swarm: &Swarm, view: SwarmView, colors: &Theme) -> Div {
    div()
        .px(px(16.))
        .py(px(12.))
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(icon("shield", 14., colors.read))
                .child(ui::label(
                    format!("{} · Read-only", swarm_lifecycle(swarm, view)),
                    colors,
                )),
        )
        .child(ui::hint(
            "No shared edits or further spawning. Peer questions stay with peers.",
            colors,
        ))
        // Connection-only changes must not alter the cached turn's row height.
        .child(
            ui::hint(
                match view {
                    SwarmView::Observed => "Observed snapshot · delivery can change.",
                    SwarmView::Sample => "Sample snapshot · no live delivery.",
                    SwarmView::Offline => "Offline · delivery may have changed.",
                },
                colors,
            )
            .line_height(px(18.))
            .min_h(px(36.)),
        )
}

fn swarm_exchanges(swarm: &Swarm, peer: Option<&str>, view: SwarmView, colors: &Theme) -> Div {
    let recent = recent_exchanges(swarm, peer);
    let total = swarm
        .messages
        .iter()
        .filter(|message| peer.is_none_or(|id| message.from == id || message.to == id))
        .count();
    div()
        .px(px(16.))
        .py(px(12.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .border_t_1()
        .border_color(colors.line)
        .child(ui::label(
            format!("Recent exchanges · {} of {total}", recent.len()),
            colors,
        ))
        .when(recent.is_empty(), |body| {
            body.child(ui::hint("No recorded peer exchanges.", colors))
        })
        .children(recent.into_iter().map(|message| {
            let hue = match message.status {
                SwarmMessageStatus::Queued => colors.amber,
                SwarmMessageStatus::Received => colors.read,
                SwarmMessageStatus::Cancelled => colors.muted,
            };
            let probe = format!("swarm-exchange-{}-{}", swarm.id, message.sequence);
            div()
                .id(ElementId::Name(probe.clone().into()))
                .debug_selector(move || probe.clone())
                .flex()
                .flex_col()
                .gap(px(4.))
                .p(px(10.))
                .rounded(px(10.))
                .bg(colors.canvas)
                .border_1()
                .border_color(colors.line)
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .text_color(colors.secondary)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(peer_label(swarm, &message.from)),
                        )
                        .child(icon("chev_r", 14., colors.muted).mt(px(2.)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(peer_label(swarm, &message.to)),
                        ),
                )
                .child(
                    ui::mono(
                        format!(
                            "#{} · {} · {}",
                            message.sequence,
                            message.id,
                            exchange_kind(message.kind)
                        ),
                        11.,
                    )
                    .text_color(colors.muted),
                )
                .children(
                    exchange_reply(swarm, message)
                        .map(|reply| ui::hint(reply, colors).text_size(px(11.5))),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .line_height(px(18.))
                        .text_color(colors.secondary)
                        .child(exchange_excerpt(&message.text)),
                )
                .child(
                    ui::hint(exchange_delivery(message.status, view), colors)
                        .text_color(hue)
                        .line_height(px(18.))
                        .min_h(px(36.)),
                )
        }))
        .child(ui::hint(RECEIVED_COPY, colors))
        .child(ui::hint(
            if peer.is_some() {
                "Full conversation below."
            } else {
                "Open a peer row for its full conversation."
            },
            colors,
        ))
}

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

/// Finished child conversations are read-only here, even if the backend can
/// start another turn. Only an exact active run offers a steering composer.
fn show_direct_input(
    target: &Result<crate::live::SubagentInputTarget, String>,
    running: bool,
) -> bool {
    match target {
        Ok(crate::live::SubagentInputTarget::Steer { .. }) => true,
        Ok(crate::live::SubagentInputTarget::FollowUp) => false,
        Err(_) => running,
    }
}

fn direct_input_copy(agent: &str) -> (&'static str, String) {
    (
        "Steers the exact run shown above",
        format!("Steer this {agent}…"),
    )
}

/// What a subagent's row says under its task.
fn status_line(subagent: &Subagent, mixed: bool) -> String {
    let progress = subagent.now.as_deref().or(subagent.output.as_deref());
    let said = match (subagent.status, progress) {
        (Status::Waiting, _) => "Waiting its turn".to_owned(),
        (Status::Stopped, None) => "Stopped".to_owned(),
        (Status::Failed, None) => "Failed".to_owned(),
        (_, Some(progress)) => first_line(progress),
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

/// Every unfinished subagent in a durable background hand-off, across the session.
pub(crate) fn background_at_work(turns: &[Turn]) -> usize {
    turns
        .iter()
        .flat_map(|turn| &turn.handoffs)
        .filter(|handoff| handoff.background)
        .flat_map(|handoff| &handoff.subagents)
        .filter(|subagent| !subagent.status.finished())
        .count()
}

impl PhoneApp {
    fn swarm_view(&self, id: SessionId) -> SwarmView {
        match self.store.as_ref() {
            Some(store) if !store.computer.connected => SwarmView::Offline,
            Some(store) if store.is_sample() => SwarmView::Sample,
            Some(store)
                if !self.is_reconnecting()
                    && store.live.as_ref().is_some_and(|live| {
                        !live.connection.is_closed() && live.session_current(id)
                    }) =>
            {
                SwarmView::Observed
            }
            _ => SwarmView::Offline,
        }
    }

    /// Roster order and exact peer IDs, never agent names or child indices, bind rows.
    #[allow(clippy::too_many_arguments)]
    fn swarm_peer_rows(
        &self,
        id: SessionId,
        turn: usize,
        handoff_index: usize,
        handoff: &Handoff,
        all: bool,
        now: u64,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(swarm) = &handoff.swarm else {
            return Vec::new();
        };
        swarm
            .members
            .iter()
            .take(shown_peer_count(swarm, all))
            .enumerate()
            .flat_map(|(position, member)| {
                if let Some((index, subagent)) = super::swarm::node(handoff, &member.id) {
                    let row = self.subagent_row(
                        id,
                        Pick {
                            turn,
                            handoff: handoff_index,
                            index,
                        },
                        subagent,
                        position == 0,
                        None,
                        true,
                        Some(swarm),
                        format!("subagent-{turn}-{handoff_index}-{index}"),
                        now,
                        colors,
                        cx,
                    );
                    std::iter::once(row.into_any_element())
                        .chain(interrupted_note(subagent).map(|text| {
                            note("alert", colors.amber, text, colors).into_any_element()
                        }))
                        .chain(subagent.model_note.clone().map(|text| {
                            note("info", colors.muted, text, colors).into_any_element()
                        }))
                        .collect::<Vec<_>>()
                } else {
                    let (glyph, hue) = agent_look(&member.agent, colors);
                    vec![
                        ui::row(
                            ElementId::Name(
                                format!("swarm-unbound-peer-{turn}-{handoff_index}-{}", member.id)
                                    .into(),
                            ),
                            position == 0,
                            colors,
                        )
                        .gap(px(12.))
                        .child(
                            div()
                                .size(px(32.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(10.))
                                .bg(colors.tint(hue))
                                .child(icon(glyph, 16., hue)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(ui::label(peer_label(swarm, &member.id), colors))
                                .child(ui::hint(first_line(&member.task), colors).truncate())
                                .child(ui::hint(
                                    "Run status unavailable · no child to open",
                                    colors,
                                )),
                        )
                        .into_any_element(),
                    ]
                }
            })
            .collect()
    }

    /// The shared subagent row used in a hand-off card and the session-wide directory.
    #[allow(clippy::too_many_arguments)]
    fn subagent_row(
        &self,
        id: SessionId,
        pick: Pick,
        subagent: &Subagent,
        first: bool,
        chain_number: Option<usize>,
        mixed: bool,
        swarm: Option<&Swarm>,
        probe: String,
        now: u64,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let opens = subagent.conversation_id.is_some();
        let peer_namespace = self.swarm_namespace(id);
        let peer_binding = swarm.and_then(|swarm| {
            subagent
                .peer_id
                .as_ref()
                .map(|peer| (swarm.id.clone(), peer.clone()))
        });
        let waiting = subagent.status == Status::Waiting;
        let identity = subagent.peer_id.as_deref().map(|peer| {
            swarm.map_or_else(
                || format!("{} · {peer}", subagent.agent),
                |swarm| peer_label(swarm, peer),
            )
        });
        let status = if identity.is_some() {
            let prefix = if self.swarm_view(id) == SwarmView::Offline {
                "Last known: "
            } else {
                ""
            };
            format!(
                "{prefix}{} · {}",
                peer_status(subagent.status),
                status_line(subagent, false)
            )
        } else {
            status_line(subagent, mixed)
        };
        let time = subagent
            .elapsed_ms(now)
            .map(|ms| duration_label(Duration::from_millis(ms)));
        ui::row(ElementId::Name(probe.clone().into()), first, colors)
            .debug_selector(move || probe.clone())
            .min_h(px(64.))
            .gap(px(12.))
            .children(chain_number.map(|number| {
                div()
                    .w(px(14.))
                    .flex_none()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(colors.muted)
                    .child(number.to_string())
            }))
            .child(tile(subagent, colors))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .children(identity.map(|identity| ui::label(identity, colors)))
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
                            .child(status),
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
                        if let Some((swarm, peer)) = &peer_binding {
                            if this.swarm_namespace(id) != peer_namespace {
                                this.notify_user(
                                    "This peer belonged to the previous session source.",
                                    cx,
                                );
                                return;
                            }
                            this.open_swarm_peer_from_handoff(
                                id,
                                CrewPick {
                                    turn: pick.turn,
                                    handoff: pick.handoff,
                                },
                                swarm,
                                peer,
                                window,
                                cx,
                            );
                        } else {
                            this.open_subagent(id, pick, window, cx);
                        }
                    }))
            })
    }

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
                            .child(if handoff.swarm.is_some() {
                                "Swarm peers"
                            } else {
                                "Handed off"
                            }),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.5))
                            .text_color(colors.muted)
                            .child(handoff.swarm.as_ref().map_or_else(
                                || tally(handoff),
                                |swarm| {
                                    format!(
                                        "{} · {}",
                                        swarm_lifecycle(swarm, self.swarm_view(id)),
                                        tally(handoff)
                                    )
                                },
                            )),
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
                if let Some(list) = this.thread_lists.get(&id) {
                    list.pause_following_tail();
                }
                if !this.folded_handoffs.remove(&key) {
                    this.folded_handoffs.insert(key);
                }
                this.remeasure_thread_turn(id, turn_index);
                cx.notify();
            }));
        let visible = if folded {
            Vec::new()
        } else {
            shown(handoff, all)
        };
        let peer_count = handoff
            .swarm
            .as_ref()
            .map(|swarm| swarm.members.len().min(SWARM_PEERS));
        let hidden = peer_count.map_or_else(
            || handoff.subagents.len() - shown(handoff, false).len(),
            |count| count.saturating_sub(SHOWN),
        );
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
                    format!("Show all {}", peer_count.unwrap_or(handoff.subagents.len()))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(list) = this.thread_lists.get(&id) {
                        list.pause_following_tail();
                    }
                    if !this.all_subagents.remove(&key) {
                        this.all_subagents.insert(key);
                    }
                    this.remeasure_thread_turn(id, turn_index);
                    cx.notify();
                }))
        });
        let rows = if handoff.swarm.is_some() {
            if folded {
                Vec::new()
            } else {
                self.swarm_peer_rows(id, turn_index, n, handoff, all, now, colors, cx)
            }
        } else {
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
                    let row = self.subagent_row(
                        id,
                        pick,
                        subagent,
                        position == 0,
                        (handoff.mode == Mode::Chain).then_some(index + 1),
                        mixed,
                        None,
                        format!("subagent-{turn_index}-{n}-{index}"),
                        now,
                        colors,
                        cx,
                    );
                    std::iter::once(row.into_any_element())
                        .chain(interrupted_note(subagent).map(|text| {
                            note("alert", colors.amber, text, colors).into_any_element()
                        }))
                        .chain(subagent.model_note.clone().map(|text| {
                            note("info", colors.muted, text, colors).into_any_element()
                        }))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(ui::label(shape(handoff), colors))
                .child(
                    ui::card(colors)
                        .child(header)
                        .when_some(handoff.swarm.as_ref().filter(|_| !folded), |card, swarm| {
                            let expected = swarm.id.clone();
                            let namespace = self.swarm_namespace(id);
                            let selector = format!("open-swarm-{turn_index}-{n}");
                            card.child(
                                ui::button(
                                    selector.clone(),
                                    ui::Button::Quiet,
                                    Some("fork"),
                                    "Open swarm map & exchanges",
                                    false,
                                    colors,
                                )
                                .debug_selector(move || selector.clone())
                                .w_full()
                                .px(px(8.))
                                .text_size(px(13.))
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        if this.swarm_namespace(id) != namespace {
                                            this.notify_user(
                                            "This swarm belonged to the previous session source.",
                                            cx,
                                        );
                                            return;
                                        }
                                        this.open_swarm(
                                            id,
                                            CrewPick {
                                                turn: turn_index,
                                                handoff: n,
                                            },
                                            &expected,
                                            window,
                                            cx,
                                        );
                                    },
                                )),
                            )
                            .child(swarm_scope(
                                swarm,
                                self.swarm_view(id),
                                colors,
                            ))
                        })
                        .children(rows)
                        .children(more)
                        .when_some(handoff.swarm.as_ref().filter(|_| !folded), |card, swarm| {
                            card.when(swarm.members.len() > SWARM_PEERS, |card| {
                                card.child(
                                    ui::hint("Only the first 8 peers are shown.", colors)
                                        .px(px(16.)),
                                )
                            })
                            .child(swarm_exchanges(
                                swarm,
                                None,
                                self.swarm_view(id),
                                colors,
                            ))
                        }),
                ),
        )
    }

    /// Every subagent started anywhere in a session, without hunting through chat.
    pub(crate) fn subagents_screen(
        &mut self,
        id: SessionId,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = theme(cx);
        let scroll = self.scroll(Route::Subagents(id));
        let back = ui::tap("back", "back", &colors).on_click(cx.listener(|this, _, window, cx| {
            this.back(window, cx);
        }));
        let Some(session) = self.store.as_ref().and_then(|store| store.session(id)) else {
            return div()
                .flex_1()
                .child(ui::appbar(back, "Subagents", None, &colors))
                .child(ui::hint("This session is no longer on this phone.", &colors).px(px(20.)))
                .into_any_element();
        };
        let title = session.title.clone();
        let summary = subagent_summary(&session.turns);
        let now = now_ms();
        let rows = session_subagents(&session.turns)
            .into_iter()
            .enumerate()
            .map(|(position, item)| {
                self.subagent_row(
                    id,
                    Pick {
                        turn: item.turn,
                        handoff: item.handoff,
                        index: item.index,
                    },
                    item.subagent,
                    position == 0,
                    None,
                    true,
                    session.turns[item.turn].handoffs[item.handoff]
                        .swarm
                        .as_ref(),
                    format!(
                        "directory-subagent-{}-{}-{}",
                        item.turn, item.handoff, item.index
                    ),
                    now,
                    &colors,
                    cx,
                )
            })
            .collect::<Vec<_>>();
        let appbar = ui::appbar(back, "Subagents", Some(title.into()), &colors);
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(appbar)
            .child(
                scroll_area("subagents-directory", &scroll).child(
                    div()
                        .px(px(20.))
                        .pt(px(8.))
                        .pb(px(24.))
                        .children(summary.map(|summary| {
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(summary)
                        }))
                        .child(
                            ui::hint(
                                "Every hand-off in this session. Active work stays at the top.",
                                &colors,
                            )
                            .mt(px(4.))
                            .mb(px(12.)),
                        )
                        .when(rows.is_empty(), |body| {
                            body.child(ui::hint("This session has no subagents.", &colors))
                        })
                        .when(!rows.is_empty(), |body| {
                            body.child(ui::card(&colors).children(rows))
                        }),
                ),
            )
            .into_any_element()
    }

    pub(crate) fn open_subagent(
        &mut self,
        id: SessionId,
        pick: Pick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let swarm_peer = self
            .store
            .as_ref()
            .and_then(|store| store.session(id))
            .and_then(|session| session.turns.get(pick.turn))
            .and_then(|turn| turn.handoffs.get(pick.handoff))
            .and_then(|handoff| {
                let swarm = handoff.swarm.as_ref()?;
                let peer = handoff.subagents.get(pick.index)?.peer_id.clone()?;
                Some((swarm.id.clone(), peer))
            });
        if let Some((swarm, peer)) = swarm_peer {
            self.open_swarm_peer_from_handoff(
                id,
                CrewPick {
                    turn: pick.turn,
                    handoff: pick.handoff,
                },
                &swarm,
                &peer,
                window,
                cx,
            );
            return;
        }
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
        if let Route::SwarmPeer(shown, original, token) = self.route() {
            if shown != id || original != pick {
                return None;
            }
            return self
                .bound_swarm_peer(self.swarm_peer_bindings.get(&token)?)
                .cloned();
        }
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

    /// One subagent: what Pi asked it, its own run, and exact-target durable input.
    pub(crate) fn subagent_screen(
        &mut self,
        id: SessionId,
        pick: Pick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
            self.follow_subagent(id, None);
            return div()
                .flex_1()
                .child(ui::appbar(back, "Subagent", None, &colors))
                .child(ui::hint("This subagent is no longer in the session.", &colors).px(px(20.)))
                .into_any_element();
        };
        let swarm = self
            .store
            .as_ref()
            .and_then(|store| store.session(id))
            .and_then(|session| session.turns.get(pick.turn))
            .and_then(|turn| turn.handoffs.get(pick.handoff))
            .and_then(|handoff| handoff.swarm.clone());
        let swarm_view = self.swarm_view(id);
        // The latest projection may attach/change the picked conversation
        // after navigation. Keep the displayed transcript and follow target
        // aligned; following the same child does not start another read.
        self.follow_subagent(id, subagent.conversation_id.clone());
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
        // Drawn like a session of its own, under an ID of the phone's: the
        // sheets that open from it find it there.
        let own = match &fetched {
            Some(Ok(pi)) => {
                let key = (id, subagent.conversation_id.clone().unwrap_or_default());
                let next = SessionId(u32::MAX - self.subagent_ids.len() as u32);
                let own = *self.subagent_ids.entry(key).or_insert(next);
                let shown = projection::project(
                    pi,
                    projection::Facts {
                        id: own,
                        cwd: &pi.cwd.to_string_lossy(),
                        folder: String::new(),
                        question: None,
                        outbox: &[],
                        key: "",
                    },
                );
                self.subagent_sessions.insert(own, shown);
                Some(own)
            }
            _ => None,
        };
        let run = match &fetched {
            Some(Ok(_)) => {
                let session = own.and_then(|own| self.subagent_sessions.get(&own));
                match session.filter(|session| !session.turns.is_empty()) {
                    Some(session) => div()
                        .flex()
                        .flex_col()
                        .gap(px(24.))
                        .children(self.turns(session, true, &colors, cx))
                        .into_any_element(),
                    None => ui::hint("Not started yet.", &colors).into_any_element(),
                }
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
                .child("Loading subagent conversation…")
                .into_any_element(),
        };
        // Until its own run shows, what Pi asked it.
        let asked = subagent.task.clone();
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
        let route = match self.route() {
            Route::SwarmPeer(shown, _, _) if shown == id => self.route(),
            _ => Route::Subagent(id, pick),
        };
        let mut brief = Some(brief);
        let scroll = self.scroll(route);
        let child_running = fetched
            .as_ref()
            .is_some_and(|fetched| fetched.as_ref().is_ok_and(pi_core::session::Session::busy));
        let conversation = subagent.conversation_id.clone();
        let (input_target, input_pending) =
            conversation
                .as_deref()
                .map_or((None, false), |conversation| {
                    let Some(live) = self.store.as_ref().and_then(|store| store.live.as_ref())
                    else {
                        return (None, false);
                    };
                    (
                        Some(if swarm.is_some() && swarm_view != SwarmView::Observed {
                            Err("Reconnect to a current session before sending to this peer. Nothing is queued offline.".to_owned())
                        } else { live.subagent_input_target(id, conversation) }),
                        live.subagent_input_pending(id, conversation),
                    )
                });
        // Only the isolated test APK can mount a direct composer without a host.
        // Its sample store has no transport, so neither Send nor Stop can affect real sessions.
        #[cfg(feature = "ui-test")]
        let input_target = if self.paused
            && self.store.as_ref().is_some_and(|store| store.is_sample())
            && cx
                .try_global::<crate::testing::State>()
                .is_some_and(|state| state.fixture == "subagent-input")
        {
            Some(Ok(crate::live::SubagentInputTarget::Steer {
                run_id: "sample-only".into(),
            }))
        } else {
            input_target
        };
        let running = child_running || subagent.status == Status::Running || input_pending;
        let label = if child_running {
            "Working"
        } else if input_pending {
            "Sending"
        } else {
            match subagent.status {
                Status::Waiting => "Waiting its turn",
                Status::Running => "Working",
                Status::Done => "Done",
                Status::Failed => "Failed",
                Status::Stopped => "Stopped",
            }
        };
        let spent = [
            subagent
                .elapsed_ms(if swarm.is_some() && swarm_view == SwarmView::Offline {
                    self.store
                        .as_ref()
                        .and_then(|store| store.observed_at.get(&id))
                        .copied()
                        .unwrap_or(subagent.started_at.unwrap_or(0))
                } else {
                    now_ms()
                })
                .map(|ms| duration_label(Duration::from_millis(ms))),
            subagent.cost.filter(|cost| *cost > 0.).map(cost_label),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let label = if swarm.is_some() && swarm_view == SwarmView::Offline {
            format!("Last known: {label}")
        } else {
            label.to_owned()
        };
        let stop_namespace = self.swarm_namespace(id);
        let swarm_stop = swarm.as_ref().and_then(|swarm| {
            subagent
                .peer_id
                .as_ref()
                .map(|peer| (swarm.id.clone(), peer.clone()))
        });
        let joined_input = conversation.is_some()
            && matches!(
                input_target.as_ref(),
                Some(Ok(crate::live::SubagentInputTarget::Steer { .. }))
            );
        let strip = ui::composer_status_strip(joined_input, &colors)
            .debug_selector(|| "subagent-working-strip".into())
            .relative()
            .child(crate::testing::probe("subagent-working-strip"))
            .pl(px(16.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .py(px(4.))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(14.))
                            .line_height(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(hue)
                            .child(label),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(if running {
                                if swarm.is_some() && swarm_view != SwarmView::Observed {
                                    div().size(px(7.)).rounded_full().bg(hue).into_any_element()
                                } else {
                                    ui::ring_dot(hue).into_any_element()
                                }
                            } else {
                                icon(
                                    if subagent.status == Status::Done { "check" } else { "stop" },
                                    16.,
                                    colors.muted,
                                )
                                .into_any_element()
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.5))
                                    .text_color(colors.muted)
                                    .child(spent),
                            ),
                    ),
            )
            .when_some(
                conversation.clone().filter(|_| {
                    (child_running || subagent.status == Status::Running)
                        && (swarm.is_none()
                            || (swarm_view == SwarmView::Observed
                                && (child_running || swarm.as_ref().is_some_and(|swarm| !swarm.closed))))
                }),
                |strip, conversation| {
                    strip.child(
                        div()
                            .id("stop-subagent")
                            .debug_selector(|| "stop-subagent".into())
                            .relative()
                            .child(crate::testing::probe("stop-subagent"))
                            .h(px(if swarm.is_some() { 48. } else { 40. }))
                            .flex_none()
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
                            .aria_label(format!("Stop {}", subagent.agent))
                            .child("Stop")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some((expected, peer)) = &swarm_stop {
                                    if this.swarm_namespace(id) != stop_namespace { this.notify_user("This peer belonged to the previous session source. Nothing was sent.", cx); return; }
                                    this.request_swarm_peer_stop(
                                        id,
                                        CrewPick {
                                            turn: pick.turn,
                                            handoff: pick.handoff,
                                        },
                                        expected,
                                        peer,
                                        window,
                                        cx,
                                    );
                                    return;
                                }
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
                },
            );
        if matches!(
            input_target.as_ref(),
            Some(Ok(crate::live::SubagentInputTarget::FollowUp))
        ) && let Some(composer) = conversation
            .as_ref()
            .and_then(|conversation| self.subagent_composers.get(&(id, conversation.clone())))
        {
            let area = composer.read(cx).area.clone();
            if area.read(cx).focus_handle(cx).is_focused(window) {
                window.focus(&self.focus, cx);
                window.dismiss_virtual_keyboard();
            }
        }
        let input_target = input_target.filter(|target| show_direct_input(target, running));
        let mut strip = Some(strip);
        let direct = match (conversation.as_deref(), input_target) {
            (Some(conversation), Some(Ok(target))) => {
                if swarm.is_some() {
                    self.swarm_input_targets
                        .insert((id, conversation.to_owned()), target.clone());
                }
                let (action, placeholder) = direct_input_copy(&subagent.agent);
                let accepted_input = self
                    .store
                    .as_mut()
                    .and_then(|store| store.live.as_mut())
                    .and_then(|live| live.take_accepted_subagent_input(id, conversation));
                let composer = self.subagent_composer(id, conversation, &placeholder, window, cx);
                composer.update(cx, |composer, cx| {
                    composer.set_joined(true);
                    composer.set_sending(input_pending);
                    if let Some(accepted) = accepted_input.as_deref() {
                        composer.clear_if_text(accepted, cx);
                    }
                });
                Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            div()
                                .px(px(20.))
                                .text_size(px(12.))
                                .text_color(colors.muted)
                                .child(action),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .children(strip.take())
                                .child(composer),
                        )
                        .into_any_element(),
                )
            }
            (_, Some(Err(error))) => Some(
                div()
                    .px(px(20.))
                    .text_size(px(12.5))
                    .line_height(px(18.))
                    .text_color(colors.muted)
                    .child(error)
                    .into_any_element(),
            ),
            _ => None,
        };
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
                        .when(swarm.is_some(), |column| column.child(brief.take().unwrap()))
                        .children(swarm.as_ref().map(|swarm| {
                            ui::card(&colors)
                                .child(swarm_scope(swarm, swarm_view, &colors))
                                .child(ui::label(
                                    subagent.peer_id.as_deref().map_or_else(
                                        || "Peer ID unavailable".into(),
                                        |peer| peer_label(swarm, peer),
                                    ),
                                    &colors,
                                ).px(px(16.)).pb(px(8.)))
                                .child(ui::hint("Original tools · read + peer messaging during delegation only. No edits or further spawning.", &colors).px(px(16.)).pb(px(8.)))
                                .child(ui::hint("Workspace · shared project for reading; no isolated editing worktree or artifact support.", &colors).px(px(16.)).pb(px(8.)))
                                .child({
                                    let expected = swarm.id.clone();
                                    let namespace = self.swarm_namespace(id);
                                    ui::button("peer-open-swarm", ui::Button::Quiet, Some("fork"), "View swarm map & exchanges", false, &colors)
                                        .w_full().px(px(4.)).text_size(px(13.))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            if this.swarm_namespace(id) != namespace { this.notify_user("This swarm belonged to the previous session source.", cx); return; }
                                            this.open_swarm(id, CrewPick { turn: pick.turn, handoff: pick.handoff }, &expected, window, cx);
                                        }))
                                })
                                .when_some(subagent.peer_id.as_deref(), |card, peer| {
                                    let expected = swarm.id.clone();
                                    let peer = peer.to_owned();
                                    let namespace = self.swarm_namespace(id);
                                    card.child(ui::button("peer-exchanges", ui::Button::Quiet, Some("chat"), "Inspect this peer’s exchanges", false, &colors).debug_selector(|| "peer-exchanges".into()).w_full().px(px(4.)).text_size(px(13.))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            if this.swarm_namespace(id) != namespace { this.notify_user("This peer belonged to the previous session source.", cx); return; }
                                            let pick = CrewPick { turn: pick.turn, handoff: pick.handoff };
                                            this.open_swarm(id, pick, &expected, window, cx);
                                            if let Some(state) = this.swarm_views.get_mut(&(id, pick)).filter(|state| state.swarm_id == expected) { state.tab = super::swarm::SwarmTab::Exchanges; state.filter = super::swarm::ExchangeFilter::All; state.peer = Some(peer.clone()); state.open_message = None; }
                                            cx.notify();
                                        })))
                                })
                        }))
                        .when(own.is_none() && swarm.is_none(), |column| column.child(brief.take().unwrap()))
                        .children(interrupted_note(&subagent).map(|text| {
                            note("alert", colors.amber, text, &colors).mx(px(0.)).mb(px(0.))
                        }))
                        .child(run),
                ),
            )
            .child(
                div()
                    .flex_none()
                    .pt(px(8.))
                    .pb(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .children(direct)
                    .when(conversation.is_none(), |dock| {
                        dock.child(
                            div()
                                .px(px(20.))
                                .text_size(px(12.5))
                                .line_height(px(18.))
                                .text_color(colors.muted)
                                .child(format!(
                                    "This subagent has no durable conversation to message. Its answer goes back to {parent}."
                                )),
                        )
                    })
                    .children(strip),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "ui-test")]
    #[gpui::test]
    fn direct_input_joins_working_and_stop_to_the_regular_composer(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| cx.set_global(crate::testing::State::default()));
        for (width, height) in [(320., 640.), (412., 915.), (640., 360.), (320., 400.)] {
            let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
            cx.simulate_resize(gpui::size(px(width), px(height)));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.preview("subagent-input", window, cx));
            });
            cx.run_until_parked();
            let strip = cx.debug_bounds("subagent-working-strip").unwrap();
            let composer = cx.debug_bounds("composer").unwrap();
            let draft = cx.debug_bounds("composer-draft").unwrap();
            let stop = cx.debug_bounds("stop-subagent").unwrap();
            let send = cx.debug_bounds("send").unwrap();
            assert_eq!(strip.left(), composer.left());
            assert_eq!(strip.right(), composer.right());
            assert_eq!(strip.bottom(), composer.top() + px(1.));
            assert!(strip.top() <= stop.top() && stop.bottom() <= strip.bottom());
            assert!(strip.left() <= stop.left() && stop.right() <= strip.right());
            assert!(draft.bottom() <= send.top());
            assert!(composer.bottom() <= px(height));
            assert!(cx.debug_bounds("composer-model").is_none());
            assert!(cx.debug_bounds("composer-thinking").is_none());

            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.preview("subagent", window, cx));
            });
            cx.run_until_parked();
            assert!(cx.debug_bounds("subagent-working-strip").is_some());
            assert!(cx.debug_bounds("stop-subagent").is_some());
            assert!(
                cx.debug_bounds("composer").is_none(),
                "no editable input without a live target"
            );
        }
    }

    fn example_swarm() -> Swarm {
        Swarm::from_details(&serde_json::json!({
            "version":1,"id":"test-swarm","closed":false,
            "members":[
                {"id":"peer-1","agent":"scout","task":"Read Anthropic"},
                {"id":"peer-2","agent":"scout","task":"Read OpenAI"}
            ],
            "messages":[
                {"id":"finding-1","sequence":1,"from":"peer-1","to":"peer-2","kind":"finding","text":"Header found","status":"received","createdAt":1,"receivedAt":2},
                {"id":"question-2","sequence":2,"from":"peer-2","to":"peer-1","kind":"question","text":"Which parser?","status":"received","createdAt":3,"receivedAt":4},
                {"id":"reply-3","sequence":3,"from":"peer-1","to":"peer-2","kind":"reply","replyTo":"question-2","text":"The shared parser","status":"queued","createdAt":5}
            ]
        })).unwrap()
    }

    #[test]
    fn swarm_copy_names_exact_peers_receipts_and_lifecycle_without_success_claims() {
        let mut swarm = example_swarm();
        assert_eq!(peer_label(&swarm, "peer-1"), "scout · peer-1");
        assert_eq!(peer_label(&swarm, "peer-2"), "scout · peer-2");
        assert_eq!(
            peer_label(&swarm, "foreign-id"),
            "Unknown peer · foreign-id"
        );
        assert_eq!(swarm_lifecycle(&swarm, SwarmView::Observed), "Open");
        swarm.closed = true;
        assert_eq!(swarm_lifecycle(&swarm, SwarmView::Observed), "Closed");
        assert_eq!(
            swarm_lifecycle(&swarm, SwarmView::Offline),
            "Last known: Closed"
        );
        assert_eq!(
            exchange_delivery(SwarmMessageStatus::Received, SwarmView::Observed),
            "Received · retrieved by recipient"
        );
        assert_eq!(
            exchange_delivery(SwarmMessageStatus::Queued, SwarmView::Observed),
            "Queued · not retrieved"
        );
        assert_eq!(
            exchange_delivery(SwarmMessageStatus::Cancelled, SwarmView::Observed),
            "Cancelled"
        );
        for status in [
            SwarmMessageStatus::Queued,
            SwarmMessageStatus::Received,
            SwarmMessageStatus::Cancelled,
        ] {
            assert!(exchange_delivery(status, SwarmView::Offline).starts_with("Last known: "));
            assert!(exchange_delivery(status, SwarmView::Sample).starts_with("Sample: "));
        }
        assert!(RECEIVED_COPY.contains("explicitly retrieved"));
        assert!(RECEIVED_COPY.contains("not agreement or successful execution"));
        assert_eq!(exchange_kind(SwarmMessageKind::Question), "Question");
    }

    #[test]
    fn swarm_replies_keep_exact_correlation_even_outside_recent_history() {
        let mut swarm = example_swarm();
        assert_eq!(
            exchange_reply(&swarm, &swarm.messages[2]).as_deref(),
            Some("Reply to #2 · question-2")
        );
        assert_eq!(exchange_reply(&swarm, &swarm.messages[0]), None);
        swarm.messages[2].reply_to = Some("older-question-id".into());
        assert_eq!(
            exchange_reply(&swarm, &swarm.messages[2]).as_deref(),
            Some("Reply to older-question-id · original not in snapshot")
        );
    }

    #[test]
    fn swarm_history_is_a_bounded_chronological_tail_and_child_scope_is_exact() {
        let mut swarm = example_swarm();
        let template = swarm.messages[0].clone();
        swarm.messages = (0..40)
            .rev()
            .map(|n| SwarmMessage {
                id: format!("msg-{n}"),
                sequence: n,
                from: if n % 2 == 0 {
                    "peer-1".into()
                } else {
                    "peer-other".into()
                },
                to: "peer-2".into(),
                ..template.clone()
            })
            .collect();
        assert_eq!(
            recent_exchanges(&swarm, None)
                .iter()
                .map(|m| m.sequence)
                .collect::<Vec<_>>(),
            [37, 38, 39]
        );
        assert_eq!(
            recent_exchanges(&swarm, Some("peer-1"))
                .iter()
                .map(|m| m.sequence)
                .collect::<Vec<_>>(),
            [34, 36, 38]
        );
        assert!(recent_exchanges(&swarm, Some("peer-404")).is_empty());
        swarm.messages.clear();
        assert!(recent_exchanges(&swarm, None).is_empty());
    }

    #[test]
    fn swarm_roster_and_unicode_body_bounds_do_not_abbreviate_ids() {
        let mut swarm = example_swarm();
        let mut member = swarm.members[0].clone();
        member.id = "exact-long-id-with-no-display-name-guessing".into();
        swarm.members = vec![member; 100];
        assert_eq!(shown_peer_count(&swarm, false), SHOWN);
        assert_eq!(shown_peer_count(&swarm, true), SWARM_PEERS);
        assert!(
            peer_label(&swarm, "exact-long-id-with-no-display-name-guessing")
                .ends_with("exact-long-id-with-no-display-name-guessing")
        );
        assert_eq!(
            exchange_excerpt(" A  finding\n\n中文 café "),
            "A finding 中文 café"
        );
        let excerpt = exchange_excerpt(&"中文 café 👩🏽‍💻 ".repeat(100));
        assert_eq!(excerpt.chars().count(), EXCHANGE_CHARS + 1);
        assert!(excerpt.ends_with('…'));
        assert_eq!(exchange_excerpt(""), "");
    }

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
    fn direct_input_copy_names_the_child_and_exact_lifecycle_action() {
        let steer = direct_input_copy("scout");
        assert_eq!(steer.0, "Steers the exact run shown above");
        assert_eq!(steer.1, "Steer this scout…");
    }

    #[test]
    fn finished_subagents_never_offer_a_follow_up_composer() {
        use crate::live::SubagentInputTarget::{FollowUp, Steer};
        // A cached roster can lag behind the exact child's lifecycle. The
        // backend's target, not an old Running row, determines whether to steer.
        for running in [false, true] {
            assert!(!show_direct_input(&Ok(FollowUp), running));
            assert!(show_direct_input(
                &Ok(Steer {
                    run_id: "run-7".into()
                }),
                running
            ));
        }
        let unavailable = Err("Disconnected".into());
        assert!(show_direct_input(&unavailable, true));
        assert!(!show_direct_input(&unavailable, false));
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
