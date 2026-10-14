//! Native, explicitly requested mini-apps. Scripts have no host/agent APIs.
use crate::visual_controls::{self, Gesture, Surface};
use crate::{
    theme::{MONO, Theme},
    ui,
};
use gpui::{
    AnyElement, ClipboardItem, Context, Div, FontWeight, Role, SharedString, Window, div, fill,
    point, prelude::*, px, size,
};
use pi_visual::{Node, Runtime};
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

pub struct Instance<R: Copy + Eq> {
    pub runtime: Result<Runtime, String>,
    pub source_open: bool,
    pub error: Option<String>,
    pub generation: u64,
    pub frame_pending: bool,
    pub last_frame: Option<Instant>,
    pub input_snapshot: Option<serde_json::Value>,
    pub barrier: visual_controls::ProjectionBarrier,
    pub last_input: Option<(String, [f64; 2])>,
    pub route: Option<R>,
    pub painted: RefCell<HashMap<String, Weak<Cell<bool>>>>,
    pub next_due: Option<Instant>,
    /// Shown full screen by a host that offers it; see `Host::visual_expandable`.
    pub expanded: bool,
    /// The bundle it runs, so a host can draw it away from its message.
    pub source: Arc<str>,
    /// Canvas height multiplier while expanded; drawings scale to fit.
    pub zoom: f32,
}
impl<R: Copy + Eq> Instance<R> {
    pub fn cancel_manipulation(&mut self) {
        if let Ok(runtime) = &mut self.runtime {
            runtime.cancel_input();
        }
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

    fn visible(&self, route: R) -> bool {
        self.route == Some(route)
            && self
                .painted
                .borrow()
                .values()
                .any(|p| p.upgrade().is_some_and(|p| p.get()))
    }
}
#[derive(Serialize, Deserialize)]
pub struct Saved {
    key: String,
    state: serde_json::Value,
}

pub struct VisualApps<R: Copy + Eq> {
    pub active_route: Option<R>,
    pub running: HashMap<String, Instance<R>>,
    pub order: VecDeque<String>,
    pub saved: Vec<Saved>,
    pub transient: HashMap<String, serde_json::Value>,
    pub path: Option<PathBuf>,
    pub generation: u64,
    pub replies: VecDeque<Arc<pi_visual::reply::Parsed>>,
}

impl<R: Copy + Eq> VisualApps<R> {
    pub fn load(prefs: Option<&Path>) -> Self {
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

    pub fn cancel_route(&mut self, route: R) {
        for instance in self.running.values_mut().filter(|i| i.route == Some(route)) {
            instance.cancel_manipulation();
            instance.last_frame = None;
        }
        if self.active_route == Some(route) {
            self.active_route = None;
        }
    }
    pub fn parsed_reply(&mut self, source: &str) -> Arc<pi_visual::reply::Parsed> {
        let _trace = crate::platform::trace::scope(c"pi.visual.reply.lookup");
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
            let _trace = crate::platform::trace::scope(c"pi.visual.reply.parse");
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
    pub fn projection_deferred(&mut self, route: R, available: bool) -> bool {
        self.active_route = available.then_some(route);
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_projection_barrier(&mut self, route: R) -> visual_controls::ProjectionBarrier {
        self.ensure(
            "projection-fixture",
            "app({title:'Gate',state:{x:0},view:ui.slider('X','x',{min:0,max:100,step:1})});",
            false,
        );
        let instance = self.running.get_mut("projection-fixture").unwrap();
        instance.route = Some(route);
        instance.barrier.clone()
    }

    pub fn clear_running(&mut self) {
        self.running.clear();
        self.order.clear();
        self.transient.clear();
        self.replies.clear();
    }

    pub fn ensure(&mut self, key: &str, source: &str, restore: bool) {
        let _trace = crate::platform::trace::scope(c"pi.visual.runtime.lookup");
        self.order.retain(|old| old != key);
        self.order.push_back(key.to_owned());
        if self.running.contains_key(key) {
            return;
        }
        let _miss_trace = crate::platform::trace::scope(c"pi.visual.runtime.create");
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
                source_open: false,
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
                expanded: false,
                zoom: 1.0,
                source: Arc::from(source),
            },
        );
    }

    pub fn remember_local(&mut self, key: &str, state: serde_json::Value, persist: bool) {
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

    pub fn remember(&mut self, key: &str, state: serde_json::Value) {
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
    pub fn telemetry(&self) -> serde_json::Value {
        serde_json::Value::Array(self.running.values().filter_map(|instance| instance.runtime.as_ref().ok().map(|runtime| {
            serde_json::json!({"title":runtime.definition.title,"state":runtime.state_json().ok(),"error":instance.error,"drawing_failed":runtime.drawing_failed()})
        })).collect())
    }
}

// Keep fractional explicit caps in phase with vsync rather than rounding down
// to half-refresh rates. A delayed frame skips old deadlines (no catch-up burst).
/// The card's own padding and border around a canvas.
const CARD_INSET: f32 = 26.;
fn expanded_zoom(height: f32, view_box: [f32; 4], space: gpui::Size<gpui::Pixels>) -> f32 {
    let width = (f32::from(space.width) - CARD_INSET).max(1.);
    let fill_width = width * view_box[3] / view_box[2];
    let wanted = fill_width.min(f32::from(space.height) * 0.8);
    (wanted / height).clamp(1., 12.)
}

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
pub fn identity(
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

impl<R: Copy + Eq> Default for VisualApps<R> {
    fn default() -> Self {
        Self {
            active_route: None,
            running: HashMap::new(),
            order: VecDeque::new(),
            saved: Vec::new(),
            transient: HashMap::new(),
            path: None,
            generation: 0,
            replies: VecDeque::new(),
        }
    }
}

/// Platform adapters own navigation, persistence policy and transcript measurement.
/// All runtime, control, rendering, frame scheduling and gesture logic is shared.
pub trait Host: Sized + 'static {
    type Route: Copy + Eq + 'static;
    type Target: Copy + 'static;
    fn visual_apps(&self) -> &RefCell<VisualApps<Self::Route>>;
    fn visual_route(&self) -> Self::Route;
    fn visual_available(&self) -> bool;
    fn visual_persist(&self) -> bool;
    fn visual_changed(&mut self, target: Self::Target, cx: &mut Context<Self>);
    fn visual_refocus(&self, window: &mut Window, cx: &mut Context<Self>);
    /// Whether cards offer a full-screen button. The host then draws the
    /// expanded app itself, and sets its `zoom`.
    fn visual_expandable(&self) -> bool {
        false
    }
    fn visual_toggle_expanded(
        &mut self,
        key: &str,
        target: Self::Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut apps = self.visual_apps().borrow_mut();
        let route = self.visual_route();
        let expand = !apps.running.get(key).is_some_and(|i| i.expanded);
        // One app full screen at a time.
        for (other, instance) in apps.running.iter_mut() {
            if instance.route == Some(route) {
                instance.expanded = expand && other == key;
                if !instance.expanded {
                    instance.zoom = 1.0;
                }
            }
        }
        drop(apps);
        self.visual_expanded_changed(expand.then_some(target));
        self.visual_changed(target, cx);
        self.visual_refocus(window, cx);
        cx.notify();
    }
    /// Where the full-screen app came from, or `None` once it is closed; the
    /// host passes it back to `render_expanded_visual`.
    fn visual_expanded_changed(&mut self, _target: Option<Self::Target>) {}
    fn visual_show_source(
        &mut self,
        key: &str,
        source: &str,
        target: Self::Target,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(instance) = self.visual_apps().borrow_mut().running.get_mut(key) {
            instance.source_open = !instance.source_open;
        }
        let _ = source;
        self.visual_changed(target, cx);
        cx.notify();
    }
}
pub trait HostExt: Host {
    fn visual_event_allowed(
        &self,
        key: &str,
        generation: u64,
        route: Self::Route,
        window: &Window,
    ) -> bool {
        self.visual_available()
            && self.visual_route() == route
            && window.is_window_active()
            && window.visibility().is_visible()
            && !window.has_active_prompt()
            && self
                .visual_apps()
                .borrow()
                .running
                .get(key)
                .is_some_and(|i| i.generation == generation && i.visible(route))
    }
    /// One bounded timer per visible app. Never persist or remeasure on frames.
    fn queue_visual_frame(&mut self, key: &str, window: &Window, cx: &mut Context<Self>) {
        if !window.visibility().is_visible()
            || !window.is_window_active()
            || window.has_active_prompt()
            || cx.reduce_motion()
            || !self.visual_available()
        {
            return;
        }
        let mut apps = self.visual_apps().borrow_mut();
        if apps
            .running
            .values()
            .filter(|i| i.frame_pending && i.visible(self.visual_route()))
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
            || !instance.visible(self.visual_route())
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
        let _trace = crate::platform::trace::scope(c"pi.visual.tick");
        let allowed = window.visibility().is_visible()
            && window.is_window_active()
            && !window.has_active_prompt()
            && !cx.reduce_motion()
            && self.visual_available();
        let mut apps = self.visual_apps().borrow_mut();
        let Some(instance) = apps.running.get_mut(key) else {
            return;
        };
        if instance.generation != generation {
            return;
        }
        instance.frame_pending = false;
        if !allowed || !instance.visible(self.visual_route()) || instance.error.is_some() {
            instance.cancel_manipulation();
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

    fn visual_scene_input(
        &self,
        key: &str,
        action: &str,
        generation: u64,
        target: Self::Target,
        cx: &Context<Self>,
    ) -> crate::visual_input::SceneInput {
        let entity = cx.entity().downgrade();
        let key = key.to_owned();
        let action = action.to_owned();
        let route = self.visual_route();
        Rc::new(move |event, window, cx| {
            let _ = entity.update(cx, |this, cx| {
                let available = this.visual_event_allowed(&key, generation, route, window);
                let event = if available {
                    event
                } else {
                    pi_visual::InputEvent::Cancel
                };
                let terminal = matches!(
                    event,
                    pi_visual::InputEvent::Cancel
                        | pi_visual::InputEvent::Blur
                        | pi_visual::InputEvent::Pointer {
                            phase: pi_visual::input::Phase::Up | pi_visual::input::Phase::Cancel,
                            ..
                        }
                        | pi_visual::InputEvent::Key { down: false, .. }
                );
                let mut apps = this.visual_apps().borrow_mut();
                let Some(instance) = apps
                    .running
                    .get_mut(&key)
                    .filter(|i| i.generation == generation && i.route == Some(route))
                else {
                    return;
                };
                if let Ok(runtime) = &mut instance.runtime {
                    runtime.enable_profiling(crate::visual_profile::enabled(cx));
                    instance.error = runtime.input(&action, event).err().map(|e| short_error(&e));
                    crate::visual_profile::runtime(cx, runtime);
                }
                if terminal {
                    let saved = apps
                        .running
                        .get(&key)
                        .and_then(|i| i.runtime.as_ref().ok())
                        .and_then(|r| r.state_json().ok());
                    if let Some(saved) = saved {
                        apps.remember_local(&key, saved, this.visual_persist());
                    }
                }
                drop(apps);
                if terminal {
                    this.visual_changed(target, cx);
                }
                cx.notify();
            });
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn visual_input(
        &self,
        key: &str,
        bundle: &Arc<str>,
        control: &str,
        pointer: bool,
        generation: u64,
        target: Self::Target,
        cx: &Context<Self>,
    ) -> visual_controls::Input {
        let entity = cx.entity().downgrade();
        let key = key.to_owned();
        let bundle = bundle.clone();
        let control = control.to_owned();
        let route = self.visual_route();
        Rc::new(move |position, gesture, window, cx| {
            let _trace = crate::platform::trace::scope(c"pi.visual.input");
            let delivered = entity.update(cx, |this, cx| {
                let gesture = if !this.visual_event_allowed(&key, generation, route, window)
                    && gesture != Gesture::Cancel
                {
                    Gesture::Cancel
                } else {
                    gesture
                };
                let mut apps = this.visual_apps().borrow_mut();
                if !apps
                    .running
                    .get(&key)
                    .is_some_and(|i| i.generation == generation && i.route == Some(route))
                {
                    let _trace = crate::platform::trace::scope(c"pi.visual.input.stale");
                    return;
                }
                let mut repaint = matches!(gesture, Gesture::End | Gesture::Cancel);
                apps.ensure(&key, &bundle, this.visual_persist());
                if let Some(instance) = apps.running.get_mut(&key)
                    && let Ok(runtime) = &mut instance.runtime
                {
                    runtime.enable_profiling(crate::visual_profile::enabled(cx));
                    if gesture == Gesture::Start {
                        instance.input_snapshot = runtime.state_json().ok();
                        instance.last_input = None;
                    }
                    if gesture == Gesture::Cancel {
                        let _trace = crate::platform::trace::scope(c"pi.visual.input.cancel");
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
                                        crate::platform::trace::scope(c"pi.visual.input.invoke");
                                    let result = if pointer {
                                        runtime.pointer(&control, position[0], position[1])
                                    } else {
                                        runtime.slide(&control, position[0])
                                    };
                                    instance.error = result.err().map(|e| short_error(&e));
                                } else {
                                    let _trace =
                                        crate::platform::trace::scope(c"pi.visual.input.duplicate");
                                    crate::visual_profile::duplicate(cx);
                                }
                            }
                            Err(error) => {
                                let _trace =
                                    crate::platform::trace::scope(c"pi.visual.input.reject");
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
                        apps.remember_local(&key, saved, this.visual_persist());
                    }
                }
                drop(apps);
                if matches!(gesture, Gesture::End | Gesture::Cancel) {
                    this.visual_changed(target, cx);
                }
                if repaint {
                    let _trace = crate::platform::trace::scope(c"pi.visual.input.notify");
                    cx.notify();
                }
            });
            if delivered.is_err() {
                let _trace = crate::platform::trace::scope(c"pi.visual.input.gone");
            }
        })
    }

    /// The app this route shows full screen in `space`: its tallest drawing
    /// as wide as the card if the height allows, else most of the height, so
    /// the drawing is as large as its aspect ratio permits. Controls keep their
    /// size below it.
    fn render_expanded_visual(
        &self,
        target: Self::Target,
        colors: &Theme,
        space: gpui::Size<gpui::Pixels>,
        cx: &Context<Self>,
    ) -> Option<gpui::Stateful<Div>> {
        let route = self.visual_route();
        let (key, source) = {
            let mut apps = self.visual_apps().borrow_mut();
            let (key, instance) = apps
                .running
                .iter_mut()
                .find(|(_, i)| i.expanded && i.route == Some(route))?;
            let canvas = instance
                .runtime
                .as_ref()
                .ok()
                .and_then(|r| r.definition.view.tallest_canvas());
            instance.zoom = canvas.map_or(1., |(height, view_box)| {
                expanded_zoom(height, view_box, space)
            });
            (key.clone(), instance.source.clone())
        };
        self.render_visual_card(&key, &source, 0, target, colors, cx)
    }

    fn render_visual_card(
        &self,
        key: &str,
        source: &str,
        ordinal: usize,
        target: Self::Target,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Option<gpui::Stateful<Div>> {
        let key = key.to_owned();
        let mut apps = self.visual_apps().borrow_mut();
        apps.ensure(&key, source, self.visual_persist());
        let instance = apps.running.get_mut(&key)?;
        if instance.route != Some(self.visual_route()) {
            instance.cancel_manipulation();
        }
        instance.route = Some(self.visual_route());
        let runtime = instance.runtime.as_ref().ok()?;
        let bundle: Arc<str> = Arc::from(source);
        let node = self.visual_node(
            &runtime.definition.view,
            instance,
            &key,
            &bundle,
            "root",
            target,
            colors,
            cx,
        );
        let source_copy = source.to_owned();
        let source_show = source.to_owned();
        let source_key = key.clone();
        let reset_key = key.clone();
        let reset_source = bundle.clone();
        let generation = instance.generation;
        let route = self.visual_route();
        let entity = cx.entity().downgrade();
        let frame_key = key.clone();
        let visibility = crate::visibility::Marker {
            id: format!("visual-presence-{key}-{generation}").into(),
            paint: Box::new(move |painted, window, cx| {
                let _ = entity.update(cx, |this, cx| {
                    {
                        let apps = this.visual_apps().borrow();
                        let Some(instance) = apps
                            .running
                            .get(&frame_key)
                            .filter(|i| i.generation == generation)
                        else {
                            return;
                        };
                        instance
                            .painted
                            .borrow_mut()
                            .insert("card".into(), Rc::downgrade(painted));
                    }
                    if painted.get() {
                        this.queue_visual_frame(&frame_key, window, cx);
                    }
                });
            }),
        };
        Some(
            div()
                .id(SharedString::from(format!("visual-{key}")))
                .debug_selector(|| "visual-app".into())
                .relative()
                .child(visibility)
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
                        .when(self.visual_expandable(), |header| {
                            let expand_key = key.clone();
                            let expanded = instance.expanded;
                            header.child(
                                ui::tap(
                                    ("visual-expand", ordinal),
                                    if expanded { "minimize" } else { "maximize" },
                                    colors,
                                )
                                .aria_label(if expanded {
                                    "Exit full screen"
                                } else {
                                    "Show full screen"
                                })
                                .debug_selector(|| "visual-expand".into())
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.visual_toggle_expanded(
                                            &expand_key,
                                            target,
                                            window,
                                            cx,
                                        );
                                    },
                                )),
                            )
                        })
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
                .children(instance.source_open.then(|| {
                    div()
                        .id(SharedString::from(format!("visual-code-{key}")))
                        .max_h(px(240.))
                        .overflow_y_scroll()
                        // Wheel and trackpad scrolling stay here instead of also
                        // moving the thread the card sits in.
                        .occlude()
                        .p(px(12.))
                        .font_family(MONO)
                        .text_size(px(12.))
                        .child(source.to_owned())
                }))
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
                                    if this.visual_event_allowed(
                                        &source_key,
                                        generation,
                                        route,
                                        window,
                                    ) {
                                        this.visual_show_source(
                                            &source_key,
                                            &source_show,
                                            target,
                                            window,
                                            cx,
                                        );
                                    }
                                },
                            )),
                        )
                        .child(
                            ui::tap(("visual-reset", ordinal), "refresh", colors)
                                .aria_label("Reset visual app state")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if !this
                                        .visual_event_allowed(&reset_key, generation, route, window)
                                    {
                                        return;
                                    }
                                    let mut apps = this.visual_apps().borrow_mut();
                                    apps.ensure(&reset_key, &reset_source, this.visual_persist());
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
                                        apps.remember_local(
                                            &reset_key,
                                            saved,
                                            this.visual_persist(),
                                        );
                                    }
                                    drop(apps);
                                    this.visual_changed(target, cx);
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
        instance: &Instance<Self::Route>,
        key: &str,
        bundle: &Arc<str>,
        path: &str,
        target: Self::Target,
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
                        target,
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
                        target,
                        colors,
                        cx,
                    )
                }))
                .into_any_element(),
            Node::Text { text, bind, style } => {
                let content = bind
                    .as_ref()
                    .map(|name| {
                        let _trace = crate::platform::trace::scope(c"pi.visual.text_lookup");
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
                let surface = Surface {
                    events: None,
                    held_button: false,
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
                        target,
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
                };
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(8.))
                            .child(ui::label(label.clone(), &colors).min_w_0().truncate())
                            .child(
                                ui::hint(value.to_string(), &colors)
                                    .font_family(MONO)
                                    .max_w(px(160.))
                                    .truncate(),
                            ),
                    )
                    .child(surface)
                    .into_any_element()
            }
            Node::Canvas {
                label,
                height,
                view_box,
                draw,
                pointer_action,
                input_action,
                ..
            } => {
                runtime.enable_profiling(crate::visual_profile::enabled(cx));
                let scene_result = {
                    let _trace = crate::platform::trace::scope(c"pi.visual.scene_lookup");
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
                    held_button: false,
                    events: input_action.as_ref().map(|action| {
                        self.visual_scene_input(key, action, instance.generation, target, cx)
                    }),
                    id: SharedString::from(format!(
                        "visual-canvas-{key}-{path}-{}",
                        instance.generation
                    )),
                    label: label.clone(),
                    probe: format!("visual-canvas-{path}"),
                    height: *height * instance.zoom,
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
                            target,
                            cx,
                        )
                    }),
                    painter: Rc::new(move |bounds, cache, window, cx| {
                        window.paint_quad(fill(bounds, colors.canvas).corner_radii(px(10.)));
                        let start = crate::visual_profile::enabled(cx).then(Instant::now);
                        let hits = cache.hits;
                        let builds = cache.builds;
                        let paint_result = visual_controls::paint_scene(
                            &scene, view_box, bounds, cache, window, cx,
                        );
                        if let Err(error) = &paint_result {
                            crate::visual_painter::paint_error(
                                &short_error(error),
                                bounds,
                                window,
                                cx,
                            );
                        }
                        crate::visual_profile::paths(
                            cx,
                            start,
                            cache.hits - hits,
                            cache.builds - builds,
                        );
                        let _ = entity.update(cx, |this, cx| {
                            {
                                let apps = this.visual_apps().borrow();
                                if let Some(instance) = apps.running.get(&frame_key)
                                    && instance.generation == visibility_generation
                                {
                                    instance.painted.borrow_mut().insert(
                                        visibility_path.clone(),
                                        Rc::downgrade(&cache.painted),
                                    );
                                }
                            }
                            if animate && paint_result.is_ok() && cache.painted.get() {
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
                hold,
            } => {
                let (bg, fg) = match tone.as_str() {
                    "accent" => (colors.accent, colors.on_accent),
                    "operator" => (colors.tint, colors.accent),
                    "utility" => (colors.selected, colors.secondary),
                    _ => (colors.canvas, colors.text),
                };
                if hold.is_some() {
                    let pressed = runtime.button_held(action);
                    let colors = *colors;
                    let label_text = label.clone();
                    return Surface {
                        id: SharedString::from(format!(
                            "visual-hold-{key}-{path}-{}",
                            instance.generation
                        )),
                        label: label.clone(),
                        probe: format!("visual-action-{action}"),
                        height: 48.,
                        held_button: true,
                        barrier: instance.barrier.clone(),
                        slider: None,
                        input: None,
                        events: Some(self.visual_scene_input(
                            key,
                            action,
                            instance.generation,
                            target,
                            cx,
                        )),
                        coordinates: Rc::new(|_, _| [0., 0.]),
                        painter: Rc::new(move |bounds, _, window, cx| {
                            window.paint_quad(
                                fill(bounds, if pressed { colors.selected } else { bg })
                                    .corner_radii(px(8.)),
                            );
                            crate::visual_painter::paint_button_label(
                                &label_text,
                                fg,
                                bounds,
                                window,
                                cx,
                            );
                        }),
                    }
                    .into_any_element();
                }
                let key = key.to_owned();
                let bundle = bundle.clone();
                let action = action.clone();
                let generation = instance.generation;
                let route = self.visual_route();
                div()
                    .id(SharedString::from(format!("{key}-{path}")))
                    .relative()
                    .child(crate::hooks::probe(format!("visual-action-{action}")))
                    .role(Role::Button)
                    .aria_label(label.clone())
                    .debug_selector({
                        let label = label.clone();
                        move || format!("visual-button-{label}")
                    })
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
                        if !this.visual_event_allowed(&key, generation, route, window) {
                            return;
                        }
                        let mut apps = this.visual_apps().borrow_mut();
                        apps.ensure(&key, &bundle, this.visual_persist());
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
                            apps.remember_local(&key, saved, this.visual_persist());
                        }
                        drop(apps);
                        window.dismiss_virtual_keyboard();
                        this.visual_refocus(window, cx);
                        this.visual_changed(target, cx);
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .into_any_element()
            }
        }
    }
}

