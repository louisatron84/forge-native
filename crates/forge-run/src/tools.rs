//! Ruff and Black, run on the editor's text (piped in, so unsaved changes
//! count): lint with fixes, format, organise imports. They run from the
//! project folder so its own configuration (pyproject.toml, ruff.toml)
//! applies. Each tool is looked for in the project's interpreter
//! (`python -m ruff`), then (Ruff only) on PATH. Replaces `src/main/pyTools.ts`.
//! Everything here blocks: call it from a background thread.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Ruff,
    Black,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Ruff => "Ruff",
            Tool::Black => "Black",
        }
    }

    fn module(self) -> &'static str {
        match self {
            Tool::Ruff => "ruff",
            Tool::Black => "black",
        }
    }
}

/// What a tool gave back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolResult {
    pub ok: bool,
    /// The tool isn't installed (in the interpreter or on PATH).
    pub missing: bool,
    pub output: String,
    pub error: Option<String>,
}

impl ToolResult {
    fn missing() -> Self {
        Self { missing: true, ..Default::default() }
    }

    fn failed(error: String) -> Self {
        Self { error: Some(error), ..Default::default() }
    }

    fn ok(output: String) -> Self {
        Self { ok: true, output, ..Default::default() }
    }
}

use forge_sys::process::{run_captured, Captured as Output};

/// Run a program with `input` on stdin; killed after `timeout`.
fn run(cmd: &str, args: &[String], cwd: &Path, input: &str, timeout: Duration) -> Output {
    run_captured(cmd, args, cwd, &[("PYTHONIOENCODING", "utf-8"), ("NO_COLOR", "1")], input, timeout)
}

/// How to start a tool: (program, arguments before the tool's own).
type Launcher = (String, Vec<String>);

/// Finds and runs the tools (remembers where each one was found).
#[derive(Default)]
pub struct Tools {
    launchers: Mutex<HashMap<(String, Tool), Option<Launcher>>>,
}

const TIMEOUT: Duration = Duration::from_secs(30);

fn error_text(o: &Output) -> String {
    let text = if o.stderr.trim().is_empty() { o.stdout.trim() } else { o.stderr.trim() };
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(4)..].join("\n");
    if tail.is_empty() { "it failed".to_owned() } else { tail }
}

impl Tools {
    pub fn new() -> Self {
        Self::default()
    }

    /// How to run a tool for this interpreter (cached).
    fn launcher(&self, python: Option<&str>, tool: Tool, cwd: &Path) -> Option<Launcher> {
        let key = (python.unwrap_or("").to_owned(), tool);
        if let Some(found) = self.launchers.lock().expect("tools").get(&key) {
            return found.clone();
        }
        let mut options: Vec<Launcher> = Vec::new();
        if let Some(p) = python {
            options.push((p.to_owned(), vec!["-m".into(), tool.module().into()]));
        }
        if tool == Tool::Ruff {
            options.push(("ruff".into(), Vec::new()));
        }
        let found = options.into_iter().find(|(cmd, pre)| {
            let mut args = pre.clone();
            args.push("--version".into());
            let o = run(cmd, &args, cwd, "", Duration::from_secs(20));
            !o.failed && o.code == Some(0)
        });
        self.launchers.lock().expect("tools").insert(key, found.clone());
        found
    }

    /// Run a tool with its own arguments on `text`.
    fn tool(&self, python: Option<&str>, cwd: &Path, tool: Tool, args: &[&str], text: &str) -> Result<Output, ToolResult> {
        let Some((cmd, mut all)) = self.launcher(python, tool, cwd) else { return Err(ToolResult::missing()) };
        all.extend(args.iter().map(|a| (*a).to_owned()));
        Ok(run(&cmd, &all, cwd, text, TIMEOUT))
    }

    /// Forget where the tools were found (after pip installs something).
    pub fn forget(&self) {
        self.launchers.lock().expect("tools").clear();
    }

    /// (Ruff installed, Black installed).
    pub fn available(&self, python: Option<&str>, cwd: &Path) -> (bool, bool) {
        (self.launcher(python, Tool::Ruff, cwd).is_some(), self.launcher(python, Tool::Black, cwd).is_some())
    }

    /// `ruff check` on a file's text: Ruff's JSON list of problems (with fixes).
    pub fn ruff_check(&self, python: Option<&str>, cwd: &Path, path: &str, text: &str) -> ToolResult {
        let args = ["check", "--output-format=json", "--exit-zero", "--no-cache", "--stdin-filename", path, "-"];
        match self.tool(python, cwd, Tool::Ruff, &args, text) {
            Err(missing) => missing,
            Ok(o) if o.code == Some(0) => ToolResult::ok(o.stdout),
            Ok(o) => ToolResult::failed(format!("Ruff: {}", error_text(&o))),
        }
    }

