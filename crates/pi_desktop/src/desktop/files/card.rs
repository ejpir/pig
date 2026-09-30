//! One card for what the language server says at a spot in a file tab (design
//! study 05): the diagnostic and its code, the hover text, the first quick fix and
//! **Ask pi to fix**. `pi_editor` turns Zed's own hover popovers off.
use super::*;
use gpui::{Point, Task, anchored, deferred};
use language::ToOffset as _;
use pi_editor::{HoverText, Problem, QuickFix};
use std::{ops::Range, path::Path, time::Duration};

gpui::actions!(file_editor, [AskPiToFix]);

/// Zed's hover delays.
const SHOW_DELAY: Duration = Duration::from_millis(300);
const HIDE_DELAY: Duration = Duration::from_millis(300);
const CODE_LINES: usize = 4;

/// What the pointer is over: a problem's range, or a word for plain hover text.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Target {
    path: PathBuf,
    range: Range<usize>,
}

pub(super) struct Card {
    target: Target,
    problem: Option<Problem>,
    hover: Vec<HoverText>,
    fix: Option<QuickFix>,
    at: Point<Pixels>,
    fixing: bool,
}

#[derive(Default)]
pub(super) struct HoverCard {
    card: Option<Card>,
    /// What a delayed show is waiting for.
    pending: Option<Target>,
    over_card: bool,
    show: Option<Task<()>>,
    hide: Option<Task<()>>,
}

