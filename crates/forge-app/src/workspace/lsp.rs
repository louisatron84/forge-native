//! The language server for the open project: started when a Python file is
//! open, kept in sync with the open files, its diagnostics shown in the
//! editors (wavy underlines), the Problems panel and the status bar, and the
//! edits it asks for applied. Replaces the language-server parts of `main.ts`
//! and `src/renderer/lsp/services.ts`.

use super::Workspace;
use crate::app_state;
use crate::editor::EditorDiagnostic;
use forge_core::tabs::same_path;
use forge_lsp::navigation::{apply_edits, FileEdits, LspPos};
use forge_lsp::process::{find_langserver, tools_env, tools_python, ServerProcess};
use forge_lsp::server::{python_config, Diagnostic, LspServer};
use forge_text::chars::{col_from_utf16, utf16_col};
use forge_text::Pos;
use futures::StreamExt;
use gpui_kit::{Context, Task, Window};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum LspStatus {
    #[default]
    Off,
    Starting,
    Running,
    /// basedpyright isn't installed anywhere Forge looks.
    Missing,
    Failed(String),
}

#[derive(Default)]
pub(crate) struct LspState {
    pub server: Option<Arc<LspServer>>,
    process: Option<ServerProcess>,
    pub status: LspStatus,
    /// Goes up on every start, so events from an old server are ignored.
    generation: u64,
    pump: Option<Task<()>>,
    flush_task: Option<Task<()>>,
    refresh_task: Option<Task<()>>,
    restarts: u32,
    /// (errors, warnings) across the project.
    pub counts: (usize, usize),
}

enum LspEvent {
    Diagnostics,
    Exited(u64),
    ApplyEdit(Value, Value),
}

/// A server position (UTF-16 column) as a character position in this line.
pub(crate) fn from_lsp(line_text: &str, p: LspPos) -> Pos {
    Pos::new(p.line, col_from_utf16(line_text, p.col))
}

impl Workspace {
    /// The open Python files the server should know: (path, language id, text).
    fn python_docs(&self, cx: &gpui_kit::App) -> Vec<(String, String, String)> {
        self.tabs
            .docs()
            .iter()
            .filter_map(|id| self.docs.get(id))
            .map(|d| &d.read(cx).doc)
            .filter(|d| !d.large && d.mode().language.server == Some("pyright"))
            .map(|d| (d.path().to_owned(), d.mode().language.lsp_id().to_owned(), d.text()))
            .collect()
    }

    /// The text of an open file (what the server should see).
    pub(crate) fn open_text(&self, path: &str, cx: &gpui_kit::App) -> Option<String> {
        self.docs.values().map(|d| &d.read(cx).doc).find(|d| same_path(d.path(), path)).map(|d| d.text())
    }

