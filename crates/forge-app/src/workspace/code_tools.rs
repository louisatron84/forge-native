//! Code quality tools for Python files:
//!  - Ruff warnings as you type (shown with the language server's errors),
//!    with a 💡 on the cursor's line when it has an automatic fix
//!  - Quick Fix (Ctrl+.): Ruff's fixes, the language server's code actions,
//!    "ignore this rule on this line", organise imports, format
//!  - Format Document / Selection with Ruff or Black, and format on save
//!  - Organise Imports (Ruff's isort rules + unused imports, or the server's)
//!
//! The tools run on the editor's text (unsaved changes count), on a
//! background thread. Replaces `src/renderer/codeTools.ts`.

use super::Workspace;
use crate::document::Document;
use crate::palette::Choice;
use forge_core::lint::{noqa_edit, parse_ruff_json, Edit};
use forge_core::settings::Formatter;
use forge_core::tabs::same_path;
use forge_lsp::navigation::LspPos;
use forge_lsp::server::{CodeAction, Diagnostic};
use forge_run::tools::{Tool, Tools};
use forge_text::chars::utf16_col;
use forge_text::Pos;
use gpui_kit::{Context, Entity, Task, Window};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const LINT_DELAY: Duration = Duration::from_millis(700);

#[derive(Default)]
pub(crate) struct CodeToolsState {
    pub(crate) tools: Arc<Tools>,
    /// The version of each file last linted (to skip unchanged ones).
    linted: HashMap<String, u64>,
    timers: HashMap<String, Task<()>>,
    /// Ruff wasn't found (don't keep trying on every keystroke).
    ruff_missing: bool,
}

fn is_python(path: &str) -> bool {
    let p = path.to_lowercase();
    p.ends_with(".py") || p.ends_with(".pyi")
}

fn to_pos(p: forge_core::lint::Pos) -> Pos {
    Pos::new(p.line, p.col)
}

/// A tool's edits as (start, end, text) in character positions.
pub(super) fn char_edits(edits: &[Edit]) -> Vec<(Pos, Pos, String)> {
    edits.iter().map(|e| (to_pos(e.start), to_pos(e.end), e.text.clone())).collect()
}

impl Workspace {
    fn tools_python(&self) -> Option<String> {
        self.pyenv.current.as_ref().map(|i| i.path.clone())
    }

    /// Where the tools run: the project (its config applies), else the file's folder.
    fn tools_cwd(&self, path: &str) -> PathBuf {
        self.root.clone().unwrap_or_else(|| PathBuf::from(path).parent().map(PathBuf::from).unwrap_or_default())
    }

    fn doc_entity(&self, path: &str, cx: &gpui_kit::App) -> Option<Entity<Document>> {
        self.docs.values().find(|d| same_path(d.read(cx).doc.path(), path)).cloned()
    }

    // ---------- linting ----------

    /// Lint open Python files that changed (after edits, opening and closing
    /// files, settings changes); drop the warnings of closed files.
    pub(crate) fn lint_sync(&mut self, cx: &mut Context<Self>) {
        let on = self.effective_settings(cx).lint_with_ruff;
        let open: Vec<(String, u64, bool)> =
            self.docs.values().map(|d| &d.read(cx).doc).map(|d| (d.path().to_owned(), d.version, d.large)).collect();
        let before = self.external_diagnostics.len();
        self.external_diagnostics.retain(|(p, _)| on && open.iter().any(|(o, _, _)| same_path(o, p)));
        self.code_tools.linted.retain(|p, _| open.iter().any(|(o, _, _)| same_path(o, p)));
        self.code_tools.timers.retain(|p, _| open.iter().any(|(o, _, _)| same_path(o, p)));
        if self.external_diagnostics.len() != before {
            self.refresh_diagnostics(cx);
        }
        if !on || self.code_tools.ruff_missing {
            return;
        }
        for (path, version, large) in open {
            if !is_python(&path) || large || self.code_tools.linted.get(&path) == Some(&version) {
                continue;
            }
            // (Re)starting the timer: it lints once typing pauses.
            self.lint_soon(path, LINT_DELAY, cx);
        }
    }