impl FilesView {
    pub(super) fn pointer_moved(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Off: Zed's own hover popovers show instead (`pi_editor::Preferences`).
        if self.hover.over_card
            || self.host.is_none()
            || !crate::prefs::flag(cx, "editor.problemCard", None)
        {
            return;
        }
        let shown = self.hover.card.as_ref().map(|card| card.target.clone());
        match self.target_at(position, window, cx) {
            Some((target, _)) if Some(&target) == shown.as_ref() => self.hover.hide = None,
            Some((target, _)) if Some(&target) == self.hover.pending.as_ref() => {}
            Some((target, offset)) => {
                self.hover.pending = Some(target.clone());
                self.hover.show = Some(cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(SHOW_DELAY).await;
                    this.update_in(cx, |this, window, cx| {
                        this.show_card(target, offset, window, cx)
                    })
                    .ok();
                }));
            }
            None => self.leave_card(cx),
        }
    }

    fn target_at(
        &self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<(Target, usize)> {
        let tab = self.tab()?;
        let point = pi_editor::point_at(tab.editor.as_ref()?, position, window, cx)?;
        let snapshot = tab.buffer.read(cx).snapshot();
        let offset = snapshot.point_to_offset(point);
        let range = match pi_editor::problem_at(&snapshot, offset) {
            Some(problem) => {
                problem.range.start.to_offset(&snapshot)..problem.range.end.to_offset(&snapshot)
            }
            None => pi_editor::word_at(&snapshot, offset)?,
        };
        let path = tab.path.clone();
        Some((Target { path, range }, offset))
    }

    fn show_card(
        &mut self,
        target: Target,
        offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hover.pending = None;
        let Some(host) = self.host.clone() else {
            return;
        };
        let Some((buffer, Some(editor))) = self
            .tabs
            .iter()
            .find(|tab| tab.path == target.path)
            .map(|tab| (tab.buffer.clone(), tab.editor.clone()))
        else {
            return;
        };
        let snapshot = buffer.read(cx).snapshot();
        let problem = pi_editor::problem_at(&snapshot, offset);
        let start = problem
            .as_ref()
            .map_or_else(|| snapshot.offset_to_point(target.range.start), |p| p.start);
        let Some(at) = pi_editor::below(&editor, start, window, cx) else {
            return;
        };
        let hover = host.hover_text(&buffer, offset, cx);
        let fix = problem
            .as_ref()
            .map(|problem| host.quick_fix(&buffer, problem.range.clone(), cx));
        // A problem shows at once; plain hover text once it arrives.
        let has_problem = problem.is_some();
        self.hover.card = problem.map(|problem| Card {
            target: target.clone(),
            problem: Some(problem),
            hover: Vec::new(),
            fix: None,
            at,
            fixing: false,
        });
        cx.notify();
        self.hover.show = Some(cx.spawn(async move |this, cx| {
            let hover = hover.await;
            this.update(cx, |this, cx| {
                match &mut this.hover.card {
                    Some(card) if card.target == target => card.hover = hover,
                    None if !has_problem && !hover.is_empty() && this.hover.pending.is_none() => {
                        this.hover.card = Some(Card {
                            target: target.clone(),
                            problem: None,
                            hover,
                            fix: None,
                            at,
                            fixing: false,
                        })
                    }
                    _ => {}
                }
                cx.notify();
            })
            .ok();
            if let Some(fix) = fix {
                let fix = fix.await;
                this.update(cx, |this, cx| {
                    if let Some(card) = this.hover.card.as_mut().filter(|c| c.target == target) {
                        card.fix = fix;
                        cx.notify();
                    }
                })
                .ok();
            }
        }));
    }

    /// The pointer left the text or the card: hide after a moment, so it can
    /// cross the gap between them.
    pub(super) fn leave_card(&mut self, cx: &mut Context<Self>) {
        self.hover.pending = None;
        self.hover.show = None;
        if self.hover.card.is_none() || self.hover.hide.is_some() {
            return;
        }
        self.hover.hide = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HIDE_DELAY).await;
            this.update(cx, |this, cx| {
                if !this.hover.over_card {
                    this.hide_card(cx);
                }
            })
            .ok();
        }));
    }

    pub(super) fn hide_card(&mut self, cx: &mut Context<Self>) {
        if self.hover.card.is_some() || self.hover.pending.is_some() {
            self.hover = HoverCard::default();
            cx.notify();
        }
    }

    fn card_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.hover.over_card = hovered;
        if hovered {
            self.hover.hide = None;
        } else {
            self.leave_card(cx);
        }
    }

    fn apply_fix(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.host.clone() else {
            return;
        };
        let Some(card) = self.hover.card.as_mut().filter(|card| !card.fixing) else {
            return;
        };
        let Some(fix) = card.fix.clone() else {
            return;
        };
        let Some(buffer) = self
            .tabs
            .iter()
            .find(|tab| tab.path == card.target.path)
            .map(|tab| tab.buffer.clone())
        else {
            return;
        };
        card.fixing = true;
        let task = host.apply_quick_fix(buffer, fix, cx);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.error = Some(format!("Quick fix failed: {error:#}"));
                }
                this.hide_card(cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn ask_pi(&mut self, path: PathBuf, problem: Problem, cx: &mut Context<Self>) {
        cx.emit(FileEvent::AskPi(fix_prompt(&self.root, &path, &problem)));
        self.hide_card(cx);
    }

    /// ⌥⏎: the problem under the cursor goes to pi, as the card's button does.
    pub(super) fn ask_pi_at_cursor(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tab() else {
            return;
        };
        let Some(editor) = tab.editor.as_ref() else {
            return;
        };
        let point = pi_editor::cursor_point(editor, cx);
        let snapshot = tab.buffer.read(cx).snapshot();
        let problem = pi_editor::problem_at(&snapshot, snapshot.point_to_offset(point));
        if let Some(problem) = problem {
            let path = tab.path.clone();
            self.ask_pi(path, problem, cx);
        }
    }

    pub(super) fn card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let card = self.hover.card.as_ref()?;
        let theme = theme(cx);
        let problem = card.problem.clone();
        let code = card
            .hover
            .iter()
            .filter(|block| block.code)
            .flat_map(|block| block.text.lines())
            .take(CODE_LINES)
            .map(|line| line.to_owned())
            .collect::<Vec<_>>();
        // A plain hover also shows the first paragraph of the documentation.
        let docs = problem
            .is_none()
            .then(|| card.hover.iter().find(|block| !block.code))
            .flatten()
            .and_then(|block| block.text.split("\n\n").next())
            .map(|text| text.to_owned());
        let indent = if problem.is_some() { 22. } else { 0. };
        let body = v_flex()
            .px(px(14.))
            .py(px(10.))
            .gap(px(4.))
            .when_some(problem.clone(), |v, problem| {
                v.child(
                    h_flex()
                        .items_start()
                        .gap(px(8.))
                        .child(
                            icon(
                                "warning",
                                if problem.error {
                                    theme.coral
                                } else {
                                    theme.amber
                                },
                            )
                            .size(px(14.))
                            .mt(px(2.)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(12.))
                                .line_height(px(18.))
                                .text_color(theme.text)
                                .child(problem.message),
                        )
                        .when_some(problem.code, |h, code| {
                            h.child(
                                div()
                                    .flex_shrink_0()
                                    .font_family(MONO)
                                    .text_size(px(10.5))
                                    .line_height(px(18.))
                                    .text_color(theme.faint)
                                    .child(code),
                            )
                        }),
                )
            })
            .children(code.into_iter().map(|line| {
                div()
                    .pl(px(indent))
                    .font_family(MONO)
                    .text_size(px(11.))
                    .line_height(px(17.))
                    .text_color(theme.code)
                    .whitespace_nowrap()
                    .child(line)
            }))
            .when_some(docs, |v, docs| {
                v.child(
                    div()
                        .text_size(px(11.5))
                        .line_height(px(17.))
                        .text_color(theme.secondary)
                        .child(docs),
                )
            });
        let actions = problem.map(|problem| {
            let path = card.target.path.clone();
            let fix = card.fix.as_ref().map(|fix| {
                if card.fixing {
                    "Applying the quick fix…".to_owned()
                } else {
                    format!("Quick fix: {}", fix.title)
                }
            });
            v_flex()
                .px(px(14.))
                .pt(px(8.))
                .pb(px(9.))
                .gap(px(4.))
                .border_t_1()
                .border_color(theme.line)
                .child(
                    h_flex()
                        .gap(px(10.))
                        .child(
                            div()
                                .id("quick-fix")
                                .debug_selector(|| "quick-fix".into())
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(theme.accent)
                                .when_some(fix, |d, fix| {
                                    d.cursor_pointer()
                                        .hover(|style| style.underline())
                                        .on_click(cx.listener(|this, _, _, cx| this.apply_fix(cx)))
                                        .child(fix)
                                }),
                        )
                        .child(
                            h_flex()
                                .id("ask-pi-to-fix")
                                .debug_selector(|| "ask-pi-to-fix".into())
                                .role(gpui::Role::Button)
                                .aria_label("Ask pi to fix")
                                .flex_shrink_0()
                                .gap(px(6.))
                                .h(px(22.))
                                .px(px(9.))
                                .rounded(px(5.))
                                .border_1()
                                .border_color(theme.chip_line)
                                .text_size(px(11.))
                                .text_color(theme.secondary)
                                .cursor_pointer()
                                .hover(move |style| style.bg(theme.hover))
                                .child(icon("sparkle", theme.muted).size(px(12.)))
                                .child("Ask pi to fix")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.ask_pi(path.clone(), problem.clone(), cx)
                                })),
                        ),
                )
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(px(10.))
                        .text_color(theme.faint)
                        .child(if cfg!(target_os = "macos") {
                            "⌘.  quick fixes    ⌥⏎  send to pi"
                        } else {
                            "Ctrl+.  quick fixes    Alt+Enter  send to pi"
                        }),
                )
        });
        let has_actions = actions.is_some();
        Some(
            deferred(
                anchored()
                    .position(card.at)
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        v_flex()
                            .id("problem-card")
                            .debug_selector(|| "problem-card".into())
                            .occlude()
                            .when(has_actions, |v| v.w(px(404.)))
                            .when(!has_actions, |v| v.max_w(px(520.)))
                            .mt(px(4.))
                            .rounded(px(10.))
                            .border_1()
                            .border_color(theme.chip_line)
                            .bg(if theme.light { theme.chip } else { theme.bar })
                            .shadow_lg()
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.card_hovered(*hovered, cx)
                            }))
                            .child(body)
                            .children(actions),
                    ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}

