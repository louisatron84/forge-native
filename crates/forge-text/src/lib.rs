//! forge-text: the editor model (buffer, cursors, selection, undo, pairs,
//! folding, find, tokenizers and highlighting, documents). No UI; `forge-app`
//! draws it.

pub mod chars;
pub mod commands;
pub mod config_tokenizer;
pub mod document;
pub mod encoding;
pub mod find;
pub mod folding;
pub mod highlighter;
pub mod history;
pub mod json_tokenizer;
pub mod languages;
pub mod lines;
pub mod modes;
pub mod multi_cursor;
pub mod pairs;
pub mod selection;
pub mod semantic;
pub mod text_buffer;
pub mod tokenizer;

pub use document::Doc;
pub use encoding::Encoding;
pub use history::{EditKind, History};
pub use selection::Selection;
pub use text_buffer::{pos, Change, ChangeKind, Eol, Pos, TextBuffer};
pub use tokenizer::{LineState, Token, TokenType};
