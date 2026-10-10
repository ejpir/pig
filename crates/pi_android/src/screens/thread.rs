//! 04–06 A session: each prompt, the run line through the stages Pi went
//! through, and the hand-off. The run that is going runs down the gutter, a
//! station per stage; a finished one is drawn as one line with each stretch's
//! time, with the report card under it. What moves the work forward sits in the
//! dock: Working and Stop on the composer, the question in its place, Review.

use crate::{
    app::{PhoneApp, Route, Sheet},
    model::{
        CheckResult, Flow, Reference, Session, SessionId, Stage, StageKind, StageStatus, State,
        Turn, duration_label,
    },
    theme::{MONO, Theme, theme},
    ui::{self, Button, icon},
};
use gpui::{
    AnyElement, Context, Div, FollowMode, FontWeight, Hsla, ListAlignment, SharedString, Window,
    div, list, prelude::*, px, relative,
};
use std::{collections::HashSet, sync::Arc, time::Duration};

/// How many files and searches a stage shows before "+4 more".
const CHIPS: usize = 3;
/// How many lines of a running tool's output its stage shows.
const TAIL: usize = 4;
/// How many changed files the report card lists before "Show 5 more files".
const FILES: usize = 3;
/// Small conversations appear immediately; large first opens get a painted
/// loading frame before exact full-history measurement starts.
const LARGE_THREAD_ITEMS: usize = 24;
const THREAD_LOADING_PAINT: Duration = Duration::from_millis(80);

fn new_thread_list_state(item_count: usize) -> gpui::ListState {
    // Accurate off-screen heights keep the draggable thumb mapped to the whole
    // conversation. Rows remain virtualized for painting after this first
    // measurement pass, and later renders only remeasure changed rows below.
    gpui::ListState::new(item_count, ListAlignment::Top, px(320.)).measure_all()
}

fn thread_running(session: &Session, compacting: bool) -> bool {
    session.state.is_running() || compacting
}

fn composer_status(session: &Session, compacting: bool) -> &'static str {
    if compacting {
        "Compacting"
    } else if session.state == State::NeedsYou {
        "Needs you"
    } else {
        "Working"
    }
}

fn large_thread(item_count: usize) -> bool {
    item_count >= LARGE_THREAD_ITEMS
}

#[derive(Default)]
struct ThreadLayoutChanges {
    turns: Vec<usize>,
    queue: bool,
}

impl ThreadLayoutChanges {
    fn turn(&mut self, index: usize) {
        if !self.turns.contains(&index) {
            self.turns.push(index);
        }
    }
}

/// Compare only geometry-relevant turn state. Most duration increments repaint
/// in place; the timing signature below catches the transitions that add rows.
fn turn_layout_changed(before: &Turn, after: &Turn) -> bool {
    before.prompt != after.prompt
        || before.at != after.at
        || before.attachments != after.attachments
        || before.stages != after.stages
        || before.summary != after.summary
        || before.pages != after.pages
        || before.images != after.images
        || before.handoffs != after.handoffs
        || before.reported != after.reported
        || before.flow != after.flow
        || timing_layout_signature(before) != timing_layout_signature(after)
}

/// Time text itself paints within a fixed line. Geometry changes only when
/// real timings add that line, or when a page turn's two-column key gains or
/// loses rows because some stages have no reported duration.
fn timing_layout_signature(turn: &Turn) -> (bool, usize) {
    let (times, show_times) = run_times(turn);
    let key_rows = if turn.pages.is_empty() {
        0
    } else {
        turn.stages
            .iter()
            .filter(|stage| {
                stage.status == StageStatus::Done && !times[stage.kind.index()].is_zero()
            })
            .count()
            .div_ceil(2)
    };
    (show_times, key_rows)
}

fn failed_prompt_layouts(failed: &[crate::live::FailedPrompt]) -> Vec<(String, String)> {
    failed
        .iter()
        .map(|failed| (failed.error.clone(), failed.prompt.label()))
        .collect()
}

fn changed_failed_layouts(
    before: Option<&Vec<(String, String)>>,
    after: &[(String, String)],
) -> Vec<usize> {
    let Some(before) = before.filter(|before| before.len() == after.len()) else {
        return Vec::new();
    };
    before
        .iter()
        .zip(after)
        .enumerate()
        .filter_map(|(index, (before, after))| (before != after).then_some(index))
        .collect()
}

fn thread_shape_remeasure_range(
    turn_count: usize,
    item_count: usize,
) -> Option<std::ops::Range<usize>> {
    (item_count > 0).then(|| {
        let start = turn_count.saturating_sub(1).min(item_count - 1);
        start..item_count
    })
}

/// Failed-prompt recovery can splice the list during a retained-snapshot
/// fling. Remember that shape change and perform exact measurement on settle.
fn thread_shape_remeasure_needed(
    pending: &mut HashSet<SessionId>,
    id: SessionId,
    scrolling: bool,
    shape_changed: bool,
) -> bool {
    if scrolling {
        if shape_changed {
            pending.insert(id);
        }
        false
    } else {
        let was_pending = pending.remove(&id);
        shape_changed || was_pending
    }
}

fn thread_layout_changes(before: &Session, after: &Session) -> ThreadLayoutChanges {
    let mut changes = ThreadLayoutChanges::default();
    let shared_turns = before.turns.len().min(after.turns.len());

    // Once there is more than one turn, every prompt gains its numbered header.
    if (before.turns.len() > 1) != (after.turns.len() > 1) {
        for index in 0..shared_turns {
            changes.turn(index);
        }
    } else {
        for (index, (before, after)) in before.turns.iter().zip(&after.turns).enumerate() {
            if turn_layout_changed(before, after) {
                changes.turn(index);
            }
        }
    }

    if before.turns.len() < after.turns.len() && !before.turns.is_empty() {
        // The preceding last turn stops being the latest one.
        changes.turn(before.turns.len() - 1);
    } else if before.turns.len() > after.turns.len() && !after.turns.is_empty() {
        // The remaining final turn becomes the latest one.
        changes.turn(after.turns.len() - 1);
    }

    if (before.state != after.state
        || before.files != after.files
        || before.check != after.check
        || before.failure != after.failure)
        && !after.turns.is_empty()
    {
        changes.turn(after.turns.len() - 1);
    }
    changes.queue =
        !before.queued.is_empty() && !after.queued.is_empty() && before.queued != after.queued;
    changes
}

impl PhoneApp {
    pub(crate) fn begin_thread_loading(&mut self, id: SessionId) {
        if self.thread_lists.contains_key(&id) {
            return;
        }
        let Some(store) = &self.store else {
            return;
        };
        let waiting_for_snapshot = store.live.as_ref().is_some_and(|live| live.is_loading(id));
        let item_count = store.session(id).map_or(0, |session| {
            session.turns.len()
                + usize::from(!session.queued.is_empty())
                + store
                    .live
                    .as_ref()
                    .map_or(0, |live| live.failed_prompts(id).len())
        });
        if waiting_for_snapshot || large_thread(item_count) {
            self.thread_loading_generation = self.thread_loading_generation.wrapping_add(1).max(1);
            self.thread_loading
                .insert(id, self.thread_loading_generation);
            self.thread_loading_release_scheduled.remove(&id);
        }
    }

    fn thread_visible(&self, id: SessionId) -> bool {
        self.visible && matches!(self.route(), Route::Thread(shown) if shown == id)
    }