    /// Ruff may have been installed, or the interpreter changed: look again.
    pub(crate) fn code_tools_changed(&mut self, cx: &mut Context<Self>) {
        self.code_tools.tools.forget();
        self.code_tools.ruff_missing = false;
        self.code_tools.linted.clear();
        self.lint_sync(cx);
    }

    pub(super) fn lint_soon(&mut self, path: String, delay: Duration, cx: &mut Context<Self>) {
        let key = path.clone();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |ws, cx| {
                ws.code_tools.timers.remove(&path);
                ws.lint(path, cx);
            })
            .ok();
        });
        self.code_tools.timers.insert(key, task);
    }

    fn lint(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(doc) = self.doc_entity(&path, cx) else { return };
        let (text, version) = {
            let d = &doc.read(cx).doc;
            (d.buffer.lines().join("\n"), d.version)
        };
        let (tools, python, cwd) = (self.code_tools.tools.clone(), self.tools_python(), self.tools_cwd(&path));
        cx.spawn(async move |this, cx| {
            let p = path.clone();
            let res = cx
                .background_executor()
                .spawn(async move { tools.ruff_check(python.as_deref(), &cwd, &p, &text) })
                .await;
            this.update(cx, |ws, cx| {
                if res.missing {
                    ws.code_tools.ruff_missing = true;
                    return;
                }
                let d = &doc.read(cx).doc;
                // Changed meanwhile: a newer lint is coming.
                if d.version != version {
                    return;
                }
                ws.code_tools.linted.insert(path.clone(), version);
                // A broken Ruff config: the language server still shows errors.
                if !res.ok || !ws.effective_settings(cx).lint_with_ruff {
                    return;
                }
                let lsp = |p: forge_core::lint::Pos| LspPos::new(p.line, utf16_col(d.buffer.line(p.line), p.col));
                let list: Vec<Diagnostic> = parse_ruff_json(&res.output)
                    .into_iter()
                    // The language server reports syntax errors already.
                    .filter(|p| p.code != "syntax")
                    .map(|p| Diagnostic {
                        start: lsp(p.start),
                        end: lsp(p.end),
                        severity: 2,
                        message: p.message,
                        source: Some("Ruff".into()),
                        code: Some(p.code),
                        server: None,
                        raw: serde_json::Value::Null,
                        fix: p.fix,
                        url: p.url,
                    })
                    .collect();
                ws.external_diagnostics.retain(|(q, _)| !same_path(q, &path));
                if !list.is_empty() {
                    ws.external_diagnostics.push((path.clone(), list));
                }
                ws.refresh_diagnostics(cx);
                ws.update_lightbulb(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Show the 💡 when the cursor's line has a Ruff fix.
    pub(crate) fn update_lightbulb(&mut self, cx: &mut Context<Self>) {
        let active = self.tabs.active_group.min(1);
        for (i, ed) in self.editors.clone().into_iter().enumerate() {
            let line = (i == active)
                .then(|| {
                    let e = ed.read(cx);
                    let doc = e.document()?;
                    let line = e.cursor().line;
                    let has = self
                        .diagnostics_for(doc.read(cx).doc.path())
                        .iter()
                        .any(|d| d.fix.is_some() && d.start.line <= line && line <= d.end.line);
                    has.then_some(line)
                })
                .flatten();
            ed.update(cx, |e, cx| e.set_lightbulb(line, cx));
        }
    }

    // ---------- quick fixes ----------

    /// Ctrl+. : what can be done here.
    pub(crate) fn quick_fix(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ed = self.editors[self.tabs.active_group.min(1)].clone();
        let Some(doc) = ed.read(cx).document().cloned() else { return };
        self.lsp_flush(cx);
        let path = doc.read(cx).doc.path().to_owned();
        let (start, end) = ed.read(cx).selection().range();
        let here: Vec<Diagnostic> = self
            .diagnostics_for(&path)
            .into_iter()
            .filter(|d| d.start.line <= end.line && start.line <= d.end.line)
            .collect();
        let (s16, e16) = {
            let d = &doc.read(cx).doc;
            (self.to_lsp(d, start), self.to_lsp(d, end))
        };
        let server = self.active_language_server(cx).filter(|s| s.knows(&path));
        let server_diags: Vec<Diagnostic> =
            here.iter().filter(|d| d.source.as_deref() != Some("Ruff")).cloned().collect();
        cx.spawn_in(window, async move |this, cx| {
            let actions = match &server {
                Some(s) => s.code_actions(&path, s16, e16, &server_diags, None).await,
                None => Vec::new(),
            };
            this.update_in(cx, |ws, window, cx| ws.show_quick_fixes(doc, here, actions, window, cx)).ok();
        })
        .detach();
    }

    fn show_quick_fixes(
        &mut self,
        doc: Entity<Document>,
        here: Vec<Diagnostic>,
        mut actions: Vec<CodeAction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = doc.read(cx).doc.path().to_owned();
        let mut items = Vec::new();
        // Ruff's own fixes for problems on these lines.
        for d in &here {
            let Some(fix) = d.fix.clone() else { continue };
            let (doc, path) = (doc.clone(), path.clone());
            let edits = char_edits(&fix.edits);
            items.push(
                Choice::new(fix.title.clone(), move |ws, _, cx| {
                    ws.apply_doc_edits(&doc, edits.clone(), cx);
                    ws.lint_soon(path.clone(), Duration::from_millis(100), cx);
                })
                .detail(format!("{}: {}", d.code.clone().unwrap_or_default(), d.message))
                .hint(if fix.safe { "Ruff" } else { "Ruff (review it)" }),
            );
        }
        // The language server's actions (quick fixes, refactorings).
        actions.sort_by_key(|a| !a.preferred);
        for a in actions {
            if a.kind.starts_with("source.organizeImports") {
                continue; // offered below
            }
            let title = a.title.clone();
            items.push(
                Choice::new(title, move |ws, window, cx| ws.run_code_action(a.clone(), window, cx))
                    .hint("basedpyright"),
            );
        }
        // Ignore a Ruff rule on this line.
        let mut codes: Vec<(String, usize)> = Vec::new();
        for d in here.iter().filter(|d| d.source.as_deref() == Some("Ruff")) {
            if let Some(code) = &d.code {
                if !codes.iter().any(|(c, _)| c == code) {
                    codes.push((code.clone(), d.start.line));
                }
            }
        }
        for (code, line) in codes {
            let (doc, path) = (doc.clone(), path.clone());
            let (label, detail) = (format!("Ignore {code} on this line"), format!("adds  # noqa: {code}"));
            items.push(
                Choice::new(label, move |ws, _, cx| {
                    let text = doc.read(cx).doc.buffer.line(line).to_owned();
                    if let Some(e) = noqa_edit(&text, line, &code) {
                        ws.apply_doc_edits(&doc, char_edits(&[e]), cx);
                        ws.lint_soon(path.clone(), Duration::from_millis(100), cx);
                    }
                })
                .detail(detail),
            );
        }
        if self.diagnostics_for(&path).iter().any(|d| d.fix.as_ref().is_some_and(|f| f.safe)) {
            items.push(
                Choice::new("Fix all auto-fixable problems in this file", |ws, window, cx| ws.fix_all(window, cx))
                    .detail("ruff check --fix"),
            );
        }
        items.push(
            Choice::new("Organize Imports", |ws, window, cx| ws.organize_imports(window, cx)).hint("Shift+Alt+O"),
        );
        items.push(
            Choice::new("Format Document", |ws, window, cx| ws.format_document(false, window, cx)).hint("Shift+Alt+F"),
        );
        if self.code_tools.ruff_missing {
            items.push(
                Choice::new("Install Ruff for more warnings and fixes", |ws, window, cx| {
                    ws.pip_install("ruff", window, cx)
                })
                .detail("pip install ruff"),
            );
        }
        let problems = match here.len() {
            0 => "no problems here".to_owned(),
            1 => "1 problem here".to_owned(),
            n => format!("{n} problems here"),
        };
        self.choose(format!("Quick fixes ({problems})"), items, window, cx);
    }

    pub(crate) fn run_code_action(&mut self, a: CodeAction, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(edit) = a.edit.as_ref().filter(|e| !e.is_empty()) {
            if let Err(e) = self.apply_file_edits(edit, window, cx) {
                return self.alert(format!("\"{}\" failed: {e}", a.title), window, cx);
            }
        }
        if let (Some(command), Some(server)) = (a.command.clone(), self.active_language_server(cx)) {
            cx.spawn(async move |_, _| server.execute_command(&command).await).detach();
        }
    }

    // ---------- whole-file tools ----------

    /// Which formatter to use, or None (after offering to install one, unless quiet).
    fn pick_formatter(
        &mut self,
        have: (bool, bool),
        quiet: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Tool> {
        let pref = self.effective_settings(cx).formatter;
        let (ruff, black) = have;
        let tool = match pref {
            Formatter::Ruff => ruff.then_some(Tool::Ruff),
            Formatter::Black => black.then_some(Tool::Black),
            Formatter::Auto => (ruff || black).then_some(if ruff { Tool::Ruff } else { Tool::Black }),
        };
        if tool.is_some() || quiet {
            return tool;
        }
        let want = if pref == Formatter::Black { Tool::Black } else { Tool::Ruff };
        let module = if want == Tool::Ruff { "ruff" } else { "black" };
        let mut items = vec![Choice::new(format!("Install {}", want.name()), move |ws, window, cx| {
            ws.pip_install(module, window, cx)
        })
        .detail(format!("pip install {module}"))];
        if pref == Formatter::Auto {
            items.push(
                Choice::new("Install Black instead", |ws, window, cx| ws.pip_install("black", window, cx))
                    .detail("pip install black"),
            );
        }
        items.push(Choice::new("Cancel", |_, _, _| {}));
        self.choose(format!("{} isn't installed in this project's Python", want.name()), items, window, cx);
        None
    }

    /// Format the active Python file (or just the selected lines).
    pub(crate) fn format_document(&mut self, selection: bool, window: &mut Window, cx: &mut Context<Self>) {
        let ed = self.editors[self.tabs.active_group.min(1)].clone();
        let Some(doc) = ed.read(cx).document().cloned() else { return };
        let lines = selection.then(|| ed.read(cx).selection().range()).and_then(|(s, e)| {
            (s != e).then(|| {
                let end = if e.col == 0 && e.line > s.line { e.line - 1 } else { e.line };
                (s.line + 1, end + 1)
            })
        });
        self.format_doc(doc, lines, None, window, cx);
    }

    /// Ask before applying a formatter result, including on Save.
    fn format_doc(
        &mut self,
        doc: Entity<Document>,
        lines: Option<(usize, usize)>,
        save: Option<forge_core::tabs::DocId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = doc.read(cx).doc.path().to_owned();
        let quiet = save.is_some();
        if !is_python(&path) {
            self.status_note("Formatting is for Python files.", cx);
            return;
        }
        let (before, version) = {
            let d = &doc.read(cx).doc;
            (d.buffer.lines().join("\n"), d.version)
        };
        let (tools, python, cwd) = (self.code_tools.tools.clone(), self.tools_python(), self.tools_cwd(&path));
        cx.spawn_in(window, async move |this, cx| {
            let (t, py, dir) = (tools.clone(), python.clone(), cwd.clone());
            let have = cx.background_executor().spawn(async move { t.available(py.as_deref(), &dir) }).await;
            let tool = this.update_in(cx, |ws, window, cx| ws.pick_formatter(have, quiet, window, cx)).ok().flatten();
            let res = match tool {
                Some(tool) => {
                    let text = before.clone();
                    let p = path.clone();
                    Some(
                        cx.background_executor()
                            .spawn(async move { tools.format_text(python.as_deref(), &cwd, &p, &text, tool, lines) })
                            .await,
                    )
                }
                None => None,
            };
            this.update_in(cx, |ws, window, cx| {
                match res {
                    Some(res) if res.ok && res.output != before => {
                        ws.review_tool_edit(doc, version, (before, res.output), "Format", quiet, cx);
                        return;
                    }
                    Some(res) if !res.ok => {
                        let error = res.error.unwrap_or_else(|| "Formatting failed.".into());
                        if quiet {
                            ws.status_note(format!("Could not format: {}", error.lines().next().unwrap_or("")), cx);
                        } else {
                            ws.alert(error, window, cx);
                        }
                    }
                    Some(_) if !quiet => ws.status_note("Already formatted.", cx),
                    _ => {}
                }
                if let Some(id) = save.filter(|id| ws.docs.get(id) == Some(&doc)) {
                    ws.save(Some(id), window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Ctrl+S: format first when the settings say so, then save.
    pub(crate) fn save_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.active_doc() else { return };
        let id = self.tabs.active();
        if self.effective_settings(cx).format_on_save && is_python(doc.read(cx).doc.path()) {
            return self.format_doc(doc, None, id, window, cx);
        }
        self.save(id, window, cx);
    }

    /// Sort imports and remove unused ones (Ruff), or the language server's organise imports.
    pub(crate) fn organize_imports(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.active_doc() else { return };
        let path = doc.read(cx).doc.path().to_owned();
        if !is_python(&path) {
            return;
        }
        self.lsp_flush(cx);
        let (before, version, end16) = {
            let d = &doc.read(cx).doc;
            let end = d.buffer.end();
            (d.buffer.lines().join("\n"), d.version, self.to_lsp(d, end))
        };
        let (tools, python, cwd) = (self.code_tools.tools.clone(), self.tools_python(), self.tools_cwd(&path));
        let server = self.active_language_server(cx).filter(|s| s.knows(&path));
        cx.spawn_in(window, async move |this, cx| {
            let (text, p) = (before.clone(), path.clone());
            let res = cx
                .background_executor()
                .spawn(async move { tools.ruff_fix(python.as_deref(), &cwd, &p, &text, &["I", "F401"]) })
                .await;
            if res.ok {
                this.update(cx, |ws, cx| {
                    if before == res.output {
                        ws.status_note("Imports are already organized.", cx);
                    } else {
                        ws.review_tool_edit(doc, version, (before, res.output), "Organize Imports", false, cx);
                    }
                })
                .ok();
                return;
            }
            // No Ruff: the language server can sort imports (but not remove unused ones).
            if let Some(s) = server {
                let actions =
                    s.code_actions(&path, LspPos::new(0, 0), end16, &[], Some(&["source.organizeImports"])).await;
                if let Some(a) = actions.into_iter().next() {
                    this.update_in(cx, |ws, window, cx| {
                        if doc.read(cx).doc.version != version {
                            return ws.status_note("The editor changed. Organize imports again.", cx);
                        }
                        if let Some(edits) = a.edit.as_ref().filter(|edits| !edits.is_empty()) {
                            ws.review_language_edits(edits, "Organize Imports", None, window, cx);
                        } else {
                            ws.run_code_action(a, window, cx);
                        }
                    })
                    .ok();
                    return;
                }
            }
            this.update_in(cx, |ws, window, cx| {
                if res.missing {
                    let items = vec![
                        Choice::new("Install Ruff", |ws, window, cx| ws.pip_install("ruff", window, cx))
                            .detail("pip install ruff"),
                        Choice::new("Cancel", |_, _, _| {}),
                    ];
                    ws.choose(
                        "Organize Imports uses Ruff, which isn't installed in this project's Python",
                        items,
                        window,
                        cx,
                    );
                } else if let Some(e) = res.error {
                    ws.alert(e, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Apply every safe Ruff fix in the file.
    pub(crate) fn fix_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.active_doc() else { return };
        let path = doc.read(cx).doc.path().to_owned();
        if !is_python(&path) {
            return;
        }
        let (before, version) = {
            let d = &doc.read(cx).doc;
            (d.buffer.lines().join("\n"), d.version)
        };
        let (tools, python, cwd) = (self.code_tools.tools.clone(), self.tools_python(), self.tools_cwd(&path));
        cx.spawn_in(window, async move |this, cx| {
            let (text, p) = (before.clone(), path.clone());
            let res = cx
                .background_executor()
                .spawn(async move { tools.ruff_fix(python.as_deref(), &cwd, &p, &text, &[]) })
                .await;
            this.update_in(cx, |ws, window, cx| {
                if !res.ok {
                    return if res.missing {
                        ws.pip_install("ruff", window, cx)
                    } else {
                        ws.alert(res.error.unwrap_or_else(|| "Ruff failed.".into()), window, cx)
                    };
                }
                if before == res.output {
                    ws.status_note("Nothing to fix automatically.", cx);
                } else {
                    ws.review_tool_edit(doc, version, (before, res.output), "Fix All", false, cx);
                }
            })
            .ok();
        })
        .detach();
    }
}
