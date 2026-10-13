//! Opt-in diagnostics; bounded, aggregate-only, no coordinates/source/state logging.
use gpui::{App, Global};
use std::{
    cell::RefCell,
    collections::VecDeque,
    time::{Duration, Instant},
};

/// Trusted enum-to-literal adapter. Opaque guards contain only the native trace
/// guard, never VM values. Disabled tracing allocates nothing and reads no clocks.
pub(crate) fn install_trace() {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        pi_visual::trace::install(trace_phase);
    });
}
fn trace_phase(phase: pi_visual::trace::Phase) -> Option<Box<dyn std::any::Any>> {
    if !gpui_android::trace::enabled() {
        return None;
    }
    use pi_visual::trace::Phase;
    let name = match phase {
        Phase::Execute => c"pi.js.execute",
        Phase::CreateVm => c"pi.js.vm.create",
        Phase::Compile => c"pi.js.compile",
        Phase::Bootstrap => c"pi.js.bootstrap",
        Phase::AppRegistration => c"pi.js.app.registration",
        Phase::BytecodeRead => c"pi.js.bytecode.read",
        Phase::BytecodeEval => c"pi.js.bytecode.eval",
        Phase::Definition => c"pi.js.definition",
        Phase::Bridge => c"pi.js.bridge",
        Phase::Exception => c"pi.js.exception",
        Phase::Checks => c"pi.js.checks",
        Phase::ValidateDefinition => c"pi.js.definition.validate",
        Phase::SceneHit => c"pi.js.scene.hit",
        Phase::SceneMiss => c"pi.js.scene.miss",
        Phase::DecodeScene => c"pi.js.scene.decode",
        Phase::ValidateScene => c"pi.js.scene.validate",
        Phase::TextDirect => c"pi.js.text.direct",
        Phase::TextHit => c"pi.js.text.hit",
        Phase::TextMiss => c"pi.js.text.miss",
        Phase::TextFailure => c"pi.js.text.failure",
        Phase::DecodeTexts => c"pi.js.text.decode",
    };
    Some(Box::new(gpui_android::trace::scope(name)))
}

