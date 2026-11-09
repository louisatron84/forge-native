//! Run configurations and tasks, from `<project>/.forge/run.json`:
//!
//! ```json
//! { "configurations": [
//!     { "name": "Server", "file": "app/main.py", "args": ["--port", "8000"],
//!       "env": { "DEBUG": "1" }, "cwd": "${project}" },
//!     { "name": "Flask", "module": "flask", "args": "run --debug" } ],
//!   "tasks": [ { "name": "Install requirements", "command": "python -m pip install -r requirements.txt" } ] }
//! ```
//!
//! A configuration runs a file (or a module, like `python -m`) with
//! arguments, environment variables and a working folder; Run and Debug both
//! use it. A task is any shell command, run in its own terminal.
//! `${file}`, `${fileDir}` and `${project}` are replaced in every text value.
//! Replaces `src/shared/runConfig.ts`.

use serde_json::Value;
use std::collections::BTreeMap;

pub const RUN_FILE: &str = "run.json";

#[derive(Clone, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunConfiguration {
    pub name: String,
    /// The script to run (relative to the project); default `${file}`, the open file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Run a module instead of a file (`python -m module`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    /// Working folder (relative to the project); default the project folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub name: String,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct RunFile {
    pub configurations: Vec<RunConfiguration>,
    pub tasks: Vec<Task>,
}

/// Split a command-line string into arguments, honouring "double" and 'single' quotes.
pub fn split_args(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has = false;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else if c == '\\' && q == '"' && matches!(chars.get(i + 1), Some('"' | '\\')) {
                    i += 1;
                    cur.push(chars[i]);
                } else {
                    cur.push(c);
                }
            }
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                has = true;
            }
            None if c.is_whitespace() => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                has = false;
            }
            None => cur.push(c),
        }
        i += 1;
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn text(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
}

fn env_key_ok(k: &str) -> bool {
    let mut cs = k.chars();
    cs.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && cs.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

/// Turn run.json content (anything) into valid configurations and tasks; bad entries are dropped.
pub fn normalize_run_file(raw: &Value) -> RunFile {
    let mut out = RunFile::default();
    let Some(r) = raw.as_object() else { return out };
    for c in r.get("configurations").and_then(Value::as_array).into_iter().flatten() {
        let Some(o) = c.as_object() else { continue };
        let Some(name) = text(o.get("name")) else { continue };
        if out.configurations.iter().any(|x| x.name == name) {
            continue;
        }
        let args = match o.get("args") {
            Some(Value::String(s)) => split_args(s),
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let mut env = BTreeMap::new();
        if let Some(e) = o.get("env").and_then(Value::as_object) {
            for (k, v) in e {
                let value = match v {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => continue,
                };
                if env_key_ok(k) {
                    env.insert(k.clone(), value);
                }
            }
        }
        let module = text(o.get("module"));
        let file = if module.is_some() { None } else { text(o.get("file")) };
        out.configurations.push(RunConfiguration { name, file, module, args, env, cwd: text(o.get("cwd")) });
    }
    for t in r.get("tasks").and_then(Value::as_array).into_iter().flatten() {
        let Some(o) = t.as_object() else { continue };
        let (Some(name), Some(command)) = (text(o.get("name")), text(o.get("command"))) else { continue };
        if out.tasks.iter().any(|x| x.name == name) {
            continue;
        }
        out.tasks.push(Task { name, command, cwd: text(o.get("cwd")) });
    }
    out
}

/// What `${...}` stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vars {
    pub project: String,
    /// The open file, if any.
    pub file: Option<String>,
}

fn is_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
        || p.starts_with(['/', '\\', '~'])
}

fn sep_of(root: &str) -> char {
    if root.contains('\\') {
        '\\'
    } else {
        '/'
    }
}

fn dir_of(p: &str) -> &str {
    match p.rfind(['/', '\\']) {
        Some(i) => &p[..i],
        None => p,
    }
}

/// A path relative to the project (or already absolute).
pub fn in_project(root: &str, path: &str) -> String {
    if is_absolute(path) {
        return path.to_owned();
    }
    let sep = sep_of(root);
    let rel = path.strip_prefix("./").or_else(|| path.strip_prefix(".\\")).unwrap_or(path);
    let rel: String = rel.chars().map(|c| if c == '/' || c == '\\' { sep } else { c }).collect();
    if rel == "." || rel.is_empty() {
        return root.to_owned();
    }
    format!("{}{sep}{rel}", root.trim_end_matches(['/', '\\']))
}

