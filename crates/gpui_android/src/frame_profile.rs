//! Bounded opt-in native CPU diagnostics. Present requests are NOT presented frames.
use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
static ENABLED: AtomicBool = AtomicBool::new(false);
static RECORDING: Mutex<Option<Recording>> = Mutex::new(None);
const LIMIT: usize = 120;
#[derive(Clone, Copy)]
pub enum Stage {
    InputAge,
    InputGap,
    VsyncGap,
    WakeDelay,
    Acquire,
    EncodeSubmit,
    PresentCall,
    MainTasks,
    Backend,
}
impl Stage {
    fn index(self) -> usize {
        self as usize
    }
}
#[derive(Default)]
struct Recording {
    samples: [VecDeque<Duration>; 9],
    last_input: Option<i64>,
    last_vsync: Option<i64>,
    inputs: u64,
    history_omitted: u64,
    draws: u64,
    present_requests: u64,
}
#[derive(Default, Clone, Debug)]
pub struct Snapshot {
    pub p95_ms: [f64; 9],
    pub input_hz: f64,
    pub tick_hz: f64,
    pub inputs: u64,
    pub history_omitted: u64,
    pub draws: u64,
    pub present_requests: u64,
}
impl Snapshot {
    pub fn p95(&self, stage: Stage) -> f64 {
        self.p95_ms[stage.index()]
    }
}
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}
pub fn set_enabled(enabled: bool) {
    if ENABLED.load(Ordering::Relaxed) == enabled {
        return;
    }
    let mut recording = RECORDING.lock().unwrap();
    *recording = enabled.then(Recording::default);
    ENABLED.store(enabled, Ordering::Relaxed);
}
fn with_recording(f: impl FnOnce(&mut Recording)) {
    if enabled()
        && let Some(r) = RECORDING.lock().unwrap().as_mut()
    {
        f(r);
    }
}
impl Recording {
    fn record(&mut self, stage: Stage, duration: Duration) {
        let samples = &mut self.samples[stage.index()];
        if samples.len() == LIMIT {
            samples.pop_front();
        }
        samples.push_back(duration);
    }
    fn gap(&mut self, stage: Stage, previous: Option<i64>, now: i64) {
        if let Some(last) = previous
            && now > last
        {
            let nanos = (now - last) as u64;
            if nanos <= 250_000_000 {
                self.record(stage, Duration::from_nanos(nanos));
            } else {
                self.samples[stage.index()].clear();
            }
        }
    }
}
pub fn record(stage: Stage, duration: Duration) {
    with_recording(|r| r.record(stage, duration));
}
pub(crate) fn input(event_ns: i64, now_ns: i64, history: usize) {
    with_recording(|r| {
        let previous = r.last_input.replace(event_ns);
        r.gap(Stage::InputGap, previous, event_ns);
        if now_ns >= event_ns {
            r.record(
                Stage::InputAge,
                Duration::from_nanos((now_ns - event_ns) as u64),
            );
        }
        r.inputs += 1;
        r.history_omitted += history as u64;
    });
}
pub(crate) fn vsync(frame_ns: i64) {
    with_recording(|r| {
        let previous = r.last_vsync.replace(frame_ns);
        r.gap(Stage::VsyncGap, previous, frame_ns);
    });
}
pub(crate) fn draw(present_requested: bool) {
    with_recording(|r| {
        r.draws += 1;
        r.present_requests += u64::from(present_requested);
    });
}
pub fn snapshot() -> Option<Snapshot> {
    if !enabled() {
        return None;
    }
    let recording = RECORDING.lock().unwrap();
    let r = recording.as_ref()?;
    let mut result = Snapshot {
        inputs: r.inputs,
        history_omitted: r.history_omitted,
        draws: r.draws,
        present_requests: r.present_requests,
        ..Default::default()
    };
    for (index, ring) in r.samples.iter().enumerate() {
        let mut values: Vec<_> = ring.iter().copied().collect();
        values.sort_unstable();
        result.p95_ms[index] = values
            .get(values.len().saturating_sub(1) * 95 / 100)
            .map_or(0., |d| d.as_secs_f64() * 1000.);
    }
    let hz = |stage: Stage| {
        let ring = &r.samples[stage.index()];
        let seconds: f64 = ring.iter().map(Duration::as_secs_f64).sum();
        if seconds > 0. {
            ring.len() as f64 / seconds
        } else {
            0.
        }
    };
    result.input_hz = hz(Stage::InputGap);
    result.tick_hz = hz(Stage::VsyncGap);
    Some(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_samples_are_bounded_and_idle_is_not_a_slow_vsync() {
        let mut r = Recording::default();
        for _ in 0..300 {
            r.record(Stage::Acquire, Duration::from_millis(2));
        }
        assert_eq!(r.samples[Stage::Acquire.index()].len(), LIMIT);
        r.gap(Stage::VsyncGap, Some(0), 8_333_333);
        assert_eq!(r.samples[Stage::VsyncGap.index()].len(), 1);
        r.gap(Stage::VsyncGap, Some(0), 2_000_000_000);
        assert!(r.samples[Stage::VsyncGap.index()].is_empty());
    }
}

#[cfg(target_os = "android")]
pub(crate) fn monotonic_ns() -> Option<i64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid initialized writable timespec; CLOCK_MONOTONIC matches Android input/vsync timestamps.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } == 0 {
        time.tv_sec
            .checked_mul(1_000_000_000)?
            .checked_add(time.tv_nsec)
    } else {
        None
    }
}
