//! Finger-tracked motion with a frame-rate-independent, interruptible settle.
//! A gesture never performs its action until release; cancellation returns to rest.

use gpui::{SpringConfig, SpringState};
use std::time::{Duration, Instant};

// Critically damped: a deliberate settle without bouncing destructive actions.
const SPRING: SpringConfig = SpringConfig::new(256., 32., 1.);
const EPSILON: f32 = 0.001;

pub(crate) struct SwipeMotion {
    state: SpringState,
    target: f32,
    dragging: bool,
    sampled_at: Instant,
}

impl SwipeMotion {
    pub(crate) fn at(position: f32) -> Self {
        Self {
            state: SpringState {
                position,
                velocity: 0.,
            },
            target: position,
            dragging: false,
            sampled_at: Instant::now(),
        }
    }

    pub(crate) fn position(&self) -> f32 {
        self.state.position.clamp(0., 1.)
    }

    pub(crate) fn dragging(&self) -> bool {
        self.dragging
    }

    pub(crate) fn animating(&self) -> bool {
        !self.dragging && !SPRING.is_settled(self.state, self.target, EPSILON)
    }

    pub(crate) fn begin_drag(&mut self) {
        // Catch an in-flight panel where it actually is, not at its target.
        self.dragging = true;
        self.state.velocity = 0.;
        self.sampled_at = Instant::now();
    }

    pub(crate) fn drag_by(&mut self, delta: f32) {
        let now = Instant::now();
        let dt = now.duration_since(self.sampled_at).as_secs_f32();
        let previous = self.state.position;
        self.state.position = (previous + delta).clamp(0., 1.);
        if dt > 0.001 {
            let velocity = (self.state.position - previous) / dt;
            self.state.velocity = (self.state.velocity * 0.5 + velocity * 0.5).clamp(-3., 3.);
        }
        self.sampled_at = now;
    }

    pub(crate) fn settle(&mut self, target: f32) {
        if self.dragging && self.sampled_at.elapsed() > Duration::from_millis(80) {
            self.state.velocity = 0.;
        }
        self.target = target;
        self.dragging = false;
        self.sampled_at = Instant::now();
    }

    pub(crate) fn finish(&mut self) {
        self.state = SpringState {
            position: self.target,
            velocity: 0.,
        };
        self.dragging = false;
    }

    pub(crate) fn tick(&mut self, now: Instant, reduce_motion: bool) {
        if self.dragging {
            return;
        }
        let elapsed = now.saturating_duration_since(self.sampled_at);
        self.sampled_at = now;
        if reduce_motion {
            self.finish();
        } else {
            self.advance(elapsed);
        }
    }

    fn advance(&mut self, elapsed: Duration) {
        self.state = SPRING.step(self.state, self.target, elapsed.as_secs_f32());
        // The visible range has hard edges; never spring past them on release.
        if !(0. ..=1.).contains(&self.state.position) {
            self.state.position = self.state.position.clamp(0., 1.);
            self.state.velocity = 0.;
        }
        if !self.animating() {
            self.finish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_keeps_position_and_settles_instead_of_snapping() {
        let mut motion = SwipeMotion::at(0.);
        motion.begin_drag();
        motion.drag_by(0.6);
        assert!(motion.dragging());
        assert_eq!(motion.position(), 0.6);
        motion.settle(1.);
        assert_eq!(motion.position(), 0.6);
        motion.advance(Duration::from_millis(16));
        assert!(motion.position() > 0.6 && motion.position() < 0.7);
        motion.advance(Duration::from_secs(1));
        assert_eq!(motion.position(), 1.);
        assert!(!motion.animating());
    }

    #[test]
    fn a_cancelled_partial_drag_returns_smoothly_and_can_be_interrupted() {
        let mut motion = SwipeMotion::at(0.);
        motion.begin_drag();
        motion.drag_by(0.2);
        motion.settle(0.);
        motion.advance(Duration::from_millis(80));
        let interrupted = motion.position();
        assert!(interrupted > 0. && interrupted < 0.2);
        motion.begin_drag();
        assert_eq!(motion.position(), interrupted);
        motion.drag_by(0.1);
        assert!((motion.position() - interrupted - 0.1).abs() < 1e-6);
        motion.settle(0.);
        motion.advance(Duration::from_secs(1));
        assert_eq!(motion.position(), 0.);
    }

    #[test]
    fn settle_is_consistent_at_60_and_120_hz_and_respects_reduced_motion() {
        let run = |frames, dt| {
            let mut motion = SwipeMotion::at(1.);
            motion.settle(0.);
            for _ in 0..frames {
                motion.advance(Duration::from_secs_f32(dt));
            }
            motion.position()
        };
        assert!((run(12, 1. / 60.) - run(24, 1. / 120.)).abs() < 1e-5);
        let mut motion = SwipeMotion::at(0.);
        motion.settle(1.);
        motion.tick(Instant::now(), true);
        assert_eq!(motion.position(), 1.);
        assert!(!motion.animating());
    }
}