impl<T: Host> HostExt for T {}

#[cfg(test)]
mod tests {
    use super::*;
    type VisualApps = super::VisualApps<u8>;
    #[test]
    fn expanded_drawings_fill_the_width_unless_the_height_runs_out() {
        let space = |w: f32, h: f32| gpui::size(gpui::px(w), gpui::px(h));
        // 4:3 drawing in a tall space: as wide as the card.
        let zoom = expanded_zoom(120., [0., 0., 400., 300.], space(826., 2000.));
        assert!((120. * zoom - 600.).abs() < 0.01, "{zoom}");
        // The same drawing in a wide, short window: 80% of the height.
        let zoom = expanded_zoom(120., [0., 0., 400., 300.], space(2000., 600.));
        assert!((120. * zoom - 480.).abs() < 0.01, "{zoom}");
        // Never smaller than in the card.
        assert_eq!(
            expanded_zoom(480., [0., 0., 400., 300.], space(300., 300.)),
            1.
        );
    }
    #[test]
    fn invalid_route_or_sheet_rolls_back_before_unblocking_projection() {
        let route = 1u8;
        for (next_route, available) in [(0u8, true), (route, false)] {
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
        let source = include_str!("../../pi_visual/assets/samples/spirograph-trace.pi-visual.js");
        apps.ensure("visible", source, false);
        let instance = apps.running.get_mut("visible").unwrap();
        let route = 1u8;
        instance.route = Some(route);
        let painted = Rc::new(Cell::new(true));
        instance
            .painted
            .borrow_mut()
            .insert("canvas".into(), Rc::downgrade(&painted));
        assert!(instance.visible(route));
        assert!(!instance.visible(0u8));
        painted.set(false);
        assert!(!instance.visible(route));
        drop(painted);
        assert!(!instance.visible(route));
    }

    #[test]
    fn local_state_is_bounded_and_restored_only_for_the_same_bundle() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().to_path_buf();
        let prefs = directory.join("settings.json");
        let source = include_str!("../../pi_visual/assets/samples/calculator.pi-visual.js");
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
        let source = include_str!("../../pi_visual/assets/samples/calculator.pi-visual.js");
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
        let source = include_str!("../../pi_visual/assets/samples/calculator.pi-visual.js");
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
