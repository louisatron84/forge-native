//! Review migration from the legacy project settings directory to .forge.

use crate::user_state::{plan_legacy_files, LegacyFile};
use forge_core::settings::{LEGACY_PROJECT_DIR, PROJECT_DIR};
use std::{
    io,
    path::{Path, PathBuf},
};

pub fn plan_legacy_project_import(root: &Path) -> io::Result<Option<Vec<LegacyFile>>> {
    let legacy = root.join(LEGACY_PROJECT_DIR);
    let native = root.join(PROJECT_DIR);
    if native.try_exists()? || !legacy.is_dir() {
        return Ok(None);
    }
    if !std::fs::canonicalize(&legacy)?.starts_with(std::fs::canonicalize(root)?) {
        return Err(io::Error::other("Legacy settings point outside the project."));
    }
    let files = std::fs::read_dir(&legacy)?
        .map(|entry| entry.map(|entry| PathBuf::from(entry.file_name())))
        .collect::<io::Result<_>>()?;
    Ok(Some(plan_legacy_files(&legacy, &native, files)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn legacy_project_files_are_planned_without_writing_to_either_directory() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join(LEGACY_PROJECT_DIR);
        fs::create_dir_all(legacy.join("nested")).unwrap();
        fs::write(legacy.join("nested/run.json"), "{}").unwrap();
        let plan = plan_legacy_project_import(dir.path()).unwrap().unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].relative, "nested/run.json");
        assert_eq!(plan[0].contents, b"{}");
        assert!(!dir.path().join(PROJECT_DIR).exists());
        assert!(legacy.join("nested/run.json").exists());
    }

    #[test]
    fn project_import_keeps_existing_native_settings() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(LEGACY_PROJECT_DIR)).unwrap();
        fs::create_dir(dir.path().join(PROJECT_DIR)).unwrap();
        assert!(plan_legacy_project_import(dir.path()).unwrap().is_none());
    }
}
