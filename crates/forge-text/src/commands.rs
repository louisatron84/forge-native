//! Editing and cursor commands as plain functions of (buffer, position), so
//! they can be tested without any UI. Each returns the new cursor.
//! Replaces `src/renderer/editor/commands.ts`.

use crate::chars;
use crate::text_buffer::{Pos, TextBuffer};

// ---------- tabs & columns ----------

/// On-screen column of character `col`, with tabs expanded to tab stops.
pub fn visual_col(line_text: &str, col: usize, tab_size: usize) -> usize {
    let mut v = 0;
    for ch in line_text.chars().take(col) {
        v = if ch == '\t' { (v / tab_size + 1) * tab_size } else { v + 1 };
    }
    v
}

/// Character column whose on-screen position is nearest to visual column `target`.
pub fn col_from_visual(line_text: &str, target: usize, tab_size: usize) -> usize {
    let mut v = 0;
    for (i, ch) in line_text.chars().enumerate() {
        let next = if ch == '\t' { (v / tab_size + 1) * tab_size } else { v + 1 };
        if target < next {
            return if target - v <= next - target { i } else { i + 1 };
        }
        v = next;
    }
    chars::len(line_text)
}

// ---------- movement ----------

pub fn move_left(b: &TextBuffer, p: Pos) -> Pos {
    if p.col > 0 {
        Pos::new(p.line, p.col - 1)
    } else if p.line > 0 {
        Pos::new(p.line - 1, b.line_len(p.line - 1))
    } else {
        p
    }
}

pub fn move_right(b: &TextBuffer, p: Pos) -> Pos {
    if p.col < b.line_len(p.line) {
        Pos::new(p.line, p.col + 1)
    } else if p.line + 1 < b.line_count() {
        Pos::new(p.line + 1, 0)
    } else {
        p
    }
}

/// "Smart" Home: first non-space character, or column 0 if already there.
pub fn move_home(b: &TextBuffer, p: Pos) -> Pos {
    let indent = chars::leading_whitespace(b.line(p.line)).len();
    Pos::new(p.line, if p.col == indent { 0 } else { indent })
}

pub fn move_end(b: &TextBuffer, p: Pos) -> Pos {
    Pos::new(p.line, b.line_len(p.line))
}

/// Move `delta` lines up/down. `goal_visual` is the on-screen column the user
/// started from, so moving through a short line doesn't lose the column.
pub fn move_vertical(b: &TextBuffer, p: Pos, delta: isize, goal_visual: usize, tab_size: usize) -> Pos {
    let target = p.line as isize + delta;
    if target < 0 {
        return Pos::new(0, 0);
    }
    let target = target as usize;
    if target > b.line_count() - 1 {
        let last = b.line_count() - 1;
        return Pos::new(last, b.line_len(last));
    }
    Pos::new(target, col_from_visual(b.line(target), goal_visual, tab_size))
}

// ---------- editing ----------

pub fn backspace(b: &mut TextBuffer, p: Pos, tab_size: usize) -> Pos {
    if p.col == 0 {
        if p.line == 0 {
            return p;
        }
        let prev = Pos::new(p.line - 1, b.line_len(p.line - 1));
        b.delete(prev, p);
        return prev;
    }
    let before = chars::slice(b.line(p.line), 0, p.col);
    // Inside leading spaces: remove back to the previous indent stop.
    let width = if !before.is_empty() && before.chars().all(|c| c == ' ') { (p.col - 1) % tab_size + 1 } else { 1 };
    let start = Pos::new(p.line, p.col - width);
    b.delete(start, p);
    start
}

pub fn delete_forward(b: &mut TextBuffer, p: Pos) -> Pos {
    let next = move_right(b, p);
    b.delete(p, next);
    p
}

/// Python's rule: a block opener ends with `:` (ignoring a comment).
pub fn python_indent_after(before: &str) -> bool {
    let code = before.find('#').map_or(before, |i| &before[..i]);
    code.trim_end().ends_with(':')
}

/// Never adds a level (plain text).
pub fn no_indent_after(_before: &str) -> bool {
    false
}

/// Enter: keep the indentation, plus one level when `indent_after` says the line opens a block.
pub fn newline(b: &mut TextBuffer, p: Pos, tab_size: usize, indent_after: fn(&str) -> bool) -> Pos {
    let line_text = b.line(p.line).to_owned();
    let before = chars::slice(&line_text, 0, p.col);
    let mut indent = chars::leading_whitespace(before).to_owned();
    if indent_after(before) {
        indent.push_str(&" ".repeat(tab_size));
    }
    // Text after the cursor moves down; drop its leading spaces so it lines up with the indent.
    let rest_spaces = chars::leading_whitespace(chars::slice_from(&line_text, p.col)).len();
    if rest_spaces > 0 {
        b.delete(p, Pos::new(p.line, p.col + rest_spaces));
    }
    b.insert(p, &format!("\n{indent}"))
}

