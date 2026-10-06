//! One scroll viewport with a proportional, draggable scrollbar. Content and
//! overlay are siblings: the thumb never scrolls away with a long document.

use crate::theme::theme;
use gpui::{
    AnyElement, App, Bounds, DispatchPhase, Div, ElementId, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, RenderOnce, ScrollHandle, Stateful,
    StyleRefinement, TouchDragEvent, TouchPhase, Window, canvas, div, fill, point, prelude::*, px,
    size,
};
use std::{
    cell::RefCell,
    collections::VecDeque,
    time::{Duration, Instant},
};

const THUMB_HOLD: Duration = Duration::from_millis(500);
const THUMB_FADE: Duration = Duration::from_millis(260);
/// Content that moved less than this lately looks still: a fling's slow
/// tail, or one stopped at the end of its content.
const SETTLED_WITHIN: Duration = Duration::from_millis(100);
const SETTLED_DISTANCE: f32 = 15.;

/// How far scroll content has moved lately, across every scroll area.
#[derive(Default)]
struct Motion(VecDeque<(Instant, f32)>);

impl Motion {
    fn moved(&mut self, distance: f32, now: Instant) {
        self.0.push_back((now, distance));
        while self.0.len() > 64 {
            self.0.pop_front();
        }
    }

    fn settled(&self, now: Instant) -> bool {
        let recent: f32 = self
            .0
            .iter()
            .filter(|(at, _)| now.saturating_duration_since(*at) <= SETTLED_WITHIN)
            .map(|(_, distance)| distance)
            .sum();
        recent < SETTLED_DISTANCE
    }
}

thread_local! {
    static MOTION: RefCell<Motion> = RefCell::default();
}

