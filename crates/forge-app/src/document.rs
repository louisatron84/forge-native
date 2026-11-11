//! An open file as a GPUI entity: one per file, shared by every editor view
//! showing it (both sides of a split). Edits are announced with
//! `DocumentEvent::Edited`, so other views move their cursors along.

use forge_text::{Change, Doc, Encoding};
use gpui_kit::{Context, EntityId, EventEmitter};

pub struct Document {
    pub doc: Doc,
}

#[derive(Clone, Debug)]
pub enum DocumentEvent {
    /// The text changed. `origin`: the editor view that made the edit (it has
    /// already placed its own cursors).
    Edited { origin: Option<EntityId>, changes: Vec<Change> },
    /// Saved, renamed or otherwise changed without a text edit.
    Changed,
}

impl EventEmitter<DocumentEvent> for Document {}

impl Document {
    pub fn new(path: impl Into<String>, text: &str) -> Self {
        Self { doc: Doc::new(path, text) }
    }

    /// A file read from disk in `encoding`; `large` files skip the language server.
    pub fn with_options(path: impl Into<String>, text: &str, encoding: Encoding, large: bool) -> Self {
        Self { doc: Doc::with_options(path, text, encoding, large) }
    }

    /// Run `edit` on the document, then tell everyone what changed.
    pub fn edit<R>(&mut self, origin: Option<EntityId>, cx: &mut Context<Self>, edit: impl FnOnce(&mut Doc) -> R) -> R {
        let result = edit(&mut self.doc);
        let changes = self.doc.sync();
        if !changes.is_empty() {
            cx.emit(DocumentEvent::Edited { origin, changes });
            cx.notify();
        }
        result
    }
}
