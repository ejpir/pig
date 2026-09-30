//! Turn bars (design study 05, 07): while ⌥ (Alt) is held, lines in the file tab
//! get a bar for the turn that last wrote them, from jj's line annotation, and
//! ⌥-click shows a line's turn. Nothing shows otherwise, so the editor looks as
//! it always does. `jj.turnBars` set to `never` turns this off.
use super::super::session::SessionController;
use super::*;
use gpui::{Hsla, Pixels, Point, Task, WeakEntity};
use pi_jj::ChangeId;

/// Gutter marker types, one per color.
struct ThisSession;
struct OtherSessions;

fn this_session(cx: &App) -> Hsla {
    theme(cx).accent
}

fn other_sessions(cx: &App) -> Hsla {
    theme(cx).steel
}

#[derive(Default)]
pub(super) struct TurnBars {
    session: Option<WeakEntity<SessionController>>,
    /// ⌥ is held.
    held: bool,
    /// The annotated file and which turn wrote each line.
    turns: Option<(PathBuf, pi_jj::LineTurns)>,
    load: Option<Task<()>>,
    card: Option<TurnCard>,
}

#[derive(Clone)]
struct TurnCard {
    short: String,
    /// This session's record, which Show in thread and Diff open.
    record: Option<usize>,
    who: String,
    prompt: String,
    lines: usize,
    at: Point<Pixels>,
}

impl FilesView {
    /// The session whose turns the bars tell apart from other sessions'.
    pub fn set_session(&mut self, session: WeakEntity<SessionController>) {
        self.bars.session = Some(session);
    }

    pub(super) fn alt_changed(&mut self, alt: bool, cx: &mut Context<Self>) {
        if alt == self.bars.held {
            return;
        }
        self.bars.held = alt;
        if alt {
            self.load_bars(cx);
        } else {
            self.hide_bars(cx);
        }
        cx.notify();
    }

    /// The file tab's footer says the bars exist, and while ⌥ is held what
    /// they mean. Only for a session with jj, and unless `jj.turnBars` is `never`.
    pub(super) fn turn_hint(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let session = self.bars.session.as_ref()?.upgrade()?;
        if session.read(cx).jj().project.is_none()
            || crate::prefs::choice(cx, "jj.turnBars", Some(&self.root)) != "alt"
        {
            return None;
        }
        let key = if cfg!(target_os = "macos") {
            "⌥"
        } else {
            "Alt"
        };
        let theme = theme(cx);
        if !self.bars.held {
            return Some(
                div()
                    .debug_selector(|| "turn-hint".into())
                    .child(format!("{key} shows which turn wrote each line"))
                    .into_any_element(),
            );
        }
        let bar = |color| div().w(px(3.)).h(px(12.)).bg(color);
        Some(
            h_flex()
                .debug_selector(|| "turn-legend".into())
                .gap(px(6.))
                .child(bar(this_session(cx)))
                .child("this session")
                .child(bar(other_sessions(cx)).ml(px(6.)))
                .child("other sessions")
                .child(div().ml(px(6.)).child(format!(
                    "no bar: you, or before jj · {key}-click a line for its turn"
                )))
                .text_color(theme.faint)
                .into_any_element(),
        )
    }

    fn load_bars(&mut self, cx: &mut Context<Self>) {
        if crate::prefs::choice(cx, "jj.turnBars", Some(&self.root)) != "alt" {
            return;
        }
        let Some(session) = self.bars.session.as_ref().and_then(WeakEntity::upgrade) else {
            return;
        };
        let Some(tab) = self.tab() else {
            return;
        };
        let path = tab.path.clone();
        let text = tab.buffer.read(cx).text();
        let session = session.read(cx);
        let Some(relative) = session
            .jj()
            .root
            .as_ref()
            .and_then(|root| path.strip_prefix(root).ok())
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        else {
            return;
        };
        let Some(task) = session.jj_call(move |project| project.annotate(&relative, &text), cx)
        else {
            return;
        };
        self.bars.load = Some(cx.spawn(async move |this, cx| {
            // A file jj does not know, or a failed annotation, shows no bars.
            let turns = task.await.unwrap_or_default();
            this.update(cx, |this, cx| {
                this.bars.turns = Some((path, turns));
                if this.bars.held {
                    this.paint_bars(cx);
                }
            })
            .ok();
        }));
    }

    fn paint_bars(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.tab().and_then(|tab| tab.editor.clone()) else {
            return;
        };
        let Some((path, turns)) = &self.bars.turns else {
            return;
        };
        if self.tab().map(|tab| &tab.path) != Some(path) {
            return;
        }
        let ours = self.session_turns(cx);
        let mut mine: Vec<std::ops::Range<u32>> = Vec::new();
        let mut others: Vec<std::ops::Range<u32>> = Vec::new();
        for (row, turn) in turns.lines.iter().enumerate() {
            let Some(turn) = turn else { continue };
            let rows = if ours.iter().any(|(change, _)| change == turn) {
                &mut mine
            } else {
                &mut others
            };
            let row = row as u32;
            match rows.last_mut() {
                Some(last) if last.end == row => last.end = row + 1,
                _ => rows.push(row..row + 1),
            }
        }
        pi_editor::mark_rows::<ThisSession>(&editor, &mine, this_session, cx);
        pi_editor::mark_rows::<OtherSessions>(&editor, &others, other_sessions, cx);
    }

