//! Undo/redo. Replaces `src/renderer/editor/history.ts`.
//!
//! Each user action (a keystroke, a paste, Enter...) runs inside `transact`,
//! which collects the buffer changes it made into one step. Undo applies the
//! opposite of a step's changes in reverse order; redo replays them. Quick
//! consecutive typing (or deleting) is merged into one step, so Ctrl+Z
//! removes a word, not a single letter.

use crate::text_buffer::{end_of, Change, ChangeKind, Pos, TextBuffer};
use std::time::Instant;

/// Kinds of edit. Only `Typing` and `Delete` merge with the previous step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Delete,
    Newline,
    Paste,
    Other,
}

#[derive(Clone, Debug)]
struct Step {
    id: u64,
    kind: EditKind,
    changes: Vec<Change>,
    cursor_before: Pos,
    cursor_after: Pos,
    time: u64,
    /// Never merge more edits into this step (it was saved, or undo/redo landed on it).
    sealed: bool,
}

const MERGE_WINDOW_MS: u64 = 1000;
const MAX_STEPS: usize = 1000;

/// Milliseconds, from a clock that can be replaced in tests.
pub type Clock = Box<dyn Fn() -> u64 + Send>;

pub struct History {
    undo_stack: Vec<Step>,
    redo_stack: Vec<Step>,
    next_id: u64,
    /// Id of the top undo step when last saved (0 = empty stack).
    saved_id: u64,
    now: Clock,
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for History {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("History").field("undo", &self.undo_stack.len()).field("redo", &self.redo_stack.len()).finish()
    }
}

impl History {
    pub fn new() -> Self {
        let start = Instant::now();
        Self::with_clock(Box::new(move || start.elapsed().as_millis() as u64))
    }

    pub fn with_clock(now: Clock) -> Self {
        Self { undo_stack: Vec::new(), redo_stack: Vec::new(), next_id: 1, saved_id: 0, now }
    }

    /// Run an edit as one undoable action. `edit` returns the new cursor.
    pub fn transact(
        &mut self,
        buffer: &mut TextBuffer,
        kind: EditKind,
        cursor_before: Pos,
        edit: impl FnOnce(&mut TextBuffer) -> Pos,
    ) -> Pos {
        let mark = buffer.journal_len();
        let cursor_after = edit(buffer);
        let changes = buffer.changes_since(mark).to_vec();
        self.record(kind, cursor_before, cursor_after, changes);
        cursor_after
    }

    /// Record changes made outside `transact` (e.g. by multi-cursor editing) as one step.
    pub fn record(&mut self, kind: EditKind, cursor_before: Pos, cursor_after: Pos, changes: Vec<Change>) {
        let time = (self.now)();
        let prev_index = self.undo_stack.len().checked_sub(1);
        if changes.is_empty() {
            // Typing a ")" that's already there just steps over it: keep the word
            // being typed in one undo step by moving its end along.
            if let Some(i) = prev_index {
                let prev = &mut self.undo_stack[i];
                if kind == EditKind::Typing
                    && !prev.sealed
                    && prev.kind == EditKind::Typing
                    && prev.cursor_after == cursor_before
                    && time.saturating_sub(prev.time) < MERGE_WINDOW_MS
                {
                    prev.cursor_after = cursor_after;
                    prev.time = time;
                }
            }
            return;
        }
        let can_merge = prev_index.is_some_and(|i| {
            let prev = &self.undo_stack[i];
            !prev.sealed
                && prev.kind == kind
                && matches!(kind, EditKind::Typing | EditKind::Delete)
                && prev.cursor_after == cursor_before
                && time.saturating_sub(prev.time) < MERGE_WINDOW_MS
        });
        if can_merge {
            let prev = self.undo_stack.last_mut().expect("checked above");
            prev.changes.extend(changes);
            prev.cursor_after = cursor_after;
            prev.time = time;
        } else {
            let id = self.next_id;
            self.next_id += 1;
            self.undo_stack.push(Step { id, kind, changes, cursor_before, cursor_after, time, sealed: false });
            if self.undo_stack.len() > MAX_STEPS {
                self.undo_stack.remove(0);
            }
        }
        self.redo_stack.clear();
    }

