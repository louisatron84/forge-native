//! The editor's model of a file: lines of text (without line endings).
//!
//! Replaces `src/renderer/editor/textBuffer.ts`. An array of lines is the
//! simplest structure that is fast enough for source files, since edits touch
//! one or a few lines. Columns count Unicode characters (`char`s).
//!
//! The TypeScript buffer called listeners on every change; here the buffer
//! keeps a journal of changes instead. Undo history and multi-cursor editing
//! read the changes an edit made (`changes_since`), and the document owner
//! drains the journal (`take_changes`) to update folds, versions and
//! highlighting.

use crate::chars;
use std::cmp::Ordering;

/// A place in the text. `line` and `col` are 0-based; `col` counts characters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    pub const fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

/// Shorthand for tests and callers.
pub const fn pos(line: usize, col: usize) -> Pos {
    Pos { line, col }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Insert,
    Delete,
}

/// One low-level change. `text` always uses '\n' for line breaks.
/// Insert: `text` was inserted at `at`. Delete: `text` was removed starting at `at`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    pub at: Pos,
    pub text: String,
}

impl Change {
    pub fn insert(at: Pos, text: impl Into<String>) -> Self {
        Self { kind: ChangeKind::Insert, at, text: text.into() }
    }

    pub fn delete(at: Pos, text: impl Into<String>) -> Self {
        Self { kind: ChangeKind::Delete, at, text: text.into() }
    }
}

/// Line ending used when saving: kept the same as the file had.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Eol {
    #[default]
    Lf,
    CrLf,
}

impl Eol {
    pub fn as_str(self) -> &'static str {
        match self {
            Eol::Lf => "\n",
            Eol::CrLf => "\r\n",
        }
    }

    /// The label the status bar shows.
    pub fn label(self) -> &'static str {
        match self {
            Eol::Lf => "LF",
            Eol::CrLf => "CRLF",
        }
    }
}

/// The position just after `text` if it were inserted at `at`.
pub fn end_of(at: Pos, text: &str) -> Pos {
    let pieces = split_lines(text);
    if pieces.len() == 1 {
        return Pos::new(at.line, at.col + chars::len(text));
    }
    Pos::new(at.line + pieces.len() - 1, chars::len(pieces[pieces.len() - 1]))
}

pub fn compare(a: Pos, b: Pos) -> Ordering {
    a.cmp(&b)
}

/// Split on \n, \r\n or \r. An empty string is one empty line.
pub fn split_lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                out.push(&text[start..i]);
                i += 1;
                start = i;
            }
            b'\r' => {
                out.push(&text[start..i]);
                i += if bytes.get(i + 1) == Some(&b'\n') { 2 } else { 1 };
                start = i;
            }
            _ => i += 1,
        }
    }
    out.push(&text[start..]);
    out
}

#[derive(Clone, Debug)]
pub struct TextBuffer {
    lines: Vec<String>,
    /// Line ending used when saving.
    pub eol: Eol,
    journal: Vec<Change>,
}