    fn thread_is_loading(
        &mut self,
        id: SessionId,
        item_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(generation) = self.thread_loading.get(&id).copied() else {
            return false;
        };
        let waiting_for_snapshot = self
            .store
            .as_ref()
            .and_then(|store| store.live.as_ref())
            .is_some_and(|live| live.is_loading(id));
        if waiting_for_snapshot {
            return true;
        }
        if !large_thread(item_count) {
            self.thread_loading.remove(&id);
            self.thread_loading_release_scheduled.remove(&id);
            return false;
        }
        if self.thread_loading_release_scheduled.get(&id) != Some(&generation) {
            self.thread_loading_release_scheduled.insert(id, generation);
            // This callback runs on the frame after the loading element was
            // rendered, so at least one loading frame has reached presentation
            // before the exact measure_all pass can begin.
            cx.on_next_frame(window, move |this, _, cx| {
                let same_load = this.thread_loading.get(&id) == Some(&generation);
                let still_visible = this.thread_visible(id);
                let waiting = this
                    .store
                    .as_ref()
                    .and_then(|store| store.live.as_ref())
                    .is_some_and(|live| live.is_loading(id));
                if !same_load || !still_visible || waiting {
                    if this.thread_loading_release_scheduled.get(&id) == Some(&generation) {
                        this.thread_loading_release_scheduled.remove(&id);
                    }
                    return;
                }
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(THREAD_LOADING_PAINT).await;
                    let _ = this.update(cx, |this, cx| {
                        let same_load = this.thread_loading.get(&id) == Some(&generation);
                        let release = same_load
                            && this.thread_visible(id)
                            && !this
                                .store
                                .as_ref()
                                .and_then(|store| store.live.as_ref())
                                .is_some_and(|live| live.is_loading(id));
                        if this.thread_loading_release_scheduled.get(&id) == Some(&generation) {
                            this.thread_loading_release_scheduled.remove(&id);
                        }
                        if release {
                            this.thread_loading.remove(&id);
                            cx.notify();
                        }
                    });
                })
                .detach();
            });
        }
        true
    }

    pub(crate) fn remeasure_thread_turn(&self, id: SessionId, index: usize) {
        if let Some(list) = self.thread_lists.get(&id)
            && index < list.item_count()
        {
            list.remeasure_items(index..index + 1);
        }
    }

    pub(crate) fn thread_screen(
        &mut self,
        id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = theme(cx);
        let composer = self.thread_composer(id, window, cx);
        let commands = self.command_catalog_for_session(id, cx);
        composer.update(cx, |composer, _| composer.use_commands(commands));
        let (model, thinking) = self.model_settings(cx);
        composer.update(cx, |composer, _| composer.set_model_label(model, thinking));
        let back = ui::tap("back", "back", &colors).on_click(cx.listener(|this, _, window, cx| {
            this.back(window, cx);
        }));
        let scrolling = self.scroll_in_motion();
        let mut layout_changes = ThreadLayoutChanges::default();
        let cached = scrolling
            .then(|| self.thread_snapshots.get(&id).cloned())
            .flatten();
        let session = cached.or_else(|| {
            let current = self.store.as_ref()?.session(id)?;
            if let Some(previous) = self.thread_snapshots.get(&id).cloned() {
                if previous.as_ref() == current {
                    return Some(previous);
                }
                layout_changes = thread_layout_changes(&previous, current);
            }
            let snapshot = Arc::new(current.clone());
            self.thread_snapshots.insert(id, snapshot.clone());
            Some(snapshot)
        });
        let Some(session) = session else {
            return div()
                .flex_1()
                .child(ui::appbar(back, "Session", None, &colors))
                .child(ui::hint("This session is no longer on this phone.", &colors).px(px(20.)))
                .into_any_element();
        };
        let Some(store) = &self.store else {
            return div().into_any_element();
        };
        let computer_name = store.computer.name.clone();
        let stopping = store.live.as_ref().is_some_and(|live| live.is_stopping(id));
        let compacting = self.compact_pending(id)
            || store
                .live
                .as_ref()
                .is_some_and(|live| live.is_compacting(id))
            || session.activity == "Compacting";
        let failed_prompts = store
            .live
            .as_ref()
            .map(|live| live.failed_prompts(id).to_vec())
            .unwrap_or_default();
        let failed_layouts = failed_prompt_layouts(&failed_prompts);
        let failed_layout_changes = if scrolling {
            Vec::new()
        } else {
            let changes =
                changed_failed_layouts(self.thread_failed_layouts.get(&id), &failed_layouts);
            self.thread_failed_layouts.insert(id, failed_layouts);
            changes
        };
        let running = thread_running(&session, compacting);
        let asking = !compacting
            && session.state == State::NeedsYou
            && session.question.is_some()
            && !self.questions_later.contains(&id);
        let working = running && !asking;
        composer.update(cx, |composer, _| composer.set_joined(working));
        let pages = session.turn().is_some_and(|turn| !turn.pages.is_empty());
        let area = composer.read(cx).area.clone();
        area.update(cx, |area, _| {
            area.set_placeholder(if running {
                "Steer this run…"
            } else if pages {
                "Ask for a change…"
            } else {
                "Ask a follow-up…"
            })
        });
        let subtitle = if compacting {
            format!("{} · {}", session.project, computer_name)
        } else {
            match session.state {
                State::NeedsYou | State::Working => {
                    format!("{} · {}", session.project, computer_name)
                }
                State::Done => match super::subagents::at_work(&session.turns) {
                    // Pi answered, and its subagents carry on.
                    Some(handoff) => format!(
                        "{} · {}",
                        session.project,
                        crate::projection::at_work(handoff)
                    ),
                    None => format!(
                        "{} · done in {}",
                        session.project,
                        duration_label(session.elapsed)
                    ),
                },
                State::Stopped => format!("{} · stopped by you", session.project),
                State::Failed => format!("{} · {}", session.project, session.status_line()),
            }
        };
        let appbar =
            ui::appbar(back, session.title.clone(), Some(subtitle.into()), &colors)
                .child(ui::tap("details", "info", &colors).on_click(
                    cx.listener(move |this, _, _, cx| this.open_sheet(Sheet::Details(id), cx)),
                ))
                .child(ui::tap("more", "dots", &colors).on_click(
                    cx.listener(move |this, _, _, cx| this.open_sheet(Sheet::More(id), cx)),
                ));
        let turn_count = session.turns.len();
        let failed_count = failed_prompts.len();
        let has_queued = !session.queued.is_empty();
        let item_count = turn_count + failed_count + usize::from(has_queued);
        if self.thread_is_loading(id, item_count, window, cx) {
            return div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(appbar)
                .child(
                    div()
                        .id("thread-loading")
                        .debug_selector(|| "thread-loading".into())
                        .relative()
                        .child(crate::testing::probe("thread-loading"))
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(10.))
                        .text_size(px(14.))
                        .text_color(colors.secondary)
                        .child(ui::working_indicator(&colors))
                        .child("Loading conversation…"),
                )
                .into_any_element();
        }
        let list_state = self
            .thread_lists
            .entry(id)
            .or_insert_with(|| {
                let state = new_thread_list_state(item_count);
                state.set_follow_mode(FollowMode::Tail);
                state
            })
            .clone();
        let shape = (turn_count, failed_count, has_queued);
        let previous = self.thread_list_shapes.entry(id).or_insert(shape);
        let shape_changed = *previous != shape;
        if shape_changed {
            let (old_turns, old_failed, old_queued) = *previous;
            if old_turns < turn_count {
                list_state.splice(old_turns..old_turns, turn_count - old_turns);
            } else if old_turns > turn_count {
                list_state.splice(turn_count..old_turns, 0);
            }
            if old_failed < failed_count {
                let at = turn_count + old_failed;
                list_state.splice(at..at, failed_count - old_failed);
            } else if old_failed > failed_count {
                let start = turn_count + failed_count;
                list_state.splice(start..turn_count + old_failed, 0);
            }
            let queue_index = turn_count + failed_count;
            match (old_queued, has_queued) {
                (false, true) => list_state.splice(queue_index..queue_index, 1),
                (true, false) => list_state.splice(queue_index..queue_index + 1, 0),
                _ => {}
            }
            *previous = shape;
        }
        debug_assert_eq!(list_state.item_count(), item_count);
        let shape_remeasure = thread_shape_remeasure_needed(
            &mut self.thread_pending_remeasure,
            id,
            scrolling,
            shape_changed,
        );
        if !scrolling {
            for index in layout_changes.turns {
                if index < turn_count {
                    list_state.remeasure_items(index..index + 1);
                }
            }
            if shape_remeasure {
                // Inserted rows have no measured size, and the preceding row's
                // last-item padding may also have changed. Re-arm measure_all
                // from the latest turn through failed and queued rows.
                if let Some(range) = thread_shape_remeasure_range(turn_count, item_count) {
                    list_state.remeasure_items(range);
                }
            } else {
                for index in failed_layout_changes {
                    let index = turn_count + index;
                    if index < turn_count + failed_count {
                        list_state.remeasure_items(index..index + 1);
                    }
                }
                if layout_changes.queue && has_queued {
                    let queue_index = turn_count + failed_count;
                    list_state.remeasure_items(queue_index..queue_index + 1);
                }
            }
        }
        let offset = list_state.scroll_px_offset_for_scrollbar().y;
        if let Some(previous) = self.thread_list_offsets.insert(id, offset) {
            crate::scroll::note_movement(offset - previous);
        }
        let away_from_bottom = !list_state.is_following_tail();
        let list_session = session.clone();
        let list_failed = failed_prompts;
        let list_colors = colors;
        let thread_list = list(
            list_state.clone(),
            cx.processor(move |this, index, _, cx| {
                let content = if index < list_session.turns.len() {
                    this.turn(&list_session, index, false, &list_colors, cx)
                        .into_any_element()
                } else if let Some((failed_index, failed)) = index
                    .checked_sub(list_session.turns.len())
                    .and_then(|failed_index| {
                        list_failed
                            .get(failed_index)
                            .map(|failed| (failed_index, failed))
                    })
                {
                    this.failed_prompt(id, failed_index, failed, &list_colors, cx)
                        .into_any_element()
                } else {
                    this.queued_messages(id, &list_session.queued, &list_colors, cx)
                        .into_any_element()
                };
                div()
                    .w_full()
                    .min_w_0()
                    .px(px(20.))
                    .when(index == 0, |item| item.pt(px(8.)))
                    .pb(if index + 1 == item_count {
                        px(16.)
                    } else {
                        px(24.)
                    })
                    .child(content)
                    .into_any_element()
            }),
        )
        .size_full();
        let dock = if asking {
            div()
                .px(px(12.))
                .child(self.question_card(&session, &computer_name, &colors, cx))
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .when(running, |dock| {
                            dock.child(working_strip(&session, stopping, compacting, &colors, cx))
                        })
                        .child(composer),
                )
                .into_any_element()
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(appbar)
            .child(
                // Latest reply floats over the thread: as a row of its own it
                // would resize the thread whenever growing content let the
                // reader fall behind, which moved the content under them.
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        crate::scroll::virtual_list(("thread", id.0 as usize), &list_state)
                            .flex_1()
                            .min_h_0()
                            .child(thread_list),
                    )
                    .when(away_from_bottom, |thread| {
                        thread.child(
                            div().absolute().right(px(16.)).bottom(px(4.)).child(
                                ui::button(
                                    "latest",
                                    Button::Plain,
                                    Some("chev_d"),
                                    "Latest reply",
                                    true,
                                    &colors,
                                )
                                .debug_selector(|| "latest".into())
                                .shadow(vec![gpui::BoxShadow {
                                    color: colors.shadow,
                                    offset: gpui::point(px(0.), px(4.)),
                                    blur_radius: px(12.),
                                    spread_radius: px(0.),
                                    inset: false,
                                }])
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        // This button disappears after the jump. Consume the
                                        // release so the image or change card newly underneath
                                        // cannot receive the same tap.
                                        cx.stop_propagation();
                                        window.prevent_default();
                                        if let Some(list) = this.thread_lists.get(&id) {
                                            list.set_follow_mode(FollowMode::Tail);
                                        }
                                        cx.notify();
                                    },
                                )),
                            ),
                        )
                    }),
            )
            .child(div().flex_none().pt(px(8.)).pb(px(12.)).child(dock))
            .into_any_element()
    }

    /// Each turn of a session: the prompt, the run, and what Pi said and
    /// showed. `read_only` leaves out what changes the session.
    pub(crate) fn turns(
        &self,
        session: &Session,
        read_only: bool,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Vec<gpui::Stateful<Div>> {
        session
            .turns
            .iter()
            .enumerate()
            .map(|(index, _)| self.turn(session, index, read_only, colors, cx))
            .collect()
    }

    fn turn(
        &self,
        session: &Session,
        index: usize,
        read_only: bool,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        let id = session.id;
        let turn = &session.turns[index];
        let running = session.state.is_running();
        let last = session.turns.len().saturating_sub(1);
        let live = index == last && running;
        let many = session.turns.len() > 1;
        // Review and the report card belong to the session the user runs.
        let ending =
            (index == last && !running && !read_only).then(|| self.ending(session, index, cx));
        div()
            .id(("turn", index))
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(24.))
            .when(index > 0, |turn| {
                turn.pt(px(24.)).border_t_1().border_color(colors.line)
            })
            .child(self.prompt(id, index, turn, live, many, colors, cx))
            .child(if live {
                self.stations(session, index, turn, true, colors, cx)
            } else {
                self.turn_activity(id, index, turn, false, colors, cx)
            })
            .map(|column| {
                if turn.interleaved() {
                    column.children(self.flow(id, index, turn, colors, cx))
                } else {
                    column
                        .children(reply(turn, index == last, colors).map(|reply| {
                            let text = turn.summary.as_ref().map(|s| s.text()).unwrap_or_default();
                            reply.relative().child(self.copyable(text, cx))
                        }))
                        .children(self.image_cards(id, index, turn, colors, cx))
                        .children(self.page_cards(index, turn, colors, cx))
                }
            })
            .children(ending)
    }

    fn failed_prompt(
        &self,
        id: SessionId,
        index: usize,
        failed: &crate::live::FailedPrompt,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let request_id = failed.prompt.request_id.clone();
        ui::card(colors)
            .p(px(16.))
            .border_color(colors.coral)
            .child(ui::label("Message not sent · text and images kept", colors))
            .child(ui::hint(failed.error.clone(), colors).my(px(8.)))
            .child(
                div()
                    .text_size(px(14.))
                    .max_h(px(80.))
                    .overflow_hidden()
                    .child(failed.prompt.label()),
            )
            .child(
                ui::button(
                    ("recover-prompt", index),
                    Button::Plain,
                    Some("pencil"),
                    "Edit and retry",
                    false,
                    colors,
                )
                .mt(px(12.))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.recover_prompt(id, &request_id, window, cx)
                })),
            )
    }

    fn queued_messages(
        &self,
        id: SessionId,
        queued: &[String],
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        div()
            .child(ui::label("Queued messages", colors).mb(px(8.)))
            .child(
                ui::card(colors).children(queued.iter().enumerate().map(|(index, prompt)| {
                    ui::row(("queued", index), index == 0, colors)
                        .child(icon("queue", 16., colors.muted))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(14.))
                                .child(prompt.clone()),
                        )
                        .child(
                            ui::tap(("unqueue", index), "x", colors)
                                .size(px(40.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(store) = &mut this.store
                                        && let Err(error) = store.unqueue(id, index)
                                    {
                                        this.notify_user(error, cx);
                                    }
                                    cx.notify();
                                })),
                        )
                })),
            )
    }

    /// A reply with pictures or pages between Pi's words, in the order Pi
    /// made them.
    fn flow(
        &self,
        id: SessionId,
        index: usize,
        turn: &Turn,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        turn.flow
            .iter()
            .enumerate()
            .filter_map(|(n, step)| match step {
                Flow::Text(text) => Some(
                    div()
                        .relative()
                        .min_w_0()
                        .child(crate::message::render(text, colors))
                        .child(self.copyable(text.clone(), cx))
                        .into_any_element(),
                ),
                Flow::Image(n) => self
                    .image_card(id, index, turn, *n, colors, cx)
                    .map(IntoElement::into_any_element),
                Flow::Page(n) => self
                    .page_card(index, turn, *n, colors, cx)
                    .map(IntoElement::into_any_element),
                Flow::Handoff(n) => self
                    .handoff_card(id, index, turn, *n, colors, cx)
                    .map(IntoElement::into_any_element),
                Flow::Compacted(summary) => Some(
                    self.compacted(id, index, n, summary, colors, cx)
                        .into_any_element(),
                ),
            })
            .collect()
    }

    /// Where Pi's context was summarized: a line across the thread, and the
    /// summary Pi works from after it, when tapped.
    fn compacted(
        &self,
        id: SessionId,
        index: usize,
        step: usize,
        summary: &str,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let open = self.expanded_compactions.contains(&(id, index, step));
        let rule = || div().flex_1().h(px(1.)).bg(colors.line);
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .id(("compacted", step))
                    .debug_selector(|| "compacted".into())
                    .min_h(px(40.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(8.))
                    .text_size(px(12.5))
                    .text_color(colors.muted)
                    .active(|style| style.bg(colors.selected))
                    .child(rule())
                    .child(icon("layers", 12., colors.muted))
                    .child(if open {
                        "Context summarized · hide"
                    } else {
                        "Context summarized · show summary"
                    })
                    .child(rule())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(list) = this.thread_lists.get(&id) {
                            list.pause_following_tail();
                        }
                        if !this.expanded_compactions.remove(&(id, index, step)) {
                            this.expanded_compactions.insert((id, index, step));
                        }
                        this.remeasure_thread_turn(id, index);
                        cx.notify();
                    })),
            )
            .when(open, |column| {
                column.child(
                    ui::card(colors)
                        .p(px(14.))
                        .min_w_0()
                        .child(crate::message::render(summary, colors)),
                )
            })
    }

    /// What was asked, on the right. A finished prompt rests on one line and
    /// opens in full when tapped.
    #[allow(clippy::too_many_arguments)]
    fn prompt(
        &self,
        id: SessionId,
        index: usize,
        turn: &Turn,
        live: bool,
        numbered: bool,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        // Subagents' answers can run long: they rest on one line even while Pi reads them.
        let folded = (!live || turn.reported) && !self.expanded_prompts.contains(&(id, index));
        div().flex().justify_end().child(
            div()
                .id(("prompt", index))
                .max_w(relative(0.86))
                .min_w_0()
                .px(px(16.))
                .py(px(12.))
                .rounded_tl(px(20.))
                .rounded_tr(px(20.))
                .rounded_bl(px(20.))
                .rounded_br(px(4.))
                .relative()
                .bg(colors.panel)
                .line_height(px(22.))
                .when(turn.reported, |prompt| {
                    prompt.child(
                        div()
                            .mb(px(4.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(12.5))
                            .line_height(px(16.))
                            .text_color(colors.muted)
                            .child(icon("fork", 12., colors.muted))
                            .child(format!("From subagents · {}", turn.at)),
                    )
                })
                .when(
                    !turn.reported && (numbered || !turn.pages.is_empty()),
                    |prompt| {
                        prompt.child(
                            div()
                                .mb(px(4.))
                                .text_size(px(12.5))
                                .line_height(px(16.))
                                .text_color(colors.muted)
                                .child(format!("Turn {} · {}", index + 1, turn.at)),
                        )
                    },
                )
                .child(match turn.prompt.split_once("\n\n") {
                    // Opened, subagents' answers read as the markdown they wrote.
                    Some((_, said)) if turn.reported && !folded => div()
                        .min_w_0()
                        .child(crate::message::render(said, colors))
                        .into_any_element(),
                    _ => div()
                        .min_w_0()
                        .when(folded, |text| text.truncate())
                        .child(if folded {
                            turn.prompt.lines().next().unwrap_or("").to_owned()
                        } else {
                            turn.prompt.clone()
                        })
                        .into_any_element(),
                })
                .when(!turn.attachments.is_empty(), |prompt| {
                    prompt.child(
                        div().mt(px(8.)).flex().flex_wrap().gap(px(8.)).children(
                            turn.attachments
                                .iter()
                                .enumerate()
                                .map(|(index, attachment)| {
                                    small_chip(
                                        ("sent", index),
                                        Some("clip"),
                                        attachment.clone(),
                                        colors,
                                    )
                                }),
                        ),
                    )
                })
                .child(self.copyable(turn.prompt.clone(), cx))
                .when(!live || turn.reported, |prompt| {
                    prompt.on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(list) = this.thread_lists.get(&id) {
                            list.pause_following_tail();
                        }
                        if !this.expanded_prompts.remove(&(id, index)) {
                            this.expanded_prompts.insert((id, index));
                        }
                        this.remeasure_thread_turn(id, index);
                        cx.notify();
                    }))
                }),
        )
    }

    /// A finished run as one line, with each stretch's time under it, or a key
    /// that says what each did when nothing else does (a page). Tapping it
    /// opens the stations.
    pub(crate) fn turn_activity(
        &self,
        id: SessionId,
        index: usize,
        turn: &Turn,
        live: bool,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        if turn.stages.is_empty() {
            return div();
        }
        let expanded = self
            .expanded_turns
            .get(&(id, index))
            .copied()
            .unwrap_or(live);
        let (times, timed) = run_times(turn);
        let summary = if turn.pages.is_empty() {
            ui::run_line(&times, timed, colors)
        } else {
            let entries = turn
                .stages
                .iter()
                .filter(|stage| {
                    stage.status == StageStatus::Done && !times[stage.kind.index()].is_zero()
                })
                .map(|stage| (stage.kind, key_label(stage)))
                .collect();
            ui::run_line(&times, false, colors).child(ui::run_key(entries, &times, colors))
        };
        div()
            .min_w_0()
            .child(
                div()
                    .id(("expand-turn", index))
                    .debug_selector(|| "expand-turn".into())
                    .relative()
                    .child(crate::testing::probe(format!("turn-{index}-activity")))
                    .rounded(px(8.))
                    .active(|style| style.bg(colors.selected))
                    .child(summary)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(list) = this.thread_lists.get(&id) {
                            list.pause_following_tail();
                        }
                        this.expanded_turns.insert((id, index), !expanded);
                        this.remeasure_thread_turn(id, index);
                        cx.notify();
                    })),
            )
            .when(expanded, |body| {
                body.child(
                    self.stations_of(id, None, index, turn, false, colors, cx)
                        .mt(px(16.)),
                )
            })
    }

    fn stations(
        &self,
        session: &Session,
        index: usize,
        turn: &Turn,
        live: bool,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        self.stations_of(session.id, Some(session), index, turn, live, colors, cx)
    }

    /// The run line down the gutter: a station per stage with its icon, and
    /// under each what Pi did. Stages ahead are dashed; a hand marks a run
    /// waiting for you.
    #[allow(clippy::too_many_arguments)]
    fn stations_of(
        &self,
        id: SessionId,
        session: Option<&Session>,
        index: usize,
        turn: &Turn,
        live: bool,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let waiting = live && session.is_some_and(|session| session.state == State::NeedsYou);
        // Waiting for you is where the run stands: nothing after it shows yet.
        let shown = if waiting {
            turn.stages
                .iter()
                .position(|stage| stage.status == StageStatus::Live)
                .map_or(turn.stages.len(), |live| live + 1)
        } else {
            turn.stages.len()
        };
        let stages = &turn.stages[..shown];
        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .children(stages.iter().enumerate().map(|(stage_index, stage)| {
                let next = stages.get(stage_index + 1);
                let kind = stage.kind;
                let waits = waiting && stage.status == StageStatus::Live;
                let row = if waits {
                    let session = session.expect("a waiting session");
                    waiting_row(session, next.is_some(), colors)
                } else {
                    stage_row(stage, turn, index, stage_index, next.is_some(), colors)
                };
                div()
                    .id(("stage-details", stage_index))
                    .debug_selector(|| "stage-details".into())
                    .relative()
                    .child(crate::testing::probe(format!(
                        "turn-{index}-stage-{stage_index}"
                    )))
                    .rounded(px(12.))
                    .active(|style| style.bg(colors.selected))
                    .child(row)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_sheet(Sheet::Activity(id, index, kind), cx)
                    }))
            }))
    }

    /// The report card under a finished run: each file with its change size,
    /// and the check.
    fn ending(&self, session: &Session, turn_index: usize, cx: &Context<Self>) -> Div {
        let colors = theme(cx);
        let id = session.id;
        let has_files = !session.files.is_empty();
        let check = session.check.as_ref().map(|check| {
            let (color, result) = match check.result {
                CheckResult::Passed => (colors.green, "Passed"),
                CheckResult::Failed => (colors.coral, "Failed"),
                CheckResult::NotRun => (colors.muted, "Not run"),
            };
            div()
                .id("check")
                .min_h(px(56.))
                .flex()
                .items_center()
                .gap(px(12.))
                .when(has_files, |row| row.border_t_1().border_color(colors.line))
                .child(icon("shield", 16., color))
                .child(
                    ui::mono(check.command.clone(), 12.5)
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(colors.secondary),
                )
                .child(plain_badge(result, color, &colors))
        });
        let all = self.all_files.contains(&id);
        let hidden = session.files.len().saturating_sub(FILES);
        let shown = if all { session.files.len() } else { FILES };
        let fold = (hidden > 0).then(|| {
            div()
                .id("more-files")
                .relative()
                .child(crate::testing::probe("more-files"))
                .min_h(px(48.))
                .flex()
                .items_center()
                .gap(px(12.))
                .border_t_1()
                .border_color(colors.line)
                .text_size(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors.accent)
                .active(|style| style.bg(colors.selected))
                .child(icon(
                    if all { "chev_u" } else { "chev_d" },
                    16.,
                    colors.accent,
                ))
                .child(if all {
                    "Show fewer".to_owned()
                } else if hidden == 1 {
                    "Show 1 more file".to_owned()
                } else {
                    format!("Show {hidden} more files")
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(list) = this.thread_lists.get(&id) {
                        list.pause_following_tail();
                    }
                    if !this.all_files.remove(&id) {
                        this.all_files.insert(id);
                    }
                    this.remeasure_thread_turn(id, turn_index);
                    cx.notify();
                }))
        });
        let files = session
            .files
            .iter()
            .take(shown)
            .enumerate()
            .map(|(index, file)| {
                let figure = |text: String, width: f32, color: Hsla| {
                    div()
                        .w(px(width))
                        .flex_none()
                        .flex()
                        .justify_end()
                        .font_family(MONO)
                        .text_size(px(12.5))
                        .text_color(color)
                        .child(text)
                };
                div()
                    .id(("file", index))
                    .min_h(px(56.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .when(index > 0, |row| row.border_t_1().border_color(colors.line))
                    .active(|style| style.bg(colors.selected))
                    .child(icon("file", 16., colors.muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(file.name().to_owned()),
                    )
                    .child(ui::blocks(file.added, file.removed, &colors))
                    .child(figure(
                        if file.added > 0 {
                            format!("+{}", file.added)
                        } else {
                            String::new()
                        },
                        32.,
                        colors.green,
                    ))
                    .child(figure(
                        if file.removed > 0 {
                            format!("−{}", file.removed)
                        } else {
                            String::new()
                        },
                        20.,
                        colors.coral,
                    ))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_review(id, index, window, cx)
                    }))
            });
        let failure = (session.state == State::Failed).then(|| {
            ui::card(&colors)
                .border_color(colors.coral.opacity(0.4))
                .p(px(16.))
                .flex()
                .gap(px(12.))
                .child(icon("alert", 16., colors.coral))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(14.))
                        .child(capitalized(&session.status_line()))
                        .child(ui::hint(
                            "The run failed. Any changes already made remain on the computer. Ask again to retry.",
                            &colors,
                        )),
                )
        });
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .children(failure)
            .when(session.state == State::Stopped, |ending| {
                ending.child(ui::hint(
                    "Stopped by you. Changes made before stopping stay on the computer.",
                    &colors,
                ))
            })
            .when(has_files || check.is_some(), |ending| {
                ending.child(
                    ui::card(&colors)
                        .px(px(16.))
                        .children(files)
                        .children(fold)
                        .children(check),
                )
            })
    }

    /// Pi's question in the composer's place, so the run so far stays
    /// readable. Only Answer answers.
    fn question_card(
        &self,
        session: &Session,
        computer: &str,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> AnyElement {
        let id = session.id;
        let Some(question) = session.question.as_ref() else {
            return div().into_any_element();
        };
        let choices = question.choices.iter().enumerate().map(|(index, choice)| {
            let on = self.choice == Some(choice.answer);
            let answer = choice.answer;
            div()
                .id(("choice", index))
                .min_h(px(48.))
                .px(px(12.))
                .py(px(6.))
                .flex()
                .items_center()
                .gap(px(12.))
                .rounded(px(12.))
                .when(on, |row| row.bg(colors.selected))
                .active(|style| style.bg(colors.selected))
                .child(
                    div()
                        .size(px(20.))
                        .flex_none()
                        .rounded(px(10.))
                        .border(px(if on { 6. } else { 2. }))
                        .border_color(if on {
                            colors.accent
                        } else {
                            colors.line_strong
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(choice.label.clone())
                        .children(choice.detail.clone().map(|detail| {
                            div()
                                .text_size(px(12.5))
                                .line_height(px(16.))
                                .text_color(colors.muted)
                                .child(detail)
                        })),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.choice = Some(answer);
                    cx.notify();
                }))
        });
        let answer = ui::button("answer", Button::Primary, None, "Answer", false, colors).flex_1();
        let answer = if self.choice.is_some() {
            answer.on_click(cx.listener(move |this, _, window, cx| this.answer(id, window, cx)))
        } else {
            ui::disabled(answer, colors)
        };
        div()
            .id("question-card")
            .relative()
            .child(crate::testing::probe("question-card"))
            .occlude()
            .pt(px(16.))
            .px(px(16.))
            .pb(px(12.))
            .rounded(px(24.))
            .bg(colors.composer)
            .border_1()
            .border_color(colors.line.blend(colors.wait.opacity(0.5)))
            .shadow(vec![gpui::BoxShadow {
                color: colors.shadow,
                offset: gpui::point(px(0.), px(-8.)),
                blur_radius: px(32.),
                spread_radius: px(0.),
                inset: false,
            }])
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(ui::badge("Needs you", colors.wait, colors.amber, colors))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .justify_end()
                            .text_size(px(12.5))
                            .line_height(px(16.))
                            .text_color(colors.muted)
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .child(format!("{computer} · {}", session.folder)),
                            ),
                    ),
            )
            .child(
                div()
                    .mt(px(12.))
                    .text_size(px(18.))
                    .line_height(px(24.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(question.title.clone()),
            )
            .when(!question.command.is_empty(), |card| {
                card.child(
                    div()
                        .relative()
                        .child(self.copyable(question.command.clone(), cx))
                        .mt(px(12.))
                        .px(px(12.))
                        .py(px(9.))
                        .rounded(px(12.))
                        .bg(colors.panel)
                        .font_family(MONO)
                        .text_size(px(12.5))
                        .line_height(px(22.))
                        .text_color(colors.plain)
                        .child(question.command.clone()),
                )
            })
            .child(
                div()
                    .mt(px(12.))
                    .mx(px(-4.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .children(choices),
            )
            .child(
                div()
                    .mt(px(12.))
                    .flex()
                    .gap(px(8.))
                    .child(
                        ui::button("later", Button::Quiet, None, "Later", false, colors)
                            .px(px(20.))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.questions_later.insert(id);
                                cx.notify();
                            })),
                    )
                    .child(answer),
            )
            .into_any_element()
    }
}

