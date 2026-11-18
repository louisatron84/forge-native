//! Small JSON files the app keeps between sessions (in its data folder):
//!   settings.json         your settings
//!   keybindings.json      your keyboard shortcut changes
//!   layouts.json          your workspace arrangement for each task
//!   recent.json           recently opened folders
//!   workspaces/<id>.json  each project's tabs, layout and terminals
//! and the project's own `<project>/.forge/settings.json`, run.json and boxes.json.
//! Replaces `src/main/userState.ts`.

use crate::backups::backup_name;
use forge_core::keybindings::{keybindings_json, normalize_keybindings, KeyOverrides};
use forge_core::project_state::{
    add_recent, normalize_recent, normalize_workspace_state, remove_recent, WorkspaceState,
};
use forge_core::settings::{
    normalize_project_settings, normalize_settings, ProjectSettings, Settings, PROJECT_DIR, PROJECT_SETTINGS_FILE,
};
use serde_json::Value;
use std::io;
use std::path::{Path, PathBuf};

/// Parsed JSON, or why it couldn't be read.
#[derive(Debug)]
pub enum JsonFile {
    Missing,
    Value(Value),
    /// The file exists but couldn't be read or isn't valid JSON.
    Error(String),
}

impl JsonFile {
    pub fn value(&self) -> Value {
        match self {
            JsonFile::Value(v) => v.clone(),
            _ => Value::Null,
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            JsonFile::Error(e) => Some(e),
            _ => None,
        }
    }
}

pub fn read_json(path: &Path) -> JsonFile {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return JsonFile::Missing,
        Err(e) => return JsonFile::Error(e.to_string()),
    };
    if text.trim().is_empty() {
        return JsonFile::Value(Value::Object(Default::default()));
    }
    match serde_json::from_str(&text) {
        Ok(v) => JsonFile::Value(v),
        Err(e) => JsonFile::Error(format!("It isn't valid JSON: {e}")),
    }
}

pub fn write_json(path: &Path, value: &Value) -> io::Result<()> {
    use std::io::Write;
    crate::file_write::replace_with(path, |file| {
        let mut writer = io::BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, value).map_err(io::Error::other)?;
        writer.write_all(b"\n")?;
        writer.flush()
    })
}

/// Forge's data folder: `%APPDATA%\Forge` on Windows, `~/.config/forge` on
/// Linux (`FORGE_USER_DATA` overrides it, for tests).
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("FORGE_USER_DATA") {
        return PathBuf::from(dir);
    }
    let base = dirs::config_dir().unwrap_or_else(std::env::temp_dir);
    base.join(if cfg!(windows) { "Forge" } else { "forge" })
}

/// Where the Electron app kept its data (to import on first run).
pub fn legacy_data_dir() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("ideforfun"))
}

pub struct LegacyFile {
    pub relative: String,
    pub contents: Vec<u8>,
}

/// Collect importable Electron files for review. Source files are untouched
/// and existing native settings always win.
pub fn plan_legacy_import(legacy: &Path, native: &Path) -> io::Result<Option<Vec<LegacyFile>>> {
    let marker = native.join(".electron-imported");
    if marker.try_exists()? || !legacy.is_dir() || legacy == native {
        return Ok(None);
    }
    let todo = ["settings.json", "keybindings.json", "layouts.json", "recent.json", "workspaces", "templates"]
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    Ok(Some(plan_legacy_files(legacy, native, todo)?))
}

