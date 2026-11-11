//! A terminal's screen: alacritty's terminal model, and what the view needs
//! to draw it (cells with their colours resolved, the cursor, the selection).
//! The model is fed by the pty's event loop (`Terminal`) or directly (tests).

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, TermMode};
pub use alacritty_terminal::vte::ansi::Rgb;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};

/// The terminal's size in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub cols: usize,
    pub lines: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

/// Colours for the 16 ANSI colours and the defaults (from the theme).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub fg: Rgb,
    pub bg: Rgb,
    pub cursor: Rgb,
    pub ansi: [Rgb; 16],
}

const fn rgb(hex: u32) -> Rgb {
    Rgb { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8 }
}

/// xterm.js's default ANSI colours (what the old terminal showed).
pub const XTERM_ANSI: [Rgb; 16] = [
    rgb(0x2e3436),
    rgb(0xcc0000),
    rgb(0x4e9a06),
    rgb(0xc4a000),
    rgb(0x3465a4),
    rgb(0x75507b),
    rgb(0x06989a),
    rgb(0xd3d7cf),
    rgb(0x555753),
    rgb(0xef2929),
    rgb(0x8ae234),
    rgb(0xfce94f),
    rgb(0x729fcf),
    rgb(0xad7fa8),
    rgb(0x34e2e2),
    rgb(0xeeeeec),
];

/// A 256-colour index as RGB (16 ANSI colours, the 6×6×6 cube, the grey ramp).
pub fn indexed(i: u8, palette: &Palette) -> Rgb {
    match i {
        0..=15 => palette.ansi[i as usize],
        16..=231 => {
            let n = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Rgb { r: level(n / 36), g: level((n / 6) % 6), b: level(n % 6) }
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            Rgb { r: v, g: v, b: v }
        }
    }
}

/// One cell as drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellView {
    pub c: char,
    pub fg: Rgb,
    /// None: the default background (not drawn).
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// The cell after a wide character (nothing drawn).
    pub spacer: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorView {
    Block,
    Underline,
    Beam,
}

/// Everything needed to draw the visible part of a terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub cols: usize,
    pub lines: Vec<Vec<CellView>>,
    /// (line, column) on screen, if the cursor shows.
    pub cursor: Option<(usize, usize, CursorView)>,
    /// Selected range on screen ((line, col), (line, col)), inclusive.
    pub selection: Option<((usize, usize), (usize, usize))>,
    /// How far back in the history the view is (0 = at the bottom).
    pub display_offset: usize,
    pub app_cursor: bool,
    pub bracketed_paste: bool,
    pub alt_screen: bool,
    pub mouse_mode: bool,
}

fn resolve(color: Color, palette: &Palette, colors: &alacritty_terminal::term::color::Colors) -> Option<Rgb> {
    match color {
        Color::Spec(c) => Some(c),
        Color::Indexed(i) => Some(colors[i as usize].unwrap_or_else(|| indexed(i, palette))),
        Color::Named(n) => match n {
            NamedColor::Foreground | NamedColor::BrightForeground => Some(colors[n].unwrap_or(palette.fg)),
            NamedColor::Background => colors[n],
            NamedColor::Cursor => Some(palette.cursor),
            NamedColor::DimForeground => Some(colors[n].unwrap_or(palette.fg)),
            n if (n as usize) < 16 => Some(colors[n].unwrap_or(palette.ansi[n as usize])),
            n => {
                // Dim colours: the normal colour, a little darker.
                let base = (n as usize).saturating_sub(NamedColor::DimBlack as usize).min(7);
                let c = colors[base].unwrap_or(palette.ansi[base]);
                Some(Rgb { r: c.r / 3 * 2, g: c.g / 3 * 2, b: c.b / 3 * 2 })
            }
        },
    }
}

