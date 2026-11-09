//! forge-sys: file system and OS services: reading and writing text files,
//! listing folders, file operations, crash backups, the app's saved state,
//! and the `.forge` project folder.

pub mod backups;
pub mod files;
pub mod project_dir;
pub mod text_file;
pub mod user_state;
