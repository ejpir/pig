//! Bottom sheets: a question to answer (05), a session's details (09), and
//! the short choices: attaching, the model, the project, more actions, and
//! what Pi has on the computer.

use crate::{
    app::{PhoneApp, Route, Sheet, Target},
    model::{
        DiffLine, Mark, Reference, Session, SessionId, Stage, StageKind, StageStatus, Stretch,
        StretchKind, Turn, duration_label,
    },
    theme::{MONO, Theme, theme},
    ui::{self, Button, icon},
};
use gpui::{
    AnyElement, Context, Div, ElementId, FontWeight, ListAlignment, ListState, Pixels,
    ScrollHandle, SharedString, Window, div, list, prelude::*, px, relative,
};
use std::{
    cell::RefCell,
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    rc::Rc,
    sync::Arc,
};

const MODELS: [(&str, &str); 4] = [
    ("Opus 5.5", "Anthropic · the most capable"),
    ("Sonnet 5.5", "Anthropic · fast and capable"),
    ("Fable 5.1", "Anthropic"),
    ("Haiku 4.5", "Anthropic · the fastest"),
];

fn compaction_label(enabled: bool, percent: Option<f64>) -> String {
    if !enabled {
        "auto-compaction off".into()
    } else if let Some(percent) = percent {
        format!("auto-compacts above {percent:.1}%")
    } else {
        "auto-compaction on".into()
    }
}

const THINKING: [&str; 6] = ["Off", "Minimal", "Low", "Medium", "High", "Max"];

#[derive(Clone, Debug, PartialEq)]
enum ActivityRow {
    Intro {
        what: SharedString,
    },
    DiffHeader {
        live: bool,
    },
    DiffPath(SharedString),
    DiffLine {
        index: usize,
        line: DiffLine,
        gutter: f32,
        width: f32,
    },
    Reference {
        index: usize,
        reference: Reference,
    },
    ToolHeader {
        index: usize,
        id: SharedString,
    },
    ToolTargetLine {
        index: usize,
        id: SharedString,
        line_index: usize,
        line: SharedString,
        width: f32,
        first: bool,
    },
    ToolOutputHeader {
        index: usize,
        id: SharedString,
    },
    ToolOutputLine {
        index: usize,
        id: SharedString,
        line_index: usize,
        line: SharedString,
        width: f32,
        first: bool,
        last: bool,
    },
    ToolEmpty {
        index: usize,
        id: SharedString,
    },
    Empty,
}

#[derive(Clone, PartialEq, Eq)]
struct ActivityToolSource {
    id: String,
    output_len: usize,
    output_hash: u64,
    output_ptr: usize,
    sample_hash: u64,
    ends_with_newline: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct ActivitySource {
    structure_key: u64,
    tools: Vec<ActivityToolSource>,
}

#[derive(Clone)]
struct ActivityToolRows {
    block_start: usize,
    block_end: usize,
    output_header: Option<usize>,
    output_start: usize,
    output_end: usize,
    empty: Option<usize>,
}

/// Retained, line-level virtualization for the currently open activity sheet.
/// Output text is split only when its shape changes, not on every animation frame.
pub(crate) struct ActivitySheetState {
    binding: (SessionId, usize, StageKind),
    content_key: u64,
    source: ActivitySource,
    tool_rows: Vec<ActivityToolRows>,
    pub(crate) list: ListState,
    rows: Arc<Vec<ActivityRow>>,
    wrap: bool,
    hints_applied: bool,
    pub(crate) offset: Pixels,
    horizontal: Rc<RefCell<HashMap<String, ScrollHandle>>>,
    body_height: f32,
}

impl ActivitySheetState {
    fn new(
        binding: (SessionId, usize, StageKind),
        content_key: u64,
        source: ActivitySource,
        rows: Vec<ActivityRow>,
        wrap: bool,
    ) -> Self {
        let body_height = rows.iter().map(activity_row_height).sum();
        let tool_rows = activity_tool_rows(&rows);
        let mut state = Self {
            binding,
            content_key,
            source,
            tool_rows,
            list: ListState::new(rows.len(), ListAlignment::Top, px(72.))
                .with_uniform_item_height(px(20.)),
            rows: Arc::new(rows),
            wrap,
            hints_applied: false,
            offset: px(0.),
            body_height,
            horizontal: Rc::new(RefCell::new(HashMap::new())),
        };
        state.sync_horizontal_rows();
        state
    }

    fn sync_horizontal_rows(&mut self) {
        self.horizontal.borrow_mut().retain(|key, _| {
            self.rows
                .iter()
                .any(|row| activity_pan_key(row).as_ref() == Some(key))
        });
    }

    /// Approximate content-sized panel height until the list reaches its cap.
    pub(crate) fn panel_height(&self) -> Pixels {
        let measured =
            self.list.viewport_bounds().size.height + self.list.max_offset_for_scrollbar().y;
        px((self.body_height.max(f32::from(measured)) + 92.).max(240.))
    }
}

fn activity_row_height(row: &ActivityRow) -> f32 {
    match row {
        ActivityRow::Intro { .. } => 58.,
        ActivityRow::DiffHeader { .. } => 38.,
        ActivityRow::DiffPath(_) => 26.,
        ActivityRow::DiffLine { .. } => 20.,
        ActivityRow::Reference { .. } => 48.,
        ActivityRow::ToolHeader { .. } => 48.,
        ActivityRow::ToolTargetLine { .. } => 18.,
        ActivityRow::ToolOutputHeader { .. } => 42.,
        ActivityRow::ToolOutputLine { first, last, .. } => {
            18. + if *first { 2. } else { 0. } + if *last { 14. } else { 0. }
        }
        ActivityRow::ToolEmpty { .. } => 44.,
        ActivityRow::Empty => 20.,
    }
}

fn activity_stage(turn: &Turn, kind: StageKind) -> Option<&Stage> {
    turn.stages.iter().find(|stage| stage.kind == kind)
}

fn activity_horizontal_handle(
    handles: &Rc<RefCell<HashMap<String, ScrollHandle>>>,
    key: String,
) -> ScrollHandle {
    handles.borrow_mut().entry(key).or_default().clone()
}

fn activity_pan_key(row: &ActivityRow) -> Option<String> {
    match row {
        ActivityRow::DiffLine { index, .. } => Some(format!("diff:{index}")),
        ActivityRow::ToolTargetLine { id, line_index, .. } => {
            Some(format!("{id}:target:{line_index}"))
        }
        ActivityRow::ToolOutputLine { id, line_index, .. } => {
            Some(format!("{id}:output:{line_index}"))
        }
        _ => None,
    }
}

fn activity_tool_rows(rows: &[ActivityRow]) -> Vec<ActivityToolRows> {
    let starts: Vec<_> = rows
        .iter()
        .enumerate()
        .filter_map(|(row, value)| matches!(value, ActivityRow::ToolHeader { .. }).then_some(row))
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(index, &block_start)| {
            let block_end = starts.get(index + 1).copied().unwrap_or(rows.len());
            let output_header = rows[block_start..block_end]
                .iter()
                .position(|row| matches!(row, ActivityRow::ToolOutputHeader { .. }))
                .map(|offset| block_start + offset);
            let output_start = output_header.map_or(block_end, |header| header + 1);
            let output_end = rows[output_start..block_end]
                .iter()
                .position(|row| !matches!(row, ActivityRow::ToolOutputLine { .. }))
                .map_or(block_end, |offset| output_start + offset);
            let empty = rows[block_start..block_end]
                .iter()
                .position(|row| matches!(row, ActivityRow::ToolEmpty { .. }))
                .map(|offset| block_start + offset);
            ActivityToolRows {
                block_start,
                block_end,
                output_header,
                output_start,
                output_end,
                empty,
            }
        })
        .collect()
}

fn shift_activity_tool_rows(rows: &mut ActivityToolRows, delta: isize) {
    let shift = |index: usize| index.saturating_add_signed(delta);
    rows.block_start = shift(rows.block_start);
    rows.block_end = shift(rows.block_end);
    rows.output_header = rows.output_header.map(shift);
    rows.output_start = shift(rows.output_start);
    rows.output_end = shift(rows.output_end);
    rows.empty = rows.empty.map(shift);
}

fn activity_mono_width(text: &str, font_size: f32, padding: f32) -> f32 {
    let units = text
        .chars()
        .map(|character| match character {
            '\t' => 4.,
            character if character.is_control() => 0.,
            character if character.is_ascii() => 1.,
            _ => 2.,
        })
        .sum::<f32>();
    padding + units * font_size * 0.6
}

fn hash_activity_text(text: &str, exact: bool, hasher: &mut impl Hasher) {
    text.len().hash(hasher);
    if exact || text.len() <= 256 {
        text.hash(hasher);
    } else {
        text.as_bytes()[..128].hash(hasher);
        text.as_bytes()[text.len() - 128..].hash(hasher);
    }
}

fn activity_content_key(stage: &Stage, kind: StageKind, exact: bool) -> u64 {
    let mut hasher = DefaultHasher::new();
    std::mem::discriminant(&stage.status).hash(&mut hasher);
    hash_activity_text(&stage.what, exact, &mut hasher);
    stage.diff_path.as_deref().hash(&mut hasher);
    std::mem::discriminant(&kind).hash(&mut hasher);
    for reference in &stage.references {
        std::mem::discriminant(reference).hash(&mut hasher);
        match reference {
            Reference::File(value) | Reference::Search(value) => value.hash(&mut hasher),
        }
    }
    for line in &stage.diff {
        std::mem::discriminant(&line.kind).hash(&mut hasher);
        line.number.hash(&mut hasher);
        hash_activity_text(&line.text, exact, &mut hasher);
    }
    if kind != StageKind::Change {
        for tool in &stage.tools {
            tool.id.hash(&mut hasher);
            tool.name.hash(&mut hasher);
            hash_activity_text(&tool.target, exact, &mut hasher);
            // A pointer plus bounded samples catches replacements without rescanning an
            // ever-growing live log. Length catches both in-place and assigned appends.
            tool.output.as_ptr().hash(&mut hasher);
            tool.output.len().hash(&mut hasher);
            activity_fingerprint(&tool.output).hash(&mut hasher);
            tool.finished.hash(&mut hasher);
            tool.failed.hash(&mut hasher);
        }
    }
    hasher.finish()
}

const ACTIVITY_HASH_OFFSET: u64 = 0xcbf29ce484222325;
const ACTIVITY_HASH_PRIME: u64 = 0x100000001b3;

fn extend_activity_hash_bytes(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(ACTIVITY_HASH_PRIME);
    }
    hash
}

fn extend_activity_hash(hash: u64, text: &str) -> u64 {
    extend_activity_hash_bytes(hash, text.as_bytes())
}

fn exact_activity_hash(text: &str) -> u64 {
    extend_activity_hash(ACTIVITY_HASH_OFFSET, text)
}

fn bash_suffix_for_display(raw: &str) -> std::borrow::Cow<'_, str> {
    if raw.contains('\n') || !raw.contains('\r') {
        return pi_core::command_output::for_display(raw);
    }
    let before_pending = raw.strip_suffix('\r').unwrap_or(raw);
    let start = before_pending.rfind('\r').map_or(0, |index| index + 1);
    std::borrow::Cow::Borrowed(&before_pending[start..])
}

fn activity_fingerprint(text: &str) -> u64 {
    let bytes = text.as_bytes();
    if bytes.len() <= 512 {
        return extend_activity_hash_bytes(ACTIVITY_HASH_OFFSET, bytes);
    }
    let mut hash = ACTIVITY_HASH_OFFSET;
    for sample in 0..16 {
        let start = sample * (bytes.len() - 16) / 15;
        hash = extend_activity_hash_bytes(hash, &bytes[start..start + 16]);
    }
    hash
}

