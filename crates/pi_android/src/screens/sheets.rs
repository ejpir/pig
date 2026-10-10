//! Bottom sheets: a question to answer (05), a session's details (09), and
//! the short choices: attaching, the model, the project, more actions, and
//! what Pi has on the computer.

use crate::{
    app::{PhoneApp, Route, Sheet, Target},
    model::{Reference, Session, SessionId, Stage, StageKind, StageStatus, Turn},
    theme::{MONO, Theme, theme},
    ui::{self, Button, icon},
};
use gpui::{
    AnyElement, Context, Div, ElementId, FontWeight, SharedString, Window, div, prelude::*, px,
    relative,
};

const MODELS: [(&str, &str); 4] = [
    ("Opus 5.5", "Anthropic · the most capable"),
    ("Sonnet 5.5", "Anthropic · fast and capable"),
    ("Fable 5.1", "Anthropic"),
    ("Haiku 4.5", "Anthropic · the fastest"),
];

/// Where Pi compacts the context, as a share of the window.
const COMPACTS_AT: f32 = 0.8;

const THINKING: [&str; 6] = ["Off", "Minimal", "Low", "Medium", "High", "Max"];

fn activity_stage(turn: &Turn, kind: StageKind) -> Option<&Stage> {
    turn.stages.iter().find(|stage| stage.kind == kind)
}

/// What a row of the More sheet does.
type Action = Box<dyn Fn(&mut PhoneApp, &mut Window, &mut Context<PhoneApp>)>;

fn model_matches(query: &str, name: &str, provider: &str, id: &str) -> bool {
    let searchable = format!("{name} {provider} {id}").to_lowercase();
    query
        .split_whitespace()
        .all(|word| searchable.contains(word))
}