/// Each stage's time, or equal stretches for the stages done when the
/// computer gave no times.
fn run_times(turn: &Turn) -> ([Duration; 4], bool) {
    if turn.times.iter().any(|time| !time.is_zero()) {
        return (turn.times, true);
    }
    let mut times = [Duration::ZERO; 4];
    for stage in &turn.stages {
        if stage.status == StageStatus::Done {
            times[stage.kind.index()] = Duration::from_secs(1);
        }
    }
    (times, false)
}

/// What a stage did, for a key under a finished line: "Wrote aurora.html".
fn key_label(stage: &Stage) -> SharedString {
    match stage.kind {
        StageKind::HandOff => stage.kind.name(StageStatus::Done).into(),
        _ => stage.what.clone().into(),
    }
}

/// A 28 dp chip for a file or a search.
fn small_chip(
    id: impl Into<gpui::ElementId>,
    glyph: Option<&str>,
    text: impl Into<SharedString>,
    colors: &Theme,
) -> gpui::Stateful<Div> {
    ui::chip(id, glyph, text, colors)
        .h(px(28.))
        .px(px(10.))
        .text_size(px(12.5))
}

/// A badge without its dot: "Passed".
fn plain_badge(text: &'static str, hue: Hsla, colors: &Theme) -> Div {
    div()
        .h(px(24.))
        .px(px(10.))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(12.))
        .bg(colors.tint(hue))
        .text_color(hue)
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(text)
}

