//! Brackets and quotes: auto-closing, typing over closers, deleting empty
//! pairs, Enter between brackets, finding the matching bracket, and toggling
//! line comments. Replaces `src/renderer/editor/pairs.ts`.

use crate::chars;
use crate::selection::{selected_lines, Selection};
use crate::text_buffer::{Pos, TextBuffer};
use regex::Regex;
use std::sync::LazyLock;

fn close_of(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        _ => None,
    }
}

fn open_of(close: char) -> Option<char> {
    match close {
        ')' => Some('('),
        ']' => Some('['),
        '}' => Some('{'),
        _ => None,
    }
}

fn is_quote(ch: char) -> bool {
    ch == '"' || ch == '\''
}

/// Characters before which it's fine to auto-insert a closing bracket/quote.
fn closes_before(ch: Option<char>) -> bool {
    match ch {
        None => true,
        Some(c) => c.is_whitespace() || ")]}:;,.".contains(c),
    }
}

/// Python string prefixes: f"...", r'...', b"...", rb"...".
static STRING_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:^|[^\p{L}\p{N}_])(?:f|r|b|u|rb|br|fr|rf)$").expect("valid regex"));

/// Typing one character, with bracket/quote help. Returns the new selection,
/// or `None` when nothing special applies (the caller then inserts normally).
/// `python_strings`: string prefixes (f"...") and docstrings ("""|""").
pub fn type_char(b: &mut TextBuffer, s: Selection, ch: char, python_strings: bool) -> Option<Selection> {
    let is_open = close_of(ch).is_some();
    let is_close = open_of(ch).is_some();
    let quote = is_quote(ch);
    if !is_open && !is_close && !quote {
        return None;
    }

    // With a selection: wrap it in brackets/quotes, keeping it selected.
    if !s.is_empty() {
        if is_close {
            return None;
        }
        let (start, end) = s.range();
        let close = if quote { ch } else { close_of(ch).expect("opener") };
        b.insert(end, &close.to_string());
        b.insert(start, &ch.to_string());
        let shift = |p: Pos| {
            if p.line == start.line {
                Pos::new(p.line, p.col + 1)
            } else {
                p
            }
        };
        return Some(Selection::new(shift(s.anchor), shift(s.head)));
    }

    let p = s.head;
    let line: Vec<char> = b.line(p.line).chars().collect();
    let at = |i: isize| -> Option<char> {
        if i < 0 {
            None
        } else {
            line.get(i as usize).copied()
        }
    };
    let col = p.col as isize;
    let next = at(col);
    let prev = at(col - 1);

    // Typing the closer that's already there: just step over it...
    if (is_close || quote) && next == Some(ch) {
        // ...except a third quote: "" + " makes a docstring """|"""
        if !(python_strings && quote && prev == Some(ch) && at(col - 2) == Some(ch)) {
            return Some(Selection::cursor(Pos::new(p.line, p.col + 1)));
        }
    }
    if is_open {
        if !closes_before(next) {
            return None;
        }
        b.insert(p, &format!("{ch}{}", close_of(ch).expect("opener")));
        return Some(Selection::cursor(Pos::new(p.line, p.col + 1)));
    }
    if quote {
        // Third quote of a docstring: """|"""
        if python_strings && prev == Some(ch) && at(col - 2) == Some(ch) && at(col - 3) != Some(ch) {
            b.insert(p, &ch.to_string().repeat(4));
            return Some(Selection::cursor(Pos::new(p.line, p.col + 1)));
        }
        // Don't pair after a word (it's an apostrophe: don't, it's), unless it's a string prefix like f"
        let before: String = line[..p.col.min(line.len())].iter().collect();
        if prev.is_some_and(chars::is_word) && !(python_strings && STRING_PREFIX.is_match(&before)) {
            return None;
        }
        if prev == Some(ch) {
            return None;
        }
        if !closes_before(next) {
            return None;
        }
        b.insert(p, &format!("{ch}{ch}"));
        return Some(Selection::cursor(Pos::new(p.line, p.col + 1)));
    }
    None
}