/// Whether scrolling has visibly stopped, so a touch should tap rather than
/// catch a fling that only GPUI still thinks is running.
pub(crate) fn settled() -> bool {
    MOTION.with(|motion| motion.borrow().settled(Instant::now()))
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Geometry {
    pub track: Bounds<Pixels>,
    pub thumb: Bounds<Pixels>,
    max: Pixels,
}

impl Geometry {
    pub fn new(bounds: Bounds<Pixels>, max: Pixels, offset: Pixels) -> Option<Self> {
        let height = (bounds.size.height - px(8.)).max(px(0.));
        if max <= px(1.) || height <= px(8.) {
            return None;
        }
        let track = Bounds::new(
            point(bounds.right() - px(18.), bounds.top() + px(4.)),
            size(px(18.), height),
        );
        let length = (height * (bounds.size.height / (bounds.size.height + max)))
            .max(px(28.))
            .min(height);
        let y = track.top() + (height - length) * (-offset / max).clamp(0., 1.);
        let thumb = Bounds::new(point(bounds.right() - px(6.), y), size(px(3.), length));
        Some(Self { track, thumb, max })
    }

    fn offset(&self, pointer: Pixels, grab: Pixels) -> Pixels {
        let travel = self.track.size.height - self.thumb.size.height;
        if travel <= px(0.) {
            return px(0.);
        }
        -((pointer - self.track.top() - grab) / travel).clamp(0., 1.) * self.max
    }
}

#[derive(Default)]
struct ScrollbarState {
    grab: Option<Pixels>,
    last_offset: Option<Pixels>,
    last_motion: Option<Instant>,
}

impl ScrollbarState {
    fn observe(&mut self, offset: Pixels, now: Instant) {
        if let Some(previous) = self.last_offset.replace(offset)
            && previous != offset
        {
            self.last_motion = Some(now);
            let distance = f32::from((offset - previous).abs());
            MOTION.with(|motion| motion.borrow_mut().moved(distance, now));
        }
    }

    fn reveal(&mut self, now: Instant) {
        self.last_motion = Some(now);
    }

    fn opacity(&self, now: Instant) -> f32 {
        thumb_opacity(self.last_motion, self.grab.is_some(), now)
    }
}

/// Shared by full-screen viewports and long composer drafts so every vertical
/// scroll affordance has the same quiet hold-and-fade behavior.
pub(crate) fn thumb_opacity(last_motion: Option<Instant>, active: bool, now: Instant) -> f32 {
    if active {
        return 1.;
    }
    let Some(last_motion) = last_motion else {
        return 0.;
    };
    let elapsed = now.saturating_duration_since(last_motion);
    if elapsed <= THUMB_HOLD {
        1.
    } else if elapsed < THUMB_HOLD + THUMB_FADE {
        1. - (elapsed - THUMB_HOLD).as_secs_f32() / THUMB_FADE.as_secs_f32()
    } else {
        0.
    }
}

#[derive(IntoElement)]
pub(crate) struct ScrollArea {
    #[cfg(feature = "ui-test")]
    name: String,
    outer: Stateful<Div>,
    handle: ScrollHandle,
    children: Vec<AnyElement>,
}

pub(crate) fn vertical(id: impl Into<ElementId>, handle: &ScrollHandle) -> ScrollArea {
    let id = id.into();
    ScrollArea {
        #[cfg(feature = "ui-test")]
        name: format!("{id:?}"),
        outer: div().id(id).relative().min_h_0().flex().flex_col(),
        handle: handle.clone(),
        children: Vec::new(),
    }
}

impl Styled for ScrollArea {
    fn style(&mut self) -> &mut StyleRefinement {
        self.outer.style()
    }
}

impl ParentElement for ScrollArea {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for ScrollArea {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let measure = self.handle.clone();
        let handle = self.handle.clone();
        self.outer
            .child(
                div()
                    .id("viewport")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.handle)
                    .on_scroll_wheel(|_, window, _| window.refresh())
                    .children(self.children),
            )
            .child(
                canvas(
                    move |_, window, cx| {
                        let geometry = Geometry::new(
                            measure.bounds(),
                            measure.max_offset().y,
                            measure.offset().y,
                        );
                        #[cfg(feature = "ui-test")]
                        if cx.has_global::<crate::testing::State>() {
                            let state = cx.global_mut::<crate::testing::State>();
                            let bounds = measure.bounds();
                            state.bounds.insert(
                                format!("scroll:{}", self.name),
                                [
                                    bounds.left().into(),
                                    bounds.top().into(),
                                    bounds.size.width.into(),
                                    bounds.size.height.into(),
                                ],
                            );
                            if let Some(g) = geometry {
                                state.bounds.insert(
                                    format!("thumb:{}", self.name),
                                    [
                                        g.thumb.left().into(),
                                        g.thumb.top().into(),
                                        g.thumb.size.width.into(),
                                        g.thumb.size.height.into(),
                                    ],
                                );
                            }
                        }
                        let state = window.use_keyed_state("scrollbar-state", cx, |_, _| {
                            ScrollbarState::default()
                        });
                        let now = Instant::now();
                        state.update(cx, |state, _| state.observe(measure.offset().y, now));
                        let opacity = state.read(cx).opacity(now);
                        let hitbox = geometry.filter(|_| opacity > 0.).map(|geometry| {
                            window.insert_hitbox(geometry.track, gpui::HitboxBehavior::Normal)
                        });
                        (geometry, hitbox, state, opacity)
                    },
                    move |_, (geometry, hitbox, state, opacity), window, cx| {
                        let Some(geometry) = geometry else { return };
                        if opacity > 0. {
                            window.paint_quad(
                                fill(geometry.thumb, theme(cx).muted.opacity(0.65 * opacity))
                                    .corner_radii(px(2.)),
                            );
                            // Keep rendering through the hold and fade, then stop
                            // requesting frames completely while the thumb is hidden.
                            window.request_animation_frame();
                        }
                        let Some(hitbox) = hitbox else { return };
                        let touch_hitbox = hitbox.clone();
                        let down_state = state.clone();
                        let down_scroll = handle.clone();
                        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                            if phase != DispatchPhase::Capture
                                || event.button != MouseButton::Left
                                || !hitbox.is_hovered(window)
                            {
                                return;
                            }
                            let grab = if geometry.thumb.top() <= event.position.y
                                && event.position.y <= geometry.thumb.bottom()
                            {
                                event.position.y - geometry.thumb.top()
                            } else {
                                geometry.thumb.size.height / 2.
                            };
                            down_state.update(cx, |state, _| {
                                state.grab = Some(grab);
                                state.reveal(Instant::now());
                            });
                            down_scroll.set_offset(point(
                                down_scroll.offset().x,
                                geometry.offset(event.position.y, grab),
                            ));
                            cx.stop_propagation();
                            window.prevent_default();
                            window.refresh();
                        });
                        let moving_state = state.clone();
                        let moving_scroll = handle.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if phase != DispatchPhase::Capture {
                                return;
                            }
                            if let Some(grab) = moving_state.read(cx).grab {
                                moving_scroll.set_offset(point(
                                    moving_scroll.offset().x,
                                    geometry.offset(event.position.y, grab),
                                ));
                                cx.stop_propagation();
                                window.refresh();
                            }
                        });
                        let released_state = state.clone();
                        window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
                            if phase == DispatchPhase::Capture
                                && released_state.read(cx).grab.is_some()
                            {
                                released_state.update(cx, |state, _| {
                                    state.grab = None;
                                    state.reveal(Instant::now());
                                });
                                cx.stop_propagation();
                                window.refresh();
                            }
                        });
                        // A scrollbar owns the finger from touch-down. It is a direct
                        // manipulation, not a content pan or a reconstructed fling.
                        window.on_mouse_event(move |event: &TouchDragEvent, phase, window, cx| {
                            if phase != DispatchPhase::Capture {
                                return;
                            }
                            if event.phase == TouchPhase::Started {
                                if !touch_hitbox.is_hovered(window) {
                                    return;
                                }
                                let start = event.start_position.y;
                                let grab = if geometry.thumb.top() <= start
                                    && start <= geometry.thumb.bottom()
                                {
                                    start - geometry.thumb.top()
                                } else {
                                    geometry.thumb.size.height / 2.
                                };
                                state.update(cx, |state, _| {
                                    state.grab = Some(grab);
                                    state.reveal(Instant::now());
                                });
                                window.prevent_default();
                            }
                            if let Some(grab) = state.read(cx).grab {
                                handle.set_offset(point(
                                    handle.offset().x,
                                    geometry.offset(event.position.y, grab),
                                ));
                                if matches!(event.phase, TouchPhase::Ended | TouchPhase::Cancelled)
                                {
                                    state.update(cx, |state, _| {
                                        state.grab = None;
                                        state.reveal(Instant::now());
                                    });
                                }
                                cx.stop_propagation();
                                window.refresh();
                            }
                        });
                    },
                )
                .absolute()
                .inset_0(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thumb_is_proportional_clamped_and_only_exists_for_overflow() {
        let bounds = Bounds::new(gpui::Point::default(), size(px(320.), px(500.)));
        assert!(Geometry::new(bounds, px(0.), px(0.)).is_none());
        let top = Geometry::new(bounds, px(4500.), px(0.)).unwrap();
        assert_eq!(top.thumb.top(), top.track.top());
        assert!(top.thumb.size.height >= px(28.));
        assert_eq!(top.offset(top.track.top(), px(0.)), px(0.));
        assert_eq!(top.offset(top.track.bottom(), px(0.)), px(-4500.));
        let end = Geometry::new(bounds, px(4500.), px(-9999.)).unwrap();
        assert_eq!(end.thumb.bottom(), end.track.bottom());
    }

    #[test]
    fn scrolling_settles_once_it_barely_moves() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut motion = Motion::default();
        assert!(motion.settled(start), "nothing has moved");
        for frame in 0..6 {
            motion.moved(12., at(frame * 16));
        }
        assert!(!motion.settled(at(90)), "a fling still moving");
        for frame in 6..20 {
            motion.moved(0.5, at(frame * 16));
        }
        assert!(motion.settled(at(320)), "a slow tail looks still");
        assert!(motion.settled(at(2_000)), "a fling stopped at the end");
    }

    #[test]
    fn thumb_stays_hidden_until_motion_then_holds_and_fades() {
        let start = Instant::now();
        let mut state = ScrollbarState::default();
        state.observe(px(0.), start);
        assert_eq!(state.opacity(start), 0.);

        state.observe(px(-12.), start + Duration::from_millis(10));
        assert_eq!(state.opacity(start + Duration::from_millis(500)), 1.);
        let fading = state.opacity(start + Duration::from_millis(640));
        assert!(fading > 0. && fading < 1.);
        assert_eq!(state.opacity(start + Duration::from_millis(800)), 0.);

        state.grab = Some(px(2.));
        assert_eq!(state.opacity(start + Duration::from_secs(30)), 1.);
    }
}