/// The stretch of line from a station down to the next: solid in the stage's
/// colour once done, dashed ahead.
fn connector(hue: Hsla, done: bool, colors: &Theme) -> Div {
    let line = div().absolute().top(px(32.)).bottom(px(-20.));
    if done {
        line.left(px(12.5)).w(px(3.)).rounded(px(2.)).bg(hue)
    } else {
        // Dashed: a one-sided dashed border does not draw this thin.
        line.left(px(13.))
            .w(px(2.))
            .overflow_hidden()
            .flex()
            .flex_col()
            .gap(px(3.))
            .children((0..80).map(|_| div().flex_none().h(px(4.)).bg(colors.line_strong)))
    }
}

fn meta(text: impl Into<SharedString>, colors: &Theme) -> Div {
    div()
        .text_size(px(12.5))
        .line_height(px(16.))
        .text_color(colors.muted)
        .child(text.into())
}

fn counts(stage: &Stage, colors: &Theme) -> Div {
    div()
        .flex()
        .flex_none()
        .gap(px(8.))
        .font_family(MONO)
        .text_size(px(12.5))
        .when(stage.added > 0, |counts| {
            counts.child(
                div()
                    .text_color(colors.green)
                    .child(format!("+{}", stage.added)),
            )
        })
        .when(stage.removed > 0, |counts| {
            counts.child(
                div()
                    .text_color(colors.coral)
                    .child(format!("−{}", stage.removed)),
            )
        })
}