/// What **Ask pi to fix** sends: where the problem is and what the server said.
pub(super) fn fix_prompt(root: &Path, path: &Path, problem: &Problem) -> String {
    let file = path.strip_prefix(root).unwrap_or(path).display();
    let code = problem
        .code
        .as_ref()
        .map(|code| format!(" {code}"))
        .unwrap_or_default();
    format!(
        "Fix this language server {} in `{file}:{}:{}`:\n\n{}{code}",
        if problem.error { "error" } else { "warning" },
        problem.start.row + 1,
        problem.start.column + 1,
        problem.message,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fix_prompt_names_the_file_position_and_message() {
        let problem = Problem {
            range: language::Anchor::min_min_range_for_buffer(language::BufferId::new(1).unwrap()),
            start: language::Point::new(211, 18),
            message: "'block.signature' is possibly 'undefined'.".into(),
            code: Some("ts(18048)".into()),
            error: true,
        };
        let root = Path::new("/repos/pi");
        let path = root.join("packages/ai/src/openai-completions.ts");
        assert_eq!(
            fix_prompt(root, &path, &problem),
            "Fix this language server error in `packages/ai/src/openai-completions.ts:212:19`:\n\n\
             'block.signature' is possibly 'undefined'. ts(18048)"
        );
    }
}
