//! One language server: keeps the server's view of open files in sync with
//! the editor, collects its diagnostics, and wraps the requests the IDE uses
//! (completion, hover, definitions, rename...). Nothing here is specific to
//! one language: what differs is in its `ServerConfig` (see `python_config`).
//! Replaces `src/renderer/lsp/server.ts` and `pyright.ts`.

use crate::client::LspClient;
use crate::completion::{to_completion_item, CompletionItem};
use crate::navigation::{
    parse_call_item, parse_calls, parse_highlights, parse_locations, parse_symbols, parse_workspace_edit,
    parse_workspace_symbols, pos, range, CallItem, DocSymbol, FileEdits, Location, LspPos, WorkspaceSymbol,
};
use crate::uri::{path_to_uri, uri_to_path};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub start: LspPos,
    pub end: LspPos,
    /// 1 error, 2 warning, 3 information, 4 hint.
    pub severity: u8,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<String>,
    /// Id of the language server that reported it (None for other tools).
    pub server: Option<String>,
    /// The server's own diagnostic object (sent back when asking for code actions).
    pub raw: Value,
}

/// A code action from the language server (quick fix, organise imports...).
#[derive(Clone, Debug, PartialEq)]
pub struct CodeAction {
    pub title: String,
    pub kind: String,
    pub preferred: bool,
    pub edit: Option<FileEdits>,
    pub command: Option<Value>,
}

/// What differs between language servers.
#[derive(Clone)]
pub struct ServerConfig {
    /// "pyright" (kept for saved settings) and the language ids it handles.
    pub id: &'static str,
    pub name: &'static str,
    pub languages: &'static [&'static str],
    /// Answer workspace/configuration for one section, given the start settings.
    pub configuration: fn(Option<&str>, &Value) -> Value,
}

/// Windows paths compare case-insensitively and ignore slash direction.
pub fn path_key(path: &str) -> String {
    let b = path.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        path.replace('/', "\\").to_lowercase()
    } else {
        path.to_owned()
    }
}

fn python_analysis() -> Value {
    json!({
        "diagnosticMode": "openFilesOnly",
        "autoSearchPaths": true,
        "useLibraryCodeForTypes": true,
        "typeCheckingMode": "standard",
        // Never the "unknown type" family for code without type hints.
        "diagnosticSeverityOverrides": {
            "reportUnknownParameterType": "none",
            "reportUnknownArgumentType": "none",
            "reportUnknownVariableType": "none",
            "reportUnknownMemberType": "none",
            "reportUnknownLambdaType": "none",
            "reportMissingParameterType": "none",
            "reportMissingTypeArgument": "none",
            "reportAny": "none",
            "reportExplicitAny": "none"
        }
    })
}

fn python_configuration(section: Option<&str>, settings: &Value) -> Value {
    let analysis = python_analysis();
    match section {
        Some("python") => {
            let mut v = json!({ "analysis": analysis });
            if let Some(p) = settings.get("pythonPath").and_then(Value::as_str) {
                v["pythonPath"] = json!(p);
            }
            v
        }
        Some("python.analysis" | "basedpyright.analysis") => analysis,
        Some("basedpyright") => json!({ "disableLanguageServices": false, "analysis": analysis }),
        _ => json!({}),
    }
}

/// basedpyright: Pyright plus semantic highlighting. Its checks are Pyright's
/// 'standard' ones (basedpyright's default warns about every value whose type
/// it can't work out, which is most code written without type hints).
pub fn python_config() -> ServerConfig {
    ServerConfig { id: "pyright", name: "basedpyright", languages: &["python"], configuration: python_configuration }
}

#[derive(Default)]
struct State {
    started: bool,
    settings: Value,
    capabilities: Value,
    /// Open documents (by path key): path and the version last sent.
    open: HashMap<String, (String, i64)>,
    pending: HashSet<String>,
}

type DiagnosticsMap = HashMap<String, (String, Vec<Diagnostic>)>;

