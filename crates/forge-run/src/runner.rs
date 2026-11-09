//! Running a Python program for Run (F5): unbuffered, so output appears
//! live, with stdout/stderr sent in batches (a program can print far faster
//! than a window can draw). Replaces `PythonRunner` in `src/main/python.ts`.

use crate::env::process_env;
use crate::python::{is_bare_command, python_candidates};
use forge_core::output::{skipped_note, OutputQueue, Stream};
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Output is sent in batches this often...
const BATCH: Duration = Duration::from_millis(40);
/// ...and at most this much is kept waiting (older output is skipped beyond that).
const MAX_WAITING_CHARS: usize = 1_000_000;

/// What to run.
#[derive(Clone, Debug, Default)]
pub struct RunSpec {
    /// The project folder (for the virtualenv and `.env`).
    pub root: PathBuf,
    /// The interpreter set in Settings (used as-is), if any.
    pub python: Option<String>,
    /// The script, or None to run `module`.
    pub program: Option<String>,
    pub module: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
    /// Extra arguments before the target (e.g. the tracer).
    pub prefix: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunEvent {
    Start { python: String, file: String },
    Output { stream: Stream, text: String },
    Error(String),
    Exit { code: Option<i32>, ms: u64 },
}

/// A running program.
pub struct PythonRun {
    child: Arc<Mutex<Option<Child>>>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
}

impl PythonRun {
    /// Start `spec`; events (from other threads) go to `on_event`. If no
    /// interpreter can be started, an Error and an Exit are sent.
    pub fn start(spec: RunSpec, on_event: impl Fn(RunEvent) + Send + Sync + 'static) -> Option<PythonRun> {
        let send: Arc<dyn Fn(RunEvent) + Send + Sync> = Arc::new(on_event);
        let configured = spec.python.clone().filter(|p| !p.is_empty());
        let candidates = match &configured {
            Some(c) => vec![c.clone()],
            None => python_candidates(&spec.root.to_string_lossy(), cfg!(windows), None),
        };
        let target: Vec<String> = match (&spec.module, &spec.program) {
            (Some(m), _) => vec!["-m".into(), m.clone()],
            (None, Some(p)) => vec![p.clone()],
            (None, None) => Vec::new(),
        };
        let file = match &spec.module {
            Some(m) => format!("-m {m}"),
            None => spec.program.clone().unwrap_or_default(),
        };
        let mut extra = spec.env.clone();
        extra.insert("PYTHONIOENCODING".into(), "utf-8".into());
        extra.insert("PYTHONUNBUFFERED".into(), "1".into());
        let env = process_env(Some(&spec.root), &extra);
        for python in candidates {
            // Paths (virtualenvs) must exist; bare commands are found on PATH.
            if !is_bare_command(&python) && !std::path::Path::new(&python).exists() {
                continue;
            }
            let mut cmd = Command::new(&python);
            cmd.arg("-u").args(&spec.prefix).args(&target).args(&spec.args);
            cmd.current_dir(&spec.cwd).env_clear().envs(&env);
            cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
            hide_window(&mut cmd);
            match cmd.spawn() {
                Ok(child) => return Some(Self::watch(child, python, file, send)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => {
                    send(RunEvent::Error(e.to_string()));
                    send(RunEvent::Exit { code: None, ms: 0 });
                    return None;
                }
            }
        }
        send(RunEvent::Error(match configured {
            Some(c) => format!(
                "The Python interpreter set in Settings wasn't found:\n  {c}\nFix it in Settings (Ctrl+,), or clear it to pick one automatically."
            ),
            None => "Python was not found. Install it from python.org, or create a .venv in your project.".into(),
        }));
        send(RunEvent::Exit { code: None, ms: 0 });
        None
    }

    fn watch(mut child: Child, python: String, file: String, send: Arc<dyn Fn(RunEvent) + Send + Sync>) -> PythonRun {
        let started = Instant::now();
        send(RunEvent::Start { python, file });
        let queue = Arc::new(Mutex::new(OutputQueue::new(MAX_WAITING_CHARS)));
        let open_pipes = Arc::new(AtomicUsize::new(2));
        let reader = |pipe: Option<Box<dyn Read + Send>>, stream: Stream| {
            let queue = queue.clone();
            let open = open_pipes.clone();
            std::thread::spawn(move || {
                if let Some(mut pipe) = pipe {
                    let mut buf = [0u8; 8192];
                    let mut pending: Vec<u8> = Vec::new();
                    while let Ok(n) = pipe.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        pending.extend_from_slice(&buf[..n]);
                        // Only whole UTF-8 characters go out (a character may be split between reads).
                        let valid = match std::str::from_utf8(&pending) {
                            Ok(_) => pending.len(),
                            Err(e) if e.error_len().is_none() => e.valid_up_to(),
                            Err(_) => pending.len(),
                        };
                        let text = String::from_utf8_lossy(&pending[..valid]).into_owned();
                        pending.drain(..valid);
                        if let Ok(mut q) = queue.lock() {
                            q.push(stream, &text);
                        }
                    }
                    if !pending.is_empty() {
                        if let Ok(mut q) = queue.lock() {
                            q.push(stream, &String::from_utf8_lossy(&pending));
                        }
                    }
                }
                open.fetch_sub(1, Ordering::SeqCst);
            });
        };
        reader(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>), Stream::Stdout);
        reader(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>), Stream::Stderr);
        let stdin = Arc::new(Mutex::new(child.stdin.take()));
        let child = Arc::new(Mutex::new(Some(child)));
        let flush = {
            let queue = queue.clone();
            let send = send.clone();
            move || {
                let (pieces, dropped) = queue.lock().map(|mut q| q.take()).unwrap_or_default();
                if dropped > 0 {
                    send(RunEvent::Output { stream: Stream::Stderr, text: skipped_note(dropped) });
                }
                for p in pieces {
                    send(RunEvent::Output { stream: p.stream, text: p.text });
                }
            }
        };
        {
            let child = child.clone();
            let stdin = stdin.clone();
            std::thread::spawn(move || {
                let code = loop {
                    std::thread::sleep(BATCH);
                    flush();
                    let status = match child.lock() {
                        Ok(mut c) => match c.as_mut().map(Child::try_wait) {
                            Some(Ok(Some(status))) => Some(status.code()),
                            Some(Ok(None)) => None,
                            Some(Err(_)) | None => Some(None),
                        },
                        Err(_) => Some(None),
                    };
                    if let Some(code) = status {
                        // Its last output first (the pipes close just after it ends).
                        let wait_until = Instant::now() + Duration::from_millis(500);
                        while open_pipes.load(Ordering::SeqCst) > 0 && Instant::now() < wait_until {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        break code;
                    }
                };
                flush();
                if let Ok(mut s) = stdin.lock() {
                    s.take();
                }
                send(RunEvent::Exit { code, ms: started.elapsed().as_millis() as u64 });
            });
        }
        PythonRun { child, stdin }
    }

