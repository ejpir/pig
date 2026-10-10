//! Calls in the thread (design/android/stream): while Pi works, its tool calls
//! stream in between its words, one quiet line each. Reads and searches made
//! in a row fold together, and the call still running is the only one open.
//! The run line stays: folded into the working strip, a sheet away, and drawn
//! in full once the run is done.

use crate::{
    app::{PhoneApp, Sheet},
    model::{Flow, Session, SessionId, StageKind, StageStatus, ToolActivity, Turn},
    theme::{MONO, Theme},
    ui::{self, icon},
};
use gpui::{
    AnyElement, Context, Div, FontWeight, Hsla, SharedString, Stateful, div, prelude::*, px,
};

/// How many lines of a running command's output its open call shows.
const TAIL: usize = 4;

/// Tools whose calls fold into one line when Pi makes several in a row.
fn folds(name: &str) -> bool {
    matches!(name, "read" | "grep" | "find" | "ls")
}

pub(crate) fn glyph(name: &str) -> &'static str {
    match name {
        "read" => "file",
        "edit" => "pencil",
        "write" => "plus",
        "bash" => "term",
        "grep" | "find" => "search",
        "ls" => "folder",
        _ => "spark",
    }
}

/// What a call did, or is doing: "Read", "Editing".
pub(crate) fn verb(name: &str, finished: bool) -> &str {
    match (name, finished) {
        ("read", true) => "Read",
        ("read", false) => "Reading",
        ("edit", true) => "Edited",
        ("edit", false) => "Editing",
        ("write", true) => "Wrote",
        ("write", false) => "Writing",
        ("bash", true) => "Ran",
        ("bash", false) => "Running",
        ("grep" | "find", true) => "Searched",
        ("grep" | "find", false) => "Searching",
        ("ls", true) => "Listed",
        ("ls", false) => "Listing",
        (name, _) => name,
    }
}

/// What a call touched, shortened for one line: a file's name, a command's
/// first line, a search in quotes.
pub(crate) fn target(tool: &ToolActivity) -> String {
    match tool.name.as_str() {
        "read" | "edit" | "write" | "ls" => tool
            .target
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(&tool.target)
            .to_owned(),
        "grep" | "find" => format!("“{}”", tool.target),
        _ => tool.target.lines().next().unwrap_or("").trim().to_owned(),
    }
}

/// The exit code a failed command reported: "Command exited with code 2".
pub(crate) fn exit_code(output: &str) -> Option<i32> {
    let (_, rest) = output.rsplit_once("exited with code ")?;
    let digits: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    digits.parse().ok()
}

/// The call Pi made after this one in the turn, if any.
pub(crate) fn next_call(
    turn: &Turn,
    kind: StageKind,
    index: usize,
) -> Option<(StageKind, usize, &ToolActivity)> {
    let at = turn
        .flow
        .iter()
        .position(|step| *step == Flow::Tool(kind, index))?;
    turn.flow[at + 1..].iter().find_map(|step| match step {
        Flow::Tool(kind, index) => turn.tool(*kind, *index).map(|tool| (*kind, *index, tool)),
        _ => None,
    })
}

/// Whether the target reads as code: a path or a command, not a search.
fn code_target(name: &str) -> bool {
    !matches!(name, "grep" | "find")
}

/// The line of a failed call's output that says what went wrong.
fn failure(tool: &ToolActivity) -> String {
    let output = tool.output_for_display();
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    lines
        .iter()
        .find(|line| {
            let lower = line.to_lowercase();
            lower.contains("error") || lower.contains("fail") || lower.contains("not found")
        })
        .or(lines.last())
        .map_or_else(|| "Failed".to_owned(), |line| format!("Failed · {line}"))
}

/// A piece of the stream: a step of the turn's flow, or calls folded together.
enum Piece {
    Step(usize),
    /// The first step's position in the flow, and each call.
    Folded(usize, Vec<(StageKind, usize)>),
}

/// Calls of one tool made in a row, gathering until another step comes.
struct Run<'a> {
    first: usize,
    name: &'a str,
    calls: Vec<(StageKind, usize)>,
}

impl Run<'_> {
    fn piece(self) -> Piece {
        if self.calls.len() == 1 {
            Piece::Step(self.first)
        } else {
            Piece::Folded(self.first, self.calls)
        }
    }
}