    /// Start the server when a Python file is open; keep its open files in sync.
    pub(crate) fn lsp_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.root.is_none() {
            return;
        }
        let docs = self.python_docs(cx);
        match (&self.lsp.server, &self.lsp.status) {
            (Some(server), LspStatus::Running) => server.sync_docs(&docs),
            (None, LspStatus::Off) if !docs.is_empty() => self.start_language_server(window, cx),
            _ => {}
        }
    }

    pub(crate) fn stop_language_server(&mut self, cx: &mut Context<Self>) {
        self.lsp.generation += 1;
        if let Some(s) = self.lsp.server.take() {
            s.stopped();
        }
        if let Some(p) = self.lsp.process.take() {
            p.stop();
        }
        self.lsp.pump = None;
        self.lsp.status = LspStatus::Off;
        self.refresh_diagnostics(cx);
    }

    pub(crate) fn restart_language_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stop_language_server(cx);
        self.lsp.restarts = 0;
        self.start_language_server(window, cx);
    }

    fn start_language_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        self.stop_language_server(cx);
        let generation = self.lsp.generation;
        let configured = Some(self.effective_settings(cx).python_path).filter(|p| !p.is_empty());
        let python = self.pyenv.current.as_ref().map(|i| i.path.clone()).or(configured);
        let data_dir = app_state::state(cx).user.dir().to_path_buf();
        let Some(command) = find_langserver(python.as_deref(), &data_dir) else {
            self.lsp.status = LspStatus::Missing;
            self.refresh_diagnostics(cx);
            return;
        };
        self.lsp.status = LspStatus::Starting;
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<LspEvent>();
        let exit_tx = tx.clone();
        let process = match ServerProcess::spawn(&command, &["--stdio"], &root, move |_| {
            let _ = exit_tx.unbounded_send(LspEvent::Exited(generation));
        }) {
            Ok(p) => p,
            Err(e) => {
                self.lsp.status = LspStatus::Failed(format!("The Python language server couldn't start: {e}"));
                self.refresh_diagnostics(cx);
                return;
            }
        };
        let diag_tx = tx.clone();
        let server = LspServer::new(process.client.clone(), python_config(), move || {
            let _ = diag_tx.unbounded_send(LspEvent::Diagnostics);
        });
        let edit_tx = tx;
        process.client.on_request_deferred("workspace/applyEdit", move |id, params| {
            let _ = edit_tx.unbounded_send(LspEvent::ApplyEdit(id, params));
        });
        self.lsp.server = Some(server.clone());
        self.lsp.process = Some(process);
        self.lsp.pump = Some(cx.spawn_in(window, async move |this, cx| {
            let settings = match &python {
                Some(p) => json!({ "pythonPath": p }),
                None => json!({}),
            };
            let started = server.start(&root.to_string_lossy(), settings).await;
            let ok = this.update_in(cx, |ws, window, cx| {
                if ws.lsp.generation != generation {
                    return false;
                }
                match started {
                    Ok(()) => {
                        ws.lsp.status = LspStatus::Running;
                        ws.lsp_sync(window, cx);
                        ws.refresh_semantic_tokens(cx);
                        ws.outline_file_changed(cx);
                    }
                    Err(e) => ws.lsp.status = LspStatus::Failed(format!("The Python language server didn't start: {e}")),
                }
                ws.refresh_diagnostics(cx);
                true
            });
            if !matches!(ok, Ok(true)) {
                return;
            }
            while let Some(e) = rx.next().await {
                let keep = this.update_in(cx, |ws, window, cx| {
                    if ws.lsp.generation != generation {
                        return false;
                    }
                    ws.on_lsp_event(e, window, cx);
                    true
                });
                if !matches!(keep, Ok(true)) {
                    break;
                }
            }
        }));
        self.refresh_diagnostics(cx);
    }

    fn on_lsp_event(&mut self, e: LspEvent, window: &mut Window, cx: &mut Context<Self>) {
        match e {
            LspEvent::Diagnostics => {
                // Many arrive at once: redraw a moment later.
                if self.lsp.refresh_task.is_none() {
                    self.lsp.refresh_task = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(Duration::from_millis(60)).await;
                        this.update(cx, |ws, cx| {
                            ws.lsp.refresh_task = None;
                            ws.refresh_diagnostics(cx);
                        })
                        .ok();
                    }));
                }
            }
            LspEvent::Exited(g) if g == self.lsp.generation => {
                self.lsp.server = None;
                self.lsp.process = None;
                if self.lsp.restarts < 3 {
                    self.lsp.restarts += 1;
                    self.lsp.status = LspStatus::Off;
                    self.start_language_server(window, cx);
                } else {
                    self.lsp.status = LspStatus::Failed("The Python language server stopped (Restart Python Language Server tries again).".into());
                    self.refresh_diagnostics(cx);
                }
            }
            LspEvent::Exited(_) => {}
            LspEvent::ApplyEdit(id, params) => {
                let edits = forge_lsp::navigation::parse_workspace_edit(params.get("edit").unwrap_or(&Value::Null));
                let applied = self.apply_file_edits(&edits, window, cx).is_ok();
                if let Some(s) = &self.lsp.server {
                    s.client.respond(id, json!({ "applied": applied }));
                }
            }
        }
    }

    /// An open document changed: the server hears about it a moment later.
    pub(crate) fn lsp_doc_changed(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(server) = self.lsp.server.clone() else { return };
        server.doc_changed(path);
        self.lsp.flush_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(150)).await;
            this.update(cx, |ws, cx| {
                ws.lsp_flush(cx);
                ws.refresh_semantic_tokens(cx);
            })
            .ok();
        }));
    }

    /// Send pending edits now (before asking the server about a file).
    pub(crate) fn lsp_flush(&mut self, cx: &mut Context<Self>) {
        self.lsp.flush_task = None;
        let Some(server) = self.lsp.server.clone() else { return };
        if !server.has_pending() {
            return;
        }
        let texts: Vec<(String, String)> = self.docs.values().map(|d| &d.read(cx).doc).map(|d| (d.path().to_owned(), d.text())).collect();
        server.flush(&|p| texts.iter().find(|(q, _)| same_path(q, p)).map(|(_, t)| t.clone()));
    }

    /// Tell the server a file was saved.
    pub(crate) fn lsp_did_save(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(server) = self.lsp.server.clone() else { return };
        let text = self.open_text(path, cx);
        server.did_save(path, &|_| text.clone());
    }

    /// Diagnostics for one file: the server's plus Ruff's (without duplicates).
    pub(crate) fn diagnostics_for(&self, path: &str) -> Vec<Diagnostic> {
        let own = self.lsp.server.as_ref().map(|s| s.diagnostics_for(path)).unwrap_or_default();
        let extra: Vec<Diagnostic> = self.external_diagnostics.iter().filter(|(p, _)| same_path(p, path)).flat_map(|(_, l)| l.clone()).collect();
        forge_lsp::server::merge_diagnostics(&own, &extra)
    }

    /// Show the current diagnostics in the editors, the Problems panel and the status bar.
    pub(crate) fn refresh_diagnostics(&mut self, cx: &mut Context<Self>) {
        for ed in self.editors.clone() {
            let Some(doc) = ed.read(cx).document().cloned() else { continue };
            let path = doc.read(cx).doc.path().to_owned();
            let list: Vec<EditorDiagnostic> = {
                let d = &doc.read(cx).doc;
                self.diagnostics_for(&path)
                    .into_iter()
                    .map(|x| EditorDiagnostic {
                        start: from_lsp(d.buffer.line(x.start.line), x.start),
                        end: from_lsp(d.buffer.line(x.end.line), x.end),
                        severity: x.severity,
                        message: x.message,
                    })
                    .collect()
            };
            ed.update(cx, |e, cx| e.set_diagnostics(list, cx));
        }
        let mut all: Vec<(String, Vec<Diagnostic>)> = self.lsp.server.as_ref().map(|s| s.all_diagnostics()).unwrap_or_default();
        for (p, l) in &self.external_diagnostics {
            match all.iter_mut().find(|(q, _)| same_path(q, p)) {
                Some((_, own)) => *own = forge_lsp::server::merge_diagnostics(own, l),
                None => all.push((p.clone(), l.clone())),
            }
        }
        let root = self.root.clone();
        let status = match &self.lsp.status {
            LspStatus::Starting => "Starting the Python language server…".to_owned(),
            LspStatus::Missing => "Errors, completion and navigation for Python need the language server (basedpyright): click \"Install\" in the status bar.".to_owned(),
            LspStatus::Failed(m) => m.clone(),
            _ => String::new(),
        };
        let counts = self.problems.update(cx, |p, cx| {
            p.status = status;
            p.set(
                all,
                |path| match &root {
                    Some(r) => Path::new(path).strip_prefix(r).map(|x| x.to_string_lossy().replace('\\', "/")).unwrap_or_else(|_| path.to_owned()),
                    None => path.to_owned(),
                },
                cx,
            )
        });
        self.lsp.counts = counts;
        cx.notify();
    }

    /// A position in an open document as the server counts it.
    pub(crate) fn to_lsp(&self, doc: &forge_text::Doc, p: Pos) -> LspPos {
        LspPos::new(p.line, utf16_col(doc.buffer.line(p.line), p.col))
    }

    /// Apply edits across files: open files are edited in the editor (one undo step
    /// each), closed ones on disk. Returns the closed files changed.
    pub(crate) fn apply_file_edits(&mut self, edits: &FileEdits, _window: &mut Window, cx: &mut Context<Self>) -> Result<Vec<String>, String> {
        let mut on_disk = Vec::new();
        for (path, list) in edits {
            let open = self.docs.values().find(|d| same_path(d.read(cx).doc.path(), path)).cloned();
            match open {
                Some(doc) => doc.update(cx, |d, cx| {
                    d.edit(None, cx, |doc| {
                        // Convert to character positions, then apply from the end.
                        let mut converted: Vec<(Pos, Pos, String)> = list
                            .iter()
                            .map(|e| (from_lsp(doc.buffer.line(e.start.line), e.start), from_lsp(doc.buffer.line(e.end.line), e.end), e.text.clone()))
                            .collect();
                        converted.sort_by_key(|c| std::cmp::Reverse(c.0));
                        let first = converted.last().map(|c| c.0).unwrap_or_default();
                        doc.history.transact(&mut doc.buffer, forge_text::EditKind::Other, first, |b| {
                            let mut at = first;
                            for (s, e, t) in &converted {
                                let (s, e) = (b.clamp(*s), b.clamp(*e));
                                b.delete(s, e);
                                at = b.insert(s, t);
                            }
                            at
                        });
                    });
                }),
                None => {
                    let text = std::fs::read_to_string(path).map_err(|e| format!("Couldn't read {path}: {e}"))?;
                    std::fs::write(path, apply_edits(&text, list)).map_err(|e| format!("Couldn't write {path}: {e}"))?;
                    on_disk.push(path.clone());
                }
            }
        }
        if let Some(s) = &self.lsp.server {
            s.files_changed_on_disk(&on_disk);
        }
        self.after_edits_applied(cx);
        Ok(on_disk)
    }

    fn after_edits_applied(&mut self, cx: &mut Context<Self>) {
        for ed in self.editors.clone() {
            ed.update(cx, |_, cx| cx.notify());
        }
        cx.notify();
    }

    /// Install basedpyright into Forge's own tools environment (pip, in the Output panel).
    pub(crate) fn install_language_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let data_dir = app_state::state(cx).user.dir().to_path_buf();
        let env = tools_env(&data_dir);
        let tools_py: PathBuf = tools_python(&data_dir);
        let Some(base) = self.pyenv.current.as_ref().map(|i| i.path.clone()) else {
            return self.alert("Installing the language server needs Python: pick an interpreter first (🐍 in the status bar).", window, cx);
        };
        let install = move |ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>| {
            let after: super::pyenv::AfterRun = Box::new(|ws, code, window, cx| {
                if code == Some(0) {
                    ws.lsp.status = LspStatus::Off;
                    ws.lsp_sync(window, cx);
                }
            });
            ws.run_tool(
                "Installing the Python language server (basedpyright)",
                tools_py.to_string_lossy().into_owned(),
                "pip",
                vec!["install".into(), "--upgrade".into(), "basedpyright".into(), "--disable-pip-version-check".into()],
                Some(after),
                window,
                cx,
            );
        };
        if tools_python(&data_dir).is_file() {
            return install(self, window, cx);
        }
        let after: super::pyenv::AfterRun = Box::new(move |ws, code, window, cx| {
            if code == Some(0) {
                install(ws, window, cx);
            }
        });
        self.run_tool("Creating Forge's tools environment", base, "venv", vec![env.to_string_lossy().into_owned()], Some(after), window, cx);
    }
}