pub(crate) fn plan_legacy_files(legacy: &Path, native: &Path, mut todo: Vec<PathBuf>) -> io::Result<Vec<LegacyFile>> {
    use std::io::Read;
    let mut files = Vec::new();
    let canonical_legacy = std::fs::canonicalize(legacy)?;
    let mut total = 0usize;
    let mut entries = 0usize;
    while let Some(relative) = todo.pop() {
        entries += 1;
        if entries > 10_000 {
            return Err(io::Error::other("Too many files in Electron settings"));
        }
        let source = legacy.join(&relative);
        let metadata = match std::fs::symlink_metadata(&source) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        // Canonical checks also cover Windows directory junctions.
        if !std::fs::canonicalize(&source)?.starts_with(&canonical_legacy) {
            continue;
        }
        if metadata.is_dir() {
            if relative.components().count() > 32 {
                return Err(io::Error::other("Electron template nesting is too deep"));
            }
            for entry in std::fs::read_dir(source)? {
                todo.push(relative.join(entry?.file_name()));
            }
        } else if metadata.is_file() {
            let rel = relative.to_string_lossy().replace('\\', "/");
            let target = if native.exists() { crate::connect::project_path(native, &rel)? } else { native.join(&rel) };
            if std::fs::symlink_metadata(&target).is_ok() {
                continue;
            }
            let mut bytes = Vec::new();
            std::fs::File::open(source)?.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
            total += bytes.len();
            if bytes.len() > 16 * 1024 * 1024 || total > 64 * 1024 * 1024 {
                return Err(io::Error::other("Electron settings exceed the import size limit"));
            }
            files.push(LegacyFile { relative: rel, contents: bytes });
        }
    }
    files.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(files)
}

pub fn finish_legacy_import(native: &Path) -> io::Result<()> {
    let marker = native.join(".electron-imported");
    match crate::file_write::create(&marker, "Electron settings imported\n") {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        result => result,
    }
}

pub struct UserState {
    dir: PathBuf,
}

impl UserState {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn settings_path(&self) -> PathBuf {
        self.dir.join("settings.json")
    }

    /// Your settings (a missing or broken file gives the defaults; the error is returned too).
    pub fn settings(&self) -> (Settings, Option<String>) {
        let f = read_json(&self.settings_path());
        (normalize_settings(&f.value()), f.error().map(str::to_owned))
    }

    pub fn save_settings(&self, settings: &Settings) -> io::Result<()> {
        write_json(&self.settings_path(), &serde_json::to_value(settings).map_err(io::Error::other)?)
    }

    pub fn keybindings_path(&self) -> PathBuf {
        self.dir.join("keybindings.json")
    }

    pub fn keybindings(&self) -> (KeyOverrides, Option<String>) {
        let f = read_json(&self.keybindings_path());
        (normalize_keybindings(&f.value()), f.error().map(str::to_owned))
    }

    /// Save shortcut changes ([] is written as null: "no shortcut").
    pub fn save_keybindings(&self, overrides: &KeyOverrides) -> io::Result<()> {
        write_json(&self.keybindings_path(), &keybindings_json(overrides))
    }

    /// Create keybindings.json if needed (for editing it by hand).
    pub fn ensure_keybindings_file(&self) -> io::Result<PathBuf> {
        let path = self.keybindings_path();
        if matches!(read_json(&path), JsonFile::Missing) {
            write_json(&path, &Value::Object(Default::default()))?;
        }
        Ok(path)
    }

    /// Workspace arrangements per task (normalized by the dock model).
    pub fn layouts(&self) -> Value {
        read_json(&self.dir.join("layouts.json")).value()
    }

    pub fn save_layouts(&self, layouts: &Value) -> io::Result<()> {
        write_json(&self.dir.join("layouts.json"), layouts)
    }

    fn recent_path(&self) -> PathBuf {
        self.dir.join("recent.json")
    }

    pub fn recent(&self) -> Vec<String> {
        normalize_recent(&read_json(&self.recent_path()).value())
    }

    pub fn add_recent(&self, folder: &str) -> io::Result<Vec<String>> {
        let list = add_recent(&self.recent(), folder);
        write_json(&self.recent_path(), &Value::from(list.clone()))?;
        Ok(list)
    }

    pub fn remove_recent(&self, folder: &str) -> io::Result<Vec<String>> {
        let list = remove_recent(&self.recent(), folder);
        write_json(&self.recent_path(), &Value::from(list.clone()))?;
        Ok(list)
    }

    pub fn clear_recent(&self) -> io::Result<()> {
        write_json(&self.recent_path(), &Value::Array(Vec::new()))
    }

    fn workspace_path(&self, root: &str) -> PathBuf {
        self.dir.join("workspaces").join(backup_name(root))
    }

    pub fn workspace(&self, root: &str) -> Option<WorkspaceState> {
        match read_json(&self.workspace_path(root)) {
            JsonFile::Value(v) => Some(normalize_workspace_state(&v)),
            _ => None,
        }
    }

