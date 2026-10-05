//! One scroll viewport with a proportional, draggable scrollbar. Content and
//! overlay are siblings: the thumb never scrolls away with a long document.

use crate::theme::theme;
use gpui::{
    AnyElement, App, Bounds, DispatchPhase, Div, ElementId, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, RenderOnce, ScrollHandle, Stateful,
    StyleRefinement, TouchDragEvent, TouchPhase, Window, canvas, div, fill, point, prelude::*, px,
    size,
};

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
struct Drag {
    grab: Option<Pixels>,
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
                        let drag =
                            window.use_keyed_state("scrollbar-drag", cx, |_, _| Drag::default());
                        let hitbox = geometry
                            .map(|g| window.insert_hitbox(g.track, gpui::HitboxBehavior::Normal));
                        (geometry, hitbox, drag)
                    },
                    move |_, (geometry, hitbox, drag), window, cx| {
                        let Some(geometry) = geometry else { return };
                        let hitbox = hitbox.unwrap();
                        let touch_hitbox = hitbox.clone();
                        window.paint_quad(
                            fill(geometry.thumb, theme(cx).muted.opacity(0.65))
                                .corner_radii(px(2.)),
                        );
                        let down_drag = drag.clone();
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
                            down_drag.update(cx, |drag, _| drag.grab = Some(grab));
                            down_scroll.set_offset(point(
                                down_scroll.offset().x,
                                geometry.offset(event.position.y, grab),
                            ));
                            cx.stop_propagation();
                            window.prevent_default();
                            window.refresh();
                        });
                        let moving_drag = drag.clone();
                        let moving_scroll = handle.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if phase != DispatchPhase::Capture {
                                return;
                            }
                            if let Some(grab) = moving_drag.read(cx).grab {
                                moving_scroll.set_offset(point(
                                    moving_scroll.offset().x,
                                    geometry.offset(event.position.y, grab),
                                ));
                                cx.stop_propagation();
                                window.refresh();
                            }
                        });
                        let released_drag = drag.clone();
                        window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
                            if phase == DispatchPhase::Capture
                                && released_drag.read(cx).grab.is_some()
                            {
                                released_drag.update(cx, |drag, _| drag.grab = None);
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
                                drag.update(cx, |drag, _| drag.grab = Some(grab));
                                window.prevent_default();
                            }
                            if let Some(grab) = drag.read(cx).grab {
                                handle.set_offset(point(
                                    handle.offset().x,
                                    geometry.offset(event.position.y, grab),
                                ));
                                if matches!(event.phase, TouchPhase::Ended | TouchPhase::Cancelled)
                                {
                                    drag.update(cx, |drag, _| drag.grab = None);
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
}
