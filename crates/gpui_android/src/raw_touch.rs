//! Opt-in multi-contact routing implemented entirely in the Android adapter.
//!
//! GPUI's existing TouchDrag dispatch supplies hit testing/paint-order listeners.
//! A scoped envelope distinguishes our raw offer from GPUI's single-contact
//! recognizer. Ordinary TouchDrag listeners must ignore `current()` offers.
//! Only `claim()` accepts a start; unclaimed contacts follow GPUI unchanged.
use gpui::{
    DispatchEventResult, Pixels, PlatformInput, Point, TouchDragEvent, TouchEvent, TouchId,
    TouchPhase,
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

thread_local! { static CURRENT: RefCell<Option<(TouchEvent, bool)>> = const { RefCell::new(None) }; }
/// Present only during synchronous adapter offers, never during ordinary drags.
pub fn current() -> Option<TouchEvent> {
    CURRENT.with(|v| v.borrow().as_ref().map(|v| v.0.clone()))
}
pub fn claim() {
    CURRENT.with(|v| {
        if let Some((_, claimed)) = v.borrow_mut().as_mut() {
            *claimed = true;
        }
    });
}
struct Scope(Option<(TouchEvent, bool)>);
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|v| *v.borrow_mut() = self.0.take());
    }
}

#[derive(Default)]
pub struct Router {
    captured: HashMap<TouchId, Point<Pixels>>,
    ordinary: HashSet<TouchId>,
    raw_gesture: bool,
}
impl Router {
    pub fn active(&self) -> bool {
        !self.captured.is_empty()
    }
    /// Keep quarantine through terminal pinch delivery; reset on the next gesture.
    pub fn blocks_pinch(&self) -> bool {
        self.raw_gesture
    }
    /// Returns whether this sample belongs to an exclusively owned raw stream.
    pub fn dispatch(
        &mut self,
        event: TouchEvent,
        mut dispatch: impl FnMut(PlatformInput) -> Option<DispatchEventResult>,
    ) -> bool {
        let started = event.phase == TouchPhase::Started;
        let ended = matches!(event.phase, TouchPhase::Ended | TouchPhase::Cancelled);
        if started && self.captured.is_empty() && self.ordinary.is_empty() {
            self.raw_gesture = false;
        }
        let owned = self.captured.get(&event.id).copied();
        if owned.is_some()
            || (started && self.captured.len() < 10 && event.id.0 < 9_007_199_254_740_991)
        {
            let previous = CURRENT.with(|v| v.replace(Some((event.clone(), false))));
            let scope = Scope(previous);
            dispatch(PlatformInput::TouchDrag(TouchDragEvent {
                phase: event.phase,
                start_position: owned.unwrap_or(event.position),
                position: event.position,
            }));
            let claimed = CURRENT.with(|v| v.borrow().as_ref().is_some_and(|v| v.1));
            drop(scope);
            if owned.is_some() || claimed {
                self.raw_gesture = true;
                if started {
                    self.captured.insert(event.id, event.position);
                    // Same offscreen start/cancel technique as touch::release_at.
                    // Do not disturb a different ordinary contact's recognition.
                    if self.ordinary.is_empty() {
                        let away = gpui::point(gpui::px(-10000.), gpui::px(-10000.));
                        for phase in [TouchPhase::Started, TouchPhase::Cancelled] {
                            dispatch(PlatformInput::Touch(TouchEvent {
                                id: TouchId(u64::MAX),
                                phase,
                                position: away,
                                predicted_position: None,
                                force: None,
                            }));
                        }
                    }
                }
                if ended {
                    self.captured.remove(&event.id);
                }
                return true;
            }
        }
        if started {
            self.ordinary.insert(event.id);
        }
        if ended {
            self.ordinary.remove(&event.id);
        }
        dispatch(PlatformInput::Touch(event));
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(id: u64, phase: TouchPhase) -> TouchEvent {
        TouchEvent {
            id: TouchId(id),
            phase,
            ..Default::default()
        }
    }
    #[test]
    fn independent_capture_fallback_and_removed_owner_never_becomes_a_tap() {
        let mut r = Router::default();
        let mut normal = 0;
        for id in 1..=2 {
            assert!(r.dispatch(event(id, TouchPhase::Started), |e| {
                if matches!(e, PlatformInput::TouchDrag(_)) {
                    claim();
                }
                None
            }));
        }
        assert!(r.active());
        assert!(r.dispatch(event(1, TouchPhase::Moved), |_| None));
        assert!(r.dispatch(event(1, TouchPhase::Ended), |_| None));
        assert!(r.active());
        assert!(r.dispatch(event(2, TouchPhase::Cancelled), |_| None));
        assert!(!r.active());
        assert!(!r.dispatch(event(3, TouchPhase::Started), |e| {
            if matches!(e, PlatformInput::Touch(_)) {
                normal += 1;
            }
            None
        }));
        assert_eq!(normal, 1);
        assert!(current().is_none());
    }
    #[test]
    fn raw_quarantine_does_not_suppress_the_next_ordinary_pinch() {
        let mut r = Router::default();
        r.dispatch(event(1, TouchPhase::Started), |_| {
            claim();
            None
        });
        assert!(r.blocks_pinch());
        r.dispatch(event(1, TouchPhase::Ended), |_| None);
        assert!(r.blocks_pinch()); // terminal pinch from same physical batch stays suppressed
        r.dispatch(event(2, TouchPhase::Started), |_| None);
        r.dispatch(event(3, TouchPhase::Started), |_| None);
        assert!(!r.blocks_pinch());
        r.dispatch(event(4, TouchPhase::Started), |e| {
            if matches!(e, PlatformInput::TouchDrag(_)) {
                claim();
            }
            None
        });
        assert!(r.blocks_pinch()); // new ownership suppresses the remainder of this gesture
        for id in 2..=4 {
            r.dispatch(event(id, TouchPhase::Ended), |_| None);
        }
        r.dispatch(event(5, TouchPhase::Started), |_| None);
        assert!(!r.blocks_pinch());
    }
    #[test]
    fn scope_is_cleared_on_unwind() {
        let _ = std::panic::catch_unwind(|| {
            Router::default().dispatch(event(1, TouchPhase::Started), |_| panic!("listener"))
        });
        assert!(current().is_none());
    }
}
