//! forge-term: Forge's integrated terminals: the installed shells, the
//! pseudo-terminal processes, the screen model and the keys they get.

pub mod keys;
pub mod screen;
pub mod shells;
pub mod terminal;

pub use terminal::{StartOptions, TermEvent, Terminal};