/// What the terminal shows now.
pub fn snapshot<T: EventListener>(term: &Term<T>, palette: &Palette) -> Snapshot {
    let content = term.renderable_content();
    let cols = term.columns();
    let screen_lines = term.screen_lines();
    let offset = content.display_offset as i32;
    let mut lines = vec![Vec::with_capacity(cols); screen_lines];
    for indexed_cell in content.display_iter {
        let row = (indexed_cell.point.line.0 + offset) as usize;
        let Some(line) = lines.get_mut(row) else { continue };
        let cell = indexed_cell.cell;
        let mut fg = resolve(cell.fg, palette, content.colors).unwrap_or(palette.fg);
        let mut bg = resolve(cell.bg, palette, content.colors);
        // Bold text in one of the first 8 colours uses the bright version, like xterm.
        if cell.flags.contains(Flags::BOLD) {
            if let Color::Named(n) = cell.fg {
                if (n as usize) < 8 {
                    fg = palette.ansi[n as usize + 8];
                }
            }
        }
        if cell.flags.contains(Flags::INVERSE) {
            let back = bg.unwrap_or(palette.bg);
            bg = Some(fg);
            fg = back;
        }
        let hidden = cell.flags.contains(Flags::HIDDEN);
        line.push(CellView {
            c: if hidden { ' ' } else { cell.c },
            fg,
            bg,
            bold: cell.flags.contains(Flags::BOLD),
            italic: cell.flags.contains(Flags::ITALIC),
            underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
            strike: cell.flags.contains(Flags::STRIKEOUT),
            spacer: cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER),
        });
    }
    let cursor = match content.cursor.shape {
        CursorShape::Hidden => None,
        shape => {
            let row = content.cursor.point.line.0 + offset;
            (row >= 0 && (row as usize) < screen_lines).then(|| {
                let view = match shape {
                    CursorShape::Underline => CursorView::Underline,
                    CursorShape::Beam => CursorView::Beam,
                    _ => CursorView::Block,
                };
                (row as usize, content.cursor.point.column.0, view)
            })
        }
    };
    let selection = content.selection.and_then(|r| {
        let start_row = r.start.line.0 + offset;
        let end_row = r.end.line.0 + offset;
        if end_row < 0 || start_row >= screen_lines as i32 {
            return None;
        }
        let start = if start_row < 0 { (0, 0) } else { (start_row as usize, r.start.column.0) };
        let end = if end_row >= screen_lines as i32 {
            (screen_lines - 1, cols.saturating_sub(1))
        } else {
            (end_row as usize, r.end.column.0)
        };
        Some((start, end))
    });
    let mode = content.mode;
    Snapshot {
        cols,
        lines,
        cursor,
        selection,
        display_offset: content.display_offset,
        app_cursor: mode.contains(TermMode::APP_CURSOR),
        bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
        alt_screen: mode.contains(TermMode::ALT_SCREEN),
        mouse_mode: mode.intersects(TermMode::MOUSE_MODE),
    }
}

/// A point on screen (line from the top of the view, column) as a grid point.
pub fn grid_point<T>(term: &Term<T>, line: usize, col: usize) -> Point {
    let offset = term.grid().display_offset() as i32;
    let col = col.min(term.columns().saturating_sub(1));
    Point::new(Line(line as i32 - offset), Column(col))
}

/// Start selecting at a screen point (double-click selects a word, triple a line).
pub fn start_selection<T>(term: &mut Term<T>, line: usize, col: usize, right_half: bool, clicks: usize) {
    let point = grid_point(term, line, col);
    let side = if right_half { Side::Right } else { Side::Left };
    let ty = match clicks {
        2 => SelectionType::Semantic,
        3.. => SelectionType::Lines,
        _ => SelectionType::Simple,
    };
    term.selection = Some(Selection::new(ty, point, side));
}

pub fn update_selection<T>(term: &mut Term<T>, line: usize, col: usize, right_half: bool) {
    let point = grid_point(term, line, col);
    let side = if right_half { Side::Right } else { Side::Left };
    if let Some(s) = term.selection.as_mut() {
        s.update(point, side);
    }
}

