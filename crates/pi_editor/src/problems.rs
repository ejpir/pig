//! What the language server says about the code under the mouse or cursor: the
//! diagnostic there, the hover text and the first quick fix. Zed draws these as
//! separate popovers; the desktop draws one card (design study 05), so Zed's own
//! hover popover is turned off in [`super::services`].
//!
//! Zed's editor keeps its mouse-to-text mapping private. [`point_at`] and
//! [`below`] repeat the arithmetic of its element from the public pieces:
//! `last_bounds`, the gutter dimensions, the scroll position and the line layout.
use crate::EditorProject;
use anyhow::Result;
use editor::{DisplayPoint, Editor, EditorSnapshot, ToPoint as _, display_map::DisplayRow};
use gpui::{App, Entity, Pixels, Point, Task, Window, point, px};
use language::{Anchor, Bias, Buffer, BufferSnapshot, CharKind, Point as TextPoint};
use lsp::DiagnosticSeverity;
use project::CodeAction;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub range: Range<Anchor>,
    /// Zero-based, like [`TextPoint`].
    pub start: TextPoint,
    pub message: String,
    /// Source and code as `ts(18048)`.
    pub code: Option<String>,
    pub error: bool,
}

/// One block of the language server's hover text.
#[derive(Clone, Debug, PartialEq)]
pub struct HoverText {
    pub text: String,
    pub code: bool,
}

#[derive(Clone)]
pub struct QuickFix {
    pub title: String,
    action: CodeAction,
}

