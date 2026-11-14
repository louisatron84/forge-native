//! Debug Adapter Protocol transport and Python adapter processes.
//! Replaces `src/main/debugAdapter.ts` and `src/renderer/debug/client.ts`.
pub mod breakpoints;

use forge_lsp::jsonrpc::{encode_message, MessageReader};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicI64, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::Duration;

type Reply = mpsc::Sender<Result<Value, String>>;
type Event = Arc<dyn Fn(Value) + Send + Sync>;

/// The dotted name under a hovered column, without invoking arbitrary calls.
/// Replaces `expressionAt` in the Electron renderer.
pub fn expression_at(line: &str, column: usize) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut start = column.min(chars.len());
    let mut end = start;
    while start > 0 && (chars[start - 1].is_alphanumeric() || matches!(chars[start - 1], '_' | '.')) {
        start -= 1;
    }
    while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
        end += 1;
    }
    chars[start..end].iter().collect::<String>().trim_start_matches('.').to_owned()
}

pub struct Client {
    sequence: AtomicI64,
    pending: Mutex<HashMap<i64, Reply>>,
    writer: mpsc::Sender<Value>,
    event: Event,
}

impl Client {
    fn new(writer: mpsc::Sender<Value>, event: Event) -> Arc<Self> {
        Arc::new(Self { sequence: AtomicI64::new(1), pending: Mutex::new(HashMap::new()), writer, event })
    }

    /// Blocking: run on a background thread. Every request has a timeout.
    pub fn request(&self, command: &str, arguments: Value) -> Result<Value, String> {
        let (id, reply) = self.send_request(command, arguments)?;
        let response =
            reply.recv_timeout(Duration::from_secs(20)).map_err(|_| format!("Debugger request timed out: {command}"));
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&id);
        }
        response?
    }

    /// Launch/attach answers only after configurationDone. Send without waiting.
    pub fn send_request(
        &self,
        command: &str,
        arguments: Value,
    ) -> Result<(i64, mpsc::Receiver<Result<Value, String>>), String> {
        let id = self.sequence.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().map_err(|_| "Debugger connection unavailable")?.insert(id, tx);
        if self.writer.send(json!({"seq":id,"type":"request","command":command,"arguments":arguments})).is_err() {
            self.closed("Debugger connection closed");
            return Err("Debugger connection closed".into());
        }
        Ok((id, rx))
    }

    pub fn respond(&self, request: &Value, result: Result<Value, String>) {
        let (success, body, message) = match result {
            Ok(body) => (true, body, None),
            Err(e) => (false, Value::Null, Some(e)),
        };
        let _ = self.writer.send(json!({"seq": self.sequence.fetch_add(1, Ordering::SeqCst), "type":"response", "request_seq":request["seq"], "command":request["command"], "success":success, "body":body, "message":message}));
    }

    fn receive(&self, value: Value) {
        if value["type"] == "response" {
            if let Some(id) = value["request_seq"].as_i64() {
                if let Some(tx) = self.pending.lock().ok().and_then(|mut pending| pending.remove(&id)) {
                    let result = if value["success"] == true {
                        Ok(value["body"].clone())
                    } else {
                        Err(value["message"].as_str().unwrap_or("Debugger request failed").to_owned())
                    };
                    let _ = tx.send(result);
                }
            }
        } else {
            (self.event)(value);
        }
    }

    fn closed(&self, message: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            for (_, tx) in pending.drain() {
                let _ = tx.send(Err(message.to_owned()));
            }
        }
    }
}

/// A parent adapter process or a connection to a child debug session.
pub struct Adapter {
    pub client: Arc<Client>,
    child: Option<Child>,
    socket: Option<TcpStream>,
}

