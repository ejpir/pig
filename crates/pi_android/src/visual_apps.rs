//! Native, explicitly requested mini-apps. Scripts have no phone/agent APIs.
use crate::visual_controls::{self, Gesture, Surface};
use crate::{
    app::PhoneApp,
    model::{Session, SessionId},
    theme::{MONO, Theme},
    ui,
};
use gpui::{
    AnyElement, ClipboardItem, Context, Div, FontWeight, Role, SharedString, Window, div, fill,
    point, prelude::*, px, size,
};
use pi_visual::{Node, Runtime, reply::Part};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    io::Read,
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    sync::Arc,
    time::{Duration, Instant},
};

const MAX_RUNNING: usize = 8;
const MAX_SAVED: usize = 32;
const MAX_SAVED_BYTES: usize = 256 * 1024;

struct Instance {
    runtime: Result<Runtime, String>,
    error: Option<String>,
    generation: u64,
    frame_pending: bool,
    last_frame: Option<Instant>,
    input_snapshot: Option<serde_json::Value>,
    barrier: visual_controls::ProjectionBarrier,
    last_input: Option<(String, [f64; 2])>,
    route: Option<crate::app::Route>,
    painted: RefCell<HashMap<String, Weak<Cell<bool>>>>,
    next_due: Option<Instant>,
}
impl Instance {
    fn cancel_manipulation(&mut self) {
        if !self.barrier.active() {
            return;
        }
        if let Ok(runtime) = &mut self.runtime {
            let result = self
                .input_snapshot
                .take()
                .map_or(Ok(()), |saved| runtime.restore_state(&saved));
            self.error = result.err().map(|e| short_error(&e));
        }
        self.input_snapshot = None;
        self.last_input = None;
        self.barrier.invalidate();
    }

    fn visible(&self, route: crate::app::Route) -> bool {
        self.route == Some(route)
            && self
                .painted
                .borrow()
                .values()
                .any(|p| p.upgrade().is_some_and(|p| p.get()))
    }
}
#[derive(Serialize, Deserialize)]
struct Saved {
    key: String,
    state: serde_json::Value,
}

#[derive(Default)]
pub(crate) struct VisualApps {
    running: HashMap<String, Instance>,
    order: VecDeque<String>,
    saved: Vec<Saved>,
    transient: HashMap<String, serde_json::Value>,
    path: Option<PathBuf>,
    generation: u64,
    replies: VecDeque<Arc<pi_visual::reply::Parsed>>,
}

impl VisualApps {
    pub(crate) fn load(prefs: Option<&Path>) -> Self {
        let path = prefs.map(|path| path.with_file_name("visual-app-state.json"));
        let saved = path
            .as_ref()
            .and_then(|path| {
                if !std::fs::symlink_metadata(path).ok()?.file_type().is_file() {
                    return None;
                }
                let mut options = std::fs::OpenOptions::new();
                options.read(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
                }
                let file = options.open(path).ok()?;
                if !file.metadata().ok()?.is_file() {
                    return None;
                }
                let mut bytes = Vec::new();
                file.take((MAX_SAVED_BYTES + 1) as u64)
                    .read_to_end(&mut bytes)
                    .ok()?;
                if bytes.len() > MAX_SAVED_BYTES {
                    return None;
                }
                let saved: Vec<Saved> = serde_json::from_slice(&bytes).ok()?;
                (saved.len() <= MAX_SAVED && saved.iter().all(|s| s.key.len() == 64))
                    .then_some(saved)
            })
            .unwrap_or_default();
        Self {
            saved,
            path,
            ..Default::default()
        }
    }

    fn parsed_reply(&mut self, source: &str) -> Arc<pi_visual::reply::Parsed> {
        let _trace = gpui_android::trace::scope(c"pi.visual.reply.lookup");
        if let Some(index) = self
            .replies
            .iter()
            .position(|cached| cached.source() == source)
        {
            let parsed = self.replies.remove(index).unwrap();
            self.replies.push_back(parsed.clone());
            return parsed;
        }
        let parsed = {
            let _trace = gpui_android::trace::scope(c"pi.visual.reply.parse");
            Arc::new(pi_visual::reply::Parsed::new(source))
        };
        // Cache only bounded immutable sources; streamed/oversized text stays inert.
        if source.len() <= 256 * 1024 {
            self.replies.push_back(parsed.clone());
            while self.replies.len() > 8
                || self.replies.iter().map(|p| p.source().len()).sum::<usize>() > 256 * 1024
            {
                self.replies.pop_front();
            }
        }
        parsed
    }

    /// Navigation, sheets and lifecycle invalidation must not leave a deferred
    /// protocol queue waiting on a retained previous-frame surface. Roll back
    /// before invalidating its lease; a stale release cannot cancel a newer input.
    pub(crate) fn projection_deferred(
        &mut self,
        route: crate::app::Route,
        available: bool,
    ) -> bool {
        let mut active = false;
        for instance in self.running.values_mut() {
            if !available || instance.route != Some(route) {
                instance.cancel_manipulation();
            } else {
                active |= instance.barrier.active();
            }
        }
        active
    }

    #[cfg(test)]
    pub(crate) fn test_projection_barrier(
        &mut self,
        route: crate::app::Route,
    ) -> visual_controls::ProjectionBarrier {
        self.ensure(
            "projection-fixture",
            "app({title:'Gate',state:{x:0},view:ui.slider('X','x',{min:0,max:100,step:1})});",
            false,
        );
        let instance = self.running.get_mut("projection-fixture").unwrap();
        instance.route = Some(route);
        instance.barrier.clone()
    }

    pub(crate) fn clear_running(&mut self) {
        self.running.clear();
        self.order.clear();
        self.transient.clear();
        self.replies.clear();
    }

