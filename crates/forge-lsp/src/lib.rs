//! forge-lsp: the Language Server Protocol client: message framing, the
//! client, one server's document sync and requests, reading its answers,
//! completion and hover helpers, and running the server process.

pub mod client;
pub mod completion;
pub mod jsonrpc;
pub mod navigation;
pub mod process;
pub mod server;
pub mod services;
pub mod uri;