    pub fn save_workspace(&self, root: &str, state: &WorkspaceState) -> io::Result<()> {
        let path = self.workspace_path(root);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::file_write::replace(&path, serde_json::to_vec(state).map_err(io::Error::other)?)
    }
}

pub fn project_settings_path(root: &Path) -> PathBuf {
    root.join(PROJECT_DIR).join(PROJECT_SETTINGS_FILE)
}

/// The project's settings overrides; an unreadable file is reported, not fatal.
pub struct ProjectSettingsFile {
    pub settings: ProjectSettings,
    pub path: PathBuf,
    pub exists: bool,
    pub error: Option<String>,
}

pub fn read_project_settings(root: &Path) -> ProjectSettingsFile {
    let path = project_settings_path(root);
    let f = read_json(&path);
    ProjectSettingsFile {
        settings: normalize_project_settings(&f.value()),
        exists: !matches!(f, JsonFile::Missing),
        error: f.error().map(str::to_owned),
        path,
    }
}

/// Files the app may read or create in `<project>/.forge` (nothing else).
pub const PROJECT_FILES: [&str; 2] = ["run.json", "boxes.json"];

fn project_file_path(root: &Path, name: &str) -> io::Result<PathBuf> {
    if !PROJECT_FILES.contains(&name) {
        return Err(io::Error::other(format!("Not a project file: {name}")));
    }
    crate::connect::project_path(root, &format!("{PROJECT_DIR}/{name}"))
}

/// A project file's JSON (missing / unreadable are reported).
pub fn read_project_file(root: &Path, name: &str) -> io::Result<(JsonFile, PathBuf)> {
    let path = project_file_path(root, name)?;
    Ok((read_json(&path), path))
}

pub fn ensure_project_file(root: &Path, name: &str, initial: &Value) -> io::Result<PathBuf> {
    let path = project_file_path(root, name)?;
    if matches!(read_json(&path), JsonFile::Missing) {
        write_json(&path, initial)?;
    }
    Ok(path)
}

/// Replace a project file's JSON (only the allowed names).
pub fn write_project_file(root: &Path, name: &str, value: &Value) -> io::Result<PathBuf> {
    let path = project_file_path(root, name)?;
    write_json(&path, value)?;
    Ok(path)
}

/// Change some values in the project settings file, keeping everything else
/// in it (`null` removes a key).
pub fn update_project_settings(root: &Path, patch: &serde_json::Map<String, Value>) -> io::Result<()> {
    let path = crate::connect::project_path(root, &format!("{PROJECT_DIR}/{PROJECT_SETTINGS_FILE}"))?;
    let f = read_json(&path);
    if let Some(e) = f.error() {
        return Err(io::Error::other(format!("{} can't be updated: {e}", path.display())));
    }
    let mut current = match f.value() {
        Value::Object(m) => m,
        _ => Default::default(),
    };
    for (k, v) in patch {
        if v.is_null() {
            current.remove(k);
        } else {
            current.insert(k.clone(), v.clone());
        }
    }
    write_json(&path, &Value::Object(current))
}