impl Adapter {
    pub fn spawn(python: &str, cwd: &Path, event: impl Fn(Value) + Send + Sync + 'static) -> Result<Self, String> {
        let mut command = Command::new(python);
        command
            .args(["-m", "debugpy.adapter"])
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command.spawn().map_err(|e| format!("Cannot start debugpy: {e}"))?;
        let stdin = child.stdin.take().ok_or("No adapter stdin")?;
        let stdout = child.stdout.take().ok_or("No adapter stdout")?;
        if let Some(mut stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                let _ = std::io::copy(&mut stderr, &mut std::io::sink());
            });
        }
        let client = transport(stdout, stdin, Arc::new(event));
        Ok(Self { client, child: Some(child), socket: None })
    }

    pub fn connect(host: &str, port: u16, event: impl Fn(Value) + Send + Sync + 'static) -> Result<Self, String> {
        let address: IpAddr = if host == "localhost" {
            "127.0.0.1".parse().map_err(|_| "Invalid local address")?
        } else {
            host.parse().map_err(|_| "Debugger must use a loopback address")?
        };
        if !address.is_loopback() || port == 0 {
            return Err("Debugger must use a loopback address and a nonzero port".into());
        }
        let stream =
            TcpStream::connect_timeout(&(address, port).into(), Duration::from_secs(10)).map_err(|e| e.to_string())?;
        let writer = stream.try_clone().map_err(|e| e.to_string())?;
        let reader = stream.try_clone().map_err(|e| e.to_string())?;
        Ok(Self { client: transport(reader, writer, Arc::new(event)), child: None, socket: Some(stream) })
    }
}