    /// `ruff check --fix` (optionally only some rules, e.g. I and F401 to
    /// organise imports): the fixed text.
    pub fn ruff_fix(&self, python: Option<&str>, cwd: &Path, path: &str, text: &str, select: &[&str]) -> ToolResult {
        let joined = select.join(",");
        let mut args = vec!["check", "--fix", "--exit-zero", "--no-cache", "--quiet"];
        if !select.is_empty() {
            args.extend(["--select", joined.as_str()]);
        }
        args.extend(["--stdin-filename", path, "-"]);
        match self.tool(python, cwd, Tool::Ruff, &args, text) {
            Err(missing) => missing,
            Ok(o) if o.code == Some(0) => ToolResult::ok(o.stdout),
            Ok(o) => ToolResult::failed(format!("Ruff: {}", error_text(&o))),
        }
    }

    /// Format with Ruff or Black. `lines` (1-based, inclusive) limits it to a
    /// selection where the tool supports that.
    pub fn format_text(
        &self,
        python: Option<&str>,
        cwd: &Path,
        path: &str,
        text: &str,
        tool: Tool,
        lines: Option<(usize, usize)>,
    ) -> ToolResult {
        let range = lines.map(|(from, to)| match tool {
            // Ruff's range ends at the start of its end line (so "2-2" is empty).
            Tool::Ruff => format!("--range={from}-{}", to + 1),
            Tool::Black => format!("--line-ranges={from}-{to}"),
        });
        let mut args: Vec<&str> = match tool {
            Tool::Ruff => vec!["format", "--no-cache", "--quiet"],
            Tool::Black => vec!["--quiet"],
        };
        args.extend(range.as_deref());
        args.extend(["--stdin-filename", path, "-"]);
        let o = match self.tool(python, cwd, tool, &args, text) {
            Err(missing) => return missing,
            Ok(o) => o,
        };
        if o.code == Some(0) {
            return ToolResult::ok(o.stdout);
        }
        let msg = error_text(&o);
        // Older versions don't know about formatting part of a file.
        let lower = msg.to_lowercase();
        if lines.is_some() && ["range", "line-ranges", "unexpected argument", "no such option"].iter().any(|w| lower.contains(w)) {
            return ToolResult::failed(format!(
                "This version of {} can't format just a selection; upgrade it, or format the whole file.",
                tool.name()
            ));
        }
        ToolResult::failed(format!("{} couldn't format it: {msg}", tool.name()))
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    /// Ruff on PATH (the tests are skipped without it).
    fn ruff() -> Option<Tools> {
        let t = Tools::new();
        let dir = std::env::temp_dir();
        t.launcher(None, Tool::Ruff, &dir).map(|_| t)
    }

    #[test]
    fn ruff_check_reports_problems_with_fixes() {
        let Some(t) = ruff() else { return eprintln!("ruff isn't installed: skipped") };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.py");
        let r = t.ruff_check(None, dir.path(), &path.to_string_lossy(), "import os\nx = 1\n");
        assert!(r.ok, "{r:?}");
        let problems = forge_core::lint::parse_ruff_json(&r.output);
        let f401 = problems.iter().find(|p| p.code == "F401").expect("unused import reported");
        assert!(f401.fix.is_some());
    }

    #[test]
    fn ruff_fix_organises_imports_on_the_piped_text() {
        let Some(t) = ruff() else { return eprintln!("ruff isn't installed: skipped") };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.py");
        let r = t.ruff_fix(None, dir.path(), &path.to_string_lossy(), "import sys\nimport os\n\nprint(sys.argv)\n", &["I", "F401"]);
        assert!(r.ok, "{r:?}");
        assert_eq!(r.output, "import sys\n\nprint(sys.argv)\n");
    }

    #[test]
    fn ruff_formats_the_whole_text_or_a_range() {
        let Some(t) = ruff() else { return eprintln!("ruff isn't installed: skipped") };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.py");
        let p = path.to_string_lossy();
        let r = t.format_text(None, dir.path(), &p, "x=1\ny=2\n", Tool::Ruff, None);
        assert_eq!(r, ToolResult::ok("x = 1\ny = 2\n".into()));
        let r = t.format_text(None, dir.path(), &p, "x=1\ny=2\n", Tool::Ruff, Some((2, 2)));
        assert_eq!(r.output, "x=1\ny = 2\n");
        // Broken code: an error, not a crash.
        let r = t.format_text(None, dir.path(), &p, "def (:\n", Tool::Ruff, None);
        assert!(!r.ok && r.error.as_deref().unwrap_or("").starts_with("Ruff couldn't format it"), "{r:?}");
    }

    #[test]
    fn a_missing_tool_says_so() {
        let t = Tools::new();
        let dir = tempfile::tempdir().unwrap();
        let python = Some("C:/no/such/python.exe");
        // Black is only looked for in the interpreter.
        assert_eq!(t.format_text(python, dir.path(), "a.py", "x=1\n", Tool::Black, None), ToolResult::missing());
    }

    #[test]
    fn error_text_keeps_the_last_four_lines() {
        let o = Output { code: Some(2), stdout: "out".into(), stderr: "1\n2\n\n3\n4\n5\n".into(), ..Default::default() };
        assert_eq!(error_text(&o), "2\n3\n4\n5");
        let o = Output { code: Some(2), ..Default::default() };
        assert_eq!(error_text(&o), "it failed");
    }
}
