//! Native, snapshot-driven swarm map. Roster order owns positions; exact peer
//! IDs own navigation. Coordinator rails are membership, never dependencies.

use crate::{
    app::{CrewPick, PhoneApp},
    model::SessionId,
    theme::Theme,
    ui::{self, icon},
};
use gpui::{
    Bounds, ContentMask, Context, Div, ElementId, FontWeight, Hsla, PathBuilder, Pixels, Point,
    Role, Stateful, Window, canvas, div, point, prelude::*, px,
};
use pi_core::subagent::{
    Handoff, Status, Swarm, SwarmMember, SwarmMessage, SwarmMessageKind, SwarmMessageStatus,
};
use std::{cell::RefCell, collections::HashSet, rc::Rc};

const MAX_PEERS: usize = 8;
const MAX_EDGES: usize = 8;
const COLUMN_GAP: f32 = 40.;
const ROW_GAP: f32 = 36.;

/// No sorting by name, execution state, message activity or result index.
fn roster_slots(swarm: &Swarm) -> Vec<(&str, usize, usize)> {
    swarm
        .members
        .iter()
        .enumerate()
        .map(|(index, member)| (member.id.as_str(), index / 2, index % 2))
        .collect()
}

fn child_status(handoff: &Handoff, peer_id: &str) -> Option<Status> {
    handoff.swarm.as_ref()?.member(peer_id)?;
    let mut matching = handoff
        .subagents
        .iter()
        .filter(|child| child.peer_id.as_deref() == Some(peer_id));
    let child = matching.next()?;
    matching.next().is_none().then_some(child.status)
}

/// Newest explicit message per directed pair, then a bounded newest-first tail.
/// Endpoints must exist in the actual roster. Names are intentionally irrelevant.
fn real_edges(swarm: &Swarm, limit: usize) -> Vec<&SwarmMessage> {
    let mut messages = swarm
        .messages
        .iter()
        .filter(|message| {
            swarm.member(&message.from).is_some() && swarm.member(&message.to).is_some()
        })
        .collect::<Vec<_>>();
    messages.sort_by(|a, b| b.sequence.cmp(&a.sequence).then_with(|| b.id.cmp(&a.id)));
    let mut seen = HashSet::new();
    messages
        .into_iter()
        .filter(|message| seen.insert((message.from.as_str(), message.to.as_str())))
        .take(limit.min(MAX_EDGES))
        .collect()
}

fn status_text(status: Option<Status>) -> &'static str {
    match status {
        Some(Status::Running) => "Running",
        Some(Status::Waiting) => "Waiting",
        Some(Status::Done) => "Done",
        Some(Status::Failed) => "Failed",
        Some(Status::Stopped) => "Stopped",
        None => "Unknown",
    }
}

fn last_known(text: impl AsRef<str>, offline: bool) -> String {
    if offline {
        format!("Last known: {}", text.as_ref())
    } else {
        text.as_ref().to_owned()
    }
}

fn status_hue(status: Option<Status>, colors: &Theme) -> Hsla {
    match status {
        Some(Status::Running) => colors.read,
        Some(Status::Waiting) => colors.amber,
        Some(Status::Done) => colors.check,
        Some(Status::Failed) => colors.coral,
        Some(Status::Stopped) | None => colors.muted,
    }
}

fn kind_text(kind: SwarmMessageKind) -> &'static str {
    match kind {
        SwarmMessageKind::Finding => "Finding",
        SwarmMessageKind::Question => "Question",
        SwarmMessageKind::Reply => "Reply",
    }
}

fn delivery_text(status: SwarmMessageStatus) -> &'static str {
    match status {
        SwarmMessageStatus::Queued => "Queued · not retrieved",
        SwarmMessageStatus::Received => "Received · retrieved",
        SwarmMessageStatus::Cancelled => "Cancelled",
    }
}

