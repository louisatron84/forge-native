//! User settings (`<data>/settings.json`) and project overrides
//! (`<project>/.forge/settings.json`). Replaces `src/shared/settings.ts`.
//! Anything (a hand-edited or corrupt file) is turned into valid settings.

use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Forge's own colours.
    #[default]
    Molten,
    /// Black and white, high contrast.
    Plain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Formatter {
    /// Ruff if installed, else Black.
    #[default]
    Auto,
    Ruff,
    Black,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    Compact,
    #[default]
    Normal,
    Roomy,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub font_size: u32,
    pub theme: Theme,
    /// 2, 4 or 8.
    pub tab_size: u32,
    /// Python interpreter to use for Run; '' = automatic (project .venv, then PATH).
    pub python_path: String,
    /// Shell for new terminals (an id like 'powershell', 'gitbash'); '' = the system default.
    pub shell: String,
    /// Open the last project again when the app starts.
    pub reopen_last_project: bool,
    /// Show the Welcome page when no files are open.
    pub show_welcome: bool,
    /// Formatter for Format Document.
    pub formatter: Formatter,
    /// Format Python files when saving them.
    pub format_on_save: bool,
    /// Show Ruff's lint warnings (when Ruff is installed).
    pub lint_with_ruff: bool,
    /// Run the program being debugged in a terminal (so input() works) instead of the Debug console.
    pub debug_in_terminal: bool,
    /// Switch to the Debug task while debugging (and back afterwards).
    pub switch_to_debug_task: bool,
    /// While a program runs with the Graph in view, show where it's at.
    pub show_runs_on_graph: bool,
    /// How much fits in the panels.
    pub density: Density,
    /// Animate switching tasks and moving panels.
    pub animations: bool,
    /// Keep Forge up to date (download in the background, install on close).
    pub check_for_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_size: 14,
            theme: Theme::Molten,
            tab_size: 4,
            python_path: String::new(),
            shell: String::new(),
            reopen_last_project: true,
            show_welcome: true,
            formatter: Formatter::Auto,
            format_on_save: false,
            lint_with_ruff: true,
            debug_in_terminal: true,
            switch_to_debug_task: true,
            show_runs_on_graph: true,
            density: Density::Normal,
            animations: true,
            check_for_updates: true,
        }
    }
}

pub const FONT_SIZE_MIN: u32 = 10;
pub const FONT_SIZE_MAX: u32 = 28;

/// The project's own Forge files (settings.json, run.json...).
pub const PROJECT_DIR: &str = ".forge";
/// Where they lived before the rename to Forge (moved on first open).
pub const LEGACY_PROJECT_DIR: &str = ".ideforfun";
pub const PROJECT_SETTINGS_FILE: &str = "settings.json";

fn plain_shell_id(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

fn tab_size(v: &Value) -> Option<u32> {
    v.as_u64().filter(|n| matches!(n, 2 | 4 | 8)).map(|n| n as u32).filter(|_| v.is_u64())
}

fn formatter(v: &Value) -> Option<Formatter> {
    match v.as_str()? {
        "auto" => Some(Formatter::Auto),
        "ruff" => Some(Formatter::Ruff),
        "black" => Some(Formatter::Black),
        _ => None,
    }
}

/// Turn anything into valid settings.
pub fn normalize_settings(raw: &Value) -> Settings {
    let empty = Map::new();
    let r = raw.as_object().unwrap_or(&empty);
    let mut s = Settings::default();
    let get = |k: &str| r.get(k).unwrap_or(&Value::Null);
    if let Some(n) = get("fontSize").as_f64().filter(|n| n.is_finite()) {
        s.font_size = (n.round().max(FONT_SIZE_MIN as f64).min(FONT_SIZE_MAX as f64)) as u32;
    }
    match get("theme").as_str() {
        Some("molten") => s.theme = Theme::Molten,
        Some("plain") => s.theme = Theme::Plain,
        // Settings from before Forge: dark became Molten, light became Plain.
        Some("dark") => s.theme = Theme::Molten,
        Some("light") => s.theme = Theme::Plain,
        _ => {}
    }
    if let Some(t) = tab_size(get("tabSize")) {
        s.tab_size = t;
    }
    if let Some(p) = get("pythonPath").as_str() {
        s.python_path = p.trim().to_owned();
    }
    if let Some(sh) = get("shell").as_str().filter(|sh| plain_shell_id(sh)) {
        s.shell = sh.to_owned();
    }
    let flag = |k: &str, target: &mut bool| {
        if let Some(b) = r.get(k).and_then(Value::as_bool) {
            *target = b;
        }
    };
    flag("reopenLastProject", &mut s.reopen_last_project);
    flag("showWelcome", &mut s.show_welcome);
    if let Some(f) = formatter(get("formatter")) {
        s.formatter = f;
    }
    flag("formatOnSave", &mut s.format_on_save);
    flag("lintWithRuff", &mut s.lint_with_ruff);
    flag("debugInTerminal", &mut s.debug_in_terminal);
    flag("switchToDebugTask", &mut s.switch_to_debug_task);
    flag("showRunsOnGraph", &mut s.show_runs_on_graph);
    match get("density").as_str() {
        Some("compact") => s.density = Density::Compact,
        Some("normal") => s.density = Density::Normal,
        Some("roomy") => s.density = Density::Roomy,
        _ => {}
    }
    flag("animations", &mut s.animations);
    flag("checkForUpdates", &mut s.check_for_updates);
    s
}

/// Settings a project can override in `<project>/.forge/settings.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub python_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formatter: Option<Formatter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_on_save: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lint_with_ruff: Option<bool>,
}

