//! Native drawing and direct manipulation for portable visual apps.
pub use crate::visual_painter::{SceneCache, paint_scene};
use gpui::{
    AccessibleAction, App, Bounds, DispatchPhase, Entity, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, RenderOnce, Role, SharedString, TouchDragEvent,
    TouchPhase, Window, canvas, div, point, prelude::*, px,
};
use std::{cell::Cell, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    Start,
    Move,
    End,
    Cancel,
}
pub type Input = Rc<dyn Fn([f64; 2], Gesture, &mut Window, &mut App)>;
pub type Painter = Rc<dyn Fn(Bounds<Pixels>, &mut SceneCache, &mut Window, &mut App)>;
pub type Coordinates = Rc<dyn Fn(Point<Pixels>, Bounds<Pixels>) -> [f64; 2]>;

/// Shared only by one mounted runtime. A lease defers expensive model projection,
/// not input/script/drawing work. Epoch invalidation makes stale releases harmless.
#[derive(Clone, Default)]
pub struct ProjectionBarrier(Rc<Cell<(u64, usize)>>);
impl ProjectionBarrier {
    pub fn active(&self) -> bool {
        self.0.get().1 != 0
    }
    pub fn invalidate(&self) {
        self.0.set((self.0.get().0.wrapping_add(1), 0));
    }
    pub fn acquire(&self) -> Manipulation {
        let (epoch, active) = self.0.get();
        self.0.set((epoch, active + 1));
        Manipulation {
            barrier: self.clone(),
            epoch,
        }
    }
}
pub struct Manipulation {
    barrier: ProjectionBarrier,
    epoch: u64,
}
impl Manipulation {
    pub fn current(&self) -> bool {
        self.barrier.0.get().0 == self.epoch
    }
}
impl Drop for Manipulation {
    fn drop(&mut self) {
        let (epoch, active) = self.barrier.0.get();
        if epoch == self.epoch {
            self.barrier.0.set((epoch, active.saturating_sub(1)));
        }
    }
}

#[derive(Default)]
struct Drag {
    active: bool,
    manipulation: Option<Manipulation>,
    paths: SceneCache,
    generation: u64,
    queued: bool,
    pending: Option<([f64; 2], Input, Option<std::time::Instant>)>,
}

impl Drag {
    fn is_active(&self) -> bool {
        self.active
            && self
                .manipulation
                .as_ref()
                .is_some_and(Manipulation::current)
    }
}

#[derive(IntoElement)]
pub struct Surface {
    pub id: SharedString,
    pub label: String,
    pub probe: String,
    pub height: f32,
    pub painter: Painter,
    pub coordinates: Coordinates,
    pub input: Option<Input>,
    pub events: Option<crate::visual_input::SceneInput>,
    pub held_button: bool,
    pub barrier: ProjectionBarrier,
    pub slider: Option<(f64, f64, f64, f64)>,
}

// One latest input per display frame, not a fixed 30 Hz gate or event backlog.
fn queue_move(
    drag: &Entity<Drag>,
    position: [f64; 2],
    input: Input,
    window: &mut Window,
    cx: &mut App,
) {
    let _trace = crate::platform::trace::scope(c"pi.visual.move.queue");
    let queue = drag.update(cx, |state, cx| {
        let receipt = crate::visual_profile::receive(cx, state.pending.is_some());
        state.pending = Some((position, input, receipt));
        if state.queued {
            let _trace = crate::platform::trace::scope(c"pi.visual.move.replace");
            false
        } else {
            state.queued = true;
            true
        }
    });
    if !queue {
        return;
    }
    let generation = drag.read(cx).generation;
    let weak = drag.downgrade();
    window.on_next_frame(move |window, cx| {
        let _trace = crate::platform::trace::scope(c"pi.visual.move.frame");
        let Some(drag) = weak.upgrade() else {
            let _trace = crate::platform::trace::scope(c"pi.visual.move.discard");
            return;
        };
        let pending = drag.update(cx, |state, _| {
            if !state.is_active() || state.generation != generation {
                let _trace = crate::platform::trace::scope(c"pi.visual.move.discard");
                return None;
            }
            state.queued = false;
            state.pending.take()
        });
        if let Some((position, input, receipt)) = pending {
            let _trace = crate::platform::trace::scope(c"pi.visual.move.deliver");
            crate::visual_profile::deliver(cx, receipt);
            input(position, Gesture::Move, window, cx);
        }
    });
}