// Frame-local measured bounds: layout can wrap text at 320 dp without changing
// roster slots or making painted paths depend on guessed card/text heights.
// All child prepaints finish before the background canvas paints these bounds.
type NodeBounds = Rc<RefCell<Vec<Option<Bounds<Pixels>>>>>;

fn measure_node(nodes: &NodeBounds, index: usize) -> impl IntoElement {
    let nodes = Rc::clone(nodes);
    canvas(
        move |bounds, _, _| nodes.borrow_mut()[index] = Some(bounds.dilate(px(1.))),
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

fn stroke_route(points: &[Point<Pixels>], hue: Hsla, dashed: bool, window: &mut Window) {
    let Some(start) = points.first() else {
        return;
    };
    let mut path = PathBuilder::stroke(px(if dashed { 1. } else { 1.7 }));
    if dashed {
        path = path.dash_array(&[px(3.), px(4.)]);
    }
    path.move_to(*start);
    for point in &points[1..] {
        path.line_to(*point);
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, hue);
    }
}

fn arrow(points: &[Point<Pixels>], hue: Hsla, window: &mut Window) {
    if points.len() < 2 {
        return;
    }
    let tip = points[points.len() - 1];
    let previous = points[points.len() - 2];
    let dx = (tip.x - previous.x).as_f32();
    let dy = (tip.y - previous.y).as_f32();
    let length = dx.hypot(dy);
    if length < 1. {
        return;
    }
    let (ux, uy) = (dx / length, dy / length);
    let wing = |side: f32| {
        point(
            tip.x - px(ux * 6.) + px(-uy * side * 3.),
            tip.y - px(uy * 6.) + px(ux * side * 3.),
        )
    };
    stroke_route(&[wing(-1.), tip, wing(1.)], hue, false, window);
}

/// Route through the center gap or the outside rails, never through peer cards.
/// Reciprocal routes use separate ports so their arrowheads remain distinct.
fn exchange_route(
    from: Bounds<Pixels>,
    to: Bounds<Pixels>,
    from_slot: usize,
    to_slot: usize,
    graph: Bounds<Pixels>,
    lane: usize,
) -> Vec<Point<Pixels>> {
    let from_left = from_slot.is_multiple_of(2);
    let to_left = to_slot.is_multiple_of(2);
    let port_offset = if from_slot < to_slot { -6. } else { 6. };
    let from_y = from.center().y + px(port_offset);
    let to_y = to.center().y + px(port_offset);
    if from_slot == to_slot {
        // Even a self-addressed message is drawn only if present in the data.
        let x = if from_left { from.right() } else { from.left() };
        let rail = x + px(if from_left { 12. } else { -12. });
        return vec![
            point(x, from.center().y - px(10.)),
            point(rail, from.center().y - px(10.)),
            point(rail, from.center().y + px(10.)),
            point(x, from.center().y + px(10.)),
        ];
    }
    let (from_x, to_x, rail) = if from_left != to_left {
        (
            if from_left { from.right() } else { from.left() },
            if to_left { to.right() } else { to.left() },
            graph.center().x + px((lane as f32 - 3.5) * 2.),
        )
    } else if from_left {
        (from.left(), to.left(), graph.left() + px(3. + lane as f32))
    } else {
        (
            from.right(),
            to.right(),
            graph.right() - px(3. + lane as f32),
        )
    };
    vec![
        point(from_x, from_y),
        point(rail, from_y),
        point(rail, to_y),
        point(to_x, to_y),
    ]
}

fn rails(nodes: NodeBounds, edges: Vec<(usize, usize)>, colors: Theme) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |graph, _, window, _| {
            let nodes = nodes.borrow();
            window.with_content_mask(Some(ContentMask { bounds: graph }), |window| {
                // The handoff's real roster supplies these quiet, undirected
                // coordinator links. They do not imply messages or prerequisites.
                for (slot, peer) in nodes[1..].iter().enumerate() {
                    let Some(pi) = nodes[0] else { continue };
                    let Some(peer) = peer else { continue };
                    let left = slot % 2 == 0;
                    let x = if left {
                        graph.left() + px(12.)
                    } else {
                        graph.right() - px(12.)
                    };
                    let end_x = if left { peer.left() } else { peer.right() };
                    stroke_route(
                        &[
                            point(pi.center().x, pi.bottom()),
                            point(pi.center().x, pi.bottom() + px(14.)),
                            point(x, pi.bottom() + px(14.)),
                            point(x, peer.center().y),
                            point(end_x, peer.center().y),
                        ],
                        colors.line_strong,
                        true,
                        window,
                    );
                }
                for (lane, &(from_slot, to_slot)) in edges.iter().enumerate() {
                    let (Some(from), Some(to)) = (nodes[from_slot + 1], nodes[to_slot + 1]) else {
                        continue;
                    };
                    let route = exchange_route(from, to, from_slot, to_slot, graph, lane);
                    stroke_route(&route, colors.read, false, window);
                    arrow(&route, colors.read, window);
                }
            });
        },
    )
    .absolute()
    .inset_0()
}

