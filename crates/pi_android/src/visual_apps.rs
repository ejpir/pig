//! Android navigation/admission adapter. Visualize's host implementation is shared.
use crate::{
    app::{PhoneApp, Route},
    model::{Session, SessionId},
    theme::Theme,
};
use gpui::{Context, Div, Window, div, prelude::*, px};
use pi_visual::reply::Part;
use pi_visual_ui::{Host, HostExt, identity};
pub(crate) type VisualApps = pi_visual_ui::VisualApps<Route>;
fn palette(c: &Theme) -> pi_visual_ui::theme::Theme {
    pi_visual_ui::theme::Theme {
        canvas: c.canvas,
        panel: c.panel,
        line: c.line,
        line_strong: c.line_strong,
        selected: c.selected,
        text: c.text,
        secondary: c.secondary,
        muted: c.muted,
        accent: c.accent,
        on_accent: c.on_accent,
        coral: c.coral,
        tint: c.tint(c.accent),
    }
}
impl Host for PhoneApp {
    type Route = Route;
    type Target = (SessionId, usize);
    fn visual_apps(&self) -> &std::cell::RefCell<VisualApps> {
        &self.visual_apps
    }
    fn visual_route(&self) -> Route {
        self.route()
    }
    fn visual_available(&self) -> bool {
        self.visible && self.window_active && self.sheet.is_none() && self.closing_sheet.is_none()
    }
    fn visual_persist(&self) -> bool {
        !self.paused
    }
    fn visual_changed(&mut self, (id, turn): Self::Target, _cx: &mut Context<Self>) {
        self.remeasure_thread_turn(id, turn);
    }
    fn visual_refocus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }
    fn visual_show_source(
        &mut self,
        _: &str,
        source: &str,
        _: Self::Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_selected_text(source.to_owned(), window, cx);
    }
}
impl PhoneApp {
    pub(crate) fn visual_has_app(&self, source: &str) -> bool {
        self.visual_apps.borrow_mut().parsed_reply(source).has_app()
    }

    pub(crate) fn visual_enabled(&self, session: &Session, turn: usize) -> bool {
        session
            .turns
            .iter()
            .take(turn + 1)
            .any(|turn| !turn.reported && crate::visualize_prompt::requested(&turn.prompt))
    }

    /// Explicit assistant-only seam: other Markdown remains inert.
    pub(crate) fn visual_reply(
        &self,
        session: &Session,
        turn: usize,
        slot: usize,
        source: &str,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let enabled = self.visual_enabled(session, turn);
        let parsed = self.visual_apps.borrow_mut().parsed_reply(source);
        if !enabled || !parsed.has_app() {
            return div()
                .relative()
                .min_w_0()
                .child(crate::message::render(source, colors))
                .child(self.copyable(source.to_owned(), cx));
        }
        let mut column = div().min_w_0().flex().flex_col().gap(px(12.));
        // A flow's slot is its Flow index; aggregate summaries use slot zero.
        // Normalize to a turn-wide app ordinal, stable when tools are expanded
        // or a streamed reply switches to its aggregate summary.
        let preceding = session.turns.get(turn).map_or(0, |turn| {
            turn.flow
                .iter()
                .take(slot)
                .filter_map(|step| match step {
                    crate::model::Flow::Text(text) => Some(
                        self.visual_apps
                            .borrow_mut()
                            .parsed_reply(text)
                            .parts()
                            .filter(|p| matches!(p, Part::App { .. }))
                            .count(),
                    ),
                    _ => None,
                })
                .sum::<usize>()
        });
        let mut examined = preceding;
        for part in parsed.parts() {
            match part {
                Part::Markdown(text) => column = column.child(crate::message::render(text, colors)),
                Part::App {
                    source,
                    raw,
                    ordinal,
                } => {
                    let card = (examined < 2)
                        .then(|| {
                            self.visual_card(
                                session,
                                turn,
                                0,
                                preceding + ordinal,
                                source,
                                colors,
                                cx,
                            )
                        })
                        .flatten();
                    examined += 1;
                    if let Some(card) = card {
                        column = column.child(card);
                    } else {
                        column = column.child(crate::message::render(raw, colors));
                    }
                }
            }
        }
        column
    }

