//! Drawing the editor: gutter (line numbers, fold markers), syntax-coloured
//! text, selections, cursors, bracket pairs, fold placeholders and the
//! minimap. Also remembers the layout for mouse hit-testing and the IME.

use super::layout::{gutter_digits, scroll_to_show, DisplayLine};
use super::{Autoscroll, EditorView};
use crate::theme::{ForgeTheme, Theme, CODE_FONT};
use forge_text::commands::col_from_visual;
use forge_text::folding::{fold_end, is_foldable, FoldMap};
use forge_text::{Pos, TextBuffer, Token};
use gpui_kit::{
    fill, font, point, px, relative, size, App, Bounds, ElementId, ElementInputHandler, Entity, FontStyle, Hsla,
    IntoElement, LayoutId, PaintQuad, Pixels, Point, ShapedLine, SharedString, Style, TextAlign, TextRun, Window,
};
use std::sync::Arc;
use std::time::Duration;

/// Space between the gutter and the text.
const PAD_LEFT: f32 = 8.0;
const GUTTER_PAD: f32 = 10.0;
/// Room for the 💡 at the gutter's left edge (clicks there open Quick Fix).
pub(crate) const LIGHTBULB_WIDTH: f32 = 14.0;
const FOLD_MARK_W: f32 = 14.0;
pub const MINIMAP_WIDTH: f32 = 90.0;
/// Pixels per row in the minimap when the whole file fits.
const ROW_PX: f32 = 2.0;
const CHAR_PX: f32 = 1.0;

/// One drawn row.
#[derive(Clone)]
pub(crate) struct RowLayout {
    pub line: usize,
    pub y: f32,
    pub shaped: ShapedLine,
    pub display: DisplayLine,
    pub folded: bool,
}

#[derive(Clone)]
pub(crate) struct MinimapLayout {
    pub left: f32,
    pub top: f32,
    pub height: f32,
    /// Screen rows per minimap row (1 when the whole file fits).
    pub rows_per_sample: f32,
    pub row_px: f32,
}

/// The last drawn layout, for hit-testing.
#[derive(Clone)]
pub(crate) struct LayoutCache {
    pub bounds: Bounds<Pixels>,
    pub text_left: f32,
    pub gutter_right: f32,
    pub line_height: f32,
    pub char_width: f32,
    pub viewport_height: f32,
    pub max_scroll_top: f32,
    pub max_scroll_left: f32,
    pub scroll_top: f32,
    pub rows: Vec<RowLayout>,
    pub folds: FoldMap,
    pub line_count: usize,
    pub minimap: Option<MinimapLayout>,
}

/// What the mouse is over.
pub(crate) struct Hit {
    pub pos: Pos,
    pub in_gutter: bool,
    pub on_fold_marker: bool,
    pub on_fold_placeholder: bool,
}

impl LayoutCache {
    pub fn hit_test(&self, p: Point<Pixels>, buffer: &TextBuffer, tab_size: usize) -> Hit {
        let x = f32::from(p.x);
        let y = f32::from(p.y) - f32::from(self.bounds.top()) + self.scroll_top;
        let rows = self.folds.visible_count(self.line_count);
        let in_gutter = x < self.gutter_right;
        let on_fold_marker = in_gutter && x >= self.gutter_right - FOLD_MARK_W - 2.0;
        if y < 0.0 {
            return Hit { pos: Pos::new(0, 0), in_gutter, on_fold_marker: false, on_fold_placeholder: false };
        }
        let row = (y / self.line_height).floor() as usize;
        if row >= rows {
            return Hit { pos: buffer.end(), in_gutter, on_fold_marker: false, on_fold_placeholder: false };
        }
        let line = self.folds.buffer_line(row);
        let rel = (x - self.text_left).max(0.0);
        let (col, end_x, folded) = match self.rows.iter().find(|r| r.line == line) {
            Some(r) => {
                let byte = r.shaped.closest_index_for_x(px(rel));
                (r.display.col_at_byte(byte), f32::from(r.shaped.width), r.folded)
            }
            None => {
                let visual = (rel / self.char_width).round() as usize;
                (col_from_visual(buffer.line(line), visual, tab_size), f32::MAX, false)
            }
        };
        let on_fold_placeholder = folded && rel > end_x + 2.0 && rel < end_x + 6.0 + self.char_width * 3.0;
        Hit { pos: Pos::new(line, col), in_gutter, on_fold_marker, on_fold_placeholder }
    }