#[allow(clippy::too_many_arguments)]
fn peer_node(
    id: SessionId,
    pick: CrewPick,
    swarm_id: &str,
    member: &SwarmMember,
    status: Option<Status>,
    offline: bool,
    nodes: &NodeBounds,
    slot: usize,
    colors: &Theme,
    cx: &Context<PhoneApp>,
) -> Stateful<Div> {
    let expected_swarm_id = swarm_id.to_owned();
    let peer_id = member.id.clone();
    let (glyph, _) = super::subagents::agent_look(&member.agent, colors);
    let hue = status_hue(status, colors);
    let state = last_known(status_text(status), offline);
    let probe = format!("swarm-map-peer-{swarm_id}-{}", member.id);
    div()
        .id(ElementId::Name(probe.clone().into()))
        .debug_selector({
            let probe = probe.clone();
            move || probe.clone()
        })
        .role(Role::Button)
        .aria_label(format!("{} · {} · {state}", member.agent, member.id))
        .aria_description(if status.is_some() {
            "Inspect this exact peer's conversation"
        } else {
            "No matching child in this snapshot; run status is unknown"
        })
        .relative()
        .flex_1()
        .min_w_0()
        .min_h(px(104.))
        .p(px(8.))
        .flex()
        .flex_col()
        .gap(px(3.))
        .rounded(px(12.))
        .bg(colors.composer)
        .border_1()
        .border_color(hue.opacity(0.65))
        .when(
            status == Some(Status::Waiting) || status.is_none(),
            |node| node.border_dashed(),
        )
        .active(|style| style.bg(colors.selected))
        .child(crate::testing::probe(probe))
        .child(measure_node(nodes, slot + 1))
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(4.))
                .child(icon(glyph, 12., hue).mt(px(2.)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .min_h(px(32.))
                        .line_clamp(2)
                        .text_size(px(12.5))
                        .line_height(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(member.agent.clone()),
                ),
        )
        // Separate identity from the clipped display name. Never abbreviate the
        // peer ID, and expose the full display name to accessibility clients.
        .child(
            ui::mono(member.id.clone(), 10.5)
                .line_height(px(14.))
                .text_color(colors.secondary),
        )
        .child(
            div()
                .text_size(px(11.5))
                .line_height(px(16.))
                .min_h(px(32.))
                .text_color(hue)
                .child(state),
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            this.open_swarm_peer(id, pick, &expected_swarm_id, &peer_id, window, cx);
        }))
}