    /// Undo the last step; returns where the cursor should go, or `None` if nothing to undo.
    pub fn undo(&mut self, buffer: &mut TextBuffer) -> Option<Pos> {
        let step = self.undo_stack.pop()?;
        for c in step.changes.iter().rev() {
            match c.kind {
                ChangeKind::Insert => buffer.delete(c.at, end_of(c.at, &c.text)),
                ChangeKind::Delete => {
                    buffer.insert(c.at, &c.text);
                }
            }
        }
        let cursor = step.cursor_before;
        self.redo_stack.push(step);
        self.seal_top();
        Some(cursor)
    }

    pub fn redo(&mut self, buffer: &mut TextBuffer) -> Option<Pos> {
        let step = self.redo_stack.pop()?;
        for c in &step.changes {
            match c.kind {
                ChangeKind::Insert => {
                    buffer.insert(c.at, &c.text);
                }
                ChangeKind::Delete => buffer.delete(c.at, end_of(c.at, &c.text)),
            }
        }
        let cursor = step.cursor_after;
        self.undo_stack.push(step);
        self.seal_top();
        Some(cursor)
    }

    pub fn mark_saved(&mut self) {
        self.seal_top();
        self.saved_id = self.undo_stack.last().map_or(0, |s| s.id);
    }

    fn seal_top(&mut self) {
        if let Some(top) = self.undo_stack.last_mut() {
            top.sealed = true;
        }
    }

