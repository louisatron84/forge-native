//! Debugpy sessions and debugger UI. Replaces the debugger orchestration in main.ts.
use super::Workspace;
use crate::palette::Choice;
use crate::panels::debug::{DebugEvent, DebugPanel, Variable};
use crate::panels::terminal::Program;
use forge_dap::breakpoints::{Breakpoint, Breakpoints};
use forge_dap::{Adapter, Client};
use futures::StreamExt;
use gpui_kit::{Context, Entity, Task, Window};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

type Sender = futures::channel::mpsc::UnboundedSender<(u64, Value)>;

#[derive(Default)]
pub(crate) struct DebugState {
    pub breakpoints: Breakpoints,
    pub watches: Vec<String>,
    pub filters: Vec<String>,
    sessions: BTreeMap<u64, Adapter>,
    starting: std::collections::BTreeSet<u64>,
    pub active: u64,
    thread: Option<i64>,
    frame: Option<i64>,
    frames: Vec<Value>,
    pub stopped: Option<(String, usize)>,
    paused: bool,
    generation: u64,
    next_session: u64,
    tx: Option<Sender>,
    pump: Option<Task<()>>,
    launch: Option<Value>,
    pub running: bool,
}

fn initialize(client: &Client) -> Result<(), String> {
    client.request("initialize", json!({"clientID":"forge","adapterID":"python","pathFormat":"path","linesStartAt1":true,"columnsStartAt1":true,"supportsVariableType":true,"supportsRunInTerminalRequest":true,"supportsStartDebuggingRequest":true,"supportsInvalidatedEvent":true}))?;
    Ok(())
}

fn launch(client: &Client, command: &str, arguments: Value, tx: Sender, session: u64) -> Result<(), String> {
    let (_, response) = client.send_request(command, arguments)?;
    std::thread::spawn(move || {
        let error = match response.recv_timeout(std::time::Duration::from_secs(60)) {
            Ok(Ok(_)) => None,
            Ok(Err(error)) => Some(error),
            Err(error) => Some(format!("Debugger launch did not finish: {error}")),
        };
        if let Some(error) = error {
            let _ = tx.unbounded_send((session, json!({"event":"launchError","body":{"message":error}})));
        }
    });
    Ok(())
}