impl TextBuffer {
    pub fn new(text: &str) -> Self {
        let eol = if text.contains("\r\n") { Eol::CrLf } else { Eol::Lf };
        Self { lines: split_lines(text).into_iter().map(str::to_owned).collect(), eol, journal: Vec::new() }
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// A line's text ("" past the end).
    pub fn line(&self, line: usize) -> &str {
        self.lines.get(line).map(String::as_str).unwrap_or("")
    }

    /// Length of a line in characters.
    pub fn line_len(&self, line: usize) -> usize {
        chars::len(self.line(line))
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn text(&self) -> String {
        self.lines.join(self.eol.as_str())
    }

    pub fn clamp(&self, p: Pos) -> Pos {
        let line = p.line.min(self.lines.len() - 1);
        Pos::new(line, p.col.min(chars::len(&self.lines[line])))
    }

    /// Insert `text` at `p`; returns the position just after the inserted text.
    pub fn insert(&mut self, p: Pos, text: &str) -> Pos {
        let at = self.clamp(p);
        if text.is_empty() {
            return at;
        }
        let current = &self.lines[at.line];
        let split = chars::byte_at(current, at.col);
        let before = current[..split].to_owned();
        let after = current[split..].to_owned();
        let pieces = split_lines(text);
        if pieces.len() == 1 {
            self.lines[at.line] = format!("{before}{text}{after}");
        } else {
            let last = pieces[pieces.len() - 1];
            let mut replacement = Vec::with_capacity(pieces.len());
            replacement.push(format!("{before}{}", pieces[0]));
            for piece in &pieces[1..pieces.len() - 1] {
                replacement.push((*piece).to_owned());
            }
            replacement.push(format!("{last}{after}"));
            self.lines.splice(at.line..=at.line, replacement);
        }
        let normalized = pieces.join("\n");
        let end = end_of(at, &normalized);
        self.journal.push(Change::insert(at, normalized));
        end
    }

    /// The text between two positions (in either order), with '\n' line breaks.
    pub fn range(&self, a: Pos, b: Pos) -> String {
        let (start, end) = self.ordered(a, b);
        if start.line == end.line {
            return chars::slice(&self.lines[start.line], start.col, end.col).to_owned();
        }
        let mut out = String::new();
        out.push_str(chars::slice_from(&self.lines[start.line], start.col));
        for line in &self.lines[start.line + 1..end.line] {
            out.push('\n');
            out.push_str(line);
        }
        out.push('\n');
        out.push_str(chars::slice(&self.lines[end.line], 0, end.col));
        out
    }

    fn ordered(&self, a: Pos, b: Pos) -> (Pos, Pos) {
        let (a, b) = (self.clamp(a), self.clamp(b));
        if a > b {
            (b, a)
        } else {
            (a, b)
        }
    }

    /// Delete the text between two positions (in either order).
    pub fn delete(&mut self, a: Pos, b: Pos) {
        let (start, end) = self.ordered(a, b);
        if start == end {
            return;
        }
        let removed = self.range(start, end);
        let head = chars::slice(&self.lines[start.line], 0, start.col).to_owned();
        let tail = chars::slice_from(&self.lines[end.line], end.col).to_owned();
        self.lines.splice(start.line..=end.line, [format!("{head}{tail}")]);
        self.journal.push(Change::delete(start, removed));
    }

    /// Replace the whole text (e.g. reload from disk), recorded as delete + insert.
    pub fn set_text(&mut self, text: &str) {
        let last = self.lines.len() - 1;
        let end = Pos::new(last, chars::len(&self.lines[last]));
        self.delete(Pos::new(0, 0), end);
        self.insert(Pos::new(0, 0), text);
    }

    /// Character offset of a position, counting each line break as 1 character.
    pub fn offset_at(&self, p: Pos) -> usize {
        let Pos { line, col } = self.clamp(p);
        self.lines[..line].iter().map(|l| chars::len(l) + 1).sum::<usize>() + col
    }

    pub fn position_at(&self, offset: usize) -> Pos {
        let mut remaining = offset;
        for (line, text) in self.lines.iter().enumerate() {
            let len = chars::len(text);
            if remaining <= len {
                return Pos::new(line, remaining);
            }
            remaining -= len + 1;
        }
        let last = self.lines.len() - 1;
        Pos::new(last, chars::len(&self.lines[last]))
    }

    /// The end of the document.
    pub fn end(&self) -> Pos {
        let last = self.lines.len() - 1;
        Pos::new(last, chars::len(&self.lines[last]))
    }

    // ---------- the change journal ----------

    /// How many changes have been recorded (a mark for `changes_since`).
    pub fn journal_len(&self) -> usize {
        self.journal.len()
    }

    /// The changes made since `mark` (a value of `journal_len`).
    pub fn changes_since(&self, mark: usize) -> &[Change] {
        &self.journal[mark.min(self.journal.len())..]
    }

    /// Take every recorded change (the document owner does this after each edit).
    pub fn take_changes(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.journal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_text_and_keeps_the_original_line_ending_style() {
        assert_eq!(TextBuffer::new("a\nb").text(), "a\nb");
        assert_eq!(TextBuffer::new("a\r\nb\r\n").text(), "a\r\nb\r\n");
        assert_eq!(TextBuffer::new("").line_count(), 1);
    }

    #[test]
    fn inserts_text_inside_a_line_and_returns_the_position_after_it() {
        let mut b = TextBuffer::new("helo");
        let end = b.insert(pos(0, 3), "l");
        assert_eq!(b.text(), "hello");
        assert_eq!(end, pos(0, 4));
    }

    #[test]
    fn inserts_multi_line_text() {
        let mut b = TextBuffer::new("start end");
        let end = b.insert(pos(0, 6), "one\ntwo\r\nthree ");
        assert_eq!(b.lines(), ["start one", "two", "three end"]);
        assert_eq!(end, pos(2, 6));
    }

    #[test]
    fn deletes_within_a_line() {
        let mut b = TextBuffer::new("hello");
        b.delete(pos(0, 1), pos(0, 3));
        assert_eq!(b.text(), "hlo");
    }

    #[test]
    fn deletes_across_lines_joining_them_and_accepts_reversed_ranges() {
        let mut b = TextBuffer::new("ab\ncd\nef");
        b.delete(pos(2, 1), pos(0, 1));
        assert_eq!(b.text(), "af");
    }

    #[test]
    fn converts_between_positions_and_offsets() {
        let b = TextBuffer::new("ab\ncde\nf");
        assert_eq!(b.offset_at(pos(1, 2)), 5);
        assert_eq!(b.position_at(5), pos(1, 2));
        assert_eq!(b.position_at(3), pos(1, 0));
        assert_eq!(b.position_at(999), pos(2, 1));
    }

    #[test]
    fn clamps_out_of_range_positions() {
        // (The TypeScript test also clamped negative positions; usize can't be negative.)
        let b = TextBuffer::new("ab\ncde");
        assert_eq!(b.clamp(pos(9, 9)), pos(1, 3));
        assert_eq!(b.clamp(pos(0, 0)), pos(0, 0));
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        let mut b = TextBuffer::new("héllo wörld");
        b.insert(pos(0, 2), "X");
        assert_eq!(b.text(), "héXllo wörld");
        assert_eq!(b.range(pos(0, 7), pos(0, 12)), "wörld");
        b.delete(pos(0, 0), pos(0, 3));
        assert_eq!(b.text(), "llo wörld");
    }

    #[test]
    fn records_changes_in_the_journal() {
        let mut b = TextBuffer::new("ab\ncd");
        let mark = b.journal_len();
        b.insert(pos(0, 1), "X\nY");
        b.delete(pos(0, 0), pos(1, 1));
        assert_eq!(b.changes_since(mark), [Change::insert(pos(0, 1), "X\nY"), Change::delete(pos(0, 0), "aX\nY")]);
        assert_eq!(b.take_changes().len(), 2);
        assert_eq!(b.journal_len(), 0);
    }

    #[test]
    fn splits_every_kind_of_line_break() {
        assert_eq!(split_lines("a\r\nb\rc\nd"), ["a", "b", "c", "d"]);
        assert_eq!(split_lines(""), [""]);
        assert_eq!(split_lines("x\n"), ["x", ""]);
    }
}