impl Drop for Adapter {
    fn drop(&mut self) {
        self.client.closed("Debugger stopped");
        if let Some(socket) = &self.socket {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn transport(
    mut reader: impl Read + Send + 'static,
    mut writer: impl Write + Send + 'static,
    event: Event,
) -> Arc<Client> {
    let (tx, rx) = mpsc::channel();
    let client = Client::new(tx, event);
    let weak = Arc::downgrade(&client);
    std::thread::spawn(move || {
        for value in rx {
            if writer.write_all(&encode_message(&value)).and_then(|_| writer.flush()).is_err() {
                break;
            }
        }
    });
    std::thread::spawn(move || {
        let mut frames = MessageReader::new();
        let mut buffer = [0u8; 8192];
        while let Ok(n) = reader.read(&mut buffer) {
            if n == 0 {
                break;
            }
            for value in frames.feed(&buffer[..n]) {
                if let Some(client) = weak.upgrade() {
                    client.receive(value);
                }
            }
            if frames.is_failed() {
                break;
            }
        }
        if let Some(client) = weak.upgrade() {
            client.closed("Debugger connection closed");
            (client.event)(json!({"type":"event","event":"adapterExited"}));
        }
    });
    client
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hovering_an_attribute_evaluates_its_dotted_name() {
        assert_eq!(expression_at("self.items", 7), "self.items");
        assert_eq!(expression_at(".café.nom", 6), "café.nom");
        assert_eq!(expression_at("call(x)", 4), "call");
    }
    #[test]
    fn responses_match_request_sequence_and_errors_reject() {
        let (tx, rx) = mpsc::channel();
        let client = Client::new(tx, Arc::new(|_| {}));
        let (_, reply) = client.send_request("threads", json!({})).unwrap();
        let request = rx.recv().unwrap();
        client.receive(json!({"type":"response","request_seq":request["seq"],"success":true,"body":{"threads":[]}}));
        assert_eq!(reply.recv().unwrap().unwrap(), json!({"threads":[]}));
        let (id, reply) = client.send_request("evaluate", json!({})).unwrap();
        client.receive(json!({"type":"response","request_seq":id,"success":false,"message":"not defined"}));
        assert_eq!(reply.recv().unwrap().unwrap_err(), "not defined");
    }
    #[test]
    fn refusing_remote_debugger_connections() {
        assert!(Adapter::connect("example.com", 1234, |_| {}).is_err());
        assert!(Adapter::connect("0.0.0.0", 1234, |_| {}).is_err());
        assert!(Adapter::connect("127.0.0.1", 0, |_| {}).is_err());
    }

    #[test]
    fn real_debugpy_breakpoint_stack_variables_evaluate_and_step() {
        if !Command::new("python").args(["-c", "import debugpy"]).status().is_ok_and(|s| s.success()) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.py");
        std::fs::write(&path, "x = 1\nx += 2\nprint(x)\n").unwrap();
        let (tx, rx) = mpsc::channel();
        let adapter = Adapter::spawn("python", dir.path(), move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        let client = &adapter.client;
        client.request("initialize", json!({"clientID":"forge-test","adapterID":"python","linesStartAt1":true,"columnsStartAt1":true,"pathFormat":"path"})).unwrap();
        let (_, launched) = client.send_request("launch",json!({"program":path,"cwd":dir.path(),"console":"internalConsole","justMyCode":true,"subProcess":false})).unwrap();
        let wait = |name: &str| {
            let deadline = std::time::Instant::now() + Duration::from_secs(25);
            loop {
                let e = rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())).unwrap();
                if e["event"] == name {
                    return e;
                }
            }
        };
        wait("initialized");
        let bps = client
            .request("setBreakpoints", json!({"source":{"path":path},"breakpoints":[{"line":2,"condition":"x == 1"}]}))
            .unwrap();
        assert_eq!(bps["breakpoints"][0]["verified"], true);
        client.request("setExceptionBreakpoints", json!({"filters":["uncaught"]})).unwrap();
        client.request("configurationDone", json!({})).unwrap();
        launched.recv_timeout(Duration::from_secs(20)).unwrap().unwrap();
        let stopped = wait("stopped");
        let thread = stopped["body"]["threadId"].clone();
        let frames = client.request("stackTrace", json!({"threadId":thread})).unwrap();
        assert_eq!(frames["stackFrames"][0]["line"], 2);
        let frame = frames["stackFrames"][0]["id"].clone();
        let scopes = client.request("scopes", json!({"frameId":frame})).unwrap();
        let reference = scopes["scopes"][0]["variablesReference"].clone();
        let vars = client.request("variables", json!({"variablesReference":reference})).unwrap();
        assert!(vars["variables"].as_array().unwrap().iter().any(|v| v["name"] == "x" && v["value"] == "1"));
        assert_eq!(
            client.request("evaluate", json!({"expression":"x + 10","frameId":frame,"context":"watch"})).unwrap()
                ["result"],
            "11"
        );
        client.request("next", json!({"threadId":thread})).unwrap();
        wait("stopped");
        let frames = client.request("stackTrace", json!({"threadId":thread})).unwrap();
        assert_eq!(frames["stackFrames"][0]["line"], 3);
        client.request("disconnect", json!({"terminateDebuggee":true})).unwrap();
    }

    fn wait_event(rx: &mpsc::Receiver<Value>, name: &str) -> Value {
        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        loop {
            let event = rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())).unwrap();
            if event["event"] == name {
                return event;
            }
        }
    }

    fn initialized(client: &Client) {
        client.request("initialize",json!({"clientID":"forge-test","adapterID":"python","pathFormat":"path","linesStartAt1":true,"columnsStartAt1":true,"supportsRunInTerminalRequest":true})).unwrap();
    }

