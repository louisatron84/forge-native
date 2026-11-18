//! Running a language server process and piping JSON-RPC to and from it;
//! finding basedpyright. Replaces `src/main/languageServer.ts`.
//!
//! The Electron app bundled basedpyright and ran it with Electron's own
//! Node. The native app has no Node, so it uses basedpyright's Python package
//! (which brings its own Node): from the project's interpreter, PATH, or
//! Forge's own tools environment (installed on request).

use crate::client::LspClient;
use crate::jsonrpc::{encode_message, MessageReader};
use forge_sys::process::ManagedChild;
use serde_json::Value;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

/// The command that starts basedpyright's language server.
pub const LANGSERVER: &str = "basedpyright-langserver";
const SESSION_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
const SHUTDOWN_GRACE: Duration = Duration::from_millis(60);

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

/// Forge's own tools environment (a virtualenv in its data folder).
pub fn tools_env(data_dir: &Path) -> PathBuf {
    data_dir.join("tools")
}

fn scripts_dir(env: &Path) -> PathBuf {
    if cfg!(windows) {
        env.join("Scripts")
    } else {
        env.join("bin")
    }
}

/// Where basedpyright-langserver is: next to the project's interpreter, on PATH,
/// or in Forge's tools environment.
pub fn find_langserver(python: Option<&str>, data_dir: &Path) -> Option<PathBuf> {
    if let Some(p) = python.map(Path::new).and_then(Path::parent) {
        let c = p.join(exe(LANGSERVER));
        if c.is_file() {
            return Some(c);
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let c = dir.join(exe(LANGSERVER));
            if c.is_file() {
                return Some(c);
            }
        }
    }
    let c = scripts_dir(&tools_env(data_dir)).join(exe(LANGSERVER));
    c.is_file().then_some(c)
}

/// The interpreter of Forge's tools environment (for installing into it).
pub fn tools_python(data_dir: &Path) -> PathBuf {
    scripts_dir(&tools_env(data_dir)).join(exe("python"))
}

/// A running server, wired to an LspClient.
pub struct ServerProcess {
    pub client: Arc<LspClient>,
    child: Arc<Mutex<ManagedChild>>,
    writer: Writer,
}

enum Outbound {
    Message(Value),
    Close(mpsc::SyncSender<()>),
}

type Writer = Arc<Mutex<Option<mpsc::Sender<Outbound>>>>;

fn close_writer(writer: &Writer, flush: bool) {
    let sender = writer.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(sender) = sender {
        let (ack, closed) = mpsc::sync_channel(1);
        if sender.send(Outbound::Close(ack)).is_ok() && flush {
            let _ = closed.recv_timeout(SHUTDOWN_GRACE);
        }
    }
}

