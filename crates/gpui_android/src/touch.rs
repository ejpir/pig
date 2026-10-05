//! Android motion events as GPUI touches.
//!
//! Android reuses pointer ids as soon as a finger lifts, and batches several
//! samples into one move event. GPUI wants a fresh [`TouchId`] for every
//! contact and every sample in order, so the recognizers see the real path and
//! velocity of a fling.

use gpui::{Pixels, Point, TouchEvent, TouchId, TouchPhase};

/// One pointer of a motion event, already in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Contact {
    /// Android's pointer id, stable only while this contact is down.
    pub id: i32,
    pub position: Point<Pixels>,
    /// Normalized pressure, when the hardware reports one.
    pub force: Option<f32>,
}

/// What a motion event did, with the pointer index for the actions that name one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Down,
    PointerDown(usize),
    Move,
    PointerUp(usize),
    Up,
    Cancel,
    /// Hover, scroll and button events; not touches.
    Other,
}

#[derive(Default)]
pub(crate) struct Touches {
    next: u64,
    /// Contacts that are down: Android's id, GPUI's id, last reported position.
    active: Vec<(i32, TouchId, Point<Pixels>)>,
}

impl Touches {
    /// The GPUI events for one motion event. `history` holds the batched samples
    /// before `current`, oldest first, each with one entry per pointer.
    pub fn translate(
        &mut self,
        action: Action,
        history: &[Vec<Contact>],
        current: &[Contact],
    ) -> Vec<TouchEvent> {
        let mut events = Vec::new();
        match action {
            Action::Down => {
                // A new gesture: anything still down was lost without an up.
                self.cancel_all(&mut events);
                if let Some(contact) = current.first() {
                    self.start(contact, &mut events);
                }
            }
            Action::PointerDown(index) => {
                if let Some(contact) = current.get(index) {
                    self.start(contact, &mut events);
                }
            }
            Action::Move => {
                let samples = history.iter().map(Vec::as_slice);
                for sample in samples.chain(std::iter::once(current)) {
                    for contact in sample {
                        self.moved(contact, &mut events);
                    }
                }
            }
            Action::PointerUp(index) => {
                if let Some(contact) = current.get(index) {
                    self.end(contact, TouchPhase::Ended, &mut events);
                }
            }
            Action::Up => {
                if let Some(contact) = current.first() {
                    self.end(contact, TouchPhase::Ended, &mut events);
                }
                // The last finger is up; nothing else can still be down.
                self.cancel_all(&mut events);
            }
            Action::Cancel => self.cancel_all(&mut events),
            Action::Other => {}
        }
        events
    }

    fn start(&mut self, contact: &Contact, events: &mut Vec<TouchEvent>) {
        if let Some(index) = self.active.iter().position(|(id, ..)| *id == contact.id) {
            // Android reused a pointer id we still think is down.
            let (_, touch, position) = self.active.remove(index);
            events.push(event(touch, TouchPhase::Cancelled, position, None));
        }
        let touch = TouchId(self.next);
        self.next += 1;
        self.active.push((contact.id, touch, contact.position));
        events.push(event(
            touch,
            TouchPhase::Started,
            contact.position,
            contact.force,
        ));
    }

    fn moved(&mut self, contact: &Contact, events: &mut Vec<TouchEvent>) {
        let Some((_, touch, last)) = self.active.iter_mut().find(|(id, ..)| *id == contact.id)
        else {
            return;
        };
        if *last == contact.position {
            return;
        }
        *last = contact.position;
        events.push(event(
            *touch,
            TouchPhase::Moved,
            contact.position,
            contact.force,
        ));
    }

    fn end(&mut self, contact: &Contact, phase: TouchPhase, events: &mut Vec<TouchEvent>) {
        if let Some(index) = self.active.iter().position(|(id, ..)| *id == contact.id) {
            let (_, touch, _) = self.active.remove(index);
            events.push(event(touch, phase, contact.position, contact.force));
        }
    }

    fn cancel_all(&mut self, events: &mut Vec<TouchEvent>) {
        for (_, touch, position) in self.active.drain(..) {
            events.push(event(touch, TouchPhase::Cancelled, position, None));
        }
    }
}

