//! Opt-in held input surfaces. Legacy sliders retain their rollback semantics.
use crate::visual_controls::{Manipulation, Surface};
use gpui::{
    AnyElement, App, DispatchPhase, Entity, FocusHandle, IntoElement, KeyDownEvent, KeyUpEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, TouchDragEvent, TouchPhase, Window,
    canvas, div, prelude::*, px,
};
use pi_visual::{InputEvent, input::Phase};
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

pub type SceneInput = Rc<dyn Fn(InputEvent, &mut Window, &mut App)>;
struct Contact {
    lease: Manipulation,
}
struct Held {
    focus: FocusHandle,
    contacts: HashMap<u64, Contact>,
    keys: HashSet<String>,
    key_lease: Option<Manipulation>,
    pending: HashMap<u64, InputEvent>,
    queued: bool,
    generation: u64,
    paths: crate::visual_controls::SceneCache,
}
impl Held {
    fn active(&self, id: u64) -> bool {
        self.contacts.get(&id).is_some_and(|c| c.lease.current())
    }
    fn cancel(&mut self) {
        self.contacts.clear();
        self.keys.clear();
        self.key_lease = None;
        self.pending.clear();
        self.queued = false;
        self.generation = self.generation.wrapping_add(1);
    }
}
#[allow(clippy::too_many_arguments)] // Typed transition plus native window/app contexts.
fn send_pointer(
    state: &Entity<Held>,
    id: u64,
    phase: Phase,
    position: [f64; 2],
    input: &SceneInput,
    barrier: &crate::visual_controls::ProjectionBarrier,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let event = InputEvent::Pointer {
        id,
        phase,
        x: position[0],
        y: position[1],
    };
    let accepted = state.update(cx, |state, _| {
        if phase == Phase::Down {
            // Lifecycle may invalidate leases without retaining this frame's entity.
            // Retire stale IDs and their queued samples before accepting a new start.
            if state.contacts.values().any(|c| !c.lease.current()) {
                state.contacts.retain(|_, c| c.lease.current());
                state.pending.clear();
                state.queued = false;
                state.generation = state.generation.wrapping_add(1);
            }
            if state.contacts.len() >= 10 || state.contacts.contains_key(&id) {
                return false;
            }
            state.contacts.insert(
                id,
                Contact {
                    lease: barrier.acquire(),
                },
            );
        } else if !state.active(id) {
            state.contacts.remove(&id);
            state.pending.remove(&id);
            return false;
        }
        true
    });
    if !accepted {
        return false;
    }
    if phase == Phase::Down {
        window.dismiss_virtual_keyboard();
        window.focus(&state.read(cx).focus.clone(), cx);
    }
    if phase == Phase::Move {
        let (queue, generation) = state.update(cx, |s, _| {
            s.pending.insert(id, event);
            let queue = !s.queued;
            s.queued = true;
            (queue, s.generation)
        });
        if queue {
            let weak = state.downgrade();
            let input = input.clone();
            window.on_next_frame(move |window, cx| {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                let pending = state.update(cx, |s, _| {
                    if s.generation != generation {
                        return Vec::new();
                    }
                    s.queued = false;
                    std::mem::take(&mut s.pending)
                        .into_iter()
                        .filter_map(|(id, event)| s.active(id).then_some((id, event)))
                        .collect::<Vec<_>>()
                });
                // Stable contact order rather than hash iteration order.
                let mut pending = pending;
                pending.sort_by_key(|(id, _)| *id);
                for (_, event) in pending {
                    input(event, window, cx);
                }
            });
        }
    } else {
        // Deliver a release even without movement; never deduplicate its phase.
        let lease = if matches!(phase, Phase::Up | Phase::Cancel) {
            state.update(cx, |s, _| {
                s.pending.remove(&id);
                s.contacts.remove(&id)
            })
        } else {
            None
        };
        input(event, window, cx);
        drop(lease);
    }
    true
}

