//! A text field for the on-screen keyboard: wrapped lines, the keyboard's
//! composing text (underlined), autocorrect, and a caret that stays in view.
//! One line submits on enter; several lines add a line unless told to send.
//!
//! Selecting works as in Android's own fields: a long press selects a word
//! and dragging on extends it by words; a double tap selects a word; handles
//! under the selection move its ends; and a bar over it offers Cut, Copy,
//! Paste and Select all. Tapping right on the caret offers Paste.
//!
//! The editing model follows the `touch` example's field in gpui_android,
//! which is adapted from GPUI's `input` example (Apache-2.0).

use crate::theme::{SANS, theme};
use gpui::{
    AnyElement, App, AvailableSpace, Bounds, ClipboardItem, ContentMask, Context, Corners,
    DispatchPhase, Element, ElementId, ElementInputHandler, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, FontWeight, GlobalElementId, Hitbox, HitboxBehavior, LayoutId,
    LongPressEvent, MouseButton, MouseDownEvent, MouseMoveEvent, PaintQuad, Pixels, Point,
    ScrollWheelEvent, SharedString, Style, TextInputAction, TextInputConfiguration, TextRun,
    TouchDragEvent, TouchPhase, UTF16Selection, UnderlineStyle, Window, WrappedLine, actions, div,
    fill, point, prelude::*, px, relative, size,
};
use gpui_android::activity;
use std::{
    ops::Range,
    rc::Rc,
    time::{Duration, Instant},
};
use unicode_segmentation::UnicodeSegmentation;

actions!(
    text_area,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        Home,
        End,
        Enter,
        Newline,
        Paste,
        SelectAll,
        CopySelection,
        CutSelection
    ]
);

/// A selection handle: the teardrop under each end of the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle {
    Start,
    End,
}

const HANDLE: f32 = 22.;
const LONG_PRESS_LATCH: Duration = Duration::from_secs(2);

/// What the bar over the selection offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuItem {
    Cut,
    Copy,
    Paste,
    SelectAll,
}

impl MenuItem {
    fn label(self) -> &'static str {
        match self {
            Self::Cut => "Cut",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::SelectAll => "Select all",
        }
    }
}

pub const CONTEXT: &str = "TextArea";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAreaEvent {
    Changed,
    /// Enter in a field that sends.
    Submit,
    /// The selection was copied from read-only text.
    Copied,
}

pub struct TextArea {
    focus: FocusHandle,
    content: String,
    placeholder: SharedString,
    /// Byte offsets into `content`.
    selection: Range<usize>,
    marked: Option<Range<usize>>,
    multiline: bool,
    /// Enter sends instead of adding a line, in a multi-line field.
    pub enter_sends: bool,
    configuration: TextInputConfiguration,
    /// Taller text scrolls inside the field.
    max_lines: usize,
    layout: Option<Layout>,
    bounds: Option<Bounds<Pixels>>,
    scroll: Pixels,
    scroll_x: Pixels,
    /// Last vertical motion, for the shared hold-and-fade scrollbar policy.
    scrollbar_motion: Option<Instant>,
    /// Reveal after editing or moving the caret, not after a user's scroll.
    reveal_caret: bool,
    /// The bar with Cut, Copy and Paste, after a long press or a double tap.
    menu: bool,
    /// A touch that began inside this field, before IME reflow can move it away
    /// from the finger: its window position, original text offset and time.
    pending_long_press: Option<(Point<Pixels>, usize, Instant)>,
    /// The word a long press started on; dragging extends the selection from it.
    anchor: Option<Range<usize>>,
    /// The handle being dragged, and the finger's offset from its point.
    dragging: Option<(Handle, Point<Pixels>)>,
    /// Where a mouse press started, to select by dragging in the desktop preview.
    mouse_anchor: Option<usize>,
    /// Where the bar is in the window, while it shows.
    menu_bounds: Option<Bounds<Pixels>>,
    /// What the field is clipped to, as a sheet's scroll area clips it: the
    /// bar goes where it can be seen whole.
    clip: Option<Bounds<Pixels>>,
    /// Text to read and copy from, not to edit: no keyboard, and the bar
    /// offers Copy and Select all.
    read_only: bool,
}

impl EventEmitter<TextAreaEvent> for TextArea {}

impl Focusable for TextArea {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TextArea {
    /// A field of prose that wraps and grows up to `max_lines`.
    pub fn multiline(
        placeholder: impl Into<SharedString>,
        max_lines: usize,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus: cx.focus_handle(),
            content: String::new(),
            placeholder: placeholder.into(),
            selection: 0..0,
            marked: None,
            multiline: true,
            enter_sends: false,
            configuration: TextInputConfiguration {
                autocorrect: true,
                autocapitalize: gpui::Autocapitalize::Sentences,
                suggestions: true,
                input_action: TextInputAction::Enter,
            },
            max_lines,
            layout: None,
            bounds: None,
            scroll: px(0.),
            scroll_x: px(0.),
            scrollbar_motion: None,
            reveal_caret: true,
            menu: false,
            pending_long_press: None,
            anchor: None,
            dragging: None,
            mouse_anchor: None,
            menu_bounds: None,
            clip: None,
            read_only: false,
        }
    }

    /// Text to select and copy parts of, such as Pi's reply.
    pub fn read_only(max_lines: usize, cx: &mut Context<Self>) -> Self {
        Self {
            read_only: true,
            ..Self::multiline("", max_lines, cx)
        }
    }

    /// Shows `text` with all of it selected and the bar over it.
    pub fn show_selected(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.content = normalize_input(&text.into(), self.multiline);
        self.selection = 0..self.content.len();
        self.marked = None;
        self.scroll = px(0.);
        self.menu = !self.content.is_empty();
        self.reveal_caret = false;
        cx.notify();
    }

    /// One line that submits on enter, configured for the keyboard as given.
    pub fn single_line(
        placeholder: impl Into<SharedString>,
        configuration: TextInputConfiguration,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            multiline: false,
            configuration,
            max_lines: 1,
            ..Self::multiline(placeholder, 1, cx)
        }
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    pub fn is_empty(&self) -> bool {
        self.content.trim().is_empty()
    }

    #[cfg(feature = "ui-test")]
    pub(crate) fn menu_open(&self) -> bool {
        self.menu
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>) {
        self.placeholder = placeholder.into();
    }

