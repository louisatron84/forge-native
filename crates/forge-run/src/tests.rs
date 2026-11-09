//! Discovering and running the project's Python tests: pytest when it's
//! installed (results from its JUnit XML report), else unittest (results from
//! its -v output). Output streams as it comes. Replaces `src/main/tests.ts`.

use crate::env::process_env;
use crate::pyenv::run;
use crate::runner::hide_window;
use forge_core::testing::{find_tests, is_test_file, test_args, TestFile};
use forge_sys::files::list_files;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Test files in the project and the tests in each.
pub fn discover_tests(root: &Path) -> Vec<TestFile> {
    let mut out = Vec::new();
    for f in list_files(root, 20_000).into_iter().filter(|f| is_test_file(&f.rel)).take(2000) {
        let Ok(meta) = std::fs::metadata(&f.path) else { continue };
        if meta.len() > 2_000_000 {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&f.path) else { continue };
        let tests = find_tests(&text);
        if !tests.is_empty() {
            out.push(TestFile { path: f.path.to_string_lossy().into_owned(), rel: f.rel.replace('\\', "/"), tests });
        }
    }
    out
}

pub fn has_pytest(python: &str) -> bool {
    run(python, &["-c", "import pytest"], Duration::from_secs(20)).ok
}

#[derive(Clone, Debug, Default)]
pub struct TestRunResult {
    /// None: stopped, or couldn't start.
    pub exit_code: Option<i32>,
    pub output: String,
    /// pytest's JUnit report.
    pub xml: Option<String>,
    pub seconds: f64,
    pub error: Option<String>,
}

static RUNS: AtomicU64 = AtomicU64::new(0);

/// A test run in progress.
pub struct TestRun {
    child: Arc<Mutex<Option<Child>>>,
}

impl TestRun {
    /// Run tests; output goes to `on_output` as it comes, the result to `on_done`
    /// (both called on other threads).
    pub fn start(
        python: &str,
        root: &Path,
        pytest: bool,
        ids: &[String],
        on_output: impl Fn(String) + Send + Sync + 'static,
        on_done: impl FnOnce(TestRunResult) + Send + 'static,
    ) -> TestRun {
        let n = RUNS.fetch_add(1, Ordering::SeqCst);
        let xml_path: PathBuf = std::env::temp_dir().join(format!("forge-pytest-{}-{n}.xml", std::process::id()));
        let args = test_args(pytest, ids, &xml_path.to_string_lossy());
        let extra = BTreeMap::from([
            ("PYTHONIOENCODING".to_owned(), "utf-8".to_owned()),
            ("PYTHONUNBUFFERED".to_owned(), "1".to_owned()),
            ("PYTHONDONTWRITEBYTECODE".to_owned(), "1".to_owned()),
        ]);
        let mut cmd = Command::new(python);
        cmd.args(&args).current_dir(root).env_clear().envs(process_env(Some(root), &extra));
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        hide_window(&mut cmd);
        let started = Instant::now();
        let child_slot = Arc::new(Mutex::new(None));
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                on_done(TestRunResult {
                    output: e.to_string(),
                    error: Some(format!("Couldn't start {python}")),
                    ..Default::default()
                });
                return TestRun { child: child_slot };
            }
        };
        let output = Arc::new(Mutex::new(String::new()));
        let on_output = Arc::new(on_output);
        let readers: Vec<_> = [
            child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>),
            child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>),
        ]
        .into_iter()
        .flatten()
        .map(|mut pipe| {
            let output = output.clone();
            let on_output = on_output.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                while let Ok(n) = pipe.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                    if let Ok(mut o) = output.lock() {
                        o.push_str(&text);
                    }
                    on_output(text);
                }
            })
        })
        .collect();
        *child_slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
        let slot = child_slot.clone();
        std::thread::spawn(move || {
            let code = loop {
                std::thread::sleep(Duration::from_millis(50));
                let done = match slot.lock() {
                    Ok(mut c) => match c.as_mut().map(Child::try_wait) {
                        Some(Ok(Some(status))) => Some(status.code()),
                        Some(Ok(None)) => None,
                        _ => Some(None),
                    },
                    Err(_) => Some(None),
                };
                if let Some(code) = done {
                    break code;
                }
            };
            for r in readers {
                let _ = r.join();
            }
            let xml = if pytest { std::fs::read_to_string(&xml_path).ok() } else { None };
            let _ = std::fs::remove_file(&xml_path);
            let output = output.lock().map(|o| o.clone()).unwrap_or_default();
            on_done(TestRunResult {
                exit_code: code,
                output,
                xml,
                seconds: started.elapsed().as_secs_f64(),
                error: None,
            });
        });
        TestRun { child: child_slot }
    }

    pub fn stop(&self) {
        if let Ok(mut c) = self.child.lock() {
            if let Some(child) = c.as_mut() {
                let _ = child.kill();
            }
        }
    }
}

#[cfg(test)]
mod unit {
    use super::*;
    use forge_core::testing::{parse_unittest, unittest_id, unittest_modules};
    use std::sync::mpsc::channel;

    #[test]
    fn discovers_test_files_and_runs_them_with_unittest() {
        let dir = tempfile::tempdir().expect("temp");
        let root = dir.path();
        std::fs::create_dir_all(root.join("tests")).expect("mkdir");
        std::fs::write(
            root.join("tests").join("test_math.py"),
            "import unittest\n\nclass MathTests(unittest.TestCase):\n    def test_ok(self):\n        self.assertEqual(1 + 1, 2)\n\n    def test_bad(self):\n        self.assertEqual(1, 2)\n",
        )
        .expect("write");
        std::fs::write(root.join("app.py"), "def f(): pass\n").expect("write");
        let files = discover_tests(root);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].rel, "tests/test_math.py");
        assert_eq!(files[0].tests.len(), 2);
        let Some(python) = crate::python::find_python(root, None) else { return };
        let (tx, rx) = channel();
        let ids = unittest_modules(&files);
        let _run = TestRun::start(
            &python,
            root,
            false,
            &ids,
            |_| {},
            move |r| {
                let _ = tx.send(r);
            },
        );
        let result = rx.recv_timeout(Duration::from_secs(60)).expect("finished");
        let r = parse_unittest(&result.output);
        let id = |name: &str| unittest_id("tests/test_math.py", name, Some("MathTests"));
        assert_eq!(
            r.get(&id("test_ok")).map(|x| x.status),
            Some(forge_core::testing::TestStatus::Passed),
            "{}",
            result.output
        );
        assert_eq!(r.get(&id("test_bad")).map(|x| x.status), Some(forge_core::testing::TestStatus::Failed));
        assert_eq!(result.exit_code, Some(1));
    }
}
