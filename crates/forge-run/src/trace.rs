//! Authenticated loopback stream for the unchanged `runTrace.py` helper.
//! Replaces `src/main/runTrace.ts`; packed bootstrap fits Windows argv limits.
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
pub const TRACE_SCRIPT: &str = include_str!("../../../python/runTrace.py");

pub fn trace_command(target: &[String], args: &[String]) -> Vec<String> {
    use base64::Engine;
    static BOOT: OnceLock<String> = OnceLock::new();
    let boot = BOOT.get_or_init(|| {
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        // The sink is an in-memory Vec, not a fallible file/network stream.
        encoder.write_all(TRACE_SCRIPT.as_bytes()).expect("write to in-memory compression buffer");
        let bytes = encoder.finish().expect("finish in-memory compression buffer");
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        format!(
            "import base64,zlib;exec(compile(zlib.decompress(base64.b64decode('{encoded}')),'<forge-trace>','exec'))"
        )
    });
    let mut result = vec!["-u".into(), "-c".into(), boot.clone()];
    if target.first().is_some_and(|t| t == "-m") {
        result.push("--module".into());
        result.push(target.get(1).cloned().unwrap_or_default());
    } else {
        result.push("--script".into());
        result.push(target.first().cloned().unwrap_or_default());
    }
    result.extend_from_slice(args);
    result
}
pub struct TraceServer {
    pub port: u16,
    pub token: String,
    stop: Arc<AtomicBool>,
    socket: Arc<Mutex<Option<TcpStream>>>,
}
impl TraceServer {
    pub fn open(on_message: impl Fn(Value) + Send + 'static) -> io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let bytes: [u8; 16] = rand::random();
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let stop = Arc::new(AtomicBool::new(false));
        let socket = Arc::new(Mutex::new(None));
        let done = stop.clone();
        let connection = socket.clone();
        let secret = token.clone();
        std::thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => return,
                };
                if stream.set_nonblocking(false).is_err()
                    || stream.set_read_timeout(Some(Duration::from_millis(50))).is_err()
                {
                    continue;
                }
                if let Ok(mut slot) = connection.lock() {
                    *slot = stream.try_clone().ok();
                }
                let mut pending = Vec::new();
                let mut trusted = false;
                let accepted = Instant::now();
                let mut chunk = [0; 8192];
                while !done.load(Ordering::Acquire) {
                    if !trusted && accepted.elapsed() >= Duration::from_secs(1) {
                        break;
                    }
                    let count = match stream.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => continue,
                        Err(_) => break,
                    };
                    let mut search = pending.len();
                    pending.extend_from_slice(&chunk[..count]);
                    if pending.len() > 4_000_000 {
                        break;
                    }
                    let mut cursor = 0;
                    let mut reject = false;
                    while let Some(offset) = pending[search..].iter().position(|b| *b == b'\n') {
                        let end = search + offset;
                        let message = serde_json::from_slice::<Value>(&pending[cursor..end]);
                        cursor = end + 1;
                        search = cursor;
                        let Ok(message) = message else {
                            if !trusted {
                                reject = true;
                                break;
                            }
                            continue;
                        };
                        if !trusted {
                            if message.get("t").and_then(Value::as_str) != Some("hello")
                                || message.get("token").and_then(Value::as_str) != Some(secret.as_str())
                            {
                                reject = true;
                                break;
                            }
                            trusted = true;
                        }
                        let end = message.get("t").and_then(Value::as_str) == Some("end");
                        on_message(message);
                        if end {
                            return;
                        }
                    }
                    if reject {
                        break;
                    }
                    if cursor > 0 {
                        pending.drain(..cursor);
                    }
                    if !trusted && pending.len() > 4096 {
                        break;
                    }
                }
                if let Ok(mut slot) = connection.lock() {
                    *slot = None;
                }
                // One authenticated tracer per run; reject unrelated clients.
                if trusted {
                    return;
                }
            }
        });
        Ok(Self { port, token, stop, socket })
    }
    pub fn env(&self, root: &Path) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("FORGE_TRACE_ROOT".into(), root.to_string_lossy().into_owned()),
            ("FORGE_TRACE_PORT".into(), self.port.to_string()),
            ("FORGE_TRACE_TOKEN".into(), self.token.clone()),
        ])
    }
}
impl Drop for TraceServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(socket) = self.socket.lock() {
            if let Some(socket) = socket.as_ref() {
                let _ = socket.shutdown(Shutdown::Both);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    #[test]
    fn idle_unauthenticated_client_does_not_block_the_real_tracer() {
        let (tx, rx) = std::sync::mpsc::channel();
        let server = TraceServer::open(move |v| {
            let _ = tx.send(v);
        })
        .unwrap();
        let _idle = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, server.port)).unwrap();
        let mut real = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, server.port)).unwrap();
        writeln!(real, "{}", serde_json::json!({"t":"hello","token":server.token})).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(3)).expect("idle client held the listener")["t"], "hello");
    }
    #[test]
    fn authenticates_messages_and_ignores_junk() {
        let (tx, rx) = std::sync::mpsc::channel();
        let server = TraceServer::open(move |v| {
            let _ = tx.send(v);
        })
        .unwrap();
        let mut socket = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, server.port)).unwrap();
        writeln!(socket, "{}", serde_json::json!({"t":"hello","token":server.token})).unwrap();
        socket.write_all(b"not json\n{\"t\":\"tick\",\"at\":null,\"calls\":[]}\n").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap()["t"], "hello");
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap()["t"], "tick");
    }
    #[test]
    fn wrong_tokens_do_not_reach_the_callback() {
        let (tx, rx) = std::sync::mpsc::channel();
        let server = TraceServer::open(move |v| {
            let _ = tx.send(v);
        })
        .unwrap();
        let mut socket = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, server.port)).unwrap();
        socket.write_all(b"{\"t\":\"hello\",\"token\":\"wrong\"}\n{\"t\":\"end\"}\n").unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    }
    #[test]
    fn traced_python_keeps_output_arguments_and_exit_code_and_sends_profile() {
        let dir = tempfile::tempdir().unwrap();
        let Some(python) = crate::python::find_python(dir.path(), None) else {
            return;
        };
        let path = dir.path().join("main.py");
        std::fs::write(
            &path,
            "import sys, time\ndef slow():\n    time.sleep(0.03)\nslow()\nprint('args', sys.argv[1:])\nsys.exit(3)\n",
        )
        .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let server = TraceServer::open(move |v| {
            let _ = tx.send(v);
        })
        .unwrap();
        let args = trace_command(&[path.to_string_lossy().into_owned()], &["hello world".into()]);
        assert!(args.iter().map(String::len).sum::<usize>() < 30_000);
        let mut command = Command::new(python);
        command.args(args).current_dir(dir.path()).envs(server.env(dir.path()));
        forge_sys::process::hide_window(&mut command);
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(3));
        assert!(String::from_utf8_lossy(&output.stdout).contains("args ['hello world']"));
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut messages = Vec::new();
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(v) => {
                    let end = v["t"] == "end";
                    messages.push(v);
                    if end {
                        break;
                    }
                }
                Err(_) => continue,
            }
        }
        assert!(messages.iter().any(|m| m["t"] == "tick"));
        assert!(messages.iter().any(|m| m["t"] == "prof"));
        assert!(messages.iter().any(|m| m["t"] == "end"));
    }
}
