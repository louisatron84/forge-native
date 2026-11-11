//! Selections and word-based logic. Replaces `src/renderer/editor/selection.ts`.
//!
//! A selection has an `anchor` (where it started) and a `head` (where the
//! cursor is now). The head can be before the anchor when selecting
//! backwards. An empty selection (anchor == head) is just a cursor.

use crate::chars;
use crate::text_buffer::{Pos, TextBuffer};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Pos,
    pub head: Pos,
}

impl Selection {
    pub const fn new(anchor: Pos, head: Pos) -> Self {
        Self { anchor, head }
    }

    /// A cursor (empty selection) at `p`.
    pub const fn cursor(p: Pos) -> Self {
        Self { anchor: p, head: p }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// (start, end) in document order.
    pub fn range(&self) -> (Pos, Pos) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    pub fn start(&self) -> Pos {
        self.range().0
    }

    pub fn end(&self) -> Pos {
        self.range().1
    }

    pub fn is_backwards(&self) -> bool {
        self.head < self.anchor
    }
}

/// Delete the selected text; returns where the cursor ends up (the start).
pub fn delete_selection(b: &mut TextBuffer, s: Selection) -> Pos {
    let (start, end) = s.range();
    b.delete(start, end);
    start
}

// ---------- words ----------

#[derive(Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Word,
    Space,
    Punct,
}

fn class_of(ch: char) -> CharClass {
    if chars::is_word(ch) {
        CharClass::Word
    } else if ch == ' ' || ch == '\t' {
        CharClass::Space
    } else {
        CharClass::Punct
    }
}

/// The [start, end) columns to select on double-click at `col`. Prefers the
/// character under the cursor, but if that's a space or end of line and the
/// one before is a word, selects the word before (clicking just after a word).
pub fn word_at(line_text: &str, col: usize) -> (usize, usize) {
    let text: Vec<char> = line_text.chars().collect();
    if text.is_empty() {
        return (0, 0);
    }
    let mut i = col.min(text.len() - 1);
    if i > 0 && class_of(text[i - 1]) == CharClass::Word && class_of(text[i]) != CharClass::Word {
        i -= 1;
    }
    let cls = class_of(text[i]);
    if cls == CharClass::Punct {
        return (i, i + 1);
    }
    let mut start = i;
    let mut end = i + 1;
    while start > 0 && class_of(text[start - 1]) == cls {
        start -= 1;
    }
    while end < text.len() && class_of(text[end]) == cls {
        end += 1;
    }
    (start, end)
}

/// Ctrl+Right: skip spaces, then a run of word characters (or one punctuation run).
pub fn word_right(b: &TextBuffer, p: Pos) -> Pos {
    let text: Vec<char> = b.line(p.line).chars().collect();
    if p.col >= text.len() {
        return if p.line + 1 < b.line_count() { Pos::new(p.line + 1, 0) } else { p };
    }
    let mut i = p.col;
    while i < text.len() && class_of(text[i]) == CharClass::Space {
        i += 1;
    }
    if i < text.len() {
        let cls = class_of(text[i]);
        while i < text.len() && class_of(text[i]) == cls {
            i += 1;
        }
    }
    Pos::new(p.line, i)
}

/// Ctrl+Left: the mirror image of `word_right`.
pub fn word_left(b: &TextBuffer, p: Pos) -> Pos {
    if p.col == 0 {
        return if p.line > 0 { Pos::new(p.line - 1, b.line_len(p.line - 1)) } else { p };
    }
    let text: Vec<char> = b.line(p.line).chars().collect();
    let mut i = p.col.min(text.len());
    while i > 0 && class_of(text[i - 1]) == CharClass::Space {
        i -= 1;
    }
    if i > 0 {
        let cls = class_of(text[i - 1]);
        while i > 0 && class_of(text[i - 1]) == cls {
            i -= 1;
        }
    }
    Pos::new(p.line, i)
}

// ---------- indent / dedent ----------

/// Lines touched by a selection. A selection ending at column 0 doesn't include that last line.
pub fn selected_lines(s: Selection) -> (usize, usize) {
    let (start, end) = s.range();
    let last = if end.col == 0 && end.line > start.line { end.line - 1 } else { end.line };
    (start.line, last)
}

fn shift(p: Pos, first: usize, last: usize, delta: &dyn Fn(usize) -> isize) -> Pos {
    if p.line < first || p.line > last {
        return p;
    }
    Pos::new(p.line, (p.col as isize + delta(p.line)).max(0) as usize)
}

/// Tab with a multi-line selection: add one indent level to each non-empty line.
pub fn indent_lines(b: &mut TextBuffer, s: Selection, tab_size: usize) -> Selection {
    let (first, last) = selected_lines(s);
    let unit = " ".repeat(tab_size);
    let mut added = std::collections::HashMap::new();
    for line in first..=last {
        if b.line(line).is_empty() {
            continue;
        }
        b.insert(Pos::new(line, 0), &unit);
        added.insert(line, tab_size as isize);
    }
    let delta = |line: usize| added.get(&line).copied().unwrap_or(0);
    Selection::new(shift(s.anchor, first, last, &delta), shift(s.head, first, last, &delta))
}