/// Create the project settings file (with `initial` in it) if it doesn't exist yet.
pub fn ensure_project_settings(root: &Path, initial: &ProjectSettings) -> io::Result<PathBuf> {
    let path = crate::connect::project_path(root, &format!("{PROJECT_DIR}/{PROJECT_SETTINGS_FILE}"))?;
    if matches!(read_json(&path), JsonFile::Missing) {
        write_json(&path, &serde_json::to_value(initial).map_err(io::Error::other)?)?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_saves_refuse_hardlinks_and_leave_both_names_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("private.txt");
        let settings = dir.path().join("settings.json");
        std::fs::write(&victim, "keep me").unwrap();
        std::fs::hard_link(&victim, &settings).unwrap();
        let error = write_json(&settings, &json!({"fontSize": 14})).unwrap_err();
        assert!(error.to_string().contains("multiple hard links"));
        assert_eq!(std::fs::read_to_string(victim).unwrap(), "keep me");
        assert_eq!(std::fs::read_to_string(settings).unwrap(), "keep me");
    }

    #[test]
    fn settings_save_updates_the_target_and_keeps_the_symlink() {
        #[cfg(unix)]
        use std::os::unix::fs::symlink as symlink_file;
        #[cfg(windows)]
        use std::os::windows::fs::symlink_file;

        let dir = tempfile::tempdir().unwrap();
        let state = UserState::new(dir.path());
        let target = dir.path().join("custom-settings.json");
        std::fs::write(&target, "{}").unwrap();
        if let Err(error) = symlink_file("custom-settings.json", state.settings_path()) {
            #[cfg(windows)]
            if error.raw_os_error() == Some(1314) {
                eprintln!("File symlinks are disabled for this Windows user");
                return;
            }
            panic!("Couldn't create settings symlink: {error}");
        }
        let settings = Settings { font_size: 20, ..Settings::default() };
        state.save_settings(&settings).unwrap();
        assert_eq!(std::fs::read_link(state.settings_path()).unwrap(), Path::new("custom-settings.json"));
        assert_eq!(read_json(&target).value()["fontSize"], 20);
    }

    #[test]
    fn settings_round_trip_and_broken_files_give_defaults_with_an_error() {
        let dir = tempfile::tempdir().expect("temp");
        let us = UserState::new(dir.path());
        assert_eq!(us.settings(), (Settings::default(), None));
        let s = Settings { font_size: 18, ..Default::default() };
        us.save_settings(&s).expect("save");
        assert_eq!(us.settings().0, s);
        std::fs::write(us.settings_path(), "{ nope").expect("write");
        let (s2, err) = us.settings();
        assert_eq!(s2, Settings::default());
        assert!(err.is_some_and(|e| e.contains("valid JSON")));
    }

    #[test]
    fn recent_folders_and_workspaces() {
        let dir = tempfile::tempdir().expect("temp");
        let us = UserState::new(dir.path());
        us.add_recent("/a").expect("add");
        us.add_recent("/b").expect("add");
        assert_eq!(us.recent(), ["/b", "/a"]);
        us.remove_recent("/b").expect("remove");
        assert_eq!(us.recent(), ["/a"]);
        assert!(us.workspace("/a").is_none());
        let ws = WorkspaceState { task: "debug".into(), ..Default::default() };
        us.save_workspace("/a", &ws).expect("save");
        assert_eq!(us.workspace("/a").map(|w| w.task), Some("debug".to_owned()));
    }

    #[test]
    fn keybindings_are_written_like_the_old_app() {
        let dir = tempfile::tempdir().expect("temp");
        let us = UserState::new(dir.path());
        let mut o = KeyOverrides::new();
        o.insert("run.file".into(), vec!["Ctrl+R".into()]);
        o.insert("nav.back".into(), vec![]);
        us.save_keybindings(&o).expect("save");
        assert_eq!(read_json(&us.keybindings_path()).value(), json!({ "nav.back": null, "run.file": "Ctrl+R" }));
        assert_eq!(us.keybindings().0, o);
    }

    #[test]
    fn project_files_only_the_allowed_names() {
        let dir = tempfile::tempdir().expect("temp");
        let root = dir.path();
        assert!(read_project_file(root, "secrets.json").is_err());
        let p = ensure_project_file(root, "run.json", &json!({ "configurations": [] })).expect("ensure");
        assert!(p.ends_with(".forge/run.json") || p.ends_with(".forge\\run.json"));
        write_project_file(root, "run.json", &json!({ "configurations": [1] })).expect("write");
        assert_eq!(read_project_file(root, "run.json").expect("read").0.value(), json!({ "configurations": [1] }));
    }

    #[test]
    fn updating_project_settings_keeps_other_values() {
        let dir = tempfile::tempdir().expect("temp");
        let root = dir.path();
        write_json(&project_settings_path(root), &json!({ "tabSize": 2, "note": "mine" })).expect("write");
        let mut patch = serde_json::Map::new();
        patch.insert("pythonPath".into(), json!(".venv/bin/python"));
        patch.insert("tabSize".into(), Value::Null);
        update_project_settings(root, &patch).expect("update");
        assert_eq!(
            read_json(&project_settings_path(root)).value(),
            json!({ "note": "mine", "pythonPath": ".venv/bin/python" })
        );
        let ps = read_project_settings(root);
        assert!(ps.exists);
        assert_eq!(ps.settings.python_path.as_deref(), Some(".venv/bin/python"));
    }
}