impl PhoneApp {
    pub(crate) fn sheet_content(
        &mut self,
        sheet: Sheet,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = theme(cx);
        // Search stays in place even when hundreds of models are offered.
        if sheet == Sheet::Model {
            return self.model_sheet(&colors, cx).into_any_element();
        }
        if sheet == Sheet::Project {
            return self.project_sheet(&colors, cx).into_any_element();
        }
        let content = match sheet {
            Sheet::Details(id) => self.details_sheet(id, &colors, cx),
            Sheet::Attach(target) => self.attach_sheet(target, &colors, cx),
            Sheet::Model => self.model_sheet(&colors, cx),
            Sheet::Thinking => self.thinking_sheet(&colors, cx),
            Sheet::Project => div(),
            Sheet::More(id) => self.more_sheet(id, &colors, cx),
            Sheet::Models => self.models_sheet(&colors, cx),
            Sheet::Resources => self.resources_sheet(&colors, cx),
            Sheet::Activity(id, turn, kind) => self.activity_sheet(id, turn, kind, &colors, cx),
            Sheet::Delete(id) => self.delete_sheet(id, &colors, cx),
            Sheet::RestoreHistory(id, index) => self.restore_history_sheet(id, index, &colors, cx),
            Sheet::EnableJj(id) => self.enable_jj_sheet(id, &colors, cx),
            Sheet::Computers => self.computers_sheet(&colors, cx),
            Sheet::Logs => self.logs_sheet(&colors, cx),
            Sheet::SelectText => {
                let lines = (f32::from(window.viewport_size().height) * 0.5 / 22.) as usize;
                self.selectable
                    .update(cx, |area, _| area.set_max_lines(lines.max(4)));
                let text = self.selectable.read(cx).text().to_owned();
                div()
                    .pb(px(8.))
                    .child(
                        ui::hint(
                            "Hold a word, then drag the handles to choose what to copy.",
                            &colors,
                        )
                        .mt(px(4.)),
                    )
                    .child(
                        div()
                            .mt(px(12.))
                            .p(px(12.))
                            .rounded(px(12.))
                            .bg(colors.composer)
                            .border_1()
                            .border_color(colors.line)
                            .child(self.selectable.clone()),
                    )
                    .child(
                        ui::button(
                            "copy-all",
                            Button::Plain,
                            Some("copy"),
                            "Copy all",
                            false,
                            &colors,
                        )
                        .mt(px(12.))
                        .w_full()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy(text.clone(), "all of it", cx)
                        })),
                    )
            }
            Sheet::ToolImage(id, turn, n) => {
                let image = self
                    .session(id)
                    .and_then(|session| session.turns.get(turn))
                    .and_then(|turn| turn.images.get(n))
                    .cloned();
                let shown = image.as_ref().map(|image| self.tool_image(id, image));
                div().pb(px(8.)).child(match shown {
                    Some(Ok(Some(shown))) => self
                        .zoomable_image(shown.image, shown.ratio, cx)
                        .rounded(px(12.))
                        .bg(colors.panel)
                        .into_any_element(),
                    Some(Err(error)) => ui::hint(error, &colors).into_any_element(),
                    _ => {
                        ui::hint("Getting the image from the computer…", &colors).into_any_element()
                    }
                })
            }
            Sheet::Image(target, index) => {
                let attachment = self
                    .composer(target)
                    .and_then(|composer| composer.read(cx).attachments().get(index).cloned());
                if let Some(crate::composer::Attachment::Image { name, image }) = attachment {
                    div()
                        .child(ui::hint(name, &colors).mb(px(12.)))
                        .child(gpui::img(image).w_full().h(window.fully_visible_bounds().size.height * 0.55).object_fit(gpui::ObjectFit::Contain))
                        .child(ui::hint("Included with your next message. Images are resized to fit the upload limit.", &colors).mt(px(12.)))
                } else {
                    div()
                }
            }
        };
        // Every sheet keeps its identity and explicit close action visible,
        // independently of the scroll position of long output or lists.
        let header = match sheet {
            Sheet::Details(id) => {
                let (title, subtitle) = self.session(id).map_or_else(
                    || (SharedString::from("Details"), String::new()),
                    |session| {
                        let started = session
                            .turns
                            .first()
                            .map(|turn| turn.at.clone())
                            .filter(|at| !at.is_empty())
                            .map(|at| format!(" · started {at}"))
                            .unwrap_or_default();
                        (
                            session.title.clone().into(),
                            format!("{} · {}{started}", session.project, self.computer_name()),
                        )
                    },
                );
                div()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(22.))
                                    .line_height(px(28.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(title),
                            )
                            .child(
                                div()
                                    .mt(px(4.))
                                    .text_size(px(12.5))
                                    .line_height(px(16.))
                                    .text_color(colors.muted)
                                    .truncate()
                                    .child(subtitle),
                            ),
                    )
                    .child(
                        ui::tap("close-sheet", "x", &colors)
                            .mr(px(-12.))
                            .on_click(cx.listener(|this, _, _, cx| this.close_sheet(cx))),
                    )
            }
            Sheet::Computers => div()
                .text_size(px(22.))
                .line_height(px(28.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Computers"),
            _ => {
                let title: SharedString = match sheet {
                    Sheet::Details(_) => "Details".into(),
                    Sheet::Attach(_) => "Add to the message".into(),
                    Sheet::Model => "Choose model".into(),
                    Sheet::Thinking => "Thinking level".into(),
                    Sheet::Project => format!("Projects on {}", self.computer_name()).into(),
                    Sheet::More(id) => self
                        .session(id)
                        .map_or_else(|| "Session".into(), |s| s.title.clone().into()),
                    Sheet::Models => format!("Models on {}", self.computer_name()).into(),
                    Sheet::Resources => format!("Resources on {}", self.computer_name()).into(),
                    Sheet::Activity(id, turn, kind) => self
                        .session(id)
                        .and_then(|s| s.turns.get(turn))
                        .and_then(|turn| activity_stage(turn, kind))
                        .map_or("Activity", |stage| stage.kind.name(stage.status))
                        .into(),
                    Sheet::Delete(_) => "Delete session?".into(),
                    Sheet::RestoreHistory(_, _) => "Restore project files?".into(),
                    Sheet::EnableJj(_) => "Turn on jj file history?".into(),
                    Sheet::Image(_, _) => "Attached image".into(),
                    Sheet::Logs => "Debug log".into(),
                    Sheet::ToolImage(id, turn, n) => self
                        .session(id)
                        .and_then(|session| session.turns.get(turn))
                        .and_then(|turn| turn.images.get(n))
                        .map_or_else(|| "Image".into(), |image| image.name.clone().into()),
                    Sheet::Computers => "Computers".into(),
                    Sheet::SelectText => "Select text".into(),
                };
                self.sheet_title(title, &colors, cx)
            }
        };
        div()
            .flex()
            .flex_col()
            .min_h_0()
            .child(
                header
                    .flex_none()
                    .px(px(20.))
                    .debug_selector(|| "sheet-header".into()),
            )
            .child(
                crate::scroll::vertical(
                    SharedString::from(format!("sheet-body-{sheet:?}")),
                    &self.sheet_scroll,
                )
                .min_h_0()
                .px(px(20.))
                .child(content),
            )
            .into_any_element()
    }

    /// An image to look closely at: pinch or double tap to zoom, drag to pan
    /// while zoomed in.
    fn zoomable_image(
        &self,
        image: impl Into<gpui::ImageSource>,
        aspect_ratio: f32,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        const MAX_ZOOM: f32 = 6.;
        let image_box = self.image_box.clone();
        let (zoom, pan) = (self.image_zoom, self.image_pan);
        // Zooms by `factor` around `at` in the window, keeping the image
        // covering the box.
        fn zoom_at(this: &mut PhoneApp, factor: f32, at: gpui::Point<gpui::Pixels>) {
            let bounds = this.image_box.get();
            let zoom = (this.image_zoom * factor).clamp(1., MAX_ZOOM);
            let local = at - bounds.origin;
            let scale = zoom / this.image_zoom;
            this.image_zoom = zoom;
            this.image_pan = local - (local - this.image_pan) * scale;
            clamp_pan(this);
        }
        fn clamp_pan(this: &mut PhoneApp) {
            let size = this.image_box.get().size;
            let spare = |extent: gpui::Pixels| extent * (1. - this.image_zoom);
            this.image_pan.x = this.image_pan.x.clamp(spare(size.width), px(0.));
            this.image_pan.y = this.image_pan.y.clamp(spare(size.height), px(0.));
        }
        div()
            .id("tool-image-view")
            .debug_selector(|| "tool-image-view".into())
            .relative()
            .w_full()
            // At the natural full-width ratio, portrait screenshots become a
            // tall sheet-body item. The sheet scroll then reaches every pixel
            // instead of clipping the bottom inside a viewport-sized box.
            .aspect_ratio(aspect_ratio)
            .overflow_hidden()
            .child(
                gpui::canvas(move |bounds, _, _| image_box.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            )
            .child(
                div()
                    .absolute()
                    .left(pan.x)
                    .top(pan.y)
                    .w(relative(zoom))
                    .h(relative(zoom))
                    .child(
                        gpui::img(image)
                            .size_full()
                            .object_fit(gpui::ObjectFit::Contain),
                    ),
            )
            .on_pinch(cx.listener(|this, event: &gpui::PinchEvent, _, cx| {
                if event.phase == gpui::TouchPhase::Moved {
                    zoom_at(this, 1. + event.delta, event.position);
                    cx.notify();
                }
                cx.stop_propagation();
            }))
            .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                if this.image_zoom <= 1.01 {
                    return;
                }
                this.image_pan += event.delta.pixel_delta(px(20.));
                clamp_pan(this);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                if event.click_count() < 2 {
                    return;
                }
                if this.image_zoom > 1.01 {
                    this.image_zoom = 1.;
                    this.image_pan = gpui::Point::default();
                } else {
                    zoom_at(this, 2.5, event.position());
                }
                cx.notify();
            }))
    }

    /// 13 Computers: each with its state or Connect, pairing another, the
    /// sample sessions and Settings.
    fn computers_sheet(&self, colors: &Theme, cx: &Context<Self>) -> Div {
        let live = self.live();
        let working = self.store.as_ref().map_or(0, |store| {
            store
                .sessions
                .iter()
                .filter(|s| s.state.is_running())
                .count()
        });
        let item = |id: ElementId| {
            div()
                .id(id.clone())
                .relative()
                .child(crate::testing::probe(format!("{id:?}")))
                .min_h(px(64.))
                .py(px(12.))
                .flex()
                .items_center()
                .gap(px(12.))
                .border_color(colors.line)
                .active(|style| style.bg(colors.selected))
        };
        let meta = |text: SharedString| {
            div()
                .text_size(px(12.5))
                .line_height(px(16.))
                .text_color(colors.muted)
                .child(text)
        };
        let computers = self
            .store
            .iter()
            .flat_map(|store| store.computers.iter())
            .enumerate()
            .map(|(index, computer)| {
                let current = index == 0;
                let address = computer.address.clone();
                item(("computer", index).into())
                    .when(index > 0, |row| row.border_t_1())
                    .child(icon(
                        if current { "computer" } else { "server" },
                        20.,
                        colors.muted,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(computer.name.clone()),
                            )
                            .child(meta(computer.address.clone().into()).mt(px(2.)).truncate()),
                    )
                    .child(if current {
                        div()
                            .flex()
                            .flex_col()
                            .items_end()
                            .when(working > 0, |state| {
                                state.child(meta(format!("{working} working").into()))
                            })
                            .child(
                                meta(
                                    if computer.connected || !live {
                                        "Connected"
                                    } else {
                                        "Reconnecting…"
                                    }
                                    .into(),
                                )
                                .text_color(
                                    if computer.connected || !live {
                                        colors.green
                                    } else {
                                        colors.wait
                                    },
                                ),
                            )
                            .into_any_element()
                    } else {
                        ui::button(
                            ("connect-computer", index),
                            Button::Plain,
                            None,
                            "Connect",
                            true,
                            colors,
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.close_sheet(cx);
                            this.address
                                .update(cx, |field, cx| field.set_text(address.clone(), cx));
                            this.manual_setup = true;
                            this.connect(window, cx);
                        }))
                        .into_any_element()
                    })
            })
            .collect::<Vec<_>>();
        let link = |id: &'static str, glyph: &'static str, text: &'static str| {
            item(id.into())
                .min_h(px(56.))
                .child(icon(glyph, 20., colors.muted))
                .child(div().flex_1().child(text))
                .child(icon("chev_r", 16., colors.faint))
        };
        div()
            .pb(px(8.))
            .child(
                div().mt(px(8.)).children(computers).child(
                    item("pair-computer".into())
                        .min_h(px(56.))
                        .border_t_1()
                        .text_color(colors.accent)
                        .child(icon("plus", 20., colors.accent))
                        .child(
                            div()
                                .flex_1()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Pair another computer"),
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_sheet(cx);
                            this.manual_setup = false;
                            this.push(Route::Connect, window, cx);
                        })),
                ),
            )
            .child(div().mt(px(8.)).mx(px(-20.)).h(px(1.)).bg(colors.line))
            .child(
                link("sample-sessions", "spark", "Try the sample sessions").on_click(cx.listener(
                    |this, _, window, cx| {
                        this.close_sheet(cx);
                        this.open_sample(window, cx);
                    },
                )),
            )
            .child(
                link("open-settings", "settings", "Settings")
                    .border_t_1()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close_sheet(cx);
                        this.push(Route::Settings, window, cx);
                    })),
            )
    }

    /// A session of the computer's, or a subagent's whose screen is open.
    fn session(&self, id: SessionId) -> Option<&Session> {
        self.store
            .as_ref()
            .and_then(|store| store.session(id))
            .or_else(|| self.subagent_sessions.get(&id))
    }

    fn delete_sheet(&self, id: SessionId, colors: &Theme, cx: &Context<Self>) -> Div {
        let Some(session) = self.session(id) else {
            return div();
        };
        let running = session.state.is_running();
        let deleting = self.deleting_session == Some(id);
        div().pb(px(8.))
            .child(ui::card(colors).p(px(14.)).my(px(12.))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(session.title.clone()))
                .child(ui::hint(format!("{} · {}", session.project, self.computer_name()), colors).mt(px(4.))))
            .child(ui::hint(
                if self.live() {
                    "Permanently deletes this session's conversation and execution history from the computer. This cannot be undone. Project files and edits are kept."
                } else {
                    "Removes this sample conversation. No files on your computer are affected."
                }, colors))
            .when(running, |body| body.child(ui::hint("This session is still running. Stop it before deleting its history.", colors).mt(px(12.)).text_color(colors.coral)))
            .child(div().flex().gap(px(12.)).mt(px(20.))
                .child(ui::button("cancel-delete", Button::Plain, None, "Cancel", false, colors)
                    .flex_1().on_click(cx.listener(|this, _, _, cx| this.close_sheet(cx))))
                .child(ui::button("confirm-delete", Button::Primary, Some("trash"), if deleting { "Deleting…" } else { "Delete" }, false, colors)
                    .debug_selector(|| "confirm-delete".into())
                    .flex_1().min_w_0().bg(colors.coral).border_color(colors.coral)
                    .when(running || deleting, |button| button.opacity(0.45))
                    .when(!running && !deleting, |button| button.on_click(cx.listener(move |this, _, _, cx| this.delete_session(id, cx))))))
    }

    fn restore_history_sheet(
        &self,
        id: SessionId,
        index: usize,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let operation = match self.jj_histories.get(&id) {
            Some(crate::app::JjHistoryState::Loaded(history)) => history.operations.get(index),
            _ => None,
        };
        let Some(operation) = operation else {
            return div().child(ui::hint(
                "That history point is no longer available.",
                colors,
            ));
        };
        let restoring = self.restoring_history == Some((id, index));
        let running = self.project_is_running(id);
        let short_id: String = operation.id.chars().take(12).collect();
        div()
            .pb(px(8.))
            .child(
                ui::card(colors)
                    .p(px(14.))
                    .my(px(12.))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(operation.description.clone()),
                    )
                    .child(ui::hint(format!("jj operation {short_id}"), colors).mt(px(4.))),
            )
            .child(ui::hint(
                "The project files on the computer will be restored to this point. jj first records the files as they are now, then records the restore, so the action remains recoverable.",
                colors,
            ))
            .when(running, |body| {
                body.child(
                    ui::hint(
                        "A session in this project is active. Stop it before restoring files.",
                        colors,
                    )
                    .mt(px(12.))
                    .text_color(colors.coral),
                )
            })
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .mt(px(20.))
                    .child(
                        ui::button(
                            "cancel-restore",
                            Button::Plain,
                            None,
                            "Cancel",
                            false,
                            colors,
                        )
                        .flex_1()
                        .on_click(cx.listener(|this, _, _, cx| this.close_sheet(cx))),
                    )
                    .child({
                        let button = ui::button(
                            "confirm-restore",
                            Button::Primary,
                            Some("clock"),
                            if restoring { "Restoring…" } else { "Restore" },
                            false,
                            colors,
                        )
                        .flex_1()
                        .min_w_0();
                        if running || restoring {
                            ui::disabled(button, colors)
                        } else {
                            button.on_click(cx.listener(move |this, _, _, cx| {
                                this.restore_jj_history(id, index, cx)
                            }))
                        }
                    }),
            )
    }

    fn enable_jj_sheet(&self, id: SessionId, colors: &Theme, cx: &Context<Self>) -> Div {
        let enabling = self.enabling_jj == Some(id);
        let running = self.project_is_running(id);
        div()
            .pb(px(8.))
            .child(
                ui::card(colors)
                    .p(px(14.))
                    .my(px(12.))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Recover agent edits with built-in jj"),
                    )
                    .child(
                        ui::hint(
                            "Pi initializes jj alongside this Git repository. Existing branches stay as they are; future remote turns that edit files become recoverable changes.",
                            colors,
                        )
                        .mt(px(6.)),
                    ),
            )
            .child(ui::hint(
                "This writes a .jj workspace on the computer. It does not upload project files or require a separate jj executable.",
                colors,
            ))
            .when(running, |body| {
                body.child(
                    ui::hint(
                        "A session in this project is active. Stop it before enabling file history.",
                        colors,
                    )
                    .mt(px(12.))
                    .text_color(colors.coral),
                )
            })
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .mt(px(20.))
                    .child(
                        ui::button(
                            "cancel-enable-jj",
                            Button::Plain,
                            None,
                            "Cancel",
                            false,
                            colors,
                        )
                        .flex_1()
                        .on_click(cx.listener(|this, _, _, cx| this.close_sheet(cx))),
                    )
                    .child({
                        let button = ui::button(
                            "confirm-enable-jj",
                            Button::Primary,
                            Some("shield"),
                            if enabling { "Turning on…" } else { "Turn on" },
                            false,
                            colors,
                        )
                        .flex_1();
                        if running || enabling {
                            ui::disabled(button, colors)
                        } else {
                            button.on_click(cx.listener(move |this, _, _, cx| {
                                this.enable_jj(id, cx)
                            }))
                        }
                    }),
            )
    }

    fn activity_sheet(
        &self,
        id: SessionId,
        turn_index: usize,
        kind: StageKind,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let Some(session) = self.session(id) else {
            return div();
        };
        let Some(stage) = session
            .turns
            .get(turn_index)
            .and_then(|turn| activity_stage(turn, kind))
        else {
            return div();
        };
        let wrap = self.wrap_lines(cx);
        div()
            .pb(px(12.))
            .min_w_0()
            .child(
                ui::hint(
                    format!(
                        "Turn {} · {} · {}",
                        turn_index + 1,
                        session.project,
                        self.computer_name()
                    ),
                    colors,
                )
                .mb(px(12.)),
            )
            .child(
                div()
                    .text_size(px(15.))
                    .child(stage.what.clone())
                    .mb(px(12.)),
            )
            .when(!stage.diff.is_empty(), |body| {
                body.child(
                    ui::card(colors)
                        .id(("activity-live-diff", stage.diff.len()))
                        .relative()
                        .child(crate::testing::probe(format!(
                            "activity-live-diff-{}",
                            stage.diff.len()
                        )))
                        .mb(px(12.))
                        .py(px(8.))
                        .child(
                            ui::label(
                                if stage.status == StageStatus::Live {
                                    "LIVE CHANGES"
                                } else {
                                    "CHANGES"
                                },
                                colors,
                            )
                            .px(px(12.))
                            .mb(px(6.)),
                        )
                        .children(stage.diff_path.as_ref().map(|path| {
                            div()
                                .px(px(12.))
                                .pb(px(8.))
                                .font_family(MONO)
                                .text_size(px(12.5))
                                .line_height(px(18.))
                                .text_color(colors.muted)
                                .child(path.clone())
                        }))
                        .children(
                            stage
                                .diff
                                .iter()
                                .map(|line| ui::diff_line(line, false, colors)),
                        ),
                )
            })
            .children(
                stage
                    .references
                    .iter()
                    .enumerate()
                    .map(|(index, reference)| {
                        let (glyph, target) = match reference {
                            Reference::File(path) => ("file", path),
                            Reference::Search(query) => ("search", query),
                        };
                        let file = session
                            .files
                            .iter()
                            .position(|file| file.path == *target || file.name() == target);
                        let copied = target.clone();
                        let action = if let Some(file) = file {
                            ui::button(
                                ("open-reference", index),
                                Button::Quiet,
                                None,
                                "Review",
                                true,
                                colors,
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_review(id, file, window, cx);
                            }))
                            .into_any_element()
                        } else {
                            ui::tap(("open-reference", index), "copy", colors)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        copied.clone(),
                                    ));
                                    this.notify_user("Copied", cx);
                                }))
                                .into_any_element()
                        };
                        ui::row(("activity-reference", index), index == 0, colors)
                            .child(icon(glyph, 18., colors.muted))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(ui::mono(target.clone(), 12.5)),
                            )
                            .child(action)
                    }),
            )
            .children(stage.tools.iter().enumerate().map(|(index, tool)| {
                let target = tool.target.clone();
                let output = tool.output.clone();
                let displayed_output = tool.output_for_display().into_owned();
                let status = if tool.failed {
                    "Failed"
                } else if tool.finished {
                    "Finished"
                } else {
                    "Running…"
                };
                ui::card(colors)
                    .mt(px(12.))
                    .p(px(14.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(ui::label(tool.name.clone(), colors).flex_1())
                            .child(ui::hint(status, colors).text_color(if tool.failed {
                                colors.coral
                            } else {
                                colors.muted
                            })),
                    )
                    .child(
                        div()
                            .mt(px(2.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(ui::mono(tool.target.clone(), 12.5).flex_1().min_w_0())
                            .child(
                                ui::tap(("copy-command", index), "copy", colors)
                                    .aria_label("Copy command")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                            target.clone(),
                                        ));
                                        this.notify_user("Copied", cx);
                                    })),
                            ),
                    )
                    .when(!tool.output.is_empty(), |card| {
                        card.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(ui::label("Output", colors).flex_1())
                                .child(self.wrap_toggle(("wrap-output", index), cx))
                                .child(
                                    ui::tap(("copy-output", index), "copy", colors)
                                        .aria_label("Copy output")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                                output.clone(),
                                            ));
                                            this.notify_user("Copied output", cx);
                                        })),
                                ),
                        )
                        .child(if wrap {
                            ui::mono(displayed_output.clone(), 12.)
                                .line_height(relative(1.5))
                                .into_any_element()
                        } else {
                            div()
                                .id(("output-lines", index))
                                .overflow_x_scroll()
                                .child(
                                    ui::mono(displayed_output.clone(), 12.)
                                        .line_height(relative(1.5))
                                        .whitespace_nowrap(),
                                )
                                .into_any_element()
                        })
                    })
                    .when(tool.output.is_empty(), |card| {
                        card.child(ui::hint(
                            if tool.finished {
                                "No output was recorded."
                            } else {
                                "Waiting for output…"
                            },
                            colors,
                        ))
                    })
            }))
            .when(
                stage.tools.is_empty() && stage.references.is_empty(),
                |body| body.child(ui::hint("No tool calls recorded for this stage.", colors)),
            )
    }

    /// The models to offer, with a detail: the computer's once a session
    /// reported them, the sample's otherwise.
    fn offered_models(&self) -> Vec<(String, String, String)> {
        if self.paused && !self.preview_models.is_empty() {
            return self
                .preview_models
                .iter()
                .map(|model| {
                    (
                        model.name.clone().unwrap_or_else(|| model.id.clone()),
                        model.provider.clone(),
                        model.id.clone(),
                    )
                })
                .collect();
        }
        match self.store.as_ref().and_then(|store| store.live.as_ref()) {
            Some(live) => live
                .models
                .iter()
                .map(|model| {
                    let name = model.name.clone().unwrap_or_else(|| model.id.clone());
                    (name, model.provider.clone(), model.id.clone())
                })
                .collect(),
            None => MODELS
                .iter()
                .map(|(name, detail)| (name.to_string(), detail.to_string(), name.to_string()))
                .collect(),
        }
    }

    fn live(&self) -> bool {
        self.store.as_ref().is_some_and(|store| !store.is_sample())
    }

    /// The newest lines first; warnings and errors in their colours.
    fn logs_sheet(&self, colors: &Theme, cx: &Context<Self>) -> Div {
        /// More would make the sheet slow to draw; Copy all takes everything.
        const SHOWN: usize = 300;
        let lines = gpui_android::recent_logs();
        let all = lines.join("\n");
        let count = lines.len();
        let wrap = self.wrap_lines(cx);
        div()
            .pb(px(8.))
            .child(ui::hint(
                format!(
                    "{count} lines since the app started, newest first. Hold one to select text."
                ),
                colors,
            ))
            .child(
                div()
                    .mt(px(12.))
                    .flex()
                    .gap(px(8.))
                    .child(
                        ui::button(
                            "copy-logs",
                            Button::Plain,
                            Some("copy"),
                            "Copy all",
                            true,
                            colors,
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.copy(all.clone(), "the log", cx)
                            }),
                        ),
                    )
                    .child(
                        ui::button(
                            "refresh-logs",
                            Button::Plain,
                            Some("refresh"),
                            "Refresh",
                            true,
                            colors,
                        )
                        .on_click(cx.listener(|_, _, _, cx| cx.notify())),
                    )
                    .child(div().flex_1())
                    .child(self.wrap_toggle("wrap-logs", cx)),
            )
            .when(lines.is_empty(), |body| {
                body.child(ui::hint("Nothing logged yet.", colors).mt(px(16.)))
            })
            .child(
                div()
                    .id("log-lines")
                    .mt(px(12.))
                    .when(!wrap, |lines| lines.overflow_x_scroll().whitespace_nowrap())
                    .flex()
                    .flex_col()
                    .font_family(MONO)
                    .text_size(px(11.5))
                    .line_height(px(16.))
                    .children(lines.into_iter().rev().take(SHOWN).enumerate().map(
                        |(index, line)| {
                            let level = line.split(' ').nth(1).unwrap_or("");
                            div()
                                .relative()
                                .py(px(4.))
                                .when(index > 0, |row| row.border_t_1().border_color(colors.line))
                                .text_color(match level {
                                    "E" => colors.coral,
                                    "W" => colors.amber,
                                    _ => colors.secondary,
                                })
                                .child(line.clone())
                                .child(self.copyable(line, cx))
                        },
                    )),
            )
    }

    pub(crate) fn computer_name(&self) -> String {
        self.store.as_ref().map_or_else(
            || "the computer".to_owned(),
            |store| store.computer.name.clone(),
        )
    }

    /// A sheet's title with a close button.
    fn sheet_title(
        &self,
        title: impl Into<SharedString>,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        div()
            .flex()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(22.))
                    .line_height(px(28.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(title.into()),
            )
            .child(
                ui::tap("close-sheet", "x", colors)
                    .mr(px(-12.))
                    .on_click(cx.listener(|this, _, _, cx| this.close_sheet(cx))),
            )
    }

    fn details_sheet(&self, id: SessionId, colors: &Theme, cx: &Context<Self>) -> Div {
        let Some(session) = self.session(id) else {
            return div();
        };
        let details = &session.details;
        let (model, thinking) = self.model_settings(cx);
        let meta = |text: String| {
            div()
                .text_size(px(12.5))
                .line_height(px(16.))
                .text_color(colors.muted)
                .child(text)
        };
        let row = |key: &'static str,
                   glyph: &'static str,
                   title: &'static str,
                   value: String,
                   open: Option<bool>| {
            let value_selector = format!("{key}-value");
            let value_probe = value_selector.clone();
            let chevron_selector = format!("{key}-chevron");
            let chevron_probe = chevron_selector.clone();
            div()
                .id(key)
                .debug_selector(move || key.to_owned())
                .relative()
                .child(crate::testing::probe(key))
                .min_h(px(56.))
                .flex()
                .items_center()
                .gap(px(12.))
                .border_t_1()
                .border_color(colors.line)
                .active(|style| style.bg(colors.selected))
                .child(icon(glyph, 20., colors.muted))
                .child(div().flex_1().min_w_0().truncate().child(title))
                .child(
                    meta(value)
                        .debug_selector(move || value_selector.clone())
                        .relative()
                        .child(crate::testing::probe(value_probe))
                        .min_w_0()
                        .truncate(),
                )
                .child(
                    div()
                        .debug_selector(move || chevron_selector.clone())
                        .relative()
                        .flex_none()
                        .child(crate::testing::probe(chevron_probe))
                        .child(icon(
                            if open == Some(true) {
                                "chev_d"
                            } else {
                                "chev_r"
                            },
                            16.,
                            colors.faint,
                        )),
                )
        };
        let disclosure = |key: &'static str| {
            cx.listener(move |this, _, _, cx| {
                if !this.expanded.remove(key) {
                    this.expanded.insert(key);
                }
                cx.notify();
            })
        };
        let session_file = details.session_file.clone();
        let files = session.files.len();
        let snapshots = details.snapshots;
        let runs_open = self.expanded.contains("details-run");
        let tools_open = self.expanded.contains("details-tools");
        let subagents = crate::model::subagent_details_summary(&session.turns);
        let history = div()
            .pb(px(16.))
            .children(session.turns.iter().enumerate().map(|(index, turn)| {
                div()
                    .id(("details-turn", index))
                    .mb(px(12.))
                    .child(
                        ui::label(format!("Turn {} · {}", index + 1, turn.at), colors).mb(px(8.)),
                    )
                    .child(self.turn_activity(id, index, turn, false, colors, cx))
            }))
            .when(session.turns.is_empty(), |history| {
                history.child(ui::hint("No turns have been reported yet.", colors))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        ui::mono(shorten(&details.session_file), 12.)
                            .flex_1()
                            .min_w_0()
                            .text_color(colors.muted),
                    )
                    .child(ui::tap("copy-file", "copy", colors).on_click(cx.listener(
                        move |this, _, _, cx| this.copy(session_file.clone(), "the path", cx),
                    ))),
            );
        // The run line, with a key that names the stages.
        let times = session.turn().map(|turn| turn.times).unwrap_or_default();
        let run = (times.iter().any(|time| !time.is_zero())).then(|| {
            let entries = StageKind::ALL
                .into_iter()
                .filter(|kind| !times[kind.index()].is_zero())
                .map(|kind| (kind, SharedString::from(kind.title())))
                .collect();
            div()
                .mt(px(20.))
                .child(ui::run_line(&times, false, colors))
                .child(ui::run_key(entries, &times, colors))
        });
        let stat = |label: &'static str, value: String| {
            div()
                .flex_1()
                .min_w_0()
                .py(px(16.))
                .child(ui::label(label, colors).text_color(colors.muted))
                .child(
                    div()
                        .mt(px(4.))
                        .text_size(px(22.))
                        .line_height(px(28.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate()
                        .child(value),
                )
        };
        let used = details.context_percent.unwrap_or(0).min(100) as f32 / 100.;
        div()
            .pb(px(8.))
            .children(run)
            .child(
                div()
                    .mt(px(20.))
                    .flex()
                    .border_t_1()
                    .border_b_1()
                    .border_color(colors.line)
                    .child(stat(
                        "Cost",
                        if details.cost.is_empty() {
                            "—".into()
                        } else {
                            details.cost.clone()
                        },
                    ))
                    .child(
                        stat("Turns", details.turns.to_string())
                            .pl(px(16.))
                            .border_l_1()
                            .border_color(colors.line),
                    )
                    .child(
                        stat(
                            "Changed",
                            match files {
                                1 => "1 file".into(),
                                count => format!("{count} files"),
                            },
                        )
                        .pl(px(16.))
                        .border_l_1()
                        .border_color(colors.line),
                    ),
            )
            .children(subagents.map(|summary| {
                row("details-subagents", "fork", "Subagents", summary, None).on_click(cx.listener(
                    move |this, _, window, cx| {
                        this.close_sheet(cx);
                        this.push(Route::Subagents(id), window, cx);
                    },
                ))
            }))
            .child(
                div()
                    .pt(px(20.))
                    .pb(px(24.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Context"),
                            )
                            .child(meta(details.context_tokens.clone())),
                    )
                    .child(
                        div()
                            .relative()
                            .mt(px(12.))
                            .h(px(8.))
                            .rounded(px(4.))
                            .bg(colors.raised)
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .bottom_0()
                                    .w(relative(used))
                                    .rounded(px(4.))
                                    .bg(colors.accent),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .left(relative(COMPACTS_AT))
                                    .top(px(-4.))
                                    .bottom(px(-4.))
                                    .w(px(2.))
                                    .rounded(px(1.))
                                    .bg(colors.line_strong),
                            ),
                    )
                    .child(
                        div()
                            .mt(px(8.))
                            .flex()
                            .child(
                                meta(details.context_percent.map_or_else(
                                    || "Usage not reported".into(),
                                    |percent| format!("{percent}% used"),
                                ))
                                .flex_1(),
                            )
                            .child(meta(format!(
                                "compacts at {}%",
                                (COMPACTS_AT * 100.) as u32
                            ))),
                    ),
            )
            .child(
                row(
                    "details-model",
                    "spark",
                    "Model",
                    format!("{model} · {thinking}"),
                    None,
                )
                .on_click(cx.listener(|this, _, _, cx| this.open_sheet(Sheet::Model, cx))),
            )
            .child(
                row(
                    "details-run",
                    "clock",
                    "Run history",
                    format!("{} turns", details.turns),
                    Some(runs_open),
                )
                .on_click(disclosure("details-run")),
            )
            .when(runs_open, |body| body.child(history))
            .child(
                row(
                    "details-tools",
                    "term",
                    "Tools used",
                    details.tools.len().to_string(),
                    Some(tools_open),
                )
                .on_click(disclosure("details-tools")),
            )
            .when(tools_open, |body| {
                body.child(div().pb(px(16.)).flex().flex_wrap().gap(px(8.)).children(
                    details.tools.iter().enumerate().map(|(index, tool)| {
                        ui::chip(("tool", index), None, tool.clone(), colors).font_family(MONO)
                    }),
                ))
            })
            .child(
                row(
                    "open-jj-history",
                    "restore",
                    "File history",
                    match snapshots {
                        0 => String::new(),
                        1 => "1 point".into(),
                        count => format!("{count} points"),
                    },
                    None,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.close_sheet(cx);
                    this.open_history(id, window, cx);
                })),
            )
    }

    fn attach_sheet(&self, target: Target, colors: &Theme, cx: &Context<Self>) -> Div {
        div().pb(px(8.)).child(
            ui::card(colors)
                .mt(px(8.))
                .child(
                    ui::row("choose-files", true, colors)
                        .child(icon("file", 20., colors.muted))
                        .child(ui::row_text(
                            "Choose files",
                            Some("Images, code and UTF-8 text files".into()),
                            colors,
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| this.attach_files(target, cx))),
                )
                .child(
                    ui::row("paste", false, colors)
                        .child(icon("copy", 20., colors.muted))
                        .child(ui::row_text(
                            "Paste",
                            Some("An image or text from the clipboard".into()),
                            colors,
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| this.paste_into(target, cx))),
                ),
        )
    }

    fn model_sheet(&self, colors: &Theme, cx: &Context<Self>) -> Div {
        let (selected_model, _) = self.model_settings(cx);
        let offered = self.offered_models();
        let unknown = offered.is_empty();
        let catalog = self.store.as_ref().and_then(|store| store.live.as_ref());
        let catalog_note = catalog.map(|live| {
            if live.models_loading { "Loading models from your computer…".to_owned() }
            else { live.models_error.clone().unwrap_or_else(|| "No models are configured yet. Sign in to a provider on the computer, then reload.".into()) }
        });
        let can_reload = catalog.is_some_and(|live| !live.models_loading);
        let query = self.model_search.read(cx).text().to_lowercase();
        let total = offered.len();
        let offered: Vec<_> = offered
            .into_iter()
            .filter(|(name, provider, id)| model_matches(&query, name, provider, id))
            .collect();
        let count = offered.len();
        let selected_identity = self
            .store
            .as_ref()
            .and_then(|store| store.live.as_ref())
            .and_then(|live| self.model_session().and_then(|id| live.current_model(id)))
            .map(|model| (model.provider.clone(), model.id.clone()))
            .or_else(|| {
                self.model_session()
                    .is_none()
                    .then(|| {
                        let prefs = self.prefs(cx);
                        prefs.model_provider.clone().zip(prefs.model_id.clone())
                    })
                    .flatten()
            });
        let models = offered
            .into_iter()
            .enumerate()
            .map(|(index, (name, detail, model_id))| {
                let on = selected_identity
                    .as_ref()
                    .map_or(selected_model == name, |(provider, id)| {
                        *provider == detail && *id == model_id
                    });
                let provider = detail.clone();
                ui::row(("model", index), index == 0, colors)
                    .debug_selector(move || format!("model-choice-{index}"))
                    .child(ui::row_text(
                        name.clone(),
                        Some(format!("{detail} · {model_id}").into()),
                        colors,
                    ))
                    .when(on, |row| row.child(icon("check", 20., colors.accent)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.choose_model(name.clone(), provider.clone(), model_id.clone(), cx);
                        this.close_sheet(cx);
                    }))
            });
        div()
            .flex()
            .flex_col()
            .min_h_0()
            .child(
                div()
                    .px(px(20.))
                    .flex_none()
                    .child(self.sheet_title("Choose model", colors, cx))
                    .child(ui::hint(
                        if self.model_session().is_some() {
                            "For prompts in this session."
                        } else {
                            "Default for new sessions."
                        },
                        colors,
                    ))
                    .child(
                        ui::card(colors).my(px(10.)).px(px(12.)).py(px(14.)).child(
                            div()
                                .relative()
                                .child(crate::testing::probe("model-search"))
                                .debug_selector(|| "model-search".into())
                                .child(self.model_search.clone()),
                        ),
                    )
                    .child(ui::hint(format!("{count} of {total} models"), colors).mb(px(8.))),
            )
            .child(
                crate::scroll::vertical("model-list", &self.sheet_scroll)
                    .min_h_0()
                    .px(px(20.))
                    .pb(px(8.))
                    .when(unknown, |list| {
                        list.child(ui::hint(
                            catalog_note.unwrap_or_else(|| "No models available.".into()),
                            colors,
                        ))
                        .when(can_reload, |list| {
                            list.child(
                                ui::button(
                                    "reload-models",
                                    Button::Plain,
                                    None,
                                    "Reload models",
                                    false,
                                    colors,
                                )
                                .my(px(12.))
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        if let Some(live) = this
                                            .store
                                            .as_mut()
                                            .and_then(|store| store.live.as_mut())
                                        {
                                            live.refresh_models();
                                        }
                                        cx.notify();
                                    },
                                )),
                            )
                        })
                    })
                    .when(!unknown && count == 0, |list| {
                        list.child(
                            ui::hint(
                                "No matching models. Try a model name, provider, or ID.",
                                colors,
                            )
                            .py(px(20.)),
                        )
                    })
                    .when(count > 0, |list| {
                        list.child(ui::card(colors).children(models))
                    }),
            )
    }

    fn thinking_sheet(&self, colors: &Theme, cx: &Context<Self>) -> Div {
        let (model, selected) = self.model_settings(cx);
        let levels = self
            .store
            .as_ref()
            .and_then(|store| store.live.as_ref())
            .and_then(|live| self.model_session().and_then(|id| live.thinking_levels(id)))
            .unwrap_or_else(|| THINKING.iter().map(|level| (*level).to_owned()).collect());
        div().pb(px(8.))
            .child(ui::hint(format!("Reasoning effort for {model}"), colors).mb(px(12.)))
            .child(ui::card(colors).children(levels.into_iter().enumerate().map(|(index, level)| {
                let detail = match level.as_str() {
                    "Off" => "No extra reasoning",
                    "Minimal" => "A little reasoning, quickest replies",
                    "Low" => "Fast, light reasoning",
                    "Medium" => "Balanced speed and depth",
                    "High" => "More thorough reasoning",
                    _ => "Deepest reasoning; takes longer",
                };
                ui::row(("thinking", index), index == 0, colors)
                    .debug_selector(move || format!("thinking-choice-{index}"))
                    .min_h(px(56.))
                    .child(ui::row_text(level.clone(), Some(detail.into()), colors))
                    .when(selected == level, |row| row.child(icon("check", 20., colors.accent)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.choose_thinking(&level, cx);
                        this.close_sheet(cx);
                    }))
            })))
            .child(ui::hint("Higher effort can use more tokens and take longer. Available levels depend on the model.", colors).mt(px(12.)))
    }

    fn more_sheet(&self, id: SessionId, colors: &Theme, cx: &Context<Self>) -> Div {
        let Some(session) = self.session(id) else {
            return div();
        };
        let prompt = session
            .turn()
            .map(|turn| turn.prompt.clone())
            .unwrap_or_default();
        let summary = session
            .turn()
            .and_then(|turn| turn.summary.as_ref())
            .map(|summary| summary.text());
        let reviewing = self.route() == Route::Review(id);
        let mut rows: Vec<(&'static str, &'static str, gpui::Hsla, Action)> = vec![(
            "copy",
            "Copy the prompt",
            colors.muted,
            Box::new(move |this, _, cx| this.copy(prompt.clone(), "the prompt", cx)),
        )];
        if let Some(summary) = summary {
            rows.push((
                "copy",
                "Copy the reply",
                colors.muted,
                Box::new(move |this, _, cx| this.copy(summary.clone(), "the reply", cx)),
            ));
        }
        if !session.files.is_empty() && !reviewing {
            rows.push((
                "diff",
                "Review changes",
                colors.muted,
                Box::new(move |this, window, cx| this.open_review(id, 0, window, cx)),
            ));
        }
        rows.push((
            "clock",
            "File history",
            colors.muted,
            Box::new(move |this, window, cx| this.open_history(id, window, cx)),
        ));
        rows.push((
            "info",
            if self.render_stats.visible {
                "Hide rendering stats"
            } else {
                "Show rendering stats"
            },
            colors.muted,
            Box::new(move |this, _, cx| {
                this.toggle_render_stats(cx);
                this.close_sheet(cx);
            }),
        ));
        if session.state.is_running() {
            rows.push((
                "stop",
                "Stop",
                colors.coral,
                Box::new(move |this, _, cx| this.stop(id, cx)),
            ));
        }
        rows.push((
            "trash",
            "Delete session…",
            colors.coral,
            Box::new(move |this, _, cx| this.open_sheet(Sheet::Delete(id), cx)),
        ));
        div().pb(px(8.)).child(
            ui::card(colors)
                .mt(px(8.))
                .children(rows.into_iter().enumerate().map(
                    |(index, (glyph, text, color, action))| {
                        ui::row(("action", index), index == 0, colors)
                            .child(icon(glyph, 20., color))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(15.))
                                    .when(glyph == "stop", |text| text.text_color(colors.coral))
                                    .child(text),
                            )
                            .on_click(
                                cx.listener(move |this, _, window, cx| action(this, window, cx)),
                            )
                    },
                )),
        )
    }

    fn models_sheet(&self, colors: &Theme, _: &Context<Self>) -> Div {
        let computer = self.computer_name();
        div()
            .pb(px(8.))
            .child(ui::card(colors).mt(px(8.)).children(self.offered_models().into_iter().enumerate().map(
                |(index, (name, detail, _))| {
                    ui::row(("known-model", index), index == 0, colors)
                        .child(icon("spark", 20., colors.muted))
                        .child(ui::row_text(name, Some(detail.into()), colors))
                },
            )))
            .child(
                ui::hint(
                    if self.live() {
                        format!("Models available on {computer}, loaded before starting a session. Sign-ins stay on {computer}.")
                    } else {
                        format!("A sample list. Sign-ins stay on {computer}: change them in Pi Desktop or Pi’s settings there.")
                    },
                    colors,
                )
                .mt(px(14.))
                .mx(px(4.)),
            )
    }

    fn resources_sheet(&self, colors: &Theme, cx: &Context<Self>) -> Div {
        let live = self.live();
        let commands = if live {
            self.command_catalog_for_project(self.project, cx)
                .unwrap_or_default()
        } else {
            crate::composer::sample_commands()
        };
        let computer = self.computer_name();
        div()
            .pb(px(8.))
            .when(!commands.is_empty(), |sheet| {
                sheet.child(ui::card(colors).mt(px(8.)).children(
                    commands.into_iter().enumerate().map(|(index, command)| {
                        let kind = if command.name.starts_with("skill:") {
                            "Skill"
                        } else {
                            "Prompt"
                        };
                        let detail = match command.description {
                            Some(description) if !description.is_empty() => {
                                format!("{kind} · {description}")
                            }
                            _ => kind.to_owned(),
                        };
                        ui::row(("resource", index), index == 0, colors)
                            .child(crate::testing::probe(format!("command-{}", command.name)))
                            .child(icon("slash", 20., colors.muted))
                            .child(ui::row_text(
                                format!("/{}", command.name),
                                Some(detail.into()),
                                colors,
                            ))
                    }),
                ))
            })
            .child(
                ui::hint(
                    if live {
                        format!("Prompt templates and skills from ~/.pi/agent on {computer}, and from a project Pi trusts. Durable sessions don't load Pi's extensions yet.")
                    } else {
                        "A sample list. The real one comes from Pi on the computer.".to_owned()
                    },
                    colors,
                )
                .mt(px(14.))
                .mx(px(4.)),
            )
    }
}

