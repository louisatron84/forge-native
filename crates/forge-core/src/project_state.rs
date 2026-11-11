//! What gets remembered between sessions: the recently opened folders, and
//! for each project its tabs, split layout, cursor positions, terminals and
//! panel. Replaces `src/shared/projectState.ts`. Stored as JSON in the app's
//! data folder (never inside the project).

use serde_json::Value;

pub const MAX_RECENT: usize = 12;
pub const TASKS: [&str; 4] = ["code", "connect", "debug", "run"];

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocState {
    pub path: String,
    pub line: u64,
    pub col: u64,
    pub scroll_top: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupState {
    pub tabs: Vec<DocState>,
    /// Path of the tab that was showing.
    pub active: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedBreakpointState {
    pub path: String,
    pub line: u64,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hit_condition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceState {
    pub groups: Vec<GroupState>,
    pub active_group: usize,
    /// Shell id of each open terminal ('' = default shell).
    pub terminals: Vec<String>,
    /// Bottom panel view that was open, or None if the panel was closed.
    pub panel: Option<String>,
    /// The task the project was in (code, connect, debug, run).
    pub task: String,
    /// The run configuration picked for Run / Debug (None = the open file).
    pub run_config: Option<String>,
    pub breakpoints: Vec<SavedBreakpointState>,
    /// Debugger watch expressions.
    pub watches: Vec<String>,
    /// Which exceptions pause the debugger (None = the default: uncaught).
    pub exception_filters: Option<Vec<String>>,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        normalize_workspace_state(&Value::Null)
    }
}

fn is_win_path(p: &str) -> bool {
    let b = p.as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':') || p.starts_with("\\\\")
}

/// A path as compared for "the same folder": Windows paths ignore case and slash direction.
pub fn path_key(p: &str) -> String {
    if is_win_path(p) {
        p.replace('/', "\\").trim_end_matches('\\').to_lowercase()
    } else {
        p.trim_end_matches('/').to_owned()
    }
}

/// Put `path` first in the recent list (moving it if already there), keeping at most MAX_RECENT.
pub fn add_recent(list: &[String], path: &str) -> Vec<String> {
    let key = path_key(path);
    std::iter::once(path.to_owned())
        .chain(list.iter().filter(|p| path_key(p) != key).cloned())
        .take(MAX_RECENT)
        .collect()
}

pub fn remove_recent(list: &[String], path: &str) -> Vec<String> {
    let key = path_key(path);
    list.iter().filter(|p| path_key(p) != key).cloned().collect()
}

pub fn normalize_recent(raw: &Value) -> Vec<String> {
    let Some(items) = raw.as_array() else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    for p in items.iter().filter_map(Value::as_str) {
        if !p.trim().is_empty() && !out.iter().any(|q| path_key(q) == path_key(p)) {
            out.push(p.to_owned());
        }
    }
    out.truncate(MAX_RECENT);
    out
}

fn int(v: Option<&Value>) -> u64 {
    v.and_then(Value::as_f64).filter(|f| f.is_finite()).map_or(0, |f| f.floor().max(0.0) as u64)
}

fn plain(s: &str, allow_empty: bool, allow_dot: bool) -> bool {
    (allow_empty || !s.is_empty())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || (allow_dot && c == '.'))
}

