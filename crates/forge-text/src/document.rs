//! One open file: its text, undo history, highlighting state, folded blocks,
//! encoding and line endings. Replaces `src/renderer/editor/document.ts`.
//!
//! A file open in both halves of a split view is one `Doc`; each editor view
//! keeps its own cursors and scroll position for it. After any edit the owner
//! calls `sync`, which moves folds, bumps the version and tells the
//! highlighter, and returns the changes (for the language server).

use crate::encoding::Encoding;
use crate::highlighter::Highlighter;
use crate::history::History;
use crate::lines::shift_lines;
use crate::modes::{mode_for, LanguageMode};
use crate::text_buffer::{Change, Eol, TextBuffer};
use crate::tokenizer::Token;

#[derive(Debug)]
pub struct Doc {
    path: String,
    pub buffer: TextBuffer,
    pub history: History,
    pub highlighter: Highlighter,
    /// Header lines of folded blocks (they move with edits, like breakpoints).
    pub folds: Vec<usize>,
    /// Goes up on every change to the text (cheap "has it changed since?" checks).
    pub version: u64,
    /// The file's encoding on disk (changing it makes the file "unsaved" until saved).
    pub encoding: Encoding,
    /// Big file: the language server is skipped for it to stay responsive.
    pub large: bool,
    mode: &'static LanguageMode,
    saved_encoding: Encoding,
    saved_eol: Eol,
}

impl Doc {
    pub fn new(path: impl Into<String>, text: &str) -> Self {
        Self::with_options(path, text, Encoding::Utf8, false)
    }

    pub fn with_options(path: impl Into<String>, text: &str, encoding: Encoding, large: bool) -> Self {
        let path = path.into();
        let buffer = TextBuffer::new(text);
        let saved_eol = buffer.eol;
        Self {
            mode: mode_for(&path),
            path,
            buffer,
            history: History::new(),
            highlighter: Highlighter::new(),
            folds: Vec::new(),
            version: 0,
            encoding,
            large,
            saved_encoding: encoding,
            saved_eol,
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// Rename: the language (highlighting, comments, indentation) follows the path.
    pub fn set_path(&mut self, path: impl Into<String>) {
        self.path = path.into();
        let mode = mode_for(&self.path);
        if !std::ptr::eq(mode, self.mode) {
            self.mode = mode;
            self.highlighter.reset();
        }
    }

    /// Highlighting, comments and indentation for this file's language.
    pub fn mode(&self) -> &'static LanguageMode {
        self.mode
    }

    pub fn name(&self) -> &str {
        self.path.rsplit(['/', '\\']).next().unwrap_or(&self.path)
    }

    /// Line endings used when saving.
    pub fn eol(&self) -> Eol {
        self.buffer.eol
    }

    pub fn set_eol(&mut self, eol: Eol) {
        self.buffer.eol = eol;
    }

    pub fn dirty(&self) -> bool {
        self.history.is_dirty() || self.encoding != self.saved_encoding || self.buffer.eol != self.saved_eol
    }

    pub fn text(&self) -> String {
        self.buffer.text()
    }

    pub fn mark_saved(&mut self) {
        self.history.mark_saved();
        self.saved_encoding = self.encoding;
        self.saved_eol = self.buffer.eol;
    }

    /// Apply the buffer's recorded changes to folds, version and highlighting.
    /// Returns them, in order, for whoever else needs them (the language server).
    pub fn sync(&mut self) -> Vec<Change> {
        let changes = self.buffer.take_changes();
        for change in &changes {
            self.version += 1;
            if !self.folds.is_empty() && change.text.contains('\n') {
                self.folds = shift_lines(&self.folds, change);
            }
            self.highlighter.on_change(change);
        }
        changes
    }

    /// Tokens for one line (colours).
    pub fn tokens_for(&mut self, line: usize) -> Vec<Token> {
        self.highlighter.tokens_for(&self.buffer, self.mode, line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::EditKind;
    use crate::text_buffer::pos;

    #[test]
    fn a_documents_mode_follows_its_path() {
        let mut doc = Doc::new("/proj/a.txt", "x = True");
        assert_eq!(doc.mode().language.id, "plaintext");
        assert!(doc.tokens_for(0).is_empty());
        doc.set_path("/proj/a.py");
        assert_eq!(doc.mode().language.id, "python");
        assert!(doc.tokens_for(0).iter().any(|t| t.kind.as_str() == "constant"));
    }

    #[test]
    fn dirty_follows_edits_encoding_and_line_endings() {
        let mut doc = Doc::new("a.py", "x");
        assert!(!doc.dirty());
        let Doc { buffer, history, .. } = &mut doc;
        history.transact(buffer, EditKind::Typing, pos(0, 1), |b| b.insert(pos(0, 1), "y"));
        doc.sync();
        assert!(doc.dirty());
        doc.mark_saved();
        assert!(!doc.dirty());
        doc.set_eol(Eol::CrLf);
        assert!(doc.dirty());
        doc.mark_saved();
        doc.encoding = Encoding::Latin1;
        assert!(doc.dirty());
    }

    #[test]
    fn sync_moves_folds_and_bumps_the_version() {
        let mut doc = Doc::new("a.py", "def f():\n    pass\nx = 1");
        doc.folds = vec![0];
        doc.buffer.insert(pos(0, 0), "\n");
        let changes = doc.sync();
        assert_eq!(changes.len(), 1);
        assert_eq!(doc.folds, [1]);
        assert_eq!(doc.version, 1);
    }

    #[test]
    fn name_is_the_last_path_part() {
        assert_eq!(Doc::new("C:\\p\\main.py", "").name(), "main.py");
        assert_eq!(Doc::new("/p/x/y.toml", "").name(), "y.toml");
    }
}