pub struct LspServer {
    pub client: Arc<LspClient>,
    pub config: ServerConfig,
    state: Mutex<State>,
    diagnostics: Arc<Mutex<DiagnosticsMap>>,
}

/// Reply future: Ok(result) or Err(message).
pub type Reply = futures::channel::oneshot::Receiver<Result<Value, String>>;

async fn answer(r: Reply) -> Value {
    r.await.ok().and_then(Result::ok).unwrap_or(Value::Null)
}

impl LspServer {
    /// Wrap a client; `on_diagnostics` is called (on the reader thread) when diagnostics change.
    pub fn new(
        client: Arc<LspClient>,
        config: ServerConfig,
        on_diagnostics: impl Fn() + Send + Sync + 'static,
    ) -> Arc<Self> {
        let server = Arc::new(Self {
            client: client.clone(),
            config,
            state: Mutex::new(State::default()),
            diagnostics: Arc::new(Mutex::new(HashMap::new())),
        });
        let weak = Arc::downgrade(&server);
        client.on_request("workspace/configuration", move |params| {
            let Some(s) = weak.upgrade() else { return Value::Null };
            let settings = s.state.lock().map(|st| st.settings.clone()).unwrap_or_default();
            let items = params.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
            Value::Array(
                items
                    .iter()
                    .map(|i| (s.config.configuration)(i.get("section").and_then(Value::as_str), &settings))
                    .collect(),
            )
        });
        let diags = server.diagnostics.clone();
        let id = server.config.id;
        client.on_notification("textDocument/publishDiagnostics", move |params| {
            let Some(uri) = params.get("uri").and_then(Value::as_str) else { return };
            let path = uri_to_path(uri);
            let list: Vec<Diagnostic> = params
                .get("diagnostics")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|d| {
                    let (start, end) = range(d.get("range"))?;
                    Some(Diagnostic {
                        start,
                        end,
                        severity: d.get("severity").and_then(Value::as_u64).unwrap_or(1).clamp(1, 4) as u8,
                        message: d.get("message").and_then(Value::as_str).unwrap_or("").to_owned(),
                        source: d.get("source").and_then(Value::as_str).map(str::to_owned),
                        code: d
                            .get("code")
                            .filter(|c| !c.is_null())
                            .map(|c| c.as_str().map(str::to_owned).unwrap_or_else(|| c.to_string())),
                        server: Some(id.to_owned()),
                        raw: d.clone(),
                    })
                })
                .collect();
            if let Ok(mut m) = diags.lock() {
                if list.is_empty() {
                    m.remove(&path_key(&path));
                } else {
                    m.insert(path_key(&path), (path, list));
                }
            }
            on_diagnostics();
        });
        server
    }

    pub fn is_started(&self) -> bool {
        self.state.lock().is_ok_and(|s| s.started)
    }

    /// Does this server handle files of this language?
    pub fn handles(&self, language: &str) -> bool {
        self.config.languages.contains(&language)
    }

    /// Talk to a (freshly started) server for a project folder.
    pub async fn start(&self, root: &str, settings: Value) -> Result<(), String> {
        if let Ok(mut s) = self.state.lock() {
            s.settings = settings;
            s.pending.clear();
            s.started = false;
        }
        if let Ok(mut d) = self.diagnostics.lock() {
            d.clear();
        }
        let root_uri = path_to_uri(root);
        let name = root.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(root);
        let reply = self.client.request("initialize", json!({
            "processId": std::process::id(),
            "clientInfo": { "name": "Forge" },
            "rootUri": root_uri,
            "workspaceFolders": [{ "uri": root_uri, "name": name }],
            "capabilities": {
                "workspace": { "configuration": true, "workspaceFolders": true, "workspaceEdit": { "documentChanges": true }, "applyEdit": true, "executeCommand": {} },
                "textDocument": {
                    "synchronization": { "didSave": true, "dynamicRegistration": false },
                    "publishDiagnostics": { "relatedInformation": false },
                    "completion": { "completionItem": { "snippetSupport": false, "documentationFormat": ["plaintext", "markdown"] }, "contextSupport": true },
                    "hover": { "contentFormat": ["markdown", "plaintext"] },
                    "definition": { "linkSupport": false },
                    "references": {},
                    "rename": { "prepareSupport": true },
                    "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
                    "typeDefinition": { "linkSupport": false },
                    "documentHighlight": {},
                    "callHierarchy": {},
                    "semanticTokens": {
                        "requests": { "full": true },
                        "tokenTypes": ["namespace", "type", "class", "enum", "interface", "struct", "typeParameter", "parameter", "variable", "property", "enumMember", "function", "method", "decorator", "module", "selfParameter", "clsParameter"],
                        "tokenModifiers": ["declaration", "definition", "readonly", "static", "async", "defaultLibrary", "builtin"],
                        "formats": ["relative"],
                        "overlappingTokenSupport": false,
                        "multilineTokenSupport": false
                    },
                    "codeAction": { "codeActionLiteralSupport": { "codeActionKind": { "valueSet": ["quickfix", "refactor", "source", "source.organizeImports"] } }, "isPreferredSupport": true }
                }
            }
        }));
        let result = reply.await.map_err(|_| "the language server went away".to_owned())??;
        let docs: Vec<String> = {
            let mut s = self.state.lock().map_err(|_| "lock")?;
            s.capabilities = result.get("capabilities").cloned().unwrap_or_default();
            s.started = true;
            s.open.values().map(|(p, _)| p.clone()).collect()
        };
        self.client.notify("initialized", json!({}));
        self.client.notify("workspace/didChangeConfiguration", json!({ "settings": {} }));
        // Documents tracked before (a restart) are reopened by the next sync.
        if let Ok(mut s) = self.state.lock() {
            for p in docs {
                s.open.remove(&path_key(&p));
            }
        }
        Ok(())
    }

    pub fn capabilities(&self) -> Value {
        self.state.lock().map(|s| s.capabilities.clone()).unwrap_or_default()
    }

    /// Match the server's open documents to these (path, language id, text) — the open
    /// tabs in its languages.
    pub fn sync_docs(&self, docs: &[(String, String, String)]) {
        let Ok(mut s) = self.state.lock() else { return };
        let wanted: HashSet<String> = docs.iter().map(|(p, _, _)| path_key(p)).collect();
        let gone: Vec<(String, String)> =
            s.open.iter().filter(|(k, _)| !wanted.contains(*k)).map(|(k, (p, _))| (k.clone(), p.clone())).collect();
        for (k, p) in gone {
            s.open.remove(&k);
            s.pending.remove(&k);
            if s.started {
                self.client.notify("textDocument/didClose", json!({ "textDocument": { "uri": path_to_uri(&p) } }));
            }
        }
        for (p, lang, text) in docs {
            let k = path_key(p);
            if s.open.contains_key(&k) {
                continue;
            }
            s.open.insert(k, (p.clone(), 1));
            if s.started {
                self.client.notify("textDocument/didOpen", json!({ "textDocument": { "uri": path_to_uri(p), "languageId": lang, "version": 1, "text": text } }));
            }
        }
    }

    /// The editor changed this document: it's sent with the next `flush`.
    pub fn doc_changed(&self, path: &str) {
        if let Ok(mut s) = self.state.lock() {
            let k = path_key(path);
            if s.open.contains_key(&k) {
                s.pending.insert(k);
            }
        }
    }

    pub fn has_pending(&self) -> bool {
        self.state.lock().is_ok_and(|s| !s.pending.is_empty())
    }

    /// Send pending edits now (call before asking the server about a document).
    pub fn flush(&self, text_of: &dyn Fn(&str) -> Option<String>) {
        let Ok(mut s) = self.state.lock() else { return };
        if !s.started {
            return;
        }
        let pending: Vec<String> = s.pending.drain().collect();
        for k in pending {
            let Some((path, version)) = s.open.get(&k).cloned() else { continue };
            let Some(text) = text_of(&path) else { continue };
            s.open.insert(k, (path.clone(), version + 1));
            self.client.notify(
                "textDocument/didChange",
                json!({ "textDocument": { "uri": path_to_uri(&path), "version": version + 1 }, "contentChanges": [{ "text": text }] }),
            );
        }
    }

    pub fn did_save(&self, path: &str, text_of: &dyn Fn(&str) -> Option<String>) {
        if !self.knows(path) {
            return;
        }
        self.flush(text_of);
        self.client.notify("textDocument/didSave", json!({ "textDocument": { "uri": path_to_uri(path) } }));
    }

    /// Is this file open on the server (and the server started)?
    pub fn knows(&self, path: &str) -> bool {
        self.state.lock().is_ok_and(|s| s.started && s.open.contains_key(&path_key(path)))
    }

    fn doc(path: &str) -> Value {
        json!({ "uri": path_to_uri(path) })
    }

    // ---------- requests about a document (call `flush` first) ----------

    pub async fn completion(&self, path: &str, at: LspPos, trigger: Option<char>) -> (Vec<CompletionItem>, bool) {
        if !self.knows(path) {
            return (Vec::new(), false);
        }
        let context = match trigger {
            Some(c) => json!({ "triggerKind": 2, "triggerCharacter": c.to_string() }),
            None => json!({ "triggerKind": 1 }),
        };
        let res = answer(self.client.request(
            "textDocument/completion",
            json!({ "textDocument": Self::doc(path), "position": at.to_json(), "context": context }),
        ))
        .await;
        let (list, incomplete) = match &res {
            Value::Array(a) => (a.clone(), false),
            v => (
                v.get("items").and_then(Value::as_array).cloned().unwrap_or_default(),
                v.get("isIncomplete").and_then(Value::as_bool).unwrap_or(false),
            ),
        };
        (list.iter().map(to_completion_item).collect(), incomplete)
    }

    /// Fill in a suggestion's details/docs (Pyright does this lazily).
    pub async fn resolve_completion(&self, item: CompletionItem) -> CompletionItem {
        let can = self
            .capabilities()
            .get("completionProvider")
            .and_then(|c| c.get("resolveProvider"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !self.is_started() || !can || item.raw.is_null() {
            return item;
        }
        let res = answer(self.client.request("completionItem/resolve", item.raw.clone())).await;
        let extra = crate::completion::additional_edits(&res);
        CompletionItem {
            additional_edits: if extra.is_empty() { item.additional_edits.clone() } else { extra },
            detail: res.get("detail").and_then(Value::as_str).map(str::to_owned).or(item.detail.clone()),
            documentation: res
                .get("documentation")
                .and_then(crate::completion::doc_text)
                .or(item.documentation.clone()),
            ..item
        }
    }

    pub async fn hover(&self, path: &str, at: LspPos) -> Option<String> {
        if !self.knows(path) {
            return None;
        }
        let res = answer(
            self.client
                .request("textDocument/hover", json!({ "textDocument": Self::doc(path), "position": at.to_json() })),
        )
        .await;
        crate::completion::hover_markdown(&res)
    }

    async fn locations(&self, method: &str, path: &str, at: LspPos, extra: Value) -> Vec<Location> {
        if !self.knows(path) {
            return Vec::new();
        }
        let mut params = json!({ "textDocument": Self::doc(path), "position": at.to_json() });
        if let (Some(p), Value::Object(e)) = (params.as_object_mut(), extra) {
            p.extend(e);
        }
        parse_locations(&answer(self.client.request(method, params)).await)
    }

    /// Where the symbol at `at` is defined (first result).
    pub async fn definition(&self, path: &str, at: LspPos) -> Option<Location> {
        let res = self.locations("textDocument/definition", path, at, json!({})).await;
        // Keep the server's order for the first: re-query is not needed, sorting is stable enough.
        res.into_iter().next()
    }

    /// Every use of the symbol at `at` across the project (including its definition).
    pub async fn references(&self, path: &str, at: LspPos) -> Vec<Location> {
        self.locations("textDocument/references", path, at, json!({ "context": { "includeDeclaration": true } })).await
    }

    pub async fn type_definition(&self, path: &str, at: LspPos) -> Vec<Location> {
        self.locations("textDocument/typeDefinition", path, at, json!({})).await
    }

    /// Can the thing at `at` be renamed? Its range, and its name if the server says.
    pub async fn prepare_rename(&self, path: &str, at: LspPos) -> Option<(LspPos, LspPos, Option<String>)> {
        if !self.knows(path) {
            return None;
        }
        let res = answer(self.client.request(
            "textDocument/prepareRename",
            json!({ "textDocument": Self::doc(path), "position": at.to_json() }),
        ))
        .await;
        let r = res.get("range").or(if res.get("start").is_some() { Some(&res) } else { None });
        let (start, end) = range(r)?;
        Some((start, end, res.get("placeholder").and_then(Value::as_str).map(str::to_owned)))
    }

    /// The edits that rename the symbol at `at` to `new_name`, per file.
    pub async fn rename(&self, path: &str, at: LspPos, new_name: &str) -> Result<FileEdits, String> {
        if !self.knows(path) {
            return Ok(FileEdits::new());
        }
        let res = self
            .client
            .request(
                "textDocument/rename",
                json!({ "textDocument": Self::doc(path), "position": at.to_json(), "newName": new_name }),
            )
            .await;
        res.map_err(|_| "the language server went away".to_owned())?.map(|v| parse_workspace_edit(&v))
    }

    /// The classes, functions, methods and variables in a file (the outline).
    pub async fn symbols(&self, path: &str) -> Vec<DocSymbol> {
        if !self.knows(path) {
            return Vec::new();
        }
        parse_symbols(
            &answer(self.client.request("textDocument/documentSymbol", json!({ "textDocument": Self::doc(path) })))
                .await,
        )
    }

    /// Semantic tokens and the legend's type names, if the server provides them.
    pub async fn semantic_tokens(&self, path: &str) -> Option<(Vec<u32>, Vec<String>, Vec<String>)> {
        let caps = self.capabilities();
        let provider = caps.get("semanticTokensProvider")?;
        let legend = provider.get("legend")?;
        provider.get("full").filter(|f| !matches!(f, Value::Bool(false)))?;
        if !self.knows(path) {
            return None;
        }
        let names = |k: &str| -> Vec<String> {
            legend
                .get(k)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())
                .unwrap_or_default()
        };
        let res =
            answer(self.client.request("textDocument/semanticTokens/full", json!({ "textDocument": Self::doc(path) })))
                .await;
        let data: Vec<u32> =
            res.get("data").and_then(Value::as_array)?.iter().filter_map(|n| n.as_u64().map(|n| n as u32)).collect();
        Some((data, names("tokenTypes"), names("tokenModifiers")))
    }

    /// Other uses of the name at `at` in the same file (to highlight them): (start, end, write).
    pub async fn highlights(&self, path: &str, at: LspPos) -> Vec<(LspPos, LspPos, bool)> {
        if !self.knows(path) {
            return Vec::new();
        }
        parse_highlights(
            &answer(self.client.request(
                "textDocument/documentHighlight",
                json!({ "textDocument": Self::doc(path), "position": at.to_json() }),
            ))
            .await,
        )
    }

    /// Classes, functions and variables across the project whose name matches `query`.
    pub async fn workspace_symbols(&self, query: &str) -> Vec<WorkspaceSymbol> {
        if !self.is_started() {
            return Vec::new();
        }
        parse_workspace_symbols(&answer(self.client.request("workspace/symbol", json!({ "query": query }))).await)
    }

    pub async fn prepare_call_hierarchy(&self, path: &str, at: LspPos) -> Vec<CallItem> {
        if !self.knows(path) {
            return Vec::new();
        }
        let res = answer(self.client.request(
            "textDocument/prepareCallHierarchy",
            json!({ "textDocument": Self::doc(path), "position": at.to_json() }),
        ))
        .await;
        res.as_array().into_iter().flatten().filter_map(parse_call_item).collect()
    }

    /// Who calls `item` (incoming) or what it calls (outgoing), with where the calls are.
    pub async fn calls(&self, item: &CallItem, incoming: bool) -> Vec<(CallItem, Vec<LspPos>)> {
        if !self.is_started() {
            return Vec::new();
        }
        let method = if incoming { "callHierarchy/incomingCalls" } else { "callHierarchy/outgoingCalls" };
        parse_calls(
            &answer(self.client.request(method, json!({ "item": item.raw }))).await,
            if incoming { "from" } else { "to" },
        )
    }

    /// Quick fixes and other actions for a range (with the server's diagnostics there).
    pub async fn code_actions(
        &self,
        path: &str,
        start: LspPos,
        end: LspPos,
        diagnostics: &[Diagnostic],
        only: Option<&[&str]>,
    ) -> Vec<CodeAction> {
        if !self.knows(path) {
            return Vec::new();
        }
        let raw: Vec<Value> =
            diagnostics.iter().filter(|d| d.server.as_deref() == Some(self.config.id)).map(|d| d.raw.clone()).collect();
        let mut context = json!({ "diagnostics": raw });
        if let Some(o) = only {
            context["only"] = json!(o);
        }
        let res = answer(self.client.request(
            "textDocument/codeAction",
            json!({ "textDocument": Self::doc(path), "range": { "start": start.to_json(), "end": end.to_json() }, "context": context }),
        ))
        .await;
        let mut out = Vec::new();
        for a in res.as_array().into_iter().flatten() {
            let Some(title) = a.get("title").and_then(Value::as_str) else { continue };
            // A bare Command (no kind/edit) is also allowed.
            if let Some(cmd) = a.get("command").and_then(Value::as_str) {
                out.push(CodeAction {
                    title: title.into(),
                    kind: String::new(),
                    preferred: false,
                    edit: None,
                    command: Some(
                        json!({ "command": cmd, "arguments": a.get("arguments").cloned().unwrap_or(json!([])) }),
                    ),
                });
                continue;
            }
            let edit = a.get("edit").map(parse_workspace_edit);
            let command = a.get("command").filter(|c| c.get("command").is_some()).cloned();
            if edit.is_none() && command.is_none() {
                continue;
            }
            out.push(CodeAction {
                title: title.into(),
                kind: a.get("kind").and_then(Value::as_str).unwrap_or("").into(),
                preferred: a.get("isPreferred").and_then(Value::as_bool).unwrap_or(false),
                edit,
                command,
            });
        }
        out
    }

    /// Run a server command (e.g. from a code action). Edits come back via workspace/applyEdit.
    pub async fn execute_command(&self, command: &Value) {
        if !self.is_started() {
            return;
        }
        let _ = answer(self.client.request(
            "workspace/executeCommand",
            json!({ "command": command.get("command").cloned().unwrap_or_default(), "arguments": command.get("arguments").cloned().unwrap_or(json!([])) }),
        ))
        .await;
    }

    /// Files changed on disk behind the server's back (e.g. a rename edited closed files).
    pub fn files_changed_on_disk(&self, paths: &[String]) {
        if !self.is_started() || paths.is_empty() {
            return;
        }
        let changes: Vec<Value> = paths.iter().map(|p| json!({ "uri": path_to_uri(p), "type": 2 })).collect();
        self.client.notify("workspace/didChangeWatchedFiles", json!({ "changes": changes }));
    }

    // ---------- diagnostics ----------

    pub fn diagnostics_for(&self, path: &str) -> Vec<Diagnostic> {
        self.diagnostics.lock().ok().and_then(|m| m.get(&path_key(path)).map(|d| d.1.clone())).unwrap_or_default()
    }

    /// (path, diagnostics) for every file that has some.
    pub fn all_diagnostics(&self) -> Vec<(String, Vec<Diagnostic>)> {
        self.diagnostics.lock().map(|m| m.values().cloned().collect()).unwrap_or_default()
    }

    /// The server went away: forget it was started (its requests fail).
    pub fn stopped(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.started = false;
        }
        self.client.dispose("the language server stopped");
    }
}