pub(crate) fn stage_row(
    stage: &Stage,
    turn: &Turn,
    turn_index: usize,
    stage_index: usize,
    line_below: bool,
    colors: &Theme,
) -> Div {
    let hue = ui::stage_hue(stage.kind, colors);
    let ahead = matches!(stage.status, StageStatus::Planned | StageStatus::Skipped);
    let row = div()
        .relative()
        .flex()
        .gap(px(12.))
        .when(line_below, |row| {
            row.child(connector(hue, stage.status == StageStatus::Done, colors))
        })
        .child(ui::station(stage.kind.glyph(), hue, stage.status, colors));
    if ahead {
        return row.items_center().child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_baseline()
                .gap(px(8.))
                .child(
                    div()
                        .flex_none()
                        .text_color(colors.faint)
                        .child(stage.kind.name(stage.status)),
                )
                .child(
                    meta(stage.what.clone(), colors)
                        .text_color(colors.faint)
                        .truncate(),
                ),
        );
    }
    let changed = stage.added > 0 || stage.removed > 0;
    let time = turn.time(stage.kind);
    let live_diff = stage.kind == StageKind::Change
        && stage.status == StageStatus::Live
        && !stage.diff.is_empty();
    let references = if live_diff {
        &[][..]
    } else {
        &stage.references[..]
    };
    let more = references.len().saturating_sub(CHIPS);
    let chips = (!references.is_empty()).then(|| {
        div()
            .mt(px(12.))
            .flex()
            .flex_wrap()
            .gap(px(8.))
            .children(
                references
                    .iter()
                    .take(CHIPS)
                    .enumerate()
                    .map(|(index, reference)| {
                        let (glyph, text) = match reference {
                            Reference::File(file) => {
                                ("file", file.rsplit('/').next().unwrap_or(file).to_owned())
                            }
                            Reference::Search(search) => ("search", search.clone()),
                        };
                        small_chip(("reference", index), Some(glyph), text, colors)
                    }),
            )
            .when(more > 0, |chips| {
                chips.child(
                    small_chip("more-references", None, format!("+{more} more"), colors)
                        .text_color(colors.accent)
                        .font_weight(FontWeight::SEMIBOLD),
                )
            })
    });
    let diff = live_diff.then(|| {
        ui::card(colors)
            .relative()
            .child(crate::testing::probe(format!(
                "turn-{turn_index}-stage-{stage_index}-live-diff-{}",
                stage.diff.len()
            )))
            .mt(px(12.))
            .py(px(4.))
            .rounded(px(12.))
            // A glance at the change: long lines are cut, Review shows them.
            .overflow_hidden()
            .children(stage.diff_path.as_ref().map(|path| {
                div()
                    .px(px(10.))
                    .pt(px(6.))
                    .pb(px(4.))
                    .font_family(MONO)
                    .text_size(px(11.5))
                    .line_height(px(16.))
                    .text_color(colors.muted)
                    .child(path.clone())
            }))
            .children(
                stage
                    .diff
                    .iter()
                    .map(|line| ui::code_line(line, false, 40., false, colors)),
            )
    });
    let output = (stage.status == StageStatus::Live)
        .then(|| stage.tools.iter().rev().find(|t| !t.output.is_empty()))
        .flatten()
        .map(|tool| {
            // The last lines, each on one line, in a box that never changes
            // height: streaming output must not move the thread under it.
            let displayed = tool.output_for_display();
            let lines: Vec<&str> = displayed.lines().rev().take(TAIL).collect();
            div()
                .mt(px(12.))
                .px(px(12.))
                .py(px(8.))
                .h(px(TAIL as f32 * 18. + 16.))
                .overflow_hidden()
                .rounded(px(8.))
                .bg(colors.panel)
                .flex()
                .flex_col()
                .justify_end()
                .children(lines.into_iter().rev().map(|line| {
                    ui::mono(line.to_owned(), 12.)
                        .h(px(18.))
                        .line_height(px(18.))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_color(colors.secondary)
                }))
        });
    row.child(
        div()
            .flex_1()
            .min_w_0()
            .pt(px(4.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(stage.kind.name(stage.status)),
                    )
                    .map(|title| {
                        if changed {
                            title.child(counts(stage, colors))
                        } else if !time.is_zero() {
                            title.child(meta(duration_label(time), colors))
                        } else {
                            title
                        }
                    }),
            )
            .child(meta(stage.what.clone(), colors).mt(px(2.)))
            .children(output)
            .children(chips)
            .children(diff),
    )
}

