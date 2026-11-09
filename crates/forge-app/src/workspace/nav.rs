//! Finding your way around with the language server: references,
//! implementations (subclasses and overrides), type definition, symbols in
//! the file and the project, call hierarchy, the outline, renaming across the
//! project, and highlighting the other uses of the name under the cursor.
//! Replaces those parts of `main.ts`.

use super::Workspace;
use crate::palette::Choice;
use crate::panels::lsp_views::{Found, OpenAt};
use forge_core::tabs::{is_inside, same_path};
use forge_lsp::navigation::{method_in_class, subclass_at, symbol_kind, symbol_path_at, DocSymbol, Location, LspPos};
use forge_text::Pos;
use gpui_kit::{Context, Focusable, PromptLevel, Window};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

impl Workspace {
    fn rel_path(&self, path: &str) -> String {
        match &self.root {
            Some(r) if is_inside(path, &r.to_string_lossy()) => Path::new(path).strip_prefix(r).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_else(|_| path.to_owned()),
            _ => path.to_owned(),
        }
    }

    /// Why a feature isn't available for the active file (None: it is).
    fn no_language_server(&self, feature: &str, cx: &gpui_kit::App) -> Option<String> {
        let ed = self.editors[self.tabs.active_group.min(1)].read(cx);
        let doc = ed.document()?;
        let d = &doc.read(cx).doc;
        let lang = d.mode().language;
        if lang.server.is_none() {
            return Some(format!("{feature} needs a language server, and there isn't one for {} files yet.", lang.name));
        }
        if d.large {
            return Some("This file is too large for the language server (it is skipped to stay responsive).".into());
        }
        match &self.lsp.server {
            Some(s) if s.knows(d.path()) => None,
            _ if self.lsp.status == super::lsp::LspStatus::Missing => Some(format!("{feature} needs the Python language server: click \"Install\" in the status bar.")),
            _ => Some(format!("{feature} will work once basedpyright has started (it starts when a Python file is open).")),
        }
    }

    /// The active file's path and the cursor (on the word) as the server counts it.
    fn lsp_here(&mut self, cx: &mut Context<Self>) -> Option<(String, LspPos, Pos)> {
        self.lsp_flush(cx);
        let ed = self.editors[self.tabs.active_group.min(1)].read(cx);
        let doc = ed.document()?.clone();
        let c = ed.cursor();
        // Just after a name counts as on it.
        let line: Vec<char> = doc.read(cx).doc.buffer.line(c.line).chars().collect();
        let on = if c.col > 0 && c.col <= line.len() && !line.get(c.col).is_some_and(|ch| ch.is_alphanumeric() || *ch == '_') && (line[c.col - 1].is_alphanumeric() || line[c.col - 1] == '_') {
            Pos::new(c.line, c.col - 1)
        } else {
            c
        };
        let d = &doc.read(cx).doc;
        Some((d.path().to_owned(), self.to_lsp(d, on), on))
    }

    /// Lines of files: open ones from the editor, others from disk (cached).
    fn line_reader(&self, cx: &gpui_kit::App) -> impl FnMut(&str) -> Vec<String> + use<> {
        let open: Vec<(String, Vec<String>)> = self.docs.values().map(|d| &d.read(cx).doc).map(|d| (d.path().to_owned(), d.buffer.lines().to_vec())).collect();
        let mut cache: HashMap<String, Vec<String>> = HashMap::new();
        move |path: &str| {
            if let Some((_, l)) = open.iter().find(|(p, _)| same_path(p, path)) {
                return l.clone();
            }
            cache
                .entry(path.to_owned())
                .or_insert_with(|| std::fs::read_to_string(path).map(|t| t.lines().map(str::to_owned).collect()).unwrap_or_default())
                .clone()
        }
    }