impl Workspace {
    pub(crate) fn start_debug(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.debug.running {
            return self.debug_control("continue", window, cx);
        }
        let spec = match self.launch_spec(cx) {
            Ok(s) => s,
            Err(e) => return self.alert(e, window, cx),
        };
        if !self.save_all_dirty(window, cx) {
            return;
        }
        let root = self.root.clone().unwrap_or_else(|| PathBuf::from(&spec.cwd));
        let configured = self.effective_settings(cx).python_path;
        let Some(python) = forge_run::python::find_python(&root, Some(configured.as_str()).filter(|s| !s.is_empty()))
        else {
            return self.alert("Python was not found. Select an interpreter first.", window, cx);
        };
        let mut arguments = json!({"name":"Python","type":"python","request":"launch","cwd":spec.cwd,"args":spec.args,"env":forge_run::env::process_env(Some(&root), &spec.env),"python":[python.clone()],"console":"integratedTerminal","justMyCode":false,"subProcess":true});
        if let Some(program) = spec.program {
            arguments["program"] = json!(program);
        }
        if let Some(module) = spec.module {
            arguments["module"] = json!(module);
        }
        self.debug.generation += 1;
        let generation = self.debug.generation;
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        self.debug.tx = Some(tx.clone());
        self.debug.next_session = 1;
        self.debug.active = 0;
        self.debug.running = true;
        self.debug.starting.insert(0);
        self.debug.launch = Some(arguments.clone());
        self.tasks.begin_debug(self.effective_settings(cx).switch_to_debug_task);
        self.reveal_panel("debug", cx);
        self.debug_panel.update(cx, |p, cx| {
            p.status = "Starting debugpy…".into();
            p.variables.clear();
            p.frames.clear();
            cx.notify();
        });
        self.debug.pump = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some((id, event)) = rx.next().await {
                if this
                    .update_in(cx, |ws, window, cx| {
                        if ws.debug.generation == generation {
                            ws.debug_event(id, event, window, cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
        let event_tx = tx.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move {
                let adapter = Adapter::spawn(&python, &root, move |event| { let _ = event_tx.unbounded_send((0, event)); })?;
                initialize(&adapter.client)?;
                launch(&adapter.client, "launch", arguments, tx, 0)?;
                Ok::<_, String>(adapter)
            }).await;
            this.update_in(cx, |ws, window, cx| {
                if ws.debug.generation != generation { return; }
                match result {
                    Ok(adapter) => { ws.debug.starting.remove(&0); ws.debug.sessions.insert(0, adapter); ws.refresh_debug_panel(cx); }
                    Err(error) => { ws.stop_debug(cx); ws.alert(format!("{error}\n\nInstall debugpy into the selected Python environment: python -m pip install debugpy"), window, cx); }
                }
            }).ok();
        }).detach();
    }

    pub(crate) fn stop_debug(&mut self, cx: &mut Context<Self>) {
        self.debug.generation += 1;
        let adapters = std::mem::take(&mut self.debug.sessions);
        for (_, adapter) in adapters {
            cx.background_executor()
                .spawn(async move {
                    let _ = adapter.client.request("disconnect", json!({"terminateDebuggee":true}));
                    drop(adapter);
                })
                .detach();
        }
        self.debug.running = false;
        self.tasks.end_debug();
        self.layouts_changed(cx);
        self.debug.paused = false;
        self.debug.starting.clear();
        self.debug.stopped = None;
        self.debug.thread = None;
        self.debug.frame = None;
        self.debug.tx = None;
        self.debug.pump = None;
        self.debug_panel.update(cx, |p, cx| {
            p.status = "Not debugging".into();
            p.sessions.clear();
            p.threads.clear();
            p.frames.clear();
            p.variables.clear();
            cx.notify();
        });
        self.refresh_debug_markers(cx);
    }

    fn debug_request(
        &mut self,
        id: u64,
        command: &str,
        arguments: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, Value, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let Some(client) = self.debug.sessions.get(&id).map(|s| s.client.clone()) else { return };
        let command = command.to_owned();
        let generation = self.debug.generation;
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move { client.request(&command, arguments) }).await;
            this.update_in(cx, |ws, window, cx| {
                if ws.debug.generation != generation || !ws.debug.sessions.contains_key(&id) {
                    return;
                }
                match result {
                    Ok(body) => done(ws, body, window, cx),
                    Err(error) => ws.debug_panel.update(cx, |p, cx| p.append(&error, cx)),
                }
            })
            .ok();
        })
        .detach();
    }

    fn configure_debug(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(client) = self.debug.sessions.get(&id).map(|s| s.client.clone()) else {
            // initialized can arrive before the spawn task stores the adapter.
            let tx = self.debug.tx.clone();
            cx.spawn(async move |_, cx| {
                cx.background_executor().timer(std::time::Duration::from_millis(20)).await;
                if let Some(tx) = tx {
                    let _ = tx.unbounded_send((id, json!({"event":"initialized"})));
                }
            })
            .detach();
            return;
        };
        let breakpoints = self.debug.breakpoints.saved.clone();
        let filters =
            if self.debug.filters.is_empty() { vec!["uncaught".to_owned()] } else { self.debug.filters.clone() };
        self.debug_request(id, "setExceptionBreakpoints", json!({"filters":filters}), window, cx, |_, _, _, _| {});
        let tx = self.debug.tx.clone();
        cx.background_executor().spawn(async move {
            let mut files: BTreeMap<String, Vec<Value>> = BTreeMap::new();
            for bp in breakpoints.into_iter().filter(|b| b.enabled) {
                files.entry(bp.path).or_default().push(json!({"line":bp.line + 1,"condition":bp.condition,"hitCondition":bp.hit_condition,"logMessage":bp.log_message}));
            }
            let result = (|| {
                for (path, bps) in files { client.request("setBreakpoints", json!({"source":{"path":path},"breakpoints":bps}))?; }
                client.request("configurationDone", json!({}))?;
                Ok::<_, String>(())
            })();
            if let (Err(error), Some(tx)) = (result, tx) { let _ = tx.unbounded_send((id, json!({"event":"launchError","body":{"message":error}}))); }
        }).detach();
    }

    fn debug_event(&mut self, id: u64, event: Value, window: &mut Window, cx: &mut Context<Self>) {
        if !self.debug.sessions.contains_key(&id) && self.debug.starting.contains(&id) {
            let tx = self.debug.tx.clone();
            cx.spawn(async move |_, cx| {
                cx.background_executor().timer(std::time::Duration::from_millis(20)).await;
                if let Some(tx) = tx {
                    let _ = tx.unbounded_send((id, event));
                }
            })
            .detach();
            return;
        }
        if !self.debug.sessions.contains_key(&id) {
            return;
        }
        if event["type"] == "request" {
            if event["command"] == "runInTerminal" {
                let args: Vec<String> = event["arguments"]["args"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|a| a.as_str().map(str::to_owned))
                    .collect();
                let cwd = PathBuf::from(event["arguments"]["cwd"].as_str().unwrap_or("."));
                let env = event["arguments"]["env"]
                    .as_object()
                    .map(|e| e.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned()))).collect())
                    .unwrap_or_default();
                self.reveal_panel("terminal", cx);
                self.terminal.update(cx, |p, cx| {
                    p.run_program("Python Debug", Program { args, env }, cwd, cx);
                });
                if let Some(s) = self.debug.sessions.get(&id) {
                    s.client.respond(&event, Ok(json!({})));
                }
            } else if event["command"] == "startDebugging" {
                let config = event["arguments"]["configuration"].clone();
                self.debug_attach(config, window, cx);
                if let Some(s) = self.debug.sessions.get(&id) {
                    s.client.respond(&event, Ok(json!({})));
                }
            } else if let Some(s) = self.debug.sessions.get(&id) {
                s.client.respond(&event, Err("Unsupported debugger request".into()));
            }
            return;
        }
        match event["event"].as_str().unwrap_or("") {
            "initialized" => self.configure_debug(id, window, cx),
            "stopped" => {
                self.debug.paused = true;
                self.debug.active = id;
                self.debug.thread = event["body"]["threadId"].as_i64();
                self.debug_panel.update(cx, |p, cx| {
                    p.status = format!("Paused: {}", event["body"]["reason"].as_str().unwrap_or("breakpoint"));
                    cx.notify();
                });
                self.debug_threads(id, window, cx);
            }
            "continued" => {
                if id != self.debug.active {
                    return;
                }
                self.debug.paused = false;
                self.debug.stopped = None;
                self.debug.frame = None;
                self.debug_panel.update(cx, |p, cx| {
                    p.status = "Running".into();
                    p.frames.clear();
                    p.variables.clear();
                    cx.notify();
                });
                self.refresh_debug_markers(cx);
            }
            "output" => {
                self.debug_panel.update(cx, |p, cx| p.append(event["body"]["output"].as_str().unwrap_or(""), cx))
            }
            "debugpyAttach" => self.debug_attach(event["body"].clone(), window, cx),
            "launchError" => {
                self.debug_panel.update(cx, |p, cx| {
                    p.append(event["body"]["message"].as_str().unwrap_or("Debug launch failed"), cx)
                });
                self.stop_debug(cx);
            }
            "terminated" | "adapterExited" => {
                self.debug.sessions.remove(&id);
                if self.debug.sessions.is_empty() {
                    self.stop_debug(cx);
                } else {
                    self.debug.active = *self.debug.sessions.keys().next().unwrap_or(&0);
                    self.refresh_debug_panel(cx);
                }
            }
            _ => {}
        }
    }

    fn debug_attach(&mut self, configuration: Value, window: &mut Window, cx: &mut Context<Self>) {
        let connect = configuration.get("connect").unwrap_or(&configuration);
        let host = connect["host"].as_str().unwrap_or("127.0.0.1").to_owned();
        let Some(port) = connect["port"].as_u64().and_then(|p| u16::try_from(p).ok()) else { return };
        let id = self.debug.next_session;
        self.debug.next_session += 1;
        self.debug.starting.insert(id);
        let Some(tx) = self.debug.tx.clone() else { return };
        let events = tx.clone();
        let generation = self.debug.generation;
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let adapter = Adapter::connect(&host, port, move |e| {
                        let _ = events.unbounded_send((id, e));
                    })?;
                    initialize(&adapter.client)?;
                    launch(&adapter.client, "attach", configuration, tx, id)?;
                    Ok::<_, String>(adapter)
                })
                .await;
            this.update_in(cx, |ws, _, cx| {
                if ws.debug.generation != generation {
                    return;
                }
                ws.debug.starting.remove(&id);
                match result {
                    Ok(adapter) => {
                        ws.debug.sessions.insert(id, adapter);
                        ws.refresh_debug_panel(cx);
                    }
                    Err(e) => ws.debug_panel.update(cx, |p, cx| p.append(&e, cx)),
                }
            })
            .ok();
        })
        .detach();
    }

    fn debug_threads(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.debug_request(id, "threads", json!({}), window, cx, move |ws, body, window, cx| {
            let threads: Vec<(i64, String)> = body["threads"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| Some((t["id"].as_i64()?, t["name"].as_str().unwrap_or("Thread").to_owned())))
                .collect();
            ws.debug.thread = ws.debug.thread.or_else(|| threads.first().map(|t| t.0));
            ws.debug_panel.update(cx, |p, cx| {
                p.threads = threads;
                cx.notify();
            });
            if let Some(thread) = ws.debug.thread {
                ws.debug_stack(id, thread, window, cx);
            }
        });
    }

    fn debug_stack(&mut self, id: u64, thread: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.debug.thread = Some(thread);
        self.debug_request(
            id,
            "stackTrace",
            json!({"threadId":thread,"startFrame":0,"levels":100}),
            window,
            cx,
            move |ws, body, window, cx| {
                if !ws.debug.paused || ws.debug.active != id || ws.debug.thread != Some(thread) {
                    return;
                }
                ws.debug.frames = body["stackFrames"].as_array().cloned().unwrap_or_default();
                let frames = ws
                    .debug
                    .frames
                    .iter()
                    .filter_map(|f| {
                        Some((
                            f["id"].as_i64()?,
                            format!(
                                "{} — {}:{}",
                                f["name"].as_str().unwrap_or("?"),
                                f["source"]["name"].as_str().unwrap_or("?"),
                                f["line"]
                            ),
                        ))
                    })
                    .collect();
                ws.debug_panel.update(cx, |p, cx| {
                    p.frames = frames;
                    cx.notify();
                });
                if let Some(frame) = ws.debug.frames.first().and_then(|f| f["id"].as_i64()) {
                    ws.debug_frame(frame, window, cx);
                }
            },
        );
    }

    fn debug_frame(&mut self, frame: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.debug.frame = Some(frame);
        let id = self.debug.active;
        if let Some(f) = self.debug.frames.iter().find(|f| f["id"] == frame) {
            if let Some(path) = f["source"]["path"].as_str() {
                let path = path.to_owned();
                let line = f["line"].as_u64().unwrap_or(1).saturating_sub(1) as usize;
                self.debug.stopped = Some((path.clone(), line));
                self.open_path_at(PathBuf::from(path), Some(line), window, cx);
                self.refresh_debug_markers(cx);
            }
        }
        self.debug_request(id, "scopes", json!({"frameId":frame}), window, cx, move |ws, body, window, cx| {
            if ws.debug.frame != Some(frame) {
                return;
            }
            let scopes: Vec<Variable> = body["scopes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|s| Variable {
                    name: s["name"].as_str().unwrap_or("Scope").into(),
                    value: String::new(),
                    reference: s["variablesReference"].as_i64().unwrap_or(0),
                    depth: 0,
                })
                .collect();
            let first = scopes.first().map(|s| s.reference);
            ws.debug_panel.update(cx, |p, cx| {
                p.variables = scopes;
                cx.notify();
            });
            if let Some(reference) = first {
                ws.debug_expand(reference, 0, window, cx);
            }
        });
        self.debug_watches(window, cx);
    }

    fn debug_expand(&mut self, reference: i64, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if reference <= 0 {
            return;
        }
        let panel = self.debug_panel.read(cx);
        let Some(parent) = panel.variables.get(index) else { return };
        let depth = parent.depth;
        if panel.variables.get(index + 1).is_some_and(|v| v.depth > depth) {
            self.debug_panel.update(cx, |p, cx| {
                let end = p
                    .variables
                    .iter()
                    .enumerate()
                    .skip(index + 1)
                    .find(|(_, v)| v.depth <= depth)
                    .map(|(i, _)| i)
                    .unwrap_or(p.variables.len());
                p.variables.drain(index + 1..end);
                cx.notify();
            });
            return;
        }
        let frame = self.debug.frame;
        let id = self.debug.active;
        self.debug_request(
            id,
            "variables",
            json!({"variablesReference":reference,"start":0,"count":1000}),
            window,
            cx,
            move |ws, body, _, cx| {
                if ws.debug.frame != frame {
                    return;
                }
                let vars = body["variables"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(1000)
                    .map(|v| Variable {
                        name: v["name"].as_str().unwrap_or("").into(),
                        value: v["value"].as_str().unwrap_or("").into(),
                        reference: v["variablesReference"].as_i64().unwrap_or(0),
                        depth: depth + 1,
                    })
                    .collect::<Vec<_>>();
                ws.debug_panel.update(cx, |p, cx| {
                    if p.variables.get(index).is_some_and(|v| v.reference == reference) {
                        p.variables.splice(index + 1..index + 1, vars);
                        cx.notify();
                    }
                });
            },
        );
    }

    fn debug_watches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let watches = self.debug.watches.clone();
        let frame = self.debug.frame;
        let id = self.debug.active;
        self.debug_panel.update(cx, |p, cx| {
            p.watches = watches
                .iter()
                .map(|s| (s.clone(), if frame.is_some() { "…" } else { "Not paused" }.into(), 0))
                .collect();
            cx.notify();
        });
        let Some(frame) = frame else { return };
        let Some(client) = self.debug.sessions.get(&id).map(|s| s.client.clone()) else { return };
        let generation = self.debug.generation;
        for (index, expression) in watches.into_iter().enumerate() {
            let client = client.clone();
            cx.spawn_in(window, async move |this, cx| {
                let expr = expression.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        client.request("evaluate", json!({"expression":expr,"frameId":frame,"context":"watch"}))
                    })
                    .await;
                this.update(cx, |ws, cx| {
                    if ws.debug.generation != generation || ws.debug.frame != Some(frame) || ws.debug.active != id {
                        return;
                    }
                    ws.debug_panel.update(cx, |p, cx| {
                        if let Some(w) = p.watches.get_mut(index).filter(|w| w.0 == expression) {
                            match result {
                                Ok(body) => {
                                    w.1 = body["result"].as_str().unwrap_or("").into();
                                    w.2 = body["variablesReference"].as_i64().unwrap_or(0);
                                }
                                Err(error) => {
                                    w.1 = error;
                                    w.2 = 0;
                                }
                            }
                            cx.notify();
                        }
                    });
                })
                .ok();
            })
            .detach();
        }
    }

    pub(crate) fn debug_control(&mut self, command: &str, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.debug.active;
        if command == "pause" {
            self.debug_request(id, "threads", json!({}), window, cx, move |ws, body, window, cx| {
                if let Some(thread) =
                    body["threads"].as_array().and_then(|threads| threads.first()).and_then(|t| t["id"].as_i64())
                {
                    ws.debug_request(id, "pause", json!({"threadId":thread}), window, cx, |_, _, _, _| {});
                }
            });
            return;
        }
        if !self.debug.paused {
            return;
        }
        let thread = self.debug.thread;
        self.debug_request(id, command, json!({"threadId":thread}), window, cx, |_, _, _, _| {});
    }

    pub(crate) fn on_debug_event(
        &mut self,
        _: &Entity<DebugPanel>,
        event: &DebugEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            DebugEvent::Watch(index) => {
                let watch = self.debug_panel.read(cx).watches.get(*index).cloned();
                if let Some((name, value, reference)) = watch.filter(|(_, _, reference)| *reference > 0) {
                    self.debug_panel.update(cx, |p, cx| {
                        p.variables = vec![Variable { name, value, reference, depth: 0 }];
                        cx.notify();
                    });
                    self.debug_expand(reference, 0, window, cx);
                }
            }
            DebugEvent::Command(command) => self.debug_command(command, window, cx),
            DebugEvent::Session(id) => {
                self.debug.active = *id;
                self.debug.thread = None;
                self.debug_threads(*id, window, cx);
                self.refresh_debug_panel(cx);
            }
            DebugEvent::Thread(thread) => self.debug_stack(self.debug.active, *thread, window, cx),
            DebugEvent::Frame(frame) => self.debug_frame(*frame, window, cx),
            DebugEvent::Expand(reference, index) => self.debug_expand(*reference, *index, window, cx),
            DebugEvent::Evaluate(expression) => {
                let expression = expression.clone();
                self.debug_panel.update(cx, |p, cx| p.append(&format!("> {expression}"), cx));
                self.debug_request(
                    self.debug.active,
                    "evaluate",
                    json!({"expression":expression,"frameId":self.debug.frame,"context":"repl"}),
                    window,
                    cx,
                    |ws, body, window, cx| {
                        ws.debug_panel.update(cx, |p, cx| p.append(body["result"].as_str().unwrap_or(""), cx));
                        ws.debug_watches(window, cx);
                    },
                );
            }
            DebugEvent::AddWatch => self.palette.update(cx, |p, cx| {
                p.prompt(
                    "Watch expression",
                    "",
                    "Add",
                    |ws, text, window, cx| {
                        if !text.trim().is_empty() {
                            ws.debug.watches.push(text);
                            ws.save_state_soon(cx);
                            ws.debug_watches(window, cx);
                        }
                    },
                    window,
                    cx,
                )
            }),
            DebugEvent::RemoveWatch(index) => {
                if *index < self.debug.watches.len() {
                    self.debug.watches.remove(*index);
                    self.save_state_soon(cx);
                    self.debug_watches(window, cx);
                }
            }
            DebugEvent::ToggleBreakpoint(path, line) => {
                self.debug.breakpoints.toggle(path, *line);
                self.breakpoints_changed(path, cx);
            }
            DebugEvent::EditBreakpoint(path, line) => self.edit_breakpoint(path.clone(), *line, window, cx),
            DebugEvent::ClearConsole => self.debug_panel.update(cx, |p, cx| {
                p.console.clear();
                cx.notify();
            }),
        }
    }

    pub(crate) fn refresh_debug_panel(&mut self, cx: &mut Context<Self>) {
        let sessions = self
            .debug
            .sessions
            .keys()
            .map(|id| (*id, if *id == 0 { "Python".into() } else { format!("Child process {id}") }))
            .collect();
        let bps = self
            .debug
            .breakpoints
            .saved
            .iter()
            .map(|b| {
                (
                    b.path.clone(),
                    b.line as usize,
                    format!(
                        "{}{}",
                        if b.enabled { "●" } else { "○" },
                        b.condition
                            .as_ref()
                            .map(|c| format!(" if {c}"))
                            .or_else(|| b.log_message.as_ref().map(|m| format!(" log: {m}")))
                            .unwrap_or_default()
                    ),
                )
            })
            .collect();
        self.debug_panel.update(cx, |p, cx| {
            p.sessions = sessions;
            p.active_session = self.debug.active;
            p.breakpoints = bps;
            cx.notify();
        });
    }

    pub(crate) fn breakpoints_changed(&mut self, path: &str, cx: &mut Context<Self>) {
        let bps:Vec<Value>=self.debug.breakpoints.for_file(path).into_iter().filter(|b|b.enabled).map(|b|json!({"line":b.line+1,"condition":b.condition,"hitCondition":b.hit_condition,"logMessage":b.log_message})).collect();
        for adapter in self.debug.sessions.values() {
            let client = adapter.client.clone();
            let path = path.to_owned();
            let bps = bps.clone();
            cx.background_executor()
                .spawn(async move {
                    let _ = client.request("setBreakpoints", json!({"source":{"path":path},"breakpoints":bps}));
                })
                .detach();
        }
        self.refresh_debug_markers(cx);
        self.refresh_debug_panel(cx);
        self.save_state_soon(cx);
    }

    pub(crate) fn refresh_debug_markers(&mut self, cx: &mut Context<Self>) {
        for ed in self.editors.clone() {
            let path = ed.read(cx).document().map(|d| d.read(cx).doc.path().to_owned());
            let bps = path
                .as_ref()
                .map(|p| self.debug.breakpoints.for_file(p).iter().map(|b| (b.line as usize, b.enabled)).collect())
                .unwrap_or_default();
            let stopped = self
                .debug
                .stopped
                .as_ref()
                .filter(|(p, _)| path.as_ref().is_some_and(|q| forge_core::tabs::same_path(p, q)))
                .map(|(_, line)| *line);
            ed.update(cx, |e, cx| {
                e.debug_lines = bps;
                e.debug_stopped = stopped;
                cx.notify();
            });
        }
    }

    fn edit_breakpoint(&mut self, path: String, line: usize, window: &mut Window, cx: &mut Context<Self>) {
        let choices = [
            ("Condition", "condition"),
            ("Hit count", "hitCondition"),
            ("Log message", "logMessage"),
            ("Toggle enabled", "enabled"),
        ]
        .into_iter()
        .map(|(label, field)| {
            let path = path.clone();
            Choice::new(label, move |ws, window, cx| {
                let bp = ws.debug.breakpoints.for_file(&path).into_iter().find(|b| b.line == line as u64).unwrap_or(
                    Breakpoint { path: path.clone(), line: line as u64, enabled: true, ..Default::default() },
                );
                if field == "enabled" {
                    ws.debug.breakpoints.set(Breakpoint { enabled: !bp.enabled, ..bp });
                    ws.breakpoints_changed(&path, cx);
                    return;
                }
                let initial = match field {
                    "condition" => bp.condition.clone(),
                    "hitCondition" => bp.hit_condition.clone(),
                    _ => bp.log_message.clone(),
                }
                .unwrap_or_default();
                let path = path.clone();
                ws.palette.update(cx, |p, cx| {
                    p.prompt(
                        label,
                        &initial,
                        "Set",
                        move |ws, text, _, cx| {
                            let mut bp = bp.clone();
                            match field {
                                "condition" => bp.condition = Some(text),
                                "hitCondition" => bp.hit_condition = Some(text),
                                _ => bp.log_message = Some(text),
                            }
                            ws.debug.breakpoints.set(bp);
                            ws.breakpoints_changed(&path, cx);
                        },
                        window,
                        cx,
                    )
                });
            })
        })
        .collect();
        self.choose("Edit breakpoint", choices, window, cx);
    }

    pub(crate) fn debug_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            "debug.start" => self.start_debug(window, cx),
            "debug.stop" => self.stop_debug(cx),
            "debug.restart" => {
                self.stop_debug(cx);
                self.start_debug(window, cx);
            }
            "debug.continue" => self.debug_control("continue", window, cx),
            "debug.pause" => self.debug_control("pause", window, cx),
            "debug.stepOver" => self.debug_control("next", window, cx),
            "debug.stepInto" => self.debug_control("stepIn", window, cx),
            "debug.stepOut" => self.debug_control("stepOut", window, cx),
            "debug.clearConsole" => self.debug_panel.update(cx, |p, cx| {
                p.console.clear();
                cx.notify();
            }),
            "debug.toggleRaised" => {
                if self.debug.filters.iter().any(|f| f == "raised") {
                    self.debug.filters.retain(|f| f != "raised");
                } else {
                    self.debug.filters = vec!["uncaught".into(), "raised".into()];
                }
                let filters = self.debug.filters.clone();
                for adapter in self.debug.sessions.values() {
                    let client = adapter.client.clone();
                    let filters = filters.clone();
                    cx.background_executor()
                        .spawn(async move {
                            let _ = client.request("setExceptionBreakpoints", json!({"filters":filters}));
                        })
                        .detach();
                }
                self.save_state_soon(cx);
                self.status_note(
                    if self.debug.filters.iter().any(|f| f == "raised") {
                        "Pause on all raised exceptions"
                    } else {
                        "Pause on uncaught exceptions"
                    },
                    cx,
                );
            }
            "debug.clearBreakpoints" => {
                let paths: Vec<String> = self.debug.breakpoints.saved.iter().map(|b| b.path.clone()).collect();
                self.debug.breakpoints.saved.clear();
                for path in paths {
                    self.breakpoints_changed(&path, cx);
                }
            }
            "debug.toggleBreakpoint" | "debug.editBreakpoint" => {
                let ed = self.editors[self.tabs.active_group.min(1)].clone();
                let Some(doc) = ed.read(cx).document().cloned() else { return };
                let path = doc.read(cx).doc.path().to_owned();
                let line = ed.read(cx).cursor().line;
                if id == "debug.editBreakpoint" {
                    self.edit_breakpoint(path, line, window, cx);
                } else {
                    self.debug.breakpoints.toggle(&path, line);
                    self.breakpoints_changed(&path, cx);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn debug_hover(&mut self, expression: String, generation: u64, cx: &mut Context<Self>) -> bool {
        let Some(frame) = self.debug.frame else { return false };
        let Some(client) = self.debug.sessions.get(&self.debug.active).map(|s| s.client.clone()) else { return false };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    client.request("evaluate", json!({"expression":expression,"frameId":frame,"context":"hover"}))
                })
                .await;
            this.update(cx, |ws, cx| {
                if ws.hover.generation == generation {
                    if let Ok(value) = result {
                        ws.hover.sections = forge_lsp::completion::hover_sections(&format!(
                            "```python\n{}\n```",
                            value["result"].as_str().unwrap_or("")
                        ));
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
        true
    }
}