const LIMIT: usize = 120;
#[derive(Clone, Copy)]
pub(crate) enum Stage {
    InputGap,
    QueueWait,
    Callback,
    DrawScript,
    TextScript,
    Decode,
    Paths,
    LayoutPrepaint,
    Paint,
    InputToPaint,
}
impl Stage {
    fn index(self) -> usize {
        self as usize
    }
}
#[derive(Default)]
struct Recording {
    enabled: bool,
    samples: [VecDeque<Duration>; 10],
    last_input: Option<Instant>,
    pending_paint: Option<Instant>,
    received: u64,
    delivered: u64,
    replaced: u64,
    duplicates: u64,
    scene_hits: u64,
    scene_misses: u64,
    path_hits: u64,
    path_builds: u64,
    last_label: Option<Instant>,
    cached_label: String,
}
#[derive(Default)]
pub(crate) struct Profile(RefCell<Recording>);
impl Global for Profile {}
impl Recording {
    fn record(&mut self, stage: Stage, elapsed: Duration) {
        let samples = &mut self.samples[stage.index()];
        if samples.len() == LIMIT {
            samples.pop_front();
        }
        samples.push_back(elapsed);
    }
    fn p95(&self, stage: Stage) -> f64 {
        let mut samples: Vec<_> = self.samples[stage.index()].iter().copied().collect();
        samples.sort_unstable();
        samples
            .get(samples.len().saturating_sub(1) * 95 / 100)
            .map_or(0., |d| d.as_secs_f64() * 1000.)
    }
    fn input_hz(&self) -> f64 {
        let samples = &self.samples[Stage::InputGap.index()];
        let seconds: f64 = samples.iter().map(Duration::as_secs_f64).sum();
        if seconds > 0. {
            samples.len() as f64 / seconds
        } else {
            0.
        }
    }
}
pub(crate) fn enable(cx: &mut App, enabled: bool) {
    let mut r = cx.default_global::<Profile>().0.borrow_mut();
    if r.enabled != enabled {
        *r = Recording {
            enabled,
            ..Default::default()
        };
    }
}
pub(crate) fn enabled(cx: &App) -> bool {
    cx.try_global::<Profile>()
        .is_some_and(|p| p.0.borrow().enabled)
}
pub(crate) fn receive(cx: &App, replaced: bool) -> Option<Instant> {
    let profile = cx.try_global::<Profile>()?;
    let mut r = profile.0.borrow_mut();
    if !r.enabled {
        return None;
    }
    let now = Instant::now();
    if let Some(last) = r.last_input.replace(now) {
        let elapsed = now.duration_since(last);
        if elapsed <= Duration::from_millis(250) {
            r.record(Stage::InputGap, elapsed);
        } else {
            r.samples[Stage::InputGap.index()].clear();
        }
    }
    r.received += 1;
    r.replaced += u64::from(replaced);
    Some(now)
}
pub(crate) fn deliver(cx: &App, receipt: Option<Instant>) {
    if let Some(receipt) = receipt
        && let Some(p) = cx.try_global::<Profile>()
    {
        let mut r = p.0.borrow_mut();
        if r.enabled {
            r.delivered += 1;
            r.pending_paint = Some(receipt);
            r.record(Stage::QueueWait, receipt.elapsed());
        }
    }
}
pub(crate) fn duplicate(cx: &App) {
    if let Some(p) = cx.try_global::<Profile>() {
        let mut r = p.0.borrow_mut();
        if r.enabled {
            r.duplicates += 1;
            r.pending_paint = None;
        }
    }
}
pub(crate) fn runtime(cx: &App, runtime: &pi_visual::Runtime) {
    let batch = runtime.take_profile();
    if let Some(p) = cx.try_global::<Profile>() {
        let mut r = p.0.borrow_mut();
        if !r.enabled {
            return;
        }
        r.scene_hits += batch.scene_hits;
        r.scene_misses += batch.scene_misses;
        for (stage, elapsed) in batch.times {
            let stage = match stage {
                pi_visual::profile::Stage::Callback => Stage::Callback,
                pi_visual::profile::Stage::DrawScript => Stage::DrawScript,
                pi_visual::profile::Stage::TextScript => Stage::TextScript,
                pi_visual::profile::Stage::Decode => Stage::Decode,
            };
            r.record(stage, elapsed);
        }
    }
}
pub(crate) fn paths(cx: &App, start: Option<Instant>, hits: u64, builds: u64) {
    if let Some(start) = start
        && let Some(p) = cx.try_global::<Profile>()
    {
        let mut r = p.0.borrow_mut();
        if !r.enabled {
            return;
        }
        r.path_hits += hits;
        r.path_builds += builds;
        r.record(Stage::Paths, start.elapsed());
    }
}
pub(crate) fn label(cx: &App) -> String {
    let Some(p) = cx.try_global::<Profile>() else {
        return String::new();
    };
    let mut r = p.0.borrow_mut();
    if !r.enabled {
        return String::new();
    }
    let now = Instant::now();
    if r.last_label
        .is_some_and(|last| now.duration_since(last) < Duration::from_millis(100))
    {
        return r.cached_label.clone();
    }
    let label = format!(
        "\ninput {:.0} Hz · wait95 {:.1} ms\nVM95 {:.2} · decode95 {:.2} · cb95 {:.2}\ntext95 {:.2} · paths95 {:.2} ms · reuse {}/{}\nmoves {}/{} · coalesce {} · same {}",
        r.input_hz(),
        r.p95(Stage::QueueWait),
        r.p95(Stage::DrawScript),
        r.p95(Stage::Decode),
        r.p95(Stage::Callback),
        r.p95(Stage::TextScript),
        r.p95(Stage::Paths),
        r.path_hits,
        r.path_hits + r.path_builds,
        r.delivered,
        r.received,
        r.replaced,
        r.duplicates
    ) + format!(
        "\nlayout+pre95 {:.2} · paint95 {:.2}\nin→paint95 {:.1} ms (not display)",
        r.p95(Stage::LayoutPrepaint),
        r.p95(Stage::Paint),
        r.p95(Stage::InputToPaint)
    )
    .as_str()
        + native_label().as_str();
    r.last_label = Some(now);
    r.cached_label = label.clone();
    label
}
fn native_label() -> String {
    use gpui_android::frame_profile::{self, Stage as S};
    let Some(s) = frame_profile::snapshot() else {
        return String::new();
    };
    format!(
        "\nCPU95 acquire {:.2} · submit {:.2}\npresent-call95 {:.2} · service95 {:.2} ms\nnative input {:.0} Hz · tick {:.0} Hz",
        s.p95(S::Acquire),
        s.p95(S::EncodeSubmit),
        s.p95(S::PresentCall),
        s.p95(S::WakeDelay),
        s.input_hz,
        s.tick_hz
    )
}
#[cfg(feature = "ui-test")]
pub(crate) fn telemetry(cx: &App) -> serde_json::Value {
    let Some(p) = cx.try_global::<Profile>() else {
        return serde_json::Value::Null;
    };
    let r = p.0.borrow();
    serde_json::json!({"enabled":r.enabled,"input_hz":r.input_hz(),
        "queue_p95_ms":r.p95(Stage::QueueWait), "callback_p95_ms":r.p95(Stage::Callback),
        "draw_vm_p95_ms":r.p95(Stage::DrawScript),"text_vm_p95_ms":r.p95(Stage::TextScript),"decode_p95_ms":r.p95(Stage::Decode),
        "paths_p95_ms":r.p95(Stage::Paths),
        "layout_prepaint_p95_ms":r.p95(Stage::LayoutPrepaint),"paint_p95_ms":r.p95(Stage::Paint),
        "input_to_paint_p95_ms":r.p95(Stage::InputToPaint),"received":r.received,"delivered":r.delivered,
        "replaced":r.replaced,"duplicates":r.duplicates,"scene_hits":r.scene_hits,
        "scene_misses":r.scene_misses,"path_hits":r.path_hits,"path_builds":r.path_builds,
        "native":gpui_android::frame_profile::snapshot().map(|s|serde_json::json!({"p95_ms":s.p95_ms,"main_tasks_p95_ms":s.p95(gpui_android::frame_profile::Stage::MainTasks),"tick_service_p95_ms":s.p95(gpui_android::frame_profile::Stage::WakeDelay),"tick_gap_p95_ms":s.p95(gpui_android::frame_profile::Stage::VsyncGap),"input_age_p95_ms":s.p95(gpui_android::frame_profile::Stage::InputAge),"input_hz":s.input_hz,"tick_hz":s.tick_hz,"inputs":s.inputs,"history_omitted":s.history_omitted,"draws":s.draws,"present_requests":s.present_requests}))})
}
// Transparent root wrapper: request-layout start through root prepaint end
// includes Taffy layout; paint end is CPU scene construction, not presentation.
// It adds no element id, style, clipping, focus or accessibility role.
pub(crate) struct Root(gpui::AnyElement);
pub(crate) fn root(element: impl gpui::IntoElement) -> Root {
    Root(element.into_any_element())
}
impl gpui::IntoElement for Root {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl gpui::Element for Root {
    type RequestLayoutState = Option<Instant>;
    type PrepaintState = ();
    fn id(&self) -> Option<gpui::ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut gpui::Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let start = enabled(cx).then(Instant::now);
        let _trace = gpui_android::trace::scope(c"pi.root.request_layout");
        (self.0.request_layout(window, cx), start)
    }
    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: gpui::Bounds<gpui::Pixels>,
        start: &mut Self::RequestLayoutState,
        window: &mut gpui::Window,
        cx: &mut App,
    ) {
        // Do not retain a synchronous trace guard in arena-backed element
        // state: a panic between layout and prepaint must not leave it open.
        let _trace = gpui_android::trace::scope(c"pi.root.prepaint");
        self.0.prepaint(window, cx);
        if let Some(start) = start.take()
            && let Some(p) = cx.try_global::<Profile>()
        {
            p.0.borrow_mut()
                .record(Stage::LayoutPrepaint, start.elapsed());
        }
    }
    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: gpui::Bounds<gpui::Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut (),
        window: &mut gpui::Window,
        cx: &mut App,
    ) {
        let _trace = gpui_android::trace::scope(c"pi.root.paint");
        let start = enabled(cx).then(Instant::now);
        self.0.paint(window, cx);
        if let Some(start) = start
            && let Some(p) = cx.try_global::<Profile>()
        {
            let mut r = p.0.borrow_mut();
            r.record(Stage::Paint, start.elapsed());
            if let Some(receipt) = r.pending_paint.take()
                && receipt.elapsed() <= Duration::from_millis(250)
            {
                r.record(Stage::InputToPaint, receipt.elapsed());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timing_samples_are_bounded_and_percentiles_use_real_samples() {
        let mut r = Recording::default();
        assert_eq!(r.p95(Stage::QueueWait), 0.);
        for i in 0..200 {
            r.record(Stage::QueueWait, Duration::from_millis(i));
        }
        assert_eq!(r.samples[Stage::QueueWait.index()].len(), LIMIT);
        assert_eq!(r.p95(Stage::QueueWait), 193.);
    }
}