fn pieces(turn: &Turn) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut run: Option<Run> = None;
    for (n, step) in turn.flow.iter().enumerate() {
        let foldable = match step {
            Flow::Tool(kind, index) => turn
                .tool(*kind, *index)
                .filter(|tool| tool.finished && !tool.failed && folds(&tool.name))
                .map(|tool| (tool.name.as_str(), (*kind, *index))),
            _ => None,
        };
        match (foldable, &mut run) {
            (Some((name, call)), Some(current)) if current.name == name => current.calls.push(call),
            (Some((name, call)), _) => {
                pieces.extend(run.take().map(Run::piece));
                run = Some(Run {
                    first: n,
                    name,
                    calls: vec![call],
                });
            }
            (None, _) => {
                pieces.extend(run.take().map(Run::piece));
                pieces.push(Piece::Step(n));
            }
        }
    }
    pieces.extend(run.map(Run::piece));
    pieces
}

/// A call's icon in a 24 dp tile of its stage's colour; ringed while it runs.
fn tile(glyph: &str, hue: Hsla, live: bool, colors: &Theme) -> Div {
    let tile = ui::tile_box(24., 8., hue, colors).child(icon(glyph, 14., hue));
    if live {
        tile.bg(colors.composer)
            .border(px(1.5))
            .border_color(hue)
            .shadow(ui::ring(hue, 3., 0.18))
    } else {
        tile
    }
}

/// "+3 −1", for an edit.
fn change(added: u32, removed: u32, colors: &Theme) -> Div {
    div()
        .flex()
        .flex_none()
        .gap(px(8.))
        .font_family(MONO)
        .text_size(px(12.5))
        .when(added > 0, |counts| {
            counts.child(div().text_color(colors.green).child(format!("+{added}")))
        })
        .when(removed > 0, |counts| {
            counts.child(div().text_color(colors.coral).child(format!("−{removed}")))
        })
}

/// A verb and what it touched, on one line that ends in an ellipsis.
fn what(verb: &str, target: String, code: bool, colors: &Theme) -> Div {
    div()
        .flex()
        .items_baseline()
        .gap(px(6.))
        .min_w_0()
        .text_size(px(14.))
        .line_height(px(20.))
        .child(
            div()
                .flex_none()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors.text)
                .child(verb.to_owned()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(colors.secondary)
                .when(code, |target| target.font_family(MONO).text_size(px(12.5)))
                .child(target),
        )
}

fn sub(text: impl Into<gpui::SharedString>, color: Hsla) -> Div {
    div()
        .text_size(px(12.5))
        .line_height(px(16.))
        .text_color(color)
        .truncate()
        .child(text.into())
}

/// A line of a command's output, coloured the way a terminal reads: echoed
/// commands dim, passes green, errors and where they are in coral, paths in
/// the text colour, warnings amber. Everything else keeps the line's colour.
pub(crate) fn output_text(line: &str, colors: &Theme) -> gpui::StyledText {
    gpui::StyledText::new(SharedString::from(line.to_owned())).with_highlights(
        output_spans(line, colors)
            .into_iter()
            .map(|(range, color)| (range, gpui::HighlightStyle::color(color))),
    )
}

