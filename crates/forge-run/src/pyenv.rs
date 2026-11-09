//! Python environments: the interpreters on this machine and in the project
//! (virtualenvs), what one is (version, is it a venv), its installed
//! packages, and the project's requirements compared with them. These run
//! programs and wait for them: call them off the UI thread. Replaces
//! `src/main/pyenv.ts`.

use crate::python::{is_bare_command, python_candidates};
use crate::runner::hide_window;
use forge_core::deps::{
    missing_packages, normalize_name, parse_pip_list, parse_pyproject_dependencies, parse_requirements_txt,
    InstalledPackage, Requirement,
};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterpreterInfo {
    pub path: String,
    pub version: String,
    pub is_venv: bool,
    pub prefix: String,
}

/// "Python 3.12.1 (.venv)" or "Python 3.12.1".
pub fn interpreter_label(info: &InterpreterInfo) -> String {
    let env = if info.is_venv {
        let name = info.prefix.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or("");
        format!(" ({name})")
    } else {
        String::new()
    };
    format!("Python {}{env}", info.version)
}

pub struct Output {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Run a program and wait (at most `timeout`) for it.
pub fn run(cmd: &str, args: &[&str], timeout: Duration) -> Output {
    let mut c = Command::new(cmd);
    c.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    hide_window(&mut c);
    let mut child = match c.spawn() {
        Ok(ch) => ch,
        Err(e) => return Output { ok: false, stdout: String::new(), stderr: e.to_string() },
    };
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_t = std::thread::spawn(move || {
        let mut s = Vec::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_end(&mut s);
        }
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = Vec::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_end(&mut s);
        }
        s
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                break None;
            }
        }
    };
    let stdout = String::from_utf8_lossy(&out_t.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_t.join().unwrap_or_default()).into_owned();
    let ok = status.is_some_and(|s| s.success());
    Output { ok, stdout, stderr: if stderr.is_empty() && !ok { "timed out".into() } else { stderr } }
}

const DESCRIBE: &str = "import sys, json; print(json.dumps({\"exe\": sys.executable, \"version\": sys.version.split()[0], \"venv\": sys.prefix != sys.base_prefix, \"prefix\": sys.prefix}))";

/// What an interpreter is (None if it doesn't run, e.g. the Microsoft Store stub).
pub fn describe_python(python: &str) -> Option<InterpreterInfo> {
    let r = run(python, &["-c", DESCRIBE], Duration::from_secs(15));
    if !r.ok {
        return None;
    }
    let line = r.stdout.trim().lines().last()?;
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let exe = v.get("exe")?.as_str().filter(|s| !s.is_empty())?.to_owned();
    Some(InterpreterInfo {
        path: exe,
        version: v.get("version").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
        is_venv: v.get("venv").and_then(|x| x.as_bool()).unwrap_or(false),
        prefix: v.get("prefix").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
    })
}

fn venv_python(dir: &Path) -> PathBuf {
    if cfg!(windows) {
        dir.join("Scripts").join("python.exe")
    } else {
        dir.join("bin").join("python")
    }
}

/// Folders directly inside the project that are virtualenvs (have pyvenv.cfg).
fn project_venvs(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let dir = e.path();
        if dir.is_dir() && dir.join("pyvenv.cfg").is_file() && venv_python(&dir).is_file() {
            out.push(venv_python(&dir).to_string_lossy().into_owned());
        }
    }
    out.sort();
    out
}

/// Interpreters on PATH and (Windows) registered with the py launcher.
fn system_pythons() -> Vec<String> {
    let mut out = Vec::new();
    if cfg!(windows) {
        let py = run("py", &["-0p"], Duration::from_secs(15));
        if py.ok {
            // Lines like " -V:3.12 *        C:\Python312\python.exe"
            for line in py.stdout.lines() {
                if let Some(i) = line.find(":\\").map(|i| i - 1) {
                    let p = line[i..].trim();
                    if p.to_lowercase().ends_with(".exe") {
                        out.push(p.to_owned());
                    }
                }
            }
        }
        let w = run("where", &["python"], Duration::from_secs(15));
        out.extend(w.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned));
    } else {
        let w = run("which", &["-a", "python3", "python"], Duration::from_secs(15));
        out.extend(w.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned));
    }
    out
}

fn key(p: &str) -> String {
    if cfg!(windows) {
        p.to_lowercase()
    } else {
        p.to_owned()
    }
}

/// Every interpreter that can be found, project venvs first; each is checked by running it.
pub fn find_interpreters(root: Option<&Path>) -> Vec<InterpreterInfo> {
    let mut candidates = root.map(project_venvs).unwrap_or_default();
    candidates.extend(system_pythons());
    let mut seen = Vec::new();
    candidates.retain(|c| {
        let k = key(c);
        if seen.contains(&k) {
            return false;
        }
        seen.push(k);
        true
    });
    let infos: Vec<Option<InterpreterInfo>> = std::thread::scope(|s| {
        let handles: Vec<_> = candidates.iter().map(|c| s.spawn(move || describe_python(c))).collect();
        handles.into_iter().map(|h| h.join().ok().flatten()).collect()
    });
    let mut out: Vec<InterpreterInfo> = Vec::new();
    for info in infos.into_iter().flatten() {
        if !out.iter().any(|o| key(&o.path) == key(&info.path)) {
            out.push(info);
        }
    }
    out
}