/// Shift+Tab: remove up to one indent level from each selected line.
pub fn dedent_lines(b: &mut TextBuffer, s: Selection, tab_size: usize) -> Selection {
    let (first, last) = selected_lines(s);
    let mut removed = std::collections::HashMap::new();
    for line in first..=last {
        let text = b.line(line);
        let n = if text.starts_with('\t') { 1 } else { text.len() - text.trim_start_matches(' ').len() };
        let count = n.min(tab_size);
        if count == 0 {
            continue;
        }
        b.delete(Pos::new(line, 0), Pos::new(line, count));
        removed.insert(line, count as isize);
    }
    let delta = |line: usize| -removed.get(&line).copied().unwrap_or(0);
    Selection::new(shift(s.anchor, first, last, &delta), shift(s.head, first, last, &delta))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_buffer::pos;

    fn sel(a: (usize, usize), h: (usize, usize)) -> Selection {
        Selection::new(pos(a.0, a.1), pos(h.0, h.1))
    }

    #[test]
    fn orders_a_backwards_selection_into_start_end() {
        assert_eq!(sel((2, 1), (0, 3)).range(), (pos(0, 3), pos(2, 1)));
    }

    #[test]
    fn knows_when_it_is_empty() {
        assert!(sel((1, 1), (1, 1)).is_empty());
        assert!(!sel((1, 1), (1, 2)).is_empty());
    }

    #[test]
    fn deletes_the_selected_text_and_returns_the_start() {
        let mut b = TextBuffer::new("hello\nworld");
        assert_eq!(delete_selection(&mut b, sel((1, 2), (0, 2))), pos(0, 2));
        assert_eq!(b.text(), "herld");
    }

    const LINE: &str = "x = my_var.attr  + 1";

    #[test]
    fn word_at_selects_identifier_characters() {
        assert_eq!(word_at(LINE, 6), (4, 10));
        assert_eq!(word_at(LINE, 10), (4, 10)); // just after the word still selects it
    }

    #[test]
    fn word_at_selects_a_run_of_spaces() {
        assert_eq!(word_at(LINE, 16), (15, 17));
    }

    #[test]
    fn word_at_selects_a_single_punctuation_character() {
        assert_eq!(word_at(LINE, 2), (2, 3));
    }

    #[test]
    fn word_at_handles_empty_lines() {
        assert_eq!(word_at("", 0), (0, 0));
    }

    #[test]
    fn moves_right_over_a_word_then_over_punctuation() {
        let b = TextBuffer::new("foo.bar(baz)  qux\nnext");
        assert_eq!(word_right(&b, pos(0, 0)), pos(0, 3));
        assert_eq!(word_right(&b, pos(0, 3)), pos(0, 4));
        assert_eq!(word_right(&b, pos(0, 12)), pos(0, 17));
    }

    #[test]
    fn word_movement_wraps_at_the_edges() {
        let b = TextBuffer::new("foo.bar(baz)  qux\nnext");
        assert_eq!(word_right(&b, pos(0, 17)), pos(1, 0));
        assert_eq!(word_left(&b, pos(1, 0)), pos(0, 17));
    }

    #[test]
    fn moves_left_to_the_start_of_the_word() {
        let b = TextBuffer::new("foo.bar(baz)  qux\nnext");
        assert_eq!(word_left(&b, pos(0, 17)), pos(0, 14));
        assert_eq!(word_left(&b, pos(0, 14)), pos(0, 11));
        assert_eq!(word_left(&b, pos(0, 6)), pos(0, 4));
    }

    #[test]
    fn indents_every_selected_line_and_shifts_the_selection() {
        let mut b = TextBuffer::new("a\nb\nc");
        let out = indent_lines(&mut b, sel((0, 1), (1, 1)), 4);
        assert_eq!(b.lines(), ["    a", "    b", "c"]);
        assert_eq!(out, sel((0, 5), (1, 5)));
    }

    #[test]
    fn skips_the_last_line_when_the_selection_ends_at_its_column_0() {
        let mut b = TextBuffer::new("a\nb\nc");
        indent_lines(&mut b, sel((0, 0), (2, 0)), 4);
        assert_eq!(b.lines(), ["    a", "    b", "c"]);
    }

    #[test]
    fn does_not_add_trailing_spaces_to_empty_lines() {
        let mut b = TextBuffer::new("a\n\nb");
        indent_lines(&mut b, sel((0, 0), (2, 1)), 4);
        assert_eq!(b.lines(), ["    a", "", "    b"]);
    }

    #[test]
    fn dedents_by_up_to_one_level_never_past_column_0() {
        let mut b = TextBuffer::new("        a\n  b\nc");
        let out = dedent_lines(&mut b, sel((0, 9), (2, 1)), 4);
        assert_eq!(b.lines(), ["    a", "b", "c"]);
        assert_eq!(out, sel((0, 5), (2, 1)));
    }

    #[test]
    fn dedents_a_leading_tab_character() {
        let mut b = TextBuffer::new("\tx");
        dedent_lines(&mut b, sel((0, 1), (0, 1)), 4);
        assert_eq!(b.text(), "x");
    }
}