    /// Text typed into the program (for `input()`).
    pub fn send_input(&self, text: &str) {
        if let Ok(mut s) = self.stdin.lock() {
            if let Some(stdin) = s.as_mut() {
                // Typing after the program ended: nothing to do.
                let _ = stdin.write_all(text.as_bytes()).and_then(|_| stdin.flush());
            }
        }
    }

    pub fn stop(&self) {
        if let Ok(mut c) = self.child.lock() {
            if let Some(child) = c.as_mut() {
                let _ = child.kill();
            }
        }
    }

    pub fn running(&self) -> bool {
        self.child.lock().is_ok_and(|mut c| c.as_mut().is_some_and(|ch| matches!(ch.try_wait(), Ok(None))))
    }
}

impl Drop for PythonRun {
    fn drop(&mut self) {
        self.stop();
    }
}

/// No console window flashes up for the program on Windows.
pub fn hide_window(cmd: &mut Command) {
    forge_sys::process::hide_window(cmd);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    fn python() -> Option<String> {
        crate::python::find_python(std::path::Path::new("."), None)
    }

    fn run(code: &str, input: Option<&str>) -> Option<(String, String, Option<i32>)> {
        let py = python()?;
        let dir = tempfile::tempdir().expect("temp");
        std::fs::write(dir.path().join("main.py"), code).expect("write");
        let (tx, rx) = channel();
        let tx = Mutex::new(tx);
        let r = PythonRun::start(
            RunSpec {
                root: dir.path().to_path_buf(),
                python: Some(py),
                program: Some("main.py".into()),
                cwd: dir.path().to_path_buf(),
                ..Default::default()
            },
            move |e| {
                let _ = tx.lock().map(|t| t.send(e));
            },
        )?;
        if let Some(i) = input {
            r.send_input(i);
        }
        let (mut out, mut err) = (String::new(), String::new());
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(RunEvent::Output { stream: Stream::Stdout, text }) => out.push_str(&text),
                Ok(RunEvent::Output { stream: Stream::Stderr, text }) => err.push_str(&text),
                Ok(RunEvent::Exit { code, .. }) => return Some((out, err, code)),
                _ => {}
            }
        }
        None
    }

    #[test]
    fn runs_a_file_with_input_and_reports_output_and_exit_code() {
        let Some((out, err, code)) = run(
            "name = input()\nprint('hi', name)\nimport sys\nprint('oops', file=sys.stderr)\nsys.exit(3)\n",
            Some("Ada\n"),
        ) else {
            eprintln!("no Python here: skipped");
            return;
        };
        assert_eq!(out.replace("\r\n", "\n"), "hi Ada\n");
        assert_eq!(err.replace("\r\n", "\n"), "oops\n");
        assert_eq!(code, Some(3));
    }

    #[test]
    fn a_missing_configured_interpreter_says_so() {
        let (tx, rx) = channel();
        let tx = Mutex::new(tx);
        let r = PythonRun::start(
            RunSpec {
                python: Some("C:/definitely/not/python.exe".into()),
                program: Some("x.py".into()),
                ..Default::default()
            },
            move |e| {
                let _ = tx.lock().map(|t| t.send(e));
            },
        );
        assert!(r.is_none());
        let events: Vec<RunEvent> = rx.try_iter().collect();
        assert!(matches!(&events[0], RunEvent::Error(m) if m.contains("wasn't found")));
        assert_eq!(events[1], RunEvent::Exit { code: None, ms: 0 });
    }
}