/// A run paused on a question: the hand, in the waiting colour.
fn waiting_row(session: &Session, line_below: bool, colors: &Theme) -> Div {
    let command = session
        .question
        .as_ref()
        .is_some_and(|question| !question.command.is_empty());
    div()
        .relative()
        .flex()
        .gap(px(12.))
        .when(line_below, |row| {
            row.child(connector(colors.wait, false, colors))
        })
        .child(ui::station("hand", colors.wait, StageStatus::Live, colors))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .pt(px(4.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Waiting for you"),
                        )
                        .child(meta(duration_label(session.elapsed), colors)),
                )
                .child(
                    meta(
                        if command {
                            "Pi paused before running a command"
                        } else {
                            "Pi paused for your answer"
                        },
                        colors,
                    )
                    .mt(px(2.)),
                ),
        )
}

fn reply(turn: &Turn, latest: bool, colors: &Theme) -> Option<Div> {
    turn.summary.as_ref().map(|summary| {
        if let Some(source) = summary.source.as_deref() {
            return div()
                .min_w_0()
                .child(crate::message::render(source, colors));
        }
        div()
            .min_w_0()
            .when(!summary.headline.is_empty(), |reply| {
                reply.child(
                    ui::serif(summary.headline.clone(), if latest { 22. } else { 18. })
                        .line_height(px(if latest { 28. } else { 24. }))
                        .text_color(colors.text),
                )
            })
            .when(!summary.body.is_empty(), |reply| {
                reply.child(
                    div()
                        .when(!summary.headline.is_empty(), |body| body.mt(px(12.)))
                        .min_w_0()
                        .text_size(px(15.))
                        .line_height(px(22.))
                        .text_color(colors.secondary)
                        .child(summary.body.clone()),
                )
            })
    })
}

