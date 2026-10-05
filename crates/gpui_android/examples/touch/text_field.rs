//! A one-line text field for trying the on-screen keyboard: typing, autocorrect,
//! composing (underlined), moving the caret, and the send key.
//!
//! Adapted from GPUI's `input` example (Apache-2.0).

use gpui::{
    App, Autocapitalize, Bounds, Context, Element, ElementId, ElementInputHandler,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, Hsla, LayoutId,
    MouseButton, MouseDownEvent, PaintQuad, Pixels, Point, ShapedLine, SharedString, Style,
    TextInputAction, TextInputConfiguration, TextRun, UTF16Selection, UnderlineStyle, Window,
    actions, div, fill, point, prelude::*, px, relative, size,
};
use std::ops::Range;

actions!(
    text_field,
    [Backspace, Delete, Left, Right, Home, End, Enter]
);

pub const CONTEXT: &str = "TextField";

pub enum TextFieldEvent {
    Submit(String),
}

pub struct TextField {
    focus: FocusHandle,
    content: String,
    placeholder: SharedString,
    /// Byte offsets into `content`.
    selection: Range<usize>,
    marked: Option<Range<usize>>,
    layout: Option<ShapedLine>,
    bounds: Option<Bounds<Pixels>>,
    caret_color: Hsla,
}

impl EventEmitter<TextFieldEvent> for TextField {}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TextField {
    pub fn new(
        placeholder: impl Into<SharedString>,
        caret_color: Hsla,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus: cx.focus_handle(),
            content: String::new(),
            placeholder: placeholder.into(),
            selection: 0..0,
            marked: None,
            layout: None,
            bounds: None,
            caret_color,
        }
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.content = text.into();
        self.selection = self.content.len()..self.content.len();
        self.marked = None;
        cx.notify();
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selection = offset..offset;
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content[..offset]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content[offset..]
            .chars()
            .next()
            .map_or(offset, |character| offset + character.len_utf8())
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            self.selection.start = self.previous_boundary(self.selection.end);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            self.selection.end = self.next_boundary(self.selection.start);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let offset = if self.selection.is_empty() {
            self.previous_boundary(self.selection.start)
        } else {
            self.selection.start
        };
        self.move_to(offset, cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let offset = if self.selection.is_empty() {
            self.next_boundary(self.selection.end)
        } else {
            self.selection.end
        };
        self.move_to(offset, cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        self.submit(cx);
    }

    /// Sends the text and clears the field.
    pub fn submit(&mut self, cx: &mut Context<Self>) {
        if self.content.trim().is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.content);
        self.selection = 0..0;
        self.marked = None;
        cx.emit(TextFieldEvent::Submit(text));
        cx.notify();
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        let offset = self.index_for_position(event.position);
        self.move_to(offset, cx);
    }

    fn index_for_position(&self, position: Point<Pixels>) -> usize {
        match (&self.bounds, &self.layout) {
            (Some(bounds), Some(line)) if !self.content.is_empty() => {
                line.closest_index_for_x(position.x - bounds.left())
            }
            _ => self.content.len(),
        }
    }

    fn byte_to_utf16(&self, offset: usize) -> usize {
        self.content[..offset.min(self.content.len())]
            .encode_utf16()
            .count()
    }

    fn utf16_to_byte(&self, offset: usize) -> usize {
        utf8_offset(&self.content, offset)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.utf16_to_byte(range.start)..self.utf16_to_byte(range.end)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.byte_to_utf16(range.start)..self.byte_to_utf16(range.end)
    }

    fn replaced_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        range_utf16
            .map(|range| self.range_from_utf16(&range))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection.clone())
    }
}

/// The byte offset of a UTF-16 offset into `text`, clamped to its end.
fn utf8_offset(text: &str, offset_utf16: usize) -> usize {
    let mut units = 0;
    for (index, character) in text.char_indices() {
        if units >= offset_utf16 {
            return index;
        }
        units += character.len_utf16();
    }
    text.len()
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selection),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.replaced_range(range_utf16);
        self.content.replace_range(range.clone(), text);
        let caret = range.start + text.len();
        self.selection = caret..caret;
        self.marked = None;
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        selection_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.replaced_range(range_utf16);
        self.content.replace_range(range.clone(), text);
        self.marked = (!text.is_empty()).then(|| range.start..range.start + text.len());
        // The selection is relative to the start of the new text.
        self.selection = match selection_utf16 {
            Some(selection) => {
                range.start + utf8_offset(text, selection.start)
                    ..range.start + utf8_offset(text, selection.end)
            }
            None => range.start + text.len()..range.start + text.len(),
        };
        cx.notify();
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selection = self.range_from_utf16(&range_utf16);
        cx.notify();
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.byte_to_utf16(self.content.len()))
    }

    fn text_input_configuration(
        &mut self,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> TextInputConfiguration {
        TextInputConfiguration {
            autocorrect: true,
            autocapitalize: Autocapitalize::Sentences,
            suggestions: true,
            input_action: TextInputAction::Send,
        }
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let line = self.layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(bounds.left() + line.x_for_index(range.start), bounds.top()),
            point(bounds.left() + line.x_for_index(range.end), bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.bounds?;
        let line = self.layout.as_ref()?;
        let index = line.index_for_x(point.x - bounds.left())?;
        Some(self.byte_to_utf16(index))
    }
}

impl Render for TextField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("text-field")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::enter))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .size_full()
            .flex()
            .items_center()
            .child(TextLine { field: cx.entity() })
    }
}

struct TextLine {
    field: gpui::Entity<TextField>,
}

struct Prepaint {
    line: ShapedLine,
    caret: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl IntoElement for TextLine {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for TextLine {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let style = Style {
            size: size(relative(1.).into(), window.line_height().into()),
            ..Style::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let field = self.field.read(cx);
        let style = window.text_style();
        let (text, color) = if field.content.is_empty() {
            (field.placeholder.clone(), style.color.opacity(0.45))
        } else {
            (SharedString::from(field.content.clone()), style.color)
        };
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match &field.marked {
            Some(marked) if !field.content.is_empty() => [
                TextRun {
                    len: marked.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked.len(),
                    underline: Some(UnderlineStyle {
                        color: Some(color),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: text.len() - marked.end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect(),
            _ => vec![run],
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(text, font_size, &runs, None);
        let selection = &field.selection;
        let (caret, selection) = if selection.is_empty() {
            let x = if field.content.is_empty() {
                px(0.)
            } else {
                line.x_for_index(selection.start)
            };
            let caret = fill(
                Bounds::new(
                    point(bounds.left() + x, bounds.top()),
                    size(px(2.), bounds.size.height),
                ),
                field.caret_color,
            );
            (Some(caret), None)
        } else {
            let highlight = fill(
                Bounds::from_corners(
                    point(
                        bounds.left() + line.x_for_index(selection.start),
                        bounds.top(),
                    ),
                    point(
                        bounds.left() + line.x_for_index(selection.end),
                        bounds.bottom(),
                    ),
                ),
                field.caret_color.opacity(0.25),
            );
            (None, Some(highlight))
        };
        Prepaint {
            line,
            caret,
            selection,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.field.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.field.clone()),
            cx,
        );
        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection);
        }
        prepaint
            .line
            .paint(
                bounds.origin,
                window.line_height(),
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            )
            .ok();
        if focus.is_focused(window)
            && let Some(caret) = prepaint.caret.take()
        {
            window.paint_quad(caret);
        }
        let line = prepaint.line.clone();
        self.field.update(cx, |field, _| {
            field.layout = Some(line);
            field.bounds = Some(bounds);
        });
    }
}