#[allow(clippy::too_many_arguments)]
fn route_chip(
    id: SessionId,
    pick: CrewPick,
    swarm_id: &str,
    message: &SwarmMessage,
    offline: bool,
    colors: &Theme,
    cx: &Context<PhoneApp>,
) -> Stateful<Div> {
    let expected_swarm_id = swarm_id.to_owned();
    let message_id = message.id.clone();
    let probe = format!("swarm-map-exchange-{swarm_id}-{}", message.id);
    let label = last_known(
        format!("#{} · {}", message.sequence, kind_text(message.kind)),
        offline,
    );
    let delivery = last_known(delivery_text(message.status), offline);
    div()
        .id(ElementId::Name(probe.clone().into()))
        .debug_selector({
            let probe = probe.clone();
            move || probe.clone()
        })
        .relative()
        .child(crate::testing::probe(probe))
        .role(Role::Button)
        .aria_label(format!(
            "{} to {} · {label} · {delivery} · {}",
            message.from, message.to, message.id
        ))
        .aria_description("Inspect the recorded exchange; this does not send a message")
        .min_h(px(48.))
        .min_w_0()
        .px(px(10.))
        .py(px(8.))
        .flex()
        .flex_col()
        .gap(px(3.))
        .rounded(px(12.))
        .bg(colors.chip)
        .border_1()
        .border_color(colors.line)
        .active(|style| style.bg(colors.selected))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    ui::mono(message.from.clone(), 11.)
                        .flex_1()
                        .min_w_0()
                        .text_color(colors.secondary),
                )
                .child(div().flex_none().text_color(colors.read).child("→"))
                .child(
                    ui::mono(message.to.clone(), 11.)
                        .flex_1()
                        .min_w_0()
                        .text_color(colors.secondary),
                ),
        )
        .child(ui::label(label, colors))
        .child(ui::hint(delivery, colors).text_size(px(11.5)))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.open_swarm_exchange(id, pick, &expected_swarm_id, &message_id, cx);
        }))
}