    pub fn set_max_lines(&mut self, lines: usize) {
        self.max_lines = lines.max(1);
    }

    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.content = normalize_input(&text.into(), self.multiline);
        self.selection = self.content.len()..self.content.len();
        self.marked = None;
        self.menu = false;
        self.reveal_caret = true;
        cx.emit(TextAreaEvent::Changed);
        cx.notify();
    }

    /// Takes the text, leaving the field empty.
    pub fn take(&mut self, cx: &mut Context<Self>) -> String {
        let text = std::mem::take(&mut self.content);
        self.selection = 0..0;
        self.marked = None;
        self.scroll = px(0.);
        self.scroll_x = px(0.);
        self.menu = false;
        self.reveal_caret = true;
        cx.emit(TextAreaEvent::Changed);
        cx.notify();
        text
    }

    /// The word the caret ends, after `sigil` at the start of a word: what
    /// follows "@" or "/" while it is typed, with its range including the sigil.
    pub fn token_before_caret(&self, sigil: char) -> Option<(Range<usize>, &str)> {
        if !self.selection.is_empty() {
            return None;
        }
        let caret = self.selection.end;
        let before = &self.content[..caret];
        let start = before
            .rfind(|c: char| c.is_whitespace())
            .map_or(0, |index| {
                index + before[index..].chars().next().map_or(1, char::len_utf8)
            });
        let word = &before[start..];
        let rest = word.strip_prefix(sigil)?;
        Some((start..caret, rest))
    }

    /// Replaces `range` with `text` and puts the caret after it.
    pub fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        let range = range.start.min(self.content.len())..range.end.min(self.content.len());
        let text = normalize_input(text, self.multiline);
        self.content.replace_range(range.clone(), &text);
        let caret = range.start + text.len();
        self.selection = caret..caret;
        self.marked = None;
        self.menu = false;
        self.reveal_caret = true;
        cx.emit(TextAreaEvent::Changed);
        cx.notify();
    }

    /// Inserts at the caret, replacing any selection.
    pub fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        self.replace(self.selection.clone(), text, cx);
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.select(offset..offset, cx);
    }

    /// Moves the selection. The keyboard's composing word stays marked: the
    /// keyboard finishes it once it sees the selection move, as with Android's
    /// own fields. Dropping it here instead leaves the keyboard composing a
    /// word the field no longer has, which it then types again at the caret.
    fn select(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        self.selection = range;
        self.reveal_caret = true;
        cx.notify();
    }

    /// The field's own edit: replaces the selection, never the keyboard's
    /// composing word, which it ends.
    fn edit_selection(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        self.marked = None;
        self.replace_text_in_range(None, text, window, cx);
    }

    /// The offset under a point in the window.
    fn offset_at(&self, position: Point<Pixels>) -> usize {
        match (&self.layout, &self.bounds) {
            (Some(layout), Some(bounds)) if !self.content.is_empty() => {
                layout.offset_for_position(point(
                    position.x - bounds.left() + self.scroll_x,
                    position.y - bounds.top() + self.scroll,
                ))
            }
            _ => self.content.len(),
        }
    }

    /// The word around `offset`, or the character there between words.
    fn word_at(&self, offset: usize) -> Range<usize> {
        let in_word = |c: char| c.is_alphanumeric() || c == '_';
        let (before, after) = self.content.split_at(offset.min(self.content.len()));
        let start = before
            .char_indices()
            .rev()
            .take_while(|(_, c)| in_word(*c))
            .last()
            .map_or(offset, |(index, _)| index);
        let end = after
            .char_indices()
            .take_while(|(_, c)| in_word(*c))
            .last()
            .map_or(offset, |(index, c)| offset + index + c.len_utf8());
        if start < end {
            return start..end;
        }
        match self.next_boundary(offset) {
            next if next > offset => offset..next,
            _ => self.previous_boundary(offset)..offset,
        }
    }

    fn prepare_long_press(&mut self, position: Point<Pixels>) {
        self.pending_long_press = Some((position, self.offset_at(position), Instant::now()));
    }

    fn take_long_press_offset(&mut self, position: Point<Pixels>) -> Option<usize> {
        let (started, offset, at) = self.pending_long_press.take()?;
        (started == position && at.elapsed() <= LONG_PRESS_LATCH).then_some(offset)
    }

    fn long_press(
        &mut self,
        event: &LongPressEvent,
        original_offset: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.phase {
            TouchPhase::Started => {
                window.focus(&self.focus, cx);
                self.menu = false;
                let word = if self.content.is_empty() {
                    0..0
                } else {
                    self.word_at(
                        original_offset.unwrap_or_else(|| self.offset_at(event.start_position)),
                    )
                };
                self.anchor = Some(word.clone());
                self.select(word, cx);
                activity::long_press_feedback();
            }
            TouchPhase::Moved => {
                let Some(anchor) = self.anchor.clone().filter(|_| !self.content.is_empty()) else {
                    return;
                };
                let word = self.word_at(self.offset_at(event.position));
                let range = anchor.start.min(word.start)..anchor.end.max(word.end);
                if range != self.selection {
                    self.select(range, cx);
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.anchor = None;
                self.menu = true;
                cx.notify();
            }
        }
    }

    fn drag_handle(&mut self, handle: Handle, offset: Point<Pixels>, cx: &mut Context<Self>) {
        self.dragging = Some((handle, offset));
        self.menu = false;
        cx.notify();
    }

    fn move_handle(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((handle, offset)) = self.dragging else {
            return;
        };
        let line_height = self
            .layout
            .as_ref()
            .map_or(px(0.), |layout| layout.line_height);
        // A handle hangs under its line: aim at the middle of that line.
        let at = self.offset_at(position - offset - point(px(0.), line_height / 2.));
        let Range { start, end } = self.selection;
        let range = match handle {
            Handle::Start => at.min(self.previous_boundary(end))..end,
            Handle::End => start..at.max(self.next_boundary(start)),
        };
        if range != self.selection {
            self.select(range, cx);
        }
    }

    fn drop_handle(&mut self, cx: &mut Context<Self>) {
        if self.dragging.take().is_some() {
            self.menu = true;
            cx.notify();
        }
    }

    fn menu_items(&self) -> Vec<MenuItem> {
        let mut items = Vec::new();
        if !self.selection.is_empty() {
            if !self.read_only {
                items.push(MenuItem::Cut);
            }
            items.push(MenuItem::Copy);
        }
        if !self.read_only {
            items.push(MenuItem::Paste);
        }
        if !self.content.is_empty() && self.selection != (0..self.content.len()) {
            items.push(MenuItem::SelectAll);
        }
        items
    }

    fn choose(&mut self, item: MenuItem, window: &mut Window, cx: &mut Context<Self>) {
        match item {
            MenuItem::Cut => self.cut(&CutSelection, window, cx),
            MenuItem::Copy => {
                self.copy(&CopySelection, window, cx);
                self.move_to(self.selection.end, cx);
                if self.read_only {
                    cx.emit(TextAreaEvent::Copied);
                }
            }
            MenuItem::Paste => self.paste_text(&Paste, window, cx),
            MenuItem::SelectAll => {
                self.select(0..self.content.len(), cx);
                return;
            }
        }
        self.menu = false;
    }

    fn copy(&mut self, _: &CopySelection, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            let text = self.content[self.selection.clone()].to_owned();
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn cut(&mut self, _: &CutSelection, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            self.copy(&CopySelection, window, cx);
            self.edit_selection("", window, cx);
        }
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content[..offset]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content[offset..]
            .graphemes(true)
            .next()
            .map_or(offset, |grapheme| offset + grapheme.len())
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            self.selection.start = self.previous_boundary(self.selection.end);
        }
        self.edit_selection("", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            self.selection.end = self.next_boundary(self.selection.start);
        }
        self.edit_selection("", window, cx);
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

    fn vertical(&mut self, rows: f32, cx: &mut Context<Self>) {
        let Some(layout) = &self.layout else {
            return;
        };
        let position = layout.position_for_offset(self.selection.end);
        let target = point(position.x, position.y + layout.line_height * rows);
        let offset = if target.y < px(0.) {
            0
        } else if target.y >= layout.height() {
            self.content.len()
        } else {
            layout.offset_for_position(target)
        };
        self.move_to(offset, cx);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1., cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(1., cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let start = self.content[..self.selection.start]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        self.move_to(start, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let end = self.content[self.selection.end..]
            .find('\n')
            .map_or(self.content.len(), |index| self.selection.end + index);
        self.move_to(end, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select(0..self.content.len(), cx);
    }

    fn enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if self.multiline && !self.enter_sends {
            self.edit_selection("\n", window, cx);
        } else {
            cx.emit(TextAreaEvent::Submit);
        }
    }

    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.multiline {
            self.edit_selection("\n", window, cx);
        }
    }

    fn scroll_text(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (Some(layout), Some(bounds)) = (&self.layout, self.bounds) else {
            return;
        };
        let delta = event.delta.pixel_delta(layout.line_height);
        let (offset, maximum, delta) = if self.multiline {
            (
                &mut self.scroll,
                layout.height() - bounds.size.height,
                delta.y,
            )
        } else {
            (
                &mut self.scroll_x,
                layout.width() - bounds.size.width + px(2.),
                if delta.x == px(0.) { delta.y } else { delta.x },
            )
        };
        let next = (*offset - delta).max(px(0.)).min(maximum.max(px(0.)));
        if next != *offset {
            *offset = next;
            if self.multiline {
                self.scrollbar_motion = Some(Instant::now());
            }
            self.reveal_caret = false;
            self.menu = false;
            self.mouse_anchor = None;
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn paste_text(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = if self.multiline {
                text
            } else {
                text.replace('\n', " ")
            };
            self.edit_selection(&text, window, cx);
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_long_press = None;
        let focused = self.focus.is_focused(window);
        window.focus(&self.focus, cx);
        if !self.read_only {
            window.request_virtual_keyboard();
        }
        let offset = self.offset_at(event.position);
        if event.click_count >= 2 && !self.content.is_empty() {
            self.select(self.word_at(offset), cx);
            self.menu = true;
            return;
        }
        // Tapping right on the caret offers Paste, as Android's fields do;
        // a tap anywhere else only moves the caret.
        if focused && self.selection.is_empty() && self.on_caret(event.position) {
            self.menu = !self.menu;
            cx.notify();
            return;
        }
        self.menu = false;
        self.mouse_anchor = Some(offset);
        self.move_to(offset, cx);
    }

    /// Whether a point in the window is on the caret, give or take a fingertip.
    fn on_caret(&self, position: Point<Pixels>) -> bool {
        let (Some(layout), Some(bounds)) = (&self.layout, &self.bounds) else {
            return false;
        };
        let caret = if self.content.is_empty() {
            point(px(0.), px(0.))
        } else {
            layout.position_for_offset(self.selection.start)
        };
        let x = bounds.left() + caret.x - self.scroll_x;
        let top = bounds.top() + caret.y - self.scroll;
        (position.x - x).abs() <= px(16.)
            && top - px(8.) <= position.y
            && position.y <= top + layout.line_height + px(8.)
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(anchor) = self
            .mouse_anchor
            .filter(|_| event.pressed_button == Some(MouseButton::Left))
        else {
            return;
        };
        let offset = self.offset_at(event.position);
        let range = anchor.min(offset)..anchor.max(offset);
        if range != self.selection {
            self.select(range, cx);
        }
    }

    /// The bar for the selection: above the whole field, so it never covers
    /// text a finger may tap, or under it near the top of the screen. Its
    /// height follows the field's layout, so it moves with the field and does
    /// not jump. Across, it is centered on the selection and kept on screen.
    fn menu_bar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<(AnyElement, Bounds<Pixels>)> {
        let layout = self.layout.as_ref()?;
        let bounds = self.bounds?;
        let colors = theme(cx);
        let items = self.menu_items();
        let font_size = px(14.);
        let mut font = gpui::font(SANS);
        font.weight = FontWeight::SEMIBOLD;
        let padding = px(14.);
        let widths: Vec<Pixels> = items
            .iter()
            .map(|item| {
                let label = item.label();
                let run = TextRun {
                    len: label.len(),
                    font: font.clone(),
                    color: colors.text,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                window
                    .text_system()
                    .shape_line(label.into(), font_size, &[run], None)
                    .width
                    + padding * 2.
            })
            .collect();
        let width = widths.iter().fold(px(8.), |total, width| total + *width);
        let height = px(44.);
        let (start, end) = if self.content.is_empty() {
            (point(px(0.), px(0.)), point(px(0.), px(0.)))
        } else {
            (
                layout.position_for_offset(self.selection.start),
                layout.position_for_offset(self.selection.end),
            )
        };
        let center = if start.y == end.y {
            (start.x + end.x) / 2. - self.scroll_x
        } else {
            bounds.size.width / 2.
        };
        // Past the box around the field too, as in the composer.
        let gap = px(24.);
        // Drawn over everything, past the field's clip: only the window bounds it.
        let visible = window.fully_visible_bounds();
        let (clip_top, clip_bottom) = (visible.top(), visible.bottom());
        let above = bounds.top() - gap - height >= clip_top + px(8.);
        let below = gap + px(HANDLE);
        // Neither fits, as in a tall field in a sheet: over the field's top.
        let inside = (!above && bounds.bottom() + below + height > clip_bottom - px(8.))
            .then(|| (clip_top.max(bounds.top()) + px(8.)) - bounds.top());
        let viewport = window.viewport_size().width;
        let left = (bounds.left() + center - width / 2.)
            .min(viewport - width - px(8.))
            .max(px(8.))
            - bounds.left();
        let top = if above {
            bounds.top() - gap - height
        } else if let Some(inside) = inside {
            bounds.top() + inside
        } else {
            bounds.bottom() + below
        };
        let place = Bounds::new(point(bounds.left() + left, top), size(width, height));
        let bar = div()
            .id("text-menu")
            .child(crate::testing::probe("text-menu"))
            .h(height)
            .px(px(4.))
            .flex()
            .items_center()
            .rounded(px(12.))
            .bg(colors.composer)
            .border_1()
            .border_color(colors.line)
            .shadow(vec![gpui::BoxShadow {
                color: colors.shadow,
                offset: point(px(0.), px(4.)),
                blur_radius: px(16.),
                spread_radius: px(0.),
                inset: false,
            }])
            .occlude()
            .text_size(font_size)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(colors.text)
            .whitespace_nowrap()
            .children(items.into_iter().map(|item| {
                div()
                    .id(item.label())
                    .h(px(36.))
                    .px(padding)
                    .flex()
                    .items_center()
                    .rounded(px(8.))
                    .active(|style| style.bg(colors.selected))
                    .child(item.label())
                    .on_click(cx.listener(move |this, _, window, cx| this.choose(item, window, cx)))
            }));
        // Placed against the field's edge by layout, not by last frame's bounds.
        let bar = div()
            .absolute()
            .left(left)
            .map(|edge| {
                if above {
                    edge.bottom(relative(1.)).pb(gap)
                } else if let Some(inside) = inside {
                    edge.top(inside)
                } else {
                    edge.top(relative(1.)).pt(below)
                }
            })
            .child(bar)
            .into_any_element();
        Some((bar, place))
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
        self.utf16_to_byte(range.start.min(range.end))
            ..self.utf16_to_byte(range.start.max(range.end))
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

    /// The text to draw and its runs: the content, or the placeholder.
    fn display(
        &self,
        color: gpui::Hsla,
        placeholder: gpui::Hsla,
        font: gpui::Font,
    ) -> (SharedString, Vec<TextRun>) {
        let run = TextRun {
            len: 0,
            font,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        if self.content.is_empty() {
            let text = self.placeholder.clone();
            let runs = vec![TextRun {
                len: text.len(),
                color: placeholder,
                ..run
            }];
            return (text, runs);
        }
        let text = SharedString::from(self.content.clone());
        let runs = match &self.marked {
            Some(marked) => [
                (0..marked.start, None),
                (
                    marked.clone(),
                    Some(UnderlineStyle {
                        color: Some(color),
                        thickness: px(1.),
                        wavy: false,
                    }),
                ),
                (marked.end..text.len(), None),
            ]
            .into_iter()
            .filter(|(range, _)| !range.is_empty())
            .map(|(range, underline)| TextRun {
                len: range.len(),
                underline,
                ..run.clone()
            })
            .collect(),
            None => vec![TextRun {
                len: text.len(),
                ..run
            }],
        };
        (text, runs)
    }
}

fn normalize_input(text: &str, multiline: bool) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    if multiline {
        text
    } else {
        text.replace(['\n', '\u{2028}', '\u{2029}'], " ")
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

impl EntityInputHandler for TextArea {
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

    fn paste(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = item.text() {
            self.edit_selection(&text, window, cx);
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        self.menu = false;
        let range = self.replaced_range(range_utf16);
        let text = normalize_input(text, self.multiline);
        self.content.replace_range(range.clone(), &text);
        let caret = range.start + text.len();
        self.selection = caret..caret;
        self.marked = None;
        self.reveal_caret = true;
        cx.emit(TextAreaEvent::Changed);
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
        if self.read_only {
            return;
        }
        self.menu = false;
        let range = self.replaced_range(range_utf16);
        let text = normalize_input(text, self.multiline);
        self.content.replace_range(range.clone(), &text);
        self.marked = (!text.is_empty()).then(|| range.start..range.start + text.len());
        // The selection is relative to the start of the new text.
        self.selection = match selection_utf16 {
            Some(selection) => {
                range.start + utf8_offset(&text, selection.start.min(selection.end))
                    ..range.start + utf8_offset(&text, selection.start.max(selection.end))
            }
            None => range.start + text.len()..range.start + text.len(),
        };
        self.reveal_caret = true;
        cx.emit(TextAreaEvent::Changed);
        cx.notify();
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select(self.range_from_utf16(&range_utf16), cx);
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.byte_to_utf16(self.content.len()))
    }

    fn text_input_configuration(
        &mut self,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> TextInputConfiguration {
        let mut configuration = self.configuration.clone();
        if self.multiline && self.enter_sends {
            configuration.input_action = TextInputAction::Send;
        }
        configuration
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let origin = point(bounds.left() - self.scroll_x, bounds.top() - self.scroll);
        let start = layout.position_for_offset(range.start);
        let end = layout.position_for_offset(range.end);
        let (left, right) = if start.y == end.y {
            (start.x, end.x.max(start.x + px(1.)))
        } else {
            (px(0.), bounds.size.width)
        };
        Some(Bounds::from_corners(
            origin + point(left, start.y),
            origin + point(right, end.y + layout.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.bounds?;
        let layout = self.layout.as_ref()?;
        let local = gpui::point(
            point.x - bounds.left() + self.scroll_x,
            point.y - bounds.top() + self.scroll,
        );
        Some(self.byte_to_utf16(layout.offset_for_position(local)))
    }
}

impl Render for TextArea {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focus.is_focused(window) {
            self.menu = false;
            self.dragging = None;
        }
        let menu = if self.menu && self.dragging.is_none() && self.anchor.is_none() {
            self.menu_bar(window, cx)
        } else {
            None
        };
        self.menu_bounds = menu.as_ref().map(|(_, place)| *place);
        let menu = menu.map(|(bar, _)| bar);
        div()
            .id("text-area")
            .relative()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::paste_text))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::scroll_text))
            // A press anywhere but the bar closes it.
            .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                let on_bar = this
                    .menu_bounds
                    .is_some_and(|place| place.contains(&event.position));
                if this.menu && !on_bar {
                    this.menu = false;
                    cx.notify();
                }
            }))
            .w_full()
            .child(TextBody { area: cx.entity() })
            // Over the buttons around the field, which then don't take its taps.
            .children(menu.map(|bar| gpui::deferred(bar).with_priority(1)))
    }
}

/// Shaped text by hard line, with each line's byte offset.
#[derive(Clone)]
struct Layout {
    lines: Rc<[(usize, WrappedLine)]>,
    line_height: Pixels,
}

impl Layout {
    fn shape(
        text: SharedString,
        runs: &[TextRun],
        font_size: Pixels,
        line_height: Pixels,
        width: Option<Pixels>,
        window: &Window,
    ) -> Self {
        let shaped = window
            .text_system()
            .shape_text(text, font_size, runs, width, None)
            .unwrap_or_default();
        let mut start = 0;
        let lines = shaped
            .into_iter()
            .map(|line| {
                let line_start = start;
                start += line.len() + 1;
                (line_start, line)
            })
            .collect();
        Self { lines, line_height }
    }

    fn row_height(&self, line: &WrappedLine) -> Pixels {
        line.size(self.line_height).height
    }

    fn height(&self) -> Pixels {
        self.lines
            .iter()
            .map(|(_, line)| self.row_height(line))
            .fold(px(0.), |total, height| total + height)
            .max(self.line_height)
    }

    fn width(&self) -> Pixels {
        self.lines
            .iter()
            .map(|(_, line)| line.size(self.line_height).width)
            .fold(px(0.), Pixels::max)
    }

    fn position_for_offset(&self, offset: usize) -> Point<Pixels> {
        let mut top = px(0.);
        for (start, line) in self.lines.iter() {
            if offset <= start + line.len() {
                let position = line
                    .position_for_index(offset - start, self.line_height)
                    .unwrap_or_default();
                return point(position.x, top + position.y);
            }
            top += self.row_height(line);
        }
        point(px(0.), top)
    }

    /// Geometry for a half-open text span, with downstream affinity at wraps.
    /// Caret positions intentionally use upstream affinity; using them for a
    /// token would draw an empty pill at the end of the preceding visual row.
    fn span_bounds(&self, range: Range<usize>) -> Vec<Bounds<Pixels>> {
        let mut bounds = Vec::new();
        let mut top = px(0.);
        for (offset, line) in self.lines.iter() {
            let mut start = 0;
            for (row, end) in line
                .wrap_boundaries
                .iter()
                .map(|boundary| {
                    line.unwrapped_layout.runs[boundary.run_ix].glyphs[boundary.glyph_ix].index
                })
                .chain([line.len()])
                .enumerate()
            {
                let from = range.start.saturating_sub(*offset).max(start);
                let to = range.end.saturating_sub(*offset).min(end);
                if from < to {
                    let base = line.unwrapped_layout.x_for_index(start);
                    let left = line.unwrapped_layout.x_for_index(from) - base;
                    let right = line.unwrapped_layout.x_for_index(to) - base;
                    bounds.push(Bounds::new(
                        point(left, top + self.line_height * row),
                        size(right - left, self.line_height),
                    ));
                }
                start = end;
            }
            top += self.row_height(line);
        }
        bounds
    }

    fn offset_for_position(&self, position: Point<Pixels>) -> usize {
        let mut top = px(0.);
        for (index, (start, line)) in self.lines.iter().enumerate() {
            let height = self.row_height(line);
            if position.y < top + height || index + 1 == self.lines.len() {
                let y = (position.y - top)
                    .max(px(0.))
                    .min(height - self.line_height / 2.);
                let offset = line
                    .closest_index_for_position(point(position.x.max(px(0.)), y), self.line_height)
                    .unwrap_or_else(|offset| offset);
                return start + offset.min(line.len());
            }
            top += height;
        }
        0
    }
}

struct TextBody {
    area: gpui::Entity<TextArea>,
}

struct Prepaint {
    layout: Layout,
    scroll: Pixels,
    scroll_x: Pixels,
    scrollbar_opacity: f32,
    scrollbar_moved: bool,
    caret: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
    mentions: Vec<PaintQuad>,
    /// The text, for long presses.
    hitbox: Hitbox,
    /// The selection's handles: where each points, and where a finger takes it.
    handles: Rc<[(Handle, Point<Pixels>, Hitbox)]>,
}

impl IntoElement for TextBody {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for TextBody {
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
        let area = self.area.clone();
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let colors = theme(cx);
        let mut layout_style = Style::default();
        layout_style.size.width = relative(1.).into();
        let id =
            window.request_measured_layout(layout_style, move |known, available, window, cx| {
                let width = known.width.or(match available.width {
                    AvailableSpace::Definite(width) => Some(width),
                    _ => None,
                });
                let area = area.read(cx);
                let (text, runs) = area.display(style.color, colors.muted, style.font());
                let wrap = if area.multiline { width } else { None };
                let layout = Layout::shape(text, &runs, font_size, line_height, wrap, window);
                let height = layout.height().min(line_height * area.max_lines as f32);
                size(width.unwrap_or_default(), height)
            });
        (id, ())
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
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let colors = theme(cx);
        let area = self.area.read(cx);
        let (text, runs) = area.display(style.color, colors.muted, style.font());
        let layout = Layout::shape(
            text,
            &runs,
            font_size,
            line_height,
            area.multiline.then_some(bounds.size.width),
            window,
        );
        let empty = area.content.is_empty();
        let selection = area.selection.clone();
        let start = if empty {
            point(px(0.), px(0.))
        } else {
            layout.position_for_offset(selection.start)
        };
        let end = if empty {
            start
        } else {
            layout.position_for_offset(selection.end)
        };
        // Keep the caret in view when the text is taller than the field.
        let mut scroll = area.scroll;
        let reveal = area.reveal_caret || area.bounds.is_none_or(|old| old.size != bounds.size);
        if reveal {
            if end.y < scroll {
                scroll = end.y;
            } else if end.y + line_height > scroll + bounds.size.height {
                scroll = end.y + line_height - bounds.size.height;
            }
        }
        scroll = scroll
            .max(px(0.))
            .min((layout.height() - bounds.size.height).max(px(0.)));
        let mut scroll_x = if area.multiline {
            px(0.)
        } else {
            area.scroll_x
        };
        if reveal && !area.multiline {
            if end.x < scroll_x {
                scroll_x = end.x;
            } else if end.x + px(2.) > scroll_x + bounds.size.width {
                scroll_x = end.x + px(2.) - bounds.size.width;
            }
        }
        scroll_x = scroll_x
            .max(px(0.))
            .min((layout.width() + px(2.) - bounds.size.width).max(px(0.)));
        let now = Instant::now();
        let scrollbar_moved = scroll != area.scroll;
        let scrollbar_opacity = crate::scroll::thumb_opacity(
            if scrollbar_moved {
                Some(now)
            } else {
                area.scrollbar_motion
            },
            false,
            now,
        );
        let origin = point(bounds.left() - scroll_x, bounds.top() - scroll);
        let caret = selection.is_empty().then(|| {
            fill(
                Bounds::new(origin + end, size(px(2.), line_height)),
                colors.accent,
            )
        });
        let highlight = colors.accent.opacity(0.25);
        let mut mentions = Vec::new();
        if area.multiline {
            for range in mention_ranges(&area.content) {
                for span in layout.span_bounds(range) {
                    let rect = Bounds::new(origin + span.origin, span.size);
                    if rect.size.width > px(0.)
                        && rect.bottom() > bounds.top()
                        && rect.top() < bounds.bottom()
                    {
                        mentions.push(
                            fill(rect, colors.selected)
                                .corner_radii(px(4.))
                                .border_widths(px(1.))
                                .border_color(colors.accent.opacity(0.5)),
                        );
                    }
                }
            }
        }
        let mut quads = Vec::new();
        if !selection.is_empty() {
            if start.y == end.y {
                quads.push(fill(
                    Bounds::from_corners(
                        origin + start,
                        origin + point(end.x, end.y + line_height),
                    ),
                    highlight,
                ));
            } else {
                let right = bounds.size.width;
                quads.push(fill(
                    Bounds::from_corners(
                        origin + start,
                        origin + point(right, start.y + line_height),
                    ),
                    highlight,
                ));
                if end.y > start.y + line_height {
                    quads.push(fill(
                        Bounds::from_corners(
                            origin + point(px(0.), start.y + line_height),
                            origin + point(right, end.y),
                        ),
                        highlight,
                    ));
                }
                quads.push(fill(
                    Bounds::from_corners(
                        origin + point(px(0.), end.y),
                        origin + point(end.x, end.y + line_height),
                    ),
                    highlight,
                ));
            }
        }
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let focused = area.focus.is_focused(window);
        let handles = [(Handle::Start, start), (Handle::End, end)]
            .into_iter()
            .filter(|_| focused && !selection.is_empty() && !empty)
            .map(|(handle, at)| (handle, origin + point(at.x, at.y + line_height)))
            // Only for lines in view.
            .filter(|(_, at)| bounds.top() < at.y && at.y - line_height < bounds.bottom() + px(1.))
            .map(|(handle, at)| {
                // A finger-sized target around the teardrop.
                let left = match handle {
                    Handle::Start => at.x - px(HANDLE * 1.5),
                    Handle::End => at.x - px(HANDLE / 2.),
                };
                let target =
                    Bounds::new(point(left, at.y - px(4.)), size(px(HANDLE * 2.), px(40.)));
                (
                    handle,
                    at,
                    window.insert_hitbox(target, HitboxBehavior::Normal),
                )
            })
            .collect();
        Prepaint {
            layout,
            scroll,
            scroll_x,
            scrollbar_opacity,
            scrollbar_moved,
            caret,
            selection: quads,
            mentions,
            hitbox,
            handles,
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
        let clip = window.content_mask().bounds;
        let focus = self.area.read(cx).focus.clone();
        if !self.area.read(cx).read_only {
            window.handle_input(
                &focus,
                ElementInputHandler::new(bounds, self.area.clone()),
                cx,
            );
        }
        let layout = &prepaint.layout;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for quad in prepaint.mentions.drain(..) {
                window.paint_quad(quad);
            }
            for quad in prepaint.selection.drain(..) {
                window.paint_quad(quad);
            }
            let mut top = bounds.top() - prepaint.scroll;
            for (_, line) in layout.lines.iter() {
                if top > bounds.bottom() {
                    break;
                }
                if top + layout.row_height(line) < bounds.top() {
                    top += layout.row_height(line);
                    continue;
                }
                line.paint(
                    point(bounds.left() - prepaint.scroll_x, top),
                    layout.line_height,
                    gpui::TextAlign::Left,
                    None,
                    window,
                    cx,
                )
                .ok();
                top += layout.row_height(line);
            }
            if focus.is_focused(window)
                && let Some(caret) = prepaint.caret.take()
            {
                window.paint_quad(caret);
            }
            if prepaint.scrollbar_opacity > 0.
                && let Some(geometry) = crate::scroll::Geometry::new(
                    bounds,
                    layout.height() - bounds.size.height,
                    -prepaint.scroll,
                )
            {
                window.paint_quad(
                    fill(
                        geometry.thumb,
                        theme(cx).muted.opacity(0.65 * prepaint.scrollbar_opacity),
                    )
                    .corner_radii(px(2.)),
                );
                window.request_animation_frame();
            }
        });
        let accent = theme(cx).accent;
        for (handle, at, _) in prepaint.handles.iter() {
            let (left, corners) = match handle {
                Handle::Start => (at.x - px(HANDLE), (px(HANDLE / 2.), px(0.))),
                Handle::End => (at.x, (px(0.), px(HANDLE / 2.))),
            };
            window.paint_quad(
                fill(
                    Bounds::new(point(left, at.y), size(px(HANDLE), px(HANDLE))),
                    accent,
                )
                .corner_radii(Corners {
                    top_left: corners.0,
                    top_right: corners.1,
                    bottom_right: px(HANDLE / 2.),
                    bottom_left: px(HANDLE / 2.),
                }),
            );
        }
        self.listen(prepaint, window);
        let (layout, scroll) = (prepaint.layout.clone(), prepaint.scroll);
        self.area.update(cx, |area, _| {
            area.layout = Some(layout);
            area.bounds = Some(bounds);
            area.clip = Some(clip);
            area.scroll = scroll;
            area.scroll_x = prepaint.scroll_x;
            if prepaint.scrollbar_moved {
                area.scrollbar_motion = Some(Instant::now());
            }
            area.reveal_caret = false;
        });
    }
}

impl TextBody {
    /// Long presses select words; dragging a handle moves an end of the selection.
    fn listen(&self, prepaint: &Prepaint, window: &mut Window) {
        // The keyboard can move the focused composer between touch-down and the
        // long-press timer. Remember ownership and the original text offset at
        // touch-down, before that reflow changes hit testing and geometry.
        let area = self.area.clone();
        let text = prepaint.hitbox.clone();
        window.on_mouse_event(move |event: &TouchDragEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && event.phase == TouchPhase::Started {
                area.update(cx, |area, _| {
                    if text.is_hovered(window) {
                        area.prepare_long_press(event.start_position);
                    } else {
                        area.pending_long_press = None;
                    }
                });
            }
        });
        let area = self.area.clone();
        let text = prepaint.hitbox.clone();
        window.on_mouse_event(move |event: &LongPressEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let original_offset = if event.phase == TouchPhase::Started {
                let original_offset = area.update(cx, |area, _| {
                    area.take_long_press_offset(event.start_position)
                });
                if !text.is_hovered(window) && original_offset.is_none() {
                    return;
                }
                window.prevent_default();
                window.capture_long_press(&area);
                original_offset
            } else {
                if !window.has_long_press_capture(&area) {
                    return;
                }
                None
            };
            area.update(cx, |area, cx| {
                area.long_press(event, original_offset, window, cx)
            });
        });
        let area = self.area.clone();
        let handles = prepaint.handles.clone();
        window.on_mouse_event(move |event: &TouchDragEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            match event.phase {
                TouchPhase::Started => {
                    let Some((handle, at, _)) = handles
                        .iter()
                        .find(|(_, _, target)| target.is_hovered(window))
                    else {
                        return;
                    };
                    window.prevent_default();
                    let offset = event.start_position - *at;
                    area.update(cx, |area, cx| area.drag_handle(*handle, offset, cx));
                }
                TouchPhase::Moved => {
                    area.update(cx, |area, cx| area.move_handle(event.position, cx))
                }
                TouchPhase::Ended | TouchPhase::Cancelled => {
                    area.update(cx, |area, cx| area.drop_handle(cx))
                }
            }
        });
    }
}