/// The text position under a window position, when it is over this editor's text.
pub fn point_at(
    editor: &Entity<Editor>,
    position: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> Option<TextPoint> {
    editor.update(cx, |editor, cx| {
        let bounds = *editor.last_bounds()?;
        if !bounds.contains(&position) {
            return None;
        }
        let layout = Layout::of(editor, window, cx)?;
        let local = position - layout.origin;
        if local.x < px(0.) {
            return None;
        }
        let row = (f64::from(local.y / layout.line_height) + layout.scroll.y).floor();
        if row < 0. || row > f64::from(layout.snapshot.max_point().row().0) {
            return None;
        }
        let row = DisplayRow(row as u32);
        let x = local.x + layout.scroll_x;
        let details = editor.text_layout_details(window, cx);
        let snapshot = &layout.snapshot;
        let end = DisplayPoint::new(row, snapshot.line_len(row));
        if x > snapshot.x_for_display_point(end, &details) {
            return None;
        }
        // The closest boundary can be after the character under the mouse.
        let mut column = snapshot.display_column_for_x(row, x, &details);
        if column > 0 && snapshot.x_for_display_point(DisplayPoint::new(row, column), &details) > x
        {
            column -= 1;
        }
        let point = snapshot.display_point_to_point(DisplayPoint::new(row, column), Bias::Left);
        Some(point)
    })
}

/// Where the newest cursor is.
pub fn cursor_point(editor: &Entity<Editor>, cx: &App) -> TextPoint {
    let editor = editor.read(cx);
    let head = editor.selections.newest_anchor().head();
    head.to_point(&editor.buffer().read(cx).snapshot(cx))
}

/// The window position just below the line of `point`, where a card for it goes.
pub fn below(
    editor: &Entity<Editor>,
    text: TextPoint,
    window: &mut Window,
    cx: &mut App,
) -> Option<Point<Pixels>> {
    editor.update(cx, |editor, cx| {
        let layout = Layout::of(editor, window, cx)?;
        let display = layout.snapshot.point_to_display_point(text, Bias::Left);
        let at = editor.display_to_pixel_point(display, &layout.snapshot, window, cx)?;
        Some(layout.origin + point(at.x - layout.scroll_x, at.y + layout.line_height))
    })
}

struct Layout {
    snapshot: EditorSnapshot,
    /// The window position of the text's top-left corner, after the gutter.
    origin: Point<Pixels>,
    line_height: Pixels,
    scroll: Point<f64>,
    scroll_x: Pixels,
}

impl Layout {
    fn of(editor: &mut Editor, window: &mut Window, cx: &mut App) -> Option<Self> {
        let bounds = *editor.last_bounds()?;
        let snapshot = editor.snapshot(window, cx);
        let style = editor.style(cx).clone();
        let font_id = window.text_system().resolve_font(&style.text.font());
        let font_size = style.text.font_size.to_pixels(window.rem_size());
        let line_height = style.text.line_height_in_pixels(window.rem_size());
        let em_advance = window.text_system().em_advance(font_id, font_size).ok()?;
        let gutter = snapshot.gutter_dimensions(font_id, font_size, &style, window, cx);
        let scroll = snapshot.scroll_position();
        let scroll = point(scroll.x, scroll.y);
        Some(Self {
            origin: bounds.origin + point(gutter.width + gutter.margin, px(0.)),
            line_height,
            scroll_x: px((scroll.x * f64::from(em_advance)) as f32),
            scroll,
            snapshot,
        })
    }
}

/// The most severe error, warning or note at `offset`; hints are left to the editor.
pub fn problem_at(buffer: &BufferSnapshot, offset: usize) -> Option<Problem> {
    buffer
        .diagnostics_in_range::<_, usize>(offset..offset, false)
        .filter(|entry| {
            let range = &entry.range;
            let covers = range.start <= offset && offset < range.end;
            entry.diagnostic.is_primary
                && entry.diagnostic.severity <= DiagnosticSeverity::INFORMATION
                && (covers || range.start == range.end && range.start == offset)
        })
        .min_by_key(|entry| entry.diagnostic.severity)
        .map(|entry| {
            let diagnostic = entry.diagnostic;
            let code = diagnostic.code.as_ref().map(|code| match code {
                lsp::NumberOrString::Number(n) => n.to_string(),
                lsp::NumberOrString::String(s) => s.clone(),
            });
            Problem {
                range: buffer.anchor_before(entry.range.start)
                    ..buffer.anchor_after(entry.range.end),
                start: buffer.offset_to_point(entry.range.start),
                message: diagnostic.message.as_ref().trim().to_owned(),
                code: code_label(diagnostic.source.as_deref(), code.as_deref()),
                error: diagnostic.severity == DiagnosticSeverity::ERROR,
            }
        })
}

/// `ts(18048)` from source `ts` and code `18048`, as the study shows diagnostics.
fn code_label(source: Option<&str>, code: Option<&str>) -> Option<String> {
    match (source, code) {
        (Some(source), Some(code)) => Some(format!("{source}({code})")),
        (None, Some(code)) => Some(code.to_owned()),
        (Some(source), None) => Some(source.to_owned()),
        (None, None) => None,
    }
}

/// The identifier around `offset`, which a plain hover is about; `None` over
/// whitespace and punctuation.
pub fn word_at(buffer: &BufferSnapshot, offset: usize) -> Option<Range<usize>> {
    let (range, kind) = buffer.surrounding_word(offset, None);
    (kind == Some(CharKind::Word) && !range.is_empty()).then_some(range)
}

impl EditorProject {
    /// The language server's hover text at `offset`.
    pub fn hover_text(
        &self,
        buffer: &Entity<Buffer>,
        offset: usize,
        cx: &mut App,
    ) -> Task<Vec<HoverText>> {
        let task = self.project.update(cx, |p, cx| p.hover(buffer, offset, cx));
        cx.background_executor().spawn(async move {
            task.await
                .unwrap_or_default()
                .into_iter()
                .flat_map(|hover| hover.contents)
                .filter(|block| !block.text.trim().is_empty())
                .map(|block| HoverText {
                    text: block.text.trim().to_owned(),
                    code: matches!(block.kind, project::HoverBlockKind::Code { .. }),
                })
                .collect()
        })
    }

    /// The language server's first quick fix for `range`.
    pub fn quick_fix(
        &self,
        buffer: &Entity<Buffer>,
        range: Range<Anchor>,
        cx: &mut App,
    ) -> Task<Option<QuickFix>> {
        let task = self
            .project
            .update(cx, |p, cx| p.code_actions(buffer, range, None, cx));
        cx.background_executor().spawn(async move {
            task.await.ok()??.into_iter().find_map(|action| {
                let kind = action.lsp_action.action_kind()?;
                kind.as_str().starts_with("quickfix").then(|| QuickFix {
                    title: action.lsp_action.title().to_owned(),
                    action,
                })
            })
        })
    }

    /// Applies a quick fix to the buffer, undoably; saving stays with the user.
    pub fn apply_quick_fix(
        &self,
        buffer: Entity<Buffer>,
        fix: QuickFix,
        cx: &mut App,
    ) -> Task<Result<()>> {
        let task = self.project.update(cx, |p, cx| {
            p.apply_code_action(buffer, fix.action, true, cx)
        });
        cx.background_executor()
            .spawn(async move { task.await.map(|_| ()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, TestAppContext};
    use language::{DiagnosticEntry, DiagnosticSet, LanguageServerId};

    fn diagnostic(
        range: Range<usize>,
        severity: DiagnosticSeverity,
        message: &str,
    ) -> DiagnosticEntry<usize> {
        DiagnosticEntry::new(
            range,
            language::Diagnostic {
                severity,
                message: message.into(),
                source: Some("ts".into()),
                code: Some(lsp::NumberOrString::Number(18048)),
                is_primary: true,
                ..Default::default()
            },
        )
    }

    #[gpui::test]
    fn the_most_severe_problem_under_the_offset_wins(cx: &mut TestAppContext) {
        //                        0123456789012345678901234
        let buffer = cx.new(|cx| Buffer::local("let signature = block.sig;", cx));
        buffer.update(cx, |buffer, cx| {
            let snapshot = buffer.snapshot();
            let entries = vec![
                diagnostic(16..25, DiagnosticSeverity::WARNING, "unused"),
                diagnostic(
                    16..21,
                    DiagnosticSeverity::ERROR,
                    "'block' is possibly 'undefined'.",
                ),
                diagnostic(4..13, DiagnosticSeverity::HINT, "faded"),
            ]
            .into_iter()
            .map(|entry| {
                let range = snapshot.offset_to_point_utf16(entry.range.start)
                    ..snapshot.offset_to_point_utf16(entry.range.end);
                DiagnosticEntry::new(range, entry.diagnostic)
            });
            let set = DiagnosticSet::new(entries, &snapshot);
            buffer.update_diagnostics(LanguageServerId(0), set, cx);
        });
        let snapshot = buffer.read_with(cx, |buffer, _| buffer.snapshot());
        let problem = problem_at(&snapshot, 17).unwrap();
        assert_eq!(problem.message, "'block' is possibly 'undefined'.");
        assert_eq!(problem.code.as_deref(), Some("ts(18048)"));
        assert_eq!(problem.start, TextPoint::new(0, 16));
        assert!(problem.error);
        assert_eq!(problem_at(&snapshot, 22).unwrap().message, "unused");
        assert_eq!(
            problem_at(&snapshot, 25),
            None,
            "the range end is outside it"
        );
        assert_eq!(problem_at(&snapshot, 5), None, "hints stay with the editor");
    }

    #[test]
    fn codes_read_like_the_study() {
        assert_eq!(
            code_label(Some("ts"), Some("1005")).as_deref(),
            Some("ts(1005)")
        );
        assert_eq!(code_label(None, Some("E0308")).as_deref(), Some("E0308"));
        assert_eq!(code_label(Some("eslint"), None).as_deref(), Some("eslint"));
        assert_eq!(code_label(None, None), None);
    }
}