fn activity_source(
    stage: &Stage,
    kind: StageKind,
    previous: Option<&ActivitySource>,
) -> ActivitySource {
    let mut structure = DefaultHasher::new();
    std::mem::discriminant(&stage.status).hash(&mut structure);
    stage.what.hash(&mut structure);
    stage.diff_path.hash(&mut structure);
    std::mem::discriminant(&kind).hash(&mut structure);
    for reference in &stage.references {
        std::mem::discriminant(reference).hash(&mut structure);
        match reference {
            Reference::File(value) | Reference::Search(value) => value.hash(&mut structure),
        }
    }
    for line in &stage.diff {
        std::mem::discriminant(&line.kind).hash(&mut structure);
        line.number.hash(&mut structure);
        line.text.hash(&mut structure);
    }
    let tools = if kind == StageKind::Change {
        Vec::new()
    } else {
        stage
            .tools
            .iter()
            .enumerate()
            .map(|(index, tool)| {
                tool.id.hash(&mut structure);
                tool.name.hash(&mut structure);
                tool.target.hash(&mut structure);
                tool.finished.hash(&mut structure);
                tool.failed.hash(&mut structure);
                let output_ptr = tool.output.as_ptr() as usize;
                let sample_hash = activity_fingerprint(&tool.output);
                let old = previous
                    .and_then(|source| source.tools.get(index))
                    .filter(|old| old.id == tool.id);
                let output_hash = old
                    .and_then(|old| {
                        if old.output_len == tool.output.len()
                            && old.output_ptr == output_ptr
                            && old.sample_hash == sample_hash
                        {
                            Some(old.output_hash)
                        } else if old.output_len < tool.output.len()
                            && tool.output.get(..old.output_len).is_some_and(|prefix| {
                                activity_fingerprint(prefix) == old.sample_hash
                            })
                        {
                            tool.output
                                .get(old.output_len..)
                                .map(|suffix| extend_activity_hash(old.output_hash, suffix))
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| exact_activity_hash(&tool.output));
                ActivityToolSource {
                    id: tool.id.clone(),
                    output_len: tool.output.len(),
                    output_hash,
                    output_ptr,
                    sample_hash,
                    ends_with_newline: tool.output.ends_with('\n'),
                }
            })
            .collect()
    };
    ActivitySource {
        structure_key: structure.finish(),
        tools,
    }
}

fn incremental_activity_rows(
    rows: &mut Vec<ActivityRow>,
    tool_rows: &mut [ActivityToolRows],
    old_source: &ActivitySource,
    source: &ActivitySource,
    stage: &Stage,
) -> Option<(f32, std::ops::Range<usize>)> {
    if old_source.structure_key != source.structure_key
        || old_source.tools.len() != source.tools.len()
        || source.tools.len() != stage.tools.len()
        || tool_rows.len() != stage.tools.len()
    {
        return None;
    }

    let mut changed = Vec::new();
    for (index, ((old, new), tool)) in old_source
        .tools
        .iter()
        .zip(&source.tools)
        .zip(&stage.tools)
        .enumerate()
    {
        if old.id != new.id || new.id != tool.id {
            return None;
        }
        if old.output_len == new.output_len && old.output_hash == new.output_hash {
            continue;
        }
        let suffix = tool.output.get(old.output_len..)?;
        if new.output_len < old.output_len
            || extend_activity_hash(old.output_hash, suffix) != new.output_hash
        {
            return None;
        }
        changed.push(index);
    }
    if changed.is_empty() {
        return None;
    }

    let mut height_delta = 0.;
    let mut changed_start = rows.len();
    let mut changed_end = 0;
    for index in changed.into_iter().rev() {
        let old = &old_source.tools[index];
        let tool = &stage.tools[index];
        let positions = tool_rows[index].clone();
        let block_start = positions.block_start;
        let block_end = positions.block_end;
        let id = tool.id.as_str();
        if !matches!(rows.get(block_start), Some(ActivityRow::ToolHeader { index: row_index, id: row_id })
            if *row_index == index && row_id.as_ref() == id)
        {
            return None;
        }

        if old.output_len == 0 {
            let empty = positions.empty?;
            let displayed = tool.output_for_display();
            let lines: Vec<SharedString> = displayed.lines().map(SharedString::from).collect();
            let mut replacement = vec![ActivityRow::ToolOutputHeader {
                index,
                id: tool.id.clone().into(),
            }];
            if lines.is_empty() {
                replacement.push(ActivityRow::ToolEmpty {
                    index,
                    id: tool.id.clone().into(),
                });
            } else {
                let count = lines.len();
                replacement.extend(lines.into_iter().enumerate().map(|(line_index, line)| {
                    ActivityRow::ToolOutputLine {
                        index,
                        id: tool.id.clone().into(),
                        line_index,
                        width: activity_mono_width(&line, 12., 28.),
                        line,
                        first: line_index == 0,
                        last: line_index + 1 == count,
                    }
                }));
            }
            height_delta += replacement.iter().map(activity_row_height).sum::<f32>()
                - activity_row_height(&rows[empty]);
            changed_start = changed_start.min(empty);
            changed_end = changed_end.max(empty + replacement.len());
            let replacement_len = replacement.len();
            rows.splice(empty..empty + 1, replacement);
            let delta = replacement_len as isize - 1;
            tool_rows[index] = ActivityToolRows {
                block_start,
                block_end: block_end.saturating_add_signed(delta),
                output_header: Some(empty),
                output_start: empty + 1,
                output_end: empty + replacement_len,
                empty: (replacement_len == 2
                    && matches!(rows.get(empty + 1), Some(ActivityRow::ToolEmpty { .. })))
                .then_some(empty + 1),
            };
            for later in &mut tool_rows[index + 1..] {
                shift_activity_tool_rows(later, delta);
            }
            continue;
        }

        positions.output_header?;
        let output_end = positions.output_end;
        let old_line_count = output_end.saturating_sub(positions.output_start);
        if old_line_count == 0 {
            return None;
        }
        let (replace_start, line_index, suffix_start) = if old.ends_with_newline {
            (output_end, old_line_count, old.output_len)
        } else {
            (
                output_end - 1,
                old_line_count - 1,
                tool.output[..old.output_len]
                    .rfind('\n')
                    .map_or(0, |position| position + 1),
            )
        };
        let raw_suffix = &tool.output[suffix_start..];
        let displayed_suffix = if tool.name == "bash" {
            bash_suffix_for_display(raw_suffix)
        } else {
            std::borrow::Cow::Borrowed(raw_suffix)
        };
        let lines: Vec<SharedString> = displayed_suffix.lines().map(SharedString::from).collect();
        if lines.is_empty() {
            return None;
        }
        if replace_start == output_end
            && let Some(ActivityRow::ToolOutputLine { last, .. }) = rows.get_mut(output_end - 1)
        {
            height_delta -= 14.;
            *last = false;
        }
        let count = lines.len();
        let replacement: Vec<_> = lines
            .into_iter()
            .enumerate()
            .map(|(offset, line)| ActivityRow::ToolOutputLine {
                index,
                id: tool.id.clone().into(),
                line_index: line_index + offset,
                width: activity_mono_width(&line, 12., 28.),
                line,
                first: line_index + offset == 0,
                last: offset + 1 == count,
            })
            .collect();
        height_delta += replacement.iter().map(activity_row_height).sum::<f32>()
            - rows[replace_start..output_end]
                .iter()
                .map(activity_row_height)
                .sum::<f32>();
        changed_start = changed_start.min(replace_start);
        changed_end = changed_end.max(replace_start + replacement.len());
        let replacement_len = replacement.len();
        let replaced_len = output_end - replace_start;
        rows.splice(replace_start..output_end, replacement);
        let delta = replacement_len as isize - replaced_len as isize;
        tool_rows[index].block_end = block_end.saturating_add_signed(delta);
        tool_rows[index].output_end = replace_start + replacement_len;
        tool_rows[index].empty = None;
        for later in &mut tool_rows[index + 1..] {
            shift_activity_tool_rows(later, delta);
        }
    }
    Some((height_delta, changed_start..changed_end))
}

fn activity_rows(stage: &Stage, kind: StageKind) -> Vec<ActivityRow> {
    let mut rows = vec![ActivityRow::Intro {
        what: stage.what.clone().into(),
    }];
    if !stage.diff.is_empty() {
        rows.push(ActivityRow::DiffHeader {
            live: stage.status == StageStatus::Live,
        });
        rows.extend(
            stage
                .diff_path
                .as_ref()
                .map(|path| ActivityRow::DiffPath(path.clone().into())),
        );
        let gutter = ui::diff_gutter(&stage.diff);
        let width = stage
            .diff
            .iter()
            .map(|line| ui::unwrapped_code_line_width(line, gutter))
            .fold(0., f32::max);
        rows.extend(stage.diff.iter().cloned().enumerate().map(|(index, line)| {
            ActivityRow::DiffLine {
                index,
                line,
                gutter,
                width,
            }
        }));
    }
    rows.extend(
        stage
            .references
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, reference)| ActivityRow::Reference { index, reference }),
    );
    if kind != StageKind::Change {
        for (index, tool) in stage.tools.iter().enumerate() {
            let id: SharedString = tool.id.clone().into();
            rows.push(ActivityRow::ToolHeader {
                index,
                id: id.clone(),
            });
            let target_lines: Vec<SharedString> = if tool.target.is_empty() {
                vec!["".into()]
            } else {
                tool.target.lines().map(SharedString::from).collect()
            };
            let target_width = target_lines
                .iter()
                .map(|line| activity_mono_width(line, 12.5, 28.))
                .fold(0., f32::max);
            rows.extend(
                target_lines
                    .into_iter()
                    .enumerate()
                    .map(|(line_index, line)| ActivityRow::ToolTargetLine {
                        index,
                        id: id.clone(),
                        line_index,
                        line,
                        width: target_width,
                        first: line_index == 0,
                    }),
            );
            if tool.output.is_empty() {
                rows.push(ActivityRow::ToolEmpty { index, id });
                continue;
            }
            rows.push(ActivityRow::ToolOutputHeader {
                index,
                id: id.clone(),
            });
            let displayed = tool.output_for_display();
            let output_lines: Vec<SharedString> =
                displayed.lines().map(SharedString::from).collect();
            if output_lines.is_empty() {
                rows.push(ActivityRow::ToolEmpty { index, id });
            } else {
                let count = output_lines.len();
                let output_width = output_lines
                    .iter()
                    .map(|line| activity_mono_width(line, 12., 28.))
                    .fold(0., f32::max);
                rows.extend(
                    output_lines
                        .into_iter()
                        .enumerate()
                        .map(|(line_index, line)| ActivityRow::ToolOutputLine {
                            index,
                            id: id.clone(),
                            line_index,
                            line,
                            width: output_width,
                            first: line_index == 0,
                            last: line_index + 1 == count,
                        }),
                );
            }
        }
    }
    if kind != StageKind::Change && stage.tools.is_empty() && stage.references.is_empty() {
        rows.push(ActivityRow::Empty);
    }
    rows
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
            Sheet::Call(id, turn, kind, n) => self.call_sheet(id, turn, kind, n, &colors, cx),
            Sheet::Run(id, turn) => self.run_sheet(id, turn, &colors, cx),
            Sheet::Delete(id) => self.delete_sheet(id, &colors, cx),
            Sheet::RestoreHistory(id, index) => self.restore_history_sheet(id, index, &colors, cx),
            Sheet::EnableJj(id) => self.enable_jj_sheet(id, &colors, cx),
            Sheet::Computers => self.computers_sheet(&colors, cx),
            Sheet::Logs => self.logs_sheet(&colors, cx),
            Sheet::InlineSource | Sheet::InlineUpdate | Sheet::InlinePermissions => {
                self.inline_page_sheet(sheet, &colors, window, cx)
            }
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
            Sheet::PromptImage(id, turn, n) => {
                let image = self
                    .session(id)
                    .and_then(|session| session.turns.get(turn))
                    .and_then(|turn| turn.prompt_images.get(n))
                    .cloned();
                if let Some(image) = &image {
                    self.demand_tool_image(id, image);
                }
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
            Sheet::ToolImage(id, turn, n) => {
                let image = self
                    .session(id)
                    .and_then(|session| session.turns.get(turn))
                    .and_then(|turn| turn.images.get(n))
                    .cloned();
                if let Some(image) = &image {
                    self.demand_tool_image(id, image);
                }
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
            Sheet::Call(id, turn, kind, n) => self.call_header(id, turn, kind, n, &colors, cx),
            Sheet::Computers => div()
                .text_size(px(22.))
                .line_height(px(28.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(if cfg!(feature = "local-backend") {
                    "Where Pi runs"
                } else {
                    "Computers"
                }),
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
                    Sheet::Call(id, turn, kind, n) => self
                        .session(id)
                        .and_then(|s| s.turns.get(turn))
                        .and_then(|turn| turn.tool(kind, n))
                        .map_or_else(
                            || "Call".into(),
                            |tool| {
                                format!(
                                    "{} {}",
                                    super::stream::verb(&tool.name, tool.finished),
                                    super::stream::target(tool)
                                )
                                .into()
                            },
                        ),
                    Sheet::Run(..) => "Run so far".into(),
                    Sheet::Delete(_) => "Delete session?".into(),
                    Sheet::RestoreHistory(_, _) => "Restore project files?".into(),
                    Sheet::EnableJj(_) => "Turn on jj file history?".into(),
                    Sheet::Image(_, _) => "Attached image".into(),
                    Sheet::PromptImage(id, turn, n) => self
                        .session(id)
                        .and_then(|session| session.turns.get(turn))
                        .and_then(|turn| turn.prompt_images.get(n))
                        .map_or_else(
                            || "Attached image".into(),
                            |image| image.name.clone().into(),
                        ),
                    Sheet::Logs => "Debug log".into(),
                    Sheet::ToolImage(id, turn, n) => self
                        .session(id)
                        .and_then(|session| session.turns.get(turn))
                        .and_then(|turn| turn.images.get(n))
                        .map_or_else(|| "Image".into(), |image| image.name.clone().into()),
                    Sheet::Computers => "Computers".into(),
                    Sheet::SelectText => "Select text".into(),
                    Sheet::InlineSource => "Page source".into(),
                    Sheet::InlineUpdate => "Update the running page?".into(),
                    Sheet::InlinePermissions => "Local page permissions".into(),
                };
                self.sheet_title(title, &colors, cx)
            }
        };
        div()
            .flex()
            .flex_col()
            .min_h_0()
            .when(matches!(sheet, Sheet::Activity(..)), |body| body.flex_1())
            .child(
                header
                    .flex_none()
                    .px(px(20.))
                    .debug_selector(|| "sheet-header".into()),
            )
            .child(if matches!(sheet, Sheet::Activity(..)) {
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(content)
                    .into_any_element()
            } else {
                crate::scroll::vertical(
                    SharedString::from(format!("sheet-body-{sheet:?}")),
                    &self.sheet_scroll,
                )
                .min_h_0()
                .px(px(20.))
                .child(content)
                .into_any_element()
            })
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
        let known: Vec<_> = self
            .store
            .iter()
            .filter(|s| !s.is_local())
            .flat_map(|s| s.computers.iter().cloned())
            .collect();
        #[cfg(feature = "local-backend")]
        let known = {
            let mut known = known;
            for address in self.prefs(cx).host_keys.keys() {
                if !known.iter().any(|c| &c.address == address) {
                    known.push(crate::model::Computer {
                        name: address.clone(),
                        address: address.clone(),
                        pi_version: None,
                        connected: false,
                    });
                }
            }
            known
        };
        let computers = known
            .into_iter()
            .enumerate()
            .map(|(index, computer)| {
                let current = self
                    .store
                    .as_ref()
                    .is_some_and(|s| !s.is_local() && s.computer.address == computer.address);
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
                            this.cancel_connection_attempt();
                            this.preserve_local_drafts(cx);
                            if this.is_local() {
                                this.start.update(cx, |c, cx| c.clear(cx));
                                this.replace_store(None);
                            }
                            #[cfg(feature = "local-backend")]
                            this.close_provider_accounts();
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
        let menu = div().pb(px(8.));
        #[cfg(feature = "local-backend")]
        let menu = menu.child(
            item("backend-local".into())
                .debug_selector(|| "backend-local".into())
                .child(icon("pi", 20., colors.accent))
                .child(ui::row_text(
                    "Local",
                    Some("Experimental · run Pi on this phone".into()),
                    colors,
                ))
                .when(self.is_local(), |row| {
                    row.child(icon("check", 20., colors.accent))
                })
                .on_click(cx.listener(|this, _, window, cx| this.select_local_backend(window, cx))),
        );
        menu.child(
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
        .when(self.store.is_some() && !self.is_local(), |menu| {
            menu.child(
                link("open-schedules", "calendar", "Schedules")
                    .debug_selector(|| "open-schedules".into())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_schedules(window, cx);
                    })),
            )
        })
        .when(self.is_local(), |menu| {
            menu.child(
                link("local-accounts", "settings", "Local providers").on_click(cx.listener(
                    |this, _, window, cx| {
                        #[cfg(feature = "local-backend")]
                        {
                            let synthetic = this
                                .store
                                .as_ref()
                                .and_then(|s| s.local.as_ref())
                                .is_some_and(|l| l.synthetic);
                            this.open_url(
                                if synthetic {
                                    "pi://test/local-providers/synthetic"
                                } else {
                                    "pi://test/local-providers"
                                },
                                window,
                                cx,
                            );
                        }
                        #[cfg(not(feature = "local-backend"))]
                        let _ = (this, window, cx);
                    },
                )),
            )
        })
        .child(
            link("sample-sessions", "spark", "Try the sample sessions")
                .when(self.store.is_some(), |row| row.border_t_1())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.close_sheet(cx);
                    this.open_sample(window, cx);
                })),
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
    pub(crate) fn session(&self, id: SessionId) -> Option<&Session> {
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
                if self.is_local() {if cfg!(feature="local-backend") {"Hides this saved chat from the Local list. Its SQLite state and workspace files are retained."} else {"Removes this in-memory conversation from this phone. No workspace files exist for local chat."}} else if self.live() {
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
        &mut self,
        id: SessionId,
        turn_index: usize,
        kind: StageKind,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let binding = (id, turn_index, kind);
        let wrap = self.wrap_lines(cx);
        let exact_content_key = !self.sheet_motion.animating();
        let Some(content_key) = self
            .session(id)
            .and_then(|session| session.turns.get(turn_index))
            .and_then(|turn| activity_stage(turn, kind))
            .map(|stage| activity_content_key(stage, kind, exact_content_key))
        else {
            self.activity_view = None;
            return div();
        };
        let reset = self
            .activity_view
            .as_ref()
            .is_none_or(|view| view.binding != binding);
        if reset {
            let stage = self
                .session(id)
                .and_then(|session| session.turns.get(turn_index))
                .and_then(|turn| activity_stage(turn, kind))
                .expect("the activity stage was resolved above");
            self.activity_view = Some(ActivitySheetState::new(
                binding,
                content_key,
                activity_source(stage, kind, None),
                activity_rows(stage, kind),
                wrap,
            ));
        } else {
            let mut view = self.activity_view.take().unwrap();
            if view.content_key != content_key {
                let stage = self
                    .session(id)
                    .and_then(|session| session.turns.get(turn_index))
                    .and_then(|turn| activity_stage(turn, kind))
                    .expect("the activity stage was resolved above");
                let source = activity_source(stage, kind, Some(&view.source));
                if view.source != source {
                    let old_len = view.rows.len();
                    let top = view.list.logical_scroll_top();
                    let anchor = view.rows.get(top.item_ix).cloned();
                    let incremental = incremental_activity_rows(
                        Arc::make_mut(&mut view.rows),
                        &mut view.tool_rows,
                        &view.source,
                        &source,
                        stage,
                    );
                    if let Some((height_delta, changed)) = incremental {
                        view.body_height += height_delta;
                        if old_len == view.rows.len() {
                            view.list.remeasure_items(changed);
                        } else {
                            let mut anchored = top;
                            anchored.item_ix = anchor
                                .as_ref()
                                .and_then(|anchor| view.rows.iter().position(|row| row == anchor))
                                .unwrap_or_else(|| {
                                    top.item_ix.min(view.rows.len().saturating_sub(1))
                                });
                            view.list
                                .reset_with_uniform_height(view.rows.len(), px(20.));
                            if !view.rows.is_empty() {
                                view.list.scroll_to(anchored);
                            }
                        }
                    } else {
                        let rows = activity_rows(stage, kind);
                        let old = view.rows.as_ref();
                        let mut prefix = 0;
                        while prefix < old.len()
                            && prefix < rows.len()
                            && old[prefix] == rows[prefix]
                        {
                            prefix += 1;
                        }
                        let mut suffix = 0;
                        while suffix < old.len().saturating_sub(prefix)
                            && suffix < rows.len().saturating_sub(prefix)
                            && old[old.len() - 1 - suffix] == rows[rows.len() - 1 - suffix]
                        {
                            suffix += 1;
                        }
                        let old_end = old.len() - suffix;
                        let new_end = rows.len() - suffix;
                        if old.len() == rows.len() {
                            if prefix != old_end {
                                view.list.remeasure_items(prefix..new_end);
                            }
                        } else {
                            let mut anchored = top;
                            anchored.item_ix = anchor
                                .as_ref()
                                .and_then(|anchor| rows.iter().position(|row| row == anchor))
                                .unwrap_or_else(|| top.item_ix.min(rows.len().saturating_sub(1)));
                            view.list.reset_with_uniform_height(rows.len(), px(20.));
                            if !rows.is_empty() {
                                view.list.scroll_to(anchored);
                            }
                        }
                        view.body_height = rows.iter().map(activity_row_height).sum();
                        view.tool_rows = activity_tool_rows(&rows);
                        view.rows = Arc::new(rows);
                        view.sync_horizontal_rows();
                    }
                    view.source = source;
                }
                view.content_key = content_key;
            }
            if view.wrap != wrap {
                view.list.remeasure();
                view.wrap = wrap;
            }
            self.activity_view = Some(view);
        }
        let Some(view) = self.activity_view.as_mut() else {
            return div();
        };
        // GPUI invalidates construction-time hints when it learns the list's
        // width. Reapply them once that first layout has established a viewport.
        if !view.hints_applied && view.list.viewport_bounds().size.width > px(0.) {
            let top = view.list.logical_scroll_top();
            view.list
                .reset_with_uniform_height(view.rows.len(), px(20.));
            if !view.rows.is_empty() {
                view.list.scroll_to(top);
            }
            view.hints_applied = true;
        }
        let list_state = view.list.clone();
        let rows = view.rows.clone();
        let horizontal = view.horizontal.clone();
        let offset = list_state.scroll_px_offset_for_scrollbar().y;
        crate::scroll::note_movement(offset - view.offset);
        view.offset = offset;

        let row_count = rows.len();
        let row_colors = *colors;
        let shown_session = id;
        let body = list(
            list_state.clone(),
            cx.processor(move |this, row_index, _, cx| {
                let Some(row) = rows.get(row_index).cloned() else {
                    return div().into_any_element();
                };
                let session = this.session(id);
                let stage = session
                    .and_then(|session| session.turns.get(turn_index))
                    .and_then(|turn| activity_stage(turn, kind));
                match row {
                    ActivityRow::Intro { what } => {
                        let Some(session) = session else {
                            return div().into_any_element();
                        };
                        div()
                            .w_full()
                            .pb(px(12.))
                            .min_w_0()
                            .child(
                                ui::hint(
                                    format!(
                                        "Turn {} · {} · {}",
                                        turn_index + 1,
                                        session.project,
                                        this.computer_name()
                                    ),
                                    &row_colors,
                                )
                                .mb(px(12.)),
                            )
                            .child(div().text_size(px(15.)).child(what))
                            .into_any_element()
                    }
                    ActivityRow::DiffHeader { live } => div()
                        .id("activity-live-diff")
                        .w_full()
                        .debug_selector(|| "activity-live-diff".into())
                        .child(crate::testing::probe("activity-live-diff"))
                        .mt(px(4.))
                        .pt(px(10.))
                        .px(px(12.))
                        .pb(px(6.))
                        .rounded_t(px(12.))
                        .bg(row_colors.panel)
                        .child(ui::label(
                            if live { "LIVE CHANGES" } else { "CHANGES" },
                            &row_colors,
                        ))
                        .into_any_element(),
                    ActivityRow::DiffPath(path) => div()
                        .w_full()
                        .px(px(12.))
                        .pb(px(8.))
                        .bg(row_colors.panel)
                        .font_family(MONO)
                        .text_size(px(12.5))
                        .line_height(px(18.))
                        .text_color(row_colors.muted)
                        .child(path)
                        .into_any_element(),
                    ActivityRow::DiffLine {
                        index,
                        line,
                        gutter,
                        width,
                    } => {
                        let last = stage.is_some_and(|stage| index + 1 == stage.diff.len());
                        let line = div()
                            .id(("activity-diff-line", index))
                            .debug_selector(move || format!("activity-diff-line-{index}"))
                            .relative()
                            .w_full()
                            .when(!wrap, |line| line.min_w(px(width)))
                            .bg(row_colors.panel)
                            .when(last, |line| line.rounded_b(px(12.)).pb(px(8.)))
                            .child(ui::code_line(&line, false, gutter, wrap, &row_colors))
                            .child(this.copyable(line.text.clone(), cx));
                        if wrap {
                            line.into_any_element()
                        } else {
                            let scroll =
                                activity_horizontal_handle(&horizontal, format!("diff:{index}"));
                            div()
                                .id(("activity-diff-pan", index))
                                .w_full()
                                .min_w_0()
                                .max_w_full()
                                .overflow_x_scroll()
                                .restrict_scroll_to_axis()
                                .track_scroll(&scroll)
                                .child(div().min_w(px(width)).child(line))
                                .into_any_element()
                        }
                    }
                    ActivityRow::Reference { index, reference } => {
                        let Some(session) = session else {
                            return div().into_any_element();
                        };
                        let (glyph, target) = match &reference {
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
                                &row_colors,
                            )
                            .debug_selector(move || format!("activity-reference-action-{index}"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_review(id, file, window, cx);
                            }))
                            .into_any_element()
                        } else {
                            ui::tap(("open-reference", index), "copy", &row_colors)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        copied.clone(),
                                    ));
                                    this.notify_user("Copied", cx);
                                }))
                                .into_any_element()
                        };
                        ui::row(("activity-reference", index), index == 0, &row_colors)
                            .debug_selector(move || format!("activity-reference-{index}"))
                            .w_full()
                            .child(icon(glyph, 18., row_colors.muted))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(ui::mono(target.clone(), 12.5)),
                            )
                            .child(action)
                            .into_any_element()
                    }
                    ActivityRow::ToolHeader { index, id } => {
                        let Some(tool) = stage
                            .and_then(|stage| stage.tools.get(index))
                            .filter(|tool| tool.id == id.as_ref())
                        else {
                            return div().into_any_element();
                        };
                        let status = if tool.failed {
                            "Failed"
                        } else if tool.finished {
                            "Finished"
                        } else {
                            "Running…"
                        };
                        div()
                            .debug_selector(move || format!("activity-tool-{index}"))
                            .w_full()
                            .mt(px(12.))
                            .pt(px(14.))
                            .px(px(14.))
                            .pb(px(4.))
                            .rounded_t(px(12.))
                            .bg(row_colors.panel)
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(ui::label(tool.name.clone(), &row_colors).flex_1())
                            .child(ui::hint(status, &row_colors).text_color(if tool.failed {
                                row_colors.coral
                            } else {
                                row_colors.muted
                            }))
                            .into_any_element()
                    }
                    ActivityRow::ToolTargetLine {
                        index,
                        id: tool_id,
                        line_index,
                        line,
                        width,
                        first,
                    } => {
                        let Some(_) = stage
                            .and_then(|stage| stage.tools.get(index))
                            .filter(|tool| tool.id == tool_id.as_ref())
                        else {
                            return div().into_any_element();
                        };
                        let target_line = if wrap {
                            ui::mono(line.clone(), 12.5)
                                .flex_1()
                                .min_w_0()
                                .into_any_element()
                        } else {
                            let scroll = activity_horizontal_handle(
                                &horizontal,
                                format!("{tool_id}:target:{line_index}"),
                            );
                            div()
                                .id(("activity-target-pan", index * 1_000_000 + line_index))
                                .flex_1()
                                .min_w_0()
                                .overflow_x_scroll()
                                .restrict_scroll_to_axis()
                                .track_scroll(&scroll)
                                .child(
                                    div()
                                        .min_w(px(width))
                                        .child(ui::mono(line.clone(), 12.5).whitespace_nowrap()),
                                )
                                .into_any_element()
                        };
                        let copied_tool = tool_id.clone();
                        div()
                            .id(("activity-tool-target", index * 100_000 + line_index))
                            .debug_selector(move || {
                                format!("activity-tool-{index}-target-{line_index}")
                            })
                            .w_full()
                            .px(px(14.))
                            .min_h(px(18.))
                            .bg(row_colors.panel)
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(target_line)
                            .when(first, |row| {
                                row.child(
                                    ui::tap(("copy-command", index), "copy", &row_colors)
                                        .aria_label("Copy command")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            let target = this
                                                .session(shown_session)
                                                .and_then(|session| session.turns.get(turn_index))
                                                .and_then(|turn| activity_stage(turn, kind))
                                                .and_then(|stage| stage.tools.get(index))
                                                .filter(|tool| tool.id == copied_tool.as_ref())
                                                .map(|tool| tool.target.clone())
                                                .unwrap_or_default();
                                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                                target,
                                            ));
                                            this.notify_user("Copied", cx);
                                        })),
                                )
                            })
                            .into_any_element()
                    }
                    ActivityRow::ToolOutputHeader { index, id: tool_id } => {
                        let Some(_) = stage
                            .and_then(|stage| stage.tools.get(index))
                            .filter(|tool| tool.id == tool_id.as_ref())
                        else {
                            return div().into_any_element();
                        };
                        let copied_tool = tool_id.clone();
                        div()
                            .debug_selector(move || format!("activity-tool-{index}-output-header"))
                            .w_full()
                            .pt(px(10.))
                            .px(px(14.))
                            .pb(px(4.))
                            .bg(row_colors.panel)
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(ui::label("Output", &row_colors).flex_1())
                            .child(
                                this.wrap_toggle(("wrap-output", index), cx).debug_selector(
                                    move || format!("activity-wrap-output-{index}"),
                                ),
                            )
                            .child(
                                ui::tap(("copy-output", index), "copy", &row_colors)
                                    .aria_label("Copy output")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let output = this
                                            .session(shown_session)
                                            .and_then(|session| session.turns.get(turn_index))
                                            .and_then(|turn| activity_stage(turn, kind))
                                            .and_then(|stage| stage.tools.get(index))
                                            .filter(|tool| tool.id == copied_tool.as_ref())
                                            .map(|tool| tool.output.clone())
                                            .unwrap_or_default();
                                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                            output,
                                        ));
                                        this.notify_user("Copied output", cx);
                                    })),
                            )
                            .into_any_element()
                    }
                    ActivityRow::ToolOutputLine {
                        index,
                        id,
                        line_index,
                        line,
                        width,
                        first,
                        last,
                    } => {
                        let Some(_) = stage
                            .and_then(|stage| stage.tools.get(index))
                            .filter(|tool| tool.id == id.as_ref())
                        else {
                            return div().into_any_element();
                        };
                        let output_line = div()
                            .id(("activity-output-line", index * 1_000_000 + line_index))
                            .debug_selector(move || {
                                format!("activity-tool-{index}-output-{line_index}")
                            })
                            .relative()
                            .w_full()
                            .min_h(px(18.))
                            .px(px(14.))
                            .when(first, |line| line.pt(px(2.)))
                            .when(last, |line| line.pb(px(14.)).rounded_b(px(12.)))
                            .bg(row_colors.panel)
                            .font_family(MONO)
                            .text_size(px(12.))
                            .line_height(px(18.))
                            .when(!wrap, |line| line.min_w(px(width)).whitespace_nowrap())
                            .text_color(row_colors.plain)
                            .child(super::stream::output_block(line.clone(), &row_colors))
                            .child(this.copyable(line, cx));
                        if wrap {
                            output_line.into_any_element()
                        } else {
                            let output_scroll = format!("{id}:output:{line_index}");
                            let scroll = activity_horizontal_handle(&horizontal, output_scroll);
                            div()
                                .id(("activity-output-pan", index * 1_000_000 + line_index))
                                .debug_selector(move || {
                                    format!("activity-tool-{index}-output-pan-{line_index}")
                                })
                                .w_full()
                                .min_w_0()
                                .max_w_full()
                                .overflow_x_scroll()
                                .restrict_scroll_to_axis()
                                .track_scroll(&scroll)
                                .child(div().min_w(px(width)).child(output_line))
                                .into_any_element()
                        }
                    }
                    ActivityRow::ToolEmpty { index, id } => {
                        let Some(tool) = stage
                            .and_then(|stage| stage.tools.get(index))
                            .filter(|tool| tool.id == id.as_ref())
                        else {
                            return div().into_any_element();
                        };
                        div()
                            .debug_selector(move || format!("activity-tool-{index}-empty"))
                            .w_full()
                            .px(px(14.))
                            .pt(px(4.))
                            .pb(px(14.))
                            .rounded_b(px(12.))
                            .bg(row_colors.panel)
                            .child(ui::hint(
                                if tool.finished {
                                    "No output was recorded."
                                } else {
                                    "Waiting for output…"
                                },
                                &row_colors,
                            ))
                            .into_any_element()
                    }
                    ActivityRow::Empty => {
                        ui::hint("No tool calls recorded for this stage.", &row_colors)
                            .into_any_element()
                    }
                }
            }),
        )
        .size_full();

        debug_assert_eq!(list_state.item_count(), row_count);
        div().flex_1().min_h_0().flex().flex_col().child(
            crate::scroll::virtual_list(
                SharedString::from(format!("activity-{}-{turn_index}-{}", id.0, kind as usize)),
                &list_state,
            )
            .flex_1()
            .min_h_0()
            .px(px(20.))
            .pb(px(12.))
            .child(body),
        )
    }

    /// One call opened from the thread: what it ran, its whole output, what
    /// Pi did next, and the call to copy or ask about.
    fn call_sheet(
        &self,
        id: SessionId,
        turn_index: usize,
        kind: StageKind,
        tool_index: usize,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let Some(session) = self.session(id) else {
            return div();
        };
        let Some(turn) = session.turns.get(turn_index) else {
            return div();
        };
        let Some(tool) = turn.tool(kind, tool_index) else {
            return div();
        };
        let wrap = self.wrap_lines(cx);
        let displayed = SharedString::from(tool.output_for_display().into_owned());
        let command = match tool.name.as_str() {
            "bash" => format!("$ {}", tool.target),
            "grep" | "find" => format!("{} “{}”", tool.name, tool.target),
            _ => tool.target.clone(),
        };
        let next = super::stream::next_call(turn, kind, tool_index);
        let header_output = SharedString::from(tool.output.clone());
        let diff_is_preview = tool.diff_is_preview;
        let asked_command = command.clone();
        let asked_output = displayed.clone();
        let asked_name = format!("{} output", super::stream::target(tool));
        div()
            .pb(px(12.))
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .mt(px(12.))
                    .px(px(12.))
                    .py(px(12.))
                    .rounded(px(12.))
                    .bg(colors.panel)
                    .child(ui::mono(command.clone(), 13.).text_color(colors.text)),
            )
            .when(tool.added > 0 || tool.removed > 0, |body| {
                body.child(
                    ui::hint(
                        format!("{} lines added · {} removed", tool.added, tool.removed),
                        colors,
                    )
                    .mt(px(8.)),
                )
            })
            .when(!tool.diff.is_empty(), |body| {
                body.child(
                    div()
                        .id("call-diff")
                        .debug_selector(|| "call-diff".into())
                        .child(crate::testing::probe("call-diff"))
                        .mt(px(16.))
                        .py(px(8.))
                        .rounded(px(12.))
                        .border_1()
                        .border_color(colors.line)
                        .overflow_hidden()
                        .child(
                            ui::label(
                                if tool.diff_is_preview {
                                    "Requested changes"
                                } else {
                                    "Changes"
                                },
                                colors,
                            )
                            .debug_selector(move || {
                                if diff_is_preview {
                                    "call-diff-requested"
                                } else {
                                    "call-diff-applied"
                                }
                                .into()
                            })
                            .px(px(12.))
                            .mb(px(8.)),
                        )
                        .children(ui::diff_lines(&tool.diff, colors).enumerate().map(
                            |(index, row)| {
                                row.debug_selector(move || format!("call-diff-line-{index}"))
                            },
                        )),
                )
            })
            .child(
                div()
                    .mt(px(20.))
                    .mb(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(ui::label("Output", colors).flex_1())
                    .child(self.wrap_toggle("wrap-call-output", cx))
                    .child(
                        ui::tap("copy-call-output", "copy", colors)
                            .aria_label("Copy output")
                            .debug_selector(|| "copy-call-output".into())
                            .when(header_output.is_empty(), |button| button.opacity(0.5))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !header_output.is_empty() {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        header_output.to_string(),
                                    ));
                                    this.notify_user("Copied output", cx);
                                }
                            })),
                    ),
            )
            .child(
                div()
                    .id("call-output")
                    .debug_selector(|| "call-output".into())
                    .relative()
                    .child(crate::testing::probe("call-output"))
                    .p(px(12.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(colors.line)
                    .bg(colors.canvas)
                    .when(!wrap, |output| output.overflow_x_scroll())
                    .map(|box_| {
                        if displayed.trim().is_empty() {
                            box_.child(ui::hint(
                                if tool.finished {
                                    "No output was recorded."
                                } else {
                                    "Waiting for output…"
                                },
                                colors,
                            ))
                        } else {
                            box_.child(
                                div()
                                    .font_family(MONO)
                                    .text_size(px(12.))
                                    .line_height(px(18.))
                                    .when(!wrap, |text| text.whitespace_nowrap())
                                    .text_color(colors.plain)
                                    .child(super::stream::output_block(displayed.clone(), colors)),
                            )
                        }
                    })
                    .when(!displayed.trim().is_empty(), |output| {
                        output.child(self.copyable(displayed.clone(), cx))
                    }),
            )
            .children(next.map(|(next_kind, next_index, next_tool)| {
                let label = format!(
                    "{} {}",
                    super::stream::verb(&next_tool.name, next_tool.finished),
                    super::stream::target(next_tool)
                );
                div()
                    .id("call-next")
                    .debug_selector(|| "call-next".into())
                    .mt(px(16.))
                    .min_h(px(48.))
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .rounded(px(12.))
                    .bg(colors.panel)
                    .active(|style| style.bg(colors.selected))
                    .child(icon(
                        super::stream::glyph(&next_tool.name),
                        16.,
                        ui::stage_hue(next_kind, colors),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(colors.secondary)
                            .child(if tool.failed {
                                format!("Pi went on to: {label}")
                            } else {
                                format!("Next: {label}")
                            }),
                    )
                    .child(icon("chev_r", 16., colors.faint))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_call_detail(id, turn_index, next_kind, next_index, window, cx)
                    }))
            }))
            .child(
                div()
                    .mt(px(16.))
                    .w_full()
                    .min_w_0()
                    // Synthetic subagent sessions are read-only and have no visible composer.
                    .when(
                        self.store
                            .as_ref()
                            .is_some_and(|store| store.session(id).is_some()),
                        |actions| {
                            actions.child(
                                ui::button(
                                    "ask-about-call",
                                    Button::Plain,
                                    Some("clip"),
                                    "Ask about this",
                                    false,
                                    colors,
                                )
                                .w_full()
                                .min_w_0()
                                .bg(colors.tint(colors.accent))
                                .border_color(gpui::transparent_black())
                                .text_color(colors.accent)
                                .debug_selector(|| "ask-about-call".into())
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        if !this
                                            .store
                                            .as_ref()
                                            .is_some_and(|store| store.session(id).is_some())
                                        {
                                            return;
                                        }
                                        let composer = this.thread_composer(id, window, cx);
                                        let asked = format!("{asked_command}\n\n{asked_output}");
                                        let attachment = crate::composer::Attachment::File {
                                            name: asked_name.clone(),
                                            size: crate::app::size_label(asked.len() as u64),
                                            contents: asked,
                                        };
                                        let attached = composer.update(cx, |composer, cx| {
                                            composer.try_attach(attachment, cx)
                                        });
                                        match attached {
                                            Ok(()) => {
                                                this.close_sheet(cx);
                                                composer.update(cx, |composer, cx| {
                                                    composer.focus(window, cx)
                                                });
                                            }
                                            Err(error) => this.notify_user(error, cx),
                                        }
                                    },
                                )),
                            )
                        },
                    ),
            )
    }

    /// The call sheet's header: the call's icon, what it did, where, and how
    /// it ended.
    fn call_header(
        &self,
        id: SessionId,
        turn_index: usize,
        kind: StageKind,
        tool_index: usize,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let session = self.session(id);
        let tool = session
            .and_then(|session| session.turns.get(turn_index))
            .and_then(|turn| turn.tool(kind, tool_index));
        let (glyph, title, hue, badge) = match tool {
            Some(tool) => {
                let hue = if tool.failed {
                    colors.coral
                } else {
                    ui::stage_hue(kind, colors)
                };
                let badge = match (tool.finished, tool.failed) {
                    (false, _) => ("Running".to_owned(), ui::stage_hue(kind, colors)),
                    (true, true) => (
                        super::stream::exit_code(&tool.output)
                            .map_or_else(|| "Failed".to_owned(), |code| format!("Exit {code}")),
                        colors.coral,
                    ),
                    (true, false) => ("Done".to_owned(), colors.green),
                };
                (
                    super::stream::glyph(&tool.name),
                    format!(
                        "{} {}",
                        super::stream::verb(&tool.name, tool.finished),
                        super::stream::target(tool)
                    ),
                    hue,
                    Some(badge),
                )
            }
            None => ("spark", "Call".to_owned(), colors.muted, None),
        };
        let place = session.map_or_else(String::new, |session| {
            format!("{} · Turn {}", session.folder, turn_index + 1)
        });
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(ui::tile_box(32., 10., hue, colors).child(icon(glyph, 16., hue)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(18.))
                            .line_height(px(24.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(12.5))
                            .line_height(px(16.))
                            .text_color(colors.muted)
                            .truncate()
                            .child(place),
                    ),
            )
            .children(badge.map(|(text, hue)| {
                div()
                    .h(px(24.))
                    .px(px(10.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .rounded(px(12.))
                    .bg(colors.tint(hue))
                    .text_color(hue)
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(text)
            }))
            .child(
                ui::tap("close-sheet", "x", colors)
                    .mr(px(-12.))
                    .on_click(cx.listener(|this, _, _, cx| this.close_sheet(cx))),
            )
    }

    /// The run line pulled up from the working strip: a station per stage,
    /// each opening what Pi did in it.
    fn run_sheet(
        &self,
        id: SessionId,
        turn_index: usize,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let Some(session) = self.session(id) else {
            return div();
        };
        let Some(turn) = session.turns.get(turn_index) else {
            return div();
        };
        let (calls, failed) = turn.call_counts();
        let mut said = vec![crate::model::duration_label(session.elapsed)];
        said.push(if calls == 1 {
            "1 call".to_owned()
        } else {
            format!("{calls} calls")
        });
        if failed > 0 {
            said.push(format!("{failed} failed"));
        }
        let changing = turn
            .stages
            .iter()
            .find(|stage| stage.kind == StageKind::Change)
            .map_or(0, |stage| stage.references.len());
        let live = session.state.is_running() && turn_index + 1 == session.turns.len();
        match (changing, live) {
            (0, _) => {}
            (1, true) => said.push("1 file changing".into()),
            (count, true) => said.push(format!("{count} files changing")),
            (1, false) => said.push("1 file changed".into()),
            (count, false) => said.push(format!("{count} files changed")),
        }
        div()
            .pb(px(16.))
            .min_w_0()
            .child(ui::hint(said.join(" · "), colors).mb(px(20.)))
            .child(self.stations_of(id, Some(session), turn_index, turn, live, true, colors, cx))
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
        match self.store.as_ref().and_then(|store| store.models()) {
            Some(models) => models
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
        let rail_open = self.expanded.contains("details-rail");
        let context_open = self.expanded.contains("details-context");
        let runs_open = self.expanded.contains("details-run");
        let tools_open = self.expanded.contains("details-tools");
        let transport = super::diagnostics::transport_diagnostics(self, id);
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
                    .when(!session.local, |row| {
                        row.child(ui::tap("copy-file", "copy", colors).on_click(cx.listener(
                            move |this, _, _, cx| this.copy(session_file.clone(), "the path", cx),
                        )))
                    }),
            );
        // The run line stays visible; its vertical rail is a disclosure so a
        // long session does not take over Details before the options.
        let running = session.state.is_running();
        let run = session
            .turn()
            .filter(|turn| !turn.run.is_empty())
            .map(|turn| {
                let turn_index = session.turns.len() - 1;
                let stretches = turn.run.len();
                div()
                    .mt(px(20.))
                    .child(ui::run_line(&turn.run, running, 14., colors))
                    .child(
                        row(
                            "details-rail",
                            "layers",
                            "Run details",
                            format!("{stretches} steps"),
                            Some(rail_open),
                        )
                        .mt(px(16.))
                        .on_click(disclosure("details-rail")),
                    )
                    .when(rail_open, |run| {
                        run.child(
                            self.run_stretches(id, turn_index, turn, running, colors, cx)
                                .mt(px(16.)),
                        )
                    })
            });
        let context_detail = |label: &'static str, value: String| {
            div()
                .min_h(px(40.))
                .flex()
                .items_center()
                .gap(px(12.))
                .border_t_1()
                .border_color(colors.line)
                .child(meta(label.into()).flex_1())
                .child(meta(value))
        };
        let used = details.context_percent.unwrap_or(0).min(100) as f32 / 100.;
        let live = self.store.as_ref().and_then(|store| store.live.as_ref());
        let adjustable = live.is_some_and(|live| live.supports_compaction_threshold(id))
            && details.auto_compaction_enabled
            && details.compaction_percent.is_some();
        let slider = adjustable.then(|| {
            let live = live.unwrap();
            let pending = live.compaction_threshold_pending(id);
            let max = live.compaction_threshold_max(id).unwrap_or(95.);
            let app = cx.entity().downgrade();
            crate::threshold_slider::ContextSlider {
                id,
                percent: pending.unwrap_or(details.compaction_percent.unwrap()),
                max,
                used: details.context_percent.map(|used| used.min(100) as u8),
                enabled: live.session_current(id) && pending.is_none() && max >= 10.,
                pending: pending.is_some(),
                colors: *colors,
                on_commit: std::rc::Rc::new(move |percent, _, cx| {
                    let _ = app.update(cx, |this, cx| {
                        // A released gesture must still belong to this sheet/session.
                        if this.sheet != Some(Sheet::Details(id)) {
                            return;
                        }
                        if let Some(live) =
                            this.store.as_mut().and_then(|store| store.live.as_mut())
                            && let Err(error) = live.set_compaction_threshold(id, percent)
                        {
                            this.notify_user(error, cx);
                        }
                        cx.notify();
                    });
                }),
            }
        });
        div()
            .pb(px(8.))
            .children(run)
            .child(div().h(px(20.)))
            .when(files > 0, |body| {
                body.child(
                    row(
                        "details-changes",
                        "diff",
                        "Changes",
                        match files {
                            1 => "1 file".into(),
                            count => format!("{count} files"),
                        },
                        None,
                    )
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.open_review(id, 0, window, cx)),
                    ),
                )
            })
            .children(subagents.map(|summary| {
                row("details-subagents", "fork", "Subagents", summary, None).on_click(cx.listener(
                    move |this, _, window, cx| {
                        this.close_sheet(cx);
                        this.push(Route::Subagents(id), window, cx);
                    },
                ))
            }))
            .child(
                row(
                    "details-context",
                    "layers",
                    "Context",
                    details.context_tokens.clone(),
                    Some(context_open),
                )
                .on_click(disclosure("details-context")),
            )
            .when(context_open, |body| {
                body.child(
                    div()
                        .debug_selector(|| "details-context-body".into())
                        .pl(px(32.))
                        .pt(px(4.))
                        .pb(px(16.))
                        .children(slider)
                        .when(!adjustable, |section| {
                            section
                                .child(
                                    div()
                                        .debug_selector(|| "details-context-usage".into())
                                        .relative()
                                        .mt(px(8.))
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
                                        .children(
                                            details
                                                .compaction_percent
                                                .filter(|_| details.auto_compaction_enabled)
                                                .map(|percent| {
                                                    div()
                                                        .absolute()
                                                        .left(relative(
                                                            (percent / 100.).clamp(0., 1.) as f32,
                                                        ))
                                                        .top(px(-4.))
                                                        .bottom(px(-4.))
                                                        .w(px(2.))
                                                        .rounded(px(1.))
                                                        .bg(colors.line_strong)
                                                }),
                                        ),
                                )
                                .child(
                                    div()
                                        .mt(px(8.))
                                        .flex()
                                        .gap(px(12.))
                                        .child(
                                            meta(details.context_percent.map_or_else(
                                                || "Usage not reported".into(),
                                                |percent| format!("{percent}% used"),
                                            ))
                                            .flex_1(),
                                        )
                                        .child(
                                            meta(compaction_label(
                                                details.auto_compaction_enabled,
                                                details.compaction_percent,
                                            ))
                                            .debug_selector(|| "details-compaction-policy".into()),
                                        ),
                                )
                        })
                        .when(details.background_compacting, |section| {
                            section.child(
                                meta("Compacting in background…".into())
                                    .debug_selector(|| "details-background-compaction".into())
                                    .mt(px(8.)),
                            )
                        })
                        .child(
                            div()
                                .mt(px(12.))
                                .child(
                                    context_detail(
                                        "Cost",
                                        if details.cost.is_empty() {
                                            "—".into()
                                        } else {
                                            details.cost.clone()
                                        },
                                    )
                                    .debug_selector(|| "details-context-cost".into()),
                                )
                                .child(
                                    context_detail("Turns", details.turns.to_string())
                                        .debug_selector(|| "details-context-turns".into()),
                                ),
                        ),
                )
            })
            .child(
                row(
                    "details-model",
                    "spark",
                    "Model",
                    if thinking.is_empty() {
                        model
                    } else {
                        format!("{model} · {thinking}")
                    },
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
            .when_some(transport, |body, entries| {
                body.child(
                    row(
                        "details-transport",
                        "code",
                        "Transport diagnostics",
                        super::diagnostics::transport_summary(entries),
                        None,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_transport_diagnostics(id, window, cx);
                    })),
                )
            })
            .when(session.supports("file_history"), |body| {
                body.child(
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
            })
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
            .schedules
            .draft
            .as_ref()
            .filter(|_| self.route() == Route::ScheduleEdit)
            .map(|draft| (draft.provider.clone(), draft.model_id.clone()))
            .or_else(|| {
                self.store
                    .as_ref()
                    .and_then(|store| store.current_model(self.model_session()))
                    .map(|model| (model.provider.clone(), model.id.clone()))
                    .or_else(|| {
                        self.model_session()
                            .is_none()
                            .then(|| {
                                let prefs = self.prefs(cx);
                                prefs.model_provider.clone().zip(prefs.model_id.clone())
                            })
                            .flatten()
                    })
            });
        let owner = self.store.as_ref().map(|s| s.generation);
        let session_owner = self.model_session();
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
                    .child(crate::testing::probe(format!("model-choice-{index}")))
                    .child(ui::row_text(
                        name.clone(),
                        Some(format!("{detail} · {model_id}").into()),
                        colors,
                    ))
                    .when(on, |row| row.child(icon("check", 20., colors.accent)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.store.as_ref().map(|s| s.generation) != owner
                            || this.model_session() != session_owner
                            || this.sheet != Some(Sheet::Model)
                        {
                            return;
                        }
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
                        if self.route() == Route::ScheduleEdit {
                            "For future occurrences of this schedule."
                        } else if self.model_session().is_some() {
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
            .and_then(|store| store.thinking_levels(self.model_session()))
            .unwrap_or_else(|| THINKING.iter().map(|level| (*level).to_owned()).collect());
        let owner = self.store.as_ref().map(|s| s.generation);
        let session_owner = self.model_session();
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
                    .child(crate::testing::probe(format!("thinking-choice-{index}")))
                    .min_h(px(56.))
                    .child(ui::row_text(level.clone(), Some(detail.into()), colors))
                    .when(selected == level, |row| row.child(icon("check", 20., colors.accent)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.store.as_ref().map(|s|s.generation)!=owner || this.model_session()!=session_owner || this.sheet!=Some(Sheet::Thinking) {return;}
                        this.choose_thinking(&level, cx);
                        this.close_sheet(cx);
                    }))
            })))
            .child(ui::hint("Higher effort can use more tokens and take longer. Available levels depend on the model.", colors).mt(px(12.)))
    }

    fn swarm_toggle_row(
        &self,
        id: SessionId,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        let store = self.store.as_ref();
        let live = store.and_then(|store| store.live.as_ref());
        let sample = store.is_some_and(|store| store.is_sample());
        let on = if sample {
            self.session(id)
                .map(|session| session.details.swarm_enabled.unwrap_or(false))
        } else {
            live.and_then(|live| live.swarm_enabled(id))
        };
        let pending = live.is_some_and(|live| live.swarm_toggle_pending(id));
        let connected = sample || live.is_some_and(|live| live.session_current(id));
        let available = on.is_some() && connected && !pending;
        ui::row("swarm-toggle", true, colors)
            .debug_selector(|| "swarm-toggle".into())
            .child(icon("fork", 20., colors.muted))
            .child(div().flex_1().min_w_0()
                .child(div().text_size(px(15.)).child("Swarm"))
                .child(ui::hint(if pending {
                    "Waiting for the computer…"
                } else if on.is_none() {
                    "This session needs an updated computer helper."
                } else if !connected {
                    "Reconnect to change this session's Swarm setting."
                } else {
                    "Allow peer messaging for new subagent crews. Existing crews continue."
                }, colors).mt(px(4.))))
            .child(match on {
                Some(on) => super::switch(on, colors).when(!available, |switch| switch.opacity(0.5)).into_any_element(),
                None => ui::hint("Unavailable", colors).into_any_element(),
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                if !available { return; }
                let Some(store) = this.store.as_mut() else { return; };
                let enabled = !on.unwrap();
                if let Some(live) = store.live.as_mut() {
                    if let Err(error) = live.set_swarm_enabled(id, enabled) {
                        this.notify_user(error, cx);
                    }
                } else if store.is_sample()
                    && let Some(session) = store.sessions.iter_mut().find(|session| session.id == id)
                {
                    session.details.swarm_enabled = Some(enabled);
                }
                cx.notify();
            }))
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
        if session.supports("file_history") {
            rows.push((
                "clock",
                "File history",
                colors.muted,
                Box::new(move |this, window, cx| this.open_history(id, window, cx)),
            ));
        }
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
        div()
            .pb(px(8.))
            .when(session.supports("swarm"), |body| {
                body.child(
                    ui::card(colors)
                        .mt(px(8.))
                        .child(self.swarm_toggle_row(id, colors, cx)),
                )
            })
            .child(
                ui::card(colors)
                    .mt(px(8.))
                    .children(rows.into_iter().enumerate().map(
                        |(index, (glyph, text, color, action))| {
                            ui::row(("action", index), index == 0, colors)
                                .when(glyph == "info", |row| {
                                    row.debug_selector(|| "rendering-stats-toggle".into())
                                })
                                .child(icon(glyph, 20., color))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(px(15.))
                                        .when(glyph == "stop", |text| text.text_color(colors.coral))
                                        .child(text),
                                )
                                .on_click(
                                    cx.listener(move |this, _, window, cx| {
                                        action(this, window, cx)
                                    }),
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
                    if self.is_local() {"Bundled provider catalog; availability depends on your subscription. Sign-ins stay encrypted on this phone.".into()}
                    else if self.live() {
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

impl PhoneApp {
    /// Each stretch of a run in the order it happened (09 Details): what it
    /// went on and did, how long it took, and how each check ended. A stretch
    /// with calls opens its first one.
    pub(crate) fn run_stretches(
        &self,
        id: SessionId,
        turn_index: usize,
        turn: &Turn,
        running: bool,
        colors: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let count = turn.run.len();
        let timed = turn.run.iter().any(|stretch| !stretch.time.is_zero());
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .children(turn.run.iter().enumerate().map(|(index, stretch)| {
                let last = index + 1 == count;
                let live = running && last;
                let hue = ui::stretch_hue(stretch.kind, colors);
                let (title, detail) = stretch_words(stretch, turn, live, last);
                let failed = stretch.mark == Some(Mark::Failed);
                let first_call = stretch.calls.first().copied();
                let row = div()
                    .id(("stretch", index))
                    .debug_selector(move || format!("run-stretch-{index}"))
                    .relative()
                    .child(crate::testing::probe(format!("run-stretch-{index}")))
                    .flex()
                    .gap(px(12.))
                    .rounded(px(12.))
                    .when(!last, |row| {
                        row.child(
                            div()
                                .absolute()
                                .top(px(32.))
                                .bottom(px(-12.))
                                .left(px(12.5))
                                .w(px(3.))
                                .rounded(px(2.))
                                .bg(hue),
                        )
                    })
                    .child(ui::station(
                        stretch.kind.glyph(),
                        hue,
                        if live {
                            StageStatus::Live
                        } else {
                            StageStatus::Done
                        },
                        colors,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .pt(px(3.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(title),
                                    )
                                    .children(stretch.mark.and_then(|mark| match mark {
                                        Mark::Passed => {
                                            Some(result_badge("Passed", colors.green, colors))
                                        }
                                        Mark::Failed => {
                                            Some(result_badge("Failed", colors.coral, colors))
                                        }
                                        Mark::Answered => None,
                                    }))
                                    .when(timed, |row| {
                                        row.child(
                                            div()
                                                .flex_none()
                                                .text_size(px(12.5))
                                                .text_color(colors.muted)
                                                .child(duration_label(stretch.time)),
                                        )
                                    }),
                            )
                            .children(detail.map(|detail| {
                                div()
                                    .mt(px(2.))
                                    .text_size(px(13.))
                                    .line_height(px(18.))
                                    .text_color(if failed { colors.coral } else { colors.muted })
                                    .when(stretch.kind == StretchKind::Check, |line| {
                                        line.font_family(MONO).text_size(px(12.))
                                    })
                                    .truncate()
                                    .child(detail)
                            })),
                    );
                match first_call {
                    Some((kind, tool)) => row
                        .active(|style| style.bg(colors.selected))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_call_detail(id, turn_index, kind, tool, window, cx)
                        }))
                        .into_any_element(),
                    None => row.into_any_element(),
                }
            }))
    }
}

/// A check's result beside its stretch.
fn result_badge(text: &'static str, hue: gpui::Hsla, colors: &Theme) -> Div {
    div()
        .h(px(22.))
        .px(px(8.))
        .flex()
        .flex_none()
        .items_center()
        .rounded_full()
        .bg(colors.tint(hue))
        .text_color(hue)
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(text)
}

/// A stretch's name and what it did, from its calls: "Looked" and "Read 5
/// files · searched twice"; "Checked" and the command, or why it failed.
fn stretch_words(
    stretch: &Stretch,
    turn: &Turn,
    live: bool,
    last: bool,
) -> (SharedString, Option<String>) {
    let tools: Vec<&crate::model::ToolActivity> = stretch
        .calls
        .iter()
        .filter_map(|(kind, index)| turn.tool(*kind, *index))
        .collect();
    let named = |done: &'static str, doing: &'static str| -> SharedString {
        if live { doing } else { done }.into()
    };
    match stretch.kind {
        StretchKind::Look if tools.is_empty() && last && !live => {
            ("Wrote the hand-off".into(), None)
        }
        StretchKind::Look => {
            let count = |names: &[&str]| {
                tools
                    .iter()
                    .filter(|tool| names.contains(&tool.name.as_str()))
                    .count()
            };
            let parts: Vec<String> = [
                (count(&["read"]), "read", "file", "files"),
                (count(&["grep", "find"]), "searched", "time", "times"),
                (count(&["ls"]), "listed", "folder", "folders"),
                (count(&["bash", "javascript"]), "ran", "command", "commands"),
            ]
            .into_iter()
            .filter(|(count, ..)| *count > 0)
            .map(|(count, verb, one, many)| match (verb, count) {
                ("searched", 1) => "searched once".to_owned(),
                ("searched", 2) => "searched twice".to_owned(),
                (verb, 1) => format!("{verb} 1 {one}"),
                (verb, count) => format!("{verb} {count} {many}"),
            })
            .collect();
            let detail = if parts.is_empty() {
                "Thinking".to_owned()
            } else {
                let mut detail = parts.join(" · ");
                detail[..1].make_ascii_uppercase();
                detail
            };
            (named("Looked", "Looking"), Some(detail))
        }
        StretchKind::Change => {
            let mut names: Vec<&str> = Vec::new();
            for tool in &tools {
                let name = tool.target.rsplit('/').next().unwrap_or(&tool.target);
                if !name.is_empty() && !names.contains(&name) {
                    names.push(name);
                }
            }
            let (added, removed) = tools.iter().fold((0, 0), |(added, removed), tool| {
                (added + tool.added, removed + tool.removed)
            });
            let mut detail = match names.len() {
                0 => String::new(),
                1 | 2 => names.join(", "),
                more => format!("{}, +{}", names[..2].join(", "), more - 2),
            };
            if added > 0 || removed > 0 {
                if !detail.is_empty() {
                    detail.push_str(" · ");
                }
                detail.push_str(&match (added, removed) {
                    (added, 0) => format!("+{added}"),
                    (0, removed) => format!("−{removed}"),
                    (added, removed) => format!("+{added} −{removed}"),
                });
            }
            (
                named("Changed", "Changing"),
                (!detail.is_empty()).then_some(detail),
            )
        }
        StretchKind::Check => {
            let command = tools
                .first()
                .map(|tool| tool.target.lines().next().unwrap_or("").trim().to_owned());
            let failure = tools
                .iter()
                .find(|tool| tool.failed)
                .map(|tool| crate::screens::stream::failure(tool));
            let detail = match (command, failure) {
                (Some(command), Some(failure)) => Some(format!(
                    "{command} · {}",
                    failure.trim_start_matches("Failed · ")
                )),
                (command, failure) => command.or(failure),
            };
            (named("Checked", "Checking"), detail)
        }
        StretchKind::Wait => (
            named("Waited for you", "Waiting for you"),
            Some(
                if live {
                    "Pi is waiting for your answer"
                } else {
                    "You answered"
                }
                .to_owned(),
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{ActivityRow, activity_content_key, activity_stage, compaction_label, shorten};
    use crate::model::{Stage, StageKind, StageStatus, ToolActivity, Turn};

    #[test]
    fn bounded_activity_key_changes_when_repetitive_output_grows() {
        let mut stage = Stage::new(StageKind::Verify, StageStatus::Live, "Checking");
        stage.tools.push(ToolActivity {
            id: "command".into(),
            name: "bash".into(),
            target: "yes x".into(),
            output: "x".repeat(4_096),
            finished: false,
            failed: false,
            added: 0,
            removed: 0,
            diff: Vec::new(),
            diff_is_preview: false,
        });
        let before = activity_content_key(&stage, StageKind::Verify, false);
        stage.tools[0].output.push('x');
        assert_ne!(
            before,
            activity_content_key(&stage, StageKind::Verify, false)
        );
    }

    #[gpui::test]
    fn the_session_swarm_toggle_does_not_change_existing_crews(cx: &mut gpui::TestAppContext) {
        use crate::{
            app::{PhoneApp, Sheet},
            model::SessionId,
        };
        use gpui::{px, size};
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        for width in [320., 412.] {
            cx.simulate_resize(size(px(width), px(640.)));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.preview("subagents", window, cx);
                    app.open_sheet(Sheet::More(SessionId(1)), cx);
                    app.sheet_motion.finish();
                })
            });
            cx.run_until_parked();
            let before = app.read_with(cx, |app, _| app.store.as_ref().unwrap().sessions.clone());
            for enabled in [true, false] {
                let toggle = cx.debug_bounds("swarm-toggle").unwrap();
                assert!(toggle.left() >= px(0.) && toggle.right() <= px(width));
                cx.simulate_click(toggle.center(), gpui::Modifiers::none());
                cx.run_until_parked();
                app.read_with(cx, |app, _| {
                    let mut expected = before.clone();
                    expected
                        .iter_mut()
                        .find(|session| session.id == SessionId(1))
                        .unwrap()
                        .details
                        .swarm_enabled = Some(enabled);
                    assert_eq!(app.store.as_ref().unwrap().sessions, expected);
                });
            }
        }
    }

    #[gpui::test]
    fn changed_sheet_omits_redundant_edit_actions_and_aligns_review_rows(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::{
            app::PhoneApp,
            model::{Reference, SessionId},
        };
        use gpui::{px, size};
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(size(px(384.), px(854.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("diff-spacing", window, cx);
                let session = app
                    .store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|session| session.id == SessionId(1))
                    .unwrap();
                let template = session.files[0].clone();
                let paths = ["short.rs", "crates/pi_android/src/screens/sheets.rs"];
                session.files = paths
                    .iter()
                    .map(|path| {
                        let mut file = template.clone();
                        file.path = (*path).into();
                        file
                    })
                    .collect();
                let stage = session.turns[0].stage_mut(StageKind::Change);
                stage.diff.truncate(1);
                stage.references = paths
                    .iter()
                    .map(|path| Reference::File((*path).into()))
                    .collect();
                app.sheet_motion.finish();
            })
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("activity-live-diff").is_some());
        assert!(cx.debug_bounds("activity-tool-0").is_none());

        let rows = ["activity-reference-0", "activity-reference-1"].map(|selector| {
            cx.debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing {selector}"))
        });
        let actions =
            ["activity-reference-action-0", "activity-reference-action-1"].map(|selector| {
                cx.debug_bounds(selector)
                    .unwrap_or_else(|| panic!("missing {selector}"))
            });
        assert!(
            rows.windows(2)
                .all(|rows| rows[0].left() == rows[1].left() && rows[0].right() == rows[1].right())
        );
        assert!(actions.windows(2).all(|actions| {
            actions[0].left() == actions[1].left() && actions[0].right() == actions[1].right()
        }));
    }

    #[gpui::test]
    fn looked_sheet_virtualizes_large_tool_output(cx: &mut gpui::TestAppContext) {
        use crate::{app::PhoneApp, model::SessionId};
        use gpui::{px, size};
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(size(px(384.), px(854.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("tool-output", window, cx);
                let stage = app
                    .store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|session| session.id == SessionId(1))
                    .unwrap()
                    .turns[0]
                    .stage_mut(StageKind::Verify);
                let mut tool = stage
                    .tools
                    .iter()
                    .find(|tool| tool.id == "test-command")
                    .unwrap()
                    .clone();
                tool.output = (0..2_000)
                    .map(|line| format!("output line {line}: {}", "x".repeat(120)))
                    .collect::<Vec<_>>()
                    .join("\n");
                stage.tools = vec![tool];
                app.sheet_motion.finish();
                cx.notify();
            })
        });
        cx.run_until_parked();
        let full_width_rows = [
            "activity-tool-0",
            "activity-tool-0-target-0",
            "activity-tool-0-output-header",
            "activity-tool-0-output-0",
        ]
        .map(|selector| {
            cx.debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing {selector}"))
        });
        assert!(full_width_rows.windows(2).all(|rows| {
            rows[0].left() == rows[1].left() && rows[0].right() == rows[1].right()
        }));
        let wrap = cx.debug_bounds("activity-wrap-output-0").unwrap();
        cx.simulate_click(wrap.center(), gpui::Modifiers::none());
        cx.run_until_parked();

        assert!(cx.debug_bounds("activity-tool-0").is_some());
        assert!(cx.debug_bounds("activity-tool-0-output-0").is_some());
        assert!(
            cx.debug_bounds("activity-tool-0-output-1500").is_none(),
            "off-screen output rows must not be mounted"
        );
        let (list, retained_rows) = app.read_with(cx, |app, _| {
            let view = app.activity_view.as_ref().unwrap();
            assert!(view.list.item_count() > 2_000);
            assert!(view.list.viewport_bounds().size.height > px(0.));
            assert!(view.list.max_offset_for_scrollbar().y > px(0.));
            assert!(
                view.horizontal.borrow()["test-command:output:0"]
                    .max_offset()
                    .x
                    > px(0.)
            );
            (view.list.clone(), view.rows.clone())
        });
        cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(std::sync::Arc::ptr_eq(
                &retained_rows,
                &app.activity_view.as_ref().unwrap().rows
            ));
        });
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                let tool = app
                    .store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|session| session.id == SessionId(1))
                    .unwrap()
                    .turns[0]
                    .stage_mut(StageKind::Verify)
                    .tools
                    .iter_mut()
                    .find(|tool| tool.id == "test-command")
                    .unwrap();
                tool.output.replace_range(..1, "X");
                cx.notify();
            })
        });
        cx.run_until_parked();
        let (first_line, old_count, rows_allocation) = app.read_with(cx, |app, _| {
            let rows = &app.activity_view.as_ref().unwrap().rows;
            assert!(!std::sync::Arc::ptr_eq(&retained_rows, rows));
            let first = rows
                .iter()
                .find_map(|row| match row {
                    ActivityRow::ToolOutputLine {
                        line_index: 0,
                        line,
                        ..
                    } => Some(line.clone()),
                    _ => None,
                })
                .unwrap();
            assert!(first.starts_with('X'));
            (first, rows.len(), std::sync::Arc::as_ptr(rows) as usize)
        });
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                let tool = app
                    .store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|session| session.id == SessionId(1))
                    .unwrap()
                    .turns[0]
                    .stage_mut(StageKind::Verify)
                    .tools
                    .iter_mut()
                    .find(|tool| tool.id == "test-command")
                    .unwrap();
                tool.output.push_str("\nstreamed tail");
                cx.notify();
            })
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            let rows = &app.activity_view.as_ref().unwrap().rows;
            assert_eq!(std::sync::Arc::as_ptr(rows) as usize, rows_allocation);
            assert_eq!(rows.len(), old_count + 1);
            let (retained, tail) = rows
                .iter()
                .filter_map(|row| match row {
                    ActivityRow::ToolOutputLine { line, .. } => Some(line),
                    _ => None,
                })
                .fold((None, None), |(first, _), line| {
                    (first.or(Some(line)), Some(line))
                });
            assert_eq!(retained.unwrap().as_ptr(), first_line.as_ptr());
            assert_eq!(tail.unwrap().as_ref(), "streamed tail");
        });
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                let tool = app
                    .store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|session| session.id == SessionId(1))
                    .unwrap()
                    .turns[0]
                    .stage_mut(StageKind::Verify)
                    .tools
                    .iter_mut()
                    .find(|tool| tool.id == "test-command")
                    .unwrap();
                tool.output.push_str("\rupdated tail");
                cx.notify();
            })
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            let rows = &app.activity_view.as_ref().unwrap().rows;
            assert_eq!(rows.len(), old_count + 1);
            let output: Vec<_> = rows
                .iter()
                .filter_map(|row| match row {
                    ActivityRow::ToolOutputLine { line, .. } => Some(line),
                    _ => None,
                })
                .collect();
            assert_eq!(output[0].as_ptr(), first_line.as_ptr());
            assert_eq!(output.last().unwrap().as_ref(), "updated tail");
        });
        let header = cx.debug_bounds("activity-tool-0").unwrap();
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: header.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-500.))),
            modifiers: gpui::Modifiers::none(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.run_until_parked();
        assert!(
            list.logical_scroll_top().item_ix > 10,
            "one swipe must use off-screen row height hints"
        );
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: header.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-100_000.))),
            modifiers: gpui::Modifiers::none(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("activity-tool-0-output-1999").is_some());
        assert!(
            cx.debug_bounds("activity-tool-0-output-0").is_none(),
            "early output rows must unmount after a distant swipe"
        );

        list.scroll_to(gpui::ListOffset {
            item_ix: 0,
            offset_in_item: px(0.),
        });
        cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
        cx.run_until_parked();
        let first = cx.debug_bounds("activity-tool-0-output-pan-0").unwrap();
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: first.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(-200.), px(0.))),
            modifiers: gpui::Modifiers::none(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(
                app.activity_view.as_ref().unwrap().horizontal.borrow()["test-command:output:0"]
                    .offset()
                    .x
                    < px(0.)
            );
        });

        for output in ["", "10%\r50%"] {
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.store
                        .as_mut()
                        .unwrap()
                        .sessions
                        .iter_mut()
                        .find(|session| session.id == SessionId(1))
                        .unwrap()
                        .turns[0]
                        .stage_mut(StageKind::Verify)
                        .tools
                        .iter_mut()
                        .find(|tool| tool.id == "test-command")
                        .unwrap()
                        .output = output.into();
                    cx.notify();
                })
            });
            cx.run_until_parked();
        }
        app.read_with(cx, |app, _| {
            let lines: Vec<_> = app
                .activity_view
                .as_ref()
                .unwrap()
                .rows
                .iter()
                .filter_map(|row| match row {
                    ActivityRow::ToolOutputLine { line, .. } => Some(line.as_ref()),
                    _ => None,
                })
                .collect();
            assert_eq!(lines, ["50%"]);
        });
    }

    #[gpui::test]
    fn diff_numbers_keep_card_padding_and_a_shared_code_column(cx: &mut gpui::TestAppContext) {
        use crate::{app::PhoneApp, model::SessionId};
        use gpui::{px, size};
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        for width in [320., 412.] {
            cx.simulate_resize(size(px(width), px(915.)));
            for (fixture, card) in [
                ("diff-spacing", "activity-live-diff"),
                ("call-diff-spacing", "call-diff"),
            ] {
                // Cross digit boundaries, including the largest supported number.
                for start in [998, 99_998, u32::MAX - 4] {
                    cx.update(|window, cx| {
                        app.update(cx, |app, cx| {
                            app.preview(fixture, window, cx);
                            let stage = app
                                .store
                                .as_mut()
                                .unwrap()
                                .sessions
                                .iter_mut()
                                .find(|session| session.id == SessionId(1))
                                .unwrap()
                                .turns[0]
                                .stage_mut(StageKind::Change);
                            for (index, line) in stage.diff.iter_mut().enumerate() {
                                line.number = if index == 0 { 0 } else { start + index as u32 };
                            }
                            stage.tools[0].diff = stage.diff.clone();
                            app.sheet_motion.finish();
                            cx.notify();
                        })
                    });
                    cx.run_until_parked();
                    let card = cx.debug_bounds(card).unwrap();
                    let mut code_left = None;
                    for number in [0, start + 1, start + 2, start + 3, start + 4] {
                        if fixture == "diff-spacing"
                            && cx
                                .debug_bounds(format!("diff-number-{number}").leak())
                                .is_none()
                        {
                            let (list, row) = app.read_with(cx, |app, _| {
                                let view = app.activity_view.as_ref().unwrap();
                                let row = view
                                    .rows
                                    .iter()
                                    .position(|row| {
                                        matches!(row, ActivityRow::DiffLine { line, .. } if line.number == number)
                                    })
                                    .unwrap();
                                (view.list.clone(), row)
                            });
                            list.scroll_to(gpui::ListOffset {
                                item_ix: row,
                                offset_in_item: px(0.),
                            });
                            cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
                            cx.run_until_parked();
                        }
                        let digits = cx
                            .debug_bounds(format!("diff-number-{number}").leak())
                            .unwrap_or_else(|| panic!("missing number {number} in {fixture}"));
                        let code = cx
                            .debug_bounds(format!("diff-code-{number}").leak())
                            .unwrap_or_else(|| panic!("missing code {number} in {fixture}"));
                        assert!(
                            digits.left() >= card.left() + px(12.),
                            "{fixture}: {number} touches the border"
                        );
                        assert!(
                            digits.size.height <= px(20.),
                            "line numbers must never wrap"
                        );
                        assert!(
                            code.left() - digits.right() >= px(10.),
                            "keep the gap before code"
                        );
                        assert!(
                            code.right() <= card.right(),
                            "wrapped code stays inside the card"
                        );
                        if let Some(left) = code_left {
                            assert_eq!(code.left(), left, "all code rows must align");
                        }
                        code_left = Some(code.left());
                    }
                }
            }
        }
    }

    #[gpui::test]
    fn an_open_call_shows_its_full_diff_and_updates_without_reopening(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::{
            app::{PhoneApp, Sheet},
            model::{DiffLine, LineKind, SessionId},
        };
        use gpui::{px, size};
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(size(px(384.), px(854.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("done", window, cx);
                let session = app
                    .store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|s| s.id == SessionId(1))
                    .unwrap();
                let tool = &mut session.turns[0].stage_mut(StageKind::Change).tools[0];
                tool.diff = (0..24)
                    .map(|n| DiffLine::new(LineKind::Added, 0, &format!("new_{n}();")))
                    .collect();
                tool.diff_is_preview = true;
                tool.finished = false;
                // A different call owns the stage summary; never show that here.
                session.turns[0].stage_mut(StageKind::Change).diff.clear();
                app.open_sheet(Sheet::Call(SessionId(1), 0, StageKind::Change, 0), cx);
                app.sheet_motion.finish();
            });
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("call-diff").is_some());
        assert!(cx.debug_bounds("call-diff-requested").is_some());
        assert!(
            cx.debug_bounds("call-diff-line-23").is_some(),
            "do not truncate to eight lines"
        );
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                let session = app
                    .store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|s| s.id == SessionId(1))
                    .unwrap();
                let tool = &mut session.turns[0].stage_mut(StageKind::Change).tools[0];
                tool.diff = vec![DiffLine::new(LineKind::Added, 42, "applied();")];
                tool.diff_is_preview = false;
                tool.finished = true;
                cx.notify();
            })
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("call-diff-applied").is_some());
        assert!(cx.debug_bounds("call-diff-requested").is_none());
        assert!(cx.debug_bounds("call-diff-line-0").is_some());
        assert!(cx.debug_bounds("call-diff-line-23").is_none());
    }

    #[gpui::test]
    fn long_call_output_can_be_copied_from_the_top_or_selected_by_holding(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::{
            app::{PhoneApp, Sheet},
            model::SessionId,
        };
        use gpui::{InputEvent, point, px, size};
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        cx.simulate_resize(size(px(384.), px(854.)));
        let output = "test passed — café 中文\n".repeat(120);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.preview("call", window, cx);
                let session = app
                    .store
                    .as_mut()
                    .unwrap()
                    .sessions
                    .iter_mut()
                    .find(|s| s.id == SessionId(1))
                    .unwrap();
                session.turns[0].stage_mut(StageKind::Verify).tools[0].output = output.clone();
                app.sheet_motion.finish();
            });
        });
        cx.run_until_parked();
        let button = cx
            .debug_bounds("copy-call-output")
            .expect("copy beside the output heading");
        assert!(button.top() > px(0.) && button.bottom() < px(854.));
        cx.simulate_click(button.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some(output.as_str())
        );
        let output_bounds = cx.debug_bounds("call-output").unwrap();
        let position = output_bounds.origin + point(px(40.), px(30.));
        for phase in [gpui::TouchPhase::Started, gpui::TouchPhase::Ended] {
            cx.update(|window, cx| {
                window.dispatch_event(
                    gpui::TouchEvent {
                        id: gpui::TouchId(95),
                        position,
                        phase,
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                );
            });
            cx.run_until_parked();
            if phase == gpui::TouchPhase::Started {
                cx.executor()
                    .advance_clock(std::time::Duration::from_millis(501));
                cx.run_until_parked();
            }
        }
        app.read_with(cx, |app, cx| {
            assert_eq!(app.sheet, Some(Sheet::SelectText));
            assert_eq!(app.selectable.read(cx).text(), output);
        });
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.sheet_motion.finish();
                cx.notify();
            })
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("text-menu").is_some(),
            "the first selection must show Copy without an extra tap"
        );
    }

    #[gpui::test]
    fn ask_about_call_fits_the_footer_on_narrow_screens(cx: &mut gpui::TestAppContext) {
        use crate::app::PhoneApp;
        use gpui::{point, px, size};
        let (app, cx) = cx.add_window_view(|window, cx| PhoneApp::new(None, window, cx));
        for (width, height) in [(320., 640.), (384., 854.), (640., 360.)] {
            cx.simulate_resize(size(px(width), px(height)));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.preview("call", window, cx);
                    app.sheet_motion.finish();
                    app.sheet_scroll.set_offset(point(px(0.), px(-100_000.)));
                    cx.notify();
                })
            });
            cx.run_until_parked();
            let ask = cx.debug_bounds("ask-about-call").unwrap();
            assert!(ask.left() >= px(0.) && ask.right() <= px(width));
            assert!(ask.top() >= px(0.) && ask.bottom() <= px(height));
            assert!(
                ask.size.width > px(width - 60.),
                "Ask gets the full footer width"
            );
        }
    }

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
    fn compaction_labels_never_invent_an_unreported_threshold() {
        assert_eq!(
            compaction_label(true, Some(81.9294)),
            "auto-compacts above 81.9%"
        );
        assert_eq!(
            compaction_label(false, Some(81.9294)),
            "auto-compaction off"
        );
        assert_eq!(compaction_label(true, None), "auto-compaction on");
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
