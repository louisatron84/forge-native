//! Forge keeps a project's own files (settings.json, run.json...) in
//! `<project>/.forge`. Projects from before the rename have
//! `<project>/.ideforfun`: that folder is renamed the first time the project
//! is opened, or copied when it can't be renamed (on Windows a file inside it
//! may be locked). Replaces `src/main/projectDir.ts`.

use forge_core::settings::{LEGACY_PROJECT_DIR, PROJECT_DIR};
use std::io;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectDirResult {
    None,
    Migrated,
    Copied,
    /// Both exist: .forge is used, .ideforfun left alone.
    Both,
    /// Neither moving nor copying worked; nothing was changed.
    Error,
}

/// The file-system operations used (replaceable in tests).
pub struct FsOps<'a> {
    pub rename: &'a dyn Fn(&Path, &Path) -> io::Result<()>,
    pub copy_dir: &'a dyn Fn(&Path, &Path) -> io::Result<()>,
}

pub fn copy_dir_all(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir_all(&e.path(), &target)?;
        } else {
            std::fs::copy(e.path(), target)?;
        }
    }
    Ok(())
}

fn real_rename(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::rename(from, to)
}

pub fn prepare_project_dir(root: &Path) -> ProjectDirResult {
    prepare_project_dir_with(root, &FsOps { rename: &real_rename, copy_dir: &copy_dir_all })
}

pub fn prepare_project_dir_with(root: &Path, ops: &FsOps<'_>) -> ProjectDirResult {
    let current = root.join(PROJECT_DIR);
    let legacy = root.join(LEGACY_PROJECT_DIR);
    let has_legacy = legacy.is_dir();
    if current.is_dir() {
        return if has_legacy { ProjectDirResult::Both } else { ProjectDirResult::None };
    }
    if !has_legacy {
        return ProjectDirResult::None;
    }
    if (ops.rename)(&legacy, &current).is_ok() {
        return ProjectDirResult::Migrated;
    }
    // Copy into a temporary folder and only then give it the real name, so a copy that
    // fails partway never leaves a half-filled .forge that would win from then on.
    let temp = root.join(format!("{PROJECT_DIR}-copying-{}", std::process::id()));
    match (ops.copy_dir)(&legacy, &temp).and_then(|_| std::fs::rename(&temp, &current)) {
        Ok(()) => ProjectDirResult::Copied,
        Err(_) => {
            let _ = std::fs::remove_dir_all(&temp);
            ProjectDirResult::Error
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(p, text).expect("write");
    }

    #[test]
    fn renames_a_legacy_ideforfun_folder_to_forge() {
        let dir = tempfile::tempdir().expect("temp");
        put(dir.path(), ".ideforfun/settings.json", "{\"tabSize\":2}");
        assert_eq!(prepare_project_dir(dir.path()), ProjectDirResult::Migrated);
        assert!(dir.path().join(".forge/settings.json").exists());
        assert!(!dir.path().join(".ideforfun").exists());
    }

    #[test]
    fn nothing_to_do_without_a_legacy_folder_and_forge_isnt_created_eagerly() {
        let dir = tempfile::tempdir().expect("temp");
        assert_eq!(prepare_project_dir(dir.path()), ProjectDirResult::None);
        assert!(!dir.path().join(".forge").exists());
        put(dir.path(), ".forge/settings.json", "{}");
        assert_eq!(prepare_project_dir(dir.path()), ProjectDirResult::None);
    }

    #[test]
    fn uses_forge_and_leaves_ideforfun_alone_when_both_exist() {
        let dir = tempfile::tempdir().expect("temp");
        put(dir.path(), ".forge/settings.json", "{\"tabSize\":2}");
        put(dir.path(), ".ideforfun/settings.json", "{\"tabSize\":8}");
        assert_eq!(prepare_project_dir(dir.path()), ProjectDirResult::Both);
        assert_eq!(std::fs::read_to_string(dir.path().join(".forge/settings.json")).expect("read"), "{\"tabSize\":2}");
        assert!(dir.path().join(".ideforfun/settings.json").exists());
    }

    #[test]
    fn copies_when_the_rename_fails_and_keeps_the_original() {
        let dir = tempfile::tempdir().expect("temp");
        put(dir.path(), ".ideforfun/run.json", "{\"configurations\":[]}");
        let fail = |_: &Path, _: &Path| -> io::Result<()> { Err(io::Error::other("busy")) };
        let res = prepare_project_dir_with(dir.path(), &FsOps { rename: &fail, copy_dir: &copy_dir_all });
        assert_eq!(res, ProjectDirResult::Copied);
        assert!(dir.path().join(".forge/run.json").exists());
        assert!(dir.path().join(".ideforfun/run.json").exists());
    }

    #[test]
    fn reports_an_error_and_leaves_no_half_copied_forge_when_the_copy_fails_too() {
        let dir = tempfile::tempdir().expect("temp");
        put(dir.path(), ".ideforfun/settings.json", "{}");
        put(dir.path(), ".ideforfun/run.json", "{}");
        let fail = |_: &Path, _: &Path| -> io::Result<()> { Err(io::Error::other("busy")) };
        let partial = |_: &Path, to: &Path| -> io::Result<()> {
            std::fs::create_dir_all(to)?;
            std::fs::write(to.join("settings.json"), "{}")?; // got partway
            Err(io::Error::other("busy"))
        };
        let res = prepare_project_dir_with(dir.path(), &FsOps { rename: &fail, copy_dir: &partial });
        assert_eq!(res, ProjectDirResult::Error);
        assert!(!dir.path().join(".forge").exists());
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .expect("dir")
            .map(|e| e.expect("e").file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, [".ideforfun"]);
    }
}
