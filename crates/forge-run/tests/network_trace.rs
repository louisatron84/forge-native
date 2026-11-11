//! Real-Python ports of the network cases in `main/runTrace.test.ts`.
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
struct Http {
    port: u16,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Http {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let worker = std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(s) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut headers = Vec::new();
                let mut byte = [0; 1];
                while stream.read(&mut byte).unwrap_or(0) > 0 {
                    headers.push(byte[0]);
                    if headers.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                if headers.is_empty() {
                    continue;
                }
                let headers = String::from_utf8_lossy(&headers);
                let path = headers.split_whitespace().nth(1).unwrap_or("");
                let len = headers
                    .lines()
                    .find_map(|l| {
                        l.to_lowercase().strip_prefix("content-length:").and_then(|n| n.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                let mut body = vec![0; len];
                let _ = stream.read_exact(&mut body);
                let (status, body) = if path.starts_with("/users") {
                    ("200 OK", "[{\"id\":1},{\"id\":2}]")
                } else if path == "/login" {
                    ("201 Created", "ok")
                } else {
                    ("404 Not Found", "not here")
                };
                let _=write!(stream,"HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nSet-Cookie: sid=SECRET4\r\nConnection: close\r\n\r\n{body}",body.len());
            }
        });
        Self { port, stop, worker: Some(worker) }
    }
}
impl Drop for Http {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
fn run(file: &str, expected: &str) -> Option<Vec<Arc<forge_core::network::NetRecord>>> {
    let dir = tempfile::tempdir().unwrap();
    let python = forge_run::python::find_python(dir.path(), None)?;
    if file == "libs.py"
        && !forge_sys::process::run_captured(
            &python,
            &["-c".into(), "import requests,httpx,aiohttp".into()],
            dir.path(),
            &[],
            "",
            Duration::from_secs(15),
        )
        .ok()
    {
        eprintln!("Optional requests/httpx/aiohttp are unavailable; skipping their real-library case.");
        return None;
    }
    let files: serde_json::Value = serde_json::from_str(include_str!("fixtures/network_trace.json")).unwrap();
    let path = dir.path().join(file);
    std::fs::write(&path, files[file].as_str().unwrap()).unwrap();
    let http = Http::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let server = forge_run::trace::TraceServer::open(move |value| {
        let _ = tx.send(value);
    })
    .unwrap();
    let env = server.env(dir.path());
    let env = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect::<Vec<_>>();
    let args = forge_run::trace::trace_command(&[path.to_string_lossy().into()], &[http.port.to_string()]);
    let result = forge_sys::process::run_captured(&python, &args, dir.path(), &env, "", Duration::from_secs(20));
    assert!(result.ok(), "{result:?}");
    assert_eq!(result.stdout, expected);
    assert!(result.stderr.is_empty(), "{}", result.stderr);
    let mut messages = Vec::new();
    let mut records = BTreeMap::new();
    loop {
        let message = rx.recv_timeout(Duration::from_secs(3)).expect("Tracer sends final end");
        let end = message["t"] == "end";
        if let Some(record) = forge_core::network::parse_net(&message) {
            records.insert(record.id, Arc::new(record));
        }
        messages.push(message);
        if end {
            break;
        }
    }
    assert!(!serde_json::to_string(&messages).unwrap().contains("SECRET"));
    Some(records.into_values().collect())
}
#[test]
fn traces_http_errors_bodies_sockets_and_masks_secrets() {
    let Some(records) = run("net.py", "2 201 404 refused\n") else { return };
    assert_eq!(records.len(), 5);
    assert_eq!(
        records.iter().map(|r| (r.method.as_str(), r.status)).collect::<Vec<_>>(),
        vec![("GET", Some(200)), ("POST", Some(201)), ("GET", Some(404)), ("GET", None), ("CONNECT", None)]
    );
    assert_eq!(records[0].lib, "urllib");
    assert!(records[0].url.contains("api_key=••••"));
    assert!(records[0].req_headers.contains(&("Authorization".into(), "••••".into())));
    assert!(records[0].resp_headers.contains(&("Set-Cookie".into(), "••••".into())));
    assert_eq!(records[0].resp_body.as_deref(), Some("[{\"id\":1},{\"id\":2}]"));
    assert!(records[0].ms.unwrap() >= 0.);
    assert!(records[1].req_body.as_deref().unwrap().contains("\"password\": \"••••\""));
    assert!(records[3].error.is_some());
    assert_eq!(records[4].kind, forge_core::network::NetKind::Socket);
    assert_eq!(records[4].func.as_deref(), Some("net:raw"));
}
#[test]
fn traces_requests_httpx_sync_async_and_aiohttp() {
    let Some(records) = run("libs.py", "200 200 404 200\n") else { return };
    assert_eq!(
        records.iter().map(|r| (r.lib.as_str(), r.status, r.func.as_deref().unwrap())).collect::<Vec<_>>(),
        vec![
            ("requests", Some(200), "libs:with_requests"),
            ("httpx", Some(200), "libs:with_httpx"),
            ("httpx", Some(404), "libs:with_httpx_async"),
            ("aiohttp", Some(200), "libs:with_aiohttp")
        ]
    );
    assert_eq!(records[1].resp_body.as_deref(), Some("[{\"id\":1},{\"id\":2}]"));
    assert_eq!(records[3].resp_body.as_deref(), Some("[{\"id\":1},{\"id\":2}]"));
}