fn begin(state: &mut Drag, barrier: &ProjectionBarrier) {
    state.manipulation = Some(barrier.acquire());
    state.active = true;
    state.generation = state.generation.wrapping_add(1);
    state.pending = None;
    state.queued = false;
}
fn finish(state: &mut Drag) -> Option<Manipulation> {
    state.active = false;
    state.generation = state.generation.wrapping_add(1);
    state.pending = None;
    state.queued = false;
    state.manipulation.take()
}

impl RenderOnce for Surface {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if let Some(input) = self.events.take() {
            return crate::visual_input::render(self, input, window, cx);
        }
        let cancel = self.input.clone();
        let drag = window.use_keyed_state(self.id.clone(), cx, |window, cx| {
            cx.on_release_in(window, move |state: &mut Drag, window, cx| {
                state.paths.painted.set(false);
                let active = state.is_active();
                // Hold through rollback even if the runtime callback no longer exists.
                let _manipulation = finish(state);
                if active && let Some(cancel) = cancel {
                    cancel([0.0, 0.0], Gesture::Cancel, window, cx);
                }
            })
            .detach();
            Drag::default()
        });
        let mut view = div()
            .id(self.id.clone())
            .debug_selector(move || self.id.to_string())
            .relative()
            .child(crate::hooks::probe(self.probe))
            .w_full()
            .min_w_0()
            .role(if self.slider.is_some() {
                Role::Slider
            } else {
                Role::Image
            })
            .aria_label(self.label);
        if let Some((value, min, max, step)) = self.slider {
            view = view
                .aria_numeric_value(value)
                .aria_min_numeric_value(min)
                .aria_max_numeric_value(max)
                .aria_numeric_value_step(step);
            if let Some(input) = self.input.clone() {
                let increment = input.clone();
                let increment_barrier = self.barrier.clone();
                let decrement_barrier = self.barrier.clone();
                view = view
                    .on_a11y_action(AccessibleAction::Increment, move |_, window, cx| {
                        let _manipulation = increment_barrier.acquire();
                        increment([(value + step).min(max), 0.0], Gesture::Start, window, cx);
                        increment([(value + step).min(max), 0.0], Gesture::End, window, cx);
                    })
                    .on_a11y_action(AccessibleAction::Decrement, move |_, window, cx| {
                        let _manipulation = decrement_barrier.acquire();
                        input([(value - step).max(min), 0.0], Gesture::Start, window, cx);
                        input([(value - step).max(min), 0.0], Gesture::End, window, cx);
                    });
            }
        }
        view.child(
            canvas(
                |bounds, window, _| {
                    (
                        bounds,
                        window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
                    )
                },
                move |_, (bounds, hitbox), window, cx| {
                    drag.update(cx, |state, cx| {
                        state
                            .paths
                            .painted
                            .set(bounds.intersects(&window.content_mask().bounds));
                        (self.painter)(bounds, &mut state.paths, window, cx);
                        if !state.paths.painted.get() && state.is_active() {
                            let _lease = finish(state);
                            if let Some(input) = &self.input {
                                input([0., 0.], Gesture::Cancel, window, cx);
                            }
                        }
                    });
                    let Some(input) = self.input else {
                        return;
                    };
                    let touch_hitbox = hitbox.clone();
                    let down = drag.clone();
                    let down_input = input.clone();
                    let down_coords = self.coordinates.clone();
                    let down_barrier = self.barrier.clone();
                    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                        if phase != DispatchPhase::Capture
                            || event.button != MouseButton::Left
                            || !hitbox.is_hovered(window)
                        {
                            return;
                        }
                        if !down.read(cx).is_active() {
                            down.update(cx, |state, _| {
                                begin(state, &down_barrier);
                            });
                            down_input(
                                down_coords(event.position, bounds),
                                Gesture::Start,
                                window,
                                cx,
                            );
                        }
                        window.prevent_default();
                        cx.stop_propagation();
                    });
                    let moving = drag.clone();
                    let move_input = input.clone();
                    let move_coords = self.coordinates.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase != DispatchPhase::Capture || !moving.read(cx).is_active() {
                            return;
                        }
                        queue_move(
                            &moving,
                            move_coords(event.position, bounds),
                            move_input.clone(),
                            window,
                            cx,
                        );
                        cx.stop_propagation();
                    });
                    let release = drag.clone();
                    let end_input = input.clone();
                    let end_coords = self.coordinates.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                        if phase != DispatchPhase::Capture
                            || event.button != MouseButton::Left
                            || !release.read(cx).is_active()
                        {
                            return;
                        }
                        let _manipulation = release.update(cx, |state, _| finish(state));
                        end_input(end_coords(event.position, bounds), Gesture::End, window, cx);
                        cx.stop_propagation();
                    });
                    window.on_mouse_event(move |event: &TouchDragEvent, phase, window, cx| {
                        if crate::platform::raw_touch::current().is_some() {
                            return;
                        }
                        if phase != DispatchPhase::Capture {
                            return;
                        }
                        if event.phase == TouchPhase::Started {
                            if !touch_hitbox.is_hovered(window) {
                                return;
                            }
                            if !drag.read(cx).is_active() {
                                drag.update(cx, |state, _| {
                                    begin(state, &self.barrier);
                                });
                                input(
                                    (self.coordinates)(event.position, bounds),
                                    Gesture::Start,
                                    window,
                                    cx,
                                );
                            }
                            window.prevent_default();
                            cx.stop_propagation();
                            return;
                        }
                        if !drag.read(cx).is_active() {
                            return;
                        }
                        let gesture = match event.phase {
                            TouchPhase::Ended => Gesture::End,
                            TouchPhase::Cancelled => Gesture::Cancel,
                            _ => Gesture::Move,
                        };
                        let position = (self.coordinates)(event.position, bounds);
                        if gesture == Gesture::Move {
                            queue_move(&drag, position, input.clone(), window, cx);
                        } else {
                            let _manipulation = drag.update(cx, |state, _| finish(state));
                            input(position, gesture, window, cx);
                        }
                        cx.stop_propagation();
                    });
                },
            )
            .w_full()
            .h(px(self.height)),
        )
        .into_any_element()
    }
}