    #[test]
    fn real_debugpy_attaches_to_a_multiprocessing_child() {
        if !Command::new("python").args(["-c", "import debugpy"]).status().is_ok_and(|s| s.success()) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("child.py");
        std::fs::write(&path,"import multiprocessing\ndef work():\n    value = 7\n    print(value)\nif __name__ == '__main__':\n    p = multiprocessing.Process(target=work)\n    p.start()\n    p.join()\n").unwrap();
        let (tx, rx) = mpsc::channel();
        let parent = Adapter::spawn("python", dir.path(), move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        initialized(&parent.client);
        parent
            .client
            .send_request("launch", json!({"program":path,"console":"internalConsole","subProcess":true}))
            .unwrap();
        wait_event(&rx, "initialized");
        parent.client.request("configurationDone", json!({})).unwrap();
        let config = wait_event(&rx, "debugpyAttach")["body"].clone();
        let host = config["connect"]["host"].as_str().unwrap_or("127.0.0.1");
        let port = config["connect"]["port"].as_u64().unwrap() as u16;
        let (tx, rx) = mpsc::channel();
        let child = Adapter::connect(host, port, move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        initialized(&child.client);
        child.client.send_request("attach", config).unwrap();
        wait_event(&rx, "initialized");
        child
            .client
            .request("setBreakpoints", json!({"source":{"path":path},"breakpoints":[{"line":4,"hitCondition":">= 1"}]}))
            .unwrap();
        child.client.request("configurationDone", json!({})).unwrap();
        let event = wait_event(&rx, "stopped");
        let stack = child.client.request("stackTrace", json!({"threadId":event["body"]["threadId"]})).unwrap();
        let frame = stack["stackFrames"][0]["id"].clone();
        assert_eq!(
            child.client.request("evaluate", json!({"expression":"value","frameId":frame,"context":"watch"})).unwrap()
                ["result"],
            "7"
        );
        child.client.request("disconnect", json!({"terminateDebuggee":true})).unwrap();
        let _ = parent.client.request("disconnect", json!({"terminateDebuggee":true}));
    }

    #[test]
    fn real_debugpy_integrated_terminal_input_and_raised_exception_filter() {
        if !Command::new("python").args(["-c", "import debugpy"]).status().is_ok_and(|s| s.success()) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("input.py");
        std::fs::write(
            &path,
            "name = input('Name: ')\ntry:\n    raise ValueError(name)\nexcept ValueError:\n    print(name)\n",
        )
        .unwrap();
        let (tx, rx) = mpsc::channel();
        let adapter = Adapter::spawn("python", dir.path(), move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        initialized(&adapter.client);
        adapter
            .client
            .send_request("launch", json!({"program":path,"console":"integratedTerminal","subProcess":false}))
            .unwrap();
        let mut terminals = Vec::new();
        loop {
            let e = rx.recv_timeout(Duration::from_secs(25)).unwrap();
            if e["command"] == "runInTerminal" {
                let args = e["arguments"]["args"].as_array().unwrap();
                let mut cmd = Command::new(args[0].as_str().unwrap());
                cmd.args(args[1..].iter().map(|v| v.as_str().unwrap()))
                    .current_dir(e["arguments"]["cwd"].as_str().unwrap())
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                if let Some(env) = e["arguments"]["env"].as_object() {
                    for (key, value) in env {
                        if let Some(value) = value.as_str() {
                            cmd.env(key, value);
                        }
                    }
                }
                let mut process = cmd.spawn().unwrap();
                process.stdin.take().unwrap().write_all(b"Ada\n").unwrap();
                adapter.client.respond(&e, Ok(json!({"processId":process.id()})));
                terminals.push(process);
            }
            if e["event"] == "initialized" {
                break;
            }
        }
        adapter.client.request("setExceptionBreakpoints", json!({"filters":["raised","uncaught"]})).unwrap();
        adapter.client.request("configurationDone", json!({})).unwrap();
        let event = wait_event(&rx, "stopped");
        assert_eq!(event["body"]["reason"], "exception");
        let stack = adapter.client.request("stackTrace", json!({"threadId":event["body"]["threadId"]})).unwrap();
        let frame = stack["stackFrames"][0]["id"].clone();
        assert_eq!(
            adapter.client.request("evaluate", json!({"expression":"name","frameId":frame,"context":"hover"})).unwrap()
                ["result"],
            "'Ada'"
        );
        adapter.client.request("disconnect", json!({"terminateDebuggee":true})).unwrap();
        for mut process in terminals {
            let _ = process.kill();
            let _ = process.wait();
        }
    }
}