/// Working, the time and Stop, on the composer's top edge.
fn working_strip(
    session: &Session,
    stopping: bool,
    compacting: bool,
    colors: &Theme,
    cx: &Context<PhoneApp>,
) -> Div {
    let id = session.id;
    let waiting = !compacting && session.state == State::NeedsYou;
    let status = composer_status(session, compacting);
    let stop = div()
        .id("stop")
        .relative()
        .child(crate::testing::probe("stop"))
        .debug_selector(|| "composer-stop".into())
        .h(px(40.))
        .px(px(12.))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .rounded(px(20.))
        .text_size(px(14.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(colors.coral)
        .active(|style| style.bg(colors.selected))
        .child(icon("stop", 16., colors.coral))
        .child("Stop")
        .when(stopping, |button| button.opacity(0.5))
        .when(!stopping, |button| {
            button.on_click(cx.listener(move |this, _, _, cx| this.stop(id, cx)))
        });
    div()
        .mx(px(12.))
        .mb(px(-1.))
        .min_h(px(48.))
        .pl(px(16.))
        .pr(px(8.))
        .flex()
        .items_center()
        .gap(px(12.))
        .bg(colors.panel)
        .border_1()
        .border_b_0()
        .border_color(colors.line_strong)
        .rounded_t(px(24.))
        .child(ui::ring_dot(if waiting {
            colors.wait
        } else {
            colors.read
        }))
        .child(
            div()
                .flex_none()
                .text_size(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(status),
        )
        .child(meta(duration_label(session.elapsed), colors).flex_1())
        .when(waiting, |strip| {
            strip.child(
                ui::button("answer", Button::Primary, None, "Answer", true, colors).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.questions_later.remove(&id);
                        cx.notify();
                    }),
                ),
            )
        })
        .child(stop)
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Render, TestAppContext};
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    struct TallTurns {
        state: gpui::ListState,
    }

    impl Render for TallTurns {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let rows = list(
                self.state.clone(),
                cx.processor(|_, _, _, _| div().h(px(1000.)).into_any_element()),
            )
            .size_full();
            div().size_full().child(rows)
        }
    }

    struct SizedTurns {
        state: gpui::ListState,
        heights: Rc<RefCell<Vec<gpui::Pixels>>>,
    }

    impl Render for SizedTurns {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let heights = self.heights.clone();
            let rows = list(
                self.state.clone(),
                cx.processor(move |_, index, _, _| {
                    div().h(heights.borrow()[index]).into_any_element()
                }),
            )
            .size_full();
            div().size_full().child(rows)
        }
    }

    #[test]
    fn only_large_first_loads_get_the_measurement_indicator() {
        assert!(!large_thread(LARGE_THREAD_ITEMS - 1));
        assert!(large_thread(LARGE_THREAD_ITEMS));
    }

    #[gpui::test]
    fn large_thread_loading_waits_for_a_visible_presented_frame(cx: &mut TestAppContext) {
        let id = SessionId(1);
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.open_store("sample@computer.local");
                app.visible = true;
                app.routes = vec![Route::Sessions, Route::Thread(id)];
                app.thread_loading.insert(id, 1);
                assert!(app.thread_is_loading(id, LARGE_THREAD_ITEMS, window, cx));
                assert_eq!(app.thread_loading_release_scheduled.get(&id), Some(&1));
            })
        });

        // Time alone cannot release a loader that has not reached the next frame.
        cx.executor().advance_clock(THREAD_LOADING_PAINT);
        cx.run_until_parked();
        assert!(app.read_with(cx, |app, _| app.thread_loading.contains_key(&id)));

        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            app.update(cx, |app, _| app.routes = vec![Route::Sessions]);
        });
        cx.executor().advance_clock(THREAD_LOADING_PAINT);
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.thread_loading.contains_key(&id));
            assert!(!app.thread_loading_release_scheduled.contains_key(&id));
        });

        // Returning to the thread presents its loader again before release.
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.routes.push(Route::Thread(id));
                assert!(app.thread_is_loading(id, LARGE_THREAD_ITEMS, window, cx));
            })
        });
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
        });
        cx.executor().advance_clock(THREAD_LOADING_PAINT);
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(!app.thread_loading.contains_key(&id));
            assert!(!app.thread_loading_release_scheduled.contains_key(&id));
        });
    }

    #[gpui::test]
    fn hidden_thread_keeps_loader_until_the_visibility_observer_redraws(cx: &mut TestAppContext) {
        let id = SessionId(1);
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.open_store("sample@computer.local");
                app.visible = true;
                app.routes = vec![Route::Sessions, Route::Thread(id)];
                let session = app
                    .store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|session| session.id == id)
                    .unwrap();
                let turn = session.turns[0].clone();
                while session.turns.len() < LARGE_THREAD_ITEMS {
                    session.turns.push(turn.clone());
                }
                app.thread_loading.insert(id, 1);
                assert!(app.thread_is_loading(id, LARGE_THREAD_ITEMS, window, cx));
            })
        });
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
        });
        cx.simulate_visibility_change(gpui::WindowVisibility::Hidden);

        cx.executor().advance_clock(THREAD_LOADING_PAINT);
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(!app.visible);
            assert!(app.thread_loading.contains_key(&id));
            assert!(!app.thread_loading_release_scheduled.contains_key(&id));
        });

        let notifications = Rc::new(Cell::new(0));
        let _visibility_observer = cx.update(|_, cx| {
            cx.observe(&app, {
                let notifications = notifications.clone();
                move |_, _| notifications.set(notifications.get() + 1)
            })
        });
        cx.simulate_visibility_change(gpui::WindowVisibility::Visible);
        cx.run_until_parked();
        assert!(notifications.get() > 0);
        assert!(app.read_with(cx, |app, _| app.visible));

        // Draw the invalidated foreground frame through the real thread screen;
        // it must show the loader and arm release only for the following frame.
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        app.read_with(cx, |app, _| {
            assert!(app.thread_loading.contains_key(&id));
            assert_eq!(app.thread_loading_release_scheduled.get(&id), Some(&1));
        });
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
        });
        cx.executor().advance_clock(THREAD_LOADING_PAINT);
        cx.run_until_parked();
        assert!(!app.read_with(cx, |app, _| app.thread_loading.contains_key(&id)));
    }

    #[gpui::test]
    fn stale_loading_timer_cannot_release_a_new_generation(cx: &mut TestAppContext) {
        let id = SessionId(1);
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.open_store("sample@computer.local");
                app.visible = true;
                app.routes = vec![Route::Sessions, Route::Thread(id)];
                app.thread_loading.insert(id, 1);
                assert!(app.thread_is_loading(id, LARGE_THREAD_ITEMS, window, cx));
            })
        });
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            app.update(cx, |app, cx| {
                app.thread_loading.insert(id, 2);
                assert!(app.thread_is_loading(id, LARGE_THREAD_ITEMS, window, cx));
            });
        });

        cx.executor().advance_clock(THREAD_LOADING_PAINT);
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.thread_loading.get(&id), Some(&2));
            assert_eq!(app.thread_loading_release_scheduled.get(&id), Some(&2));
        });

        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
        });
        cx.executor().advance_clock(THREAD_LOADING_PAINT);
        cx.run_until_parked();
        assert!(!app.read_with(cx, |app, _| app.thread_loading.contains_key(&id)));
    }

    #[test]
    fn pending_compaction_immediately_joins_and_labels_the_composer() {
        let mut session = crate::demo::new_session(SessionId(1), 0, "Test".into(), Vec::new());
        session.state = State::Done;
        session.activity.clear();

        assert!(!thread_running(&session, false));
        assert!(thread_running(&session, true));
        assert_eq!(composer_status(&session, true), "Compacting");
    }

    #[test]
    fn retained_thread_layout_skips_time_only_remeasurement() {
        let mut before = crate::demo::new_session(SessionId(1), 0, "Test".into(), Vec::new());
        before.turns[0].times[0] = Duration::from_secs(1);
        let mut after = before.clone();
        after.elapsed = Duration::from_secs(10);
        after.turns[0].times[0] = Duration::from_secs(10);

        let changes = thread_layout_changes(&before, &after);
        assert!(changes.turns.is_empty());
        assert!(!changes.queue);
    }

    #[test]
    fn first_real_timing_line_remeasures_the_finished_turn() {
        let before = crate::demo::new_session(SessionId(1), 0, "Test".into(), Vec::new());
        let mut after = before.clone();
        after.turns[0].times[0] = Duration::from_secs(1);

        assert_eq!(thread_layout_changes(&before, &after).turns, [0]);
    }

    #[test]
    fn changed_thread_content_remeasures_only_affected_rows() {
        let mut before = crate::demo::new_session(SessionId(1), 0, "First".into(), Vec::new());
        before.queued = vec!["old follow-up".into()];
        let mut after = before.clone();
        after.turns[0].summary = Some(crate::model::Summary {
            headline: "Done".into(),
            body: "A new reply".into(),
            source: None,
        });
        after.queued = vec!["new follow-up".into()];

        let changes = thread_layout_changes(&before, &after);
        assert_eq!(changes.turns, [0]);
        assert!(changes.queue);
    }

    #[test]
    fn changed_failure_detail_remeasures_the_ending_card() {
        let mut before = crate::demo::new_session(SessionId(1), 0, "Test".into(), Vec::new());
        before.state = State::Failed;
        before.failure = Some("brief failure".into());
        let mut after = before.clone();
        after.failure = Some("a much longer failure detail that can wrap".into());
        assert_eq!(thread_layout_changes(&before, &after).turns, [0]);
    }

    #[test]
    fn failed_prompt_text_changes_remeasure_the_same_row() {
        let before = vec![("old error".into(), "prompt".into())];
        let after = vec![("a longer error".into(), "prompt".into())];
        assert_eq!(changed_failed_layouts(Some(&before), &after), [0]);
        assert!(changed_failed_layouts(None, &after).is_empty());
    }

    #[test]
    fn a_shape_change_during_a_fling_is_remeasured_on_settle() {
        let id = SessionId(1);
        let mut pending = HashSet::new();
        assert!(!thread_shape_remeasure_needed(&mut pending, id, true, true));
        assert!(pending.contains(&id));
        assert!(!thread_shape_remeasure_needed(
            &mut pending,
            id,
            true,
            false
        ));
        assert!(thread_shape_remeasure_needed(
            &mut pending,
            id,
            false,
            false
        ));
        assert!(!pending.contains(&id));
    }

    #[test]
    fn adding_a_second_turn_remeasures_the_numbered_first_prompt() {
        let before = crate::demo::new_session(SessionId(1), 0, "First".into(), Vec::new());
        let mut after = before.clone();
        after.turns.push(Turn::new("Second", "10:00"));

        let changes = thread_layout_changes(&before, &after);
        assert_eq!(changes.turns, [0]);
    }

    #[gpui::test]
    fn an_offscreen_inserted_status_row_updates_the_exact_scrollbar_range(cx: &mut TestAppContext) {
        let state = new_thread_list_state(3);
        let heights = Rc::new(RefCell::new(vec![px(1000.), px(1000.), px(1000.)]));
        let (view, cx) = cx.add_window_view({
            let state = state.clone();
            let heights = heights.clone();
            move |_, _| SizedTurns { state, heights }
        });
        cx.simulate_resize(gpui::size(px(320.), px(500.)));
        cx.run_until_parked();
        state.scroll_to(gpui::ListOffset {
            item_ix: 0,
            offset_in_item: px(0.),
        });

        heights.borrow_mut().push(px(400.));
        state.splice(3..3, 1);
        // Thread shape changes invalidate the preceding last row and every
        // failed/queued row, which also re-arms eager exact measurement.
        state.remeasure_items(thread_shape_remeasure_range(3, 4).unwrap());
        cx.update(|_, cx| view.update(cx, |_, cx| cx.notify()));
        cx.run_until_parked();

        assert_eq!(state.max_offset_for_scrollbar().y, px(2900.));
    }

    #[gpui::test]
    fn multi_turn_scrollbar_reaches_unseen_history_on_its_first_drag(cx: &mut TestAppContext) {
        let state = new_thread_list_state(3);
        state.set_follow_mode(FollowMode::Tail);
        let (_, cx) = cx.add_window_view({
            let state = state.clone();
            move |_, _| TallTurns { state }
        });
        cx.simulate_resize(gpui::size(px(320.), px(500.)));
        cx.run_until_parked();

        assert_eq!(state.max_offset_for_scrollbar().y, px(2500.));
        state.scrollbar_drag_started();
        state.set_offset_from_scrollbar(gpui::point(px(0.), px(0.)));
        state.scrollbar_drag_ended();
        assert_eq!(state.logical_scroll_top().item_ix, 0);
        assert_eq!(state.logical_scroll_top().offset_in_item, px(0.));
    }
}