/// Backspace inside an empty pair, e.g. (|) or "|": delete both. Returns the new cursor, or `None`.
pub fn backspace_pair(b: &mut TextBuffer, p: Pos) -> Option<Pos> {
    if p.col == 0 {
        return None;
    }
    let line: Vec<char> = b.line(p.line).chars().collect();
    let prev = *line.get(p.col - 1)?;
    let next = *line.get(p.col)?;
    let before_prev = if p.col >= 2 { line.get(p.col - 2).copied() } else { None };
    let paired = close_of(prev) == Some(next) || (is_quote(prev) && next == prev && before_prev != Some(prev));
    if !paired {
        return None;
    }
    b.delete(Pos::new(p.line, p.col - 1), Pos::new(p.line, p.col + 1));
    Some(Pos::new(p.line, p.col - 1))
}

/// Enter between brackets, e.g. foo(|): put the cursor on its own indented
/// line with the closer below. Returns the new cursor, or `None`.
pub fn newline_between_brackets(b: &mut TextBuffer, p: Pos, tab_size: usize) -> Option<Pos> {
    let line: Vec<char> = b.line(p.line).chars().collect();
    let col = p.col.min(line.len());
    let before_trimmed = line[..col].iter().rposition(|c| !c.is_whitespace()).map(|i| i + 1).unwrap_or(0);
    let prev = if before_trimmed > 0 { Some(line[before_trimmed - 1]) } else { None }?;
    let next_index = line[col..].iter().position(|c| !c.is_whitespace())?;
    let next = line[col + next_index];
    if close_of(prev) != Some(next) {
        return None;
    }
    let indent: String = chars::leading_whitespace(b.line(p.line)).to_owned();
    let inner = format!("{indent}{}", " ".repeat(tab_size));
    // Remove spaces around the cursor, then: newline + inner indent | newline + indent + closer
    b.delete(Pos::new(p.line, before_trimmed), Pos::new(p.line, col + next_index));
    b.insert(Pos::new(p.line, before_trimmed), &format!("\n{inner}\n{indent}"));
    Some(Pos::new(p.line + 1, chars::len(&inner)))
}

/// A bracket and the bracket matching it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BracketMatch {
    pub bracket: Pos,
    pub matching: Pos,
}

/// The bracket matching the one at or just before `p`, skipping brackets
/// where `is_code` says no (inside strings and comments).
pub fn find_matching_bracket(
    b: &TextBuffer,
    p: Pos,
    is_code: &dyn Fn(usize, usize) -> bool,
    max_lines: usize,
) -> Option<BracketMatch> {
    let line: Vec<char> = b.line(p.line).chars().collect();
    // Prefer the bracket right after the cursor, then the one right before.
    for col in [Some(p.col), p.col.checked_sub(1)].into_iter().flatten() {
        let Some(&ch) = line.get(col) else { continue };
        if (close_of(ch).is_none() && open_of(ch).is_none()) || !is_code(p.line, col) {
            continue;
        }
        let forward = close_of(ch).is_some();
        let want = if forward { close_of(ch) } else { open_of(ch) }.expect("bracket");
        let mut depth: isize = 0;
        let mut l = p.line;
        let mut c = col as isize;
        let last_line = (b.line_count() - 1).min(p.line + max_lines);
        let first_line = p.line.saturating_sub(max_lines);
        let mut text: Vec<char> = line.clone();
        loop {
            if let Some(&cur) = usize::try_from(c).ok().and_then(|i| text.get(i)) {
                if (cur == ch || cur == want) && is_code(l, c as usize) {
                    depth += if cur == ch { 1 } else { -1 };
                    if depth == 0 {
                        return Some(BracketMatch {
                            bracket: Pos::new(p.line, col),
                            matching: Pos::new(l, c as usize),
                        });
                    }
                }
            }
            if forward {
                c += 1;
                if c as usize >= text.len() {
                    l += 1;
                    // Skip empty lines quickly.
                    while l <= last_line && b.line(l).is_empty() {
                        l += 1;
                    }
                    if l > last_line {
                        break;
                    }
                    text = b.line(l).chars().collect();
                    c = 0;
                }
            } else {
                c -= 1;
                if c < 0 {
                    if l == 0 || l - 1 < first_line {
                        break;
                    }
                    l -= 1;
                    while l >= first_line && b.line(l).is_empty() {
                        if l == 0 {
                            return None;
                        }
                        l -= 1;
                    }
                    if l < first_line {
                        break;
                    }
                    text = b.line(l).chars().collect();
                    c = text.len() as isize - 1;
                }
            }
        }
        return None;
    }
    None
}

