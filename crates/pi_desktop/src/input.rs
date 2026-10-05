// Adapted from Zed's crates/gpui/examples/input.rs (Apache-2.0).
// See THIRD_PARTY.md for revision, license, and local changes.
use crate::theme::{MONO, theme};

use std::{cell::RefCell, ops::Range, rc::Rc};

use gpui::{
    Anchor, App, AvailableSpace, Bounds, ClipboardItem, ContentMask, Context, CursorStyle,
    DismissEvent, ElementId, ElementInputHandler, Entity, EntityInputHandler, FocusHandle,
    Focusable, Font, GlobalElementId, KeyBinding, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, ScrollWheelEvent, SharedString, Size,
    Style, TextRun, TransformationMatrix, UTF16Selection, UnderlineStyle, Window, WrappedLine,
    actions, anchored, deferred, div, fill, point, prelude::*, px, quad, relative, size,
};
use ui::{ContextMenu, ScrollAxes, ScrollableHandle, Scrollbars, WithScrollbar as _};
use unicode_segmentation::*;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectAll,
        Home,
        End,
        Newline,
        ShowCharacterPalette,
        Paste,
        Cut,
        Copy,
    ]
);

/// An atomic token drawn as a chip: an icon, then its label. The cursor never
/// stops inside it, and an edit that touches it removes all of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Chip {
    /// Covers [`CHIP_LEAD`] and the label.
    pub range: Range<usize>,
    pub icon: &'static str,
    /// The owner's key for what the chip stands for.
    pub id: usize,
}

/// Starts a chip's text and leaves room for its icon: an em space and a hair space.
pub const CHIP_LEAD: &str = "\u{2003}\u{200A}";

/// Bridge the custom text layout to the shared native scrollbar. GPUI scrollbar
/// offsets are negative; the text renderer uses positive offsets.
#[derive(Clone, Default)]
struct InputScrollHandle(Rc<RefCell<InputScrollState>>);

#[derive(Default)]
struct InputScrollState {
    viewport: Bounds<Pixels>,
    overflow: Pixels,
    offset: Pixels,
    requested: Option<Pixels>,
}

impl ScrollableHandle for InputScrollHandle {
    fn max_offset(&self) -> Point<Pixels> {
        point(px(0.), self.0.borrow().overflow)
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        let mut state = self.0.borrow_mut();
        state.offset = (-offset.y).clamp(px(0.), state.overflow);
        state.requested = Some(state.offset);
    }

    fn offset(&self) -> Point<Pixels> {
        point(px(0.), -self.0.borrow().offset)
    }

    fn viewport(&self) -> Bounds<Pixels> {
        self.0.borrow().viewport
    }
}

pub struct TextInput {
    focus_handle: FocusHandle,
    content: SharedString,
    chips: Vec<Chip>,
    placeholder: SharedString,
    font_size: Pixels,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<Layout>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    /// Sideways for a single-line input, downwards for a multi-line one.
    scroll_offset: Point<Pixels>,
    scrollbar: InputScrollHandle,
    /// Follow the caret until manually scrolled; editing/navigation resumes following.
    reveal_cursor: bool,
    /// `None` keeps one line that scrolls sideways. `Some(n)` wraps and grows to `n`
    /// lines before scrolling.
    max_lines: Option<usize>,
    /// A multi-line input that fills its parent's height instead of growing with the text.
    fill: bool,
    /// The x that Up and Down keep, so moving through a short line returns to the column.
    goal_x: Option<Pixels>,
    /// The right-click menu and where it opened. Drawn by the input itself rather than
    /// ui's `right_click_menu`, whose wrapper would give the input an auto height.
    context_menu: Option<(Point<Pixels>, Entity<ContextMenu>)>,
}

pub fn init(cx: &mut App) {
    let context = Some("TextInput");
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("left", Left, context),
        KeyBinding::new("right", Right, context),
        KeyBinding::new("up", Up, context),
        KeyBinding::new("down", Down, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("shift-up", SelectUp, context),
        KeyBinding::new("shift-down", SelectDown, context),
        KeyBinding::new("home", Home, context),
        KeyBinding::new("end", End, context),
        KeyBinding::new("shift-enter", Newline, context),
    ]);
    // `secondary` is Cmd on macOS and Ctrl elsewhere.
    cx.bind_keys([
        KeyBinding::new("secondary-a", SelectAll, context),
        KeyBinding::new("secondary-c", Copy, context),
        KeyBinding::new("secondary-v", Paste, context),
        KeyBinding::new("secondary-x", Cut, context),
    ]);
}