    /// Locations as rows with their line's text (for the References panel).
    fn found(&self, locations: &[Location], cx: &gpui_kit::App) -> Vec<(String, Found)> {
        let mut read = self.line_reader(cx);
        locations
            .iter()
            .map(|l| {
                let lines = read(&l.path);
                let line = lines.get(l.start.line).cloned().unwrap_or_default();
                let indent = line.chars().take_while(|c| c.is_whitespace()).count();
                let start = forge_text::chars::col_from_utf16(&line, l.start.col);
                let end = if l.end.line == l.start.line { forge_text::chars::col_from_utf16(&line, l.end.col) } else { line.chars().count() };
                let preview = line.trim().to_owned();
                let hl = (start.saturating_sub(indent), end.saturating_sub(indent).max(start.saturating_sub(indent)));
                (self.rel_path(&l.path), Found { location: l.clone(), preview, hl })
            })
            .collect()
    }

    /// Shift+F12: every use of the name at the cursor.
    pub(crate) fn find_references(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reveal_panel("references", cx);
        if let Some(why) = self.no_language_server("Find references", cx) {
            return self.references.update(cx, |r, cx| r.message(why, cx));
        }
        let Some((path, at, _)) = self.lsp_here(cx) else { return };
        let name = self.word_at_cursor(cx).map(|w| w.2).unwrap_or_default();
        self.references.update(cx, |r, cx| r.message(format!("Looking for references to \"{name}\"…"), cx));
        let Some(server) = self.lsp.server.clone() else { return };
        cx.spawn_in(window, async move |this, cx| {
            let locations = server.references(&path, at).await;
            this.update(cx, |ws, cx| {
                let found = ws.found(&locations, cx);
                ws.references.update(cx, |r, cx| r.show(&name, found, "reference", cx));
            })
            .ok();
        })
        .detach();
    }