    /// Screen bounds of columns `start..end` on `line` (for the IME candidate window).
    pub fn bounds_for_cols(&self, line: usize, start: usize, end: usize) -> Option<Bounds<Pixels>> {
        let r = self.rows.iter().find(|r| r.line == line)?;
        let x0 = self.text_left + f32::from(r.shaped.x_for_index(r.display.byte(start)));
        let x1 = self.text_left + f32::from(r.shaped.x_for_index(r.display.byte(end)));
        Some(Bounds::new(point(px(x0), px(r.y)), size(px((x1 - x0).max(1.0)), px(self.line_height))))
    }

    pub fn in_minimap(&self, p: Point<Pixels>) -> bool {
        self.minimap.as_ref().is_some_and(|m| f32::from(p.x) >= m.left)
    }

    /// The screen row the minimap shows at window y.
    pub fn minimap_row_at(&self, y: Pixels) -> usize {
        let Some(m) = &self.minimap else { return 0 };
        let i = ((f32::from(y) - m.top) / m.row_px).max(0.0);
        (i * m.rows_per_sample) as usize
    }
}

pub(crate) struct EditorElement {
    editor: Entity<EditorView>,
}

impl EditorElement {
    pub fn new(editor: Entity<EditorView>) -> Self {
        Self { editor }
    }
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

pub(crate) struct Prepaint {
    layout: Option<LayoutCache>,
    quads_back: Vec<PaintQuad>,
    quads_front: Vec<PaintQuad>,
    /// Wavy underlines under problems: start, width, colour.
    squiggles: Vec<(Point<Pixels>, Pixels, Hsla)>,
    gutter: Vec<(ShapedLine, Point<Pixels>)>,
    placeholders: Vec<(ShapedLine, Point<Pixels>, Bounds<Pixels>)>,
    minimap: Vec<PaintQuad>,
    message: Option<(ShapedLine, Point<Pixels>)>,
    text_clip: Bounds<Pixels>,
    gutter_clip: Bounds<Pixels>,
    line_height: Pixels,
}

fn code_font(style: FontStyle) -> gpui_kit::Font {
    let mut f = font(CODE_FONT);
    f.style = style;
    f
}

/// Text runs for a drawn line: token colours, the default colour elsewhere.
fn runs_for(display: &DisplayLine, tokens: &[Token], theme: &Theme) -> Vec<TextRun> {
    let plain = |len: usize| TextRun {
        len,
        font: code_font(FontStyle::Normal),
        color: theme.fg,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let mut runs = Vec::new();
    let mut at = 0;
    let mut sorted: Vec<&Token> = tokens.iter().collect();
    sorted.sort_by_key(|t| t.start);
    for t in sorted {
        let start = display.byte(t.start);
        let end = display.byte(t.end);
        if start < at || end <= start {
            continue;
        }
        if start > at {
            runs.push(plain(start - at));
        }
        let (color, weight, style) = theme.token_style(t.kind);
        let mut f = code_font(style);
        f.weight = weight;
        runs.push(TextRun {
            len: end - start,
            font: f,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        });
        at = end;
    }
    if display.text.len() > at {
        runs.push(plain(display.text.len() - at));
    }
    runs
}

impl gpui_kit::Element for EditorElement {
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
        _id: Option<&gpui_kit::GlobalElementId>,
        _inspector_id: Option<&gpui_kit::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui_kit::GlobalElementId>,
        _inspector_id: Option<&gpui_kit::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let theme: Arc<Theme> = cx.global::<ForgeTheme>().0.clone();
        let editor = self.editor.read(cx);
        let font_size = px(editor.font_size);
        let lh = editor.line_height;
        let line_height = px(lh);
        let tab = editor.tab_size;
        let show_minimap = editor.minimap && f32::from(bounds.size.width) > 400.0;
        let focused = editor.focus.is_focused(window);
        let marked = editor.marked;
        let autoscroll = editor.autoscroll;
        let view = editor.view();
        let document = editor.document.clone();
        let placeholder = editor.placeholder.clone();

        let text_system = window.text_system().clone();
        let shape =
            |text: String, runs: &[TextRun]| text_system.shape_line(SharedString::from(text), font_size, runs, None);
        let plain_run = |len: usize, color: Hsla| TextRun {
            len,
            font: code_font(FontStyle::Normal),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let char_width = f32::from(shape("0000000000".into(), &[plain_run(10, theme.fg)]).width) / 10.0;

        let empty = |message: Option<(ShapedLine, Point<Pixels>)>| Prepaint {
            layout: None,
            quads_back: Vec::new(),
            quads_front: Vec::new(),
            squiggles: Vec::new(),
            gutter: Vec::new(),
            placeholders: Vec::new(),
            minimap: Vec::new(),
            message,
            text_clip: bounds,
            gutter_clip: bounds,
            line_height,
        };
        let Some(document) = document else {
            let message = placeholder.map(|m| {
                let text = m.to_string();
                let len = text.len();
                let shaped = shape(text, &[plain_run(len, theme.fg_muted)]);
                let origin = point(bounds.center().x - shaped.width / 2.0, bounds.center().y - line_height / 2.0);
                (shaped, origin)
            });
            return empty(message);
        };

        // ----- what to draw -----
        let (line_count, folds, cursor_line_len) = {
            let doc = &document.read(cx).doc;
            let folds = if doc.folds.is_empty() {
                FoldMap::empty()
            } else {
                FoldMap::new(&doc.folds, |h| fold_end(doc.buffer.lines(), h))
            };
            (doc.buffer.line_count(), folds, doc.buffer.line_len(view.cursor.line))
        };
        let _ = cursor_line_len;
        let digits = gutter_digits(line_count);
        let gutter_width = digits as f32 * char_width + GUTTER_PAD * 2.0 + FOLD_MARK_W;
        let minimap_width = if show_minimap { MINIMAP_WIDTH } else { 0.0 };
        let left = f32::from(bounds.left());
        let top = f32::from(bounds.top());
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        let gutter_right = left + gutter_width;
        let text_area_left = gutter_right + PAD_LEFT;
        let text_area_width = (width - gutter_width - PAD_LEFT - minimap_width).max(char_width);
        let rows = folds.visible_count(line_count);
        let content_height = rows as f32 * lh;
        let max_scroll_top = (content_height - lh).max(0.0);

        // ----- keep the cursor in view -----
        let mut scroll_top = view.scroll_top.min(max_scroll_top);
        let mut scroll_left = view.scroll_left;
        if let Some(mode) = autoscroll {
            let row = folds.visual_line(view.cursor.line);
            scroll_top = scroll_to_show(scroll_top, row, lh, height, mode == Autoscroll::Center).min(max_scroll_top);
            let cursor_x = {
                let line = document.read(cx).doc.buffer.line(view.cursor.line).to_owned();
                let display = DisplayLine::new(&line, tab);
                let byte = display.byte(view.cursor.col);
                let len = display.text.len();
                f32::from(shape(display.text, &[plain_run(len, theme.fg)]).x_for_index(byte))
            };
            if cursor_x < scroll_left {
                scroll_left = (cursor_x - char_width * 4.0).max(0.0);
            } else if cursor_x > scroll_left + text_area_width - char_width * 2.0 {
                scroll_left = cursor_x - text_area_width + char_width * 4.0;
            }
        }
        self.editor.update(cx, |e, _| {
            e.autoscroll = None;
            if let Some(v) = e.view_mut() {
                v.scroll_top = scroll_top;
                v.scroll_left = scroll_left;
            }
        });
        let text_left = text_area_left - scroll_left;

        // ----- the visible rows, with their tokens -----
        let first_row = (scroll_top / lh).floor() as usize;
        let last_row = rows.min(((scroll_top + height) / lh).ceil() as usize + 1);
        let row_lines: Vec<usize> = (first_row..last_row).map(|r| folds.buffer_line(r)).collect();
        let lines_and_tokens: Vec<(usize, String, Vec<Token>, bool)> = document.update(cx, |d, _| {
            let doc = &mut d.doc;
            let mode = doc.mode();
            row_lines
                .iter()
                .map(|&line| {
                    let tokens = doc.highlighter.tokens_for(&doc.buffer, mode, line);
                    let foldable = is_foldable(doc.buffer.lines(), line);
                    (line, doc.buffer.line(line).to_owned(), tokens, foldable)
                })
                .collect()
        });
        let mut row_layouts = Vec::with_capacity(lines_and_tokens.len());
        let lightbulb = self.editor.read(cx).lightbulb;
        let mut gutter = Vec::new();
        let mut placeholders = Vec::new();
        for (i, (line, text, tokens, foldable)) in lines_and_tokens.iter().enumerate() {
            let y = top + (first_row + i) as f32 * lh - scroll_top;
            let display = DisplayLine::new(text, tab);
            let runs = runs_for(&display, tokens, &theme);
            let shaped = shape(display.text.clone(), &runs);
            let folded = folds.is_folded(*line);
            // Line number (current line brighter) and fold marker.
            let number = (line + 1).to_string();
            let color = if *line == view.cursor.line { theme.fg } else { theme.fg_muted };
            let num_shaped = shape(number.clone(), &[plain_run(number.len(), color)]);
            let num_x = gutter_right - FOLD_MARK_W - GUTTER_PAD - f32::from(num_shaped.width);
            gutter.push((num_shaped, point(px(num_x), px(y))));
            if let Some((_, enabled)) = self.editor.read(cx).debug_lines.iter().find(|(l, _)| l == line) {
                let mark = if *enabled { "●" } else { "○" };
                gutter.push((shape(mark.into(), &[plain_run(mark.len(), theme.err)]), point(px(left), px(y))));
            }
            if self.editor.read(cx).debug_stopped == Some(*line) {
                gutter.push((shape("▶".into(), &[plain_run("▶".len(), theme.warn)]), point(px(left + 10.0), px(y))));
            }
            if lightbulb == Some(*line) {
                let bulb = shape("💡".into(), &[plain_run("💡".len(), theme.fg)]);
                gutter.push((bulb, point(px(left), px(y))));
            }
            if *foldable || folded {
                let mark = if folded { "▸" } else { "▾" };
                let m = shape(mark.into(), &[plain_run(mark.len(), theme.fg_muted)]);
                gutter.push((m, point(px(gutter_right - FOLD_MARK_W), px(y))));
            }
            if folded {
                let ph = shape(" ⋯ ".into(), &[plain_run(" ⋯ ".len(), theme.fg_muted)]);
                let x = text_left + f32::from(shaped.width) + 4.0;
                let b = Bounds::new(point(px(x), px(y + 2.0)), size(ph.width, px(lh - 4.0)));
                placeholders.push((ph, point(px(x), px(y)), b));
            }
            row_layouts.push(RowLayout { line: *line, y, shaped, display, folded });
        }

        // ----- selections, current line, brackets, cursors -----
        let mut back = Vec::new();
        let mut front = Vec::new();
        let x_of = |r: &RowLayout, col: usize| text_left + f32::from(r.shaped.x_for_index(r.display.byte(col)));
        let mut selections: Vec<(Pos, Pos)> = vec![(view.anchor, view.cursor)];
        selections.extend(view.extras.iter().map(|s| (s.anchor, s.head)));
        let single_empty = view.extras.is_empty() && view.anchor == view.cursor;
        if single_empty {
            if let Some(r) = row_layouts.iter().find(|r| r.line == view.cursor.line) {
                back.push(fill(
                    Bounds::new(
                        point(px(gutter_right), px(r.y)),
                        size(px(width - gutter_width - minimap_width), line_height),
                    ),
                    theme.shade,
                ));
            }
        }
        // Find matches on the visible lines (the list is sorted, so skip to the first visible one).
        let matches = self.editor.read(cx).matches.clone();
        if let (Some(first), Some(last)) = (row_layouts.first(), row_layouts.last()) {
            let start = matches.partition_point(|m| m.start.line < first.line);
            for m in matches[start..].iter().take_while(|m| m.start.line <= last.line) {
                if let Some(r) = row_layouts.iter().find(|r| r.line == m.start.line) {
                    let x0 = x_of(r, m.start.col);
                    let x1 = if m.end.line == m.start.line {
                        x_of(r, m.end.col)
                    } else {
                        text_left + f32::from(r.shaped.width)
                    };
                    if x1 > x0 {
                        back.push(fill(
                            Bounds::new(point(px(x0), px(r.y)), size(px(x1 - x0), line_height)),
                            theme.match_bg,
                        ));
                    }
                }
            }
        }
        // Other uses of the name under the cursor (where it's written to gets a line underneath).
        let occurrences = self.editor.read(cx).occurrences.clone();
        for (s, e, write) in occurrences.iter() {
            if let Some(r) = row_layouts.iter().find(|r| r.line == s.line) {
                let x0 = x_of(r, s.col);
                let x1 = if e.line == s.line { x_of(r, e.col) } else { text_left + f32::from(r.shaped.width) };
                if x1 > x0 {
                    back.push(fill(
                        Bounds::new(point(px(x0), px(r.y)), size(px(x1 - x0), line_height)),
                        theme.highlight,
                    ));
                    if *write {
                        back.push(fill(
                            Bounds::new(point(px(x0), px(r.y) + line_height - px(2.0)), size(px(x1 - x0), px(2.0))),
                            theme.highlight_strong,
                        ));
                    }
                }
            }
        }
        // Problems from the language server and Ruff: a wavy line under each.
        let mut squiggles = Vec::new();
        let diagnostics = self.editor.read(cx).diagnostics.clone();
        for d in diagnostics.iter() {
            let color = match d.severity {
                1 => theme.err,
                2 => theme.warn,
                _ => theme.info,
            };
            for r in row_layouts.iter().filter(|r| r.line >= d.start.line && r.line <= d.end.line) {
                let x0 = if r.line == d.start.line { x_of(r, d.start.col) } else { text_left };
                let mut x1 =
                    if r.line == d.end.line { x_of(r, d.end.col) } else { text_left + f32::from(r.shaped.width) };
                // An empty range (end of line) still shows a short mark.
                if x1 - x0 < 4.0 {
                    x1 = x0 + char_width.max(4.0);
                }
                squiggles.push((point(px(x0), px(r.y + lh - 3.0)), px(x1 - x0), color));
            }
        }
        let sel_color = if focused { theme.selection } else { theme.selection_unfocused };
        for &(a, h) in &selections {
            let (start, end) = if a <= h { (a, h) } else { (h, a) };
            if start == end {
                continue;
            }
            for r in &row_layouts {
                if r.line < start.line || r.line > end.line {
                    continue;
                }
                let from = if r.line == start.line { x_of(r, start.col) } else { text_left };
                let to = if r.line == end.line {
                    x_of(r, end.col)
                } else {
                    // The line break is selected too: a little past the end.
                    text_left + f32::from(r.shaped.width) + char_width * 0.6
                };
                if to > from {
                    back.push(fill(Bounds::new(point(px(from), px(r.y)), size(px(to - from), line_height)), sel_color));
                }
            }
        }
        if let Some(m) = self.editor.update(cx, |e, cx| e.bracket_match(cx)) {
            for p in [m.bracket, m.matching] {
                if let Some(r) = row_layouts.iter().find(|r| r.line == p.line) {
                    let x0 = x_of(r, p.col);
                    let x1 = x_of(r, p.col + 1);
                    back.push(fill(
                        Bounds::new(point(px(x0), px(r.y)), size(px(x1 - x0), line_height)),
                        theme.highlight_strong,
                    ));
                }
            }
        }
        if let Some((ms, me)) = marked {
            if let Some(r) = row_layouts.iter().find(|r| r.line == view.cursor.line) {
                let x0 = x_of(r, ms);
                let x1 = x_of(r, me);
                front.push(fill(
                    Bounds::new(point(px(x0), px(r.y + lh - 2.0)), size(px((x1 - x0).max(1.0)), px(1.0))),
                    theme.fg,
                ));
            }
        }
        if focused {
            for &(_, h) in &selections {
                if let Some(r) = row_layouts.iter().find(|r| r.line == h.line) {
                    let x = x_of(r, h.col);
                    front.push(fill(Bounds::new(point(px(x), px(r.y)), size(px(2.0), line_height)), theme.accent));
                }
            }
        }

        // ----- the minimap -----
        let mut minimap_quads = Vec::new();
        let mut minimap_layout = None;
        if show_minimap {
            let mm_left = left + width - MINIMAP_WIDTH;
            minimap_quads.push(fill(
                Bounds::new(point(px(mm_left), px(top)), size(px(MINIMAP_WIDTH), px(height))),
                theme.bg_panel,
            ));
            let capacity = (height / ROW_PX).floor().max(1.0) as usize;
            let rows_per_sample = if rows > capacity { rows as f32 / capacity as f32 } else { 1.0 };
            let samples = rows.min(capacity);
            // Colour lines whose highlighting is cheap now; warm up a little more each frame.
            let more = document.update(cx, |d, _| {
                let doc = &mut d.doc;
                let mode = doc.mode();
                let more =
                    doc.highlighter.warm_up(&doc.buffer, mode, Duration::from_millis(4), line_count.saturating_sub(1));
                for i in 0..samples {
                    let row = (i as f32 * rows_per_sample) as usize;
                    let line = folds.buffer_line(row);
                    let y = top + i as f32 * ROW_PX;
                    let text = doc.buffer.line(line).to_owned();
                    let display = DisplayLine::new(&text, tab);
                    let indent = display.text.len() - display.text.trim_start().len();
                    let len = display.text.trim_end().len();
                    if len > indent {
                        let x = mm_left + 4.0 + indent as f32 * CHAR_PX;
                        let w = ((len - indent) as f32 * CHAR_PX).min(MINIMAP_WIDTH - 8.0 - indent as f32 * CHAR_PX);
                        if w > 0.0 {
                            minimap_quads.push(fill(
                                Bounds::new(point(px(x), px(y)), size(px(w), px(ROW_PX - 0.5))),
                                theme.fg_muted.opacity(0.35),
                            ));
                        }
                    }
                    if let Some(tokens) = doc.highlighter.tokens_if_cached(&doc.buffer, mode, line) {
                        for t in tokens {
                            let s = display.byte(t.start) as f32 * CHAR_PX;
                            let e = (display.byte(t.end) as f32 * CHAR_PX).min(MINIMAP_WIDTH - 8.0);
                            if e > s {
                                minimap_quads.push(fill(
                                    Bounds::new(point(px(mm_left + 4.0 + s), px(y)), size(px(e - s), px(ROW_PX - 0.5))),
                                    theme.token_style(t.kind).0.opacity(0.8),
                                ));
                            }
                        }
                    }
                }
                more
            });
            if more {
                window.request_animation_frame();
            }
            // The part that's on screen.
            let view_top = top + (scroll_top / lh) / rows_per_sample * ROW_PX;
            let view_h = (height / lh) / rows_per_sample * ROW_PX;
            minimap_quads.push(fill(
                Bounds::new(point(px(mm_left), px(view_top)), size(px(MINIMAP_WIDTH), px(view_h.max(4.0)))),
                theme.minimap_view,
            ));
            minimap_layout = Some(MinimapLayout { left: mm_left, top, height, rows_per_sample, row_px: ROW_PX });
        }

        let text_clip =
            Bounds::new(point(px(gutter_right), px(top)), size(px(width - gutter_width - minimap_width), px(height)));
        let gutter_clip = Bounds::new(point(px(left), px(top)), size(px(gutter_width), px(height)));
        let max_line_width = row_layouts.iter().map(|r| f32::from(r.shaped.width)).fold(0.0, f32::max);
        Prepaint {
            layout: Some(LayoutCache {
                bounds,
                text_left,
                gutter_right,
                line_height: lh,
                char_width,
                viewport_height: height,
                max_scroll_top,
                max_scroll_left: (max_line_width - text_area_width + char_width * 4.0).max(scroll_left).max(0.0),
                scroll_top,
                rows: row_layouts,
                folds,
                line_count,
                minimap: minimap_layout,
            }),
            quads_back: back,
            quads_front: front,
            squiggles,
            gutter,
            placeholders,
            minimap: minimap_quads,
            message: None,
            text_clip,
            gutter_clip,
            line_height,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&gpui_kit::GlobalElementId>,
        _inspector_id: Option<&gpui_kit::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.editor.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.editor.clone()), cx);
        let line_height = prepaint.line_height;
        if let Some((line, origin)) = prepaint.message.take() {
            let _ = line.paint(origin, line_height, TextAlign::Left, None, window, cx);
            return;
        }
        let Some(layout) = prepaint.layout.take() else { return };
        let theme = cx.global::<ForgeTheme>().0.clone();
        window.with_content_mask(Some(gpui_kit::ContentMask { bounds: prepaint.text_clip }), |window| {
            for q in prepaint.quads_back.drain(..) {
                window.paint_quad(q);
            }
            for r in &layout.rows {
                let _ = r.shaped.paint(
                    point(px(layout.text_left), px(r.y)),
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
            for (ph, origin, b) in prepaint.placeholders.drain(..) {
                window.paint_quad(fill(b, theme.bg_hover));
                let _ = ph.paint(origin, line_height, TextAlign::Left, None, window, cx);
            }
            for (origin, width, color) in prepaint.squiggles.drain(..) {
                window.paint_underline(
                    origin,
                    width,
                    &gpui_kit::UnderlineStyle { thickness: px(1.0), color: Some(color), wavy: true },
                );
            }
            for q in prepaint.quads_front.drain(..) {
                window.paint_quad(q);
            }
        });
        window.with_content_mask(Some(gpui_kit::ContentMask { bounds: prepaint.gutter_clip }), |window| {
            window.paint_quad(fill(prepaint.gutter_clip, theme.bg));
            for (line, origin) in prepaint.gutter.drain(..) {
                let _ = line.paint(origin, line_height, TextAlign::Left, None, window, cx);
            }
        });
        for q in prepaint.minimap.drain(..) {
            window.paint_quad(q);
        }
        self.editor.update(cx, |e, _| e.layout = Some(layout));
    }
}