/// Turn a stored (possibly old or hand-edited) workspace state into a valid one.
pub fn normalize_workspace_state(raw: &Value) -> WorkspaceState {
    let empty = serde_json::Map::new();
    let r = raw.as_object().unwrap_or(&empty);
    let mut groups = Vec::new();
    if let Some(gs) = r.get("groups").and_then(Value::as_array) {
        for g in gs.iter().take(2) {
            let Some(gr) = g.as_object() else { continue };
            let mut tabs = Vec::new();
            if let Some(ts) = gr.get("tabs").and_then(Value::as_array) {
                for t in ts.iter().take(50) {
                    let Some(tr) = t.as_object() else { continue };
                    let Some(path) = tr.get("path").and_then(Value::as_str).filter(|p| !p.is_empty()) else { continue };
                    tabs.push(DocState {
                        path: path.to_owned(),
                        line: int(tr.get("line")),
                        col: int(tr.get("col")),
                        scroll_top: int(tr.get("scrollTop")),
                    });
                }
            }
            let active = gr
                .get("active")
                .and_then(Value::as_str)
                .filter(|a| tabs.iter().any(|t| t.path == *a))
                .map(str::to_owned)
                .or_else(|| tabs.first().map(|t| t.path.clone()));
            groups.push(GroupState { tabs, active });
        }
    }
    if groups.is_empty() {
        groups.push(GroupState::default());
    }
    let terminals = r
        .get("terminals")
        .and_then(Value::as_array)
        .map(|ts| {
            ts.iter().filter_map(Value::as_str).filter(|t| plain(t, true, true)).take(10).map(str::to_owned).collect()
        })
        .unwrap_or_default();
    let breakpoints = r
        .get("breakpoints")
        .and_then(Value::as_array)
        .map(|bs| {
            bs.iter()
                .take(2000)
                .filter_map(|b| {
                    let o = b.as_object()?;
                    let path = o.get("path")?.as_str().filter(|p| !p.is_empty())?;
                    let line = o.get("line")?.as_u64()?;
                    let text =
                        |k: &str| o.get(k).and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_owned);
                    Some(SavedBreakpointState {
                        path: path.to_owned(),
                        line,
                        enabled: o.get("enabled").and_then(Value::as_bool) != Some(false),
                        condition: text("condition"),
                        hit_condition: text("hitCondition"),
                        log_message: text("logMessage"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    WorkspaceState {
        active_group: (int(r.get("activeGroup")) as usize).min(groups.len() - 1),
        groups,
        terminals,
        panel: r.get("panel").and_then(Value::as_str).filter(|p| plain(p, false, false)).map(str::to_owned),
        task: r.get("task").and_then(Value::as_str).filter(|t| TASKS.contains(t)).unwrap_or("code").to_owned(),
        run_config: r.get("runConfig").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned),
        breakpoints,
        watches: r
            .get("watches")
            .and_then(Value::as_array)
            .map(|ws| {
                ws.iter()
                    .filter_map(Value::as_str)
                    .filter(|w| !w.trim().is_empty())
                    .take(100)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        exception_filters: r.get("exceptionFilters").and_then(Value::as_array).map(|fs| {
            fs.iter().filter_map(Value::as_str).filter(|f| plain(f, false, false)).map(str::to_owned).collect()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn puts_the_newest_first_without_duplicates() {
        let mut list = add_recent(&[], "/a");
        list = add_recent(&list, "/b");
        list = add_recent(&list, "/a/");
        assert_eq!(list, ["/a/", "/b"]);
    }

    #[test]
    fn treats_windows_paths_as_the_same_regardless_of_case_and_slashes() {
        let list = add_recent(&["C:\\Code\\App".into(), "D:\\x".into()], "c:/code/app");
        assert_eq!(list, ["c:/code/app", "D:\\x"]);
        assert_eq!(remove_recent(&list, "C:\\CODE\\APP"), ["D:\\x"]);
    }

    #[test]
    fn keeps_at_most_max_recent() {
        let mut list: Vec<String> = Vec::new();
        for i in 0..MAX_RECENT + 5 {
            list = add_recent(&list, &format!("/p{i}"));
        }
        assert_eq!(list.len(), MAX_RECENT);
        assert_eq!(list[0], format!("/p{}", MAX_RECENT + 4));
    }

    #[test]
    fn cleans_a_stored_list() {
        assert_eq!(normalize_recent(&json!(["/a", 3, "", "/a", "/b"])), ["/a", "/b"]);
        assert!(normalize_recent(&json!("x")).is_empty());
    }

    #[test]
    fn gives_an_empty_layout_for_garbage() {
        let s = normalize_workspace_state(&Value::Null);
        assert_eq!(s.groups, [GroupState::default()]);
        assert_eq!(
            (s.active_group, s.task.as_str(), s.panel.clone(), s.exception_filters.clone()),
            (0, "code", None, None)
        );
        assert!(s.terminals.is_empty() && s.breakpoints.is_empty() && s.watches.is_empty() && s.run_config.is_none());
    }

    #[test]
    fn keeps_valid_tabs_and_fixes_the_rest() {
        let s = normalize_workspace_state(&json!({
            "groups": [
                { "tabs": [{ "path": "/a.py", "line": 3.7, "col": -1, "scrollTop": 40 }, { "nope": 1 }], "active": "/missing.py" },
                { "tabs": [{ "path": "/b.py" }], "active": "/b.py" },
                { "tabs": [{ "path": "/c.py" }] }
            ],
            "activeGroup": 9,
            "terminals": ["powershell", "bad id;", ""],
            "panel": "terminal"
        }));
        assert_eq!(
            s.groups,
            [
                GroupState {
                    tabs: vec![DocState { path: "/a.py".into(), line: 3, col: 0, scroll_top: 40 }],
                    active: Some("/a.py".into())
                },
                GroupState {
                    tabs: vec![DocState { path: "/b.py".into(), ..Default::default() }],
                    active: Some("/b.py".into())
                },
            ]
        );
        assert_eq!(s.active_group, 1);
        assert_eq!(s.terminals, ["powershell", ""]);
        assert_eq!(s.panel.as_deref(), Some("terminal"));
    }

    #[test]
    fn task_defaults_to_code_and_keeps_a_valid_task() {
        assert_eq!(normalize_workspace_state(&json!({})).task, "code");
        assert_eq!(normalize_workspace_state(&json!({ "task": "debug" })).task, "debug");
        assert_eq!(normalize_workspace_state(&json!({ "task": "party" })).task, "code");
    }

    #[test]
    fn breakpoints_and_watches_are_cleaned() {
        let s = normalize_workspace_state(&json!({
            "breakpoints": [{ "path": "/a.py", "line": 4, "condition": " x > 1 ", "enabled": false }, { "path": "", "line": 1 }, { "path": "/b.py", "line": -2 }],
            "watches": ["x", " ", 3],
            "exceptionFilters": ["raised", "bad id!"]
        }));
        assert_eq!(s.breakpoints.len(), 1);
        assert!(!s.breakpoints[0].enabled);
        assert_eq!(s.breakpoints[0].condition.as_deref(), Some(" x > 1 "));
        assert_eq!(s.watches, ["x"]);
        assert_eq!(s.exception_filters, Some(vec!["raised".to_owned()]));
    }
}