/// Ctrl+/: comment the selected lines with "# " (or the language's marker),
/// or uncomment them if they all are already. Returns the selection adjusted to the edit.
pub fn toggle_comment(b: &mut TextBuffer, s: Selection, marker: &str) -> Selection {
    let (first, last) = selected_lines(s);
    let lines: Vec<usize> = (first..=last).filter(|&l| !b.line(l).trim().is_empty()).collect();
    if lines.is_empty() {
        return s;
    }
    let marker_len = chars::len(marker);
    let all_commented = lines.iter().all(|&l| b.line(l).trim_start().starts_with(marker));
    // line -> (col, by)
    let mut delta: std::collections::HashMap<usize, (usize, isize)> = std::collections::HashMap::new();
    if all_commented {
        for &l in &lines {
            let text = b.line(l).to_owned();
            let byte = text.find(marker).expect("commented line has the marker");
            let col = chars::col_of_byte(&text, byte);
            let after = chars::at(&text, col + marker_len);
            let len = marker_len + usize::from(after == Some(' '));
            b.delete(Pos::new(l, col), Pos::new(l, col + len));
            delta.insert(l, (col, -(len as isize)));
        }
    } else {
        // Line the markers up at the smallest indentation of the selected lines.
        let indent = lines.iter().map(|&l| chars::leading_whitespace(b.line(l)).len()).min().unwrap_or(0);
        for &l in &lines {
            b.insert(Pos::new(l, indent), &format!("{marker} "));
            delta.insert(l, (indent, marker_len as isize + 1));
        }
    }
    let adjust = |p: Pos| -> Pos {
        let Some(&(col, by)) = delta.get(&p.line) else {
            return p;
        };
        if by > 0 {
            return if p.col >= col { Pos::new(p.line, p.col + by as usize) } else { p };
        }
        // Removed text: positions inside it move to its start.
        if p.col <= col {
            return p;
        }
        Pos::new(p.line, ((p.col as isize + by).max(col as isize)) as usize)
    };
    // An empty selection stays a cursor that moved with its line's text.
    let anchor = adjust(s.anchor);
    let head = adjust(s.head);
    if s.anchor == s.head {
        Selection::cursor(head)
    } else {
        Selection::new(anchor, head)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_buffer::pos;

    /// Type `ch` at the cursor marked | in `text`; returns the text with | at the new cursor.
    fn type_at(text: &str, ch: char) -> String {
        let lines: Vec<&str> = text.split('\n').collect();
        let line = lines.iter().position(|l| l.contains('|')).expect("cursor");
        let col = chars::col_of_byte(lines[line], lines[line].find('|').expect("cursor"));
        let mut b = TextBuffer::new(&text.replacen('|', "", 1));
        let res = type_char(&mut b, Selection::cursor(pos(line, col)), ch, true);
        let at = match res {
            Some(s) => s.head,
            None => b.insert(pos(line, col), &ch.to_string()),
        };
        let mut out: Vec<String> = b.text().split('\n').map(str::to_owned).collect();
        let l = &out[at.line];
        out[at.line] = format!("{}|{}", chars::slice(l, 0, at.col), chars::slice_from(l, at.col));
        out.join("\n")
    }

    #[test]
    fn auto_closes_brackets_before_spaces_or_the_line_end() {
        assert_eq!(type_at("print|", '('), "print(|)");
        assert_eq!(type_at("x = |", '['), "x = [|]");
        assert_eq!(type_at("d = {|}", '{'), "d = {{|}}");
    }

    #[test]
    fn does_not_auto_close_right_before_a_word() {
        assert_eq!(type_at("|foo", '('), "(|foo");
    }

    #[test]
    fn types_over_an_existing_closer() {
        assert_eq!(type_at("print(x|)", ')'), "print(x)|");
        assert_eq!(type_at("s = \"abc|\"", '"'), "s = \"abc\"|");
    }

    #[test]
    fn auto_closes_quotes_including_after_string_prefixes() {
        assert_eq!(type_at("s = |", '"'), "s = \"|\"");
        assert_eq!(type_at("s = f|", '"'), "s = f\"|\"");
        assert_eq!(type_at("x = rb|", '\''), "x = rb'|'");
    }

    #[test]
    fn leaves_apostrophes_alone() {
        assert_eq!(type_at("# don|", '\''), "# don'|");
    }

    #[test]
    fn makes_docstrings() {
        assert_eq!(type_at("    \"\"|", '"'), "    \"\"\"|\"\"\"");
    }

    #[test]
    fn wraps_a_selection() {
        let mut b = TextBuffer::new("x = a + b");
        let res = type_char(&mut b, Selection::new(pos(0, 4), pos(0, 9)), '(', true);
        assert_eq!(b.text(), "x = (a + b)");
        assert_eq!(res, Some(Selection::new(pos(0, 5), pos(0, 10))));
    }

    #[test]
    fn ignores_other_characters() {
        assert_eq!(type_char(&mut TextBuffer::new(""), Selection::cursor(pos(0, 0)), 'a', true), None);
    }

    #[test]
    fn backspace_deletes_both_halves_of_an_empty_pair() {
        let mut b = TextBuffer::new("f()");
        assert_eq!(backspace_pair(&mut b, pos(0, 2)), Some(pos(0, 1)));
        assert_eq!(b.text(), "f");
        let mut q = TextBuffer::new("\"\"");
        backspace_pair(&mut q, pos(0, 1));
        assert_eq!(q.text(), "");
    }

    #[test]
    fn backspace_pair_does_nothing_elsewhere() {
        assert_eq!(backspace_pair(&mut TextBuffer::new("f(x)"), pos(0, 3)), None);
    }

    #[test]
    fn enter_between_brackets_puts_the_cursor_on_an_indented_line() {
        let mut b = TextBuffer::new("    x = foo()");
        let at = newline_between_brackets(&mut b, pos(0, 12), 4);
        assert_eq!(b.text(), "    x = foo(\n        \n    )");
        assert_eq!(at, Some(pos(1, 8)));
    }

    #[test]
    fn enter_between_brackets_only_between_a_matching_pair() {
        assert_eq!(newline_between_brackets(&mut TextBuffer::new("foo(x"), pos(0, 5), 4), None);
    }

    fn code(_: usize, _: usize) -> bool {
        true
    }

    #[test]
    fn finds_the_closer_for_an_opener_across_lines_and_nesting() {
        let b = TextBuffer::new("f(a, [1, 2],\n  (b))");
        assert_eq!(
            find_matching_bracket(&b, pos(0, 1), &code, 5000),
            Some(BracketMatch { bracket: pos(0, 1), matching: pos(1, 5) })
        );
    }

    #[test]
    fn finds_the_opener_for_a_closer_just_before_the_cursor() {
        let b = TextBuffer::new("x = [1, (2)]");
        assert_eq!(
            find_matching_bracket(&b, pos(0, 12), &code, 5000),
            Some(BracketMatch { bracket: pos(0, 11), matching: pos(0, 4) })
        );
    }

    #[test]
    fn skips_brackets_in_strings() {
        let b = TextBuffer::new("f(\")\", x)");
        let not_in_string = |_l: usize, c: usize| !(2..=4).contains(&c);
        assert_eq!(find_matching_bracket(&b, pos(0, 1), &not_in_string, 5000).map(|m| m.matching), Some(pos(0, 8)));
    }

    #[test]
    fn returns_none_when_nothing_matches() {
        assert_eq!(find_matching_bracket(&TextBuffer::new("f(x"), pos(0, 1), &code, 5000), None);
        assert_eq!(find_matching_bracket(&TextBuffer::new("abc"), pos(0, 1), &code, 5000), None);
    }

    #[test]
    fn finds_an_opener_on_an_earlier_line() {
        let b = TextBuffer::new("f(\n\n  x)");
        assert_eq!(find_matching_bracket(&b, pos(2, 4), &code, 5000).map(|m| m.matching), Some(pos(0, 1)));
    }

    #[test]
    fn comments_lines_at_their_shared_indentation() {
        let mut b = TextBuffer::new("def f():\n    x = 1\n\n    return x");
        toggle_comment(&mut b, Selection::new(pos(1, 0), pos(3, 3)), "#");
        assert_eq!(b.text(), "def f():\n    # x = 1\n\n    # return x");
    }

    #[test]
    fn uncomments_when_every_line_is_commented() {
        let mut b = TextBuffer::new("# a\n  #b");
        toggle_comment(&mut b, Selection::new(pos(0, 0), pos(1, 2)), "#");
        assert_eq!(b.text(), "a\n  b");
    }

    #[test]
    fn keeps_the_cursor_on_the_same_text() {
        let mut b = TextBuffer::new("x = 1");
        assert_eq!(toggle_comment(&mut b, Selection::cursor(pos(0, 4)), "#").head, pos(0, 6));
        assert_eq!(toggle_comment(&mut b, Selection::cursor(pos(0, 6)), "#").head, pos(0, 4));
    }
}
