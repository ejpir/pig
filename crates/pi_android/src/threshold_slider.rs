//! Direct manipulation of the compaction marker without panning the sheet.
//! Preview locally while dragging; send exactly one setting on release.

use crate::{model::SessionId, testing, theme::Theme};
use gpui::{
    AccessibleAction, App, Bounds, DispatchPhase, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, RenderOnce, Role, TouchDragEvent, TouchPhase, Window, canvas,
    div, fill, point, prelude::*, px, size,
};
use std::rc::Rc;

type Commit = Rc<dyn Fn(f64, &mut Window, &mut App)>;

#[derive(Default)]
struct Drag {
    preview: Option<f64>,
}

fn percent_at(x: f32, left: f32, width: f32, max: f64) -> f64 {
    if width <= 0. {
        return 10.;
    }
    // Tenths preserve useful control on large context windows.
    ((f64::from((x - left) / width) * 1000.).round() / 10.).clamp(10., max)
}

#[derive(IntoElement)]
pub(crate) struct ContextSlider {
    pub id: SessionId,
    pub percent: f64,
    pub max: f64,
    pub used: Option<u8>,
    pub enabled: bool,
    pub pending: bool,
    pub colors: Theme,
    pub on_commit: Commit,
}

impl RenderOnce for ContextSlider {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state =
            window.use_keyed_state(("compaction-drag", self.id.0), cx, |_, _| Drag::default());
        if !self.enabled {
            state.update(cx, |state, _| state.preview = None);
        }
        let percent = state.read(cx).preview.unwrap_or(self.percent);
        let colors = self.colors;
        let used = self.used.unwrap_or(0).min(100) as f32 / 100.;
        let enabled = self.enabled;
        let max = self.max;
        let on_commit = self.on_commit;
        let increment = on_commit.clone();
        let decrement = on_commit.clone();
        let meta = |text: String| {
            div()
                .text_size(px(12.5))
                .line_height(px(16.))
                .text_color(colors.muted)
                .child(text)
        };
        div()
            .child(
                div()
                    .id("details-compaction-slider")
                    .debug_selector(|| "details-compaction-slider".into())
                    .role(Role::Slider)
                    .aria_label("Auto-compaction threshold")
                    .aria_numeric_value(percent)
                    .aria_min_numeric_value(10.)
                    .aria_max_numeric_value(max)
                    .aria_numeric_value_step(1.)
                    .on_a11y_action(AccessibleAction::Increment, move |_, window, cx| {
                        if enabled {
                            increment((percent + 1.).clamp(10., max), window, cx);
                        }
                    })
                    .on_a11y_action(AccessibleAction::Decrement, move |_, window, cx| {
                        if enabled {
                            decrement((percent - 1.).clamp(10., max), window, cx);
                        }
                    })
                    .relative()
                    .child(testing::probe("details-compaction-slider"))
                    .child(
                        canvas(
                            move |bounds, window, _| {
                                let hitbox =
                                    window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal);
                                (bounds, hitbox)
                            },
                            move |_, (bounds, hitbox), window, _| {
                                // Leave room for the thumb at either end of the context bar.
                                let left = bounds.left() + px(11.);
                                let width = (bounds.size.width - px(22.)).max(px(1.));
                                let y = bounds.top() + px(20.);
                                let track =
                                    Bounds::new(point(left, y - px(4.)), size(width, px(8.)));
                                window.paint_quad(fill(track, colors.raised).corner_radii(px(4.)));
                                window.paint_quad(
                                    fill(
                                        Bounds::new(track.origin, size(width * used, px(8.))),
                                        colors.accent,
                                    )
                                    .corner_radii(px(4.)),
                                );
                                let thumb = Bounds::new(
                                    point(
                                        left + width * (percent / 100.) as f32 - px(10.),
                                        y - px(10.),
                                    ),
                                    size(px(20.), px(20.)),
                                );
                                window.paint_quad(
                                    fill(thumb, if enabled { colors.accent } else { colors.muted })
                                        .corner_radii(px(10.)),
                                );
                                if !enabled {
                                    return;
                                }

                                let touch_hitbox = hitbox.clone();
                                let down = state.clone();
                                window.on_mouse_event(
                                    move |event: &MouseDownEvent, phase, window, cx| {
                                        if phase != DispatchPhase::Capture
                                            || event.button != MouseButton::Left
                                            || !hitbox.is_hovered(window)
                                        {
                                            return;
                                        }
                                        down.update(cx, |state, cx| {
                                            state.preview = Some(percent_at(
                                                event.position.x.into(),
                                                left.into(),
                                                width.into(),
                                                max,
                                            ));
                                            cx.notify();
                                        });
                                        window.prevent_default();
                                        cx.stop_propagation();
                                    },
                                );
                                let moving = state.clone();
                                window.on_mouse_event(
                                    move |event: &MouseMoveEvent, phase, _, cx| {
                                        if phase != DispatchPhase::Capture
                                            || moving.read(cx).preview.is_none()
                                        {
                                            return;
                                        }
                                        moving.update(cx, |state, cx| {
                                            state.preview = Some(percent_at(
                                                event.position.x.into(),
                                                left.into(),
                                                width.into(),
                                                max,
                                            ));
                                            cx.notify();
                                        });
                                        cx.stop_propagation();
                                    },
                                );
                                let released = state.clone();
                                let commit = on_commit.clone();
                                window.on_mouse_event(
                                    move |event: &MouseUpEvent, phase, window, cx| {
                                        if phase != DispatchPhase::Capture
                                            || event.button != MouseButton::Left
                                        {
                                            return;
                                        }
                                        let preview = released.update(cx, |state, cx| {
                                            cx.notify();
                                            state.preview.take()
                                        });
                                        if let Some(percent) = preview {
                                            commit(percent, window, cx);
                                            cx.stop_propagation();
                                        }
                                    },
                                );
                                window.on_mouse_event(
                                    move |event: &TouchDragEvent, phase, window, cx| {
                                        if phase != DispatchPhase::Capture {
                                            return;
                                        }
                                        if event.phase == TouchPhase::Started {
                                            if !touch_hitbox.is_hovered(window) {
                                                return;
                                            }
                                            state.update(cx, |state, _| {
                                                state.preview = Some(percent)
                                            });
                                            window.prevent_default();
                                        }
                                        if state.read(cx).preview.is_none() {
                                            return;
                                        }
                                        let selected = percent_at(
                                            event.position.x.into(),
                                            left.into(),
                                            width.into(),
                                            max,
                                        );
                                        state.update(cx, |state, cx| {
                                            state.preview = if matches!(
                                                event.phase,
                                                TouchPhase::Ended | TouchPhase::Cancelled
                                            ) {
                                                None
                                            } else {
                                                Some(selected)
                                            };
                                            cx.notify();
                                        });
                                        if event.phase == TouchPhase::Ended {
                                            on_commit(selected, window, cx);
                                        }
                                        // Cancelled gestures never change the backend setting.
                                        cx.stop_propagation();
                                    },
                                );
                            },
                        )
                        .h(px(48.))
                        .w_full(),
                    ),
            )
            .child(
                div()
                    .flex()
                    .child(
                        meta(self.used.map_or_else(
                            || "Usage not reported".into(),
                            |used| format!("{used}% used"),
                        ))
                        .flex_1(),
                    )
                    .child(
                        meta(format!("auto-compacts above {percent:.1}%"))
                            .debug_selector(|| "details-compaction-policy".into()),
                    ),
            )
            .child(
                meta(
                    if self.pending {
                        "Saving threshold…"
                    } else if enabled {
                        "Drag the marker to set this session’s threshold"
                    } else {
                        "Reconnect to adjust the threshold"
                    }
                    .into(),
                )
                .mt(px(6.)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, InputEvent, Render};
    use std::cell::RefCell;

    struct Fixture {
        commits: Rc<RefCell<Vec<f64>>>,
        enabled: bool,
    }

    impl Render for Fixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let commits = self.commits.clone();
            div().w_full().child(ContextSlider {
                id: SessionId(1),
                percent: 81.9,
                max: 91.8,
                used: Some(56),
                enabled: self.enabled,
                pending: false,
                colors: Theme::new(true),
                on_commit: Rc::new(move |percent, _, _| commits.borrow_mut().push(percent)),
            })
        }
    }

    #[gpui::test]
    fn touch_previews_then_commits_once_and_cancel_or_disabled_never_commits(
        cx: &mut gpui::TestAppContext,
    ) {
        let commits = Rc::new(RefCell::new(Vec::new()));
        let (fixture, cx) = cx.add_window_view(|_, _| Fixture {
            commits: commits.clone(),
            enabled: true,
        });
        cx.simulate_resize(size(px(320.), px(300.)));
        cx.run_until_parked();
        let bounds = cx.debug_bounds("details-compaction-slider").unwrap();
        let at = |percent: f32| {
            point(
                bounds.left() + px(11.) + (bounds.size.width - px(22.)) * percent / 100.,
                bounds.center().y,
            )
        };
        let mut touch = |phase, position| {
            cx.update(|window, cx| {
                window.dispatch_event(
                    gpui::TouchEvent {
                        id: gpui::TouchId(1),
                        phase,
                        position,
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                )
            });
            cx.run_until_parked();
        };
        touch(TouchPhase::Started, at(81.9));
        touch(TouchPhase::Moved, at(70.));
        assert!(commits.borrow().is_empty(), "no RPC while dragging");
        touch(TouchPhase::Ended, at(70.));
        assert_eq!(&*commits.borrow(), &[70.]);
        touch(TouchPhase::Started, at(70.));
        touch(TouchPhase::Moved, at(50.));
        touch(TouchPhase::Cancelled, at(50.));
        assert_eq!(&*commits.borrow(), &[70.], "cancel keeps the setting");
        fixture.update(cx, |fixture, cx| {
            fixture.enabled = false;
            cx.notify();
        });
        cx.run_until_parked();
        cx.simulate_click(at(60.), gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(
            &*commits.borrow(),
            &[70.],
            "offline/pending sliders are inert"
        );
    }

    #[gpui::test]
    fn mouse_tap_and_drag_have_one_commit_and_clamp_at_safety_limit(cx: &mut gpui::TestAppContext) {
        let commits = Rc::new(RefCell::new(Vec::new()));
        let (_, cx) = cx.add_window_view(|_, _| Fixture {
            commits: commits.clone(),
            enabled: true,
        });
        cx.simulate_resize(size(px(320.), px(300.)));
        cx.run_until_parked();
        let bounds = cx.debug_bounds("details-compaction-slider").unwrap();
        let at = |percent: f32| {
            point(
                bounds.left() + px(11.) + (bounds.size.width - px(22.)) * percent / 100.,
                bounds.center().y,
            )
        };
        cx.simulate_click(at(60.), gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(&*commits.borrow(), &[60.]);
        cx.simulate_mouse_down(at(70.), MouseButton::Left, gpui::Modifiers::none());
        cx.run_until_parked();
        cx.simulate_mouse_move(at(120.), Some(MouseButton::Left), gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(&*commits.borrow(), &[60.]);
        cx.simulate_mouse_up(at(120.), MouseButton::Left, gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(&*commits.borrow(), &[60., 91.8]);
    }

    #[test]
    fn pointer_position_maps_to_context_percentage_and_safe_limits() {
        assert_eq!(percent_at(180., 20., 200., 91.8), 80.);
        assert_eq!(percent_at(183.8, 20., 200., 91.8), 81.9);
        assert_eq!(percent_at(-50., 20., 200., 91.8), 10.);
        assert_eq!(percent_at(500., 20., 200., 91.8), 91.8);
        assert_eq!(percent_at(50., 20., 0., 91.8), 10.);
    }
}
