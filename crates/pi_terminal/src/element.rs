//! The GPUI element that paints a terminal grid and routes pointer and IME input to it.
//!
//! Adapted from Zed's `terminal_view::terminal_element` (GPL-3.0-or-later). Zed's
//! version needs a `Workspace` and `editor`; this one drops the workspace, inline
//! (embedded) layout, blocks below the cursor, search highlights and hover tooltips,
//! and paints the cursor and selection itself.

use crate::{TerminalStyle, TerminalView};
use gpui::{
    App, BorderStyle, Bounds, ContentMask, DispatchPhase, Element, ElementId, Entity, FocusHandle,
    Font, FontFeatures, FontStyle, FontWeight, GlobalElementId, HighlightStyle, Hitbox, Hsla,
    InputHandler, InteractiveElement, Interactivity, IntoElement, LayoutId, ModifiersChangedEvent,
    MouseButton, MouseMoveEvent, Pixels, Point as GpuiPoint, ShapedLine,
    StatefulInteractiveElement, StrikethroughStyle, TextRun, TextStyle, UTF16Selection,
    UnderlineStyle, WhiteSpace, Window, fill, outline, point, px, relative, size,
};
use itertools::Itertools;
use settings::Settings;
use std::mem;
use terminal::{
    Cell, Color, Content, CursorShape, IndexedCell, Modes, MouseInputMode, NamedColor, Point,
    Range, Terminal, TerminalBounds,
    is_app_chosen_exact_color as terminal_is_app_chosen_exact_color, is_default_background_color,
    terminal_settings::TerminalSettings,
};
use ui::utils::ensure_minimum_contrast;
use util::ResultExt;
use zed_theme::{ActiveTheme, Theme};

/// The information generated during layout that is necessary for painting.
pub struct LayoutState {
    hitbox: Hitbox,
    batched_text_runs: Vec<BatchedTextRun>,
    block_element_rects: Vec<BlockElementLayoutRect>,
    rects: Vec<LayoutRect>,
    selection: Option<Range>,
    cursor: Option<CursorLayout>,
    ime_cursor_bounds: Option<Bounds<Pixels>>,
    background_color: Hsla,
    dimensions: TerminalBounds,
    display_offset: usize,
    base_text_style: TextStyle,
}

/// Helper struct for converting terminal cursor points to displayed cursor points.
#[derive(Copy, Clone)]
struct DisplayCursor {
    line: i32,
    col: usize,
}

impl DisplayCursor {
    fn from(cursor_point: Point, display_offset: usize) -> Self {
        Self {
            line: cursor_point.line + display_offset as i32,
            col: cursor_point.column,
        }
    }
}

#[derive(Copy, Clone, Debug, Default)]
struct LayoutPoint {
    line: i32,
    column: i32,
}

impl LayoutPoint {
    fn new(line: i32, column: i32) -> Self {
        Self { line, column }
    }
}

/// A batched text run that combines multiple adjacent cells with the same style.
#[derive(Debug)]
struct BatchedTextRun {
    start_point: LayoutPoint,
    text: String,
    cell_count: usize,
    style: TextRun,
    font_size: Pixels,
}

impl BatchedTextRun {
    fn new_from_char(start_point: LayoutPoint, c: char, style: TextRun, font_size: Pixels) -> Self {
        let mut text = String::with_capacity(100);
        text.push(c);
        BatchedTextRun {
            start_point,
            text,
            cell_count: 1,
            style,
            font_size,
        }
    }

    fn can_append(&self, other_style: &TextRun) -> bool {
        self.style.font == other_style.font
            && self.style.color == other_style.color
            && self.style.background_color == other_style.background_color
            && self.style.underline == other_style.underline
            && self.style.strikethrough == other_style.strikethrough
    }

    fn append_char(&mut self, c: char) {
        self.append_char_internal(c, true);
    }

    fn append_zero_width_chars(&mut self, chars: &[char]) {
        for &c in chars {
            self.append_char_internal(c, false);
        }
    }

    fn append_char_internal(&mut self, c: char, counts_cell: bool) {
        self.text.push(c);
        if counts_cell {
            self.cell_count += 1;
        }
        self.style.len += c.len_utf8();
    }

    fn paint(
        &self,
        origin: GpuiPoint<Pixels>,
        dimensions: &TerminalBounds,
        window: &mut Window,
        cx: &mut App,
    ) {
        let pos = GpuiPoint::new(
            origin.x + self.start_point.column as f32 * dimensions.cell_width,
            origin.y + self.start_point.line as f32 * dimensions.line_height,
        );

        window
            .text_system()
            .shape_line(
                self.text.clone().into(),
                self.font_size,
                std::slice::from_ref(&self.style),
                Some(dimensions.cell_width),
            )
            .paint(
                pos,
                dimensions.line_height,
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            )
            .log_err();
    }
}

/// Block element glyphs are painted on a subcell grid: each terminal cell is
/// divided into 8 columns (for eighth blocks) and 24 lines (LCM of the 8-way
/// splits of eighth blocks and the 3-way splits of sextants).
const BLOCK_SUBCELL_COLUMNS: i32 = 8;
const BLOCK_SUBCELL_LINES: i32 = 24;

#[derive(Clone, Debug)]
struct BlockElementLayoutRect {
    point: LayoutPoint,
    num_of_columns: usize,
    num_of_lines: usize,
    color: Hsla,
}