    /// Ctrl+F12: on a class, the classes that inherit from it; on a method,
    /// the subclasses' own versions of it (found from the class's references).
    pub(crate) fn find_implementations(&mut self, followed: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(why) = self.no_language_server("Go to Implementations", cx) {
            return self.status_note(why, cx);
        }
        let Some((path, at, _)) = self.lsp_here(cx) else { return };
        let Some(server) = self.lsp.server.clone() else { return };
        cx.spawn_in(window, async move |this, cx| {
            let symbols = server.symbols(&path).await;
            let chain = symbol_path_at(&symbols, at);
            let inner = chain.last().cloned();
            let parent = chain.len().checked_sub(2).and_then(|i| chain.get(i)).cloned();
            let target: Option<(DocSymbol, Option<String>)> = match (&inner, &parent) {
                (Some(i), _) if i.kind == 5 && i.name_start.line == at.line => Some((i.clone(), None)),
                (Some(i), Some(p)) if [6, 9, 12].contains(&i.kind) && p.kind == 5 && i.name_start.line == at.line => Some((p.clone(), Some(i.name.clone()))),
                _ => None,
            };
            let Some((cls, method)) = target else {
                // A class or method used here: look from its definition.
                if !followed {
                    if let Some(def) = server.definition(&path, at).await {
                        if !same_path(&def.path, &path) || def.start.line != at.line {
                            this.update_in(cx, |ws, window, cx| {
                                ws.open_at_lsp(PathBuf::from(&def.path), def.start.line, def.start.col, window, cx);
                                ws.find_implementations(true, window, cx);
                            })
                            .ok();
                            return;
                        }
                    }
                }
                this.update(cx, |ws, cx| ws.status_note("Put the cursor on a class or a method to find its implementations.", cx)).ok();
                return;
            };
            let (what, title) = match &method {
                Some(m) => ("override", format!("{}.{m}", cls.name)),
                None => ("subclass", cls.name.clone()),
            };
            this.update(cx, |ws, cx| {
                ws.reveal_panel("references", cx);
                let text = if method.is_some() { format!("Looking for overrides of {title}…") } else { format!("Looking for subclasses of {title}…") };
                ws.references.update(cx, |r, cx| r.message(text, cx));
            })
            .ok();
            let refs = server.references(&path, cls.name_start).await;
            this.update_in(cx, |ws, window, cx| {
                let mut read = ws.line_reader(cx);
                let mut found = Vec::new();
                for r in refs {
                    let lines = read(&r.path);
                    let Some(text) = lines.get(r.start.line) else { continue };
                    let col = forge_text::chars::col_from_utf16(text, r.start.col);
                    let Some((sub, sub_col)) = subclass_at(text, col) else { continue };
                    let to16 = |line: &str, c: usize| forge_text::chars::utf16_col(line, c);
                    match &method {
                        None => found.push(Location {
                            path: r.path.clone(),
                            start: LspPos::new(r.start.line, to16(text, sub_col)),
                            end: LspPos::new(r.start.line, to16(text, sub_col + sub.chars().count())),
                        }),
                        Some(m) => {
                            let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
                            if let Some((l, c)) = method_in_class(&refs, r.start.line, m) {
                                found.push(Location { path: r.path.clone(), start: LspPos::new(l, to16(&lines[l], c)), end: LspPos::new(l, to16(&lines[l], c + m.chars().count())) });
                            }
                        }
                    }
                }
                let rows = ws.found(&found, cx);
                ws.references.update(cx, |r, cx| r.show(&title, rows, what, cx));
                if found.len() == 1 {
                    let l = &found[0];
                    ws.open_at_lsp(PathBuf::from(&l.path), l.start.line, l.start.col, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn go_to_type_definition(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((path, at, _)) = self.lsp_here(cx) else { return };
        let Some(server) = self.lsp.server.clone() else { return };
        cx.spawn_in(window, async move |this, cx| {
            let locs = server.type_definition(&path, at).await;
            this.update_in(cx, |ws, window, cx| match locs.first() {
                Some(l) => {
                    ws.push_nav(cx);
                    ws.open_at_lsp(PathBuf::from(&l.path), l.start.line, l.start.col, window, cx);
                }
                None => ws.status_note("No type definition found here.", cx),
            })
            .ok();
        })
        .detach();
    }

    /// Ctrl+Shift+O: jump to a class, function or method in this file.
    pub(crate) fn go_to_symbol(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(why) = self.no_language_server("Go to symbol", cx) {
            return self.choose(why, Vec::new(), window, cx);
        }
        let Some((path, _, _)) = self.lsp_here(cx) else { return };
        let Some(server) = self.lsp.server.clone() else { return };
        cx.spawn_in(window, async move |this, cx| {
            let symbols = server.symbols(&path).await;
            this.update_in(cx, |ws, window, cx| {
                let mut items = Vec::new();
                fn walk(list: &[DocSymbol], parent: &str, items: &mut Vec<Choice>) {
                    for s in list {
                        let kind = symbol_kind(s.kind).1;
                        let at = s.name_start;
                        items.push(
                            Choice::new(s.name.clone(), move |ws, window, cx| ws.go_to_lsp_in_active(at, window, cx))
                                .detail(if parent.is_empty() { kind.to_owned() } else { format!("{kind} in {parent}") })
                                .hint(format!("line {}", s.name_start.line + 1)),
                        );
                        let p = if parent.is_empty() { s.name.clone() } else { format!("{parent}.{}", s.name) };
                        walk(&s.children, &p, items);
                    }
                }
                walk(&symbols, "", &mut items);
                let prompt = if items.is_empty() { "No classes or functions in this file" } else { "Go to a symbol in this file" };
                ws.choose(prompt, items, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Move the active editor's cursor to a server position.
    pub(crate) fn go_to_lsp_in_active(&mut self, at: LspPos, window: &mut Window, cx: &mut Context<Self>) {
        let ed = self.editors[self.tabs.active_group.min(1)].clone();
        let col = ed.read(cx).document().map(|d| forge_text::chars::col_from_utf16(d.read(cx).doc.buffer.line(at.line), at.col)).unwrap_or(at.col);
        ed.update(cx, |e, cx| e.go_to(at.line, col, cx));
        window.focus(&ed.focus_handle(cx), cx);
    }

    /// Ctrl+T: find a class / function anywhere in the project by name.
    pub(crate) fn go_to_project_symbol(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.lsp.server.as_ref().is_none_or(|s| !s.is_started()) {
            return self.choose("No language server is running (open a Python file first).", Vec::new(), window, cx);
        }
        let initial = self.editors[self.tabs.active_group.min(1)].read(cx).selected_text(cx).trim().lines().next().unwrap_or("").to_owned();
        self.palette.update(cx, |p, cx| {
            p.search(
                "Go to a class, function or variable anywhere in the project",
                &initial,
                |ws, query, generation, window, cx| ws.search_project_symbols(query, generation, window, cx),
                window,
                cx,
            )
        });
    }

    /// The palette asks for project symbols matching `query`.
    pub(crate) fn search_project_symbols(&mut self, query: String, generation: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(server) = self.lsp.server.clone() else { return };
        let root = self.root.clone();
        cx.spawn_in(window, async move |this, cx| {
            let mut found = server.workspace_symbols(&query).await;
            // Project files first (the standard library can match too).
            let in_project = |p: &str| {
                root.as_ref().is_some_and(|r| is_inside(p, &r.to_string_lossy()))
                    && !p.contains("site-packages")
                    && !p.replace('\\', "/").contains("/.venv/")
                    && !p.replace('\\', "/").contains("/venv/")
            };
            found.sort_by_key(|s| !in_project(&s.location.path));
            this.update(cx, |ws, cx| {
                let choices: Vec<Choice> = found
                    .iter()
                    .map(|s| {
                        let loc = s.location.clone();
                        let kind = symbol_kind(s.kind).1;
                        let detail = format!("{kind}{} · {}", s.container.as_ref().map(|c| format!(" in {c}")).unwrap_or_default(), ws.rel_path(&loc.path));
                        Choice::new(s.name.clone(), move |ws, window, cx| {
                            ws.push_nav(cx);
                            ws.open_at_lsp(PathBuf::from(&loc.path), loc.start.line, loc.start.col, window, cx);
                        })
                        .detail(detail)
                        .hint(format!("line {}", s.location.start.line + 1))
                    })
                    .collect();
                let empty = format!("Nothing called \"{query}\" found");
                ws.palette.update(cx, |p, cx| p.set_search_results(generation, choices, &empty, cx));
            })
            .ok();
        })
        .detach();
    }

    /// Shift+Alt+H: who calls the function at the cursor.
    pub(crate) fn show_call_hierarchy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reveal_panel("calls", cx);
        if let Some(why) = self.no_language_server("Call hierarchy", cx) {
            return self.calls.update(cx, |c, cx| c.message(why, cx));
        }
        let Some((path, at, _)) = self.lsp_here(cx) else { return };
        let Some(server) = self.lsp.server.clone() else { return };
        cx.spawn_in(window, async move |this, cx| {
            let items = server.prepare_call_hierarchy(&path, at).await;
            this.update(cx, |ws, cx| {
                ws.calls.update(cx, |c, cx| match items.into_iter().next() {
                    Some(item) => c.show(server.clone(), item, cx),
                    None => c.message("Put the cursor on a function or method name (its definition or a call to it).", cx),
                });
            })
            .ok();
        })
        .detach();
    }

    /// A panel asked to open a place (an empty path: in the active file).
    pub(crate) fn on_open_at(&mut self, e: &OpenAt, window: &mut Window, cx: &mut Context<Self>) {
        self.push_nav(cx);
        if e.0.as_os_str().is_empty() {
            self.go_to_lsp_in_active(e.1, window, cx);
        } else {
            self.open_at_lsp(e.0.clone(), e.1.line, e.1.col, window, cx);
        }
    }

    /// The active file was edited: the Outline follows (or is asked again when next shown).
    pub(crate) fn outline_edited(&mut self, cx: &mut Context<Self>) {
        if self.tasks.is_shown("outline") {
            self.outline_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                this.update(cx, |ws, cx| ws.refresh_outline(cx)).ok();
            }));
        } else {
            self.outline_for = None;
        }
    }

    /// Another file became active (or the language server started).
    pub(crate) fn outline_file_changed(&mut self, cx: &mut Context<Self>) {
        if self.tasks.is_shown("outline") {
            self.refresh_outline(cx);
        } else {
            self.outline_for = None;
        }
    }

    pub(crate) fn refresh_outline(&mut self, cx: &mut Context<Self>) {
        let ed = self.editors[self.tabs.active_group.min(1)].read(cx);
        let Some(doc) = ed.document().cloned() else {
            self.outline_for = Some(String::new());
            return self.outline.update(cx, |o, cx| o.set(Vec::new(), Some("Open a Python file to see its outline.".into()), cx));
        };
        let path = doc.read(cx).doc.path().to_owned();
        self.outline_for = Some(path.clone());
        if let Some(why) = self.no_language_server("The outline", cx) {
            return self.outline.update(cx, |o, cx| o.set(Vec::new(), Some(why), cx));
        }
        self.lsp_flush(cx);
        let Some(server) = self.lsp.server.clone() else { return };
        self.outline_task = Some(cx.spawn(async move |this, cx| {
            let symbols = server.symbols(&path).await;
            this.update(cx, |ws, cx| {
                ws.outline.update(cx, |o, cx| o.set(symbols, None, cx));
                ws.highlight_outline(cx);
            })
            .ok();
        }));
    }

    pub(crate) fn highlight_outline(&mut self, cx: &mut Context<Self>) {
        let ed = self.editors[self.tabs.active_group.min(1)].read(cx);
        let at = ed.document().map(|d| self.to_lsp(&d.read(cx).doc, ed.cursor()));
        self.outline.update(cx, |o, cx| o.highlight(at, cx));
    }

    /// Highlight the other uses of the name under the cursor, a moment after it stops.
    pub(crate) fn update_occurrences(&mut self, cx: &mut Context<Self>) {
        let group = self.tabs.active_group.min(1);
        let ed = self.editors[group].clone();
        let clear = |ws: &mut Workspace, cx: &mut Context<Workspace>| {
            ws.occurrence_key.clear();
            ws.occurrence_task = None;
            for e in ws.editors.clone() {
                e.update(cx, |e, cx| e.set_occurrences(Vec::new(), cx));
            }
        };
        let Some(doc) = ed.read(cx).document().cloned() else { return clear(self, cx) };
        let Some(server) = self.lsp.server.clone() else { return clear(self, cx) };
        let path = doc.read(cx).doc.path().to_owned();
        if !server.knows(&path) || !ed.read(cx).selected_text(cx).is_empty() {
            return clear(self, cx);
        }
        let Some((start, _, name)) = self.word_at_cursor(cx) else { return clear(self, cx) };
        if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return clear(self, cx);
        }
        let key = format!("{path}|{}|{}|{}", doc.read(cx).doc.version, start.line, start.col);
        if key == self.occurrence_key {
            return;
        }
        clear(self, cx);
        self.occurrence_key = key.clone();
        let at = self.to_lsp(&doc.read(cx).doc, start);
        self.occurrence_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(250)).await;
            this.update(cx, |ws, cx| ws.lsp_flush(cx)).ok();
            let list = server.highlights(&path, at).await;
            this.update(cx, |ws, cx| {
                if ws.occurrence_key != key {
                    return;
                }
                let d = &doc.read(cx).doc;
                // Just the one under the cursor isn't worth highlighting.
                let ranges: Vec<(Pos, Pos, bool)> = if list.len() > 1 {
                    list.iter()
                        .map(|(s, e, w)| (super::lsp::from_lsp(d.buffer.line(s.line), *s), super::lsp::from_lsp(d.buffer.line(e.line), *e), *w))
                        .collect()
                } else {
                    Vec::new()
                };
                ed.update(cx, |e, cx| e.set_occurrences(ranges, cx));
            })
            .ok();
        }));
    }

    /// F2: rename the name at the cursor everywhere.
    pub(crate) fn rename_symbol(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(why) = self.no_language_server("Rename", cx) {
            return self.alert(why, window, cx);
        }
        let Some((path, at, _)) = self.lsp_here(cx) else { return };
        let Some(server) = self.lsp.server.clone() else { return };
        let word = self.word_at_cursor(cx).map(|w| w.2).unwrap_or_default();
        cx.spawn_in(window, async move |this, cx| {
            let prep = server.prepare_rename(&path, at).await;
            this.update_in(cx, |ws, window, cx| {
                let Some((_, _, name)) = prep else {
                    return ws.alert("That can't be renamed. Put the cursor on a name defined in your code (a variable, function, class, parameter...).", window, cx);
                };
                let old = name.unwrap_or(word);
                let old2 = old.clone();
                ws.palette.update(cx, |p, cx| {
                    p.prompt(
                        format!("Rename \"{old}\" everywhere to…"),
                        &old,
                        "Rename to",
                        move |ws, text, window, cx| ws.do_rename(path.clone(), at, old2.clone(), text.trim().to_owned(), window, cx),
                        window,
                        cx,
                    )
                });
            })
            .ok();
        })
        .detach();
    }

    fn do_rename(&mut self, path: String, at: LspPos, old: String, new: String, window: &mut Window, cx: &mut Context<Self>) {
        if new == old || new.is_empty() {
            return;
        }
        let valid = new.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') && new.chars().all(|c| c.is_alphanumeric() || c == '_');
        if !valid {
            return self.alert(format!("\"{new}\" isn't a valid Python name (letters, digits and _, not starting with a digit)."), window, cx);
        }
        let Some(server) = self.lsp.server.clone() else { return };
        cx.spawn_in(window, async move |this, cx| {
            let edits = server.rename(&path, at, &new).await;
            this.update_in(cx, |ws, window, cx| {
                let edits = match edits {
                    Ok(e) if !e.is_empty() => e,
                    Ok(_) => return ws.alert("Nothing to rename.", window, cx),
                    Err(e) => return ws.alert(format!("Rename failed: {e}"), window, cx),
                };
                let count: usize = edits.values().map(Vec::len).sum();
                let files = edits.len();
                let closed: Vec<String> = edits.keys().filter(|p| !ws.docs.values().any(|d| same_path(d.read(cx).doc.path(), p))).cloned().collect();
                let go = move |ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>| {
                    if let Err(e) = ws.apply_file_edits(&edits, window, cx) {
                        ws.alert(format!("These files couldn't be changed, so the rename is incomplete there:\n{e}"), window, cx);
                    }
                    ws.file_tree.update(cx, |t, cx| t.refresh_all(cx));
                };
                if closed.is_empty() {
                    return go(ws, window, cx);
                }
                let mut list: Vec<String> = closed.iter().take(10).map(|p| format!("  • {}", ws.rel_path(p))).collect();
                if closed.len() > 10 {
                    list.push(format!("  …and {} more", closed.len() - 10));
                }
                let message = format!(
                    "Rename \"{old}\" to \"{new}\": {count} change{} in {files} file{}.",
                    if count == 1 { "" } else { "s" },
                    if files == 1 { "" } else { "s" }
                );
                let detail = format!(
                    "{} of them {} open and will be changed and saved on disk:\n{}\n\nContinue?",
                    closed.len(),
                    if closed.len() == 1 { "isn't" } else { "aren't" },
                    list.join("\n")
                );
                let answer = window.prompt(PromptLevel::Info, &message, Some(&detail), &["Rename", "Cancel"], cx);
                cx.spawn_in(window, async move |this, cx| {
                    if answer.await == Ok(0) {
                        this.update_in(cx, |ws, window, cx| go(ws, window, cx)).ok();
                    }
                })
                .detach();
            })
            .ok();
        })
        .detach();
    }
}