pub fn transform(bounds: Bounds<Pixels>, view_box: [f32; 4]) -> (Point<Pixels>, f32) {
    let scale = (f32::from(bounds.size.width) / view_box[2])
        .min(f32::from(bounds.size.height) / view_box[3]);
    let origin =
        bounds.center() - point(px(view_box[2] * scale * 0.5), px(view_box[3] * scale * 0.5));
    (
        origin - point(px(view_box[0] * scale), px(view_box[1] * scale)),
        scale,
    )
}

pub fn pointer_at(position: Point<Pixels>, bounds: Bounds<Pixels>, view_box: [f32; 4]) -> [f64; 2] {
    let (origin, scale) = transform(bounds, view_box);
    if scale <= 0.0 {
        return [f64::from(view_box[0]), f64::from(view_box[1])];
    }
    [
        f64::from(
            (f32::from(position.x - origin.x) / scale)
                .clamp(view_box[0], view_box[0] + view_box[2]),
        ),
        f64::from(
            (f32::from(position.y - origin.y) / scale)
                .clamp(view_box[1], view_box[1] + view_box[3]),
        ),
    ]
}

pub fn slider_at(position: Point<Pixels>, bounds: Bounds<Pixels>, min: f64, max: f64) -> [f64; 2] {
    let width = f32::from(bounds.size.width - px(24.)).max(1.);
    let fraction = (f32::from(position.x - bounds.left() - px(12.)) / width).clamp(0., 1.);
    [min + f64::from(fraction) * (max - min), 0.0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, InputEvent, Render, size};
    use std::cell::RefCell;

    type Events = Rc<RefCell<Vec<([f64; 2], Gesture)>>>;
    struct Fixture {
        events: Events,
        visible: bool,
        barrier: ProjectionBarrier,
    }
    impl Render for Fixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            if !self.visible {
                return div().w_full();
            }
            let events = self.events.clone();
            let barrier = self.barrier.clone();
            div().w_full().child(Surface {
                id: "test-visual-slider".into(),
                label: "Speed".into(),
                probe: "test-visual-slider".into(),
                height: 48.,
                barrier: self.barrier.clone(),
                painter: Rc::new(|_, _, _, _| {}),
                coordinates: Rc::new(|p, b| slider_at(p, b, 0., 100.)),
                input: Some(Rc::new(move |p, gesture, _, _| {
                    // End/Cancel must still be protected while committing/rolling back.
                    assert!(barrier.active(), "lease released before {gesture:?}");
                    events.borrow_mut().push((p, gesture))
                })),
                events: None,
                held_button: false,
                slider: Some((50., 0., 100., 1.)),
            })
        }
    }

    #[test]
    fn manipulation_leases_are_scoped_and_stale_release_cannot_clear_new_input() {
        let barrier = ProjectionBarrier::default();
        assert!(!barrier.active());
        let first = barrier.acquire();
        let second = barrier.acquire();
        drop(first);
        assert!(barrier.active());
        barrier.invalidate();
        assert!(!barrier.active());
        let newer = barrier.acquire();
        drop(second);
        assert!(barrier.active());
        drop(newer);
        assert!(!barrier.active());
    }

    #[gpui::test]
    fn invalidation_suppresses_queued_move_and_stale_unmount_cancel(cx: &mut gpui::TestAppContext) {
        let barrier = ProjectionBarrier::default();
        let events = Rc::new(RefCell::new(Vec::new()));
        let (app, cx) = cx.add_window_view(|_, _| Fixture {
            events: events.clone(),
            visible: true,
            barrier: barrier.clone(),
        });
        cx.simulate_resize(size(px(320.), px(160.)));
        cx.run_until_parked();
        let bounds = cx.debug_bounds("test-visual-slider").unwrap();
        cx.simulate_mouse_down(bounds.center(), MouseButton::Left, gpui::Modifiers::none());
        cx.simulate_mouse_move(
            bounds.center(),
            Some(MouseButton::Left),
            gpui::Modifiers::none(),
        );
        assert!(barrier.active());
        barrier.invalidate();
        let newer = barrier.acquire();
        cx.update(|window, cx| window.simulate_next_frame(cx));
        assert_eq!(events.borrow().len(), 1);
        app.update(cx, |app, cx| {
            app.visible = false;
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(events.borrow().len(), 1); // Already rolled back: no stale Cancel.
        assert!(barrier.active());
        drop(newer);
        assert!(!barrier.active());
    }

    #[gpui::test]
    fn native_mouse_and_touch_end_or_cancel_their_gesture(cx: &mut gpui::TestAppContext) {
        let events = Rc::new(RefCell::new(Vec::new()));
        let (_, cx) = cx.add_window_view(|_, _| Fixture {
            events: events.clone(),
            visible: true,
            barrier: ProjectionBarrier::default(),
        });
        cx.simulate_resize(size(px(320.), px(160.)));
        cx.run_until_parked();
        let bounds = cx.debug_bounds("test-visual-slider").unwrap();
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(events.borrow()[0].1, Gesture::Start);
        assert_eq!(events.borrow().last().unwrap().1, Gesture::End);
        assert!((events.borrow().last().unwrap().0[0] - 50.).abs() < 0.01);
        events.borrow_mut().clear();
        for phase in [
            TouchPhase::Started,
            TouchPhase::Moved,
            TouchPhase::Cancelled,
        ] {
            cx.update(|window, cx| {
                window.dispatch_event(
                    gpui::TouchEvent {
                        id: gpui::TouchId(1),
                        phase,
                        position: bounds.center(),
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                )
            });
            cx.run_until_parked();
        }
        assert_eq!(events.borrow()[0].1, Gesture::Start);
        assert_eq!(events.borrow().last().unwrap().1, Gesture::Cancel);
        assert!(!events.borrow().iter().any(|(_, g)| *g == Gesture::End));
    }

    #[gpui::test]
    fn fast_moves_coalesce_to_latest_display_frame_and_release_cancels_pending(
        cx: &mut gpui::TestAppContext,
    ) {
        let events = Rc::new(RefCell::new(Vec::new()));
        let (_, cx) = cx.add_window_view(|_, _| Fixture {
            events: events.clone(),
            visible: true,
            barrier: ProjectionBarrier::default(),
        });
        cx.simulate_resize(size(px(320.), px(160.)));
        cx.run_until_parked();
        let bounds = cx.debug_bounds("test-visual-slider").unwrap();
        cx.simulate_mouse_down(bounds.center(), MouseButton::Left, gpui::Modifiers::none());
        for fraction in [0.2, 0.5, 0.9] {
            cx.simulate_mouse_move(
                point(
                    bounds.left() + bounds.size.width * fraction,
                    bounds.center().y,
                ),
                Some(MouseButton::Left),
                gpui::Modifiers::none(),
            );
        }
        assert_eq!(events.borrow().len(), 1); // Start only; no dropped-last/move backlog.
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
        });
        assert_eq!(events.borrow().len(), 2);
        assert_eq!(events.borrow()[1].1, Gesture::Move);
        assert!(events.borrow()[1].0[0] > 90.0);
        cx.simulate_mouse_move(
            bounds.center(),
            Some(MouseButton::Left),
            gpui::Modifiers::none(),
        );
        cx.simulate_mouse_up(bounds.center(), MouseButton::Left, gpui::Modifiers::none());
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
        });
        assert_eq!(events.borrow().len(), 3);
        assert_eq!(events.borrow()[2].1, Gesture::End);
    }

    #[gpui::test]
    fn unmounted_active_surface_cancels_without_replaying_moves(cx: &mut gpui::TestAppContext) {
        let events = Rc::new(RefCell::new(Vec::new()));
        let (app, cx) = cx.add_window_view(|_, _| Fixture {
            events: events.clone(),
            visible: true,
            barrier: ProjectionBarrier::default(),
        });
        cx.simulate_resize(size(px(320.), px(160.)));
        cx.run_until_parked();
        let bounds = cx.debug_bounds("test-visual-slider").unwrap();
        cx.simulate_mouse_down(bounds.center(), MouseButton::Left, gpui::Modifiers::none());
        app.update(cx, |app, cx| {
            app.visible = false;
            cx.notify();
        });
        cx.run_until_parked();
        // GPUI retains the previous frame's event/state buffers until another
        // frame; exercise actual release rather than only logical removal.
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(events.borrow().last().unwrap().1, Gesture::Cancel);
    }

    #[test]
    fn coordinates_preserve_aspect_ratio_and_clamp_outside_drags() {
        let bounds = Bounds::new(point(px(10.), px(20.)), size(px(300.), px(200.)));
        assert_eq!(
            pointer_at(bounds.center(), bounds, [-100., -100., 200., 200.]),
            [0., 0.]
        );
        assert_eq!(
            pointer_at(
                point(px(-500.), px(900.)),
                bounds,
                [-100., -100., 200., 200.]
            ),
            [-100., 100.]
        );
        assert_eq!(slider_at(bounds.center(), bounds, 10., 90.), [50., 0.]);
    }
}