impl ServerProcess {
    /// Start `command args`; `on_exit` is called (on a reader thread) when it ends.
    pub fn spawn(
        command: &Path,
        args: &[&str],
        cwd: &Path,
        on_exit: impl FnOnce(Option<i32>) + Send + 'static,
    ) -> std::io::Result<Self> {
        let mut cmd = Command::new(command);
        cmd.args(args).current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = ManagedChild::spawn(&mut cmd, SESSION_TIMEOUT)?;
        let mut stdin = child.take_stdin().ok_or_else(|| io::Error::other("No language server stdin"))?;
        let mut stdout = child.take_stdout().ok_or_else(|| io::Error::other("No language server stdout"))?;
        let mut stderr = child.take_stderr();
        let (tx, rx) = mpsc::channel();
        // Writing happens on its own thread, so a busy server never blocks the UI.
        std::thread::spawn(move || {
            while let Ok(m) = rx.recv() {
                match m {
                    Outbound::Message(message) => {
                        if stdin.write_all(&encode_message(&message)).and_then(|_| stdin.flush()).is_err() {
                            break;
                        }
                    }
                    Outbound::Close(closed) => {
                        drop(stdin);
                        let _ = closed.send(());
                        break;
                    }
                }
            }
        });
        let writer = Arc::new(Mutex::new(Some(tx)));
        let outgoing = writer.clone();
        let client = LspClient::new(Arc::new(move |m| {
            if let Some(sender) = outgoing.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                let _ = sender.send(Outbound::Message(m));
            }
        }));
        let child = Arc::new(Mutex::new(child));
        {
            let client = client.clone();
            let child = child.clone();
            let writer = writer.clone();
            std::thread::spawn(move || {
                let mut reader = MessageReader::new();
                let mut buf = [0u8; 16384];
                while let Ok(n) = stdout.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    for m in reader.feed(&buf[..n]) {
                        client.handle_message(m);
                    }
                    if reader.is_failed() {
                        break;
                    }
                }
                client.dispose("the language server stopped");
                close_writer(&writer, false);
                if let Ok(mut child) = child.lock() {
                    // If stdout closed before the process finished, its protocol
                    // cannot continue. When it has exited, preserve its code.
                    if !matches!(child.try_wait(), Ok(Some(_))) {
                        let _ = child.stop();
                    }
                }
            });
        }
        {
            let child = child.clone();
            let client = client.clone();
            let writer = writer.clone();
            std::thread::spawn(move || {
                let code = loop {
                    let status = child
                        .lock()
                        .map_err(|_| io::Error::other("Language server process lock failed"))
                        .and_then(|mut child| child.try_wait());
                    match status {
                        Ok(Some(status)) => break status.code(),
                        Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                        Err(_) => {
                            if let Ok(mut child) = child.lock() {
                                let _ = child.stop();
                            }
                            break None;
                        }
                    }
                };
                client.dispose("the language server stopped");
                close_writer(&writer, false);
                on_exit(code);
            });
        }
        // Drain stderr (FORGE_LSP_LOG=1 shows it).
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let log = std::env::var_os("FORGE_LSP_LOG").is_some();
            if let Some(e) = stderr.as_mut() {
                while let Ok(n) = e.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    if log {
                        eprint!("{}", String::from_utf8_lossy(&buf[..n]));
                    }
                }
            }
        });
        Ok(Self { client, child, writer })
    }

    pub fn stop(&self) {
        // Settle request futures on the caller before tearing down its executor.
        // EOF may reach the reader much later (or while a callback is blocked).
        self.client.dispose("the language server stopped");
        close_writer(&self.writer, true);
        if let Ok(mut c) = self.child.lock() {
            let _ = c.stop();
        }
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopping_settles_pending_requests_before_a_blocked_reader_can_exit() {
        let release = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let script = "import sys,time\nlength=0\nwhile True:\n    line=sys.stdin.buffer.readline()\n    if line in (b'\\r\\n',b'\\n'): break\n    if line.lower().startswith(b'content-length:'): length=int(line.split(b':',1)[1])\nsys.stdin.buffer.read(length)\nbody=b'{\"jsonrpc\":\"2.0\",\"method\":\"hold\",\"params\":{}}'\nsys.stdout.buffer.write(('Content-Length: %d\\r\\n\\r\\n'%len(body)).encode()+body)\nsys.stdout.buffer.flush()\ntime.sleep(60)\n";
        let program = Path::new(if cfg!(windows) { "python" } else { "python3" });
        let process =
            match ServerProcess::spawn(program, &["-I", "-S", "-u", "-c", script], &std::env::temp_dir(), |_| {}) {
                Ok(p) => p,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                Err(e) => panic!("{e}"),
            };
        let gate = release.clone();
        let (entered, received) = std::sync::mpsc::channel();
        process.client.on_notification("hold", move |_| {
            entered.send(()).unwrap();
            let (lock, condition) = &*gate;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = condition.wait(released).unwrap();
            }
        });
        process.client.notify("hold", serde_json::Value::Null);
        received.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        let mut pending = process.client.request("never", serde_json::Value::Null);
        process.stop();
        let result = pending.try_recv().unwrap();
        *release.0.lock().unwrap() = true;
        release.1.notify_all();
        assert!(matches!(result, Some(Err(_))), "stop left cancellation to the reader thread");
    }

    #[test]
    fn finds_the_langserver_in_the_tools_environment() {
        let dir = tempfile::tempdir().expect("temp");
        assert_eq!(find_langserver(None, dir.path()).filter(|p| p.starts_with(dir.path())), None);
        let s = scripts_dir(&tools_env(dir.path()));
        std::fs::create_dir_all(&s).expect("mkdir");
        std::fs::write(s.join(exe(LANGSERVER)), "").expect("write");
        let found = find_langserver(None, dir.path()).expect("found");
        // PATH may have one too; the tools one is found when PATH has none.
        assert!(found.ends_with(exe(LANGSERVER)));
    }

    #[test]
    fn talks_to_a_real_basedpyright_when_installed() {
        let data = std::env::temp_dir();
        let Some(server) = find_langserver(None, &data) else { return };
        let dir = tempfile::tempdir().expect("temp");
        std::fs::write(dir.path().join("a.py"), "x: int = 'nope'\n").expect("write");
        let p = ServerProcess::spawn(&server, &["--stdio"], dir.path(), |_| {}).expect("spawn");
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = Mutex::new(tx);
        let s = crate::server::LspServer::new(p.client.clone(), crate::server::python_config(), move || {
            let _ = tx.lock().map(|t| t.send(()));
        });
        futures::executor::block_on(s.start(&dir.path().to_string_lossy(), serde_json::json!({}))).expect("initialize");
        assert!(s.is_started());
        let path = dir.path().join("a.py").to_string_lossy().into_owned();
        let text = "x: int = 'nope'\nimport os\nos.pa\n".to_owned();
        s.sync_docs(&[(path.clone(), "python".into(), text)]);
        // Diagnostics arrive by themselves: the type error on line 1.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut diags = Vec::new();
        while std::time::Instant::now() < deadline && diags.is_empty() {
            let _ = rx.recv_timeout(std::time::Duration::from_millis(500));
            diags = s.diagnostics_for(&path).into_iter().filter(|d| d.start.line == 0).collect();
        }
        assert!(!diags.is_empty(), "no diagnostic for the bad assignment");
        assert_eq!(diags[0].severity, 1);
        let hover =
            futures::executor::block_on(s.hover(&path, crate::navigation::LspPos::new(1, 8))).expect("hover on os");
        assert!(hover.contains("os"), "{hover}");
        let (items, _) = futures::executor::block_on(s.completion(&path, crate::navigation::LspPos::new(2, 5), None));
        assert!(items.iter().any(|i| i.label == "path"), "no os.path in completions");
    }
}