/// Scroll the view back (positive) or forward (negative) through the history.
pub fn scroll<T: EventListener>(term: &mut Term<T>, lines: i32) {
    term.scroll_display(Scroll::Delta(lines));
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::Processor;

    fn palette() -> Palette {
        Palette { fg: rgb(0xf3e2d4), bg: rgb(0x120c0a), cursor: rgb(0xffb02e), ansi: XTERM_ANSI }
    }

    fn term(cols: usize, lines: usize, input: &str) -> Term<VoidListener> {
        let mut t = Term::new(Config::default(), &Size { cols, lines }, VoidListener);
        let mut p: Processor = Processor::new();
        p.advance(&mut t, input.as_bytes());
        t
    }

    fn row_text(s: &Snapshot, row: usize) -> String {
        s.lines[row].iter().filter(|c| !c.spacer).map(|c| c.c).collect::<String>().trim_end().to_owned()
    }

    #[test]
    fn shows_text_and_where_the_cursor_is() {
        let t = term(20, 3, "hello\r\nworld");
        let s = snapshot(&t, &palette());
        assert_eq!(row_text(&s, 0), "hello");
        assert_eq!(row_text(&s, 1), "world");
        assert_eq!(s.cursor, Some((1, 5, CursorView::Block)));
        assert_eq!(s.lines[0].len(), 20);
    }

    #[test]
    fn resolves_ansi_256_and_true_colours() {
        let t = term(20, 2, "\x1b[31mr\x1b[0m\x1b[38;5;21mb\x1b[38;2;1;2;3mt\x1b[42mg");
        let s = snapshot(&t, &palette());
        let row = &s.lines[0];
        assert_eq!(row[0].fg, XTERM_ANSI[1]);
        assert_eq!(row[1].fg, Rgb { r: 0, g: 0, b: 255 });
        assert_eq!(row[2].fg, Rgb { r: 1, g: 2, b: 3 });
        assert_eq!(row[3].bg, Some(XTERM_ANSI[2]));
        assert_eq!(row[4].bg, None);
    }

    #[test]
    fn bold_brightens_and_inverse_swaps() {
        let t = term(10, 1, "\x1b[1;34mB\x1b[0m\x1b[7mI");
        let s = snapshot(&t, &palette());
        assert_eq!(s.lines[0][0].fg, XTERM_ANSI[12]);
        assert!(s.lines[0][0].bold);
        assert_eq!(s.lines[0][1].bg, Some(palette().fg));
        assert_eq!(s.lines[0][1].fg, palette().bg);
    }

    #[test]
    fn modes_are_reported() {
        let s = snapshot(&term(10, 2, "\x1b[?1h\x1b[?2004h"), &palette());
        assert!(s.app_cursor && s.bracketed_paste && !s.alt_screen);
        let s = snapshot(&term(10, 2, "\x1b[?1049h\x1b[?25l"), &palette());
        assert!(s.alt_screen);
        assert_eq!(s.cursor, None);
    }

    #[test]
    fn scrolling_back_shows_history_and_selection_gives_text() {
        let mut t = term(10, 2, "one\r\ntwo\r\nthree\r\nfour");
        assert_eq!(row_text(&snapshot(&t, &palette()), 0), "three");
        scroll(&mut t, 2);
        let s = snapshot(&t, &palette());
        assert_eq!(s.display_offset, 2);
        assert_eq!(row_text(&s, 0), "one");
        assert_eq!(s.cursor, None);
        start_selection(&mut t, 0, 0, false, 1);
        update_selection(&mut t, 1, 2, true);
        assert_eq!(t.selection_to_string().as_deref(), Some("one\ntwo"));
        let s = snapshot(&t, &palette());
        assert_eq!(s.selection, Some(((0, 0), (1, 2))));
    }

    #[test]
    fn the_256_colour_cube_and_greys() {
        let p = palette();
        assert_eq!(indexed(16, &p), Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(indexed(231, &p), Rgb { r: 255, g: 255, b: 255 });
        assert_eq!(indexed(232, &p), Rgb { r: 8, g: 8, b: 8 });
        assert_eq!(indexed(9, &p), XTERM_ANSI[9]);
    }
}