pub fn render(
    surface: Surface,
    input: SceneInput,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let cancel = input.clone();
    let blur = input.clone();
    let held = window.use_keyed_state(surface.id.clone(), cx, move |window, cx| {
        let focus = cx.focus_handle().tab_stop(true);
        cx.on_blur(&focus, window, move |s: &mut Held, window, cx| {
            if !s.keys.is_empty() {
                s.keys.clear();
                let lease = s.key_lease.take();
                blur(InputEvent::Blur, window, cx);
                drop(lease);
            }
        })
        .detach();
        cx.on_release_in(window, move |s: &mut Held, window, cx| {
            s.paths.painted.set(false);
            let active = s.contacts.values().any(|c| c.lease.current())
                || s.key_lease.as_ref().is_some_and(Manipulation::current);
            // Keep projection leased through cancellation delivery.
            if active {
                cancel(InputEvent::Cancel, window, cx);
            }
            s.cancel();
        })
        .detach();
        Held {
            focus,
            contacts: HashMap::new(),
            keys: HashSet::new(),
            key_lease: None,
            pending: HashMap::new(),
            queued: false,
            generation: 0,
            paths: Default::default(),
        }
    });
    let focus = held.read(cx).focus.clone();
    let key_state = held.clone();
    let key_input = input.clone();
    let key_barrier = surface.barrier.clone();
    let up_state = held.clone();
    let up_input = input.clone();
    let a11y_state = held.clone();
    let a11y_input = input.clone();
    let a11y_barrier = surface.barrier.clone();
    div()
        .id(surface.id.clone())
        .debug_selector(move || surface.id.to_string())
        .relative()
        .w_full()
        .min_w_0()
        .track_focus(&focus)
        .role(if surface.held_button {
            gpui::Role::Button
        } else {
            gpui::Role::Group
        })
        .aria_label(surface.label)
        .child(crate::hooks::probe(surface.probe))
        .when(surface.held_button, |view| {
            view.on_a11y_action(gpui::AccessibleAction::Click, move |_, window, cx| {
                const A11Y_CONTACT: u64 = 9_007_199_254_740_990;
                if !send_pointer(
                    &a11y_state,
                    A11Y_CONTACT,
                    Phase::Down,
                    [0., 0.],
                    &a11y_input,
                    &a11y_barrier,
                    window,
                    cx,
                ) {
                    return;
                }
                let state = a11y_state.downgrade();
                let input = a11y_input.clone();
                let barrier = a11y_barrier.clone();
                // A bounded pulse permits animation to observe the held control, without
                // requiring TalkBack to synthesize a physical touch-and-hold.
                window
                    .spawn(cx, async move |cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(150))
                            .await;
                        let _ = cx.update(|window, cx| {
                            if let Some(state) = state.upgrade() {
                                send_pointer(
                                    &state,
                                    A11Y_CONTACT,
                                    Phase::Up,
                                    [0., 0.],
                                    &input,
                                    &barrier,
                                    window,
                                    cx,
                                );
                            }
                        });
                    })
                    .detach();
            })
        })
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            let key = &event.keystroke.key;
            if key == "tab"
                || key == "escape"
                || event.keystroke.modifiers.control
                || event.keystroke.modifiers.platform
            {
                return;
            }
            let accepted = key_state.update(cx, |s, _| {
                if s.keys.len() >= 32 && !s.keys.contains(key) {
                    return false;
                }
                if s.key_lease.as_ref().is_none_or(|l| !l.current()) {
                    s.keys.clear();
                    s.key_lease = Some(key_barrier.acquire());
                }
                s.keys.insert(key.clone());
                true
            });
            if accepted {
                key_input(
                    InputEvent::Key {
                        key: key.clone(),
                        down: true,
                        repeat: event.is_held,
                    },
                    window,
                    cx,
                );
                cx.stop_propagation();
            }
        })
        .on_key_up(move |event: &KeyUpEvent, window, cx| {
            let (accepted, lease) = up_state.update(cx, |s, _| {
                let accepted = s.keys.remove(&event.keystroke.key)
                    && s.key_lease.as_ref().is_some_and(Manipulation::current);
                let lease = if s.keys.is_empty() {
                    s.key_lease.take()
                } else {
                    None
                };
                (accepted, lease)
            });
            if accepted {
                up_input(
                    InputEvent::Key {
                        key: event.keystroke.key.clone(),
                        down: false,
                        repeat: false,
                    },
                    window,
                    cx,
                );
                cx.stop_propagation();
            }
            drop(lease);
        })
        .child(
            canvas(
                |bounds, window, _| {
                    (
                        bounds,
                        window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
                    )
                },
                move |_, (bounds, hitbox), window, cx| {
                    held.update(cx, |s, cx| {
                        let visible = bounds.intersects(&window.content_mask().bounds);
                        s.paths.painted.set(visible);
                        // Focus may enter and leave between two paints, so GPUI's
                        // frame-to-frame blur observer need not see the transition.
                        if !s.focus.is_focused(window) && !s.keys.is_empty() {
                            s.keys.clear();
                            let lease = s.key_lease.take();
                            input(InputEvent::Blur, window, cx);
                            drop(lease);
                        }
                        (surface.painter)(bounds, &mut s.paths, window, cx);
                        if !visible && (!s.contacts.is_empty() || !s.keys.is_empty()) {
                            input(InputEvent::Cancel, window, cx);
                            s.cancel();
                        }
                    });
                    let state = held.clone();
                    let callback = input.clone();
                    let coords = surface.coordinates.clone();
                    let barrier = surface.barrier.clone();
                    let touch_hitbox = hitbox.clone();
                    window.on_mouse_event(move |_event: &TouchDragEvent, phase, window, cx| {
                        let Some(event) = crate::platform::raw_touch::current() else {
                            return;
                        };
                        if phase != DispatchPhase::Bubble {
                            return;
                        }
                        if event.phase == TouchPhase::Started
                            && !touch_hitbox.is_hovered_at(event.position, window)
                        {
                            return;
                        }
                        let phase = match event.phase {
                            TouchPhase::Started => Phase::Down,
                            TouchPhase::Moved => Phase::Move,
                            TouchPhase::Ended => Phase::Up,
                            TouchPhase::Cancelled => Phase::Cancel,
                        };
                        if send_pointer(
                            &state,
                            event.id.0 + 1,
                            phase,
                            coords(event.position, bounds),
                            &callback,
                            &barrier,
                            window,
                            cx,
                        ) {
                            crate::platform::raw_touch::claim();
                            window.prevent_default();
                            cx.stop_propagation();
                        }
                    });
                    let state = held.clone();
                    let callback = input.clone();
                    let coords = surface.coordinates.clone();
                    let barrier = surface.barrier.clone();
                    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble
                            && event.button == MouseButton::Left
                            && hitbox.is_hovered(window)
                            && send_pointer(
                                &state,
                                0,
                                Phase::Down,
                                coords(event.position, bounds),
                                &callback,
                                &barrier,
                                window,
                                cx,
                            )
                        {
                            window.prevent_default();
                            cx.stop_propagation();
                        }
                    });
                    let state = held.clone();
                    let callback = input.clone();
                    let coords = surface.coordinates.clone();
                    let barrier = surface.barrier.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture
                            && send_pointer(
                                &state,
                                0,
                                if event.pressed_button == Some(MouseButton::Left) {
                                    Phase::Move
                                } else {
                                    Phase::Cancel
                                },
                                coords(event.position, bounds),
                                &callback,
                                &barrier,
                                window,
                                cx,
                            )
                        {
                            cx.stop_propagation();
                        }
                    });
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture
                            && event.button == MouseButton::Left
                            && send_pointer(
                                &held,
                                0,
                                Phase::Up,
                                (surface.coordinates)(event.position, bounds),
                                &input,
                                &surface.barrier,
                                window,
                                cx,
                            )
                        {
                            cx.stop_propagation();
                        }
                    });
                },
            )
            .w_full()
            .h(px(surface.height)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visual_controls::ProjectionBarrier;
    use gpui::{Context, Render, size};
    #[cfg(feature = "android")]
    use gpui::{TouchEvent, TouchId, point};
    use std::cell::RefCell;
    type Events = Rc<RefCell<Vec<InputEvent>>>;
    struct Fixture {
        events: Events,
        barrier: ProjectionBarrier,
        visible: bool,
    }
    impl Render for Fixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            if !self.visible {
                return div().into_any_element();
            }
            let events = self.events.clone();
            Surface {
                id: "held-test".into(),
                label: "Input".into(),
                probe: "held-test".into(),
                height: 120.,
                held_button: false,
                painter: Rc::new(|_, _, _, _| {}),
                coordinates: Rc::new(|p, _| [p.x.into(), p.y.into()]),
                input: None,
                slider: None,
                barrier: self.barrier.clone(),
                events: Some(Rc::new(move |e, _, _| events.borrow_mut().push(e))),
            }
            .into_any_element()
        }
    }
    #[cfg(feature = "android")]
    #[gpui::test]
    fn raw_contacts_are_independent_and_terminal_phases_bypass_coalescing(
        cx: &mut gpui::TestAppContext,
    ) {
        let events = Rc::new(RefCell::new(Vec::new()));
        let barrier = ProjectionBarrier::default();
        let (_, cx) = cx.add_window_view(|_, _| Fixture {
            events: events.clone(),
            barrier: barrier.clone(),
            visible: true,
        });
        cx.simulate_resize(size(px(320.), px(200.)));
        cx.run_until_parked();
        let bounds = cx.debug_bounds("held-test").unwrap();
        let mut router = crate::platform::raw_touch::Router::default();
        for (id, phase, x) in [
            (1, TouchPhase::Started, 10.),
            (2, TouchPhase::Started, 20.),
            (1, TouchPhase::Moved, 30.),
            (1, TouchPhase::Moved, 40.),
            (2, TouchPhase::Ended, 20.),
        ] {
            cx.update(|window, cx| {
                assert!(router.dispatch(
                    TouchEvent {
                        id: TouchId(id),
                        phase,
                        position: point(bounds.left() + px(x), bounds.center().y),
                        ..Default::default()
                    },
                    |event| Some(window.dispatch_event(event, cx))
                ))
            });
        }
        assert_eq!(events.borrow().len(), 3); // 2 down, stationary up
        cx.update(|window, cx| window.simulate_next_frame(cx));
        assert!(
            matches!(events.borrow().last(),Some(InputEvent::Pointer{id:2,phase:Phase::Move,x,..}) if (*x-40.).abs()<0.01)
        );
        cx.update(|window, cx| {
            assert!(router.dispatch(
                TouchEvent {
                    id: TouchId(1),
                    phase: TouchPhase::Cancelled,
                    position: bounds.center(),
                    ..Default::default()
                },
                |event| Some(window.dispatch_event(event, cx))
            ))
        });
        assert!(!barrier.active());
        assert!(!router.active());
    }
    #[gpui::test]
    fn stale_contacts_and_missing_mouse_up_cannot_leave_a_hold(cx: &mut gpui::TestAppContext) {
        let events = Rc::new(RefCell::new(Vec::new()));
        let barrier = ProjectionBarrier::default();
        let (_, cx) = cx.add_window_view(|_, _| Fixture {
            events: events.clone(),
            barrier: barrier.clone(),
            visible: true,
        });
        cx.simulate_resize(size(px(320.), px(200.)));
        cx.run_until_parked();
        let p = cx.debug_bounds("held-test").unwrap().center();
        cx.simulate_mouse_down(p, MouseButton::Left, gpui::Modifiers::none());
        assert!(barrier.active());
        barrier.invalidate();
        cx.simulate_mouse_down(p, MouseButton::Left, gpui::Modifiers::none());
        assert!(barrier.active());
        assert_eq!(
            events
                .borrow()
                .iter()
                .filter(|e| matches!(
                    e,
                    InputEvent::Pointer {
                        phase: Phase::Down,
                        ..
                    }
                ))
                .count(),
            2
        );
        cx.simulate_mouse_move(p, None, gpui::Modifiers::none());
        assert!(!barrier.active());
        assert!(matches!(
            events.borrow().last(),
            Some(InputEvent::Pointer {
                phase: Phase::Cancel,
                ..
            })
        ));
    }
    #[gpui::test]
    fn focus_and_unmount_clear_keyboard_and_contacts(cx: &mut gpui::TestAppContext) {
        let events = Rc::new(RefCell::new(Vec::new()));
        let barrier = ProjectionBarrier::default();
        let (app, cx) = cx.add_window_view(|_, _| Fixture {
            events: events.clone(),
            barrier: barrier.clone(),
            visible: true,
        });
        cx.simulate_resize(size(px(320.), px(200.)));
        cx.run_until_parked();
        let p = cx.debug_bounds("held-test").unwrap().center();
        cx.simulate_mouse_down(p, MouseButton::Left, gpui::Modifiers::none());
        cx.update(|window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::KeyDown(KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("space").unwrap(),
                    is_held: false,
                    prefer_character_input: false,
                }),
                cx,
            );
        });
        assert!(
            events
                .borrow()
                .iter()
                .any(|e| matches!(e, InputEvent::Key { down: true, .. }))
        );
        cx.update(|window, cx| window.blur(cx));
        cx.run_until_parked();
        assert!(
            events
                .borrow()
                .iter()
                .any(|e| matches!(e, InputEvent::Blur))
        );
        app.update(cx, |app, cx| {
            app.visible = false;
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert!(!barrier.active());
        assert!(
            events
                .borrow()
                .iter()
                .any(|e| matches!(e, InputEvent::Cancel))
        );
    }
}
