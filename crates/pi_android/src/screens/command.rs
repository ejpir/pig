//! A running shell call as a full-screen, live command/output view.

use super::scroll_area;
use crate::{
    app::{PhoneApp, Route},
    model::{SessionId, ToolActivity},
    theme::{MONO, theme},
    ui::{self, icon},
};
use gpui::{
    AnyElement, Context, FontWeight, ListAlignment, ListState, Role, ScrollHandle, SharedString,
    Window, div, list, prelude::*, px,
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CommandTab {
    Command,
    #[default]
    Output,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CommandOutputSource {
    len: usize,
    hash: u64,
    sample_hash: u64,
    pointer: usize,
    ends_with_newline: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct CommandScreenState {
    pub(crate) tool_id: String,
    pub(crate) tab: CommandTab,
    pub(crate) command_scroll: ScrollHandle,
    pub(crate) output_list: ListState,
    pub(crate) output_horizontal: ScrollHandle,
    output_source: Option<CommandOutputSource>,
    output_width: f32,
    output_wrap: bool,
    output_rows: Arc<Vec<SharedString>>,
}

impl CommandScreenState {
    fn new(tool_id: String) -> Self {
        let output_list =
            ListState::new(0, ListAlignment::Top, px(160.)).with_uniform_item_height(px(20.));
        output_list.set_follow_mode(gpui::FollowMode::Tail);
        Self {
            tool_id,
            tab: CommandTab::Output,
            command_scroll: ScrollHandle::new(),
            output_list,
            output_horizontal: ScrollHandle::new(),
            output_source: None,
            output_width: 0.,
            output_wrap: false,
            output_rows: Arc::new(Vec::new()),
        }
    }
}

fn output_line_width(text: &str) -> f32 {
    let units = text
        .chars()
        .map(|character| match character {
            '\t' => 4.,
            character if character.is_control() => 0.,
            character if character.is_ascii() => 1.,
            _ => 2.,
        })
        .sum::<f32>();
    40. + units * 12. * 0.6
}

const OUTPUT_HASH_OFFSET: u64 = 0xcbf29ce484222325;
const OUTPUT_HASH_PRIME: u64 = 0x100000001b3;

fn extend_output_hash(mut hash: u64, text: &str) -> u64 {
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(OUTPUT_HASH_PRIME);
    }
    hash
}

fn output_fingerprint(text: &str) -> u64 {
    let bytes = text.as_bytes();
    if bytes.len() <= 512 {
        return extend_output_hash(OUTPUT_HASH_OFFSET, text);
    }
    let mut hash = OUTPUT_HASH_OFFSET;
    for sample in 0..16 {
        let start = sample * (bytes.len() - 16) / 15;
        for byte in &bytes[start..start + 16] {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(OUTPUT_HASH_PRIME);
        }
    }
    hash
}

fn command_output_source(
    output: &str,
    previous: Option<&CommandOutputSource>,
) -> CommandOutputSource {
    let pointer = output.as_ptr() as usize;
    let sample_hash = output_fingerprint(output);
    let hash = previous
        .and_then(|old| {
            if old.len == output.len() && old.pointer == pointer && old.sample_hash == sample_hash {
                Some(old.hash)
            } else if old.len < output.len()
                && output
                    .get(..old.len)
                    .is_some_and(|prefix| output_fingerprint(prefix) == old.sample_hash)
            {
                output
                    .get(old.len..)
                    .map(|suffix| extend_output_hash(old.hash, suffix))
            } else {
                None
            }
        })
        .unwrap_or_else(|| extend_output_hash(OUTPUT_HASH_OFFSET, output));
    CommandOutputSource {
        len: output.len(),
        hash,
        sample_hash,
        pointer,
        ends_with_newline: output.ends_with('\n'),
    }
}

fn command_suffix_for_display(raw: &str) -> std::borrow::Cow<'_, str> {
    if raw.contains('\n') || !raw.contains('\r') {
        return pi_core::command_output::for_display(raw);
    }
    let before_pending = raw.strip_suffix('\r').unwrap_or(raw);
    let start = before_pending.rfind('\r').map_or(0, |index| index + 1);
    std::borrow::Cow::Borrowed(&before_pending[start..])
}

fn replace_command_output_rows(
    view: &mut CommandScreenState,
    tool: &ToolActivity,
    old: &CommandOutputSource,
    new: &CommandOutputSource,
) -> bool {
    if new.len <= old.len
        || tool
            .output
            .get(old.len..)
            .is_none_or(|suffix| extend_output_hash(old.hash, suffix) != new.hash)
    {
        return false;
    }
    let old_count = view.output_rows.len();
    if old_count == 0 {
        return false;
    }
    let (replace_start, suffix_start) = if old.ends_with_newline {
        (old_count, old.len)
    } else {
        (
            old_count - 1,
            tool.output[..old.len]
                .rfind('\n')
                .map_or(0, |position| position + 1),
        )
    };
    let raw_suffix = &tool.output[suffix_start..];
    let displayed = if tool.name == "bash" {
        command_suffix_for_display(raw_suffix)
    } else {
        std::borrow::Cow::Borrowed(raw_suffix)
    };
    let replacement: Vec<SharedString> = displayed.lines().map(SharedString::from).collect();
    if replacement.is_empty() {
        return false;
    }

    let removed_width = view.output_rows[replace_start..]
        .iter()
        .map(|line| output_line_width(line))
        .fold(0., f32::max);
    let replacement_width = replacement
        .iter()
        .map(|line| output_line_width(line))
        .fold(0., f32::max);
    let rows = Arc::make_mut(&mut view.output_rows);
    rows.splice(replace_start.., replacement);
    if replacement_width >= view.output_width {
        view.output_width = replacement_width;
    } else if removed_width >= view.output_width {
        view.output_width = rows
            .iter()
            .map(|line| output_line_width(line))
            .fold(0., f32::max);
    }
    view.output_list
        .splice(replace_start..old_count, rows.len() - replace_start);
    true
}

fn reset_command_output(view: &mut CommandScreenState, tool: &ToolActivity) {
    let displayed = tool.output_for_display();
    let rows: Vec<SharedString> = if displayed.trim().is_empty() {
        Vec::new()
    } else {
        displayed.lines().map(SharedString::from).collect()
    };
    view.output_width = rows
        .iter()
        .map(|line| output_line_width(line))
        .fold(0., f32::max);
    view.output_list
        .reset_with_uniform_height(rows.len(), px(20.));
    view.output_rows = Arc::new(rows);
}

impl PhoneApp {
    pub(crate) fn open_command_detail(
        &mut self,
        id: SessionId,
        tool_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let matches = self
            .session(id)
            .into_iter()
            .flat_map(|session| &session.turns)
            .flat_map(|turn| &turn.stages)
            .flat_map(|stage| &stage.tools)
            .filter(|tool| tool.id == tool_id)
            .count();
        if matches != 1 {
            return;
        }
        let token = self.next_command_view;
        self.next_command_view = self.next_command_view.wrapping_add(1).max(1);
        self.command_views
            .insert(token, CommandScreenState::new(tool_id));
        self.push(Route::Command(id, token), window, cx);
    }

    /// Resolve a command by stable tool identity. A duplicate or vanished ID
    /// is never allowed to silently bind this screen to another call.
    pub(crate) fn bound_command(&self, id: SessionId, token: u64) -> Option<&ToolActivity> {
        let expected = &self.command_views.get(&token)?.tool_id;
        let mut found = None;
        for tool in self
            .session(id)?
            .turns
            .iter()
            .flat_map(|turn| &turn.stages)
            .flat_map(|stage| &stage.tools)
        {
            if &tool.id == expected {
                if found.is_some() {
                    return None;
                }
                found = Some(tool);
            }
        }
        found
    }

    fn command_tab(&mut self, token: u64, tab: CommandTab, cx: &mut Context<Self>) {
        let Some(view) = self.command_views.get_mut(&token) else {
            return;
        };
        view.tab = tab;
        if tab == CommandTab::Output && view.output_list.is_following_tail() {
            view.output_list.scroll_to_end();
        }
        cx.notify();
    }

    fn command_tabs(
        &self,
        token: u64,
        active: CommandTab,
        colors: &crate::theme::Theme,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_none()
            .px(px(20.))
            .border_b_1()
            .border_color(colors.line)
            .children(
                [
                    (CommandTab::Command, "Command", "command-tab-command"),
                    (CommandTab::Output, "Output", "command-tab-output"),
                ]
                .into_iter()
                .map(|(tab, label, selector)| {
                    let selected = active == tab;
                    div()
                        .id(selector)
                        .debug_selector(move || selector.into())
                        .role(Role::Button)
                        .aria_label(label)
                        .flex_1()
                        .h(px(48.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .border_b(px(2.))
                        .border_color(if selected {
                            colors.accent
                        } else {
                            gpui::transparent_black()
                        })
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if selected {
                            colors.accent
                        } else {
                            colors.muted
                        })
                        .active(|style| style.bg(colors.selected))
                        .child(label)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.command_tab(token, tab, cx)),
                        )
                }),
            )
    }

    pub(crate) fn command_screen(
        &mut self,
        id: SessionId,
        token: u64,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = theme(cx);
        let back = ui::tap("command-back", "back", &colors).on_click(cx.listener(
            |this, _, window, cx| {
                this.back(window, cx);
            },
        ));
        let Some(tool) = self.bound_command(id, token).cloned() else {
            return div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(ui::appbar(back, "Command", None, &colors))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(10.))
                        .px(px(24.))
                        .child(icon("info", 16., colors.muted))
                        .child(ui::hint("Command no longer available.", &colors)),
                )
                .into_any_element();
        };
        let wrap = self.wrap_lines(cx);
        let Some(view) = self.command_views.get_mut(&token) else {
            return div().into_any_element();
        };
        let output_source = command_output_source(&tool.output, view.output_source.as_ref());
        if view.output_source.as_ref() != Some(&output_source) {
            let incremented = view
                .output_source
                .clone()
                .is_some_and(|old| replace_command_output_rows(view, &tool, &old, &output_source));
            if !incremented {
                reset_command_output(view, &tool);
            }
            view.output_source = Some(output_source);
        }
        if view.output_wrap != wrap {
            view.output_list.remeasure();
            view.output_wrap = wrap;
        }
        let tab = view.tab;
        let command_scroll = view.command_scroll.clone();
        let output_list = view.output_list.clone();
        let output_horizontal = view.output_horizontal.clone();
        let output_rows = view.output_rows.clone();
        let output_width = view.output_width;
        let output_follow = output_list.is_following_tail();
        if tab == CommandTab::Output && output_follow {
            output_list.scroll_to_end();
        }

        let status: SharedString = if !tool.finished {
            "Running".into()
        } else if tool.failed {
            super::stream::exit_code(&tool.output).map_or_else(
                || "Failed".into(),
                |code| format!("Failed · exit {code}").into(),
            )
        } else {
            "Done".into()
        };
        let appbar = ui::appbar(back, "Command", Some(status), &colors);
        let tabs = self.command_tabs(token, tab, &colors, cx);

        let body = match tab {
            CommandTab::Command => {
                scroll_area(("command-command", token as usize), &command_scroll)
                    .child(
                        div().p(px(20.)).child(
                            div()
                                .id("command-command-text")
                                .relative()
                                .p(px(16.))
                                .rounded(px(12.))
                                .bg(colors.panel)
                                .when(!wrap, |command| command.overflow_x_scroll())
                                .child(
                                    ui::mono(format!("$ {}", tool.target), 13.)
                                        .line_height(px(20.))
                                        .when(!wrap, |command| command.whitespace_nowrap()),
                                )
                                .child(self.copyable(tool.target.clone(), cx)),
                        ),
                    )
                    .into_any_element()
            }
            CommandTab::Output => {
                let copy = tool.output.clone();
                let row_count = output_rows.len();
                let row_colors = colors;
                let row_scroll = output_horizontal.clone();
                let rows: Arc<Vec<SharedString>> = output_rows.clone();
                let output = list(
                    output_list.clone(),
                    cx.processor(move |this, index: usize, _, cx| {
                        let text = rows[index].clone();
                        let line = div()
                            .relative()
                            .min_h(px(20.))
                            .px(px(20.))
                            .when(index == 0, |line| line.pt(px(16.)))
                            .when(index + 1 == row_count, |line| line.pb(px(16.)))
                            .font_family(MONO)
                            .text_size(px(12.))
                            .line_height(px(18.))
                            .when(!wrap, |line| {
                                line.min_w(px(output_width)).whitespace_nowrap()
                            })
                            .text_color(row_colors.plain)
                            .child(super::stream::output_block(text.clone(), &row_colors))
                            .child(this.copyable(text, cx));
                        if wrap {
                            line.into_any_element()
                        } else {
                            super::shared_horizontal_pan(
                                ("command-output-pan", index),
                                row_scroll.clone(),
                                px(output_width),
                                div()
                                    .flex_none()
                                    .min_w(px(output_width))
                                    .child(line)
                                    .into_any_element(),
                                cx,
                            )
                            .debug_selector(move || format!("command-output-pan-{index}"))
                            .w_full()
                            .min_w_0()
                            .max_w_full()
                            .into_any_element()
                        }
                    }),
                )
                .size_full();
                let output_body = if row_count == 0 {
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .px(px(20.))
                        .child(ui::hint(
                            if tool.finished {
                                "No output was recorded."
                            } else {
                                "Waiting for output…"
                            },
                            &colors,
                        ))
                        .into_any_element()
                } else {
                    div()
                        .id("command-output")
                        .debug_selector(|| "command-output".into())
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .child(
                            crate::scroll::virtual_list(
                                ("command-output", token as usize),
                                &output_list,
                            )
                            .flex_1()
                            .min_h_0()
                            .bg(colors.canvas)
                            .child(output),
                        )
                        .into_any_element()
                };
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex_none()
                            .h(px(48.))
                            .px(px(20.))
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(12.5))
                                    .text_color(colors.muted)
                                    .child(if output_follow {
                                        "Following live output"
                                    } else {
                                        "Live output paused"
                                    }),
                            )
                            .child(self.wrap_toggle("wrap-command-output", cx))
                            .child(
                                ui::tap("copy-command-output", "copy", &colors)
                                    .aria_label("Copy output")
                                    .when(copy.is_empty(), |button| button.opacity(0.5))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if !copy.is_empty() {
                                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                                copy.clone(),
                                            ));
                                            this.notify_user("Copied output", cx);
                                        }
                                    })),
                            ),
                    )
                    .child(output_body)
                    .into_any_element()
            }
        };

        div()
            .debug_selector(|| "command-screen".into())
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(appbar)
            .child(tabs)
            .child(body)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StageKind;
    use gpui::TestAppContext;

    fn running_bash(app: &PhoneApp) -> (usize, StageKind, usize, String) {
        app.session(SessionId(1))
            .unwrap()
            .turns
            .iter()
            .enumerate()
            .flat_map(|(turn_index, turn)| {
                turn.stages.iter().flat_map(move |stage| {
                    stage
                        .tools
                        .iter()
                        .enumerate()
                        .map(move |(tool_index, tool)| (turn_index, stage.kind, tool_index, tool))
                })
            })
            .find(|(_, _, _, tool)| tool.name == "bash" && !tool.finished)
            .map(|(turn, kind, tool, activity)| (turn, kind, tool, activity.id.clone()))
            .expect("the verifying fixture has a running shell command")
    }

    #[gpui::test]
    fn running_bash_opens_a_two_tab_full_screen(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(px(384.), px(854.)));
        cx.update(|window, cx| app.update(cx, |app, cx| app.preview("verifying", window, cx)));
        cx.run_until_parked();
        let running = cx
            .debug_bounds("call")
            .expect("the running command is visible");
        cx.simulate_click(running.center(), gpui::Modifiers::none());
        cx.run_until_parked();

        let token = app.read_with(cx, |app, _| match app.route() {
            Route::Command(SessionId(1), token) => token,
            route => panic!("expected command screen, got {route:?}"),
        });
        assert!(app.read_with(cx, |app, _| {
            app.command_views[&token].output_list.is_following_tail()
        }));
        assert!(cx.debug_bounds("command-screen").is_some());
        for selector in ["command-tab-command", "command-tab-output"] {
            assert!(cx.debug_bounds(selector).unwrap().size.height >= px(48.));
        }
        assert!(cx.debug_bounds("command-output").is_some());

        let command_tab = cx.debug_bounds("command-tab-command").unwrap();
        cx.simulate_click(command_tab.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(app.command_views[&token].tab, CommandTab::Command)
        });
    }

    #[gpui::test]
    fn live_output_follows_until_the_reader_scrolls_away(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(px(384.), px(640.)));
        let (token, tool_id) = cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("verifying", window, cx);
                app.update_prefs(cx, |prefs| prefs.wrap_lines = false);
                let (turn, kind, tool, tool_id) = running_bash(app);
                let activity = &mut app
                    .store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|session| session.id == SessionId(1))
                    .unwrap()
                    .turns[turn]
                    .stages[kind.index()]
                .tools[tool];
                activity.output = (0..120)
                    .map(|line| format!("streamed output line {line} {}", "wide-".repeat(40)))
                    .collect::<Vec<_>>()
                    .join("\n");
                app.open_call_detail(SessionId(1), turn, kind, tool, window, cx);
                let Route::Command(_, token) = app.route() else {
                    panic!("command route");
                };
                (token, tool_id)
            })
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            let view = &app.command_views[&token];
            assert!(view.output_list.max_offset_for_scrollbar().y > px(0.));
            assert!(view.output_list.logical_scroll_top().item_ix > 0);
            assert!(view.output_list.is_following_tail());
        });

        let pan = cx.debug_bounds("command-output-pan-119").unwrap();
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: pan.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(-120.), px(0.))),
            modifiers: gpui::Modifiers::none(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: pan.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(0.))),
            modifiers: gpui::Modifiers::none(),
            touch_phase: gpui::TouchPhase::Ended,
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.command_views[&token].output_horizontal.offset().x < px(0.));
        });
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: pan.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(500.))),
            modifiers: gpui::Modifiers::none(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.run_until_parked();
        let away = app.read_with(cx, |app, _| {
            let view = &app.command_views[&token];
            assert!(!view.output_list.is_following_tail());
            view.output_list.logical_scroll_top()
        });

        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                let tool = app
                    .store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .flat_map(|session| &mut session.turns)
                    .flat_map(|turn| &mut turn.stages)
                    .flat_map(|stage| &mut stage.tools)
                    .find(|tool| tool.id == tool_id)
                    .unwrap();
                tool.output.push_str("\na newly streamed line");
                cx.notify();
            })
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            let view = &app.command_views[&token];
            let current = view.output_list.logical_scroll_top();
            assert_eq!(current.item_ix, away.item_ix);
            assert_eq!(current.offset_in_item, away.offset_in_item);
            assert!(!view.output_list.is_following_tail());
        });

        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                let view = app.command_views.get_mut(&token).unwrap();
                view.output_list.set_follow_mode(gpui::FollowMode::Tail);
                view.output_list.scroll_to_end();
                cx.notify();
            })
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            let view = &app.command_views[&token];
            assert!(view.output_list.logical_scroll_top().item_ix > 0);
            assert!(view.output_list.is_following_tail());
        });
    }

    #[gpui::test]
    fn unwrapped_live_output_pans_horizontally_and_keeps_its_position(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(gpui::size(px(384.), px(640.)));
        let (token, tool_id) = cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("verifying", window, cx);
                app.update_prefs(cx, |prefs| prefs.wrap_lines = false);
                let (turn, kind, tool, tool_id) = running_bash(app);
                app.store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|session| session.id == SessionId(1))
                    .unwrap()
                    .turns[turn]
                    .stages[kind.index()]
                .tools[tool]
                    .output = [
                    format!("first {}", "wide-output-".repeat(30)),
                    "short second line".into(),
                ]
                .join("\n");
                app.open_call_detail(SessionId(1), turn, kind, tool, window, cx);
                let Route::Command(_, token) = app.route() else {
                    panic!("command route");
                };
                (token, tool_id)
            })
        });
        cx.run_until_parked();

        let first = cx.debug_bounds("command-output-pan-0").unwrap();
        app.read_with(cx, |app, _| {
            assert!(app.command_views[&token].output_width > 384.);
        });
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: first.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(-200.), px(0.))),
            modifiers: gpui::Modifiers::none(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: first.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(0.))),
            modifiers: gpui::Modifiers::none(),
            touch_phase: gpui::TouchPhase::Ended,
        });
        cx.run_until_parked();
        let first_pan = app.read_with(cx, |app, _| {
            let offset = app.command_views[&token].output_horizontal.offset().x;
            assert!(offset < px(0.));
            offset
        });
        let second = cx.debug_bounds("command-output-pan-1").unwrap();
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: second.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(-200.), px(0.))),
            modifiers: gpui::Modifiers::none(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: second.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(0.))),
            modifiers: gpui::Modifiers::none(),
            touch_phase: gpui::TouchPhase::Ended,
        });
        cx.run_until_parked();
        let panned = app.read_with(cx, |app, _| {
            let offset = app.command_views[&token].output_horizontal.offset().x;
            assert!(offset < first_pan, "a second gesture must continue panning");
            offset
        });
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: first.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(100.), px(0.))),
            modifiers: gpui::Modifiers::none(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.run_until_parked();
        let reversed = app.read_with(cx, |app, _| {
            let offset = app.command_views[&token].output_horizontal.offset().x;
            assert!(offset > panned);
            offset
        });

        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .flat_map(|session| &mut session.turns)
                    .flat_map(|turn| &mut turn.stages)
                    .flat_map(|stage| &mut stage.tools)
                    .find(|tool| tool.id == tool_id)
                    .unwrap()
                    .output
                    .push_str("\nshort streamed line");
                cx.notify();
            })
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.command_views[&token].output_horizontal.offset().x,
                reversed
            );
        });

        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .flat_map(|session| &mut session.turns)
                    .flat_map(|turn| &mut turn.stages)
                    .flat_map(|stage| &mut stage.tools)
                    .find(|tool| tool.id == tool_id)
                    .unwrap()
                    .output
                    .replace_range(..1, "X");
                cx.notify();
            })
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(app.command_views[&token].output_rows[0].starts_with('X'));
        });

        cx.simulate_resize(gpui::size(px(4096.), px(640.)));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.command_views[&token].output_horizontal.offset().x,
                px(0.),
                "a wider viewport must clamp a retained pan without another gesture"
            );
        });
    }

    #[gpui::test]
    fn command_binding_uses_the_stable_unique_tool_id(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        let (token, expected) = cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("verifying", window, cx);
                let (turn, kind, tool, expected) = running_bash(app);
                app.open_call_detail(SessionId(1), turn, kind, tool, window, cx);
                let Route::Command(_, token) = app.route() else {
                    panic!("command route");
                };
                (token, expected)
            })
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.bound_command(SessionId(1), token).unwrap().id, expected)
        });

        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                let session = app
                    .store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|session| session.id == SessionId(1))
                    .unwrap();
                let tools = &mut session.turns[0].stages[StageKind::Verify.index()].tools;
                tools.reverse();
                cx.notify();
            })
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.bound_command(SessionId(1), token).unwrap().id, expected)
        });

        cx.update(|_, cx| {
            app.update(cx, |app, _| {
                let session = app
                    .store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|session| session.id == SessionId(1))
                    .unwrap();
                let tools = &mut session.turns[0].stages[StageKind::Verify.index()].tools;
                let duplicate = tools
                    .iter()
                    .find(|tool| tool.id == expected)
                    .unwrap()
                    .clone();
                tools.push(duplicate);
            })
        });
        app.read_with(cx, |app, _| {
            assert!(app.bound_command(SessionId(1), token).is_none())
        });
    }

    #[gpui::test]
    fn finished_bash_keeps_the_existing_call_sheet(cx: &mut TestAppContext) {
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("tool-output", window, cx);
                app.close_sheet(cx);
                let session = app.session(SessionId(1)).unwrap();
                let (tool_index, _) = session.turns[0].stages[StageKind::Verify.index()]
                    .tools
                    .iter()
                    .enumerate()
                    .find(|(_, tool)| tool.id == "test-command")
                    .unwrap();
                app.open_call_detail(SessionId(1), 0, StageKind::Verify, tool_index, window, cx);
            })
        });
        app.read_with(cx, |app, _| {
            assert_eq!(app.route(), Route::Thread(SessionId(1)));
            assert!(matches!(app.sheet, Some(crate::app::Sheet::Call(..))));
        });
    }
}
