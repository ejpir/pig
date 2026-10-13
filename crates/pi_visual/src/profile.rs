//! Opt-in, host-only bounded CPU timing. Never exposed to generated scripts.
use std::{
    cell::Cell,
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Callback,
    DrawScript,
    TextScript,
    Decode,
}
#[derive(Default)]
struct Samples {
    times: VecDeque<(Stage, Duration)>,
    hits: u64,
    misses: u64,
}
#[derive(Default)]
pub(crate) struct Profile {
    enabled: Cell<bool>,
    sink: Arc<Mutex<Samples>>,
}
#[derive(Default)]
pub struct Batch {
    pub times: Vec<(Stage, Duration)>,
    pub scene_hits: u64,
    pub scene_misses: u64,
}
impl Profile {
    pub fn enable(&self, enabled: bool) {
        if self.enabled.replace(enabled) != enabled {
            *self.sink.lock().unwrap() = Samples::default();
        }
    }
    pub fn timer(&self, stage: Stage) -> Timer {
        Timer {
            sink: self.enabled.get().then(|| self.sink.clone()),
            stage,
            start: self.enabled.get().then(Instant::now),
        }
    }
    pub fn cache(&self, hit: bool) {
        if self.enabled.get() {
            let mut samples = self.sink.lock().unwrap();
            if hit {
                samples.hits += 1;
            } else {
                samples.misses += 1;
            }
        }
    }
    pub fn take(&self) -> Batch {
        if !self.enabled.get() {
            return Batch::default();
        }
        let mut samples = self.sink.lock().unwrap();
        Batch {
            times: samples.times.drain(..).collect(),
            scene_hits: std::mem::take(&mut samples.hits),
            scene_misses: std::mem::take(&mut samples.misses),
        }
    }
}
pub(crate) struct Timer {
    sink: Option<Arc<Mutex<Samples>>>,
    stage: Stage,
    start: Option<Instant>,
}
impl Drop for Timer {
    fn drop(&mut self) {
        if let Some(start) = self.start
            && let Some(sink) = self.sink.as_ref()
        {
            let mut samples = sink.lock().unwrap();
            if samples.times.len() == 120 {
                samples.times.pop_front();
            }
            samples.times.push_back((self.stage, start.elapsed()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiling_is_opt_in_bounded_and_drainable() {
        let profile = Profile::default();
        drop(profile.timer(Stage::Callback));
        assert!(profile.take().times.is_empty());
        profile.enable(true);
        for _ in 0..140 {
            drop(profile.timer(Stage::DrawScript));
            profile.cache(true);
        }
        let batch = profile.take();
        assert_eq!(batch.times.len(), 120);
        assert_eq!(batch.scene_hits, 140);
        assert!(profile.take().times.is_empty());
        profile.enable(false);
        drop(profile.timer(Stage::Callback));
        assert!(profile.take().times.is_empty());
    }
}
