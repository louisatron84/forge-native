//! Running a language server process and piping JSON-RPC to and from it;
//! finding basedpyright. Replaces `src/main/languageServer.ts`.
//!
//! The Electron app bundled basedpyright and ran it with Electron's own
//! Node. The native app has no Node, so it uses basedpyright's Python package
//! (which brings its own Node): from the project's interpreter, PATH, or
//! Forge's own tools environment (installed on request).

use crate::client::LspClient;
use crate::jsonrpc::{encode_message, MessageReader};
use serde_json::Value;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

/// The command that starts basedpyright's language server.
pub const LANGSERVER: &str = "basedpyright-langserver";

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
    child: Arc<Mutex<Child>>,
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
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let mut child = cmd.spawn()?;
        let mut stdin = child.stdin.take().ok_or_else(|| std::io::Error::other("no stdin"))?;
        let mut stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("no stdout"))?;
        let mut stderr = child.stderr.take();
        let (tx, rx) = std::sync::mpsc::channel::<Value>();
        // Writing happens on its own thread, so a busy server never blocks the UI.
        std::thread::spawn(move || {
            while let Ok(m) = rx.recv() {
                if stdin.write_all(&encode_message(&m)).and_then(|_| stdin.flush()).is_err() {
                    break;
                }
            }
        });
        let tx = Mutex::new(tx);
        let client = LspClient::new(Arc::new(move |m| {
            if let Ok(t) = tx.lock() {
                let _ = t.send(m);
            }
        }));
        let child = Arc::new(Mutex::new(child));
        {
            let client = client.clone();
            let child = child.clone();
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
                }
                client.dispose("the language server stopped");
                let code = child.lock().ok().and_then(|mut c| c.wait().ok()).and_then(|s| s.code());
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
        Ok(Self { client, child })
    }

    pub fn stop(&self) {
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
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