fn output_spans(line: &str, colors: &Theme) -> Vec<(std::ops::Range<usize>, Hsla)> {
    let mut spans: Vec<(std::ops::Range<usize>, Hsla)> = Vec::new();
    let trimmed = line.trim_start();
    if trimmed.starts_with("> ") || trimmed.starts_with("$ ") || trimmed == ">" {
        spans.push((0..line.len(), colors.faint));
    } else if let Some(at) = line.find("exited with code ") {
        let code = at + "exited with code ".len();
        spans.push((0..code, colors.muted));
        if !line[code..].starts_with('0') {
            spans.push((code..line.len(), colors.coral));
        }
    } else {
        let mut last_number: Option<std::ops::Range<usize>> = None;
        let mut start = 0;
        for word in line.split_inclusive(char::is_whitespace) {
            let range = start..start + word.trim_end().len();
            start += word.len();
            let token = &line[range.clone()];
            if token.is_empty() {
                continue;
            }
            let color = token_color(token, colors);
            if let Some(color) = color {
                // "1 error", "3 failed": the count goes with its word.
                if color == colors.coral
                    && let Some(number) = last_number.take()
                {
                    spans.push((number.start..range.start, color));
                }
                spans.push((range, color));
                continue;
            }
            last_number = token
                .chars()
                .all(|c| c.is_ascii_digit())
                .then_some(range.clone());
            // A place in a file: "test/qwen.test.ts:14:22".
            let location = token.find(':').filter(|&colon| {
                let (path, rest) = token.split_at(colon);
                (path.contains('/') || path.contains('.'))
                    && rest[1..].starts_with(|c: char| c.is_ascii_digit())
            });
            match location {
                Some(colon) => {
                    spans.push((range.start..range.start + colon, colors.text));
                    spans.push((range.start + colon..range.end, colors.coral));
                }
                None if token.contains('/') && token.contains('.') => {
                    spans.push((range, colors.text))
                }
                None => {}
            }
        }
    }
    spans
}

/// The colour a word of output says something with: a pass, a failure, an
/// error code such as TS18048 or E0308, a warning.
fn token_color(token: &str, colors: &Theme) -> Option<Hsla> {
    let first = token.chars().next()?;
    let bare = token
        .trim_matches(|c: char| matches!(c, ':' | ',' | '.' | '(' | ')' | '[' | ']'))
        .to_lowercase();
    let code = |prefix: &str| {
        bare.strip_prefix(prefix)
            .is_some_and(|digits| digits.len() >= 3 && digits.chars().all(|c| c.is_ascii_digit()))
    };
    if matches!(first, '✓' | '√' | '✔') || matches!(bare.as_str(), "pass" | "passed" | "ok") {
        Some(colors.green)
    } else if matches!(first, '✗' | '×' | '✘')
        || bare.starts_with("error")
        || matches!(
            bare.as_str(),
            "errors" | "fail" | "failed" | "failure" | "failures" | "panicked"
        )
        || code("ts")
        || code("e")
    {
        Some(colors.coral)
    } else if bare.starts_with("warn") {
        Some(colors.amber)
    } else {
        None
    }
}

/// A line of a running command's output; the newest line darkest.
fn tail_line(line: String, newest: bool, colors: &Theme) -> Div {
    div()
        .h(px(18.))
        .overflow_hidden()
        .font_family(MONO)
        .text_size(px(12.))
        .line_height(px(18.))
        .whitespace_nowrap()
        .text_color(if newest {
            colors.text
        } else {
            colors.secondary
        })
        .child(output_text(&line, colors))
}

/// The run line folded to four segments, for the working strip: each stage
/// done in its colour, the live one ringed, those ahead plain.
pub(crate) fn segments(turn: &Turn, colors: &Theme) -> Div {
    div()
        .w(px(56.))
        .flex_none()
        .flex()
        .gap(px(3.))
        .children(StageKind::ALL.into_iter().map(|kind| {
            let hue = ui::stage_hue(kind, colors);
            let status = turn
                .stages
                .iter()
                .find(|stage| stage.kind == kind)
                .map_or(StageStatus::Planned, |stage| stage.status);
            let segment = div().flex_1().h(px(4.)).rounded(px(2.));
            match status {
                StageStatus::Done => segment.bg(hue),
                StageStatus::Live => segment.bg(hue).shadow(ui::ring(hue, 2., 0.25)),
                StageStatus::Planned | StageStatus::Skipped => segment.bg(colors.raised),
            }
        }))
}