fn event(
    id: TouchId,
    phase: TouchPhase,
    position: Point<Pixels>,
    force: Option<f32>,
) -> TouchEvent {
    TouchEvent {
        id,
        phase,
        position,
        predicted_position: None,
        force,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px};

    fn contact(id: i32, x: f32, y: f32) -> Contact {
        Contact {
            id,
            position: point(px(x), px(y)),
            force: None,
        }
    }

    fn phases(events: &[TouchEvent]) -> Vec<(u64, TouchPhase)> {
        events.iter().map(|e| (e.id.0, e.phase)).collect()
    }

    #[test]
    fn a_tap_starts_and_ends_one_touch() {
        let mut touches = Touches::default();
        let down = touches.translate(Action::Down, &[], &[contact(0, 10., 20.)]);
        let up = touches.translate(Action::Up, &[], &[contact(0, 10., 20.)]);
        assert_eq!(phases(&down), [(0, TouchPhase::Started)]);
        assert_eq!(phases(&up), [(0, TouchPhase::Ended)]);
        assert_eq!(down[0].position, point(px(10.), px(20.)));
    }

    #[test]
    fn batched_samples_arrive_in_order_and_still_samples_are_dropped() {
        let mut touches = Touches::default();
        touches.translate(Action::Down, &[], &[contact(0, 0., 0.)]);
        let history = vec![vec![contact(0, 0., 5.)], vec![contact(0, 0., 5.)]];
        let moved = touches.translate(Action::Move, &history, &[contact(0, 0., 12.)]);
        let ys: Vec<_> = moved.iter().map(|e| e.position.y).collect();
        assert_eq!(ys, [px(5.), px(12.)], "the repeated sample is not a move");
        assert!(moved.iter().all(|e| e.phase == TouchPhase::Moved));
    }

    #[test]
    fn a_second_finger_gets_its_own_touch_and_lifts_on_its_own() {
        let mut touches = Touches::default();
        touches.translate(Action::Down, &[], &[contact(0, 0., 0.)]);
        let second = touches.translate(
            Action::PointerDown(1),
            &[],
            &[contact(0, 0., 0.), contact(1, 50., 50.)],
        );
        assert_eq!(phases(&second), [(1, TouchPhase::Started)]);
        let pinch = touches.translate(
            Action::Move,
            &[],
            &[contact(0, -5., 0.), contact(1, 55., 50.)],
        );
        assert_eq!(
            phases(&pinch),
            [(0, TouchPhase::Moved), (1, TouchPhase::Moved)]
        );
        let lift = touches.translate(
            Action::PointerUp(0),
            &[],
            &[contact(0, -5., 0.), contact(1, 55., 50.)],
        );
        assert_eq!(phases(&lift), [(0, TouchPhase::Ended)]);
        let up = touches.translate(Action::Up, &[], &[contact(1, 55., 50.)]);
        assert_eq!(phases(&up), [(1, TouchPhase::Ended)]);
    }

    #[test]
    fn reused_pointer_ids_never_reuse_a_touch_id() {
        let mut touches = Touches::default();
        touches.translate(Action::Down, &[], &[contact(0, 0., 0.)]);
        touches.translate(Action::Up, &[], &[contact(0, 0., 0.)]);
        let again = touches.translate(Action::Down, &[], &[contact(0, 1., 1.)]);
        assert_eq!(phases(&again), [(1, TouchPhase::Started)]);
    }

    #[test]
    fn cancel_unwinds_every_contact_and_a_lost_up_is_cancelled_by_the_next_down() {
        let mut touches = Touches::default();
        touches.translate(Action::Down, &[], &[contact(0, 0., 0.)]);
        touches.translate(
            Action::PointerDown(1),
            &[],
            &[contact(0, 0., 0.), contact(1, 9., 9.)],
        );
        let cancel = touches.translate(Action::Cancel, &[], &[]);
        assert_eq!(
            phases(&cancel),
            [(0, TouchPhase::Cancelled), (1, TouchPhase::Cancelled)]
        );

        touches.translate(Action::Down, &[], &[contact(0, 0., 0.)]);
        let next = touches.translate(Action::Down, &[], &[contact(3, 4., 4.)]);
        assert_eq!(
            phases(&next),
            [(2, TouchPhase::Cancelled), (3, TouchPhase::Started)]
        );
    }
}

/// Android touch streams through GPUI's own recognizers, as on a device.
#[cfg(test)]
mod gesture_tests {
    use super::*;
    use gpui::{
        Context, InputEvent, IntoElement, ParentElement, Render, ScrollHandle, Styled,
        TestAppContext, Window, div, point, prelude::*, px,
    };

    struct List {
        handle: ScrollHandle,
        taps: usize,
    }

    impl Render for List {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .id("tap")
                        .h(px(56.))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.taps += 1;
                            cx.notify();
                        }))
                        .child("Tap me"),
                )
                .child(
                    div()
                        .id("rows")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .track_scroll(&self.handle)
                        .children((0..200).map(|row| div().h(px(52.)).child(format!("Row {row}")))),
                )
        }
    }

    fn finger(x: f32, y: f32) -> [Contact; 1] {
        [Contact {
            id: 0,
            position: point(px(x), px(y)),
            force: Some(1.),
        }]
    }

    #[gpui::test]
    fn dragging_the_list_scrolls_it(cx: &mut TestAppContext) {
        let handle = ScrollHandle::new();
        let window = cx.add_window({
            let handle = handle.clone();
            |_, _| List { handle, taps: 0 }
        });
        cx.run_until_parked();

        let mut touches = Touches::default();
        let mut events = touches.translate(Action::Down, &[], &finger(100., 600.));
        for step in 1..=20 {
            let y = 600. - 15. * step as f32;
            events.extend(touches.translate(Action::Move, &[], &finger(100., y)));
        }
        events.extend(touches.translate(Action::Up, &[], &finger(100., 300.)));
        for event in events {
            window
                .update(cx, |_, window, cx| {
                    window.dispatch_event(event.to_platform_input(), cx);
                })
                .unwrap();
            cx.run_until_parked();
        }
        assert!(
            handle.offset().y < px(-250.),
            "a 300px drag up scrolls the list: {:?}",
            handle.offset()
        );
        window
            .update(cx, |list, _, _| {
                assert_eq!(list.taps, 0, "a drag is not a tap")
            })
            .unwrap();
    }
}