impl BlockElementLayoutRect {
    fn paint(&self, origin: GpuiPoint<Pixels>, dimensions: &TerminalBounds, window: &mut Window) {
        let subcell_width = dimensions.cell_width / BLOCK_SUBCELL_COLUMNS as f32;
        let subcell_height = dimensions.line_height / BLOCK_SUBCELL_LINES as f32;
        let position = point(
            origin.x + self.point.column as f32 * subcell_width,
            origin.y + self.point.line as f32 * subcell_height,
        );
        let size = size(
            subcell_width * self.num_of_columns as f32,
            subcell_height * self.num_of_lines as f32,
        );

        window.paint_quad(fill(Bounds::new(position, size), self.color));
    }
}

#[derive(Clone, Debug, Default)]
struct LayoutRect {
    point: LayoutPoint,
    num_of_cells: usize,
    color: Hsla,
}

impl LayoutRect {
    fn paint(&self, origin: GpuiPoint<Pixels>, dimensions: &TerminalBounds, window: &mut Window) {
        let position = point(
            (origin.x + self.point.column as f32 * dimensions.cell_width).floor(),
            origin.y + self.point.line as f32 * dimensions.line_height,
        );
        let size = point(
            (dimensions.cell_width * self.num_of_cells as f32).ceil(),
            dimensions.line_height,
        )
        .into();

        window.paint_quad(fill(Bounds::new(position, size), self.color));
    }
}

/// A rectangular region with a specific color on a logical grid.
#[derive(Debug, Clone)]
struct BackgroundRegion {
    start_line: i32,
    start_col: i32,
    end_line: i32,
    end_col: i32,
    color: Hsla,
}

impl BackgroundRegion {
    fn new(line: i32, col: i32, color: Hsla) -> Self {
        Self::with_extents(line, col, line, col, color)
    }

    fn with_extents(
        start_line: i32,
        start_col: i32,
        end_line: i32,
        end_col: i32,
        color: Hsla,
    ) -> Self {
        BackgroundRegion {
            start_line,
            start_col,
            end_line,
            end_col,
            color,
        }
    }

    fn can_merge_with(&self, other: &BackgroundRegion) -> bool {
        if self.color != other.color {
            return false;
        }
        if self.start_line == other.start_line && self.end_line == other.end_line {
            return self.end_col + 1 == other.start_col || other.end_col + 1 == self.start_col;
        }
        if self.start_col == other.start_col && self.end_col == other.end_col {
            return self.end_line + 1 == other.start_line || other.end_line + 1 == self.start_line;
        }
        false
    }

    fn merge_with(&mut self, other: &BackgroundRegion) {
        self.start_line = self.start_line.min(other.start_line);
        self.start_col = self.start_col.min(other.start_col);
        self.end_line = self.end_line.max(other.end_line);
        self.end_col = self.end_col.max(other.end_col);
    }
}