pub const PROJECT_SETTING_KEYS: [&str; 6] = ["tabSize", "pythonPath", "shell", "formatter", "formatOnSave", "lintWithRuff"];

/// Only the valid project-level values from a project settings file; anything else is ignored.
pub fn normalize_project_settings(raw: &Value) -> ProjectSettings {
    let Some(r) = raw.as_object() else { return ProjectSettings::default() };
    let get = |k: &str| r.get(k).unwrap_or(&Value::Null);
    ProjectSettings {
        tab_size: tab_size(get("tabSize")),
        python_path: get("pythonPath").as_str().map(|p| p.trim().to_owned()),
        shell: get("shell").as_str().filter(|s| plain_shell_id(s)).map(str::to_owned),
        formatter: formatter(get("formatter")),
        format_on_save: get("formatOnSave").as_bool(),
        lint_with_ruff: get("lintWithRuff").as_bool(),
    }
}

/// The settings in effect: yours, with the project's overrides on top.
pub fn effective_settings(user: &Settings, project: &ProjectSettings) -> Settings {
    let mut s = user.clone();
    if let Some(t) = project.tab_size {
        s.tab_size = t;
    }
    if let Some(p) = &project.python_path {
        s.python_path = p.clone();
    }
    if let Some(sh) = &project.shell {
        s.shell = sh.clone();
    }
    if let Some(f) = project.formatter {
        s.formatter = f;
    }
    if let Some(b) = project.format_on_save {
        s.format_on_save = b;
    }
    if let Some(b) = project.lint_with_ruff {
        s.lint_with_ruff = b;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fills_in_defaults_for_missing_or_garbage_input() {
        assert_eq!(normalize_settings(&Value::Null), Settings::default());
        assert_eq!(normalize_settings(&json!("nonsense")), Settings::default());
        assert_eq!(normalize_settings(&json!({})), Settings::default());
    }

    #[test]
    fn keeps_valid_values() {
        let raw = json!({ "fontSize": 18, "theme": "plain", "tabSize": 2, "pythonPath": "C:\\Py\\python.exe", "shell": "gitbash", "reopenLastProject": false, "showWelcome": false, "formatter": "black", "formatOnSave": true, "lintWithRuff": false, "debugInTerminal": false, "switchToDebugTask": false, "showRunsOnGraph": false, "density": "compact", "animations": false, "checkForUpdates": false });
        let s = normalize_settings(&raw);
        assert_eq!(serde_json::to_value(&s).expect("json"), raw);
    }

    #[test]
    fn density_is_one_of_three_normal_by_default() {
        assert_eq!(normalize_settings(&json!({ "density": "tiny" })).density, Density::Normal);
        assert_eq!(normalize_settings(&json!({ "density": "roomy" })).density, Density::Roomy);
    }

    #[test]
    fn only_accepts_plain_shell_ids() {
        assert_eq!(normalize_settings(&json!({ "shell": "rm -rf /" })).shell, "");
    }

    #[test]
    fn clamps_the_font_size_and_rounds_it() {
        assert_eq!(normalize_settings(&json!({ "fontSize": 3 })).font_size, 10);
        assert_eq!(normalize_settings(&json!({ "fontSize": 99 })).font_size, 28);
        assert_eq!(normalize_settings(&json!({ "fontSize": 15.6 })).font_size, 16);
    }

    #[test]
    fn rejects_unknown_themes_and_tab_sizes() {
        assert_eq!(normalize_settings(&json!({ "theme": "purple" })).theme, Theme::Molten);
        assert_eq!(normalize_settings(&json!({ "tabSize": 3 })).tab_size, 4);
    }

    #[test]
    fn old_dark_and_light_themes_map_to_molten_and_plain() {
        assert_eq!(normalize_settings(&json!({ "theme": "dark" })).theme, Theme::Molten);
        assert_eq!(normalize_settings(&json!({ "theme": "light" })).theme, Theme::Plain);
        let s = normalize_settings(&json!({ "theme": "neon", "fontSize": 16 }));
        assert_eq!((s.theme, s.font_size), (Theme::Molten, 16));
    }

    #[test]
    fn trims_the_python_path_and_treats_blank_as_automatic() {
        assert_eq!(normalize_settings(&json!({ "pythonPath": "   " })).python_path, "");
        assert_eq!(normalize_settings(&json!({ "pythonPath": " py " })).python_path, "py");
    }

    #[test]
    fn project_settings_keep_only_valid_project_level_values() {
        let p = normalize_project_settings(&json!({ "tabSize": 2, "pythonPath": " .venv/bin/python ", "shell": "bash", "fontSize": 30, "theme": "light" }));
        assert_eq!(
            p,
            ProjectSettings {
                tab_size: Some(2),
                python_path: Some(".venv/bin/python".into()),
                shell: Some("bash".into()),
                ..Default::default()
            }
        );
        assert_eq!(normalize_project_settings(&json!({ "tabSize": 3, "shell": "a b" })), ProjectSettings::default());
        assert_eq!(normalize_project_settings(&json!([1])), ProjectSettings::default());
        let eff = effective_settings(&Settings::default(), &ProjectSettings { tab_size: Some(2), ..Default::default() });
        assert_eq!(eff.tab_size, 2);
    }

    #[test]
    fn switch_to_debug_task_and_show_runs_default_to_true() {
        assert!(Settings::default().switch_to_debug_task);
        assert!(!normalize_settings(&json!({ "switchToDebugTask": false })).switch_to_debug_task);
        assert!(normalize_settings(&json!({ "switchToDebugTask": "no" })).switch_to_debug_task);
        assert!(normalize_settings(&json!({ "showRunsOnGraph": 1 })).show_runs_on_graph);
    }
}