    fn ensure(&mut self, key: &str, source: &str, restore: bool) {
        let _trace = gpui_android::trace::scope(c"pi.visual.runtime.lookup");
        self.order.retain(|old| old != key);
        self.order.push_back(key.to_owned());
        if self.running.contains_key(key) {
            return;
        }
        let _miss_trace = gpui_android::trace::scope(c"pi.visual.runtime.create");
        while self.running.len() >= MAX_RUNNING {
            if let Some(old) = self.order.pop_front() {
                self.running.remove(&old);
            }
        }
        let runtime = Runtime::from_bundle(source)
            .map(|mut runtime| {
                let saved = if restore {
                    self.saved.iter().find(|s| s.key == key).map(|s| &s.state)
                } else {
                    self.transient.get(key)
                };
                if let Some(saved) = saved {
                    let _ = runtime.restore_state(saved);
                }
                runtime
            })
            .map_err(|error| short_error(&error));
        self.generation = self.generation.wrapping_add(1);
        self.running.insert(
            key.to_owned(),
            Instance {
                runtime,
                error: None,
                generation: self.generation,
                frame_pending: false,
                last_frame: None,
                input_snapshot: None,
                barrier: visual_controls::ProjectionBarrier::default(),
                last_input: None,
                route: None,
                painted: RefCell::new(HashMap::new()),
                next_due: None,
            },
        );
    }

    fn remember_local(&mut self, key: &str, state: serde_json::Value, persist: bool) {
        if persist {
            self.remember(key, state);
        } else {
            self.transient.insert(key.to_owned(), state);
            while self.transient.len() > MAX_SAVED {
                if let Some(old) = self.transient.keys().next().cloned() {
                    self.transient.remove(&old);
                }
            }
        }
    }

    fn remember(&mut self, key: &str, state: serde_json::Value) {
        self.saved.retain(|saved| saved.key != key);
        self.saved.push(Saved {
            key: key.to_owned(),
            state,
        });
        while self.saved.len() > MAX_SAVED {
            self.saved.remove(0);
        }
        let mut bytes = serde_json::to_vec(&self.saved).unwrap_or_default();
        while bytes.len() > MAX_SAVED_BYTES && !self.saved.is_empty() {
            self.saved.remove(0);
            bytes = serde_json::to_vec(&self.saved).unwrap_or_default();
        }
        if let Some(path) = &self.path {
            let temporary = path.with_extension("tmp");
            if std::fs::write(&temporary, bytes)
                .and_then(|_| std::fs::rename(&temporary, path))
                .is_err()
            {
                log::warn!("Could not save local visual app state");
            }
        }
    }

    #[cfg(feature = "ui-test")]
    pub(crate) fn telemetry(&self) -> serde_json::Value {
        serde_json::Value::Array(self.running.values().filter_map(|instance| instance.runtime.as_ref().ok().map(|runtime| {
            serde_json::json!({"title":runtime.definition.title,"state":runtime.state_json().ok(),"error":instance.error,"drawing_failed":runtime.drawing_failed()})
        })).collect())
    }
}

// Keep fractional explicit caps in phase with vsync rather than rounding down
// to half-refresh rates. A delayed frame skips old deadlines (no catch-up burst).
fn advance_cap(due: &mut Option<Instant>, fps: u32, now: Instant) -> bool {
    let period = Duration::from_secs_f64(1.0 / f64::from(fps));
    let deadline = due.unwrap_or(now);
    if now < deadline {
        return false;
    }
    let next = deadline + period;
    *due = Some(if next > now { next } else { now + period });
    true
}