/// The interpreter Run would use (a configured one is the only one tried).
pub fn resolve_python(root: &Path, configured: Option<&str>) -> Option<String> {
    let list = match configured.filter(|c| !c.is_empty()) {
        Some(c) => vec![c.to_owned()],
        None => python_candidates(&root.to_string_lossy(), cfg!(windows), None),
    };
    for c in list {
        if !is_bare_command(&c) {
            if Path::new(&c).is_file() {
                return Some(c);
            }
            continue;
        }
        let r = run(&c, &["-c", "import sys; print(sys.executable)"], Duration::from_secs(15));
        if let Some(exe) = r.stdout.lines().last().map(str::trim).filter(|e| r.ok && !e.is_empty()) {
            return Some(exe.to_owned());
        }
    }
    None
}

/// The interpreter in use, described.
pub fn current_interpreter(root: &Path, configured: Option<&str>) -> Option<InterpreterInfo> {
    describe_python(&resolve_python(root, configured)?)
}

/// `pip list` for an interpreter.
pub fn list_packages(python: &str) -> Result<Vec<InstalledPackage>, String> {
    let r =
        run(python, &["-m", "pip", "list", "--format=json", "--disable-pip-version-check"], Duration::from_secs(60));
    if !r.ok {
        if r.stderr.to_lowercase().contains("no module named pip") {
            return Err("pip isn't installed in this interpreter.".into());
        }
        let lines: Vec<&str> = r.stderr.trim().lines().collect();
        return Err(lines[lines.len().saturating_sub(3)..].join("\n"));
    }
    Ok(parse_pip_list(r.stdout.lines().find(|l| l.starts_with('[')).unwrap_or("[]")))
}

/// `pip freeze` text (for writing requirements.txt).
pub fn freeze(python: &str) -> Result<String, String> {
    let r = run(python, &["-m", "pip", "freeze", "--disable-pip-version-check"], Duration::from_secs(60));
    if r.ok {
        Ok(r.stdout)
    } else {
        Err(r.stderr.trim().to_owned())
    }
}

/// The project's declared dependencies (requirements.txt and pyproject.toml).
pub fn project_requirements(root: &Path) -> (Vec<String>, Vec<Requirement>) {
    let mut sources = Vec::new();
    let mut required: Vec<Requirement> = Vec::new();
    let mut add = |list: Vec<Requirement>| {
        for r in list {
            if !required.iter().any(|x| normalize_name(&x.name) == normalize_name(&r.name)) {
                required.push(r);
            }
        }
    };
    if let Ok(t) = std::fs::read_to_string(root.join("requirements.txt")) {
        add(parse_requirements_txt(&t));
        sources.push("requirements.txt".to_owned());
    }
    if let Ok(t) = std::fs::read_to_string(root.join("pyproject.toml")) {
        sources.push("pyproject.toml".to_owned());
        add(parse_pyproject_dependencies(&t));
    }
    (sources, required)
}

pub struct DependencyCheck {
    pub sources: Vec<String>,
    pub required: Vec<Requirement>,
    pub missing: Vec<Requirement>,
    pub error: Option<String>,
}

/// Which declared dependencies aren't installed in `python`.
pub fn check_dependencies(root: &Path, python: &str) -> DependencyCheck {
    let (sources, required) = project_requirements(root);
    if required.is_empty() {
        return DependencyCheck { sources, required, missing: Vec::new(), error: None };
    }
    match list_packages(python) {
        Ok(packages) => {
            let missing = missing_packages(&required, &packages);
            DependencyCheck { sources, required, missing, error: None }
        }
        Err(e) => DependencyCheck { sources, required, missing: Vec::new(), error: Some(e) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_interpreters() {
        let venv = InterpreterInfo {
            path: "x".into(),
            version: "3.12.1".into(),
            is_venv: true,
            prefix: "C:\\p\\.venv\\".into(),
        };
        assert_eq!(interpreter_label(&venv), "Python 3.12.1 (.venv)");
        let sys = InterpreterInfo { is_venv: false, ..venv };
        assert_eq!(interpreter_label(&sys), "Python 3.12.1");
    }

    #[test]
    fn reads_the_project_requirements() {
        let dir = tempfile::tempdir().expect("temp");
        std::fs::write(dir.path().join("requirements.txt"), "flask\nrequests\n").expect("write");
        std::fs::write(dir.path().join("pyproject.toml"), "[project]\ndependencies = [\"Flask\", \"rich\"]\n")
            .expect("write");
        let (sources, required) = project_requirements(dir.path());
        assert_eq!(sources, ["requirements.txt", "pyproject.toml"]);
        assert_eq!(required.into_iter().map(|r| r.name).collect::<Vec<_>>(), ["flask", "requests", "rich"]);
    }

    #[test]
    fn describes_a_real_interpreter_when_there_is_one() {
        let Some(py) = crate::python::find_python(Path::new("."), None) else { return };
        let info = describe_python(&py).expect("python runs");
        assert!(info.version.starts_with('3'), "{info:?}");
        assert!(Path::new(&info.path).is_file());
        assert!(list_packages(&info.path).is_ok_and(|p| !p.is_empty()) || list_packages(&info.path).is_err());
    }

    #[test]
    fn a_program_that_does_not_exist_is_not_ok() {
        assert!(!run("definitely-not-a-program-xyz", &[], Duration::from_secs(2)).ok);
        assert_eq!(describe_python("definitely-not-a-program-xyz"), None);
    }
}