/// Replace `${file}`, `${fileDir}`, `${project}`; None if `${file}` is needed but no file is open.
pub fn expand(text: &str, vars: &Vars) -> Option<String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[i..]);
            return Some(out);
        };
        match &after[..end] {
            "project" => out.push_str(&vars.project),
            "file" => out.push_str(vars.file.as_deref()?),
            "fileDir" => out.push_str(dir_of(vars.file.as_deref()?)),
            other => {
                out.push_str("${");
                out.push_str(other);
                out.push('}');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// Everything needed to start a configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchSpec {
    /// The script, or None when running a module.
    pub program: Option<String>,
    pub module: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: String,
}

/// Fill in a configuration's variables and paths, or say why it can't run.
pub fn resolve_configuration(cfg: &RunConfiguration, vars: &Vars) -> Result<LaunchSpec, String> {
    let need = || format!("\"{}\" runs the open file, so open a Python file first.", cfg.name);
    let mut args = Vec::new();
    for a in &cfg.args {
        args.push(expand(a, vars).ok_or_else(need)?);
    }
    let mut env = BTreeMap::new();
    for (k, v) in &cfg.env {
        env.insert(k.clone(), expand(v, vars).ok_or_else(need)?);
    }
    let cwd_text = match &cfg.cwd {
        Some(c) => expand(c, vars).ok_or_else(need)?,
        None => vars.project.clone(),
    };
    let cwd = in_project(&vars.project, &cwd_text);
    if let Some(m) = &cfg.module {
        return Ok(LaunchSpec { program: None, module: Some(m.clone()), args, env, cwd });
    }
    let file = expand(cfg.file.as_deref().unwrap_or("${file}"), vars).filter(|f| !f.is_empty()).ok_or_else(need)?;
    Ok(LaunchSpec { program: Some(in_project(&vars.project, &file)), module: None, args, env, cwd })
}

/// A task's command and folder with variables filled in, or why it can't run.
pub fn resolve_task(task: &Task, vars: &Vars) -> Result<(String, String), String> {
    let err = || format!("\"{}\" uses the open file, so open a file first.", task.name);
    let command = expand(&task.command, vars).ok_or_else(err)?;
    let cwd = match &task.cwd {
        Some(c) => expand(c, vars).ok_or_else(err)?,
        None => vars.project.clone(),
    };
    Ok((command, in_project(&vars.project, &cwd)))
}

/// What a new run.json starts with.
pub fn starter_run_file(has_requirements: bool) -> RunFile {
    RunFile {
        configurations: vec![RunConfiguration {
            name: "Current file".into(),
            file: Some("${file}".into()),
            cwd: Some("${project}".into()),
            ..Default::default()
        }],
        tasks: vec![if has_requirements {
            Task {
                name: "Install requirements".into(),
                command: "python -m pip install -r requirements.txt".into(),
                cwd: None,
            }
        } else {
            Task { name: "Python version".into(), command: "python --version".into(), cwd: None }
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn splits_on_spaces_and_keeps_quoted_parts_together() {
        assert_eq!(split_args("run --port 8000"), ["run", "--port", "8000"]);
        assert_eq!(split_args(r#"--name "Ada Lovelace" --x 'a b' """#), ["--name", "Ada Lovelace", "--x", "a b", ""]);
        assert!(split_args("  ").is_empty());
        assert_eq!(split_args(r#""say \"hi\"""#), [r#"say "hi""#]);
    }

    #[test]
    fn keeps_valid_configurations_and_tasks() {
        let f = normalize_run_file(&json!({
            "configurations": [
                { "name": "Server", "file": "app.py", "args": ["--port", 8000], "env": { "DEBUG": 1, "bad key": "x" }, "cwd": "src" },
                { "name": "Flask", "module": "flask", "file": "ignored.py", "args": "run --debug" },
                { "name": "Server", "file": "dupe.py" },
                { "file": "no-name.py" },
                "junk"
            ],
            "tasks": [{ "name": "Lint", "command": "ruff check ." }, { "name": "No command" }, { "name": "Lint", "command": "dupe" }]
        }));
        assert_eq!(
            f.configurations,
            vec![
                RunConfiguration {
                    name: "Server".into(),
                    file: Some("app.py".into()),
                    module: None,
                    args: vec!["--port".into(), "8000".into()],
                    env: env(&[("DEBUG", "1")]),
                    cwd: Some("src".into())
                },
                RunConfiguration {
                    name: "Flask".into(),
                    file: None,
                    module: Some("flask".into()),
                    args: vec!["run".into(), "--debug".into()],
                    env: BTreeMap::new(),
                    cwd: None
                },
            ]
        );
        assert_eq!(f.tasks, vec![Task { name: "Lint".into(), command: "ruff check .".into(), cwd: None }]);
    }

    #[test]
    fn gives_empty_lists_for_garbage() {
        assert_eq!(normalize_run_file(&Value::Null), RunFile::default());
        assert_eq!(normalize_run_file(&json!({ "configurations": "x", "tasks": {} })), RunFile::default());
    }

    #[test]
    fn makes_paths_relative_to_the_project() {
        assert_eq!(in_project("/p", "src/app.py"), "/p/src/app.py");
        assert_eq!(in_project("/p", "./src"), "/p/src");
        assert_eq!(in_project("/p", "."), "/p");
        assert_eq!(in_project("/p", "/abs/x.py"), "/abs/x.py");
        assert_eq!(in_project("C:\\p", "src/app.py"), "C:\\p\\src\\app.py");
        assert_eq!(in_project("C:\\p", "D:\\x.py"), "D:\\x.py");
    }

    #[test]
    fn replaces_variables() {
        let vars = Vars { project: "/p".into(), file: Some("/p/src/a.py".into()) };
        assert_eq!(
            expand("${file} in ${fileDir} of ${project}", &vars).as_deref(),
            Some("/p/src/a.py in /p/src of /p")
        );
        let none = Vars { project: "/p".into(), file: None };
        assert_eq!(expand("${file}", &none), None);
        assert_eq!(expand("${project}/x", &none).as_deref(), Some("/p/x"));
    }

    #[test]
    fn resolves_configurations() {
        let vars = Vars { project: "/p".into(), file: Some("/p/src/a.py".into()) };
        let cfg = RunConfiguration {
            name: "S".into(),
            file: Some("app.py".into()),
            args: vec!["--in".into(), "${file}".into()],
            env: env(&[("HOME_DIR", "${project}")]),
            cwd: Some("src".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_configuration(&cfg, &vars),
            Ok(LaunchSpec {
                program: Some("/p/app.py".into()),
                module: None,
                args: vec!["--in".into(), "/p/src/a.py".into()],
                env: env(&[("HOME_DIR", "/p")]),
                cwd: "/p/src".into()
            })
        );
        let plain = RunConfiguration { name: "C".into(), ..Default::default() };
        assert_eq!(
            resolve_configuration(&plain, &vars).map(|s| (s.program, s.cwd)),
            Ok((Some("/p/src/a.py".into()), "/p".into()))
        );
        let none = Vars { project: "/p".into(), file: None };
        let module = RunConfiguration {
            name: "M".into(),
            module: Some("pytest".into()),
            args: vec!["-q".into()],
            ..Default::default()
        };
        assert_eq!(resolve_configuration(&module, &none).map(|s| s.module), Ok(Some("pytest".into())));
        assert!(resolve_configuration(&plain, &none).is_err());
    }

    #[test]
    fn resolves_tasks() {
        let t = Task { name: "T".into(), command: "python ${file}".into(), cwd: Some("src".into()) };
        let vars = Vars { project: "/p".into(), file: Some("/p/a.py".into()) };
        assert_eq!(resolve_task(&t, &vars), Ok(("python /p/a.py".into(), "/p/src".into())));
        let none = Vars { project: "/p".into(), file: None };
        let plain = Task { name: "T".into(), command: "pytest".into(), cwd: None };
        assert_eq!(resolve_task(&plain, &none), Ok(("pytest".into(), "/p".into())));
        let needs = Task { name: "T".into(), command: "x ${file}".into(), cwd: None };
        assert!(resolve_task(&needs, &none).is_err());
    }

    #[test]
    fn writes_the_old_format() {
        let v = serde_json::to_value(starter_run_file(true)).expect("json");
        assert_eq!(v["configurations"][0]["file"], "${file}");
        assert!(v["configurations"][0].get("module").is_none());
        assert_eq!(v["tasks"][0]["name"], "Install requirements");
        assert_eq!(normalize_run_file(&v), starter_run_file(true));
    }
}
