//! Connects the tokenizer to a text buffer. Replaces `src/renderer/editor/highlighter.ts`.
//!
//! To colour line N we need the state at the start of line N (are we inside a
//! docstring?), which depends on every line before it. Those states are
//! cached and, when the text changes at line L, only the states after L are
//! thrown away. States are computed lazily, only up to the lines being drawn.

use crate::modes::LanguageMode;
use crate::semantic::{merge_tokens, SemToken};
use crate::text_buffer::{Change, TextBuffer};
use crate::tokenizer::{LineState, Token};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct Highlighter {
    /// states[i] = state at the start of line i. Always valid for every index it holds.
    states: Vec<LineState>,
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
        Self { states: vec![LineState::Normal], semantic: None }
    }

    /// Forget everything (the language changed, e.g. after a rename).
    pub fn reset(&mut self) {
        self.states.truncate(1);
        self.semantic = None;
    }

    /// How many line states are cached (for tests).
    pub fn cached_lines(&self) -> usize {
        self.states.len()
    }

    /// Tell the highlighter about one change to the buffer.
    pub fn on_change(&mut self, change: &Change) {
        // A change on a line can't affect the state at its start, only states after it.
        let line = change.at.line;
        if self.states.len() > line + 1 {
            self.states.truncate(line + 1);
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
        (line < self.states.len()).then(|| self.tokens_for(buffer, mode, line))
    }

    /// Cache more line states, for at most `budget`. Returns true if there's more to do.
    pub fn warm_up(&mut self, buffer: &TextBuffer, mode: &LanguageMode, budget: Duration, up_to: usize) -> bool {
        let up_to = up_to.min(buffer.line_count().saturating_sub(1));
        let end = Instant::now() + budget;
        while self.states.len() <= up_to {
            self.state_at(buffer, mode, up_to.min(self.states.len() + 200));
            if Instant::now() > end {
                break;
            }
        }
        self.states.len() <= up_to
    }

    fn state_at(&mut self, buffer: &TextBuffer, mode: &LanguageMode, line: usize) -> LineState {
        while self.states.len() <= line {
            let prev = self.states.len() - 1;
            let end = (mode.tokenize)(buffer.line(prev), self.states[prev]).1;
            self.states.push(end);
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