impl PhoneApp {
    /// A turn's words and calls in the order Pi made them. `live` opens the
    /// calls still running.
    pub(crate) fn stream(
        &self,
        session: &Session,
        index: usize,
        live: bool,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let id = session.id;
        let turn = &session.turns[index];
        let mut column = div().min_w_0().flex().flex_col().gap(px(16.));
        let mut calls: Vec<AnyElement> = Vec::new();
        let flush = |column: Div, calls: &mut Vec<AnyElement>| {
            if calls.is_empty() {
                column
            } else {
                column.child(div().min_w_0().flex().flex_col().children(calls.drain(..)))
            }
        };
        for piece in pieces(turn) {
            match piece {
                Piece::Folded(first, folded) => {
                    calls.push(
                        self.folded_calls(id, index, turn, first, &folded, colors, cx)
                            .into_any_element(),
                    );
                }
                Piece::Step(n) => match &turn.flow[n] {
                    Flow::Tool(kind, tool_index) => {
                        let Some(tool) = turn.tool(*kind, *tool_index) else {
                            continue;
                        };
                        if live && !tool.finished {
                            column = flush(column, &mut calls);
                            column = column.child(self.open_call(
                                id,
                                index,
                                turn,
                                *kind,
                                *tool_index,
                                tool,
                                colors,
                                cx,
                            ));
                        } else {
                            calls.push(
                                self.call_row(
                                    id,
                                    index,
                                    *kind,
                                    *tool_index,
                                    tool,
                                    false,
                                    colors,
                                    cx,
                                )
                                .into_any_element(),
                            );
                        }
                    }
                    step => {
                        column = flush(column, &mut calls);
                        let element = match step {
                            Flow::Text(text) => Some(
                                div()
                                    .relative()
                                    .min_w_0()
                                    .child(crate::message::render(text, colors))
                                    .child(self.copyable(text.clone(), cx))
                                    .into_any_element(),
                            ),
                            Flow::Image(n) => self
                                .image_card(id, index, turn, *n, colors, cx)
                                .map(IntoElement::into_any_element),
                            Flow::Page(n) => self
                                .page_card(index, turn, *n, colors, cx)
                                .map(IntoElement::into_any_element),
                            Flow::Handoff(n) => self
                                .handoff_card(id, index, turn, *n, colors, cx)
                                .map(IntoElement::into_any_element),
                            Flow::Compacted(summary) => Some(
                                self.compacted(id, index, n, summary, colors, cx)
                                    .into_any_element(),
                            ),
                            Flow::Tool(..) => None,
                        };
                        column = column.children(element);
                    }
                },
            }
        }
        flush(column, &mut calls)
    }