/// Decode `pos` helper kept public for callers that read raw positions.
pub fn lsp_pos(v: &Value) -> Option<LspPos> {
    pos(v)
}

/// Group diagnostics by path, merging another tool's (e.g. Ruff's) minus those a
/// language server already reports at the same place.
pub fn merge_diagnostics(own: &[Diagnostic], extra: &[Diagnostic]) -> Vec<Diagnostic> {
    let at: HashSet<(usize, usize)> =
        own.iter().filter(|d| d.severity <= 3).map(|d| (d.start.line, d.start.col)).collect();
    let mut out = own.to_vec();
    out.extend(extra.iter().filter(|d| !at.contains(&(d.start.line, d.start.col))).cloned());
    out
}

/// (path key -> path) of every file in these lists, for merging.
pub fn by_path(lists: Vec<(String, Vec<Diagnostic>)>) -> BTreeMap<String, (String, Vec<Diagnostic>)> {
    let mut out: BTreeMap<String, (String, Vec<Diagnostic>)> = BTreeMap::new();
    for (p, l) in lists {
        out.entry(path_key(&p)).or_insert_with(|| (p.clone(), Vec::new())).1.extend(l);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// A fake server: records what the client sends and answers `initialize`.
    type Setup = (Arc<LspServer>, Arc<StdMutex<Vec<Value>>>, Arc<StdMutex<usize>>);

    fn setup() -> Setup {
        let sent = Arc::new(StdMutex::new(Vec::new()));
        let client_slot: Arc<StdMutex<Option<Arc<LspClient>>>> = Arc::new(StdMutex::new(None));
        let (s, slot) = (sent.clone(), client_slot.clone());
        let client = LspClient::new(Arc::new(move |m: Value| {
            s.lock().expect("lock").push(m.clone());
            if m["method"] == "initialize" {
                let c = slot.lock().expect("lock").clone();
                if let Some(c) = c {
                    std::thread::spawn(move || {
                        c.handle_message(json!({ "jsonrpc": "2.0", "id": m["id"], "result": { "capabilities": {} } }))
                    });
                }
            }
        }));
        *client_slot.lock().expect("lock") = Some(client.clone());
        let calls = Arc::new(StdMutex::new(0));
        let c2 = calls.clone();
        let server = LspServer::new(client, python_config(), move || *c2.lock().expect("lock") += 1);
        (server, sent, calls)
    }

    fn methods(sent: &StdMutex<Vec<Value>>) -> Vec<String> {
        sent.lock().expect("lock").iter().filter_map(|m| m["method"].as_str().map(str::to_owned)).collect()
    }

    #[test]
    fn initializes_then_sends_initialized_and_the_configuration() {
        let (server, sent, _) = setup();
        futures::executor::block_on(
            server.start("C:\\proj", json!({ "pythonPath": "C:\\proj\\.venv\\Scripts\\python.exe" })),
        )
        .expect("start");
        assert_eq!(methods(&sent), ["initialize", "initialized", "workspace/didChangeConfiguration"]);
        assert_eq!(sent.lock().expect("lock")[0]["params"]["rootUri"], "file:///c%3A/proj");
    }

    #[test]
    fn answers_workspace_configuration_with_the_python_path_and_analysis_settings() {
        let (server, sent, _) = setup();
        futures::executor::block_on(server.start("/proj", json!({ "pythonPath": "/proj/.venv/bin/python" })))
            .expect("start");
        server.client.handle_message(json!({ "jsonrpc": "2.0", "id": "c1", "method": "workspace/configuration", "params": { "items": [{ "section": "python" }, { "section": "python.analysis" }, { "section": "other" }] } }));
        let reply = sent.lock().expect("lock").iter().find(|m| m["id"] == "c1").cloned().expect("reply");
        assert_eq!(reply["result"][0]["pythonPath"], "/proj/.venv/bin/python");
        assert_eq!(reply["result"][0]["analysis"]["typeCheckingMode"], "standard");
        assert_eq!(reply["result"][1]["diagnosticMode"], "openFilesOnly");
        assert_eq!(reply["result"][2], json!({}));
    }

    #[test]
    fn opens_and_closes_documents_and_batches_changes() {
        let (server, sent, _) = setup();
        futures::executor::block_on(server.start("/proj", json!({}))).expect("start");
        server.sync_docs(&[("/proj/a.py".into(), "python".into(), "x = 1".into())]);
        let opens: Vec<Value> =
            sent.lock().expect("lock").iter().filter(|m| m["method"] == "textDocument/didOpen").cloned().collect();
        assert_eq!(
            opens[0]["params"]["textDocument"],
            json!({ "uri": "file:///proj/a.py", "languageId": "python", "version": 1, "text": "x = 1" })
        );
        server.doc_changed("/proj/a.py");
        server.doc_changed("/proj/a.py");
        assert!(!methods(&sent).contains(&"textDocument/didChange".to_owned()));
        server.flush(&|_| Some("x = 1\ny = 2".into()));
        let changes: Vec<Value> =
            sent.lock().expect("lock").iter().filter(|m| m["method"] == "textDocument/didChange").cloned().collect();
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0]["params"],
            json!({ "textDocument": { "uri": "file:///proj/a.py", "version": 2 }, "contentChanges": [{ "text": "x = 1\ny = 2" }] })
        );
        server.sync_docs(&[]);
        assert_eq!(sent.lock().expect("lock").last().expect("msg")["method"], "textDocument/didClose");
    }

    #[test]
    fn stores_diagnostics_per_file() {
        let (server, _, calls) = setup();
        futures::executor::block_on(server.start("C:\\proj", json!({}))).expect("start");
        server.client.handle_message(json!({
            "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
            "params": { "uri": "file:///c%3A/proj/a.py", "diagnostics": [
                { "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 7 } }, "severity": 1, "message": "\"foo\" is not defined", "source": "Pyright", "code": "reportUndefinedVariable" }
            ] }
        }));
        assert_eq!(*calls.lock().expect("lock"), 1);
        let d = server.diagnostics_for("c:\\PROJ\\a.py");
        assert_eq!(
            (d[0].start, d[0].severity, d[0].code.as_deref(), d[0].server.as_deref()),
            (LspPos::new(0, 4), 1, Some("reportUndefinedVariable"), Some("pyright"))
        );
        assert_eq!(server.all_diagnostics().into_iter().map(|(p, _)| p).collect::<Vec<_>>(), ["C:\\proj\\a.py"]);
    }

    #[test]
    fn other_tools_diagnostics_skip_places_the_server_already_reports() {
        let d = |line, col, sev| Diagnostic {
            start: LspPos::new(line, col),
            end: LspPos::new(line, col + 1),
            severity: sev,
            message: String::new(),
            source: None,
            code: None,
            server: None,
            raw: Value::Null,
        };
        let merged = merge_diagnostics(&[d(1, 0, 1)], &[d(1, 0, 2), d(2, 0, 2)]);
        assert_eq!(merged.iter().map(|x| x.start.line).collect::<Vec<_>>(), [1, 2]);
    }
}