impl TextInput {
    pub fn new(placeholder: &str, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: "".into(),
            chips: Vec::new(),
            placeholder: placeholder.to_owned().into(),
            font_size: px(14.),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            scroll_offset: Point::default(),
            scrollbar: InputScrollHandle::default(),
            reveal_cursor: true,
            max_lines: None,
            fill: false,
            goal_x: None,
            context_menu: None,
        }
    }

    pub fn compact(mut self) -> Self {
        self.font_size = px(12.);
        self
    }

    pub fn font_size(mut self, size: Pixels) -> Self {
        self.font_size = size;
        self
    }

    /// Keeps newlines and wraps, growing to `max_lines` before it scrolls.
    pub fn multiline(mut self, max_lines: usize) -> Self {
        self.max_lines = Some(max_lines);
        self
    }

    /// For a multi-line input: fill the parent's height rather than fit the text.
    pub fn set_fill(&mut self, fill: bool, cx: &mut Context<Self>) {
        self.fill = fill;
        cx.notify();
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn chips(&self) -> &[Chip] {
        &self.chips
    }

    pub fn cursor(&self) -> usize {
        self.cursor_offset()
    }

    /// Replaces `range` with a chip and a space after it, and puts the cursor after both.
    pub fn insert_chip(
        &mut self,
        range: Range<usize>,
        label: &str,
        icon: &'static str,
        id: usize,
        cx: &mut Context<Self>,
    ) {
        let range = self.expand_to_chips(range);
        // Non-breaking spaces keep a label's words together.
        let text = format!("{CHIP_LEAD}{}", label.replace(' ', "\u{a0}"));
        self.edit(range.clone(), &format!("{text} "));
        let chip = range.start..range.start + text.len();
        let at = self.chips.partition_point(|c| c.range.start < chip.start);
        self.chips.insert(
            at,
            Chip {
                range: chip,
                icon,
                id,
            },
        );
        let end = range.start + text.len() + 1;
        self.selected_range = end..end;
        self.selection_reversed = false;
        self.marked_range = None;
        self.goal_x = None;
        cx.notify();
    }

    /// Replaces a byte range with plain text and puts the cursor after it.
    pub fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        let range = self.expand_to_chips(range);
        self.edit(range.clone(), text);
        let end = range.start + text.len();
        self.selected_range = end..end;
        self.selection_reversed = false;
        self.marked_range = None;
        self.goal_x = None;
        cx.notify();
    }

    /// Text as typed: chips give their labels, with plain spaces.
    pub fn plain_text(&self, range: Range<usize>) -> String {
        self.content[range]
            .replace(CHIP_LEAD, "")
            .replace('\u{a0}', " ")
    }

    /// Replaces text, dropping chips the range touches and moving the ones after it.
    fn edit(&mut self, range: Range<usize>, new_text: &str) {
        self.request_cursor_reveal();
        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        let grown = new_text.len() as isize - (range.end - range.start) as isize;
        // An insertion at a chip's edge keeps it; anything overlapping it removes it.
        self.chips
            .retain(|chip| !(chip.range.start < range.end && range.start < chip.range.end));
        for chip in &mut self.chips {
            if chip.range.start >= range.end {
                chip.range.start = (chip.range.start as isize + grown) as usize;
                chip.range.end = (chip.range.end as isize + grown) as usize;
            }
        }
    }

    fn chip_at(&self, offset: usize) -> Option<&Chip> {
        self.chips
            .iter()
            .find(|chip| chip.range.start < offset && offset < chip.range.end)
    }

    /// The nearest offset that is not inside a chip.
    fn snap(&self, offset: usize) -> usize {
        match self.chip_at(offset) {
            Some(chip) if offset - chip.range.start < chip.range.end - offset => chip.range.start,
            Some(chip) => chip.range.end,
            None => offset,
        }
    }

    /// A range that covers every chip it touches entirely.
    fn expand_to_chips(&self, range: Range<usize>) -> Range<usize> {
        let start = self
            .chip_at(range.start)
            .map_or(range.start, |c| c.range.start);
        let end = self.chip_at(range.end).map_or(range.end, |c| c.range.end);
        start..end
    }

    #[cfg(test)]
    pub fn height(&self) -> Pixels {
        self.last_bounds.map_or(px(0.), |bounds| bounds.size.height)
    }

    #[cfg(test)]
    pub fn vertical_scroll(&self) -> Pixels {
        self.scroll_offset.y
    }

    pub fn set_content(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.reset();
        let text = text.into();
        self.content = if self.max_lines.is_some() {
            text.replace("\r\n", "\n").into()
        } else {
            text.replace(['\n', '\r'], " ").into()
        };
        self.chips.clear();
        self.selected_range = self.content.len()..self.content.len();
        cx.notify();
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(-1., false, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(1., false, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(-1., true, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertically(1., true, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let line = self.line_range(self.cursor_offset());
        self.move_to(line.start, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let line = self.line_range(self.cursor_offset());
        self.move_to(line.end, cx);
    }

    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.max_lines.is_some() {
            self.replace_text_in_range(None, "\n", window, cx);
        }
    }

    /// Up and Down move a row; past the first or last row they go to the start or end.
    fn move_vertically(&mut self, rows: f32, select: bool, cx: &mut Context<Self>) {
        let offset = match &self.last_layout {
            Some(layout) if self.max_lines.is_some() => {
                let current = layout.position_for_offset(self.cursor_offset());
                let x = self.goal_x.unwrap_or(current.x);
                let y = current.y + layout.line_height * rows;
                let offset = if y < px(0.) {
                    0
                } else if y >= layout.height() {
                    self.content.len()
                } else {
                    layout.offset_for_position(point(x, y + layout.line_height / 2.))
                };
                (offset, Some(x))
            }
            _ if rows < 0. => (0, None),
            _ => (self.content.len(), None),
        };
        if select {
            self.select_to(offset.0, cx);
        } else {
            self.move_to(offset.0, cx);
        }
        self.goal_x = offset.1;
    }

    /// The hard line (between newlines) containing `offset`.
    fn line_range(&self, offset: usize) -> Range<usize> {
        let start = self.content[..offset]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let end = self.content[offset..]
            .find('\n')
            .map_or(self.content.len(), |index| offset + index);
        start..end
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let prev = self.previous_boundary(self.cursor_offset());
            if self.cursor_offset() == prev {
                window.play_system_bell();
                return;
            }
            self.select_to(prev, cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let next = self.next_boundary(self.cursor_offset());
            if self.cursor_offset() == next {
                window.play_system_bell();
                return;
            }
            self.select_to(next, cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window, cx);
        self.is_selecting = true;
        let offset = self.index_for_mouse_position(event.position);
        match event.click_count {
            2 => {
                let word = self.word_range(offset);
                self.move_to(word.start, cx);
                self.select_to(word.end, cx);
            }
            3.. => {
                self.move_to(0, cx);
                self.select_to(self.content.len(), cx);
            }
            _ if event.modifiers.shift => self.select_to(offset, cx),
            _ => self.move_to(offset, cx),
        }
    }

    /// The word (or run of spaces/punctuation) at `offset`, by Unicode word boundaries.
    fn word_range(&self, offset: usize) -> Range<usize> {
        if let Some(chip) = self.chips.iter().find(|c| c.range.contains(&offset)) {
            return chip.range.clone();
        }
        let mut segments = self
            .content
            .split_word_bound_indices()
            .map(|(start, segment)| start..start + segment.len());
        segments
            .find(|segment| segment.contains(&offset) || segment.end == self.content.len())
            .unwrap_or(offset..offset)
    }

    fn on_right_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window, cx);
        let focus = self.focus_handle.clone();
        // Actions dispatch to the input, which does not hold focus while the menu is open.
        let menu = ContextMenu::build(window, cx, move |menu, _, _| {
            menu.context(focus)
                .action("Cut", Box::new(Cut))
                .action("Copy", Box::new(Copy))
                .action("Paste", Box::new(Paste))
                .separator()
                .action("Select All", Box::new(SelectAll))
        });
        cx.subscribe_in(&menu, window, |input, _, _: &DismissEvent, window, cx| {
            input.context_menu = None;
            input.focus_handle.focus(window, cx);
            cx.notify();
        })
        .detach();
        // Deferred menus join the dispatch tree a frame later; focus them after that, as
        // ui's right-click menu does.
        let menu_focus = menu.focus_handle(cx);
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |window, cx| window.focus(&menu_focus, cx));
        });
        self.context_menu = Some((event.position, menu));
        cx.stop_propagation();
        cx.notify();
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(layout), Some(bounds)) = (&self.last_layout, self.last_bounds) else {
            return;
        };
        let overflow = layout.height() - bounds.size.height;
        if self.max_lines.is_none() || overflow <= px(0.) {
            return;
        }
        let delta = event.delta.pixel_delta(window.line_height()).y;
        let offset = self
            .scrollbar
            .0
            .borrow_mut()
            .requested
            .take()
            .unwrap_or(self.scroll_offset.y);
        self.scroll_offset.y = (offset - delta).max(px(0.)).min(overflow);
        self.reveal_cursor = false;
        cx.stop_propagation();
        cx.notify();
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = if self.max_lines.is_some() {
                text.replace("\r\n", "\n")
            } else {
                text.replace('\n', " ")
            };
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.plain_text(self.selected_range.clone()),
            ));
        }
    }
    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.plain_text(self.selected_range.clone()),
            ));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn request_cursor_reveal(&mut self) {
        self.reveal_cursor = true;
        self.scrollbar.0.borrow_mut().requested = None;
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.request_cursor_reveal();
        let offset = self.snap(offset);
        self.selection_reversed = false;
        self.selected_range = offset..offset;
        self.goal_x = None;
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(layout)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        if self.max_lines.is_none() {
            if position.y < bounds.top() {
                return 0;
            }
            if position.y > bounds.bottom() {
                return self.content.len();
            }
        }
        layout.offset_for_position(position - bounds.origin + self.scroll_offset)
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.request_cursor_reveal();
        let offset = self.snap(offset);
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.goal_x = None;
        cx.notify()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;

        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }

        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;

        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }

        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    /// A chip counts as one character.
    fn previous_boundary(&self, offset: usize) -> usize {
        let previous = self
            .content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0);
        self.chip_at(previous)
            .map_or(previous, |chip| chip.range.start)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        let next = self
            .content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len());
        self.chip_at(next).map_or(next, |chip| chip.range.end)
    }

    fn reset(&mut self) {
        self.content = "".into();
        self.chips.clear();
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        self.last_layout = None;
        self.last_bounds = None;
        self.is_selecting = false;
        self.scroll_offset = Point::default();
        *self.scrollbar.0.borrow_mut() = InputScrollState::default();
        self.reveal_cursor = true;
        self.goal_x = None;
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        let range = self.expand_to_chips(range);

        self.edit(range.clone(), new_text);
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        self.selection_reversed = false;
        self.goal_x = None;
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        let range = self.expand_to_chips(range);

        self.edit(range.clone(), new_text);
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        self.selected_range = new_selected_range_utf16
            .map(|selection| {
                let offset = |n| {
                    let mut units = 0;
                    new_text
                        .char_indices()
                        .find_map(|(index, ch)| {
                            if units >= n {
                                Some(index)
                            } else {
                                units += ch.len_utf16();
                                None
                            }
                        })
                        .unwrap_or(new_text.len())
                };
                range.start + offset(selection.start)..range.start + offset(selection.end)
            })
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        self.selection_reversed = false;

        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let start = layout.position_for_offset(range.start) - self.scroll_offset;
        let end = layout.position_for_offset(range.end) - self.scroll_offset;
        Some(Bounds::from_corners(
            bounds.origin + start,
            bounds.origin + point(end.x.max(start.x), end.y + layout.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let local = self.last_bounds?.localize(&point)?;
        let layout = self.last_layout.as_ref()?;
        if layout.text != self.content {
            return None;
        }
        let utf8_index = layout.offset_for_position(local + self.scroll_offset);
        Some(self.offset_to_utf16(utf8_index))
    }
}

/// Shaped text by hard line, with each line's byte offset.
struct Layout {
    text: SharedString,
    lines: Vec<(usize, WrappedLine)>,
    line_height: Pixels,
}

impl Layout {
    fn shape(
        text: SharedString,
        runs: &[TextRun],
        font_size: Pixels,
        line_height: Pixels,
        wrap_width: Option<Pixels>,
        window: &Window,
    ) -> Self {
        let shaped = window
            .text_system()
            .shape_text(text.clone(), font_size, runs, wrap_width, None)
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
        Self {
            text,
            lines,
            line_height,
        }
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

    fn position_for_offset(&self, offset: usize) -> Point<Pixels> {
        let mut top = px(0.);
        for (start, line) in &self.lines {
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

    fn offset_for_position(&self, position: Point<Pixels>) -> usize {
        let mut top = px(0.);
        for (index, (start, line)) in self.lines.iter().enumerate() {
            let height = self.row_height(line);
            if position.y < top + height || index + 1 == self.lines.len() {
                // Stay inside this line's rows; past its last row the layout reports index 0.
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

    fn paint(&self, origin: Point<Pixels>, window: &mut Window, cx: &mut App) {
        let mut top = px(0.);
        for (_, line) in &self.lines {
            if let Err(error) = line.paint(
                origin + point(px(0.), top),
                self.line_height,
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            ) {
                log::error!("Input paint failed: {error}");
            }
            top += self.row_height(line);
        }
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

/// Placeholder text starts after the caret instead of underneath it.
const PLACEHOLDER_INSET: Pixels = px(5.);

struct PrepaintState {
    scroll_offset: Point<Pixels>,
    reveal_cursor: bool,
    /// Horizontal offset of the painted text; nonzero only for the placeholder.
    text_inset: Pixels,
    layout: Option<Layout>,
    cursor: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
    chips: Vec<PaintQuad>,
    icons: Vec<(Bounds<Pixels>, SharedString)>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// The display text and its runs: the content, or the placeholder when empty.
/// Chips are mono and accent-colored; IME text is underlined.
fn display_runs(input: &TextInput, font: Font, cx: &App) -> (SharedString, Vec<TextRun>) {
    let colors = theme(cx);
    let run = TextRun {
        len: 0,
        font: font.clone(),
        color: colors.text,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    if input.content.is_empty() {
        let text = input.placeholder.clone();
        let placeholder = TextRun {
            len: text.len(),
            color: colors.muted,
            ..run
        };
        return (text, vec![placeholder]);
    }
    let text = input.content.clone();
    let chip_run = TextRun {
        font: Font {
            family: MONO.into(),
            ..font
        },
        color: colors.accent,
        ..run.clone()
    };
    let mut cuts = vec![0, text.len()];
    for chip in &input.chips {
        cuts.extend([chip.range.start, chip.range.end]);
    }
    if let Some(marked) = &input.marked_range {
        cuts.extend([marked.start, marked.end]);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let runs = cuts
        .windows(2)
        .map(|pair| {
            let piece = pair[0]..pair[1];
            let in_chip = input
                .chips
                .iter()
                .any(|chip| chip.range.start <= piece.start && piece.end <= chip.range.end);
            let mut run = if in_chip {
                chip_run.clone()
            } else {
                run.clone()
            };
            run.len = piece.len();
            if input
                .marked_range
                .as_ref()
                .is_some_and(|marked| marked.start <= piece.start && piece.end <= marked.end)
            {
                run.underline = Some(UnderlineStyle {
                    color: Some(run.color),
                    thickness: px(1.0),
                    wavy: false,
                });
            }
            run
        })
        .filter(|run| run.len > 0)
        .collect();
    (text, runs)
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let input = self.input.read(cx);
        let line_height = window.line_height();
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let Some(max_lines) = input.max_lines else {
            style.size.height = line_height.into();
            return (window.request_layout(style, [], cx), ());
        };
        if input.fill {
            style.size.height = relative(1.).into();
            style.min_size.height = line_height.into();
            return (window.request_layout(style, [], cx), ());
        }
        // Grow with the wrapped text, from one line to `max_lines`.
        let text_style = window.text_style();
        let font = text_style.font();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let entity = self.input.clone();
        let layout = window.request_measured_layout(style, move |known, available, window, cx| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(width) => Some(width),
                _ => None,
            });
            let (text, runs) = display_runs(entity.read(cx), font.clone(), cx);
            let layout = Layout::shape(text, &runs, font_size, line_height, width, window);
            let height = layout.height().min(line_height * max_lines as f32);
            Size {
                width: width.unwrap_or_default(),
                height,
            }
        });
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let selected_range = input.selected_range.clone();
        let cursor = input.cursor_offset();
        let multiline = input.max_lines.is_some();
        let style = window.text_style();
        let text_inset = if input.content.is_empty() {
            PLACEHOLDER_INSET
        } else {
            px(0.)
        };
        let (text, runs) = display_runs(input, style.font(), cx);
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let layout = Layout::shape(
            text,
            &runs,
            font_size,
            line_height,
            multiline.then_some(bounds.size.width),
            window,
        );

        // Follow the caret (including on resize) until the user manually scrolls.
        // Editing or keyboard navigation resumes following.
        let caret = layout.position_for_offset(cursor);
        let mut scroll_offset = input.scroll_offset;
        let mut reveal_cursor = input.reveal_cursor;
        if multiline {
            let mut scrollbar = input.scrollbar.0.borrow_mut();
            let requested = scrollbar.requested.take();
            if let Some(offset) = requested {
                scroll_offset.y = offset;
                reveal_cursor = false;
            }
            if reveal_cursor {
                scroll_offset.y = scroll_offset
                    .y
                    .max(caret.y + line_height - bounds.size.height)
                    .min(caret.y);
            }
            let overflow = (layout.height() - bounds.size.height).max(px(0.));
            scroll_offset.y = scroll_offset.y.clamp(px(0.), overflow);
            scrollbar.viewport = bounds;
            scrollbar.overflow = overflow;
            scrollbar.offset = scroll_offset.y;
        } else {
            scroll_offset.x = scroll_offset
                .x
                .max(caret.x - bounds.size.width + px(4.))
                .min(caret.x)
                .max(px(0.));
        }
        let origin = bounds.origin - scroll_offset;
        let colors = theme(cx);
        let (selection, cursor) = if selected_range.is_empty() {
            (
                Vec::new(),
                Some(fill(
                    Bounds::new(origin + caret, size(px(2.), line_height)),
                    colors.accent,
                )),
            )
        } else {
            let start = layout.position_for_offset(selected_range.start);
            let end = layout.position_for_offset(selected_range.end);
            let row = |left: Pixels, right: Pixels, top: Pixels, bottom: Pixels| {
                fill(
                    Bounds::from_corners(origin + point(left, top), origin + point(right, bottom)),
                    colors.selection(),
                )
            };
            let quads = if start.y == end.y {
                vec![row(start.x, end.x, start.y, start.y + line_height)]
            } else {
                // First row to the edge, whole rows between, then the last row's start.
                let width = bounds.size.width;
                vec![
                    row(start.x, width, start.y, start.y + line_height),
                    row(px(0.), width, start.y + line_height, end.y),
                    row(px(0.), end.x, end.y, end.y + line_height),
                ]
            };
            (quads, None)
        };
        // A rounded box behind each chip, one per row it wraps across, and its icon
        // over the lead spaces.
        let mut chips = Vec::new();
        let mut icons = Vec::new();
        for chip in &input.chips {
            let start = layout.position_for_offset(chip.range.start);
            let end = layout.position_for_offset(chip.range.end);
            let rows = if start.y == end.y {
                vec![(start.x, end.x, start.y)]
            } else {
                vec![
                    (start.x, bounds.size.width, start.y),
                    (px(0.), end.x, end.y),
                ]
            };
            for (left, right, top) in rows {
                chips.push(quad(
                    Bounds::from_corners(
                        origin + point(left - px(3.), top + px(1.)),
                        origin + point(right + px(3.), top + line_height - px(1.)),
                    ),
                    px(4.),
                    colors.selected,
                    px(1.),
                    colors.focus,
                    gpui::BorderStyle::Solid,
                ));
            }
            icons.push((
                Bounds::new(
                    origin + point(start.x + px(1.), start.y + (line_height - px(12.)) / 2.),
                    size(px(12.), px(12.)),
                ),
                SharedString::from(format!("icons/{}.svg", chip.icon)),
            ));
        }
        PrepaintState {
            scroll_offset,
            reveal_cursor,
            text_inset,
            layout: Some(layout),
            cursor,
            selection,
            chips,
            icons,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let Some(layout) = prepaint.layout.take() else {
            return;
        };
        // Scrolled text stays inside the input.
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for quad in prepaint.selection.drain(..).chain(prepaint.chips.drain(..)) {
                window.paint_quad(quad);
            }
            layout.paint(
                bounds.origin + point(prepaint.text_inset, px(0.)) - prepaint.scroll_offset,
                window,
                cx,
            );
            let accent = theme(cx).accent;
            for (icon, path) in prepaint.icons.drain(..) {
                if let Err(error) =
                    window.paint_svg(icon, path, None, TransformationMatrix::unit(), accent, cx)
                {
                    log::error!("Chip icon paint failed: {error}");
                }
            }
            if focus_handle.is_focused(window)
                && let Some(cursor) = prepaint.cursor.take()
            {
                window.paint_quad(cursor);
            }
        });

        self.input.update(cx, |input, _cx| {
            input.last_layout = Some(layout);
            input.last_bounds = Some(bounds);
            input.scroll_offset = prepaint.scroll_offset;
            input.reveal_cursor = prepaint.reveal_cursor;
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("text-input")
            .flex()
            .key_context("TextInput")
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .w_full()
            .when(self.fill, |input| input.h_full())
            .min_w_0()
            .overflow_hidden()
            .line_height(px(20.))
            .text_size(self.font_size)
            .text_color(theme(cx).text)
            .when(self.max_lines.is_some(), |input| input.items_start())
            .child(
                div()
                    .id("input-viewport")
                    .w_full()
                    .when(self.fill, |text| text.h_full())
                    .py(px(2.))
                    // Match the shared regular scrollbar's 6px thumb + 4px padding
                    // on each side. A stable gutter avoids rewrapping when it appears.
                    .when(self.max_lines.is_some(), |text| text.pr(px(14.)))
                    .child(TextElement { input: cx.entity() })
                    .when(self.max_lines.is_some(), |text| {
                        text.custom_scrollbars(
                            Scrollbars::always_visible(ScrollAxes::Vertical)
                                .id("input-scrollbar")
                                .style(ui::ScrollbarStyle::Regular)
                                .tracked_scroll_handle(&self.scrollbar)
                                .tracked_entity(cx.entity_id())
                                // The shared scrollbar blends its thumb into this base;
                                // a transparent base would make the idle thumb invisible.
                                .with_track_along(ScrollAxes::Vertical, theme(cx).composer),
                            window,
                            cx,
                        )
                    }),
            )
            .children(self.context_menu.as_ref().map(|(position, menu)| {
                deferred(
                    anchored()
                        .position(*position)
                        .anchor(Anchor::TopLeft)
                        .snap_to_window_with_margin(px(8.))
                        .child(menu.clone()),
                )
                .with_priority(1)
            }))
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_window(
        cx: &mut gpui::TestAppContext,
        build: impl FnOnce(TextInput) -> TextInput + 'static,
    ) -> (Entity<TextInput>, gpui::VisualTestContext) {
        let window = cx.update(|cx| {
            cx.set_global(crate::theme::Theme::new(false));
            // Zed's theme and settings, which the edit menu's components read.
            crate::markdown_view::init(cx);
            init(cx);
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| build(TextInput::new("", cx)))
            })
            .unwrap()
        });
        let mut cx = gpui::VisualTestContext::from_window(window.into(), cx);
        let input = window.root(&mut cx).unwrap();
        (input, cx)
    }

    #[gpui::test]
    fn utf16_selection_and_grapheme_deletion(cx: &mut gpui::TestAppContext) {
        let (input, mut cx) = input_window(cx, |input| input);
        input.update_in(&mut cx, |input, window, cx| {
            input.set_content("a🐈e\u{301}", cx);
            assert_eq!(input.offset_to_utf16(input.content.len()), 5);
            input.backspace(&Backspace, window, cx);
            assert_eq!(input.content(), "a🐈");
            input.replace_text_in_range(Some(1..3), "b", window, cx);
            assert_eq!(input.content(), "ab");
        });
    }

    #[gpui::test]
    fn double_click_word_follows_unicode_word_boundaries(cx: &mut gpui::TestAppContext) {
        let (input, mut cx) = input_window(cx, |input| input);
        input.update(&mut cx, |input, cx| {
            input.set_content("Run the qwen🐈 tests", cx);
            assert_eq!(input.word_range(9), 8..12);
            assert_eq!(input.word_range(7), 7..8);
            assert_eq!(input.word_range(12), 12..16);
            assert_eq!(input.word_range(input.content.len()), 17..22);
            input.set_content("", cx);
            assert_eq!(input.word_range(0), 0..0);
        });
    }

    #[gpui::test]
    fn ime_selection_is_relative_to_inserted_text_not_replaced_range(
        cx: &mut gpui::TestAppContext,
    ) {
        let (input, mut cx) = input_window(cx, |input| input);
        input.update_in(&mut cx, |input, window, cx| {
            input.set_content("prefix old suffix", cx);
            input.replace_and_mark_text_in_range(Some(7..10), "🐈x", Some(2..3), window, cx);
            assert_eq!(input.content(), "prefix 🐈x suffix");
            assert_eq!(input.selected_range, 11..12);
            assert_eq!(input.marked_range, Some(7..12));
            input.replace_text_in_range(None, "done", window, cx);
            assert_eq!(input.content(), "prefix done suffix");
            assert!(input.marked_range.is_none());
        });
    }

    #[gpui::test]
    fn single_line_inputs_flatten_newlines_and_multi_line_inputs_keep_them(
        cx: &mut gpui::TestAppContext,
    ) {
        let (single, mut single_cx) = input_window(cx, |input| input);
        single.update(&mut single_cx, |input, cx| {
            input.set_content("one\ntwo", cx);
            assert_eq!(input.content(), "one two");
        });
        let (multi, mut cx) = input_window(cx, |input| input.multiline(8));
        multi.update_in(&mut cx, |input, window, cx| {
            input.set_content("one\r\ntwo", cx);
            assert_eq!(input.content(), "one\ntwo");
            input.newline(&Newline, window, cx);
            assert_eq!(input.content(), "one\ntwo\n");
            assert_eq!(input.line_range(5), 4..7);
        });
    }

    #[gpui::test]
    fn multi_line_input_grows_moves_between_lines_and_then_scrolls(cx: &mut gpui::TestAppContext) {
        let (input, mut cx) = input_window(cx, |input| input.multiline(3));
        input.update_in(&mut cx, |input, window, cx| {
            input.focus_handle.focus(window, cx);
            input.set_content("first\nsecond", cx)
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("up");
        input.read_with(&cx, |input, _| {
            // From the end of "second" (column 6) to the same column on "first" (its end, 5).
            assert_eq!(input.cursor_offset(), 5);
        });
        cx.simulate_keystrokes("down");
        input.read_with(&cx, |input, _| assert_eq!(input.cursor_offset(), 12));
        cx.simulate_keystrokes("shift-up");
        input.read_with(&cx, |input, _| assert_eq!(input.selected_range, 5..12));
        let two_lines = input.read_with(&cx, |input, _| input.last_bounds.unwrap().size.height);
        assert_eq!(two_lines, px(40.));
        input.update(&mut cx, |input, cx| {
            input.set_content("1\n2\n3\n4\n5\n6", cx)
        });
        cx.run_until_parked();
        input.read_with(&cx, |input, _| {
            // Capped at three lines, scrolled to keep the caret on the last line in view.
            assert_eq!(input.last_bounds.unwrap().size.height, px(60.));
            assert_eq!(input.scroll_offset.y, px(60.));
        });
    }

    #[gpui::test]
    fn multiline_wheel_scroll_persists_until_cursor_navigation(cx: &mut gpui::TestAppContext) {
        let (input, mut cx) = input_window(cx, |input| input.multiline(3));
        input.update_in(&mut cx, |input, window, cx| {
            input.focus_handle.focus(window, cx);
            input.set_content("1\n2\n3\n4\n5\n6", cx);
        });
        cx.run_until_parked();
        let bounds = input.read_with(&cx, |input, _| input.last_bounds.unwrap());
        cx.simulate_event(ScrollWheelEvent {
            position: bounds.center(),
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(60.))),
            ..Default::default()
        });
        cx.run_until_parked();
        input.read_with(&cx, |input, _| {
            assert_eq!(input.scroll_offset.y, px(0.));
            assert_eq!(input.cursor_offset(), input.content.len());
            assert_eq!(input.scrollbar.max_offset().y, px(60.));
            assert_eq!(input.scrollbar.offset().y, px(0.));
        });
        input.update(&mut cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert_eq!(
            input.read_with(&cx, |input, _| input.vertical_scroll()),
            px(0.)
        );
        cx.simulate_keystrokes("end");
        cx.run_until_parked();
        assert_eq!(
            input.read_with(&cx, |input, _| input.vertical_scroll()),
            px(60.)
        );
    }

    #[gpui::test]
    fn multiline_scrollbar_track_and_drag_preserve_text_and_selection(
        cx: &mut gpui::TestAppContext,
    ) {
        let (input, mut cx) = input_window(cx, |input| input.multiline(3));
        input.update(&mut cx, |input, cx| {
            input.set_content("1\n2\n3\n4\n5\n6", cx)
        });
        cx.run_until_parked();
        let bounds = input.read_with(&cx, |input, _| input.last_bounds.unwrap());
        let top = point(bounds.right() + px(7.), bounds.top() + px(7.));
        cx.simulate_click(top, gpui::Modifiers::default());
        cx.run_until_parked();
        input.read_with(&cx, |input, _| {
            assert_eq!(input.vertical_scroll(), px(0.));
            assert_eq!(input.selected_range, 11..11);
            assert!(!input.is_selecting);
        });
        cx.simulate_mouse_down(top, MouseButton::Left, gpui::Modifiers::default());
        // Drag outside the input: the shared scrollbar retains capture.
        let bottom = point(top.x, bounds.bottom() + px(100.));
        cx.simulate_mouse_move(bottom, Some(MouseButton::Left), gpui::Modifiers::default());
        cx.simulate_mouse_up(bottom, MouseButton::Left, gpui::Modifiers::default());
        cx.run_until_parked();
        input.read_with(&cx, |input, _| {
            assert_eq!(input.vertical_scroll(), px(60.));
            assert_eq!(input.selected_range, 11..11);
            assert_eq!(input.content(), "1\n2\n3\n4\n5\n6");
        });
        input.update(&mut cx, |input, cx| input.set_content("short", cx));
        cx.run_until_parked();
        input.read_with(&cx, |input, _| {
            assert_eq!(input.scrollbar.max_offset().y, px(0.));
            assert_eq!(input.vertical_scroll(), px(0.));
        });
    }

    #[gpui::test]
    fn wrapped_and_expanded_inputs_report_scrollbar_overflow(cx: &mut gpui::TestAppContext) {
        let (input, mut cx) = input_window(cx, |input| input.multiline(3));
        input.update(&mut cx, |input, cx| {
            input.set_content("wrapped text ".repeat(300), cx)
        });
        cx.run_until_parked();
        input.read_with(&cx, |input, _| {
            assert_eq!(input.height(), px(60.));
            assert!(input.scrollbar.max_offset().y > px(0.));
        });
        input.update(&mut cx, |input, cx| {
            input.set_fill(true, cx);
            input.set_content("line\n".repeat(100), cx);
        });
        cx.run_until_parked();
        input.read_with(&cx, |input, _| {
            assert!(input.height() > px(60.));
            assert!(input.scrollbar.max_offset().y > px(0.));
            assert_eq!(-input.scrollbar.offset().y, input.vertical_scroll());
        });
    }

    #[gpui::test]
    fn resizing_keeps_the_caret_visible_unless_manually_scrolled(cx: &mut gpui::TestAppContext) {
        let (input, mut cx) = input_window(cx, |input| input.multiline(3));
        input.update(&mut cx, |input, cx| {
            input.set_content("line\n".repeat(100), cx)
        });
        cx.run_until_parked();
        for fill in [true, false] {
            input.update(&mut cx, |input, cx| input.set_fill(fill, cx));
            cx.run_until_parked();
            input.read_with(&cx, |input, _| {
                assert_eq!(input.vertical_scroll(), input.scrollbar.max_offset().y);
            });
        }
        input.update(&mut cx, |input, cx| {
            input.scrollbar.set_offset(Point::default());
            cx.notify();
        });
        cx.run_until_parked();
        for fill in [true, false] {
            input.update(&mut cx, |input, cx| input.set_fill(fill, cx));
            cx.run_until_parked();
            assert_eq!(
                input.read_with(&cx, |input, _| input.vertical_scroll()),
                px(0.)
            );
        }
    }

    #[gpui::test]
    fn chips_move_and_delete_as_one_character(cx: &mut gpui::TestAppContext) {
        let (input, mut cx) = input_window(cx, |input| input.multiline(8));
        input.update_in(&mut cx, |input, window, cx| {
            input.set_content("Compare @op", cx);
            input.insert_chip(8..11, "opencode ts", "file", 7, cx);
            let chip = input.chips()[0].clone();
            assert_eq!(chip.id, 7);
            assert_eq!(
                &input.content()[chip.range.clone()],
                format!("{CHIP_LEAD}opencode\u{a0}ts")
            );
            assert_eq!(
                input.plain_text(0..input.content().len()),
                "Compare opencode ts "
            );
            // Typing before the chip moves it along.
            input.move_to(0, cx);
            input.replace_text_in_range(None, "> ", window, cx);
            let chip = input.chips()[0].clone();
            assert_eq!(chip.range.start, 10);
            // Right and Left step over it whole; the cursor never lands inside.
            input.move_to(chip.range.start, cx);
            input.right(&Right, window, cx);
            assert_eq!(input.cursor_offset(), chip.range.end);
            input.left(&Left, window, cx);
            assert_eq!(input.cursor_offset(), chip.range.start);
            input.move_to(chip.range.start + 2, cx);
            assert!(
                [chip.range.start, chip.range.end].contains(&input.cursor_offset()),
                "clicks inside a chip snap to its edge"
            );
            // Backspace after it removes all of it.
            input.move_to(chip.range.end, cx);
            input.backspace(&Backspace, window, cx);
            assert_eq!(input.content(), "> Compare  ");
            assert!(input.chips().is_empty());
        });
    }

    #[gpui::test]
    fn right_click_opens_the_edit_menu(cx: &mut gpui::TestAppContext) {
        let (input, mut cx) = input_window(cx, |input| input.multiline(8));
        cx.run_until_parked();
        let bounds = input.read_with(&cx, |input, _| input.last_bounds.unwrap());
        cx.simulate_mouse_down(bounds.center(), MouseButton::Right, gpui::Modifiers::none());
        input.read_with(&cx, |input, _| assert!(input.context_menu.is_some()));
    }
}