    fn hide_bars(&mut self, cx: &mut Context<Self>) {
        for editor in self.tabs.iter().filter_map(|tab| tab.editor.as_ref()) {
            pi_editor::unmark_rows::<ThisSession>(editor, cx);
            pi_editor::unmark_rows::<OtherSessions>(editor, cx);
        }
        self.bars.load = None;
        self.bars.turns = None;
        self.bars.card = None;
        cx.notify();
    }

    /// This session's turns: change and record index.
    fn session_turns(&self, cx: &App) -> Vec<(ChangeId, usize)> {
        self.bars
            .session
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .map(|session| {
                session
                    .read(cx)
                    .jj()
                    .records
                    .iter()
                    .enumerate()
                    .map(|(index, record)| (record.change.clone(), index))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// ⌥-click: the clicked line's turn in a card. Returns whether it handled
    /// the click, so the editor does not also add a cursor.
    pub(super) fn click_turn(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.bars.held {
            return false;
        }
        self.bars.card = None;
        let Some(editor) = self.tab().and_then(|tab| tab.editor.clone()) else {
            return false;
        };
        let Some(point) = pi_editor::point_at(&editor, position, window, cx) else {
            return false;
        };
        let Some((_, turns)) = &self.bars.turns else {
            return false;
        };
        let Some(turn) = turns.lines.get(point.row as usize).cloned().flatten() else {
            cx.notify();
            return true;
        };
        let lines = turns
            .lines
            .iter()
            .filter(|line| line.as_ref() == Some(&turn))
            .count();
        let prompt = turns.descriptions.get(&turn).cloned().unwrap_or_default();
        let record = self
            .session_turns(cx)
            .into_iter()
            .find(|(change, _)| *change == turn)
            .map(|(_, index)| index);
        let Some(at) = pi_editor::below(&editor, point, window, cx) else {
            return true;
        };
        self.bars.card = Some(TurnCard {
            short: pi_jj::short_change_id(&turn),
            who: match record {
                Some(index) => format!("turn {} · this session", index + 1),
                None => "another session's turn".into(),
            },
            record,
            prompt,
            lines,
            at,
        });
        cx.notify();
        true
    }

    pub(super) fn turn_card(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let card = self.bars.card.clone()?;
        let theme = theme(cx);
        let record = card.record;
        Some(
            gpui::deferred(
                gpui::anchored().position(card.at).child(
                    v_flex()
                        .id("turn-card")
                        .debug_selector(|| "turn-card".into())
                        .occlude()
                        .w(px(400.))
                        .p(px(12.))
                        .gap(px(8.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme.chip_line)
                        .bg(if theme.light { theme.chip } else { theme.bar })
                        .shadow_lg()
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.bars.card = None;
                            cx.notify();
                        }))
                        .child(
                            h_flex()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .font_family(MONO)
                                        .text_size(px(11.))
                                        .text_color(theme.accent)
                                        .child(card.short.clone()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(px(11.5))
                                        .text_color(theme.muted)
                                        .child(card.who.clone()),
                                )
                                .child(
                                    div()
                                        .font_family(MONO)
                                        .text_size(px(10.5))
                                        .text_color(theme.faint)
                                        .child(format!(
                                            "{} line{}",
                                            card.lines,
                                            if card.lines == 1 { "" } else { "s" }
                                        )),
                                ),
                        )
                        .when(!card.prompt.is_empty(), |v| {
                            v.child(
                                div()
                                    .max_h(px(72.))
                                    .overflow_hidden()
                                    .pl(px(10.))
                                    .border_l_2()
                                    .border_color(theme.line_strong)
                                    .text_size(px(11.5))
                                    .line_height(px(17.))
                                    .text_color(theme.secondary)
                                    .child(card.prompt.clone()),
                            )
                        })
                        .when_some(record, |v, record| {
                            v.child(
                                h_flex()
                                    .gap(px(8.))
                                    .child(
                                        button("turn-card-thread", "Show in thread", theme)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.bars.card = None;
                                                cx.emit(FileEvent::ShowTurn(record));
                                            })),
                                    )
                                    .child(button("turn-card-diff", "Diff", theme).on_click(
                                        cx.listener(move |this, _, _, cx| {
                                            this.bars.card = None;
                                            cx.emit(FileEvent::DiffTurn(record));
                                        }),
                                    )),
                            )
                        }),
                ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}