fn short_error(error: &anyhow::Error) -> String {
    format!("{error:#}").chars().take(240).collect()
}
fn identity(
    computer: &str,
    session: &str,
    turn: impl ToString,
    slot: usize,
    ordinal: usize,
    prompt: &str,
    source: &str,
) -> String {
    let mut hash = Sha256::new();
    for text in [
        computer,
        session,
        &turn.to_string(),
        &slot.to_string(),
        &ordinal.to_string(),
        prompt,
        source,
    ] {
        hash.update((text.len() as u64).to_le_bytes());
        hash.update(text.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

impl PhoneApp {
    /// One bounded timer per visible app. Never persist or remeasure on frames.
    fn queue_visual_frame(&mut self, key: &str, window: &Window, cx: &mut Context<Self>) {
        if !window.visibility().is_visible()
            || !window.is_window_active()
            || cx.reduce_motion()
            || self.sheet.is_some()
            || self.closing_sheet.is_some()
        {
            return;
        }
        let mut apps = self.visual_apps.borrow_mut();
        if apps
            .running
            .values()
            .filter(|i| i.frame_pending && i.visible(self.route()))
            .count()
            >= 2
        {
            return;
        }
        let Some(instance) = apps.running.get_mut(key) else {
            return;
        };
        let Ok(runtime) = &instance.runtime else {
            return;
        };
        if instance.frame_pending
            || !instance.visible(self.route())
            || instance.error.is_some()
            || runtime.drawing_failed()
            || !runtime.animating()
        {
            return;
        }
        let cap = runtime.definition.animation.as_ref().and_then(|a| a.fps);
        instance.frame_pending = true;
        let generation = instance.generation;
        let now = Instant::now();
        if instance
            .last_frame
            .is_none_or(|last| now.duration_since(last) > Duration::from_millis(200))
        {
            instance.last_frame = Some(now);
            instance.next_due = cap.map(|fps| now + Duration::from_secs_f64(1.0 / f64::from(fps)));
        }
        let key = key.to_owned();
        drop(apps);
        if let Some(fps) = cap.filter(|fps| *fps <= 30) {
            // Preserve explicitly requested low-rate simulations without waking
            // every display frame. Default/unrestricted motion is vsync-driven.
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_secs_f64(1.0 / f64::from(fps)))
                    .await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.visual_frame(&key, generation, window, cx);
                });
            })
            .detach();
        } else {
            let entity = cx.entity().downgrade();
            window.on_next_frame(move |window, cx| {
                let _ = entity.update(cx, |this, cx| {
                    this.visual_frame(&key, generation, window, cx)
                });
            });
        }
    }

    fn visual_frame(
        &mut self,
        key: &str,
        generation: u64,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let _trace = gpui_android::trace::scope(c"pi.visual.tick");
        let allowed = window.visibility().is_visible()
            && window.is_window_active()
            && !cx.reduce_motion()
            && self.sheet.is_none()
            && self.closing_sheet.is_none();
        let mut apps = self.visual_apps.borrow_mut();
        let Some(instance) = apps.running.get_mut(key) else {
            return;
        };
        if instance.generation != generation {
            return;
        }
        instance.frame_pending = false;
        if !allowed || !instance.visible(self.route()) || instance.error.is_some() {
            instance.last_frame = None;
            return;
        }
        let Ok(runtime) = &mut instance.runtime else {
            return;
        };
        if runtime.drawing_failed() || !runtime.animating() {
            instance.last_frame = None;
            return;
        }
        let now = Instant::now();
        let elapsed = instance
            .last_frame
            .map_or(1.0 / 60.0, |last| now.duration_since(last).as_secs_f64());
        let cap = runtime.definition.animation.as_ref().and_then(|a| a.fps);
        if cap.is_some_and(|fps| fps > 30 && !advance_cap(&mut instance.next_due, fps, now)) {
            drop(apps);
            self.queue_visual_frame(key, window, cx);
            return;
        }
        instance.last_frame = Some(now);
        runtime.enable_profiling(crate::visual_profile::enabled(cx));
        instance.error = runtime
            .tick(elapsed.min(0.1))
            .err()
            .map(|e| short_error(&e));
        crate::visual_profile::runtime(cx, runtime);
        drop(apps);
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    fn visual_input(
        &self,
        key: &str,
        bundle: &Arc<str>,
        control: &str,
        pointer: bool,
        generation: u64,
        id: SessionId,
        turn: usize,
        cx: &Context<Self>,
    ) -> visual_controls::Input {
        let entity = cx.entity().downgrade();
        let key = key.to_owned();
        let bundle = bundle.clone();
        let control = control.to_owned();
        let route = self.route();
        Rc::new(move |position, gesture, _, cx| {
            let _trace = gpui_android::trace::scope(c"pi.visual.input");
            let delivered = entity.update(cx, |this, cx| {
                let gesture = if (this.route() != route
                    || this.sheet.is_some()
                    || this.closing_sheet.is_some())
                    && gesture != Gesture::Cancel
                {
                    Gesture::Cancel
                } else {
                    gesture
                };
                let mut apps = this.visual_apps.borrow_mut();
                if !apps
                    .running
                    .get(&key)
                    .is_some_and(|i| i.generation == generation)
                {
                    let _trace = gpui_android::trace::scope(c"pi.visual.input.stale");
                    return;
                }
                let mut repaint = matches!(gesture, Gesture::End | Gesture::Cancel);
                apps.ensure(&key, &bundle, !this.paused);
                if let Some(instance) = apps.running.get_mut(&key)
                    && let Ok(runtime) = &mut instance.runtime
                {
                    runtime.enable_profiling(crate::visual_profile::enabled(cx));
                    if gesture == Gesture::Start {
                        instance.input_snapshot = runtime.state_json().ok();
                        instance.last_input = None;
                    }
                    if gesture == Gesture::Cancel {
                        let _trace = gpui_android::trace::scope(c"pi.visual.input.cancel");
                        let result = instance
                            .input_snapshot
                            .take()
                            .map_or(Ok(()), |saved| runtime.restore_state(&saved));
                        instance.error = result.err().map(|e| short_error(&e));
                    } else {
                        let normalized = if pointer {
                            Ok(position)
                        } else {
                            runtime
                                .slider_value(&control, position[0])
                                .map(|v| [v, 0.0])
                        };
                        match normalized {
                            Ok(position) => {
                                let input = (control.clone(), position);
                                // End still persists, but a tap/same quantized value
                                // must not invoke the script a second time.
                                if instance.last_input.as_ref() != Some(&input) {
                                    instance.last_input = Some(input);
                                    repaint = true;
                                    let _trace =
                                        gpui_android::trace::scope(c"pi.visual.input.invoke");
                                    let result = if pointer {
                                        runtime.pointer(&control, position[0], position[1])
                                    } else {
                                        runtime.slide(&control, position[0])
                                    };
                                    instance.error = result.err().map(|e| short_error(&e));
                                } else {
                                    let _trace =
                                        gpui_android::trace::scope(c"pi.visual.input.duplicate");
                                    crate::visual_profile::duplicate(cx);
                                }
                            }
                            Err(error) => {
                                let _trace = gpui_android::trace::scope(c"pi.visual.input.reject");
                                instance.error = Some(short_error(&error));
                                repaint = true;
                            }
                        }
                    }
                    crate::visual_profile::runtime(cx, runtime);
                    if matches!(gesture, Gesture::End | Gesture::Cancel) {
                        instance.input_snapshot = None;
                        instance.last_input = None;
                    }
                }
                if gesture == Gesture::End {
                    let saved = apps
                        .running
                        .get(&key)
                        .and_then(|i| i.runtime.as_ref().ok())
                        .and_then(|r| r.state_json().ok());
                    if let Some(saved) = saved {
                        apps.remember_local(&key, saved, !this.paused);
                    }
                }
                drop(apps);
                if matches!(gesture, Gesture::End | Gesture::Cancel) {
                    this.remeasure_thread_turn(id, turn);
                }
                if repaint {
                    let _trace = gpui_android::trace::scope(c"pi.visual.input.notify");
                    cx.notify();
                }
            });
            if delivered.is_err() {
                let _trace = gpui_android::trace::scope(c"pi.visual.input.gone");
            }
        })
    }

    pub(crate) fn visual_has_app(&self, source: &str) -> bool {
        self.visual_apps.borrow_mut().parsed_reply(source).has_app()
    }

    pub(crate) fn visual_enabled(&self, session: &Session, turn: usize) -> bool {
        session
            .turns
            .iter()
            .take(turn + 1)
            .any(|turn| !turn.reported && crate::visualize_prompt::requested(&turn.prompt))
    }

    /// Explicit assistant-only seam: other Markdown remains inert.
    pub(crate) fn visual_reply(
        &self,
        session: &Session,
        turn: usize,
        slot: usize,
        source: &str,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let enabled = self.visual_enabled(session, turn);
        let parsed = self.visual_apps.borrow_mut().parsed_reply(source);
        if !enabled || !parsed.has_app() {
            return div()
                .relative()
                .min_w_0()
                .child(crate::message::render(source, colors))
                .child(self.copyable(source.to_owned(), cx));
        }
        let mut column = div().min_w_0().flex().flex_col().gap(px(12.));
        // A flow's slot is its Flow index; aggregate summaries use slot zero.
        // Normalize to a turn-wide app ordinal, stable when tools are expanded
        // or a streamed reply switches to its aggregate summary.
        let preceding = session.turns.get(turn).map_or(0, |turn| {
            turn.flow
                .iter()
                .take(slot)
                .filter_map(|step| match step {
                    crate::model::Flow::Text(text) => Some(
                        self.visual_apps
                            .borrow_mut()
                            .parsed_reply(text)
                            .parts()
                            .filter(|p| matches!(p, Part::App { .. }))
                            .count(),
                    ),
                    _ => None,
                })
                .sum::<usize>()
        });
        let mut examined = preceding;
        for part in parsed.parts() {
            match part {
                Part::Markdown(text) => column = column.child(crate::message::render(text, colors)),
                Part::App {
                    source,
                    raw,
                    ordinal,
                } => {
                    let card = (examined < 2)
                        .then(|| {
                            self.visual_card(
                                session,
                                turn,
                                0,
                                preceding + ordinal,
                                source,
                                colors,
                                cx,
                            )
                        })
                        .flatten();
                    examined += 1;
                    if let Some(card) = card {
                        column = column.child(card);
                    } else {
                        column = column.child(crate::message::render(raw, colors));
                    }
                }
            }
        }
        column
    }

    #[allow(clippy::too_many_arguments)]
    fn visual_card(
        &self,
        session: &Session,
        turn: usize,
        slot: usize,
        ordinal: usize,
        source: &str,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Option<gpui::Stateful<Div>> {
        let id = session.id;
        let projected = session.turns.get(turn)?;
        if !self.paused
            && (projected.visual_identity.is_none()
                || session
                    .turns
                    .iter()
                    .filter(|t| t.visual_identity == projected.visual_identity)
                    .count()
                    > 1)
        {
            return None;
        }
        let turn_key = session
            .turns
            .get(turn)?
            .visual_identity
            .clone()
            .unwrap_or_else(|| format!("sample-turn-{turn}-{}", session.turns[turn].at));
        let computer = &self.store.as_ref()?.computer.address;
        // Scope comes from the same immutable projected message as the UI,
        // not a newer live watch whose positional SessionId may have changed.
        let session_key = session
            .turns
            .get(turn)?
            .visual_identity
            .clone()
            .unwrap_or_else(|| format!("sample-{}-{}", id.0, session.details.session_file));
        let key = identity(
            computer,
            &session_key,
            &turn_key,
            slot,
            ordinal,
            &session.turns.get(turn)?.prompt,
            source,
        );
        let mut apps = self.visual_apps.borrow_mut();
        apps.ensure(&key, source, !self.paused);
        let instance = apps.running.get_mut(&key)?;
        if instance.route != Some(self.route()) {
            instance.cancel_manipulation();
        }
        instance.route = Some(self.route());
        let runtime = instance.runtime.as_ref().ok()?;
        let bundle: Arc<str> = Arc::from(source);
        let node = self.visual_node(
            &runtime.definition.view,
            instance,
            &key,
            &bundle,
            "root",
            id,
            turn,
            colors,
            cx,
        );
        let source_copy = source.to_owned();
        let source_show = source.to_owned();
        let reset_key = key.clone();
        let reset_source = bundle.clone();
        Some(
            div()
                .id(SharedString::from(format!("visual-{key}")))
                .debug_selector(|| "visual-app".into())
                .min_w_0()
                .w_full()
                .rounded(px(12.))
                .bg(colors.panel)
                .border_1()
                .border_color(colors.line)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .min_h(px(48.))
                        .pl(px(14.))
                        .gap(px(8.))
                        .child(ui::icon("spark", 16., colors.accent))
                        .child(
                            ui::label(runtime.definition.title.clone(), colors)
                                .flex_1()
                                .min_w_0()
                                .truncate(),
                        )
                        .child(
                            ui::tap(("visual-copy", ordinal), "copy", colors)
                                .aria_label("Copy visual app source")
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        source_copy.clone(),
                                    ))
                                }),
                        ),
                )
                .child(div().p(px(12.)).child(node))
                .children((runtime.animating() && cx.reduce_motion()).then(|| {
                    ui::hint("Animation paused by system reduced motion", colors).px(px(12.))
                }))
                .children(instance.error.as_ref().map(|error| {
                    ui::hint(error.clone(), colors)
                        .px(px(12.))
                        .text_color(colors.coral)
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .border_t_1()
                        .border_color(colors.line)
                        .child(
                            ui::hint("Native · local state", colors)
                                .pl(px(12.))
                                .flex_1(),
                        )
                        .child(
                            ui::button(
                                ("visual-source", ordinal),
                                ui::Button::Quiet,
                                Some("code"),
                                "Source",
                                false,
                                colors,
                            )
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.show_selected_text(source_show.clone(), window, cx);
                                },
                            )),
                        )
                        .child(
                            ui::tap(("visual-reset", ordinal), "refresh", colors)
                                .aria_label("Reset visual app state")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let mut apps = this.visual_apps.borrow_mut();
                                    apps.ensure(&reset_key, &reset_source, !this.paused);
                                    apps.generation = apps.generation.wrapping_add(1);
                                    let generation = apps.generation;
                                    if let Some(instance) = apps.running.get_mut(&reset_key) {
                                        instance.barrier.invalidate();
                                        instance.barrier =
                                            visual_controls::ProjectionBarrier::default();
                                        instance.generation = generation;
                                        instance.frame_pending = false;
                                        instance.last_frame = None;
                                        instance.next_due = None;
                                        instance.input_snapshot = None;
                                        instance.last_input = None;
                                        instance.painted.borrow_mut().clear();
                                        instance.runtime = Runtime::from_bundle(&reset_source)
                                            .map_err(|error| short_error(&error));
                                        instance.error = None;
                                    }
                                    let saved = apps
                                        .running
                                        .get(&reset_key)
                                        .and_then(|i| i.runtime.as_ref().ok())
                                        .and_then(|r| r.state_json().ok());
                                    if let Some(saved) = saved {
                                        apps.remember_local(&reset_key, saved, !this.paused);
                                    }
                                    drop(apps);
                                    this.remeasure_thread_turn(id, turn);
                                    cx.notify();
                                })),
                        ),
                ),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn visual_node(
        &self,
        node: &Node,
        instance: &Instance,
        key: &str,
        bundle: &Arc<str>,
        path: &str,
        id: SessionId,
        turn: usize,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> AnyElement {
        let runtime = instance.runtime.as_ref().unwrap();
        match node {
            Node::Column {
                gap,
                width,
                children,
            } => {
                let mut view = div().flex().flex_col().flex_1().min_w_0().gap(px(*gap));
                if let Some(width) = width {
                    view = view.w(px(*width)).max_w_full();
                }
                view.children(children.iter().enumerate().map(|(n, child)| {
                    self.visual_node(
                        child,
                        instance,
                        key,
                        bundle,
                        &format!("{path}-{n}"),
                        id,
                        turn,
                        colors,
                        cx,
                    )
                }))
                .into_any_element()
            }
            Node::Row { gap, children } => div()
                .flex()
                .items_start()
                .w_full()
                .min_w_0()
                .gap(px(*gap))
                .children(children.iter().enumerate().map(|(n, child)| {
                    self.visual_node(
                        child,
                        instance,
                        key,
                        bundle,
                        &format!("{path}-{n}"),
                        id,
                        turn,
                        colors,
                        cx,
                    )
                }))
                .into_any_element(),
            Node::Text { text, bind, style } => {
                let content = bind
                    .as_ref()
                    .map(|name| {
                        let _trace = gpui_android::trace::scope(c"pi.visual.text_lookup");
                        runtime.text(name)
                    })
                    .unwrap_or_else(|| text.clone().unwrap_or_default());
                let mut view = div().min_w_0().w_full().child(content);
                view = match style.as_str() {
                    "display" => view
                        .font_family(MONO)
                        .text_size(px(36.))
                        .line_height(px(52.))
                        .text_right(),
                    "expression" => view
                        .font_family(MONO)
                        .text_size(px(12.))
                        .line_height(px(20.))
                        .text_right()
                        .text_color(colors.muted),
                    "heading" => view.text_size(px(13.)).font_weight(FontWeight::SEMIBOLD),
                    "history" => view
                        .font_family(MONO)
                        .text_size(px(12.))
                        .line_height(px(20.))
                        .text_color(colors.secondary),
                    _ => view
                        .text_size(px(12.))
                        .line_height(px(20.))
                        .text_color(colors.muted),
                };
                view.into_any_element()
            }
            Node::Slider {
                label,
                bind,
                min,
                max,
                step,
                ..
            } => {
                let value = runtime.number(bind).unwrap_or(*min).clamp(*min, *max);
                let (min, max, step) = (*min, *max, *step);
                let colors = *colors;
                Surface {
                    id: SharedString::from(format!(
                        "visual-slider-{key}-{bind}-{}",
                        instance.generation
                    )),
                    label: label.clone(),
                    probe: format!("visual-slider-{bind}"),
                    height: 48.0,
                    slider: Some((value, min, max, step)),
                    barrier: instance.barrier.clone(),
                    coordinates: Rc::new(move |position, bounds| {
                        visual_controls::slider_at(position, bounds, min, max)
                    }),
                    input: Some(self.visual_input(
                        key,
                        bundle,
                        bind,
                        false,
                        instance.generation,
                        id,
                        turn,
                        cx,
                    )),
                    painter: Rc::new(move |bounds, _, window, _| {
                        let left = bounds.left() + px(12.);
                        let width = (bounds.size.width - px(24.)).max(px(1.));
                        let y = bounds.center().y;
                        let fraction = ((value - min) / (max - min)) as f32;
                        let track = gpui::Bounds::new(point(left, y - px(2.)), size(width, px(4.)));
                        window.paint_quad(fill(track, colors.line_strong).corner_radii(px(2.)));
                        window.paint_quad(
                            fill(
                                gpui::Bounds::new(track.origin, size(width * fraction, px(4.))),
                                colors.accent,
                            )
                            .corner_radii(px(2.)),
                        );
                        window.paint_quad(
                            fill(
                                gpui::Bounds::new(
                                    point(left + width * fraction - px(10.), y - px(10.)),
                                    size(px(20.), px(20.)),
                                ),
                                colors.accent,
                            )
                            .corner_radii(px(10.)),
                        );
                    }),
                }
                .into_any_element()
            }
            Node::Canvas {
                label,
                height,
                view_box,
                draw,
                pointer_action,
            } => {
                runtime.enable_profiling(crate::visual_profile::enabled(cx));
                let scene_result = {
                    let _trace = gpui_android::trace::scope(c"pi.visual.scene_lookup");
                    runtime.scene(draw)
                };
                let (scene, error) = match scene_result {
                    Ok(scene) => (scene, None),
                    Err(error) => {
                        let last = runtime.last_scene(draw);
                        let message = if last.is_some() {
                            format!("{} · last valid frame shown", short_error(&error))
                        } else {
                            short_error(&error)
                        };
                        (last.unwrap_or_else(|| Arc::new(Vec::new())), Some(message))
                    }
                };
                crate::visual_profile::runtime(cx, runtime);
                let animate = error.is_none();
                let view_box = *view_box;
                let entity = cx.entity().downgrade();
                let frame_key = key.to_owned();
                let colors = *colors;
                let visibility_path = path.to_owned();
                let visibility_generation = instance.generation;
                let surface = Surface {
                    id: SharedString::from(format!(
                        "visual-canvas-{key}-{path}-{}",
                        instance.generation
                    )),
                    label: label.clone(),
                    probe: format!("visual-canvas-{path}"),
                    height: *height,
                    slider: None,
                    barrier: instance.barrier.clone(),
                    coordinates: Rc::new(move |position, bounds| {
                        visual_controls::pointer_at(position, bounds, view_box)
                    }),
                    input: pointer_action.as_ref().map(|action| {
                        self.visual_input(
                            key,
                            bundle,
                            action,
                            true,
                            instance.generation,
                            id,
                            turn,
                            cx,
                        )
                    }),
                    painter: Rc::new(move |bounds, cache, window, cx| {
                        window.paint_quad(fill(bounds, colors.canvas).corner_radii(px(10.)));
                        let start = crate::visual_profile::enabled(cx).then(Instant::now);
                        let hits = cache.hits;
                        let builds = cache.builds;
                        visual_controls::paint_scene(&scene, view_box, bounds, cache, window);
                        crate::visual_profile::paths(
                            cx,
                            start,
                            cache.hits - hits,
                            cache.builds - builds,
                        );
                        let _ = entity.update(cx, |this, cx| {
                            {
                                let apps = this.visual_apps.borrow();
                                if let Some(instance) = apps.running.get(&frame_key)
                                    && instance.generation == visibility_generation
                                {
                                    instance.painted.borrow_mut().insert(
                                        visibility_path.clone(),
                                        Rc::downgrade(&cache.painted),
                                    );
                                }
                            }
                            if animate && cache.painted.get() {
                                this.queue_visual_frame(&frame_key, window, cx);
                            }
                        });
                    }),
                };
                // Keep the gesture surface mounted even if drawing fails, so
                // release/cancel can finish the transaction and restore state.
                div()
                    .relative()
                    .w_full()
                    .overflow_hidden()
                    .child(surface)
                    .children(error.map(|error| {
                        ui::hint(error, &colors)
                            .text_color(colors.coral)
                            .absolute()
                            .top_0()
                            .left_0()
                            .px(px(8.))
                    }))
                    .into_any_element()
            }
            Node::Button {
                label,
                action,
                tone,
            } => {
                let (bg, fg) = match tone.as_str() {
                    "accent" => (colors.accent, colors.on_accent),
                    "operator" => (colors.tint(colors.accent), colors.accent),
                    "utility" => (colors.selected, colors.secondary),
                    _ => (colors.canvas, colors.text),
                };
                let key = key.to_owned();
                let bundle = bundle.clone();
                let action = action.clone();
                div()
                    .id(SharedString::from(format!("{key}-{path}")))
                    .relative()
                    .child(crate::testing::probe(format!("visual-action-{action}")))
                    .role(Role::Button)
                    .aria_label(label.clone())
                    .flex_1()
                    .min_w_0()
                    .min_h(px(48.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .bg(bg)
                    .text_color(fg)
                    .text_size(px(18.))
                    .active(|s| s.bg(colors.selected))
                    .child(label.clone())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let mut apps = this.visual_apps.borrow_mut();
                        apps.ensure(&key, &bundle, !this.paused);
                        if let Some(instance) = apps.running.get_mut(&key)
                            && let Ok(runtime) = &mut instance.runtime
                        {
                            runtime.enable_profiling(crate::visual_profile::enabled(cx));
                            instance.error =
                                runtime.dispatch(&action).err().map(|e| short_error(&e));
                            crate::visual_profile::runtime(cx, runtime);
                        }
                        let saved = apps
                            .running
                            .get(&key)
                            .and_then(|i| i.runtime.as_ref().ok())
                            .and_then(|r| r.state_json().ok());
                        if let Some(saved) = saved {
                            apps.remember_local(&key, saved, !this.paused);
                        }
                        drop(apps);
                        window.dismiss_virtual_keyboard();
                        window.focus(&this.focus, cx);
                        this.remeasure_thread_turn(id, turn);
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .into_any_element()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_route_or_sheet_rolls_back_before_unblocking_projection() {
        let route = crate::app::Route::Thread(SessionId(1));
        for (next_route, available) in [(crate::app::Route::Sessions, true), (route, false)] {
            let mut apps = VisualApps::default();
            let barrier = apps.test_projection_barrier(route);
            let lease = barrier.acquire();
            let instance = apps.running.get_mut("projection-fixture").unwrap();
            let runtime = instance.runtime.as_mut().unwrap();
            instance.input_snapshot = Some(runtime.state_json().unwrap());
            runtime.slide("x", 10.0).unwrap();
            assert!(apps.projection_deferred(route, true));
            assert!(!apps.projection_deferred(next_route, available));
            assert_eq!(
                apps.running["projection-fixture"]
                    .runtime
                    .as_ref()
                    .unwrap()
                    .number("x"),
                Some(0.0)
            );
            assert!(apps.running["projection-fixture"].input_snapshot.is_none());
            let newer = barrier.acquire();
            drop(lease);
            assert!(barrier.active());
            drop(newer);
            assert!(!barrier.active());
        }
    }

    #[gpui::test]
    fn blur_and_brief_hidden_transition_cancel_without_waiting_for_projection_poll(
        cx: &mut gpui::TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        for hidden in [false, true] {
            cx.update(|window, _| window.activate_window());
            cx.run_until_parked();
            let barrier = app.update(cx, |app, _| {
                app.visual_apps
                    .borrow_mut()
                    .test_projection_barrier(app.route())
            });
            let lease = barrier.acquire();
            app.update(cx, |app, _| {
                let mut apps = app.visual_apps.borrow_mut();
                let instance = apps.running.get_mut("projection-fixture").unwrap();
                let runtime = instance.runtime.as_mut().unwrap();
                instance.input_snapshot = Some(runtime.state_json().unwrap());
                runtime.slide("x", 20.0).unwrap();
            });
            if hidden {
                cx.simulate_visibility_change(gpui::WindowVisibility::Hidden);
                cx.simulate_visibility_change(gpui::WindowVisibility::Visible);
            } else {
                cx.deactivate_window(); // Still visible; no synthetic native Cancel.
            }
            assert!(!barrier.active());
            app.read_with(cx, |app, _| {
                assert_eq!(
                    app.visual_apps.borrow().running["projection-fixture"]
                        .runtime
                        .as_ref()
                        .unwrap()
                        .number("x"),
                    Some(0.0)
                );
            });
            let newer = barrier.acquire();
            drop(lease);
            assert!(barrier.active());
            drop(newer);
        }
    }

    #[test]
    fn native_visibility_and_fractional_caps_do_not_assume_120_hz() {
        let start = Instant::now();
        let mut due = Some(start);
        let mut ticks = 0;
        // 90 fps cap on a 120-Hz source should yield ~90, not rounded-down 60.
        for frame in 0..120 {
            if advance_cap(
                &mut due,
                90,
                start + Duration::from_secs_f64(frame as f64 / 120.0),
            ) {
                ticks += 1;
            }
        }
        assert!((89..=91).contains(&ticks), "{ticks}");
        let mut apps = VisualApps::default();
        let source = include_str!("../assets/samples/spirograph-trace.pi-visual.js");
        apps.ensure("visible", source, false);
        let instance = apps.running.get_mut("visible").unwrap();
        let route = crate::app::Route::Thread(SessionId(1));
        instance.route = Some(route);
        let painted = Rc::new(Cell::new(true));
        instance
            .painted
            .borrow_mut()
            .insert("canvas".into(), Rc::downgrade(&painted));
        assert!(instance.visible(route));
        assert!(!instance.visible(crate::app::Route::Sessions));
        painted.set(false);
        assert!(!instance.visible(route));
        drop(painted);
        assert!(!instance.visible(route));
    }

    #[gpui::test]
    fn direct_manipulation_deduplicates_taps_and_restores_cancelled_state(
        cx: &mut gpui::TestAppContext,
    ) {
        let source: Arc<str> = r#"app({title:'Input test',state:{x:0,count:0},view:ui.column([
            ui.canvas({id:'scene',pointerId:'tap',label:'Tap canvas',height:200,viewBox:[0,0,100,100],
                draw(g,s){if(s.x>50)throw Error('Drawing error');},onPointer(s,x,y){s.count++;s.x=x;}}),
            ui.slider('Position','x',{min:0,max:100,step:1,onChange(s){s.count++;}})
        ])});"#.into();
        let key = "input-test";
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        let input = app.update(cx, |app, cx| {
            app.visual_apps.borrow_mut().ensure(key, &source, false);
            app.visual_input(
                key,
                &source,
                "tap",
                true,
                app.visual_apps.borrow().running[key].generation,
                SessionId(1),
                0,
                cx,
            )
        });
        let send = |gesture, position, cx: &mut gpui::VisualTestContext| {
            cx.update(|window, cx| input(position, gesture, window, cx));
        };
        send(Gesture::Start, [10.0, 10.0], cx);
        send(Gesture::End, [10.0, 10.0], cx);
        app.read_with(cx, |app, _| {
            let apps = app.visual_apps.borrow();
            let runtime = apps.running[key].runtime.as_ref().unwrap();
            assert_eq!(runtime.number("count"), Some(1.0));
        });
        send(Gesture::Start, [60.0, 10.0], cx);
        app.read_with(cx, |app, _| {
            assert!(
                app.visual_apps.borrow().running[key]
                    .runtime
                    .as_ref()
                    .unwrap()
                    .scene("scene")
                    .is_err()
            );
        });
        send(Gesture::Cancel, [60.0, 10.0], cx);
        app.read_with(cx, |app, _| {
            let apps = app.visual_apps.borrow();
            let runtime = apps.running[key].runtime.as_ref().unwrap();
            assert_eq!(runtime.number("x"), Some(10.0));
            assert_eq!(runtime.number("count"), Some(1.0));
            assert!(runtime.scene("scene").is_ok());
        });
        let slider = app.update(cx, |app, cx| {
            app.visual_input(
                key,
                &source,
                "x",
                false,
                app.visual_apps.borrow().running[key].generation,
                SessionId(1),
                0,
                cx,
            )
        });
        cx.update(|window, cx| slider([20.1, 0.0], Gesture::Start, window, cx));
        cx.update(|window, cx| slider([20.4, 0.0], Gesture::End, window, cx));
        app.read_with(cx, |app, _| {
            let apps = app.visual_apps.borrow();
            let runtime = apps.running[key].runtime.as_ref().unwrap();
            assert_eq!(runtime.number("x"), Some(20.0));
            assert_eq!(runtime.number("count"), Some(2.0));
        });
    }

    #[gpui::test]
    fn visual_runtime_survives_flow_summary_and_prose_changes(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.preview("visual-calculator", window, cx))
        });
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            let id = SessionId(1);
            let source = app.session(id).unwrap().turns[0]
                .summary
                .as_ref()
                .unwrap()
                .text();
            let colors = crate::theme::theme(cx);
            let _ = app.visual_reply(
                &app.session(id).unwrap().clone(),
                0,
                0,
                &source,
                &colors,
                cx,
            );
            {
                let mut apps = app.visual_apps.borrow_mut();
                assert_eq!(apps.running.len(), 1);
                apps.running
                    .values_mut()
                    .next()
                    .unwrap()
                    .runtime
                    .as_mut()
                    .unwrap()
                    .dispatch("7")
                    .unwrap();
            }
            let session = app
                .store
                .as_mut()
                .unwrap()
                .sessions
                .iter_mut()
                .find(|s| s.id == id)
                .unwrap();
            session.turns[0].flow = vec![
                crate::model::Flow::Text("Earlier prose".into()),
                crate::model::Flow::Text(source.clone()),
            ];
            let _ = app.visual_reply(
                &app.session(id).unwrap().clone(),
                0,
                1,
                &format!("{source}\nMore prose"),
                &colors,
                cx,
            );
            let apps = app.visual_apps.borrow();
            assert_eq!(apps.running.len(), 1);
            assert_eq!(
                apps.running
                    .values()
                    .next()
                    .unwrap()
                    .runtime
                    .as_ref()
                    .unwrap()
                    .text("display"),
                "7"
            );
        });
    }

    #[test]
    fn local_state_is_bounded_and_restored_only_for_the_same_bundle() {
        let directory =
            std::env::temp_dir().join(format!("pi-visual-state-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&directory).unwrap();
        let prefs = directory.join("settings.json");
        let source = include_str!("../assets/samples/calculator.pi-visual.js");
        let key = identity("host", "session", 0, 0, 0, "prompt", source);
        let mut first = VisualApps::load(Some(&prefs));
        first.ensure(&key, source, true);
        let runtime = first
            .running
            .get_mut(&key)
            .unwrap()
            .runtime
            .as_mut()
            .unwrap();
        runtime.dispatch("7").unwrap();
        let state = runtime.state_json().unwrap();
        first.remember(&key, state);
        let mut reopened = VisualApps::load(Some(&prefs));
        reopened.ensure(&key, source, true);
        assert_eq!(
            reopened.running[&key]
                .runtime
                .as_ref()
                .unwrap()
                .text("display"),
            "7"
        );
        let different = identity("host", "other", 0, 0, 0, "prompt", source);
        reopened.ensure(&different, source, true);
        assert_eq!(
            reopened.running[&different]
                .runtime
                .as_ref()
                .unwrap()
                .text("display"),
            "0"
        );
        for n in 0..40 {
            reopened.remember(&format!("{n:064x}"), serde_json::json!({"display":"1"}));
        }
        assert_eq!(reopened.saved.len(), MAX_SAVED);
        assert!(
            std::fs::metadata(directory.join("visual-app-state.json"))
                .unwrap()
                .len()
                <= MAX_SAVED_BYTES as u64
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn evicted_instances_rehydrate_before_an_action() {
        let source = include_str!("../assets/samples/calculator.pi-visual.js");
        let mut apps = VisualApps::default();
        apps.ensure("first", source, false);
        let runtime = apps
            .running
            .get_mut("first")
            .unwrap()
            .runtime
            .as_mut()
            .unwrap();
        runtime.dispatch("7").unwrap();
        let state = runtime.state_json().unwrap();
        apps.remember_local("first", state, false);
        for n in 0..12 {
            apps.ensure(&n.to_string(), source, false);
        }
        assert!(!apps.running.contains_key("first"));
        // Action and Reset callbacks execute this same re-ensure step.
        apps.ensure("first", source, false);
        let runtime = apps
            .running
            .get_mut("first")
            .unwrap()
            .runtime
            .as_mut()
            .unwrap();
        assert_eq!(runtime.text("display"), "7");
        runtime.dispatch("8").unwrap();
        assert_eq!(runtime.text("display"), "78");
    }

    #[test]
    fn cache_is_bounded_and_state_identity_ignores_prose() {
        let source = include_str!("../assets/samples/calculator.pi-visual.js");
        let mut apps = VisualApps::default();
        for n in 0..12 {
            apps.ensure(&n.to_string(), source, false);
        }
        assert_eq!(apps.running.len(), MAX_RUNNING);
        assert_ne!(
            identity("a", "s", 0, 0, 0, "p", source),
            identity("b", "s", 0, 0, 0, "p", source)
        );
        assert_ne!(
            identity("a", "s", 0, 0, 0, "p", source),
            identity("a", "s", 0, 1, 0, "p", source)
        );
        apps.ensure("11", source, false);
        let runtime = apps
            .running
            .get_mut("11")
            .unwrap()
            .runtime
            .as_mut()
            .unwrap();
        runtime.dispatch("7").unwrap();
        apps.ensure("11", source, false);
        assert_eq!(
            apps.running["11"].runtime.as_ref().unwrap().text("display"),
            "7"
        );
    }
}