/// "~/.pi/…/qwen-signatures.jsonl": the start and the file name.
fn shorten(path: &str) -> String {
    match (path.find('/'), path.rfind('/')) {
        (Some(first), Some(last)) if last > first => {
            let start = path[first + 1..]
                .find('/')
                .map_or(first, |next| first + 1 + next);
            if start >= last {
                path.to_owned()
            } else {
                format!("{}/…{}", &path[..start], &path[last..])
            }
        }
        _ => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{activity_stage, shorten};
    use crate::model::{Stage, StageKind, StageStatus, Turn};

    #[test]
    fn an_open_activity_stays_with_its_stage_when_stages_reorder() {
        let mut turn = Turn::new("prompt", "now");
        turn.stages = vec![
            Stage::new(StageKind::Change, StageStatus::Live, "Changing"),
            Stage::new(StageKind::HandOff, StageStatus::Planned, "Reporting"),
        ];
        assert_eq!(
            activity_stage(&turn, StageKind::Change).unwrap().what,
            "Changing"
        );
        turn.stages.insert(
            0,
            Stage::new(StageKind::Understand, StageStatus::Done, "Understood"),
        );
        assert_eq!(
            activity_stage(&turn, StageKind::Change).unwrap().what,
            "Changing"
        );
    }

    #[test]
    fn long_paths_keep_their_start_and_file() {
        assert_eq!(
            shorten("~/.pi/agent/sessions/--repos-pi--/qwen-signatures.jsonl"),
            "~/.pi/…/qwen-signatures.jsonl"
        );
        assert_eq!(shorten("~/.pi/x.jsonl"), "~/.pi/x.jsonl");
        assert_eq!(shorten("x.jsonl"), "x.jsonl");
    }
}