/// Merge grid regions to minimize the number of rectangles.
fn merge_background_regions(regions: Vec<BackgroundRegion>) -> Vec<BackgroundRegion> {
    let mut merged = regions;
    let mut changed = true;
    while changed {
        changed = false;
        let mut i = 0;
        while i < merged.len() {
            let mut j = i + 1;
            while j < merged.len() {
                if merged[i].can_merge_with(&merged[j]) {
                    let other = merged.remove(j);
                    merged[i].merge_with(&other);
                    changed = true;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
    }
    merged
}

/// How the cursor is painted: a block with the character under it, a bar, an
/// underline, or a hollow box when the terminal is not focused.
#[derive(Clone, Copy)]
enum CursorPaint {
    Block,
    Bar,
    Underline,
    Hollow,
}

struct CursorLayout {
    bounds: Bounds<Pixels>,
    shape: CursorPaint,
    color: Hsla,
    text: Option<ShapedLine>,
}

impl CursorLayout {
    fn paint(&self, origin: GpuiPoint<Pixels>, window: &mut Window, cx: &mut App) {
        let bounds = self.bounds + origin;
        let thickness = px(2.);
        match self.shape {
            CursorPaint::Block => {
                window.paint_quad(fill(bounds, self.color));
                if let Some(text) = &self.text {
                    text.paint(
                        bounds.origin,
                        bounds.size.height,
                        gpui::TextAlign::Left,
                        None,
                        window,
                        cx,
                    )
                    .log_err();
                }
            }
            CursorPaint::Bar => {
                window.paint_quad(fill(
                    Bounds::new(bounds.origin, size(thickness, bounds.size.height)),
                    self.color,
                ));
            }
            CursorPaint::Underline => {
                window.paint_quad(fill(
                    Bounds::new(
                        point(bounds.origin.x, bounds.bottom() - thickness),
                        size(bounds.size.width, thickness),
                    ),
                    self.color,
                ));
            }
            CursorPaint::Hollow => {
                window.paint_quad(outline(bounds, self.color, BorderStyle::Solid));
            }
        }
    }
}

/// The GPUI element that paints the terminal.
pub struct TerminalElement {
    terminal: Entity<Terminal>,
    view: Entity<TerminalView>,
    focus: FocusHandle,
    focused: bool,
    cursor_visible: bool,
    interactivity: Interactivity,
}

impl InteractiveElement for TerminalElement {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

impl StatefulInteractiveElement for TerminalElement {}

impl TerminalElement {
    pub fn new(
        terminal: Entity<Terminal>,
        view: Entity<TerminalView>,
        focus: FocusHandle,
        focused: bool,
        cursor_visible: bool,
    ) -> TerminalElement {
        TerminalElement {
            terminal,
            view,
            focused,
            focus: focus.clone(),
            cursor_visible,
            interactivity: Default::default(),
        }
        .track_focus(&focus)
    }

    fn layout_grid<'a>(
        grid: impl Iterator<Item = &'a IndexedCell>,
        start_line_offset: i32,
        text_style: &TextStyle,
        font_size: Pixels,
        hyperlink: Option<(HighlightStyle, &Range)>,
        minimum_contrast: f32,
        cx: &App,
    ) -> (
        Vec<LayoutRect>,
        Vec<BatchedTextRun>,
        Vec<BlockElementLayoutRect>,
    ) {
        let theme = cx.theme();
        let mut batched_runs = Vec::new();
        let mut block_element_regions = Vec::new();
        let mut background_regions: Vec<BackgroundRegion> = Vec::new();
        let mut current_batch: Option<BatchedTextRun> = None;

        let linegroups = grid.chunk_by(|cell| cell.point.line);
        for (line_index, (_, line)) in linegroups.into_iter().enumerate() {
            let display_line = start_line_offset + line_index as i32;

            if let Some(batch) = current_batch.take() {
                batched_runs.push(batch);
            }

            let mut previous_cell_had_extras = false;

            for indexed in line {
                let point = indexed.point;
                let cell = &indexed.cell;
                let mut fg = cell.foreground();
                let mut bg = cell.background();
                if cell.is_inverse() {
                    mem::swap(&mut fg, &mut bg);
                }

                if !is_default_background_color(bg) {
                    let color = convert_color(&bg, theme);
                    let col = point.column as i32;
                    if let Some(last_region) = background_regions.last_mut()
                        && last_region.color == color
                        && last_region.start_line == display_line
                        && last_region.end_line == display_line
                        && last_region.end_col + 1 == col
                    {
                        last_region.end_col = col;
                    } else {
                        background_regions.push(BackgroundRegion::new(display_line, col, color));
                    }
                }
                // The second cell of a wide character is a placeholder.
                if cell.is_wide_char_spacer() {
                    continue;
                }
                // Skip spaces that follow cells with extras (emoji variation sequences).
                if cell.character() == ' ' && previous_cell_had_extras {
                    previous_cell_had_extras = false;
                    continue;
                }
                previous_cell_had_extras =
                    matches!(cell.zerowidth(), Some(chars) if !chars.is_empty());

                if is_blank(cell) {
                    continue;
                }
                let cell_style = Self::cell_style(
                    point,
                    cell,
                    fg,
                    bg,
                    theme,
                    text_style,
                    hyperlink,
                    minimum_contrast,
                );

                let cell_point = LayoutPoint::new(display_line, point.column as i32);
                if collect_block_element_regions(
                    cell_point,
                    cell.character(),
                    cell_style.color,
                    &mut block_element_regions,
                ) {
                    if let Some(batch) = current_batch.take() {
                        batched_runs.push(batch);
                    }
                    continue;
                }

                let zero_width_chars = cell.zerowidth();
                if let Some(batch) = current_batch.as_mut()
                    && batch.can_append(&cell_style)
                    && batch.start_point.line == cell_point.line
                    && batch.start_point.column + batch.cell_count as i32 == cell_point.column
                {
                    batch.append_char(cell.character());
                    if let Some(chars) = zero_width_chars {
                        batch.append_zero_width_chars(chars);
                    }
                } else {
                    if let Some(batch) = current_batch.take() {
                        batched_runs.push(batch);
                    }
                    let mut new_batch = BatchedTextRun::new_from_char(
                        cell_point,
                        cell.character(),
                        cell_style,
                        font_size,
                    );
                    if let Some(chars) = zero_width_chars {
                        new_batch.append_zero_width_chars(chars);
                    }
                    current_batch = Some(new_batch);
                }
            }
        }

        if let Some(batch) = current_batch {
            batched_runs.push(batch);
        }

        // Multi-line regions become one rect per line.
        let mut rects = Vec::new();
        for region in merge_background_regions(background_regions) {
            for line in region.start_line..=region.end_line {
                rects.push(LayoutRect {
                    point: LayoutPoint::new(line, region.start_col),
                    num_of_cells: (region.end_col - region.start_col + 1) as usize,
                    color: region.color,
                });
            }
        }

        let block_element_rects = merge_background_regions(block_element_regions)
            .into_iter()
            .map(|region| BlockElementLayoutRect {
                point: LayoutPoint::new(region.start_line, region.start_col),
                num_of_columns: (region.end_col - region.start_col + 1) as usize,
                num_of_lines: (region.end_line - region.start_line + 1) as usize,
                color: region.color,
            })
            .collect();

        (rects, batched_runs, block_element_rects)
    }

    /// Computes the cursor position based on the cursor point and terminal dimensions.
    fn cursor_position(
        cursor_point: DisplayCursor,
        size: TerminalBounds,
    ) -> Option<GpuiPoint<Pixels>> {
        if cursor_point.line < size.num_lines() as i32 {
            // When on pixel boundaries round the origin down.
            Some(point(
                (cursor_point.col as f32 * size.cell_width()).floor(),
                (cursor_point.line as f32 * size.line_height()).floor(),
            ))
        } else {
            None
        }
    }

    /// Decorative block and box characters keep their exact colors, so they
    /// join up with adjacent backgrounds; see Zed issue 34234.
    fn is_decorative_character(ch: char) -> bool {
        matches!(
            ch as u32,
            0x2500..=0x257F // Box Drawing
            | 0x2580..=0x259F // Block Elements
            | 0x25A0..=0x25FF // Geometric Shapes
            | 0x1FB00..=0x1FB3B // Sextants used by terminal QR renderers
            | 0xE0B0..=0xE0B7 // Powerline separators
            | 0xE0B8..=0xE0BF
            | 0xE0C0..=0xE0CA
            | 0xE0CC..=0xE0D1
            | 0xE0D2..=0xE0D7
        )
    }

    /// Converts the Alacritty cell styles to GPUI text styles and background color.
    #[allow(clippy::too_many_arguments)]
    fn cell_style(
        point: Point,
        cell: &Cell,
        fg: Color,
        bg: Color,
        colors: &Theme,
        text_style: &TextStyle,
        hyperlink: Option<(HighlightStyle, &Range)>,
        minimum_contrast: f32,
    ) -> TextRun {
        let skip_contrast = terminal_is_app_chosen_exact_color(fg);
        let mut fg = convert_color(&fg, colors);
        let bg = convert_color(&bg, colors);

        if !skip_contrast && !Self::is_decorative_character(cell.character()) {
            fg = ensure_minimum_contrast(fg, bg, minimum_contrast);
        }

        if cell.is_dim() {
            fg.a *= 0.7;
        }

        let underline =
            (cell.has_underline() || cell.hyperlink().is_some()).then(|| UnderlineStyle {
                color: Some(fg),
                thickness: Pixels::from(1.0),
                wavy: cell.has_undercurl(),
            });

        let strikethrough = cell.has_strikeout().then(|| StrikethroughStyle {
            color: Some(fg),
            thickness: Pixels::from(1.0),
        });

        let weight = if cell.is_bold() {
            FontWeight::BOLD
        } else {
            text_style.font_weight
        };

        let style = if cell.is_italic() {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        };

        let mut result = TextRun {
            len: cell.character().len_utf8(),
            color: fg,
            background_color: None,
            font: Font {
                weight,
                style,
                ..text_style.font()
            },
            underline,
            strikethrough,
        };

        if let Some((style, range)) = hyperlink
            && range.contains(point)
        {
            if let Some(underline) = style.underline {
                result.underline = Some(underline);
            }
            if let Some(color) = style.color {
                result.color = color;
            }
        }

        result
    }

    fn register_mouse_listeners(
        &mut self,
        hitbox: &Hitbox,
        mouse_input_mode: MouseInputMode,
        window: &mut Window,
    ) {
        let focus = self.focus.clone();
        let terminal = self.terminal.clone();

        self.interactivity.on_mouse_down(MouseButton::Left, {
            let terminal = terminal.clone();
            let focus = focus.clone();
            move |e, window, cx| {
                window.focus(&focus, cx);
                terminal.update(cx, |terminal, cx| {
                    terminal.mouse_down(e, mouse_input_mode, cx);
                    cx.notify();
                })
            }
        });

        window.on_mouse_event({
            let terminal = self.terminal.clone();
            let hitbox = hitbox.clone();
            let focus = focus.clone();
            move |e: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }

                if e.pressed_button.is_some() && !cx.has_active_drag() && focus.is_focused(window) {
                    let hovered = hitbox.is_hovered(window);
                    terminal.update(cx, |terminal, cx| {
                        if terminal.selection_started() || hovered {
                            terminal.mouse_drag(e, hitbox.bounds, mouse_input_mode, cx);
                            cx.notify();
                        }
                    })
                }

                if hitbox.is_hovered(window) {
                    terminal.update(cx, |terminal, cx| {
                        terminal.mouse_move(e, mouse_input_mode, cx);
                    })
                }
            }
        });

        for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
            self.interactivity.on_mouse_up(button, {
                let terminal = terminal.clone();
                let focus = focus.clone();
                move |event, window, cx| {
                    if !focus.is_focused(window) {
                        return;
                    }
                    if button != MouseButton::Left
                        && (mouse_input_mode == MouseInputMode::LocalSelection
                            || !terminal
                                .read(cx)
                                .last_content
                                .mode
                                .intersects(Modes::MOUSE_MODE))
                    {
                        return;
                    }
                    terminal.update(cx, |terminal, cx| {
                        terminal.mouse_up(event, mouse_input_mode, cx);
                        cx.notify();
                    });
                }
            });
        }
        for button in [MouseButton::Middle, MouseButton::Right] {
            self.interactivity.on_mouse_down(button, {
                let terminal = terminal.clone();
                let focus = focus.clone();
                move |event, window, cx| {
                    if mouse_input_mode == MouseInputMode::LocalSelection
                        || (button == MouseButton::Right
                            && !terminal
                                .read(cx)
                                .last_content
                                .mode
                                .intersects(Modes::MOUSE_MODE))
                    {
                        return;
                    }
                    window.focus(&focus, cx);
                    terminal.update(cx, |terminal, cx| {
                        terminal.mouse_down(event, mouse_input_mode, cx);
                        cx.notify();
                    });
                }
            });
        }

        self.interactivity.on_scroll_wheel({
            let view = self.view.downgrade();
            move |e, _, cx| {
                view.update(cx, |view, cx| {
                    view.scroll_wheel(e, cx);
                    cx.notify();
                })
                .ok();
            }
        });
    }
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = LayoutState;

    fn id(&self) -> Option<ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |mut style, window, cx| {
                style.size.width = relative(1.).into();
                style.size.height = relative(1.).into();
                window.request_layout(style, None, cx)
            },
        );
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            bounds.size,
            window,
            cx,
            |_, _, hitbox, window, cx| {
                let hitbox = hitbox.unwrap();
                let style = cx.global::<TerminalStyle>().clone();
                let minimum_contrast = TerminalSettings::get_global(cx).minimum_contrast;
                let theme = cx.theme().clone();

                let link_style = HighlightStyle {
                    color: Some(style.link),
                    underline: Some(UnderlineStyle {
                        thickness: px(1.0),
                        color: Some(style.link),
                        wavy: false,
                    }),
                    ..Default::default()
                };

                let text_style = TextStyle {
                    font_family: style.font_family.clone(),
                    font_features: FontFeatures::disable_ligatures(),
                    font_weight: FontWeight::NORMAL,
                    font_size: style.font_size.into(),
                    font_style: FontStyle::Normal,
                    line_height: style.line_height.into(),
                    background_color: Some(theme.colors().terminal_ansi_background),
                    white_space: WhiteSpace::Normal,
                    // Overridden per cell.
                    color: theme.colors().terminal_foreground,
                    ..Default::default()
                };

                let dimensions = {
                    let font_id = cx.text_system().resolve_font(&text_style.font());
                    let cell_width = cx
                        .text_system()
                        .advance(font_id, style.font_size, 'm')
                        .map(|advance| advance.width)
                        .unwrap_or(style.font_size * 0.6);

                    let mut size = bounds.size;
                    // A one-column terminal makes alacritty misbehave on wide
                    // characters (Zed issue 2750).
                    if size.width < cell_width * 2.0 {
                        size.width = cell_width * 2.0;
                    }

                    // Whole rows only; the spare height goes above the rows while
                    // the output sits at the bottom, as in a native terminal.
                    let mut origin = bounds.origin;
                    let anchor_to_bottom = {
                        let content = self.terminal.read(cx).last_content();
                        content.mode.contains(Modes::ALT_SCREEN)
                            || (content.scrolled_to_bottom && content.bottom_row_occupied)
                    };
                    let scale_factor = window.scale_factor().max(1.0);
                    let line_height_device_px = (f32::from(style.line_height) * scale_factor)
                        .round()
                        .max(1.0) as i32;
                    let available_height_device_px =
                        (f32::from(size.height) * scale_factor).floor().max(0.0) as i32;
                    let rows = (available_height_device_px / line_height_device_px).max(1);
                    let snapped_height_device_px = rows * line_height_device_px;
                    let padding_device_px =
                        (available_height_device_px - snapped_height_device_px).max(0);
                    size.height = px(snapped_height_device_px as f32 / scale_factor);
                    if anchor_to_bottom {
                        origin.y += px(padding_device_px as f32 / scale_factor);
                    }

                    // Snap to device pixels, so glyphs do not shift while resizing.
                    let snap = |value: Pixels| {
                        Pixels::from((f32::from(value) * scale_factor).floor() / scale_factor)
                    };
                    origin.x = snap(origin.x);
                    origin.y = snap(origin.y);

                    TerminalBounds::new(style.line_height, cell_width, Bounds { origin, size })
                };

                let background_color = theme.colors().terminal_background;

                let hover_match = self.terminal.update(cx, |terminal, cx| {
                    terminal.set_size(dimensions);
                    terminal.sync(window, cx);

                    let hovered_link = self.view.read(cx).hovered_link.as_ref();
                    if window.modifiers().secondary()
                        && bounds.contains(&window.mouse_position())
                        && let Some(hovered) = terminal.last_content.last_hovered_word.as_ref()
                        && hovered_link.is_some_and(|link| link.id == hovered.id)
                    {
                        Some(hovered.word_match)
                    } else {
                        None
                    }
                });

                let Content {
                    cells,
                    display_offset,
                    cursor_char,
                    selection,
                    cursor,
                    ..
                } = &self.terminal.read(cx).last_content;
                let display_offset = *display_offset;
                let selection = selection.map(|selection| selection.point_range());

                // Only cells inside the visible part of the element, which a
                // parent can clip while the drawer is resized.
                let content_bounds = dimensions.bounds;
                let intersection = window.content_mask().bounds.intersect(&content_bounds);
                let hyperlink = hover_match.as_ref().map(|range| (link_style, range));
                let (rects, batched_text_runs, block_element_rects) =
                    if intersection.size.height <= px(0.) || intersection.size.width <= px(0.) {
                        (Vec::new(), Vec::new(), Vec::new())
                    } else if intersection == content_bounds {
                        TerminalElement::layout_grid(
                            cells.iter(),
                            0,
                            &text_style,
                            style.font_size,
                            hyperlink,
                            minimum_contrast,
                            cx,
                        )
                    } else {
                        let rows_above_viewport =
                            ((intersection.top() - content_bounds.top()).max(px(0.))
                                / style.line_height) as usize;
                        let visible_row_count =
                            (intersection.size.height / style.line_height).ceil() as usize + 1;
                        TerminalElement::layout_grid(
                            cells
                                .iter()
                                .chunk_by(|c| c.point.line)
                                .into_iter()
                                .skip(rows_above_viewport)
                                .take(visible_row_count)
                                .flat_map(|(_, line_cells)| line_cells),
                            rows_above_viewport as i32,
                            &text_style,
                            style.font_size,
                            hyperlink,
                            minimum_contrast,
                            cx,
                        )
                    };

                // The cursor rectangle positions the IME window, so lay it out
                // even when it is not painted.
                let cursor_point = DisplayCursor::from(cursor.point, display_offset);
                let cursor_text = {
                    let text = cursor_char.to_string();
                    let len = text.len();
                    window.text_system().shape_line(
                        text.into(),
                        style.font_size,
                        &[TextRun {
                            len,
                            font: text_style.font(),
                            color: theme.colors().terminal_ansi_background,
                            ..Default::default()
                        }],
                        None,
                    )
                };

                // Whitespace uses the cell width; wide characters such as emoji
                // take the larger of their shaped width and the cell width.
                let cursor_width = if cursor_char.is_whitespace() {
                    dimensions.cell_width()
                } else {
                    cursor_text.width.max(dimensions.cell_width())
                };

                let ime_cursor_bounds = TerminalElement::cursor_position(cursor_point, dimensions)
                    .map(|cursor_position| Bounds {
                        origin: cursor_position,
                        size: size(cursor_width.ceil(), dimensions.line_height),
                    });

                let cursor = match cursor.shape {
                    CursorShape::Hidden => None,
                    shape => ime_cursor_bounds.map(|bounds| {
                        let (shape, text) = match shape {
                            _ if !self.focused => (CursorPaint::Hollow, None),
                            CursorShape::Block => (CursorPaint::Block, Some(cursor_text)),
                            CursorShape::Underline => (CursorPaint::Underline, None),
                            CursorShape::Bar => (CursorPaint::Bar, None),
                            CursorShape::HollowBlock | CursorShape::Hidden => {
                                (CursorPaint::Hollow, None)
                            }
                        };
                        CursorLayout {
                            bounds,
                            shape,
                            color: style.cursor,
                            text,
                        }
                    }),
                };

                LayoutState {
                    hitbox,
                    batched_text_runs,
                    block_element_rects,
                    cursor,
                    ime_cursor_bounds,
                    background_color,
                    dimensions,
                    rects,
                    selection,
                    display_offset,
                    base_text_style: text_style,
                }
            },
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        layout: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_quad(fill(bounds, layout.background_color));

            let scale_factor = window.scale_factor();
            let snap = |value: Pixels| {
                Pixels::from((f32::from(value) * scale_factor).floor() / scale_factor)
            };
            let origin = layout.dimensions.bounds.origin;
            let origin = point(snap(origin.x), snap(origin.y));

            let view = self.view.read(cx);
            let mouse_input_mode = view.mouse_input_mode();
            let marked_text = view.marked_text.clone();
            let hovering_link = view.hovered_link.is_some();
            let selection_color = cx.global::<TerminalStyle>().selection;

            let input_handler = TerminalInputHandler {
                view: self.view.clone(),
                cursor_bounds: layout.ime_cursor_bounds.map(|bounds| bounds + origin),
            };

            self.register_mouse_listeners(&layout.hitbox, mouse_input_mode, window);
            if window.modifiers().secondary()
                && bounds.contains(&window.mouse_position())
                && hovering_link
            {
                window.set_cursor_style(gpui::CursorStyle::PointingHand, &layout.hitbox);
            } else {
                window.set_cursor_style(gpui::CursorStyle::IBeam, &layout.hitbox);
            }

            let cursor = layout.cursor.take();
            self.interactivity.paint(
                global_id,
                inspector_id,
                bounds,
                Some(&layout.hitbox),
                window,
                cx,
                |_, window, cx| {
                    window.handle_input(&self.focus, input_handler, cx);

                    window.on_key_event({
                        let terminal = self.terminal.clone();
                        move |event: &ModifiersChangedEvent, phase, window, cx| {
                            if phase != DispatchPhase::Bubble {
                                return;
                            }
                            terminal.update(cx, |terminal, cx| {
                                terminal.try_modifiers_change(&event.modifiers, window, cx)
                            });
                        }
                    });

                    for rect in &layout.rects {
                        rect.paint(origin, &layout.dimensions, window);
                    }

                    if let Some(selection) = &layout.selection {
                        for line in selection_lines(selection, layout, origin) {
                            window.paint_quad(fill(line, selection_color));
                        }
                    }

                    for batch in &layout.batched_text_runs {
                        batch.paint(origin, &layout.dimensions, window, cx);
                    }
                    for block_element_rect in &layout.block_element_rects {
                        block_element_rect.paint(origin, &layout.dimensions, window);
                    }

                    if let Some(marked_text) = marked_text.as_deref()
                        && !marked_text.is_empty()
                        && let Some(ime_bounds) = layout.ime_cursor_bounds
                    {
                        let position = (ime_bounds + origin).origin;
                        let style = &layout.base_text_style;
                        let underline = Some(UnderlineStyle {
                            color: Some(style.color),
                            thickness: px(1.0),
                            wavy: false,
                        });
                        let shaped = window.text_system().shape_line(
                            marked_text.to_owned().into(),
                            style.font_size.to_pixels(window.rem_size()),
                            &[TextRun {
                                len: marked_text.len(),
                                font: style.font(),
                                color: style.color,
                                underline,
                                ..Default::default()
                            }],
                            None,
                        );
                        // Cover the terminal text behind the marked text.
                        window.paint_quad(fill(
                            Bounds::new(
                                position,
                                size(shaped.width, layout.dimensions.line_height),
                            ),
                            layout.background_color,
                        ));
                        shaped
                            .paint(
                                position,
                                layout.dimensions.line_height,
                                gpui::TextAlign::Left,
                                None,
                                window,
                                cx,
                            )
                            .log_err();
                    }

                    if self.cursor_visible
                        && marked_text.is_none()
                        && let Some(cursor) = cursor
                    {
                        cursor.paint(origin, window, cx);
                    }
                },
            );
        });
    }
}

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

