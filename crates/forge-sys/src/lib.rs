//! forge-sys: file system and OS services: reading and writing text files,
//! listing folders, file operations, crash backups, the app's saved state,
//! and the `.forge` project folder.

pub mod backups;
pub mod connect;
pub mod file_write;
pub mod files;
pub mod generated_history;
pub mod git;
pub mod openapi;
pub mod process;
pub mod project_dir;
pub mod search;
pub mod templates;
pub mod text_file;
pub mod user_state;