    /// One call on one line: its icon, a verb, what it touched and its change
    /// size. A failed call says why, in coral. Tapping opens it.
    #[allow(clippy::too_many_arguments)]
    fn call_row(
        &self,
        id: SessionId,
        turn_index: usize,
        kind: StageKind,
        tool_index: usize,
        tool: &ToolActivity,
        live: bool,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let hue = if tool.failed {
            colors.coral
        } else {
            ui::stage_hue(kind, colors)
        };
        div()
            .id(("call", kind.index() * 100_000 + tool_index))
            .debug_selector(|| "call".into())
            .relative()
            .child(crate::testing::probe(format!(
                "turn-{turn_index}-call-{}-{tool_index}",
                kind.index()
            )))
            .min_h(px(if tool.failed {
                56.
            } else if live {
                48.
            } else {
                40.
            }))
            .flex()
            .items_center()
            .gap(px(12.))
            .rounded(px(8.))
            .active(|style| style.bg(colors.selected))
            .child(tile(glyph(&tool.name), hue, live, colors))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(what(
                        verb(&tool.name, tool.finished),
                        target(tool),
                        code_target(&tool.name),
                        colors,
                    ))
                    .when(tool.failed, |text| {
                        text.child(sub(failure(tool), colors.coral))
                    }),
            )
            .when(tool.added > 0 || tool.removed > 0, |row| {
                row.child(change(tool.added, tool.removed, colors))
            })
            .when(!live, |row| row.child(icon("chev_r", 16., colors.faint)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_sheet(Sheet::Call(id, turn_index, kind, tool_index), cx)
            }))
    }

    /// The call still running, open: an edit with its latest lines, a command
    /// with the last lines of its output in a box that keeps its height.
    #[allow(clippy::too_many_arguments)]
    fn open_call(
        &self,
        id: SessionId,
        turn_index: usize,
        turn: &Turn,
        kind: StageKind,
        tool_index: usize,
        tool: &ToolActivity,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let stage = turn.stages.iter().find(|stage| stage.kind == kind);
        let latest = stage.is_some_and(|stage| stage.tools.len() == tool_index + 1);
        let diff = stage
            .filter(|stage| latest && kind == StageKind::Change && !stage.diff.is_empty())
            .map(|stage| {
                div()
                    .relative()
                    .child(crate::testing::probe(format!(
                        "turn-{turn_index}-live-diff-{}",
                        stage.diff.len()
                    )))
                    .py(px(4.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(colors.line)
                    .bg(colors.canvas)
                    // A glance at the change: long lines are cut, Review shows them.
                    .overflow_hidden()
                    .children({
                        let path = stage.diff_path.as_deref().unwrap_or(&tool.target);
                        let language = crate::screens::language(path);
                        stage.diff.iter().map(move |line| {
                            ui::code_line_in(line, false, 40., false, language, colors)
                        })
                    })
            });
        let output = (tool.name == "bash" && !tool.output.is_empty()).then(|| {
            let displayed = tool.output_for_display();
            let lines: Vec<String> = displayed
                .lines()
                .rev()
                .take(TAIL)
                .map(str::to_owned)
                .collect();
            div()
                .px(px(12.))
                .py(px(8.))
                .h(px(TAIL as f32 * 18. + 16.))
                .overflow_hidden()
                .rounded(px(8.))
                .bg(colors.panel)
                .flex()
                .flex_col()
                .justify_end()
                .children({
                    let shown = lines.len();
                    lines
                        .into_iter()
                        .rev()
                        .enumerate()
                        .map(move |(n, line)| tail_line(line, n + 1 == shown, colors))
                })
        });
        let body = diff.is_some() || output.is_some();
        ui::card(colors)
            .px(px(12.))
            .when(body, |card| card.pb(px(12.)))
            .child(self.call_row(id, turn_index, kind, tool_index, tool, true, colors, cx))
            .children(diff)
            .children(output)
    }

    /// Calls of one kind made in a row, as one line: "Read 5 files" and the
    /// first names under it. Tapping rolls them out in place.
    #[allow(clippy::too_many_arguments)]
    fn folded_calls(
        &self,
        id: SessionId,
        turn_index: usize,
        turn: &Turn,
        first: usize,
        calls: &[(StageKind, usize)],
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let tools: Vec<&ToolActivity> = calls
            .iter()
            .filter_map(|(kind, index)| turn.tool(*kind, *index))
            .collect();
        let Some(head) = tools.first() else {
            return div();
        };
        let kind = calls[0].0;
        let open = self.expanded_calls.contains(&(id, turn_index, first));
        let count = tools.len();
        let (verb, noun) = match head.name.as_str() {
            "read" => ("Read", format!("{count} files")),
            "grep" | "find" => ("Searched", format!("{count} times")),
            "ls" => ("Listed", format!("{count} folders")),
            name => (name, format!("{count} times")),
        };
        let mut names: Vec<String> = Vec::new();
        for tool in &tools {
            let name = target(tool);
            if !names.contains(&name) {
                names.push(name);
            }
        }
        let shown = names.iter().take(2).cloned().collect::<Vec<_>>().join(", ");
        let names = match names.len().saturating_sub(2) {
            0 => shown,
            more => format!("{shown}, +{more}"),
        };
        let hue = ui::stage_hue(kind, colors);
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .id(("calls", first))
                    .debug_selector(|| "folded-calls".into())
                    .relative()
                    .child(crate::testing::probe(format!(
                        "turn-{turn_index}-calls-{first}"
                    )))
                    .min_h(px(56.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .rounded(px(8.))
                    .active(|style| style.bg(colors.selected))
                    .child(tile(glyph(&head.name), hue, false, colors))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(what(verb, noun, false, colors))
                            .when(!open, |text| text.child(sub(names, colors.muted))),
                    )
                    .child(icon(
                        if open { "chev_u" } else { "chev_d" },
                        16.,
                        colors.faint,
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(list) = this.thread_lists.get(&id) {
                            list.pause_following_tail();
                        }
                        if !this.expanded_calls.remove(&(id, turn_index, first)) {
                            this.expanded_calls.insert((id, turn_index, first));
                        }
                        this.remeasure_thread_turn(id, turn_index);
                        cx.notify();
                    })),
            )
            .when(open, |column| {
                column.children(calls.iter().zip(&tools).map(|((kind, index), tool)| {
                    self.call_row(id, turn_index, *kind, *index, tool, false, colors, cx)
                        .pl(px(16.))
                }))
            })
    }

    /// Under a finished run's line: how many calls it made, and Show to bring
    /// them back into the thread between Pi's words.
    pub(crate) fn calls_fold(
        &self,
        id: SessionId,
        index: usize,
        turn: &Turn,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Option<Stateful<Div>> {
        let (calls, failed) = turn.call_counts();
        if calls == 0 || !turn.streams() {
            return None;
        }
        let open = self.expanded_streams.contains(&(id, index));
        Some(
            div()
                .id(("calls-fold", index))
                .debug_selector(|| "calls-fold".into())
                .relative()
                .child(crate::testing::probe(format!("turn-{index}-calls-fold")))
                .mt(px(-12.))
                .min_h(px(40.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(8.))
                .text_size(px(13.))
                .text_color(colors.muted)
                .active(|style| style.bg(colors.selected))
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors.secondary)
                        .child(if calls == 1 {
                            "1 call".to_owned()
                        } else {
                            format!("{calls} calls")
                        }),
                )
                .when(failed > 0, |row| {
                    row.child(div().flex_none().child(format!("· {failed} failed")))
                })
                .child(div().flex_1())
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors.accent)
                        .child(if open { "Hide" } else { "Show" }),
                )
                .child(icon(
                    if open { "chev_u" } else { "chev_d" },
                    16.,
                    colors.accent,
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(list) = this.thread_lists.get(&id) {
                        list.pause_following_tail();
                    }
                    if !this.expanded_streams.remove(&(id, index)) {
                        this.expanded_streams.insert((id, index));
                    }
                    this.remeasure_thread_turn(id, index);
                    cx.notify();
                })),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colored<'a>(line: &'a str, colors: &Theme) -> Vec<(&'a str, Hsla)> {
        output_spans(line, colors)
            .into_iter()
            .map(|(range, color)| (&line[range], color))
            .collect()
    }

    #[test]
    fn output_reads_like_a_terminal() {
        let colors = Theme::new(false);
        assert_eq!(
            colored(
                "test/qwen.test.ts:14:22 - error TS18048: 'model' is possibly 'undefined'.",
                &colors
            ),
            [
                ("test/qwen.test.ts", colors.text),
                (":14:22", colors.coral),
                ("error", colors.coral),
                ("TS18048:", colors.coral),
            ]
        );
        assert_eq!(
            colored("Found 1 error in test/qwen.test.ts:14", &colors),
            [
                ("1 ", colors.coral),
                ("error", colors.coral),
                ("test/qwen.test.ts", colors.text),
                (":14", colors.coral),
            ]
        );
        assert_eq!(
            colored("> tsc --noEmit", &colors),
            [("> tsc --noEmit", colors.faint)]
        );
        assert_eq!(
            colored("✓ qwen › accepts an empty signature", &colors),
            [("✓", colors.green)]
        );
        assert_eq!(
            colored("Command exited with code 2", &colors),
            [
                ("Command exited with code ", colors.muted),
                ("2", colors.coral)
            ]
        );
        assert!(colored("streams thinking", &colors).is_empty());
    }

    #[test]
    fn a_finished_call_leads_to_the_next_one() {
        let mut turn = Turn::new("", "");
        turn.flow = vec![
            Flow::Tool(StageKind::Verify, 0),
            Flow::Text("Fixing it.".into()),
            Flow::Tool(StageKind::Change, 0),
        ];
        for kind in [StageKind::Verify, StageKind::Change] {
            turn.stage_mut(kind).tools.push(ToolActivity {
                id: String::new(),
                name: if kind == StageKind::Verify {
                    "bash"
                } else {
                    "edit"
                }
                .into(),
                target: "x".into(),
                output: "Command exited with code 2".into(),
                finished: true,
                failed: kind == StageKind::Verify,
                added: 0,
                removed: 0,
            });
        }
        let next = next_call(&turn, StageKind::Verify, 0).map(|(kind, index, _)| (kind, index));
        assert_eq!(next, Some((StageKind::Change, 0)));
        assert_eq!(
            next_call(&turn, StageKind::Change, 0).map(|(k, i, _)| (k, i)),
            None
        );
        assert_eq!(exit_code("…\nCommand exited with code 2"), Some(2));
    }
}
