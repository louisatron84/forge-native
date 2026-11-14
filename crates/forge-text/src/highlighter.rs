//! Connects the tokenizer to a text buffer. Replaces `src/renderer/editor/highlighter.ts`.
//!
//! To colour line N we need the state at the start of line N (are we inside a
//! docstring?), which depends on every line before it. Those states are
//! cached. After an edit, propagation stops when the state matches the old
//! state beyond all edited lines. Line splices discard the misaligned tail.
//! States are computed lazily, only up to the lines being drawn.

use crate::modes::LanguageMode;
use crate::semantic::{merge_tokens, SemToken};
use crate::text_buffer::{Change, TextBuffer};
use crate::tokenizer::{LineState, Token};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct Highlighter {
    /// states[i] = state at the start of line i. The prefix before `valid` is current;
    /// the remaining states can be reused once propagation converges.
    states: Vec<LineState>,
    valid: usize,
    /// Do not converge before processing every pending same-line edit.
    dirty_through: usize,
    /// From the language server: what each name is, by line (lines from an
    /// edit onwards are dropped until it answers again).
    semantic: Option<BTreeMap<usize, Vec<SemToken>>>,
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

impl Highlighter {
    pub fn new() -> Self {
        Self { states: vec![LineState::Normal], valid: 1, dirty_through: 0, semantic: None }
    }

    /// Forget everything (the language changed, e.g. after a rename).
    pub fn reset(&mut self) {
        self.states.truncate(1);
        self.valid = 1;
        self.dirty_through = 0;
        self.semantic = None;
    }

    /// How many line states are cached (for tests).
    pub fn cached_lines(&self) -> usize {
        self.valid
    }

    /// Tell the highlighter about one change to the buffer.
    pub fn on_change(&mut self, change: &Change) {
        // A change on a line can't affect the state at its start, only states after it.
        let line = change.at.line;
        self.valid = self.valid.min(line + 1);
        if change.text.contains('\n') {
            // A line splice changes indices; the old tail no longer aligns.
            self.states.truncate(line + 1);
            self.dirty_through = line + 1;
        } else {
            self.dirty_through = self.dirty_through.max(line + 1);
        }
        // Same number of lines: only that line's colours are stale; otherwise everything after it may have moved.
        if let Some(sem) = &mut self.semantic {
            let moved = change.text.contains('\n');
            sem.retain(|&l, _| if moved { l < line } else { l != line });
        }
    }

    pub fn tokens_for(&mut self, buffer: &TextBuffer, mode: &LanguageMode, line: usize) -> Vec<Token> {
        let state = self.state_at(buffer, mode, line);
        let tokens = (mode.tokenize)(buffer.line(line), state).0;
        match &self.semantic {
            Some(sem) => merge_tokens(&tokens, sem.get(&line).map(Vec::as_slice)),
            None => tokens,
        }
    }

    /// The language server's semantic tokens for the whole file (`None`: none).
    pub fn set_semantic(&mut self, tokens: Option<BTreeMap<usize, Vec<SemToken>>>) {
        self.semantic = tokens;
    }

    /// Tokens for `line` only if that's cheap (the state before it is cached),
    /// else `None`. For the minimap: it must never make a keystroke re-tokenize
    /// the rest of a big file.
    pub fn tokens_if_cached(&mut self, buffer: &TextBuffer, mode: &LanguageMode, line: usize) -> Option<Vec<Token>> {
        (line < self.valid).then(|| self.tokens_for(buffer, mode, line))
    }

    /// Cache more line states, for at most `budget`. Returns true if there's more to do.
    pub fn warm_up(&mut self, buffer: &TextBuffer, mode: &LanguageMode, budget: Duration, up_to: usize) -> bool {
        let up_to = up_to.min(buffer.line_count().saturating_sub(1));
        let end = Instant::now() + budget;
        while self.valid <= up_to {
            self.state_at(buffer, mode, up_to.min(self.valid + 200));
            if Instant::now() > end {
                break;
            }
        }
        self.valid <= up_to
    }

    fn state_at(&mut self, buffer: &TextBuffer, mode: &LanguageMode, line: usize) -> LineState {
        while self.valid <= line {
            let prev = self.valid - 1;
            let end = (mode.tokenize)(buffer.line(prev), self.states[prev]).1;
            if self.valid < self.states.len() {
                let converged = self.valid >= self.dirty_through && self.states[self.valid] == end;
                self.states[self.valid] = end;
                if converged {
                    self.valid = self.states.len();
                    self.dirty_through = 0;
                    continue;
                }
            } else {
                self.states.push(end);
            }
            self.valid += 1;
        }
        self.states[line]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modes::mode_for;
    use crate::text_buffer::pos;

    fn kinds(h: &mut Highlighter, b: &TextBuffer, line: usize) -> Vec<&'static str> {
        h.tokens_for(b, mode_for("a.py"), line).iter().map(|t| t.kind.as_str()).collect()
    }

    /// Apply buffer changes to the highlighter, as the document does.
    fn sync(h: &mut Highlighter, b: &mut TextBuffer) {
        for c in b.take_changes() {
            h.on_change(&c);
        }
    }

    #[test]
    fn incremental_states_match_fresh_highlighting_after_batched_edits() {
        for path in ["a.py", "a.jsonc"] {
            let mode = mode_for(path);
            let mut b = TextBuffer::new("value = 1\nnext = 2\nthird = 3\nlast = 4\n");
            let mut h = Highlighter::new();
            let mut seed = 12345u64;
            let inserts = ["x", "\"\"\"", "\n", "/*", "*/", "#", "\"", " "];
            for _ in 0..150 {
                h.tokens_for(&b, mode, b.line_count() - 1);
                for _ in 0..3 {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let line = (seed >> 32) as usize % b.line_count();
                    if seed & 3 == 0 && b.line_len(line) > 0 {
                        b.delete(pos(line, 0), pos(line, 1));
                    } else {
                        b.insert(pos(line, 0), inserts[(seed >> 16) as usize % inserts.len()]);
                    }
                }
                sync(&mut h, &mut b);
                let mut fresh = Highlighter::new();
                for line in 0..b.line_count() {
                    assert_eq!(h.tokens_for(&b, mode, line), fresh.tokens_for(&b, mode, line), "{path}:{line}");
                }
            }
        }
    }

    #[test]
    fn unchanged_state_reuses_the_warm_tail_after_typing() {
        let mut b = TextBuffer::new(&"value = 123\n".repeat(100_000));
        let mut h = Highlighter::new();
        let mode = mode_for("a.py");
        h.tokens_for(&b, mode, 99_999);
        b.insert(pos(0, 0), "x");
        sync(&mut h, &mut b);
        h.tokens_for(&b, mode, 1);
        assert_eq!(h.cached_lines(), 100_000, "unchanged lexical state must reuse the warm tail");
    }

    #[test]
    fn convergence_does_not_skip_a_second_pending_edit() {
        let mut b = TextBuffer::new("x = 1\ny = 2\nz = 3\nlast = 4");
        let mut h = Highlighter::new();
        kinds(&mut h, &b, 3);
        b.insert(pos(0, 0), "x");
        b.insert(pos(2, 0), "\"\"\"");
        sync(&mut h, &mut b);
        assert_eq!(kinds(&mut h, &b, 3), ["string"]);
        b.insert(pos(1, 0), "new = 0\n");
        sync(&mut h, &mut b);
        assert_eq!(kinds(&mut h, &b, 4), ["string"]);
    }

    #[test]
    fn colors_lines_inside_a_docstring_as_string_using_state_from_earlier_lines() {
        let b = TextBuffer::new("def f():\n    \"\"\"Doc\n    more\n    \"\"\"\n    return 1");
        let mut h = Highlighter::new();
        assert_eq!(kinds(&mut h, &b, 2), ["string"]);
        assert_eq!(kinds(&mut h, &b, 4), ["keyword", "number"]);
    }

    #[test]
    fn re_colors_later_lines_when_an_edit_opens_a_docstring() {
        let mut b = TextBuffer::new("x = 1\ny = 2\nz = 3");
        let mut h = Highlighter::new();
        assert_eq!(kinds(&mut h, &b, 2), ["number"]);
        b.insert(pos(0, 0), "\"\"\"");
        sync(&mut h, &mut b);
        assert_eq!(kinds(&mut h, &b, 1), ["string"]);
        assert_eq!(kinds(&mut h, &b, 2), ["string"]);
        b.delete(pos(0, 0), pos(0, 3));
        sync(&mut h, &mut b);
        assert_eq!(kinds(&mut h, &b, 2), ["number"]);
    }

    #[test]
    fn only_tokenizes_as_far_as_it_needs_to() {
        let text: Vec<String> = (0..10_000).map(|i| format!("x{i} = {i}")).collect();
        let b = TextBuffer::new(&text.join("\n"));
        let mut h = Highlighter::new();
        h.tokens_for(&b, mode_for("a.py"), 5);
        assert!(h.cached_lines() < 10);
    }

    #[test]
    fn minimap_only_gets_tokens_for_cached_lines_and_warms_up_on_request() {
        let text: Vec<String> = (0..1000).map(|i| format!("x{i} = \"{i}\"")).collect();
        let mut b = TextBuffer::new(&text.join("\n"));
        let mut h = Highlighter::new();
        let mode = mode_for("a.py");
        assert!(h.tokens_if_cached(&b, mode, 0).is_some());
        assert!(h.tokens_if_cached(&b, mode, 900).is_none());
        while h.warm_up(&b, mode, Duration::from_millis(50), 900) {}
        assert!(!h.tokens_if_cached(&b, mode, 900).expect("cached").is_empty());
        // An edit near the top forgets the states below it again.
        b.insert(pos(1, 0), "# ");
        sync(&mut h, &mut b);
        assert!(h.tokens_if_cached(&b, mode, 900).is_none());
    }

    #[test]
    fn semantic_tokens_on_an_edited_line_are_dropped() {
        let mut b = TextBuffer::new("x = y\nz = w");
        let mut h = Highlighter::new();
        let mut sem = BTreeMap::new();
        sem.insert(0, vec![Token::new(4, 5, crate::tokenizer::TokenType::SemVariable)]);
        sem.insert(1, vec![Token::new(4, 5, crate::tokenizer::TokenType::SemVariable)]);
        h.set_semantic(Some(sem));
        b.insert(pos(0, 5), "!");
        sync(&mut h, &mut b);
        assert!(h.tokens_for(&b, mode_for("a.py"), 0).is_empty());
        assert_eq!(h.tokens_for(&b, mode_for("a.py"), 1).len(), 1);
    }
}