/// Map tab content; the owning screen supplies its app bar, tabs and scrolling.
/// No stop/send controls, animated packets, inferred exchanges or fake peers.
pub(super) fn map(
    id: SessionId,
    pick: CrewPick,
    handoff: &Handoff,
    offline: bool,
    colors: &Theme,
    cx: &Context<PhoneApp>,
) -> Div {
    let Some(swarm) = handoff.swarm.as_ref() else {
        return div().child(ui::hint("No swarm roster in this handoff.", colors));
    };
    if !(2..=MAX_PEERS).contains(&swarm.members.len()) {
        return div().child(ui::hint(
            "Map unavailable: the roster must contain 2–8 actual peers.",
            colors,
        ));
    }
    let slots = roster_slots(swarm);
    let edges = real_edges(swarm, MAX_EDGES);
    let routes = edges
        .iter()
        .filter_map(|message| {
            Some((
                slots
                    .iter()
                    .position(|(peer, _, _)| *peer == message.from)?,
                slots.iter().position(|(peer, _, _)| *peer == message.to)?,
            ))
        })
        .collect();
    let nodes: NodeBounds = Rc::new(RefCell::new(vec![None; swarm.members.len() + 1]));
    let pi_probe = format!("swarm-map-pi-{}", swarm.id);
    let pi = div()
        .id(ElementId::Name(pi_probe.clone().into()))
        .debug_selector({
            let probe = pi_probe.clone();
            move || probe.clone()
        })
        .role(Role::Button)
        .aria_label("Pi coordinator · open parent thread")
        .aria_description("Navigation only; opening the thread does not steer or send input")
        .relative()
        .w_full()
        .max_w(px(168.))
        .min_h(px(80.))
        .p(px(10.))
        .rounded(px(12.))
        .bg(colors.panel)
        .border_1()
        .border_color(colors.accent)
        .active(|style| style.bg(colors.selected))
        .child(crate::testing::probe(pi_probe))
        .child(measure_node(&nodes, 0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(icon("pi", 14., colors.accent))
                .child(ui::label("Pi", colors)),
        )
        .child(ui::hint(last_known("Coordinator", offline), colors).text_size(px(11.5)))
        .child(ui::hint("Open parent thread", colors).text_size(px(11.5)))
        .on_click(cx.listener(move |this, _, window, cx| {
            this.swarm_parent_thread(id, window, cx);
        }));
    let graph = div()
        .relative()
        .w_full()
        .child(rails(Rc::clone(&nodes), routes, *colors))
        .child(
            div()
                .px(px(16.))
                .flex()
                .flex_col()
                .gap(px(ROW_GAP))
                .child(div().flex().justify_center().child(pi))
                .children(swarm.members.chunks(2).enumerate().map(|(row, members)| {
                    div()
                        .w_full()
                        .flex()
                        .items_stretch()
                        .gap(px(COLUMN_GAP))
                        .children(members.iter().enumerate().map(|(column, member)| {
                            peer_node(
                                id,
                                pick,
                                &swarm.id,
                                member,
                                child_status(handoff, &member.id),
                                offline,
                                &nodes,
                                row * 2 + column,
                                colors,
                                cx,
                            )
                        }))
                        // An empty layout cell is not a node or a waiting peer.
                        .when(members.len() == 1, |row| {
                            row.child(div().flex_1().min_w_0())
                        })
                })),
        );
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(12.))
        .p(px(12.))
        .rounded(px(16.))
        .bg(colors.composer)
        .border_1()
        .border_color(colors.line)
        .child(ui::label(
            last_known(
                format!("Peer map · {} peers + Pi", swarm.members.len()),
                offline,
            ),
            colors,
        ))
        .child(ui::hint(
            last_known(
                if swarm.closed {
                    "Closed swarm"
                } else {
                    "Open swarm"
                },
                offline,
            ),
            colors,
        ))
        .child(graph)
        .child(
            ui::hint(
                "Dashed gray: coordinator roster links, not dependencies.",
                colors,
            )
            .text_size(px(11.5)),
        )
        .child(
            ui::hint(
                "Blue arrows: explicit peer exchanges, sender → recipient.",
                colors,
            )
            .text_size(px(11.5)),
        )
        .when(
            swarm
                .members
                .iter()
                .any(|member| child_status(handoff, &member.id).is_none()),
            |body| {
                body.child(
                    ui::hint(
                        "Unknown: no child matches this exact peer ID in the snapshot.",
                        colors,
                    )
                    .text_size(px(11.5)),
                )
            },
        )
        .child(ui::hint("Up to 32 recent broker events, not complete history. At most 8 latest directed pairs are drawn.", colors).text_size(px(11.5)))
        .child(ui::label(
            last_known(
                format!(
                    "Latest per direction · {} links / {} snapshot events",
                    edges.len(),
                    swarm.messages.len()
                ),
                offline,
            ),
            colors,
        ))
        .when(edges.is_empty(), |body| {
            body.child(ui::hint(
                last_known("No peer exchanges in this snapshot.", offline),
                colors,
            ))
        })
        .children(
            edges
                .into_iter()
                .map(|message| route_chip(id, pick, &swarm.id, message, offline, colors, cx)),
        )
        .child(
            ui::hint(
                "Received means retrieved by the recipient, not agreement or successful execution.",
                colors,
            )
            .text_size(px(11.5)),
        )
        .when(offline, |body| {
            body.child(ui::hint(
                "Last known snapshot · inspect only. Delivery may have changed.",
                colors,
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn swarm(ids: &[&str]) -> Swarm {
        Swarm {
            id: "crew-exact".into(),
            members: ids
                .iter()
                .map(|id| SwarmMember {
                    id: (*id).into(),
                    agent: "same display name".into(),
                    task: "Read only".into(),
                })
                .collect(),
            messages: Vec::new(),
            closed: false,
        }
    }

    fn message(id: &str, sequence: u64, from: &str, to: &str) -> SwarmMessage {
        SwarmMessage {
            id: id.into(),
            sequence,
            from: from.into(),
            to: to.into(),
            kind: SwarmMessageKind::Finding,
            text: "Real finding".into(),
            reply_to: None,
            status: SwarmMessageStatus::Queued,
            created_at: 0,
            received_at: None,
        }
    }

    #[test]
    fn slots_are_roster_order_not_name_or_message_activity() {
        let mut crew = swarm(&["p-z", "p-a", "p-3", "p-4", "p-5", "p-6", "p-7", "p-8"]);
        let before = roster_slots(&crew)
            .into_iter()
            .map(|(id, row, col)| (id.to_owned(), row, col))
            .collect::<Vec<_>>();
        crew.messages.push(message("new", 9, "p-8", "p-z"));
        crew.closed = true;
        crew.members[0].agent = "An extremely long renamed display name".into();
        let after = roster_slots(&crew)
            .into_iter()
            .map(|(id, row, col)| (id.to_owned(), row, col))
            .collect::<Vec<_>>();
        assert_eq!(before, after);
        assert_eq!(before[0], ("p-z".into(), 0, 0));
        assert_eq!(before[1], ("p-a".into(), 0, 1));
        assert_eq!(before[7], ("p-8".into(), 3, 1));
        for count in 2..=MAX_PEERS {
            assert_eq!(
                roster_slots(&swarm(&["a", "b", "c", "d", "e", "f", "g", "h"][..count])).len(),
                count
            );
        }
    }

    #[test]
    fn duplicate_names_bind_exact_ids_not_child_order() {
        let mut handoff = Handoff::from_args(&json!({"tasks": [
            {"agent": "same display name", "task": "First"},
            {"agent": "same display name", "task": "Second"}
        ]}));
        handoff.swarm = Some(swarm(&["p-1", "p-2"]));
        handoff.subagents[0].peer_id = Some("p-2".into());
        handoff.subagents[0].status = Status::Done;
        handoff.subagents[1].peer_id = Some("p-1".into());
        handoff.subagents[1].status = Status::Running;
        assert_eq!(child_status(&handoff, "p-1"), Some(Status::Running));
        assert_eq!(child_status(&handoff, "p-2"), Some(Status::Done));
        assert_eq!(child_status(&handoff, "p-3"), None);
        assert_eq!(child_status(&handoff, "same display name"), None);
        handoff.subagents.reverse();
        assert_eq!(child_status(&handoff, "p-1"), Some(Status::Running));
        handoff.subagents[1].peer_id = Some("p-1".into());
        assert_eq!(child_status(&handoff, "p-1"), None);
    }

    #[test]
    fn edges_are_latest_unique_directed_real_pairs_and_bounded() {
        let mut crew = swarm(&["p-1", "p-2", "p-3", "p-4"]);
        crew.messages = vec![
            message("old", 1, "p-1", "p-2"),
            message("reverse", 2, "p-2", "p-1"),
            message("new", 3, "p-1", "p-2"),
            message("unknown", 999, "not-a-peer", "p-1"),
        ];
        assert_eq!(
            real_edges(&crew, MAX_EDGES)
                .iter()
                .map(|edge| edge.id.as_str())
                .collect::<Vec<_>>(),
            vec!["new", "reverse"]
        );
        assert_eq!(real_edges(&crew, 1)[0].id, "new");
        assert!(real_edges(&crew, 0).is_empty());
        for from in 1..=4 {
            for to in 1..=4 {
                crew.messages.push(message(
                    &format!("{from}-{to}"),
                    10 + from * 4 + to,
                    &format!("p-{from}"),
                    &format!("p-{to}"),
                ));
            }
        }
        crew.messages.reverse(); // Projection order is not sequence order.
        let selected = real_edges(&crew, usize::MAX);
        assert_eq!(selected.len(), MAX_EDGES);
        assert!(
            selected
                .windows(2)
                .all(|pair| pair[0].sequence >= pair[1].sequence)
        );
        assert_eq!(
            selected
                .iter()
                .map(|edge| (&edge.from, &edge.to))
                .collect::<HashSet<_>>()
                .len(),
            MAX_EDGES
        );
        assert!(
            selected
                .iter()
                .all(|edge| crew.messages.iter().any(|real| std::ptr::eq(*edge, real)))
        );
    }
}
