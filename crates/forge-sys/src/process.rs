//! Running a helper program to completion: text piped in, output captured,
//! killed after a timeout, and no console window flashing up on Windows.
//! Used for git, Ruff and Black.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

mod managed;
#[cfg(target_os = "linux")]
pub use managed::set_parent_death_signal;
#[cfg(windows)]
pub use managed::WindowsJob;
pub use managed::{stop_all_children, ManagedChild};

const MAX_CAPTURE_BYTES: usize = 64 * 1024 * 1024;

/// Don't open a console window for a child process (Windows).
pub fn hide_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Captured {
    /// The exit code (None if it was killed, or couldn't start).
    pub code: Option<i32>,
    /// With \r\n turned into \n.
    pub stdout: String,
    pub stderr: String,
    /// It couldn't be started at all (`not_found`: the program isn't there).
    pub failed: bool,
    pub not_found: bool,
}

impl Captured {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }
}

/// Run `program args` in `cwd` with `input` on stdin and extra environment
/// variables; killed after `timeout`.
pub fn run_captured(
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &[(&str, &str)],
    input: &str,
    timeout: Duration,
) -> Captured {
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in env {
        command.env(k, v);
    }
    let mut child = match ManagedChild::spawn(&mut command, timeout) {
        Ok(c) => c,
        Err(e) => {
            return Captured { failed: true, not_found: e.kind() == std::io::ErrorKind::NotFound, ..Default::default() }
        }
    };
    // Feed stdin and drain both pipes on threads, so a large output can't block it.
    let mut stdin = child.take_stdin();
    let input = input.to_owned();
    let writer = std::thread::spawn(move || {
        if let Some(s) = stdin.as_mut() {
            let _ = s.write_all(input.as_bytes());
        }
    });
    let overflow = Arc::new(AtomicBool::new(false));
    fn drain(pipe: Option<impl Read + Send + 'static>, overflow: Arc<AtomicBool>) -> std::thread::JoinHandle<String> {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut p) = pipe {
                let mut chunk = [0; 8192];
                while let Ok(n) = p.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    if n > MAX_CAPTURE_BYTES - bytes.len() {
                        overflow.store(true, Ordering::Release);
                        return String::new();
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                }
            }
            String::from_utf8_lossy(&bytes).replace("\r\n", "\n")
        })
    }
    let out = drain(child.take_stdout(), overflow.clone());
    let err = drain(child.take_stderr(), overflow.clone());
    let (code, failure) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (status.code(), None),
            Ok(None) if !overflow.load(Ordering::Acquire) => std::thread::sleep(Duration::from_millis(5)),
            Ok(None) => break (None, None),
            Err(error) => break (None, Some(error.to_string())),
        }
    };
    // Reap the leader and close any pipes still held open by its descendants.
    drop(child);
    let _ = writer.join();
    let stdout = out.join().unwrap_or_default();
    let mut stderr = err.join().unwrap_or_default();
    if overflow.load(Ordering::Acquire) {
        // Never let callers apply a partial formatter response or parse truncated analysis.
        return Captured {
            code: Some(-1),
            stderr: "Helper exceeded the 64 MiB output limit.".into(),
            ..Default::default()
        };
    }
    if let Some(error) = failure {
        if !stderr.is_empty() && !stderr.ends_with('\n') {
            stderr.push('\n');
        }
        stderr.push_str(&error);
    }
    Captured { code, stdout, stderr, failed: false, not_found: false }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn helper_timeout_closes_pipes_held_open_by_its_child() {
        let python = if cfg!(windows) { "python" } else { "python3" };
        let script = "import subprocess,sys,time\nsubprocess.Popen([sys.executable,'-I','-S','-c','import time; time.sleep(5)'])\nprint('child ready', flush=True)\ntime.sleep(5)\n";
        let started = Instant::now();
        let result = run_captured(
            python,
            &["-I".into(), "-S".into(), "-c".into(), script.into()],
            &std::env::temp_dir(),
            &[],
            "",
            Duration::from_millis(900),
        );
        if result.not_found {
            return eprintln!("Python not installed: skipped process-tree test");
        }
        assert!(result.stdout.contains("child ready"), "{result:?}");
        assert_eq!(result.code, None);
        assert!(started.elapsed() < Duration::from_secs(4), "A descendant kept the capture pipes open");
    }

    #[test]
    fn helper_exit_ends_the_group_without_waiting_for_inherited_pipes() {
        let script = "import subprocess,sys\nsubprocess.Popen([sys.executable,'-I','-S','-c','import time; time.sleep(5)'])\nprint('done', flush=True)\n";
        let started = Instant::now();
        let result = run_captured(
            if cfg!(windows) { "python" } else { "python3" },
            &["-I".into(), "-S".into(), "-c".into(), script.into()],
            &std::env::temp_dir(),
            &[],
            "",
            Duration::from_secs(8),
        );
        if result.not_found {
            return eprintln!("Python not installed: skipped process-tree test");
        }
        assert_eq!(result.code, Some(0), "{result:?}");
        assert_eq!(result.stdout.trim(), "done");
        assert!(started.elapsed() < Duration::from_secs(4), "A descendant kept the capture pipes open");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_helpers_start_in_a_new_group_with_the_parent_death_signal() {
        let script = "import ctypes,os,signal\nsig=ctypes.c_int()\nassert ctypes.CDLL(None).prctl(2, ctypes.byref(sig), 0, 0, 0) == 0\nassert sig.value == signal.SIGKILL\nassert os.getpgrp() == os.getpid()\n";
        let result = run_captured(
            "python3",
            &["-I".into(), "-S".into(), "-c".into(), script.into()],
            &std::env::temp_dir(),
            &[],
            "",
            Duration::from_secs(8),
        );
        if !result.not_found {
            assert!(result.ok(), "{result:?}");
        }
    }

    #[test]
    fn helper_output_limit_is_an_error_not_silent_truncation() {
        for stream in ["stdout", "stderr"] {
            let script =
                format!("import sys\nfor _ in range(65):\n    sys.{stream}.buffer.write(b'x' * (1024 * 1024))\n");
            let result = run_captured(
                if cfg!(windows) { "python" } else { "python3" },
                &["-I".into(), "-S".into(), "-c".into(), script],
                &std::env::temp_dir(),
                &[],
                "",
                Duration::from_secs(10),
            );
            if result.not_found {
                return eprintln!("Python not installed: skipped output-cap test");
            }
            assert!(!result.ok(), "an oversized helper response was accepted");
            assert!(result.stdout.is_empty(), "partial output must never be applied as formatted code");
            assert!(result.stderr.contains("output limit"), "{result:?}");
        }
    }

    #[test]
    fn a_missing_program_is_reported_as_not_found() {
        let r = run_captured("forge-no-such-program-xyz", &[], &std::env::temp_dir(), &[], "", Duration::from_secs(5));
        assert!(r.failed && r.not_found && !r.ok());
    }

    #[test]
    fn input_goes_in_and_output_comes_back() {
        // git is everywhere Forge's own tests run; it echoes its stdin with hash-object.
        let r = run_captured(
            "git",
            &["hash-object".into(), "--stdin".into()],
            &std::env::temp_dir(),
            &[],
            "hello\n",
            Duration::from_secs(20),
        );
        if r.not_found {
            return eprintln!("git isn't installed: skipped");
        }
        assert!(r.ok(), "{r:?}");
        assert_eq!(r.stdout.trim(), "ce013625030ba8dba906f756967f9e9ca394464a");
    }
}