    /// True if the text differs from the last saved (or opened) state.
    pub fn is_dirty(&self) -> bool {
        self.undo_stack.last().map_or(0, |s| s.id) != self.saved_id
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_buffer::pos;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    /// A buffer and a history with a fake clock we can move forward.
    fn setup(text: &str) -> (TextBuffer, History, Arc<AtomicU64>) {
        let now = Arc::new(AtomicU64::new(0));
        let clock = now.clone();
        (TextBuffer::new(text), History::with_clock(Box::new(move || clock.load(Ordering::SeqCst))), now)
    }

    fn tick(now: &AtomicU64, ms: u64) {
        now.fetch_add(ms, Ordering::SeqCst);
    }

    #[test]
    fn undoes_and_redoes_a_single_insert_returning_cursor_positions() {
        let (mut b, mut h, _) = setup("ab");
        h.transact(&mut b, EditKind::Paste, pos(0, 1), |b| b.insert(pos(0, 1), "X\nY"));
        assert_eq!(b.text(), "aX\nYb");
        assert_eq!(h.undo(&mut b), Some(pos(0, 1)));
        assert_eq!(b.text(), "ab");
        assert_eq!(h.redo(&mut b), Some(pos(1, 1)));
        assert_eq!(b.text(), "aX\nYb");
    }

    #[test]
    fn undoes_a_multi_line_delete() {
        let (mut b, mut h, _) = setup("ab\ncd\nef");
        h.transact(&mut b, EditKind::Delete, pos(2, 1), |b| {
            b.delete(pos(0, 1), pos(2, 1));
            pos(0, 1)
        });
        assert_eq!(b.text(), "af");
        h.undo(&mut b);
        assert_eq!(b.text(), "ab\ncd\nef");
    }

    #[test]
    fn keeps_typing_in_one_step_when_a_keystroke_only_steps_over_an_auto_closed_bracket() {
        let (mut b, mut h, now) = setup("");
        let mut p = h.transact(&mut b, EditKind::Typing, pos(0, 0), |b| b.insert(pos(0, 0), "g"));
        tick(&now, 50);
        p = h.transact(&mut b, EditKind::Typing, p, |b| {
            b.insert(pos(0, 1), "()");
            pos(0, 2)
        });
        tick(&now, 50);
        p = h.transact(&mut b, EditKind::Typing, p, |_| pos(0, 3)); // step over ")": no change
        tick(&now, 50);
        h.transact(&mut b, EditKind::Typing, p, |b| b.insert(pos(0, 3), ":"));
        assert_eq!(b.text(), "g():");
        h.undo(&mut b);
        assert_eq!(b.text(), "");
    }

    #[test]
    fn groups_quick_consecutive_typing_into_one_undo_step() {
        let (mut b, mut h, now) = setup("");
        let mut p = pos(0, 0);
        for ch in ["h", "e", "l", "l", "o"] {
            p = h.transact(&mut b, EditKind::Typing, p, |b| b.insert(p, ch));
            tick(&now, 100);
        }
        h.undo(&mut b);
        assert_eq!(b.text(), "");
    }

    #[test]
    fn starts_a_new_step_after_a_pause_a_cursor_jump_or_a_different_kind_of_edit() {
        let (mut b, mut h, now) = setup("");
        let mut p = pos(0, 0);
        p = h.transact(&mut b, EditKind::Typing, p, |b| b.insert(p, "a"));
        tick(&now, 2000); // pause
        p = h.transact(&mut b, EditKind::Typing, p, |b| b.insert(p, "b"));
        p = h.transact(&mut b, EditKind::Newline, p, |b| b.insert(p, "\n"));
        p = h.transact(&mut b, EditKind::Typing, p, |b| b.insert(p, "c"));
        let _ = p;
        h.transact(&mut b, EditKind::Typing, pos(0, 0), |b| b.insert(pos(0, 0), "Z")); // cursor jumped
        assert_eq!(b.text(), "Zab\nc");
        h.undo(&mut b);
        assert_eq!(b.text(), "ab\nc");
        h.undo(&mut b);
        assert_eq!(b.text(), "ab\n");
        h.undo(&mut b);
        assert_eq!(b.text(), "ab");
        h.undo(&mut b);
        assert_eq!(b.text(), "a");
        h.undo(&mut b);
        assert_eq!(b.text(), "");
        assert_eq!(h.undo(&mut b), None);
    }

    #[test]
    fn a_new_edit_after_undo_clears_the_redo_stack() {
        let (mut b, mut h, _) = setup("");
        h.transact(&mut b, EditKind::Paste, pos(0, 0), |b| b.insert(pos(0, 0), "a"));
        h.undo(&mut b);
        h.transact(&mut b, EditKind::Paste, pos(0, 0), |b| b.insert(pos(0, 0), "b"));
        assert_eq!(h.redo(&mut b), None);
        assert_eq!(b.text(), "b");
    }

    #[test]
    fn tracks_dirty_state_relative_to_the_last_save_including_after_undo() {
        let (mut b, mut h, _) = setup("");
        assert!(!h.is_dirty());
        let mut p = h.transact(&mut b, EditKind::Typing, pos(0, 0), |b| b.insert(pos(0, 0), "a"));
        assert!(h.is_dirty());
        h.mark_saved();
        assert!(!h.is_dirty());
        // Typing right after saving must not merge into the saved step.
        p = h.transact(&mut b, EditKind::Typing, p, |b| b.insert(p, "b"));
        let _ = p;
        assert!(h.is_dirty());
        h.undo(&mut b);
        assert_eq!(b.text(), "a");
        assert!(!h.is_dirty()); // back to what's on disk
        h.undo(&mut b);
        assert!(h.is_dirty()); // now older than what's on disk
    }

    #[test]
    fn typing_right_after_an_undo_does_not_merge_into_the_older_step() {
        let (mut b, mut h, _) = setup("");
        let mut p = h.transact(&mut b, EditKind::Typing, pos(0, 0), |b| b.insert(pos(0, 0), "abc"));
        p = h.transact(&mut b, EditKind::Newline, p, |b| b.insert(p, "\n"));
        let _ = p;
        let back = h.undo(&mut b).expect("undo");
        h.transact(&mut b, EditKind::Typing, back, |b| b.insert(back, "Q"));
        assert_eq!(b.text(), "abcQ");
        h.undo(&mut b);
        assert_eq!(b.text(), "abc");
    }

    #[test]
    fn ignores_transactions_that_change_nothing() {
        let (mut b, mut h, _) = setup("a");
        h.transact(&mut b, EditKind::Delete, pos(0, 0), |_| pos(0, 0));
        assert_eq!(h.undo(&mut b), None);
        assert_eq!(b.text(), "a");
    }
}