/// Tab: insert spaces up to the next tab stop (Python style, no tab characters).
pub fn insert_tab(b: &mut TextBuffer, p: Pos, tab_size: usize) -> Pos {
    let v = visual_col(b.line(p.line), p.col, tab_size);
    b.insert(p, &" ".repeat(tab_size - v % tab_size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_buffer::pos;

    #[test]
    fn horizontal_movement_wraps_at_the_edges() {
        let b = TextBuffer::new("ab\ncd");
        assert_eq!(move_left(&b, pos(1, 0)), pos(0, 2));
        assert_eq!(move_right(&b, pos(0, 2)), pos(1, 0));
    }

    #[test]
    fn stops_at_the_start_and_end_of_the_file() {
        let b = TextBuffer::new("ab\ncd");
        assert_eq!(move_left(&b, pos(0, 0)), pos(0, 0));
        assert_eq!(move_right(&b, pos(1, 2)), pos(1, 2));
    }

    #[test]
    fn home_goes_to_the_first_non_space_then_to_column_0() {
        let b = TextBuffer::new("    x = 1");
        assert_eq!(move_home(&b, pos(0, 9)), pos(0, 4));
        assert_eq!(move_home(&b, pos(0, 4)), pos(0, 0));
    }

    #[test]
    fn end_goes_to_the_end_of_the_line() {
        assert_eq!(move_end(&TextBuffer::new("    x = 1"), pos(0, 2)), pos(0, 9));
    }

    #[test]
    fn expands_tabs_to_the_next_tab_stop() {
        assert_eq!(visual_col("\tx", 1, 4), 4);
        assert_eq!(visual_col("ab\tx", 3, 4), 4);
        assert_eq!(visual_col("abc", 2, 4), 2);
    }

    #[test]
    fn maps_a_visual_column_back_to_the_nearest_character_column() {
        assert_eq!(col_from_visual("\tx", 4, 4), 1);
        assert_eq!(col_from_visual("\tx", 1, 4), 0);
        assert_eq!(col_from_visual("\tx", 3, 4), 1);
        assert_eq!(col_from_visual("ab", 10, 4), 2);
    }

    #[test]
    fn keeps_the_goal_column_across_short_lines() {
        let b = TextBuffer::new("long line\nab\nanother long");
        let goal = visual_col(b.line(0), 7, 4);
        let down1 = move_vertical(&b, pos(0, 7), 1, goal, 4);
        assert_eq!(down1, pos(1, 2));
        assert_eq!(move_vertical(&b, down1, 1, goal, 4), pos(2, 7));
    }

    #[test]
    fn goes_to_the_line_start_end_at_the_top_bottom_of_the_file() {
        let b = TextBuffer::new("long line\nab\nanother long");
        assert_eq!(move_vertical(&b, pos(0, 4), -1, 4, 4), pos(0, 0));
        assert_eq!(move_vertical(&b, pos(2, 3), 1, 3, 4), pos(2, 12));
    }

    #[test]
    fn backspace_deletes_one_character() {
        let mut b = TextBuffer::new("abc");
        assert_eq!(backspace(&mut b, pos(0, 2), 4), pos(0, 1));
        assert_eq!(b.text(), "ac");
    }

    #[test]
    fn backspace_joins_with_the_previous_line_at_column_0() {
        let mut b = TextBuffer::new("ab\ncd");
        assert_eq!(backspace(&mut b, pos(1, 0), 4), pos(0, 2));
        assert_eq!(b.text(), "abcd");
    }

    #[test]
    fn backspace_removes_a_whole_indent_level_inside_leading_spaces() {
        let mut b = TextBuffer::new("        x");
        assert_eq!(backspace(&mut b, pos(0, 8), 4), pos(0, 4));
        assert_eq!(b.text(), "    x");
    }

    #[test]
    fn backspace_snaps_uneven_indentation_back_to_the_previous_tab_stop() {
        let mut b = TextBuffer::new("      x");
        assert_eq!(backspace(&mut b, pos(0, 6), 4), pos(0, 4));
    }

    #[test]
    fn backspace_does_nothing_at_the_start_of_the_file() {
        let mut b = TextBuffer::new("a");
        assert_eq!(backspace(&mut b, pos(0, 0), 4), pos(0, 0));
    }

    #[test]
    fn delete_removes_the_next_character_or_joins_the_next_line() {
        let mut b = TextBuffer::new("ab\ncd");
        delete_forward(&mut b, pos(0, 0));
        assert_eq!(b.text(), "b\ncd");
        delete_forward(&mut b, pos(0, 1));
        assert_eq!(b.text(), "bcd");
    }

    #[test]
    fn enter_keeps_the_current_indentation() {
        let mut b = TextBuffer::new("    x = 1");
        assert_eq!(newline(&mut b, pos(0, 9), 4, python_indent_after), pos(1, 4));
        assert_eq!(b.text(), "    x = 1\n    ");
    }

    #[test]
    fn enter_indents_one_more_level_after_a_colon() {
        let mut b = TextBuffer::new("def f():");
        assert_eq!(newline(&mut b, pos(0, 8), 4, python_indent_after), pos(1, 4));
        assert_eq!(b.text(), "def f():\n    ");
    }

    #[test]
    fn enter_ignores_trailing_spaces_and_comments_after_the_colon() {
        let mut b = TextBuffer::new("if x:   # check");
        assert_eq!(newline(&mut b, pos(0, 15), 4, python_indent_after), pos(1, 4));
    }

    #[test]
    fn enter_splits_a_line_moving_the_rest_down_without_its_leading_spaces() {
        let mut b = TextBuffer::new("    a = 1 + 2");
        newline(&mut b, pos(0, 10), 4, python_indent_after);
        assert_eq!(b.lines(), ["    a = 1 ", "    + 2"]);
    }

    #[test]
    fn tab_inserts_spaces_up_to_the_next_tab_stop() {
        let mut b = TextBuffer::new("ab");
        assert_eq!(insert_tab(&mut b, pos(0, 2), 4), pos(0, 4));
        assert_eq!(b.text(), "ab  ");
    }
}