/// Mention decoration is derived from the actual text, including paste and IME
/// replacements. It cannot get out of sync with a separate chip/hidden payload.
fn mention_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut chars = text.char_indices().peekable();
    let mut previous = None;
    while let Some((start, ch)) = chars.next() {
        if ch == '@' && previous.is_none_or(|ch: char| ch.is_whitespace() || "([{ ".contains(ch)) {
            let mut end = start + 1;
            if chars.peek().is_some_and(|(_, ch)| *ch == '"') {
                chars.next();
                for (index, ch) in chars.by_ref() {
                    if ch == '\n' {
                        break;
                    }
                    end = index + ch.len_utf8();
                    if ch == '"' {
                        break;
                    }
                }
            } else {
                while let Some(&(index, ch)) = chars.peek() {
                    if ch.is_whitespace() || ",;:)]}\"".contains(ch) {
                        break;
                    }
                    chars.next();
                    end = index + ch.len_utf8();
                }
            }
            if end > start + 1 {
                ranges.push(start..end);
            }
        }
        previous = Some(ch);
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[test]
    fn mentions_have_exact_unicode_safe_ranges_without_marking_email() {
        let text = "Read @src/main.rs and @\"my folder/中文.rs\"; email user@example.com, then (@README.md).";
        let mentions = mention_ranges(text)
            .into_iter()
            .map(|range| &text[range])
            .collect::<Vec<_>>();
        assert_eq!(
            mentions,
            ["@src/main.rs", "@\"my folder/中文.rs\"", "@README.md"]
        );
        assert!(mention_ranges("email@test.com @ ").is_empty());
    }

    #[gpui::test]
    fn the_token_being_typed_is_found(cx: &mut TestAppContext) {
        crate::theme::install_for_tests(cx);
        let area = cx.new(|cx| TextArea::multiline("", 4, cx));
        area.update(cx, |area, cx| {
            area.set_text("Same for DeepSeek, start with @deep", cx);
            let (range, word) = area.token_before_caret('@').unwrap();
            assert_eq!(word, "deep");
            assert_eq!(&area.text()[range.clone()], "@deep");
            area.replace(range, "@deepseek.ts ", cx);
            assert_eq!(area.text(), "Same for DeepSeek, start with @deepseek.ts ");
            assert!(area.token_before_caret('@').is_none());
        });
    }

    #[gpui::test]
    fn a_long_press_keeps_the_touch_down_offset_across_reflow(cx: &mut TestAppContext) {
        crate::theme::install_for_tests(cx);
        let area = cx.new(|cx| TextArea::multiline("", 4, cx));
        area.update(cx, |area, _| {
            let started = point(px(40.), px(700.));
            area.pending_long_press = Some((started, 12, Instant::now()));
            area.bounds = Some(Bounds::new(
                point(px(8.), px(400.)),
                size(px(300.), px(80.)),
            ));
            assert_eq!(area.take_long_press_offset(started), Some(12));
            assert_eq!(
                area.take_long_press_offset(started),
                None,
                "the latch is one-shot"
            );

            area.pending_long_press = Some((started, 12, Instant::now()));
            assert_eq!(
                area.take_long_press_offset(point(px(41.), px(700.))),
                None,
                "another contact cannot inherit the old touch"
            );
        });
    }

    #[gpui::test]
    fn a_long_press_selects_the_word_under_it(cx: &mut TestAppContext) {
        crate::theme::install_for_tests(cx);
        let area = cx.new(|cx| TextArea::multiline("", 4, cx));
        area.update(cx, |area, _| {
            area.content = "Also accept it for DeepSeek, please".into();
            assert_eq!(&area.content[area.word_at(16)], "for", "inside a word");
            assert_eq!(&area.content[area.word_at(14)], "it", "just after a word");
            assert_eq!(&area.content[area.word_at(19)], "DeepSeek", "at its start");
            assert_eq!(
                &area.content[area.word_at(27)],
                "DeepSeek",
                "just after its end"
            );
            assert_eq!(&area.content[area.word_at(28)], " ", "between words");
        });
    }

    #[gpui::test]
    fn the_bar_offers_what_fits_the_selection(cx: &mut TestAppContext) {
        crate::theme::install_for_tests(cx);
        let area = cx.new(|cx| TextArea::multiline("", 4, cx));
        area.update(cx, |area, cx| {
            assert_eq!(
                area.menu_items(),
                [MenuItem::Paste],
                "an empty field can only paste"
            );
            area.set_text("lines 211–212", cx);
            assert_eq!(area.menu_items(), [MenuItem::Paste, MenuItem::SelectAll]);
            area.selection = 0..5;
            assert_eq!(
                area.menu_items(),
                [
                    MenuItem::Cut,
                    MenuItem::Copy,
                    MenuItem::Paste,
                    MenuItem::SelectAll
                ]
            );
            area.selection = 0..area.content.len();
            assert_eq!(
                area.menu_items(),
                [MenuItem::Cut, MenuItem::Copy, MenuItem::Paste]
            );
        });
    }

    #[gpui::test]
    fn tapping_away_leaves_the_composing_word_to_the_keyboard(cx: &mut TestAppContext) {
        crate::theme::install_for_tests(cx);
        let area = cx.new(|cx| TextArea::multiline("", 4, cx));
        cx.update(|cx| {
            area.update(cx, |area, cx| {
                area.content = "hello PASTED".into();
                area.marked = Some(6..12);
                area.selection = 12..12;
                area.move_to(2, cx);
                assert_eq!(area.marked, Some(6..12), "the keyboard ends it");
                assert_eq!(area.selection, 2..2);
            })
        });
        let window = cx.add_empty_window();
        window.update(|window, cx| {
            area.update(cx, |area, cx| {
                area.edit_selection("X", window, cx);
                assert_eq!(
                    area.content, "heXllo PASTED",
                    "inserted at the caret, not over the word"
                );
                assert_eq!(area.marked, None);
            })
        });
    }

    #[gpui::test]
    fn one_line_fields_flatten_pasted_lines(cx: &mut TestAppContext) {
        crate::theme::install_for_tests(cx);
        let area = cx.new(|cx| TextArea::single_line("", TextInputConfiguration::default(), cx));
        area.update(cx, |area, cx| {
            area.set_text("nick@studio-mac\n.local", cx);
            assert_eq!(area.text(), "nick@studio-mac .local");
        });
    }

    #[gpui::test]
    fn single_line_input_scrolls_horizontally_and_reveals_home(cx: &mut TestAppContext) {
        crate::theme::install_for_tests(cx);
        let (area, cx) = cx.add_window_view(|_, cx| {
            TextArea::single_line("", TextInputConfiguration::default(), cx)
        });
        cx.simulate_resize(size(px(240.), px(400.)));
        area.update(cx, |area, cx| {
            area.set_text("user@very-long-computer-name.example.com/".repeat(15), cx)
        });
        cx.run_until_parked();
        area.read_with(cx, |area, _| {
            let layout = area.layout.as_ref().unwrap();
            assert_eq!(layout.height(), layout.line_height);
            assert_eq!(area.scroll, px(0.));
            assert!(area.scroll_x > px(0.));
        });
        area.update(cx, |area, cx| area.move_to(0, cx));
        cx.run_until_parked();
        area.read_with(cx, |area, _| assert_eq!(area.scroll_x, px(0.)));
    }

    #[gpui::test]
    fn unicode_editing_preserves_graphemes_and_utf16_ranges(cx: &mut TestAppContext) {
        crate::theme::install_for_tests(cx);
        let (area, cx) = cx.add_window_view(|_, cx| TextArea::multiline("", 4, cx));
        for grapheme in ["e\u{301}", "👩🏽‍💻", "🇩🇪", "中"] {
            cx.update(|window, cx| {
                area.update(cx, |area, cx| {
                    area.set_text(format!("before {grapheme}"), cx);
                    area.backspace(&Backspace, window, cx);
                    assert_eq!(area.text(), "before ");
                    area.insert(grapheme, cx);
                    let utf16 = area.range_to_utf16(&(7..area.content.len()));
                    assert_eq!(area.range_from_utf16(&utf16), 7..area.content.len());
                    area.set_selected_text_range(utf16, window, cx);
                    area.replace_text_in_range(None, "after", window, cx);
                    assert_eq!(area.text(), "before after");
                })
            });
        }
    }

    #[gpui::test]
    fn paste_and_composition_normalize_line_endings(cx: &mut TestAppContext) {
        crate::theme::install_for_tests(cx);
        let (area, cx) = cx.add_window_view(|_, cx| {
            TextArea::single_line("", TextInputConfiguration::default(), cx)
        });
        cx.update(|window, cx| {
            area.update(cx, |area, cx| {
                area.paste(
                    ClipboardItem::new_string("one\r\ntwo\rthree\nfour".into()),
                    window,
                    cx,
                );
                assert_eq!(area.text(), "one two three four");
                area.select(0..area.content.len(), cx);
                area.replace_and_mark_text_in_range(None, "中文\r\ntext", None, window, cx);
                assert_eq!(area.text(), "中文 text");
                assert!(area.content.is_char_boundary(area.selection.end));
            })
        });
    }

    #[gpui::test]
    fn long_drafts_can_scroll_without_moving_the_caret(cx: &mut TestAppContext) {
        crate::theme::install_for_tests(cx);
        let (area, cx) = cx.add_window_view(|_, cx| TextArea::multiline("", 4, cx));
        cx.simulate_resize(size(px(280.), px(500.)));
        area.update(cx, |area, cx| {
            area.set_text("A long line of text for editing.\n".repeat(80), cx)
        });
        cx.run_until_parked();
        let (before, caret, bounds) = area.read_with(cx, |area, _| {
            (area.scroll, area.selection.clone(), area.bounds.unwrap())
        });
        assert!(before > px(0.));
        cx.simulate_event(ScrollWheelEvent {
            position: bounds.center(),
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(60.))),
            ..Default::default()
        });
        cx.run_until_parked();
        area.read_with(cx, |area, _| {
            assert!(
                area.scroll < before,
                "the viewport must follow the scroll gesture"
            );
            assert_eq!(area.selection, caret);
        });
        area.update(cx, |area, cx| area.insert("tail", cx));
        cx.run_until_parked();
        area.read_with(cx, |area, _| {
            assert!(area.scroll >= before, "editing reveals the caret again")
        });
    }
}