    #[allow(clippy::too_many_arguments)]
    fn visual_card(
        &self,
        session: &Session,
        turn: usize,
        slot: usize,
        ordinal: usize,
        source: &str,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Option<gpui::Stateful<Div>> {
        let id = session.id;
        let projected = session.turns.get(turn)?;
        if !self.paused
            && (projected.visual_identity.is_none()
                || session
                    .turns
                    .iter()
                    .filter(|t| t.visual_identity == projected.visual_identity)
                    .count()
                    > 1)
        {
            return None;
        }
        let turn_key = session
            .turns
            .get(turn)?
            .visual_identity
            .clone()
            .unwrap_or_else(|| format!("sample-turn-{turn}-{}", session.turns[turn].at));
        let computer = &self.store.as_ref()?.computer.address;
        // Scope comes from the same immutable projected message as the UI,
        // not a newer live watch whose positional SessionId may have changed.
        let session_key = session
            .turns
            .get(turn)?
            .visual_identity
            .clone()
            .unwrap_or_else(|| format!("sample-{}-{}", id.0, session.details.session_file));
        let key = identity(
            computer,
            &session_key,
            &turn_key,
            slot,
            ordinal,
            &session.turns.get(turn)?.prompt,
            source,
        );
        self.render_visual_card(&key, source, ordinal, (id, turn), &palette(colors), cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_visual_ui::visual_controls::Gesture;
    use std::sync::Arc;
    #[gpui::test]
    fn blur_and_brief_hidden_transition_cancel_without_waiting_for_projection_poll(
        cx: &mut gpui::TestAppContext,
    ) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        for hidden in [false, true] {
            cx.update(|window, _| window.activate_window());
            cx.run_until_parked();
            let barrier = app.update(cx, |app, _| {
                app.visual_apps
                    .borrow_mut()
                    .test_projection_barrier(app.route())
            });
            let lease = barrier.acquire();
            app.update(cx, |app, _| {
                let mut apps = app.visual_apps.borrow_mut();
                let instance = apps.running.get_mut("projection-fixture").unwrap();
                let runtime = instance.runtime.as_mut().unwrap();
                instance.input_snapshot = Some(runtime.state_json().unwrap());
                runtime.slide("x", 20.0).unwrap();
            });
            if hidden {
                cx.simulate_visibility_change(gpui::WindowVisibility::Hidden);
                cx.simulate_visibility_change(gpui::WindowVisibility::Visible);
            } else {
                cx.deactivate_window(); // Still visible; no synthetic native Cancel.
            }
            assert!(!barrier.active());
            app.read_with(cx, |app, _| {
                assert_eq!(
                    app.visual_apps.borrow().running["projection-fixture"]
                        .runtime
                        .as_ref()
                        .unwrap()
                        .number("x"),
                    Some(0.0)
                );
            });
            let newer = barrier.acquire();
            drop(lease);
            assert!(barrier.active());
            drop(newer);
        }
    }

    #[gpui::test]
    fn direct_manipulation_deduplicates_taps_and_restores_cancelled_state(
        cx: &mut gpui::TestAppContext,
    ) {
        let source: Arc<str> = r#"app({title:'Input test',state:{x:0,count:0},view:ui.column([
            ui.canvas({id:'scene',pointerId:'tap',label:'Tap canvas',height:200,viewBox:[0,0,100,100],
                draw(g,s){if(s.x>50)throw Error('Drawing error');},onPointer(s,x,y){s.count++;s.x=x;}}),
            ui.slider('Position','x',{min:0,max:100,step:1,onChange(s){s.count++;}})
        ])});"#.into();
        let key = "input-test";
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        let painted = std::rc::Rc::new(std::cell::Cell::new(true));
        let input = app.update(cx, |app, cx| {
            app.visual_apps.borrow_mut().ensure(key, &source, false);
            app.visual_apps.borrow().running[key]
                .painted
                .borrow_mut()
                .insert("test-mount".into(), std::rc::Rc::downgrade(&painted));
            app.visual_apps
                .borrow_mut()
                .running
                .get_mut(key)
                .unwrap()
                .route = Some(app.route());
            app.visual_input(
                key,
                &source,
                "tap",
                true,
                app.visual_apps.borrow().running[key].generation,
                (SessionId(1), 0),
                cx,
            )
        });
        let send = |gesture, position, cx: &mut gpui::VisualTestContext| {
            cx.update(|window, cx| input(position, gesture, window, cx));
        };
        send(Gesture::Start, [10.0, 10.0], cx);
        send(Gesture::End, [10.0, 10.0], cx);
        app.read_with(cx, |app, _| {
            let apps = app.visual_apps.borrow();
            let runtime = apps.running[key].runtime.as_ref().unwrap();
            assert_eq!(runtime.number("count"), Some(1.0));
        });
        send(Gesture::Start, [60.0, 10.0], cx);
        app.read_with(cx, |app, _| {
            assert!(
                app.visual_apps.borrow().running[key]
                    .runtime
                    .as_ref()
                    .unwrap()
                    .scene("scene")
                    .is_err()
            );
        });
        send(Gesture::Cancel, [60.0, 10.0], cx);
        app.read_with(cx, |app, _| {
            let apps = app.visual_apps.borrow();
            let runtime = apps.running[key].runtime.as_ref().unwrap();
            assert_eq!(runtime.number("x"), Some(10.0));
            assert_eq!(runtime.number("count"), Some(1.0));
            assert!(runtime.scene("scene").is_ok());
        });
        let slider = app.update(cx, |app, cx| {
            app.visual_input(
                key,
                &source,
                "x",
                false,
                app.visual_apps.borrow().running[key].generation,
                (SessionId(1), 0),
                cx,
            )
        });
        cx.update(|window, cx| slider([20.1, 0.0], Gesture::Start, window, cx));
        cx.update(|window, cx| slider([20.4, 0.0], Gesture::End, window, cx));
        app.read_with(cx, |app, _| {
            let apps = app.visual_apps.borrow();
            let runtime = apps.running[key].runtime.as_ref().unwrap();
            assert_eq!(runtime.number("x"), Some(20.0));
            assert_eq!(runtime.number("count"), Some(2.0));
        });
    }

    #[gpui::test]
    fn visual_runtime_survives_flow_summary_and_prose_changes(cx: &mut gpui::TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.preview("visual-calculator", window, cx))
        });
        cx.run_until_parked();
        app.update(cx, |app, cx| {
            let id = SessionId(1);
            let source = app.session(id).unwrap().turns[0]
                .summary
                .as_ref()
                .unwrap()
                .text();
            let colors = crate::theme::theme(cx);
            let _ = app.visual_reply(
                &app.session(id).unwrap().clone(),
                0,
                0,
                &source,
                &colors,
                cx,
            );
            {
                let mut apps = app.visual_apps.borrow_mut();
                assert_eq!(apps.running.len(), 1);
                apps.running
                    .values_mut()
                    .next()
                    .unwrap()
                    .runtime
                    .as_mut()
                    .unwrap()
                    .dispatch("7")
                    .unwrap();
            }
            let session = app
                .store
                .as_mut()
                .unwrap()
                .sessions
                .iter_mut()
                .find(|s| s.id == id)
                .unwrap();
            session.turns[0].flow = vec![
                crate::model::Flow::Text("Earlier prose".into()),
                crate::model::Flow::Text(source.clone()),
            ];
            let _ = app.visual_reply(
                &app.session(id).unwrap().clone(),
                0,
                1,
                &format!("{source}\nMore prose"),
                &colors,
                cx,
            );
            let apps = app.visual_apps.borrow();
            assert_eq!(apps.running.len(), 1);
            assert_eq!(
                apps.running
                    .values()
                    .next()
                    .unwrap()
                    .runtime
                    .as_ref()
                    .unwrap()
                    .text("display"),
                "7"
            );
        });
    }
}