struct TerminalInputHandler {
    view: Entity<TerminalView>,
    cursor_bounds: Option<Bounds<Pixels>>,
}

impl InputHandler for TerminalInputHandler {
    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut App,
    ) -> Option<UTF16Selection> {
        // Always a selection, so the IME window follows the cursor even in
        // full-screen programs.
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &mut self,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<std::ops::Range<usize>> {
        self.view
            .read(cx)
            .marked_text
            .as_ref()
            .map(|text| 0..text.encode_utf16().count())
    }

    fn text_for_range(
        &mut self,
        _: std::ops::Range<usize>,
        _: &mut Option<std::ops::Range<usize>>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<String> {
        None
    }

    fn replace_text_in_range(
        &mut self,
        _replacement_range: Option<std::ops::Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.view.update(cx, |view, cx| {
            view.set_marked_text(None, cx);
            view.commit_text(text, cx);
        });
        window.invalidate_character_coordinates();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range_utf16: Option<std::ops::Range<usize>>,
        new_text: &str,
        _new_marked_range: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        cx: &mut App,
    ) {
        self.view.update(cx, |view, cx| {
            view.set_marked_text(Some(new_text.to_owned()), cx)
        });
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut App) {
        self.view
            .update(cx, |view, cx| view.set_marked_text(None, cx));
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: std::ops::Range<usize>,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        let cell_width = self
            .view
            .read(cx)
            .terminal
            .read(cx)
            .last_content()
            .terminal_bounds
            .cell_width;
        let mut bounds = self.cursor_bounds?;
        bounds.origin.x += cell_width * range_utf16.start as f32;
        Some(bounds)
    }

    fn apple_press_and_hold_enabled(&mut self) -> bool {
        false
    }

    fn character_index_for_point(
        &mut self,
        _point: GpuiPoint<Pixels>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<usize> {
        None
    }
}

fn is_blank(cell: &Cell) -> bool {
    cell.character() == ' '
        && is_default_background_color(cell.background())
        && cell.hyperlink().is_none()
        && !cell.has_visible_style_modifier()
}

/// Returns `(column, line, num_of_columns, num_of_lines)` in subcell units for
/// block element characters that consist of a single rectangle.
fn block_char_to_rect(ch: char) -> Option<(i32, i32, i32, i32)> {
    let codepoint = ch as u32;
    Some(match codepoint {
        // ▀ upper half
        0x2580 => (0, 0, 8, 12),
        // ▁▂▃▄▅▆▇█ lower blocks of 1..=8 eighths
        0x2581..=0x2588 => {
            let eighths = (codepoint - 0x2580) as i32;
            (0, 24 - eighths * 3, 8, eighths * 3)
        }
        // ▉▊▋▌▍▎▏ left blocks of 7..=1 eighths
        0x2589..=0x258F => (0, 0, (0x2590 - codepoint) as i32, 24),
        // ▐ right half
        0x2590 => (4, 0, 4, 24),
        // ▔ upper eighth
        0x2594 => (0, 0, 8, 3),
        // ▕ right eighth
        0x2595 => (7, 0, 1, 24),
        _ => return None,
    })
}

/// The filled quadrants of a quadrant character, bit `row * 2 + column`.
fn quadrant_char_to_filled_bits(ch: char) -> Option<u8> {
    Some(match ch {
        '▘' => 0b0001,
        '▝' => 0b0010,
        '▖' => 0b0100,
        '▗' => 0b1000,
        '▚' => 0b1001,
        '▞' => 0b0110,
        '▛' => 0b0111,
        '▜' => 0b1011,
        '▙' => 0b1101,
        '▟' => 0b1110,
        _ => return None,
    })
}

/// The filled subcells of a sextant character, bit `row * 2 + column`.
///
/// U+1FB00..=U+1FB3B enumerate all 2x3 fill combinations except the four that
/// already exist as Block Elements, hence the gap adjustments.
fn sextant_char_to_filled_bits(ch: char) -> Option<u8> {
    let offset = (ch as u32).checked_sub(0x1FB00)?;
    if offset > 0x3B {
        return None;
    }
    Some((offset + 1 + u32::from(offset >= 20) + u32::from(offset >= 40)) as u8)
}

/// Shade characters `░▒▓` as the foreground at reduced opacity, rather than
/// the stipple patterns fonts use, so the cells join seamlessly.
fn shade_char_to_opacity(ch: char) -> Option<f32> {
    match ch {
        '░' => Some(0.25),
        '▒' => Some(0.5),
        '▓' => Some(0.75),
        _ => None,
    }
}

/// Block element glyphs are painted as rectangles, so adjacent cells join
/// without the gaps font glyphs leave. Returns whether `ch` was one.
fn collect_block_element_regions(
    point: LayoutPoint,
    ch: char,
    color: Hsla,
    regions: &mut Vec<BackgroundRegion>,
) -> bool {
    if let Some((column, line, columns, lines)) = block_char_to_rect(ch) {
        push_block_element_region(point, column, line, columns, lines, color, regions);
        return true;
    }
    if let Some(filled) = quadrant_char_to_filled_bits(ch) {
        for row in 0..2 {
            for column in 0..2 {
                if filled & (1 << (row * 2 + column)) != 0 {
                    push_block_element_region(point, column * 4, row * 12, 4, 12, color, regions);
                }
            }
        }
        return true;
    }
    if let Some(filled) = sextant_char_to_filled_bits(ch) {
        for row in 0..3 {
            for column in 0..2 {
                if filled & (1 << (row * 2 + column)) != 0 {
                    push_block_element_region(point, column * 4, row * 8, 4, 8, color, regions);
                }
            }
        }
        return true;
    }
    if let Some(opacity) = shade_char_to_opacity(ch) {
        push_block_element_region(point, 0, 0, 8, 24, color.opacity(opacity), regions);
        return true;
    }
    false
}

fn push_block_element_region(
    point: LayoutPoint,
    column: i32,
    line: i32,
    num_of_columns: i32,
    num_of_lines: i32,
    color: Hsla,
    regions: &mut Vec<BackgroundRegion>,
) {
    let start_line = point.line * BLOCK_SUBCELL_LINES + line;
    let start_col = point.column * BLOCK_SUBCELL_COLUMNS + column;
    let end_line = start_line + num_of_lines - 1;
    let end_col = start_col + num_of_columns - 1;

    // Extend the previous region when possible (runs of `█` in a QR code),
    // which keeps the quadratic merge pass small.
    if let Some(last_region) = regions.last_mut()
        && last_region.color == color
        && last_region.start_line == start_line
        && last_region.end_line == end_line
        && last_region.end_col + 1 == start_col
    {
        last_region.end_col = end_col;
        return;
    }

    regions.push(BackgroundRegion::with_extents(
        start_line, start_col, end_line, end_col, color,
    ));
}

/// The selection as one rectangle per visible line.
fn selection_lines(
    range: &Range,
    layout: &LayoutState,
    origin: GpuiPoint<Pixels>,
) -> Vec<Bounds<Pixels>> {
    // Terminal lines are negative above the viewport top when scrolled back;
    // shift them by the display offset to get viewport rows.
    let display_offset = i32::try_from(layout.display_offset).unwrap_or(i32::MAX);
    let start_line = range.start().line.saturating_add(display_offset);
    let end_line = range.end().line.saturating_add(display_offset);
    let lines = layout.dimensions.num_lines() as i32;
    if end_line < 0 || start_line > lines {
        return Vec::new();
    }
    let first = start_line.max(0);
    let last = end_line.min(lines);
    (first..=last)
        .map(|line| {
            let start = if line == start_line {
                range.start().column
            } else {
                0
            };
            let end = if line == end_line {
                range.end().column + 1
            } else {
                layout.dimensions.num_columns()
            };
            Bounds::new(
                point(
                    origin.x + start as f32 * layout.dimensions.cell_width,
                    origin.y + line as f32 * layout.dimensions.line_height,
                ),
                size(
                    layout.dimensions.cell_width * end.saturating_sub(start) as f32,
                    layout.dimensions.line_height,
                ),
            )
        })
        .collect()
}

/// Converts a 2, 8, or 24 bit ANSI color to the GPUI equivalent.
fn convert_color(fg: &Color, theme: &Theme) -> Hsla {
    let colors = theme.colors();
    match fg {
        Color::Named(color) => match color {
            NamedColor::Black => colors.terminal_ansi_black,
            NamedColor::Red => colors.terminal_ansi_red,
            NamedColor::Green => colors.terminal_ansi_green,
            NamedColor::Yellow => colors.terminal_ansi_yellow,
            NamedColor::Blue => colors.terminal_ansi_blue,
            NamedColor::Magenta => colors.terminal_ansi_magenta,
            NamedColor::Cyan => colors.terminal_ansi_cyan,
            NamedColor::White => colors.terminal_ansi_white,
            NamedColor::BrightBlack => colors.terminal_ansi_bright_black,
            NamedColor::BrightRed => colors.terminal_ansi_bright_red,
            NamedColor::BrightGreen => colors.terminal_ansi_bright_green,
            NamedColor::BrightYellow => colors.terminal_ansi_bright_yellow,
            NamedColor::BrightBlue => colors.terminal_ansi_bright_blue,
            NamedColor::BrightMagenta => colors.terminal_ansi_bright_magenta,
            NamedColor::BrightCyan => colors.terminal_ansi_bright_cyan,
            NamedColor::BrightWhite => colors.terminal_ansi_bright_white,
            NamedColor::Foreground => colors.terminal_foreground,
            NamedColor::Background => colors.terminal_ansi_background,
            NamedColor::Cursor => theme.players().local().cursor,
            NamedColor::DimBlack => colors.terminal_ansi_dim_black,
            NamedColor::DimRed => colors.terminal_ansi_dim_red,
            NamedColor::DimGreen => colors.terminal_ansi_dim_green,
            NamedColor::DimYellow => colors.terminal_ansi_dim_yellow,
            NamedColor::DimBlue => colors.terminal_ansi_dim_blue,
            NamedColor::DimMagenta => colors.terminal_ansi_dim_magenta,
            NamedColor::DimCyan => colors.terminal_ansi_dim_cyan,
            NamedColor::DimWhite => colors.terminal_ansi_dim_white,
            NamedColor::BrightForeground => colors.terminal_bright_foreground,
            NamedColor::DimForeground => colors.terminal_dim_foreground,
        },
        Color::Spec(rgb) => terminal::rgba_color(rgb.r, rgb.g, rgb.b),
        Color::Indexed(i) => terminal::get_color_at_index(*i as usize, theme),
    }
}