impl Workspace {
    /// Ask the server what each name means in the files on screen (semantic colours).
    pub(crate) fn refresh_semantic_tokens(&mut self, cx: &mut Context<Self>) {
        let Some(server) = self.lsp.server.clone() else { return };
        for ed in self.editors.clone() {
            let Some(doc) = ed.read(cx).document().cloned() else { continue };
            let path = doc.read(cx).doc.path().to_owned();
            if !server.knows(&path) {
                continue;
            }
            let version = doc.read(cx).doc.version;
            let s = server.clone();
            cx.spawn(async move |_, cx| {
                let Some((data, types, mods)) = s.semantic_tokens(&path).await else { return };
                let legend = forge_text::semantic::Legend { token_types: types, token_modifiers: mods };
                let decoded = forge_text::semantic::decode_semantic_tokens(&data, &legend);
                doc.update(cx, |d, cx| {
                    // Edited meanwhile: the next answer will do.
                    if d.doc.version != version {
                        return;
                    }
                    // The server's columns are UTF-16 units: make them characters.
                    let converted = decoded
                        .into_iter()
                        .map(|(line, tokens)| {
                            let text = d.doc.buffer.line(line).to_owned();
                            let tokens = tokens
                                .into_iter()
                                .map(|t| forge_text::Token::new(col_from_utf16(&text, t.start), col_from_utf16(&text, t.end), t.kind))
                                .collect();
                            (line, tokens)
                        })
                        .collect();
                    d.doc.highlighter.set_semantic(Some(converted));
                    cx.notify();
                });
            })
            .detach();
        }
    }
}
