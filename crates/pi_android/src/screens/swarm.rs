//! Native, session-owned swarm observation. Unsupported execution features are
//! described, never fabricated from peer questions, tool output or sample data.

use super::{scroll_area, subagents::agent_look, swarm_map};
use crate::{
    app::{CrewPick, PhoneApp, Pick, Route},
    model::{SessionId, clock_at},
    theme::{Theme, theme},
    ui::{self, icon},
};
use gpui::{
    AnyElement, Context, Div, ElementId, FontWeight, Hsla, Role, Window, div, prelude::*, px,
};
use pi_core::subagent::{
    Handoff, Status, Subagent, Swarm, SwarmMessage, SwarmMessageKind, SwarmMessageStatus,
};
use std::{
    collections::{BTreeMap, HashSet},
    time::Duration,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SwarmTab {
    #[default]
    Map,
    Exchanges,
    Agents,
    Results,
    Groups,
    Capabilities,
    Decisions,
    Artifacts,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ExchangeFilter {
    #[default]
    All,
    Findings,
    Questions,
    Replies,
}

impl ExchangeFilter {
    fn matches(self, kind: SwarmMessageKind) -> bool {
        match self {
            Self::All => true,
            Self::Findings => kind == SwarmMessageKind::Finding,
            Self::Questions => kind == SwarmMessageKind::Question,
            Self::Replies => kind == SwarmMessageKind::Reply,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SwarmScreenState {
    namespace: Option<(String, Option<String>)>,
    pub(crate) swarm_id: String,
    pub(crate) tab: SwarmTab,
    pub(crate) filter: ExchangeFilter,
    pub(crate) peer: Option<String>,
    pub(crate) group: Option<String>,
    pub(crate) open_message: Option<String>,
    pub(crate) stop_peer: Option<String>,
    stop_requested: HashSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SwarmPeerBinding {
    pub(crate) computer: String,
    pub(crate) root_key: Option<String>,
    pub(crate) id: SessionId,
    pub(crate) pick: CrewPick,
    pub(crate) swarm_id: String,
    pub(crate) peer_id: String,
}

// A late Send may not silently become a new turn or steer a replacement run.
pub(crate) fn input_target_current(
    expected: Option<&crate::live::SubagentInputTarget>,
    current: Option<&crate::live::SubagentInputTarget>,
) -> bool {
    expected.is_some() && expected == current
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Observation {
    Observed,
    Sample,
    Offline,
}

pub(super) fn node<'a>(handoff: &'a Handoff, peer: &str) -> Option<(usize, &'a Subagent)> {
    handoff.swarm.as_ref()?.member(peer)?;
    let mut matching = handoff
        .subagents
        .iter()
        .enumerate()
        .filter(|(_, child)| child.peer_id.as_deref() == Some(peer));
    let child = matching.next()?;
    matching.next().is_none().then_some(child)
}

fn may_request_stop(handoff: &Handoff, peer: &str, observation: Observation) -> bool {
    observation == Observation::Observed
        && handoff.swarm.as_ref().is_some_and(|swarm| !swarm.closed)
        && node(handoff, peer).is_some_and(|(_, child)| {
            child.status == Status::Running && child.conversation_id.is_some()
        })
}

fn may_request_conversation_stop(
    handoff: &Handoff,
    peer: &str,
    observation: Observation,
    current: Option<&crate::live::SubagentInputTarget>,
) -> bool {
    observation == Observation::Observed
        && node(handoff, peer).is_some_and(|(_, child)| child.conversation_id.is_some())
        && (may_request_stop(handoff, peer, observation)
            || matches!(
                current,
                Some(crate::live::SubagentInputTarget::Steer { .. })
            ))
}

fn status(status: Status) -> &'static str {
    match status {
        Status::Waiting => "Waiting",
        Status::Running => "Working",
        Status::Done => "Done",
        Status::Failed => "Failed",
        Status::Stopped => "Stopped",
    }
}

fn status_hue(status: Status, colors: &Theme) -> Hsla {
    match status {
        Status::Running => colors.read,
        Status::Waiting => colors.amber,
        Status::Done => colors.green,
        Status::Failed => colors.coral,
        Status::Stopped => colors.muted,
    }
}

fn kind(kind: SwarmMessageKind) -> &'static str {
    match kind {
        SwarmMessageKind::Finding => "Finding",
        SwarmMessageKind::Question => "Question",
        SwarmMessageKind::Reply => "Reply",
    }
}

fn delivery(status: SwarmMessageStatus) -> &'static str {
    match status {
        SwarmMessageStatus::Queued => "Queued · not retrieved",
        SwarmMessageStatus::Received => "Received · retrieved by recipient",
        SwarmMessageStatus::Cancelled => "Cancelled · not retrieved",
    }
}

fn time(at: u64) -> String {
    format!(
        "{}:{:02}",
        clock_at(Duration::from_millis(at)),
        at / 1000 % 60
    )
}

fn dated_time(at: u64) -> String {
    i64::try_from(at)
        .ok()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map_or_else(
            || "Time unavailable".to_owned(),
            |at| at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        )
}

fn cost(value: f64) -> String {
    if value > 0. && value < 0.005 {
        "<$0.01".into()
    } else {
        format!("${value:.2}")
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_owned()
}

fn preview(text: &str, count: usize) -> String {
    let mut chars = text.chars();
    let mut shown = chars.by_ref().take(count).collect::<String>();
    if chars.next().is_some() {
        shown.push('…');
    }
    shown
}

fn peer_label(swarm: &Swarm, peer: &str) -> String {
    swarm.member(peer).map_or_else(
        || format!("Unknown peer · {peer}"),
        |member| format!("{} · {peer}", member.agent),
    )
}

fn messages<'a>(
    swarm: &'a Swarm,
    filter: ExchangeFilter,
    peer: Option<&str>,
) -> Vec<&'a SwarmMessage> {
    let mut messages = swarm
        .messages
        .iter()
        .filter(|message| {
            filter.matches(message.kind)
                && peer.is_none_or(|peer| message.from == peer || message.to == peer)
        })
        .collect::<Vec<_>>();
    messages.sort_by_key(|message| message.sequence);
    messages
}

fn counts(handoff: &Handoff, swarm: &Swarm) -> String {
    let states = [
        (Status::Running, "working"),
        (Status::Waiting, "waiting"),
        (Status::Done, "done"),
        (Status::Failed, "failed"),
        (Status::Stopped, "stopped"),
    ];
    let mut parts = states
        .into_iter()
        .filter_map(|(state, label)| {
            let count = swarm
                .members
                .iter()
                .filter(|member| {
                    node(handoff, &member.id).is_some_and(|(_, child)| child.status == state)
                })
                .count();
            (count > 0).then(|| format!("{count} {label}"))
        })
        .collect::<Vec<_>>();
    let unknown = swarm
        .members
        .iter()
        .filter(|member| node(handoff, &member.id).is_none())
        .count();
    if unknown > 0 {
        parts.push(format!("{unknown} without status"));
    }
    parts.join(" · ")
}

fn card(title: &str, text: &str, colors: &Theme) -> Div {
    ui::card(colors)
        .p(px(16.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.to_owned()),
        )
        .child(ui::hint(text.to_owned(), colors))
}

fn detail(label: &str, text: String, colors: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(ui::label(label.to_owned(), colors))
        .child(
            div()
                .text_size(px(13.))
                .line_height(px(19.))
                .text_color(colors.secondary)
                .child(text),
        )
}

impl PhoneApp {
    fn swarm_observation(&self, id: SessionId) -> Observation {
        match self.store.as_ref() {
            Some(store) if !store.computer.connected => Observation::Offline,
            Some(store) if store.is_sample() => Observation::Sample,
            Some(store)
                if !self.is_reconnecting()
                    && store.live.as_ref().is_some_and(|live| {
                        !live.connection.is_closed() && live.session_current(id)
                    }) =>
            {
                Observation::Observed
            }
            _ => Observation::Offline,
        }
    }

    pub(crate) fn swarm_namespace(&self, id: SessionId) -> Option<(String, Option<String>)> {
        let store = self.store.as_ref()?;
        store.session(id)?;
        Some((
            store.computer.address.clone(),
            store
                .live
                .as_ref()
                .and_then(|live| live.target(id))
                .map(|target| target.key),
        ))
    }

    fn raw_swarm(&self, id: SessionId, pick: CrewPick) -> Option<&Handoff> {
        let handoff = self
            .store
            .as_ref()?
            .session(id)?
            .turns
            .get(pick.turn)?
            .handoffs
            .get(pick.handoff)?;
        handoff.swarm.as_ref()?;
        Some(handoff)
    }

    fn bound_swarm(&self, id: SessionId, pick: CrewPick, expected: &str) -> Option<&Handoff> {
        let handoff = self.raw_swarm(id, pick)?;
        (handoff.swarm.as_ref()?.id == expected
            && self.swarm_views.get(&(id, pick))?.swarm_id == expected
            && self.swarm_views.get(&(id, pick))?.namespace == self.swarm_namespace(id))
        .then_some(handoff)
    }

    /// Called only from an actual handoff; indices alone never authorize a replacement crew.
    pub(crate) fn open_swarm(
        &mut self,
        id: SessionId,
        pick: CrewPick,
        expected: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.raw_swarm(id, pick).is_some_and(|handoff| {
            handoff
                .swarm
                .as_ref()
                .is_some_and(|swarm| swarm.id == expected)
        }) {
            self.notify_user("This swarm is no longer in the session.", cx);
            return;
        }
        let namespace = self.swarm_namespace(id);
        let state = self.swarm_views.entry((id, pick)).or_default();
        if state.swarm_id != expected || state.namespace != namespace {
            *state = SwarmScreenState {
                namespace,
                swarm_id: expected.to_owned(),
                ..Default::default()
            };
        }
        if let Some(index) = self
            .routes
            .iter()
            .rposition(|route| *route == Route::Swarm(id, pick))
        {
            while self.routes.len() > index + 1 {
                self.back(window, cx);
            }
        } else {
            if let Route::Subagent(parent, _) | Route::SwarmPeer(parent, _, _) = self.route() {
                self.follow_subagent(parent, None);
            }
            self.push(Route::Swarm(id, pick), window, cx);
        }
        cx.notify();
    }

    pub(crate) fn open_swarm_peer(
        &mut self,
        id: SessionId,
        pick: CrewPick,
        expected: &str,
        peer: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.bound_swarm(id, pick, expected).is_none() {
            self.notify_user("This exact swarm is no longer in the snapshot.", cx);
            return;
        }
        self.open_swarm_peer_from_handoff(id, pick, expected, peer, window, cx);
    }

    pub(crate) fn open_swarm_peer_from_handoff(
        &mut self,
        id: SessionId,
        pick: CrewPick,
        expected: &str,
        peer: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let child = self
            .raw_swarm(id, pick)
            .filter(|handoff| {
                handoff
                    .swarm
                    .as_ref()
                    .is_some_and(|swarm| swarm.id == expected)
            })
            .and_then(|handoff| node(handoff, peer))
            .map(|(index, child)| (index, child.conversation_id.clone()));
        let Some((index, conversation)) = child else {
            self.notify_user(
                "This exact peer has no uniquely matching child in the snapshot.",
                cx,
            );
            return;
        };
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let computer = store.computer.address.clone();
        let root_key = store
            .live
            .as_ref()
            .and_then(|live| live.target(id))
            .map(|target| target.key);
        let token = self.next_swarm_peer;
        self.next_swarm_peer += 1;
        self.swarm_peer_bindings.insert(
            token,
            SwarmPeerBinding {
                computer,
                root_key,
                id,
                pick,
                swarm_id: expected.to_owned(),
                peer_id: peer.to_owned(),
            },
        );
        self.follow_subagent(id, conversation);
        self.push(
            Route::SwarmPeer(
                id,
                Pick {
                    turn: pick.turn,
                    handoff: pick.handoff,
                    index,
                },
                token,
            ),
            window,
            cx,
        );
    }

    pub(crate) fn bound_swarm_peer(&self, binding: &SwarmPeerBinding) -> Option<&Subagent> {
        let store = self.store.as_ref()?;
        if store.computer.address != binding.computer
            || store
                .live
                .as_ref()
                .and_then(|live| live.target(binding.id))
                .map(|target| target.key)
                != binding.root_key
        {
            return None;
        }
        let handoff = self.raw_swarm(binding.id, binding.pick)?;
        if handoff.swarm.as_ref()?.id != binding.swarm_id {
            return None;
        }
        node(handoff, &binding.peer_id).map(|(_, child)| child)
    }

    pub(crate) fn can_send_swarm_peer(
        &self,
        binding: &SwarmPeerBinding,
        conversation: &str,
    ) -> bool {
        matches!(self.route(), Route::SwarmPeer(id, _, token) if id == binding.id && self.swarm_peer_bindings.get(&token) == Some(binding))
            && self.swarm_observation(binding.id) == Observation::Observed
            && self
                .bound_swarm_peer(binding)
                .is_some_and(|child| child.conversation_id.as_deref() == Some(conversation))
    }

    pub(crate) fn swarm_peer_stop_available(
        &self,
        id: SessionId,
        pick: CrewPick,
        expected: &str,
        peer: &str,
    ) -> bool {
        if self.swarm_observation(id) != Observation::Observed {
            return false;
        }
        let Some(handoff) = self.raw_swarm(id, pick).filter(|handoff| {
            handoff
                .swarm
                .as_ref()
                .is_some_and(|swarm| swarm.id == expected)
        }) else {
            return false;
        };
        let Some((_, child)) = node(handoff, peer) else {
            return false;
        };
        let current = child.conversation_id.as_deref().and_then(|conversation| {
            self.store
                .as_ref()
                .and_then(|store| store.live.as_ref())
                .and_then(|live| live.subagent_input_target(id, conversation).ok())
        });
        may_request_conversation_stop(handoff, peer, Observation::Observed, current.as_ref())
    }

    pub(crate) fn request_swarm_peer_stop(
        &mut self,
        id: SessionId,
        pick: CrewPick,
        expected: &str,
        peer: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.swarm_peer_stop_available(id, pick, expected, peer) {
            self.notify_user(
                "A current running peer is required for Stop. Nothing was queued.",
                cx,
            );
            return;
        }
        self.open_swarm(id, pick, expected, window, cx);
        if let Some(state) = self.swarm_views.get_mut(&(id, pick)) {
            state.tab = SwarmTab::Agents;
            state.stop_peer = Some(peer.to_owned());
        }
        cx.notify();
    }

    pub(crate) fn open_swarm_exchange(
        &mut self,
        id: SessionId,
        pick: CrewPick,
        expected: &str,
        message: &str,
        cx: &mut Context<Self>,
    ) {
        let present = self
            .bound_swarm(id, pick, expected)
            .and_then(|handoff| handoff.swarm.as_ref())
            .is_some_and(|swarm| swarm.messages.iter().any(|item| item.id == message));
        if !present {
            self.notify_user("This exchange is outside the current snapshot.", cx);
            return;
        }
        if let Some(state) = self.swarm_views.get_mut(&(id, pick)) {
            state.tab = SwarmTab::Exchanges;
            state.filter = ExchangeFilter::All;
            state.peer = None;
            state.open_message = Some(message.to_owned());
        }
        self.scroll(Route::Swarm(id, pick))
            .set_offset(gpui::Point::default());
        cx.notify();
    }

    pub(crate) fn swarm_parent_thread(
        &mut self,
        id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self
            .routes
            .iter()
            .rposition(|route| *route == Route::Thread(id))
        {
            while self.routes.len() > index + 1 {
                self.back(window, cx);
            }
        } else {
            self.push_session(id, window, cx);
        }
    }

    fn swarm_tab(&mut self, id: SessionId, pick: CrewPick, tab: SwarmTab, cx: &mut Context<Self>) {
        if let Some(state) = self.swarm_views.get_mut(&(id, pick)) {
            state.tab = tab;
            state.stop_peer = None;
        }
        self.scroll(Route::Swarm(id, pick))
            .set_offset(gpui::Point::default());
        cx.notify();
    }

    fn confirm_swarm_stop(
        &mut self,
        id: SessionId,
        pick: CrewPick,
        expected: &str,
        peer: &str,
        cx: &mut Context<Self>,
    ) {
        if !self.swarm_views.get(&(id, pick)).is_some_and(|state| {
            state.swarm_id == expected && state.stop_peer.as_deref() == Some(peer)
        }) {
            self.notify_user(
                "Confirm this exact peer before requesting Stop. Nothing was sent.",
                cx,
            );
            return;
        }
        if self.swarm_observation(id) != Observation::Observed {
            self.notify_user("Reconnect before requesting Stop. Nothing was sent.", cx);
            return;
        }
        let conversation = self
            .bound_swarm(id, pick, expected)
            .filter(|_| self.swarm_peer_stop_available(id, pick, expected, peer))
            .and_then(|handoff| node(handoff, peer))
            .and_then(|(_, child)| child.conversation_id.clone());
        let Some(conversation) = conversation else {
            self.notify_user(
                "This peer has ended or its current conversation is unavailable. Nothing was sent.",
                cx,
            );
            return;
        };
        let result = self
            .store
            .as_mut()
            .and_then(|store| store.live.as_mut())
            .ok_or_else(|| "Reconnect before requesting Stop.".to_owned())
            .and_then(|live| live.stop_subagent(id, &conversation));
        match result {
            Ok(()) => {
                if let Some(state) = self.swarm_views.get_mut(&(id, pick)) {
                    state.stop_peer = None;
                    state.stop_requested.insert(peer.to_owned());
                }
                self.notify_user(
                    "Stop requested for this peer only. Waiting for the computer's state update.",
                    cx,
                );
            }
            Err(error) => self.notify_user(error, cx),
        }
        cx.notify();
    }

    fn swarm_tabs(
        &self,
        id: SessionId,
        pick: CrewPick,
        active: SwarmTab,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        div()
            .flex()
            .flex_none()
            .px(px(12.))
            .border_b_1()
            .border_color(colors.line)
            .children(
                [
                    (SwarmTab::Map, "Map"),
                    (SwarmTab::Exchanges, "Exchanges"),
                    (SwarmTab::Agents, "Agents"),
                    (SwarmTab::Results, "Results"),
                ]
                .into_iter()
                .map(|(tab, label)| {
                    let selected =
                        active == tab || (active == SwarmTab::Groups && tab == SwarmTab::Agents);
                    let selector = format!("swarm-tab-{}", label.to_lowercase());
                    div()
                        .id(ElementId::Name(selector.clone().into()))
                        .relative()
                        .child(crate::testing::probe(selector.clone()))
                        .debug_selector(move || selector.clone())
                        .role(Role::Button)
                        .aria_label(format!("Swarm {label}"))
                        .flex_1()
                        .min_w_0()
                        .h(px(48.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if selected {
                            colors.accent
                        } else {
                            colors.muted
                        })
                        .border_b(px(2.))
                        .border_color(if selected {
                            colors.accent
                        } else {
                            gpui::transparent_black()
                        })
                        .active(|style| style.bg(colors.selected))
                        .child(label)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.swarm_tab(id, pick, tab, cx)),
                        )
                }),
            )
    }

    fn observation_banner(&self, id: SessionId, observation: Observation, colors: &Theme) -> Div {
        let at = self
            .store
            .as_ref()
            .and_then(|store| store.observed_at.get(&id))
            .copied();
        let (title, body, hue) = match observation {
            Observation::Observed => ("Observed on the computer", at.map_or_else(|| "Current session snapshot. Updates arrive over SSH.".into(), |at| format!("Last observed {} · phone receipt time.", dated_time(at))), colors.read),
            Observation::Sample => ("Sample snapshot", "No agents run and no commands are sent by this preview.".into(), colors.muted),
            Observation::Offline => (if self.is_reconnecting() || self.store.as_ref().is_none_or(|store| !store.computer.connected || store.live.as_ref().is_some_and(|live| live.connection.is_closed())) { "Connection lost · current state unknown" } else { "Session reconnecting · current state unknown" }, at.map_or_else(|| "Last observation time unavailable. The computer may still be working.".into(), |at| format!("Last observed {} · phone receipt time. The computer may still be working.", dated_time(at))), colors.amber),
        };
        div()
            .debug_selector(|| "swarm-observation".into())
            .p(px(12.))
            .rounded(px(12.))
            .bg(colors.tint(hue))
            .border_1()
            .border_color(hue.opacity(0.3))
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(ui::label(title, colors).text_color(hue))
            .child(ui::hint(body, colors))
    }

    #[allow(clippy::too_many_arguments)]
    fn swarm_roster(
        &self,
        id: SessionId,
        pick: CrewPick,
        handoff: &Handoff,
        state: &SwarmScreenState,
        observation: Observation,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let swarm = handoff.swarm.as_ref().unwrap();
        div().flex().flex_col().gap(px(10.))
            .children(state.group.as_ref().map(|group| ui::label(format!("Group · {group}"), colors)))
            .children(swarm.members.iter().filter(|member| state.group.as_ref().is_none_or(|group| &member.agent == group)).map(|member| {
                let peer = member.id.clone();
                let expected = swarm.id.clone();
                let child = node(handoff, &peer).map(|(_, child)| child);
                let (glyph, hue) = agent_look(&member.agent, colors);
                let label = child.map_or_else(|| "Status unavailable".into(), |child| {
                    let label = if state.stop_requested.contains(&peer) && child.status == Status::Running { "Stop requested · verify current state" } else { status(child.status) };
                    if observation == Observation::Offline { format!("Last known: {label}") } else { label.to_owned() }
                });
                let can_stop = may_request_stop(handoff, &peer, observation);
                let selector = format!("swarm-agent-{peer}");
                let open_peer = peer.clone();
                let open_expected = expected.clone();
                ui::card(colors).flex().flex_col()
                    .child(div().id(ElementId::Name(selector.clone().into())).relative().child(crate::testing::probe(selector.clone())).debug_selector(move || selector.clone())
                        .role(Role::Button).aria_label(format!("Inspect {}", peer_label(swarm, &peer)))
                        .min_h(px(80.)).p(px(14.)).flex().items_start().gap(px(10.))
                        .active(|style| style.bg(colors.selected))
                        .child(div().size(px(34.)).flex_none().rounded(px(10.)).bg(colors.tint(hue)).flex().items_center().justify_center().child(icon(glyph, 17., hue)))
                        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(4.))
                            .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(peer_label(swarm, &peer)))
                            .child(div().text_size(px(13.)).line_height(px(18.)).text_color(colors.secondary).child(preview(&first_line(child.map_or(member.task.as_str(), |child| child.task.as_str())), 220)))
                            .child(ui::hint(label, colors).text_color(child.map_or(colors.muted, |child| status_hue(child.status, colors))))
                            .children(child.and_then(|child| child.model.as_ref()).map(|model| ui::hint(model.clone(), colors).text_size(px(11.5)))))
                        .child(icon("chev_r", 16., colors.faint))
                        .on_click(cx.listener(move |this, _, window, cx| this.open_swarm_peer(id, pick, &open_expected, &open_peer, window, cx))))
                    .when(can_stop, |card| card.child(div().px(px(10.)).pb(px(8.)).child(ui::button(format!("swarm-stop-{peer}"), ui::Button::Quiet, Some("stop"), if state.stop_requested.contains(&peer) { "Retry Stop request…" } else { "Stop this peer…" }, false, colors)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.bound_swarm(id, pick, &expected).is_some() {
                                if let Some(state) = this.swarm_views.get_mut(&(id, pick)) { state.stop_peer = Some(peer.clone()); }
                                this.scroll(Route::Swarm(id, pick)).set_offset(gpui::Point::default());
                                cx.notify();
                            }
                        })))))
            }))
            .child(ui::hint("Tap a peer to inspect its brief and full conversation. Questions between peers are not requests for your permission.", colors))
    }

    #[allow(clippy::too_many_arguments)]
    fn swarm_exchange_list(
        &self,
        id: SessionId,
        pick: CrewPick,
        swarm: &Swarm,
        state: &SwarmScreenState,
        observation: Observation,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let visible = messages(swarm, state.filter, state.peer.as_deref());
        let mut body = div().flex().flex_col().gap(px(12.))
            .child(div().flex().flex_wrap().gap(px(4.)).children([(ExchangeFilter::All, "All"), (ExchangeFilter::Findings, "Findings"), (ExchangeFilter::Questions, "Questions"), (ExchangeFilter::Replies, "Replies")].into_iter().map(|(filter, label)| {
                let selected = state.filter == filter;
                div().id(format!("swarm-filter-{}", label.to_lowercase())).relative().child(crate::testing::probe(format!("swarm-filter-{}", label.to_lowercase()))).debug_selector(move || format!("swarm-filter-{}", label.to_lowercase()))
                    .role(Role::Button).aria_label(format!("Filter exchanges: {label}"))
                    .min_h(px(48.)).px(px(10.)).rounded_full().flex().items_center().text_size(px(12.5))
                    .bg(if selected { colors.panel } else { gpui::transparent_black() }).text_color(if selected { colors.text } else { colors.muted })
                    .font_weight(if selected { FontWeight::SEMIBOLD } else { FontWeight::NORMAL }).active(|style| style.bg(colors.selected))
                    .child(label).on_click(cx.listener(move |this, _, _, cx| { if let Some(state) = this.swarm_views.get_mut(&(id, pick)) { state.filter = filter; state.open_message = None; } cx.notify(); }))
            })))
            .child(ui::hint(format!("{} matching exchanges in the current snapshot. Up to 32 recent message events; older exchanges and full message bodies are not fetched here.", visible.len()), colors))
            .child(div().flex().flex_wrap().gap(px(4.)).child(ui::button("swarm-peer-filter-all", ui::Button::Quiet, None, if state.peer.is_none() { "All peers ✓" } else { "All peers" }, false, colors)
                .on_click(cx.listener(move |this, _, _, cx| { if let Some(state) = this.swarm_views.get_mut(&(id, pick)) { state.peer = None; state.open_message = None; } cx.notify(); })))
                .children(swarm.members.iter().map(|member| {
                    let peer = member.id.clone();
                    let label = format!("{}{}", peer, if state.peer.as_deref() == Some(peer.as_str()) { " ✓" } else { "" });
                    ui::button(format!("swarm-peer-filter-{peer}"), ui::Button::Quiet, None, label, false, colors).px(px(12.)).text_size(px(12.))
                        .on_click(cx.listener(move |this, _, _, cx| { if let Some(state) = this.swarm_views.get_mut(&(id, pick)) { state.peer = Some(peer.clone()); state.open_message = None; } cx.notify(); }))
                })))
            .when(visible.is_empty(), |body| body.child(card("No matching exchanges", "There are no matching messages in this snapshot. This does not imply that no earlier exchanges occurred.", colors)));
        // An inspected exchange stays at the top so map/reply navigation never
        // needs to guess a scroll offset in a varying-height list.
        if let Some(message) = state
            .open_message
            .as_deref()
            .and_then(|id| swarm.messages.iter().find(|message| message.id == id))
        {
            body = body.child(self.swarm_message_card(
                id,
                pick,
                swarm,
                message,
                true,
                observation,
                colors,
                cx,
            ));
        } else if state.open_message.is_some() {
            body = body.child(card("Exchange no longer in the snapshot", "New events moved this exchange out of the bounded history. Its original peer conversation may still contain it.", colors));
        }
        body.children(visible.into_iter().filter(|message| state.open_message.as_deref() != Some(message.id.as_str())).map(|message| {
            self.swarm_message_card(id, pick, swarm, message, false, observation, colors, cx)
        })).child(ui::hint("Received means explicit recipient tool retrieval—not agreement, an answer, successful execution or guaranteed effects.", colors))
    }

    #[allow(clippy::too_many_arguments)]
    fn swarm_message_card(
        &self,
        id: SessionId,
        pick: CrewPick,
        swarm: &Swarm,
        message: &SwarmMessage,
        open: bool,
        observation: Observation,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let expected = swarm.id.clone();
        let message_id = message.id.clone();
        let selector = format!("swarm-message-{}", message.sequence);
        let hue = match message.status {
            SwarmMessageStatus::Queued => colors.amber,
            SwarmMessageStatus::Received => colors.read,
            SwarmMessageStatus::Cancelled => colors.muted,
        };
        let receipt = if observation == Observation::Offline {
            format!("Last known: {}", delivery(message.status))
        } else {
            delivery(message.status).to_owned()
        };
        ui::card(colors).flex().flex_col().gap(px(8.)).p(px(14.))
            .child(div().id(ElementId::Name(selector.clone().into())).relative().child(crate::testing::probe(selector.clone())).debug_selector(move || selector.clone()).min_h(px(48.)).flex().flex_col().gap(px(5.))
                .child(ui::label(format!("{} → {}", peer_label(swarm, &message.from), peer_label(swarm, &message.to)), colors))
                .child(ui::hint(format!("#{} · {} · accepted {}", message.sequence, kind(message.kind), time(message.created_at)), colors).text_size(px(11.5)))
                .active(|style| style.bg(colors.selected))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.bound_swarm(id, pick, &expected).is_some() {
                        if let Some(state) = this.swarm_views.get_mut(&(id, pick)) {
                            state.open_message = if state.open_message.as_deref() == Some(message_id.as_str()) { None } else { Some(message_id.clone()) };
                        }
                        cx.notify();
                    }
                })))
            .child(div().text_size(px(14.)).line_height(px(21.)).text_color(colors.secondary).child(if open { message.text.clone() } else { preview(&message.text, 180) }))
            .child(ui::hint(receipt, colors).text_color(hue))
            .when_some(message.reply_to.as_ref(), |card, original| {
                let question = swarm.messages.iter().find(|item| &item.id == original && item.kind == SwarmMessageKind::Question && item.from == message.to && item.to == message.from);
                if let Some(question) = question {
                    let expected = swarm.id.clone();
                    let question_id = original.clone();
                    card.child(ui::button(format!("swarm-reply-{}", message.sequence), ui::Button::Quiet, Some("back"), format!("Reply to #{} · inspect question", question.sequence), false, colors).px(px(4.)).text_size(px(12.))
                        .on_click(cx.listener(move |this, _, _, cx| this.open_swarm_exchange(id, pick, &expected, &question_id, cx))))
                } else { card.child(ui::hint("Referenced question is not available in this snapshot.", colors)) }
            })
            .when(open, |card| {
                let expected = swarm.id.clone();
                let peer = message.to.clone();
                card.child(detail("Accepted by the broker", dated_time(message.created_at), colors))
                    .child(detail("Exact message ID", message.id.clone(), colors))
                    .children(message.reply_to.as_ref().map(|id| detail("Exact reply-to ID", id.clone(), colors)))
                    .children(message.received_at.map(|at| detail("Recipient retrieval", dated_time(at), colors)))
                    .child(ui::hint("This is the broker's text preview (up to 500 characters), not the entire mailbox. Inspect the recipient's conversation for its recorded tool result.", colors))
                    .child(ui::button(format!("swarm-recipient-{}", message.sequence), ui::Button::Quiet, Some("chat"), "Open recipient conversation", false, colors).px(px(4.)).text_size(px(12.5))
                        .on_click(cx.listener(move |this, _, window, cx| this.open_swarm_peer(id, pick, &expected, &peer, window, cx))) )
            })
    }

    fn swarm_results(
        &self,
        id: SessionId,
        pick: CrewPick,
        handoff: &Handoff,
        observation: Observation,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let swarm = handoff.swarm.as_ref().unwrap();
        let all_done = swarm.closed
            && swarm.members.iter().all(|member| {
                node(handoff, &member.id).is_some_and(|(_, child)| child.status == Status::Done)
            });
        let headline = if observation == Observation::Offline {
            "Last observed results"
        } else if all_done {
            "The peer assignments finished"
        } else if swarm.closed {
            "The swarm closed"
        } else {
            "Assignments are still open"
        };
        let reported = swarm
            .members
            .iter()
            .filter_map(|member| node(handoff, &member.id).and_then(|(_, child)| child.cost))
            .collect::<Vec<_>>();
        let spend = if reported.is_empty() {
            "Spend not reported".to_owned()
        } else {
            format!(
                "{} reported spend{}",
                cost(reported.iter().sum()),
                if reported.len() == swarm.members.len() {
                    ""
                } else {
                    " (partial)"
                }
            )
        };
        div().flex().flex_col().gap(px(12.))
            .child(card(headline, &format!("{} · {spend}. Finished assignments are not passing checks or accepted changes.", counts(handoff, swarm)), colors))
            .children(swarm.members.iter().map(|member| {
                let expected = swarm.id.clone();
                let peer = member.id.clone();
                let child = node(handoff, &peer).map(|(_, child)| child);
                let card_id = format!("swarm-result-card-{peer}");
                let body_id = format!("swarm-result-markdown-{peer}");
                let output = child.and_then(|child| child.output.as_deref()).filter(|text| !text.trim().is_empty());
                ui::card(colors).id(card_id.clone()).debug_selector(move || card_id.clone())
                    .min_w_0().p(px(14.)).flex().flex_col().gap(px(7.))
                    .child(ui::label(peer_label(swarm, &peer), colors))
                    .child(ui::hint(child.map_or_else(|| "Status unavailable".into(), |child| format!("{} · {}", status(child.status), child.cost.map_or_else(|| "Spend unavailable".into(), cost))), colors))
                    .child(match output {
                        Some(output) => crate::message::render(output, colors)
                            .id(body_id.clone())
                            .relative()
                            .child(crate::testing::probe(body_id.clone()))
                            .debug_selector(move || body_id.clone())
                            .into_any_element(),
                        None => ui::hint("No final answer in this snapshot.", colors).into_any_element(),
                    })
                    .child(ui::button(format!("swarm-result-{peer}"), ui::Button::Quiet, Some("chat"), "Open full conversation", false, colors).px(px(4.)).text_size(px(12.5))
                        .on_click(cx.listener(move |this, _, window, cx| this.open_swarm_peer(id, pick, &expected, &peer, window, cx))))
            }))
            .child(card("No structured patch or check records", "This read-only runtime does not publish isolated worktrees, patch revisions, integration candidates or exact-revision check results. No merge or check status is reported for this swarm.", colors))
    }

    fn swarm_groups(
        &self,
        id: SessionId,
        pick: CrewPick,
        handoff: &Handoff,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let swarm = handoff.swarm.as_ref().unwrap();
        let mut groups: BTreeMap<String, Vec<&str>> = BTreeMap::new();
        for member in &swarm.members {
            groups
                .entry(member.agent.clone())
                .or_default()
                .push(&member.id);
        }
        div().flex().flex_col().gap(px(12.))
            .child(ui::hint("Grouped by the actual agent names in this roster. Names are display groups, never routing addresses.", colors))
            .children(groups.into_iter().map(|(group, peers)| {
                let title = group.clone();
                ui::card(colors).p(px(14.)).flex().flex_col().gap(px(6.))
                    .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(ui::hint(format!("{} {} · {}", peers.len(), if peers.len() == 1 { "peer" } else { "peers" }, peers.join(", ")), colors))
                    .child(ui::button(format!("swarm-group-{group}"), ui::Button::Quiet, Some("chev_r"), "Open group members", false, colors).px(px(4.)).text_size(px(12.5))
                        .on_click(cx.listener(move |this, _, _, cx| { if let Some(state) = this.swarm_views.get_mut(&(id, pick)) { state.group = Some(group.clone()); state.tab = SwarmTab::Agents; } cx.notify(); })))
            }))
            .child(card("Larger swarms are not available yet", "The current computer runtime admits 2–8 parallel read-only peers. A 24-peer roster, yielding dependency scheduler and aggregate budget controls are not implemented.", colors))
    }

    fn swarm_capabilities(
        &self,
        id: SessionId,
        pick: CrewPick,
        swarm: &Swarm,
        tab: SwarmTab,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let mut body = div().flex().flex_col().gap(px(12.));
        match tab {
            SwarmTab::Decisions => {
                body = body.child(card("No structured user decisions", "Peer questions stay between addressed peers. This runtime cannot escalate one into a structured user decision or track an Answer/Later workflow. No answer has been sent from this screen.", colors))
                    .child(ui::button("swarm-open-questions", ui::Button::Plain, Some("chat"), "Inspect peer questions", false, colors)
                        .on_click(cx.listener(move |this, _, _, cx| { if let Some(state) = this.swarm_views.get_mut(&(id, pick)) { state.tab = SwarmTab::Exchanges; state.filter = ExchangeFilter::Questions; state.peer = None; state.open_message = None; } cx.notify(); })));
            }
            SwarmTab::Artifacts => {
                body = body.child(card("Artifacts and edits are unavailable", "Peers currently have read and bounded peer-messaging tools only. There are no isolated editing worktrees, shared artifact revisions, patch candidates, merge actions or conflict resolution controls.", colors))
                    .child(card("No integration record is available", "This runtime publishes no integration candidates or authoritative integration state. A peer's final answer or a received message is not evidence of an applied patch, passing check or accepted change.", colors));
            }
            _ => {
                body = body.child(card("Read-only peer communication", "Each peer has its own conversation. The computer binds exact sender/recipient IDs and stores bounded durable mailboxes. Only explicit findings, questions and replies are shown here.", colors))
                    .child(detail("Swarm identity", swarm.id.clone(), colors))
                    .child(card("Current limits", "2–8 peers. This screen observes up to 32 recent message events with bounded text previews. Waiting for dependencies, full paginated history, larger swarms, artifacts and parallel editing are not supported yet.", colors));
                for (view, label) in [
                    (SwarmTab::Decisions, "Decision support"),
                    (SwarmTab::Artifacts, "Artifact and editing support"),
                    (SwarmTab::Groups, "Groups and larger swarms"),
                ] {
                    body = body.child(
                        ui::button(
                            format!("swarm-support-{view:?}"),
                            ui::Button::Plain,
                            Some("info"),
                            label,
                            false,
                            colors,
                        )
                        .text_size(px(13.))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.swarm_tab(id, pick, view, cx)),
                        ),
                    );
                }
            }
        }
        body.child(ui::hint("Received is retrieval only. Messages do not spawn turns or grant tools. Read-only tool selection is not an operating-system sandbox.", colors))
    }

    pub(crate) fn swarm_screen(
        &mut self,
        id: SessionId,
        pick: CrewPick,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = theme(cx);
        let back = ui::tap("swarm-back", "back", &colors)
            .debug_selector(|| "swarm-back".into())
            .on_click(cx.listener(|this, _, window, cx| {
                this.back(window, cx);
            }));
        let Some(state) = self.swarm_views.get(&(id, pick)).cloned() else {
            return div()
                .flex_1()
                .flex()
                .flex_col()
                .child(ui::appbar(back, "Swarm", None, &colors))
                .child(
                    ui::hint("Open this swarm from its originating session.", &colors).p(px(20.)),
                )
                .into_any_element();
        };
        let Some(handoff) = self.bound_swarm(id, pick, &state.swarm_id).cloned() else {
            return div().flex_1().flex().flex_col().child(ui::appbar(back, "Swarm unavailable", None, &colors)).child(ui::hint("The originating handoff is no longer available. Cached indices do not select another crew.", &colors).p(px(20.))).into_any_element();
        };
        let swarm = handoff.swarm.as_ref().unwrap();
        let observation = self.swarm_observation(id);
        let session = self.store.as_ref().and_then(|store| store.session(id));
        let title = session.map_or_else(|| "Swarm".to_owned(), |session| session.title.clone());
        let lifecycle = if observation == Observation::Offline {
            format!(
                "Last known: {}",
                if swarm.closed { "Closed" } else { "Open" }
            )
        } else {
            (if swarm.closed { "Closed" } else { "Open" }).to_owned()
        };
        let info = ui::tap("swarm-info", "info", &colors)
            .debug_selector(|| "swarm-info".into())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.swarm_tab(id, pick, SwarmTab::Capabilities, cx)
            }));
        let scroll = self.scroll(Route::Swarm(id, pick));
        let mut body = div()
            .px(px(20.))
            .pt(px(12.))
            .pb(px(24.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(self.observation_banner(id, observation, &colors))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(ui::label(
                        format!("{lifecycle} · {} peers · Read-only", swarm.members.len()),
                        &colors,
                    ))
                    .child(ui::hint(counts(&handoff, swarm), &colors)),
            );
        if let Some(peer) = &state.stop_peer {
            let expected = swarm.id.clone();
            let peer_id = peer.clone();
            let enabled = self.swarm_peer_stop_available(id, pick, &swarm.id, peer);
            let confirm = ui::button(
                "swarm-confirm-stop",
                ui::Button::Plain,
                Some("stop"),
                "Request Stop for this conversation",
                false,
                &colors,
            )
            .text_size(px(13.));
            body = body.child(card(&format!("Stop the conversation for {}?", peer_label(swarm, peer)), "Only this child conversation is targeted, including any current follow-up. Other peers and Pi are not asked to stop. Stop does not undo external effects; inspect the peer conversation for the computer’s reported state.", &colors))
                .child(if enabled { confirm.on_click(cx.listener(move |this, _, _, cx| this.confirm_swarm_stop(id, pick, &expected, &peer_id, cx))) } else { ui::disabled(confirm, &colors) })
                .when(!enabled, |body| body.child(ui::hint("Stop requires a current running assignment or child turn, and is unavailable offline or in sample mode. Nothing is queued for reconnect.", &colors)))
                .child(ui::button("swarm-cancel-stop", ui::Button::Quiet, None, "Keep this peer", false, &colors).on_click(cx.listener(move |this, _, _, cx| { if let Some(state) = this.swarm_views.get_mut(&(id, pick)) { state.stop_peer = None; } cx.notify(); })));
        } else {
            body = match state.tab {
                SwarmTab::Map => body.child(swarm_map::map(id, pick, &handoff, observation == Observation::Offline, &colors, cx))
                    .child(ui::button("swarm-open-groups", ui::Button::Quiet, Some("layers"), "Grouped peer list", false, &colors).on_click(cx.listener(move |this, _, _, cx| this.swarm_tab(id, pick, SwarmTab::Groups, cx))))
                    .child(card("Peer questions stay with peers", "An explicit exchange is not a dependency or a request for your approval. This runtime reports assignment states but does not track durable blockers or waiting-on-reply states.", &colors)),
                SwarmTab::Exchanges => body.child(self.swarm_exchange_list(id, pick, swarm, &state, observation, &colors, cx)),
                SwarmTab::Agents => body.child(self.swarm_roster(id, pick, &handoff, &state, observation, &colors, cx))
                    .when(state.group.is_some(), |body| body.child(ui::button("swarm-clear-group", ui::Button::Quiet, None, "Show all peers", false, &colors).on_click(cx.listener(move |this, _, _, cx| { if let Some(state) = this.swarm_views.get_mut(&(id, pick)) { state.group = None; } cx.notify(); })))),
                SwarmTab::Results => body.child(self.swarm_results(id, pick, &handoff, observation, &colors, cx)),
                SwarmTab::Groups => body.child(self.swarm_groups(id, pick, &handoff, &colors, cx)),
                tab => body.child(self.swarm_capabilities(id, pick, swarm, tab, &colors, cx)),
            };
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                ui::appbar(
                    back,
                    title,
                    Some("Swarm · independent conversations".into()),
                    &colors,
                )
                .child(info),
            )
            .child(self.swarm_tabs(id, pick, state.tab, &colors, cx))
            .child(scroll_area("swarm-observation-scroll", &scroll).child(body))
            .child(
                div()
                    .flex_none()
                    .p(px(12.))
                    .border_t_1()
                    .border_color(colors.line)
                    .child(
                        ui::button(
                            "swarm-parent-session",
                            ui::Button::Plain,
                            Some("pi"),
                            if observation == Observation::Offline {
                                "Open cached parent session"
                            } else {
                                "Talk to Pi in the parent session"
                            },
                            false,
                            &colors,
                        )
                        .w_full()
                        .px(px(8.))
                        .text_size(px(13.))
                        .on_click(cx.listener(
                            move |this, _, window, cx| this.swarm_parent_thread(id, window, cx),
                        )),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn swarm_results_use_markdown_without_overflowing_peer_cards(cx: &mut gpui::TestAppContext) {
        use crate::theme::Appearance;
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        for width in [320., 412.] {
            cx.simulate_resize(gpui::size(px(width), px(915.)));
            for appearance in [Appearance::Moonstone, Appearance::Evening] {
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.preview("swarm-results", window, cx);
                        app.update_prefs(cx, |prefs| prefs.appearance = appearance);
                        app.apply_theme(window, cx);
                    })
                });
                cx.run_until_parked();
                for (card, markdown) in [
                    ("swarm-result-card-peer-1", "swarm-result-markdown-peer-1"),
                    ("swarm-result-card-peer-2", "swarm-result-markdown-peer-2"),
                ] {
                    let card = cx.debug_bounds(card).unwrap();
                    let markdown = cx
                        .debug_bounds(markdown)
                        .expect("result uses the shared renderer");
                    assert!(card.left() >= px(20.) && card.right() <= px(width - 20.));
                    assert!(
                        markdown.left() >= card.left() + px(14.)
                            && markdown.right() <= card.right() - px(14.)
                    );
                    assert!(
                        markdown.size.height > px(80.),
                        "headings, lists and code are distinct blocks"
                    );
                }
                assert!(cx.debug_bounds("swarm-result-card-peer-3").is_some());
                assert!(
                    cx.debug_bounds("swarm-result-markdown-peer-3").is_none(),
                    "missing output keeps the placeholder"
                );
                app.read_with(cx, |app, _| {
                    let handoff = app
                        .raw_swarm(
                            SessionId(1),
                            CrewPick {
                                turn: 0,
                                handoff: 0,
                            },
                        )
                        .unwrap();
                    assert_eq!(
                        handoff.subagents[0].output.as_deref(),
                        Some(crate::preview::SWARM_RESULT_MARKDOWN)
                    );
                });
            }
        }
    }

    fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
        let bounds = cx.debug_bounds(selector).expect("native control");
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
    }

    #[test]
    fn preview_preserves_unicode_and_only_shortens_bodies() {
        assert_eq!(preview("中文🚀 café", 3), "中文🚀…");
        assert_eq!(preview("short", 30), "short");
        assert_eq!(first_line("\n task\nsecond"), "task");
    }

    #[test]
    fn filters_distinguish_questions_from_replies_without_inferring_a_blocker() {
        assert!(ExchangeFilter::All.matches(SwarmMessageKind::Reply));
        assert!(ExchangeFilter::Questions.matches(SwarmMessageKind::Question));
        assert!(!ExchangeFilter::Questions.matches(SwarmMessageKind::Reply));
        assert!(ExchangeFilter::Replies.matches(SwarmMessageKind::Reply));
        assert!(ExchangeFilter::Findings.matches(SwarmMessageKind::Finding));
        assert_eq!(
            delivery(SwarmMessageStatus::Received),
            "Received · retrieved by recipient"
        );
    }

    #[gpui::test]
    fn swarm_map_duplicate_names_use_exact_ids_and_back_returns_to_origin(
        cx: &mut gpui::TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(px(320.), px(640.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("swarm-map", window, cx);
                app.store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|session| session.id == SessionId(1))
                    .unwrap()
                    .turns[0]
                    .handoffs[0]
                    .subagents
                    .swap(0, 1);
                cx.notify();
            })
        });
        cx.run_until_parked();
        click(cx, "swarm-map-peer-retry-swarm-peer-2");
        cx.run_until_parked();
        let pick = CrewPick {
            turn: 0,
            handoff: 0,
        };
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.route(),
                Route::SwarmPeer(
                    SessionId(1),
                    Pick {
                        turn: 0,
                        handoff: 0,
                        index: 0
                    },
                    1
                )
            );
            assert_eq!(
                app.store
                    .as_ref()
                    .unwrap()
                    .session(SessionId(1))
                    .unwrap()
                    .turns[0]
                    .handoffs[0]
                    .subagents[0]
                    .peer_id
                    .as_deref(),
                Some("peer-2")
            );
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.back(window, cx);
            })
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.route(), Route::Swarm(SessionId(1), pick))
        });
        click(cx, "swarm-back");
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.route(), Route::Thread(SessionId(1)))
        });
    }

    #[gpui::test]
    fn swarm_callbacks_reject_replaced_rosters_and_reset_old_selections(
        cx: &mut gpui::TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        let id = SessionId(1);
        let pick = CrewPick {
            turn: 0,
            handoff: 0,
        };
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("swarm-map", window, cx);
                let message = app
                    .raw_swarm(id, pick)
                    .unwrap()
                    .swarm
                    .as_ref()
                    .unwrap()
                    .messages[0]
                    .id
                    .clone();
                app.open_swarm_exchange(id, pick, "retry-swarm", &message, cx);
                app.swarm_views.get_mut(&(id, pick)).unwrap().stop_peer = Some("peer-1".into());
                app.store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|session| session.id == id)
                    .unwrap()
                    .turns[0]
                    .handoffs[0]
                    .swarm
                    .as_mut()
                    .unwrap()
                    .id = "replacement-swarm".into();
                app.open_swarm_peer(id, pick, "retry-swarm", "peer-1", window, cx);
                app.open_swarm_exchange(id, pick, "retry-swarm", &message, cx);
                assert_eq!(app.route(), Route::Swarm(id, pick));
                assert_eq!(app.swarm_views[&(id, pick)].swarm_id, "retry-swarm");
                assert_eq!(
                    app.swarm_views[&(id, pick)].open_message.as_deref(),
                    Some(message.as_str())
                );
                app.open_swarm(id, pick, "replacement-swarm", window, cx);
                let state = &app.swarm_views[&(id, pick)];
                assert_eq!(state.swarm_id, "replacement-swarm");
                assert_eq!(state.tab, SwarmTab::Map);
                assert!(state.open_message.is_none());
                assert!(state.stop_peer.is_none());
                assert_eq!(
                    app.routes
                        .iter()
                        .filter(|route| **route == Route::Swarm(id, pick))
                        .count(),
                    1
                );
            })
        });
        cx.run_until_parked();
    }

    #[test]
    fn swarm_send_requires_the_displayed_turn_not_a_new_run_or_follow_up() {
        use crate::live::SubagentInputTarget::{FollowUp, Steer};
        let shown = Steer {
            run_id: "shown-run".into(),
        };
        let replacement = Steer {
            run_id: "replacement-run".into(),
        };
        assert!(input_target_current(Some(&shown), Some(&shown)));
        assert!(input_target_current(Some(&FollowUp), Some(&FollowUp)));
        assert!(!input_target_current(Some(&shown), Some(&replacement)));
        assert!(!input_target_current(Some(&shown), Some(&FollowUp)));
        assert!(!input_target_current(Some(&FollowUp), Some(&shown)));
        assert!(!input_target_current(None, None));
        assert!(!input_target_current(Some(&shown), None));
    }

    #[gpui::test]
    fn swarm_exchange_navigation_filters_and_history_eviction_are_snapshot_only(
        cx: &mut gpui::TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        let id = SessionId(1);
        let pick = CrewPick {
            turn: 0,
            handoff: 0,
        };
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("swarm-map", window, cx)));
        cx.run_until_parked();
        click(cx, "swarm-tab-exchanges");
        cx.run_until_parked();
        click(cx, "swarm-filter-questions");
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            let state = &app.swarm_views[&(id, pick)];
            assert_eq!(state.tab, SwarmTab::Exchanges);
            assert_eq!(state.filter, ExchangeFilter::Questions);
            let swarm = app.raw_swarm(id, pick).unwrap().swarm.as_ref().unwrap();
            let filtered = messages(swarm, state.filter, Some("peer-2"));
            assert_eq!(filtered.len(), 1);
            assert_eq!(filtered[0].from, "peer-2");
            assert_eq!(filtered[0].to, "peer-1");
            assert!(messages(swarm, state.filter, Some("peer-3")).is_empty());
        });
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                let message = app
                    .raw_swarm(id, pick)
                    .unwrap()
                    .swarm
                    .as_ref()
                    .unwrap()
                    .messages[2]
                    .id
                    .clone();
                app.open_swarm_exchange(id, pick, "retry-swarm", &message, cx);
                assert_eq!(app.swarm_views[&(id, pick)].filter, ExchangeFilter::All);
                app.store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|session| session.id == id)
                    .unwrap()
                    .turns[0]
                    .handoffs[0]
                    .swarm
                    .as_mut()
                    .unwrap()
                    .messages
                    .retain(|item| item.id != message);
                // A stale click may not inspect a different message at the same list position.
                app.open_swarm_exchange(id, pick, "retry-swarm", &message, cx);
                assert_eq!(
                    app.swarm_views[&(id, pick)].open_message.as_deref(),
                    Some(message.as_str())
                );
                cx.notify();
            })
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn swarm_offline_navigation_keeps_observation_time_and_never_queues_stop(
        cx: &mut gpui::TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        let id = SessionId(1);
        let pick = CrewPick {
            turn: 0,
            handoff: 0,
        };
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("swarm-map-offline", window, cx);
                let at = app.store.as_ref().unwrap().observed_at[&id];
                assert_eq!(app.swarm_observation(id), Observation::Offline);
                app.swarm_views.get_mut(&(id, pick)).unwrap().stop_peer = Some("peer-1".into());
                app.confirm_swarm_stop(id, pick, "retry-swarm", "peer-1", cx);
                assert!(app.swarm_views[&(id, pick)].stop_requested.is_empty());
                app.open_swarm_peer(id, pick, "retry-swarm", "peer-2", window, cx);
                assert!(matches!(app.route(), Route::SwarmPeer(_, _, _)));
                app.back(window, cx);
                assert_eq!(app.route(), Route::Swarm(id, pick));
                app.swarm_parent_thread(id, window, cx);
                assert_eq!(app.route(), Route::Thread(id));
                assert_eq!(app.store.as_ref().unwrap().observed_at[&id], at);
            })
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn swarm_peer_resolution_requires_unique_current_roster_membership(
        cx: &mut gpui::TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        let id = SessionId(1);
        let pick = CrewPick {
            turn: 0,
            handoff: 0,
        };
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("swarm-map", window, cx);
                let handoff = &mut app
                    .store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|session| session.id == id)
                    .unwrap()
                    .turns[0]
                    .handoffs[0];
                handoff.subagents[0].peer_id = Some("peer-404".into());
                assert!(node(handoff, "peer-404").is_none());
                handoff.subagents[0].peer_id = Some("peer-2".into());
                assert!(node(handoff, "peer-2").is_none());
                app.open_swarm_peer(id, pick, "retry-swarm", "peer-2", window, cx);
                assert_eq!(app.route(), Route::Swarm(id, pick));
            })
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn swarm_native_tabs_and_peer_targets_fit_320dp_in_both_themes(cx: &mut gpui::TestAppContext) {
        for fixture in [
            "swarm-map",
            "swarm-map-evening",
            "swarm-map-offline",
            "swarm-no-messages",
        ] {
            let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
            cx.simulate_resize(gpui::size(px(320.), px(640.)));
            cx.update(|window, cx| app.update(cx, |app, cx| app.preview(fixture, window, cx)));
            cx.run_until_parked();
            for tab in [
                "swarm-tab-map",
                "swarm-tab-exchanges",
                "swarm-tab-agents",
                "swarm-tab-results",
            ] {
                let bounds = cx.debug_bounds(tab).expect("tab has a native touch target");
                assert!(bounds.size.height >= px(48.));
                assert!(
                    bounds.left() >= px(0.) && bounds.right() <= px(320.),
                    "{fixture}: {tab}: {bounds:?}"
                );
            }
            for (peer, selector) in [
                ("peer-1", "swarm-map-peer-retry-swarm-peer-1"),
                ("peer-2", "swarm-map-peer-retry-swarm-peer-2"),
                ("peer-3", "swarm-map-peer-retry-swarm-peer-3"),
            ] {
                let bounds = cx.debug_bounds(selector).expect("real peer node");
                assert!(bounds.size.height >= px(48.));
                assert!(
                    bounds.left() >= px(0.) && bounds.right() <= px(320.),
                    "{fixture}: {peer}: {bounds:?}"
                );
            }
        }
    }

    #[gpui::test]
    fn swarm_peer_route_remains_bound_after_reorder_and_refuses_a_replaced_crew(
        cx: &mut gpui::TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("swarm-agent", window, cx);
                let Route::SwarmPeer(id, original, token) = app.route() else {
                    panic!("bound peer route");
                };
                let binding = app.swarm_peer_bindings[&token].clone();
                assert_eq!(binding.peer_id, "peer-2");
                assert_eq!(
                    app.bound_swarm_peer(&binding)
                        .unwrap()
                        .conversation_id
                        .as_deref(),
                    Some("12")
                );
                app.store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|session| session.id == id)
                    .unwrap()
                    .turns[0]
                    .handoffs[0]
                    .subagents
                    .swap(0, 1);
                assert_eq!(app.route(), Route::SwarmPeer(id, original, token));
                assert_eq!(
                    app.bound_swarm_peer(&binding).unwrap().peer_id.as_deref(),
                    Some("peer-2")
                );
                assert_eq!(
                    app.bound_swarm_peer(&binding)
                        .unwrap()
                        .conversation_id
                        .as_deref(),
                    Some("12")
                );
                assert!(
                    !app.can_send_swarm_peer(&binding, "12"),
                    "sample never sends"
                );
                app.store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|session| session.id == id)
                    .unwrap()
                    .turns[0]
                    .handoffs[0]
                    .swarm
                    .as_mut()
                    .unwrap()
                    .id = "replacement-swarm".into();
                assert!(app.bound_swarm_peer(&binding).is_none());
                assert!(!app.can_send_swarm_peer(&binding, "12"));
                cx.notify();
            })
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn swarm_retained_peer_callback_refuses_a_replaced_computer_source(
        cx: &mut gpui::TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(px(320.), px(640.)));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("swarm-agent", window, cx)));
        cx.run_until_parked();
        let original = app.read_with(cx, |app, _| app.route());
        cx.update(|_, cx| {
            app.update(cx, |app, _| {
                app.store.as_mut().unwrap().computer.address = "replacement-computer.local".into();
                let Route::SwarmPeer(_, _, token) = app.route() else {
                    panic!("bound peer");
                };
                assert!(
                    app.bound_swarm_peer(&app.swarm_peer_bindings[&token])
                        .is_none()
                );
            })
        });
        click(cx, "peer-exchanges");
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.route(),
                original,
                "the retained callback must not open the matching IDs in another source"
            )
        });
    }

    #[gpui::test]
    fn swarm_offline_peer_dock_has_no_stop_or_live_input_and_new_map_keeps_origin(
        cx: &mut gpui::TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(px(320.), px(640.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("swarm-offline", window, cx);
                app.open_swarm_peer_from_handoff(
                    SessionId(1),
                    CrewPick {
                        turn: 0,
                        handoff: 0,
                    },
                    "retry-swarm",
                    "peer-2",
                    window,
                    cx,
                );
            })
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("stop-subagent").is_none());
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let Route::SwarmPeer(id, original, token) = app.route() else {
                    panic!("bound peer route");
                };
                let binding = app.swarm_peer_bindings[&token].clone();
                assert!(!app.can_send_swarm_peer(&binding, "12"));
                app.open_swarm(id, binding.pick, &binding.swarm_id, window, cx);
                assert!(matches!(app.route(), Route::Swarm(_, _)));
                app.back(window, cx);
                assert_eq!(app.route(), Route::SwarmPeer(id, original, token));
                assert_eq!(
                    app.bound_swarm_peer(&binding).unwrap().peer_id.as_deref(),
                    Some("peer-2")
                );
            })
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn swarm_stop_policy_allows_explicit_retry_but_never_offline_sample_closed_or_ambiguous_targets(
        cx: &mut gpui::TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("swarm-map", window, cx);
                let pick = CrewPick {
                    turn: 0,
                    handoff: 0,
                };
                app.swarm_views
                    .get_mut(&(SessionId(1), pick))
                    .unwrap()
                    .stop_requested
                    .insert("peer-1".into());
                let handoff = &mut app
                    .store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|session| session.id == SessionId(1))
                    .unwrap()
                    .turns[0]
                    .handoffs[0];
                assert!(
                    may_request_stop(handoff, "peer-1", Observation::Observed),
                    "an uncertain request must not permanently prohibit explicit retry"
                );
                assert!(!may_request_stop(handoff, "peer-1", Observation::Offline));
                assert!(!may_request_stop(handoff, "peer-1", Observation::Sample));
                assert!(!may_request_stop(
                    handoff,
                    "peer-404",
                    Observation::Observed
                ));
                handoff.subagents[1].peer_id = Some("peer-1".into());
                assert!(!may_request_stop(handoff, "peer-1", Observation::Observed));
                handoff.subagents[1].peer_id = Some("peer-2".into());
                handoff.swarm.as_mut().unwrap().closed = true;
                assert!(!may_request_stop(handoff, "peer-1", Observation::Observed));
                handoff.subagents[0].status = Status::Done;
                let turn = crate::live::SubagentInputTarget::Steer {
                    run_id: "current-follow-up".into(),
                };
                assert!(
                    may_request_conversation_stop(
                        handoff,
                        "peer-1",
                        Observation::Observed,
                        Some(&turn)
                    ),
                    "a later child turn still has its existing exact-conversation Stop control"
                );
                for observation in [Observation::Offline, Observation::Sample] {
                    assert!(!may_request_conversation_stop(
                        handoff,
                        "peer-1",
                        observation,
                        Some(&turn)
                    ));
                }
                assert!(!may_request_conversation_stop(
                    handoff,
                    "peer-1",
                    Observation::Observed,
                    Some(&crate::live::SubagentInputTarget::FollowUp)
                ));
                assert!(!may_request_conversation_stop(
                    handoff,
                    "peer-404",
                    Observation::Observed,
                    Some(&turn)
                ));
                handoff.subagents[1].peer_id = Some("peer-1".into());
                assert!(!may_request_conversation_stop(
                    handoff,
                    "peer-1",
                    Observation::Observed,
                    Some(&turn)
                ));
            })
        });
        cx.run_until_parked();
    }
}
