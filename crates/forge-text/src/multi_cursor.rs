//! Multi-cursor logic. Replaces `src/renderer/editor/multiCursor.ts`.
//!
//! Several selections are edited one after another, top to bottom. Every edit
//! shifts the text after it, so the selections still waiting (and the results
//! already made) are moved along with each change.

use crate::selection::Selection;
use crate::text_buffer::{end_of, Change, ChangeKind, Pos, TextBuffer};

/// A selection that may be the primary one (the cursor the view follows).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MultiSelection {
    pub anchor: Pos,
    pub head: Pos,
    pub primary: bool,
}

impl MultiSelection {
    pub const fn new(anchor: Pos, head: Pos, primary: bool) -> Self {
        Self { anchor, head, primary }
    }

    pub const fn cursor(p: Pos) -> Self {
        Self { anchor: p, head: p, primary: false }
    }

    pub fn sel(&self) -> Selection {
        Selection::new(self.anchor, self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

impl From<Selection> for MultiSelection {
    fn from(s: Selection) -> Self {
        Self { anchor: s.anchor, head: s.head, primary: false }
    }
}

/// Where `p` ends up after `change`. `stick_after`: when text is inserted
/// exactly at `p`, move `p` after it (true) or keep it in front (false).
pub fn map_position(p: Pos, change: &Change, stick_after: bool) -> Pos {
    let at = change.at;
    let end = end_of(at, &change.text);
    match change.kind {
        ChangeKind::Insert => {
            if p < at || (p == at && !stick_after) {
                return p;
            }
            if p.line > at.line {
                return Pos::new(p.line + (end.line - at.line), p.col);
            }
            // Same line, at or after the insertion point.
            Pos::new(end.line, end.col + (p.col - at.col))
        }
        ChangeKind::Delete => {
            if p <= at {
                return p;
            }
            if p <= end {
                return at;
            }
            if p.line > end.line {
                return Pos::new(p.line - (end.line - at.line), p.col);
            }
            Pos::new(at.line, at.col + (p.col - end.col))
        }
    }
}

fn map_selection(s: MultiSelection, change: &Change, stick_after: bool) -> MultiSelection {
    MultiSelection {
        anchor: map_position(s.anchor, change, stick_after),
        head: map_position(s.head, change, stick_after),
        primary: s.primary,
    }
}

/// Sort selections top to bottom and merge any that overlap or sit on the
/// same spot. A merged selection stays primary if any of its parts was.
pub fn normalize(selections: &[MultiSelection]) -> Vec<MultiSelection> {
    let mut sorted = selections.to_vec();
    sorted.sort_by_key(|s| s.sel().start());
    let mut out: Vec<MultiSelection> = Vec::with_capacity(sorted.len());
    for s in sorted {
        if let Some(prev) = out.last().copied() {
            let (ps, pe) = prev.sel().range();
            let (cs, ce) = s.sel().range();
            let touching = cs < pe || cs == ps || (cs == pe && (prev.is_empty() || s.is_empty()) && ce == pe);
            if touching {
                let end = if ce > pe { ce } else { pe };
                let backwards = prev.head < prev.anchor;
                *out.last_mut().expect("non-empty") = MultiSelection {
                    anchor: if backwards { end } else { ps },
                    head: if backwards { ps } else { end },
                    primary: prev.primary || s.primary,
                };
                continue;
            }
        }
        out.push(s);
    }
    if !out.is_empty() && !out.iter().any(|s| s.primary) {
        out.last_mut().expect("non-empty").primary = true;
    }
    out
}

/// Run `op` on every selection (top to bottom), keeping the others in step
/// with each change. Returns the resulting selections, normalized.
pub fn edit_all(
    buffer: &mut TextBuffer,
    selections: &[MultiSelection],
    mut op: impl FnMut(&mut TextBuffer, Selection, usize) -> Selection,
) -> Vec<MultiSelection> {
    let mut pending: std::collections::VecDeque<MultiSelection> = normalize(selections).into();
    let mut done: Vec<MultiSelection> = Vec::with_capacity(pending.len());
    let mut index = 0;
    while let Some(s) = pending.pop_front() {
        let mark = buffer.journal_len();
        let result = op(buffer, s.sel(), index);
        index += 1;
        // Shift everything else along with what this edit changed: results
        // already made sit before (or at) the edit, so they stay in front of
        // inserted text; pending ones move after it.
        let changes = buffer.changes_since(mark).to_vec();
        for change in &changes {
            for d in done.iter_mut() {
                *d = map_selection(*d, change, false);
            }
            for p in pending.iter_mut() {
                *p = map_selection(*p, change, true);
            }
        }
        done.push(MultiSelection { anchor: result.anchor, head: result.head, primary: s.primary });
    }
    normalize(&done)
}

/// Every place `text` occurs (case-sensitive), as selections.
pub fn all_occurrences(buffer: &TextBuffer, text: &str) -> Vec<Selection> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let text_len = crate::chars::len(text);
    if !text.contains('\n') {
        for (line, content) in buffer.lines().iter().enumerate() {
            for (byte, _) in content.match_indices(text) {
                let col = crate::chars::col_of_byte(content, byte);
                out.push(Selection::new(Pos::new(line, col), Pos::new(line, col + text_len)));
            }
        }
        return out;
    }
    // Multi-line text: search the whole document.
    let full = buffer.lines().join("\n");
    for (byte, _) in full.match_indices(text) {
        let start = crate::chars::col_of_byte(&full, byte);
        out.push(Selection::new(buffer.position_at(start), buffer.position_at(start + text_len)));
    }
    out
}

/// Ctrl+D: the next occurrence of `text` after `after` (wrapping around to
/// the top) that isn't already selected, or `None` if there is none.
pub fn next_occurrence(buffer: &TextBuffer, text: &str, after: Pos, existing: &[Selection]) -> Option<Selection> {
    let all = all_occurrences(buffer, text);
    let taken = |s: &Selection| existing.iter().any(|e| e.range() == (s.anchor, s.head));
    all.iter().filter(|s| s.anchor >= after).chain(all.iter().filter(|s| s.anchor < after)).find(|s| !taken(s)).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands;
    use crate::selection::delete_selection;
    use crate::text_buffer::pos;

    fn c(line: usize, col: usize) -> MultiSelection {
        MultiSelection::cursor(pos(line, col))
    }

    #[test]
    fn moves_positions_after_an_insert_on_the_same_line() {
        let ins = Change::insert(pos(0, 2), "abc");
        assert_eq!(map_position(pos(0, 5), &ins, true), pos(0, 8));
        assert_eq!(map_position(pos(0, 1), &ins, true), pos(0, 1));
    }

    #[test]
    fn handles_inserted_line_breaks() {
        assert_eq!(map_position(pos(0, 5), &Change::insert(pos(0, 2), "x\nyy"), true), pos(1, 5));
        assert_eq!(map_position(pos(3, 1), &Change::insert(pos(0, 2), "\n\n"), true), pos(5, 1));
    }

    #[test]
    fn an_insert_exactly_at_the_position_goes_after_or_in_front() {
        let ins = Change::insert(pos(0, 2), "ab");
        assert_eq!(map_position(pos(0, 2), &ins, true), pos(0, 4));
        assert_eq!(map_position(pos(0, 2), &ins, false), pos(0, 2));
    }

    #[test]
    fn handles_deletes_before_inside_and_after() {
        let d = Change::delete(pos(1, 2), "cd\nef"); // removes (1,2)..(2,2)
        assert_eq!(map_position(pos(1, 1), &d, true), pos(1, 1));
        assert_eq!(map_position(pos(2, 0), &d, true), pos(1, 2));
        assert_eq!(map_position(pos(2, 5), &d, true), pos(1, 5));
        assert_eq!(map_position(pos(4, 3), &d, true), pos(3, 3));
    }

    #[test]
    fn normalize_sorts_and_removes_duplicate_cursors_keeping_the_primary_flag() {
        let mut first = c(2, 0);
        first.primary = true;
        let out = normalize(&[first, c(0, 1), c(2, 0)]);
        assert_eq!(out.iter().map(|s| s.head).collect::<Vec<_>>(), [pos(0, 1), pos(2, 0)]);
        assert!(out[1].primary);
    }

    #[test]
    fn normalize_merges_overlapping_selections() {
        let out = normalize(&[
            MultiSelection::new(pos(0, 0), pos(0, 5), false),
            MultiSelection::new(pos(0, 3), pos(0, 8), false),
        ]);
        assert_eq!(out, [MultiSelection::new(pos(0, 0), pos(0, 8), true)]);
    }

    #[test]
    fn normalize_keeps_selections_that_only_touch_end_to_start_apart() {
        let out = normalize(&[
            MultiSelection::new(pos(0, 0), pos(0, 3), false),
            MultiSelection::new(pos(0, 3), pos(0, 6), false),
        ]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn types_at_every_cursor() {
        let mut b = TextBuffer::new("ab\ncd\nef");
        let out = edit_all(&mut b, &[c(0, 1), c(1, 1), c(2, 1)], |b, s, _| Selection::cursor(b.insert(s.head, "XY")));
        assert_eq!(b.text(), "aXYb\ncXYd\neXYf");
        assert_eq!(out.iter().map(|s| s.head).collect::<Vec<_>>(), [pos(0, 3), pos(1, 3), pos(2, 3)]);
    }

    #[test]
    fn two_cursors_on_one_line_both_type_in_the_right_spots() {
        let mut b = TextBuffer::new("a b c");
        edit_all(&mut b, &[c(0, 1), c(0, 3)], |b, s, _| Selection::cursor(b.insert(s.head, "!")));
        assert_eq!(b.text(), "a! b! c");
    }

    #[test]
    fn enter_at_several_cursors_keeps_them_lined_up() {
        let mut b = TextBuffer::new("x = 1\ny = 2");
        let out = edit_all(&mut b, &[c(0, 5), c(1, 5)], |b, s, _| {
            Selection::cursor(commands::newline(b, s.head, 4, commands::python_indent_after))
        });
        assert_eq!(b.text(), "x = 1\n\ny = 2\n");
        assert_eq!(out.iter().map(|s| s.head).collect::<Vec<_>>(), [pos(1, 0), pos(3, 0)]);
    }

    #[test]
    fn replaces_several_selections() {
        let mut b = TextBuffer::new("foo bar foo");
        edit_all(
            &mut b,
            &[MultiSelection::new(pos(0, 0), pos(0, 3), false), MultiSelection::new(pos(0, 8), pos(0, 11), false)],
            |b, s, _| {
                let at = delete_selection(b, s);
                Selection::cursor(b.insert(at, "baz"))
            },
        );
        assert_eq!(b.text(), "baz bar baz");
    }

    #[test]
    fn backspace_at_cursors_that_end_up_merging_leaves_one_cursor() {
        let mut b = TextBuffer::new("ab");
        let out = edit_all(&mut b, &[c(0, 1), c(0, 2)], |b, s, _| Selection::cursor(commands::backspace(b, s.head, 4)));
        assert_eq!(b.text(), "");
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn finds_all_occurrences() {
        let b = TextBuffer::new("foo = 1\nfoo += foo\nbar");
        let all = all_occurrences(&b, "foo");
        assert_eq!(all.iter().map(|s| s.anchor).collect::<Vec<_>>(), [pos(0, 0), pos(1, 0), pos(1, 7)]);
    }

    #[test]
    fn finds_multi_line_text() {
        let b = TextBuffer::new("foo = 1\nfoo += foo\nbar");
        assert_eq!(all_occurrences(&b, "1\nfoo"), [Selection::new(pos(0, 6), pos(1, 3))]);
    }

    #[test]
    fn next_occurrence_skips_taken_ones_and_wraps_around() {
        let b = TextBuffer::new("foo = 1\nfoo += foo\nbar");
        let taken = [Selection::new(pos(1, 7), pos(1, 10))];
        assert_eq!(next_occurrence(&b, "foo", pos(1, 3), &taken), Some(Selection::new(pos(0, 0), pos(0, 3))));
        let all = all_occurrences(&b, "foo");
        assert_eq!(next_occurrence(&b, "foo", pos(0, 0), &all), None);
    }
}
