//! What each command does, and when it can run. The ids, titles and keys are
//! in `forge_core::commands`; this is the old `main.ts` command registry's
//! `run` and `when` parts.

use super::Workspace;
use crate::app_state::{self, AppState};
use crate::palette::Choice;
use crate::panels::tests::Which;
use forge_core::settings::{Theme as ThemeSetting, FONT_SIZE_MAX, FONT_SIZE_MIN};
use forge_sys::files::create_entry;
use forge_sys::user_state::ensure_project_settings;
use forge_text::{Encoding, Eol};
use gpui_kit::{AppContext, Context, Focusable, PromptLevel, Window};
use std::path::PathBuf;

impl Workspace {
    fn editor_focused(&self, window: &Window, cx: &gpui_kit::App) -> bool {
        self.editors.iter().any(|e| e.focus_handle(cx).contains_focused(window, cx))
    }

    /// Whether a command can run right now (the old `when` conditions).
    pub fn available(&self, id: &str, window: &Window, cx: &gpui_kit::App) -> bool {
        let has_doc = self.tabs.active().is_some();
        let has_folder = self.root.is_some();
        let py_doc = self.active_doc().is_some_and(|d| {
            let p = d.read(cx).doc.path().to_lowercase();
            p.ends_with(".py") || p.ends_with(".pyi")
        });
        match id {
            "editor.toggleComment"
            | "file.save"
            | "file.rename"
            | "file.delete"
            | "tab.close"
            | "tab.next"
            | "tab.prev"
            | "find.open"
            | "find.replace"
            | "view.split"
            | "view.moveToOtherGroup"
            | "editor.fold"
            | "editor.unfold"
            | "editor.foldAll"
            | "editor.unfoldAll"
            | "file.lineEndings"
            | "file.reopenEncoding"
            | "file.saveEncoding"
            | "palette.line"
            | "nav.definition"
            | "debug.toggleBreakpoint"
            | "nav.references"
            | "nav.symbol"
            | "nav.implementations"
            | "nav.typeDefinition"
            | "nav.callHierarchy"
            | "editor.quickFix"
            | "debug.editBreakpoint" => has_doc,
            "editor.renameSymbol" => has_doc && self.editor_focused(window, cx),
            "editor.format" | "editor.formatSelection" | "editor.organizeImports" | "editor.fixAll" => py_doc,
            "find.close" => self.find_bar.read(cx).is_open(),
            "nav.projectSymbol"
            | "file.saveTemplate"
            | "run.selectConfig"
            | "run.editConfigs"
            | "tasks.run"
            | "python.restartServer"
            | "settings.project"
            | "view.database"
            | "view.projectGraph"
            | "connect.addApiBox"
            | "search.project"
            | "python.selectInterpreter"
            | "python.createVenv"
            | "python.packages"
            | "python.installRequirements"
            | "python.checkDeps"
            | "tests.show"
            | "tests.runAll"
            | "tests.runFailed"
            | "python.installRuff"
            | "palette.files"
            | "git.init" => has_folder,
            "tests.runFile" | "tests.runAtCursor" => has_doc,
            "tasks.rerun" => has_folder && self.run.last_task.is_some(),
            "run.stop" => self.is_running(cx),
            // Need a debug session, a test run or a git repository: none yet.
            "debug.restart"
            | "debug.stepOver"
            | "debug.stepInto"
            | "debug.stepOut"
            | "debug.pause"
            | "debug.clearBreakpoints"
            | "git.acceptCurrent"
            | "git.acceptIncoming"
            | "git.acceptBoth"
            | "git.nextConflict"
            | "git.prevConflict"
            | "git.history"
            | "git.fileHistory"
            | "git.toggleBlame"
            | "git.commit"
            | "git.branch"
            | "git.pull"
            | "git.push"
            | "git.fetch"
            | "git.stageAll" => false,
            "tests.stop" => self.tests.read(cx).is_running(),
            "terminal.kill" | "terminal.next" => {
                self.tasks.is_shown("terminal") && self.terminal.read(cx).session_count() > 0
            }
            "git.diffFile" => has_doc && has_folder,
            "run.smart" | "run.file" | "run.memory" | "debug.start" => self.can_launch(),
            _ => true,
        }
    }

    /// Run a command by id (shortcut, menu, palette, Welcome page, toolbar).
    pub fn run_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.available(id, window, cx) {
            return;
        }
        let group = self.tabs.active_group.min(1);
        let editor = self.editors[group].clone();
        match id {
            // ---------- files and folders ----------
            "file.openFolder" => self.open_folder_dialog(window, cx),
            "file.openFolderByPath" => {
                let initial = self.root.as_ref().map(|r| r.to_string_lossy().into_owned()).unwrap_or_default();
                self.palette.update(cx, |p, cx| {
                    p.prompt(
                        "Folder to open, e.g. C:\\Users\\me\\project or ~/project",
                        &initial,
                        "Open",
                        |ws, text, window, cx| {
                            let path = expand_home(&text);
                            if path.is_dir() {
                                ws.open_folder(path, window, cx);
                            } else {
                                ws.alert(format!("{text} isn't a folder that exists."), window, cx);
                            }
                        },
                        window,
                        cx,
                    )
                });
            }
            "file.openRecent" => {
                let list = app_state::state(cx).user.recent();
                let prompt = if list.is_empty() { "No recent folders yet" } else { "Open a recent folder" };
                let choices = list
                    .into_iter()
                    .map(|folder| {
                        let label = folder
                            .trim_end_matches(['/', '\\'])
                            .rsplit(['/', '\\'])
                            .next()
                            .unwrap_or(&folder)
                            .to_owned();
                        let f = folder.clone();
                        Choice::new(label, move |ws, window, cx| ws.open_recent(f.clone(), window, cx)).detail(folder)
                    })
                    .collect();
                self.choose(prompt, choices, window, cx);
            }
            "file.clearRecent" => {
                let _ = app_state::state(cx).user.clear_recent();
                self.refresh_recent(cx);
            }
            "file.save" => {
                self.save(None, window, cx);
            }
            "file.new" => {
                if self.root.is_some() {
                    self.tasks.reveal("files");
                    self.layouts_changed(cx);
                    self.file_tree.update(cx, |t, cx| t.start_create(false, None, window, cx));
                } else {
                    self.new_file_anywhere(window, cx);
                }
            }
            "file.newFolder" => {
                if self.root.is_some() {
                    self.tasks.reveal("files");
                    self.layouts_changed(cx);
                    self.file_tree.update(cx, |t, cx| t.start_create(true, None, window, cx));
                } else {
                    self.new_project_folder(window, cx);
                }
            }
            "file.rename" => {
                if let Some(d) = self.active_doc() {
                    let path = PathBuf::from(d.read(cx).doc.path());
                    self.tasks.reveal("files");
                    self.layouts_changed(cx);
                    self.file_tree.update(cx, |t, cx| t.start_rename(Some(path), window, cx));
                }
            }
            "file.delete" => {
                if let Some(d) = self.active_doc() {
                    let path = PathBuf::from(d.read(cx).doc.path());
                    self.delete_path(path, false, window, cx);
                }
            }
            "file.lineEndings" => self.change_line_endings(window, cx),
            "file.reopenEncoding" => self.reopen_with_encoding(window, cx),
            "file.saveEncoding" => self.save_with_encoding(window, cx),

            // ---------- tabs and groups ----------
            "tab.close" => {
                if let Some(id) = self.tabs.active() {
                    self.close_tab(id, None, window, cx);
                }
            }
            "tab.next" | "tab.prev" => {
                self.tabs.next(if id == "tab.next" { 1 } else { -1 });
                self.after_tabs_changed(window, cx);
                self.focus_editor(window, cx);
            }
            "view.split" => {
                self.tabs.split();
                self.after_tabs_changed(window, cx);
                self.focus_editor(window, cx);
            }
            "view.moveToOtherGroup" => {
                self.tabs.move_to_other_group();
                self.after_tabs_changed(window, cx);
                self.focus_editor(window, cx);
            }
            "view.focusLeft" | "view.focusRight" => {
                let i = usize::from(id == "view.focusRight");
                if i < self.tabs.groups.len() {
                    self.tabs.focus_group(i);
                    self.after_tabs_changed(window, cx);
                    self.focus_editor(window, cx);
                }
            }
            "help.welcome" => {
                if self.welcome_forced {
                    self.close_welcome(window, cx);
                } else {
                    self.welcome_forced = true;
                    self.tasks.reveal("editor");
                    cx.notify();
                }
            }

            // ---------- run ----------
            "run.smart" | "run.file" => self.run_current_file(window, cx),
            "run.stop" => self.stop_run(cx),
            "run.selectConfig" => self.pick_run_config(window, cx),
            "run.editConfigs" => self.edit_run_file(window, cx),
            "tasks.run" => self.pick_task(window, cx),
            "tasks.rerun" => {
                if let Some(t) = self.run.last_task.clone() {
                    self.run_task(t, window, cx);
                }
            }

            // ---------- tests ----------
            "tests.runAll" => self.run_tests(Which::All, window, cx),
            "tests.runFailed" => self.run_tests(Which::Failed, window, cx),
            "tests.runFile" | "tests.runAtCursor" => {
                if let Some(d) = self.active_doc() {
                    let path = d.read(cx).doc.path().to_owned();
                    let line = (id == "tests.runAtCursor").then(|| editor.read(cx).cursor().line);
                    match self.tests.read(cx).ids_at(&path, line) {
                        Some(ids) => self.run_tests(Which::These(ids), window, cx),
                        None => {
                            self.reveal_panel("tests", cx);
                            self.tests.update(cx, |t, cx| {
                                t.discover(cx);
                                t.say("This file has no tests (test files are named test_*.py or *_test.py).", cx)
                            });
                        }
                    }
                }
            }
            "tests.stop" => self.tests.update(cx, |t, _| t.stop()),

            // ---------- Python ----------
            "python.selectInterpreter" => self.pick_interpreter(window, cx),
            "python.createVenv" => self.create_venv(None, window, cx),
            "python.packages" => self.show_packages(window, cx),
            "python.installRequirements" => self.install_requirements(window, cx),
            "python.checkDeps" => self.check_deps_command(window, cx),
            "python.installRuff" => self.pip_install("ruff", window, cx),

            // ---------- find ----------
            "find.open" | "find.replace" => {
                let replace = id == "find.replace";
                self.find_bar.update(cx, |f, cx| f.open(&editor, replace, window, cx));
            }
            "find.close" => self.find_bar.update(cx, |f, cx| f.close(window, cx)),

            // ---------- the editor ----------
            "editor.toggleComment" => editor.update(cx, |e, cx| e.toggle_comment(cx)),
            "editor.fold" => editor.update(cx, |e, cx| {
                let line = e.cursor().line;
                e.fold(line, cx)
            }),
            "editor.unfold" => editor.update(cx, |e, cx| {
                let line = e.cursor().line;
                e.unfold(line, cx)
            }),
            "editor.foldAll" => editor.update(cx, |e, cx| e.fold_all(cx)),
            "editor.unfoldAll" => editor.update(cx, |e, cx| e.unfold_all(cx)),
            "editor.toggleMinimap" => {
                let on = !self.editors[0].read(cx).minimap;
                for ed in &self.editors {
                    ed.update(cx, |e, cx| {
                        e.minimap = on;
                        cx.notify();
                    });
                }
            }

            // ---------- palette ----------
            "palette.commands" => self.open_palette(">", window, cx),
            "palette.files" => self.open_palette("", window, cx),
            "palette.line" => self.open_palette(":", window, cx),

            // ---------- tasks, panels, layout ----------
            "task.code" | "task.connect" | "task.debug" | "task.run" => {
                let task = &id[5..];
                self.tasks.switch_to(task);
                self.layouts_changed(cx);
                cx.notify();
            }
            "view.resetLayout" => {
                self.tasks.reset_current();
                self.layouts_changed(cx);
            }
            "view.editLayout" => {
                let on = !self.editing_layout;
                self.set_editing_layout(on, cx);
            }
            "panel.toggle" => {
                self.tasks.toggle_drawer();
                self.layouts_changed(cx);
            }
            "view.files" => self.reveal_panel("files", cx),
            "view.outline" => self.reveal_panel("outline", cx),
            "search.project" => self.reveal_panel("search", cx),
            "git.show" => self.reveal_panel("git", cx),
            "panel.output" => self.reveal_panel("output", cx),
            "panel.problems" => self.reveal_panel("problems", cx),
            "panel.debug" => self.reveal_panel("debug", cx),
            "panel.references" => self.reveal_panel("references", cx),
            "panel.calls" => self.reveal_panel("calls", cx),
            "panel.memory" => self.reveal_panel("memory", cx),
            "panel.network" => self.reveal_panel("network", cx),
            "tests.show" => self.reveal_panel("tests", cx),
            "view.projectGraph" => self.reveal_panel("graph", cx),
            "view.pipeline" => self.reveal_panel("pipeline", cx),
            "view.database" => self.reveal_panel("database", cx),
            "terminal.toggle" => {
                if self.tasks.is_shown("terminal") && self.terminal.read(cx).has_focus(window) {
                    self.tasks.hide("terminal");
                    self.layouts_changed(cx);
                    self.focus_editor(window, cx);
                } else {
                    self.reveal_panel("terminal", cx);
                    self.terminal.update(cx, |t, cx| t.shown(window, cx));
                }
            }
            "terminal.new" => {
                self.reveal_panel("terminal", cx);
                self.terminal.update(cx, |t, cx| {
                    t.ensure_started(cx);
                    if t.session_count() > 0 {
                        t.new_terminal(None, None, None, None, cx);
                    }
                    t.focus(window, cx);
                });
            }
            "terminal.newWithShell" => {
                let choices = forge_term::shells::installed_shells()
                    .into_iter()
                    .map(|p| {
                        let file = p.file.clone();
                        Choice::new(p.name.clone(), move |ws, window, cx| {
                            ws.reveal_panel("terminal", cx);
                            let profile = p.clone();
                            ws.terminal.update(cx, |t, cx| {
                                t.new_terminal(Some(profile), None, None, None, cx);
                                t.focus(window, cx);
                            });
                        })
                        .detail(file)
                    })
                    .collect();
                self.choose("New terminal with which shell?", choices, window, cx);
            }
            "terminal.kill" => self.terminal.update(cx, |t, cx| t.kill_active(cx)),
            "terminal.next" => self.terminal.update(cx, |t, cx| t.next(cx)),

            // ---------- settings and look ----------
            "view.zoomIn" | "view.zoomOut" => {
                let up = id == "view.zoomIn";
                let _ = app_state::update_settings(cx, |s| {
                    s.font_size = if up {
                        (s.font_size + 1).min(FONT_SIZE_MAX)
                    } else {
                        s.font_size.saturating_sub(1).max(FONT_SIZE_MIN)
                    };
                });
                self.apply_settings(cx);
            }
            "view.toggleTheme" => {
                let _ = app_state::update_settings(cx, |s| {
                    s.theme = if s.theme == ThemeSetting::Molten { ThemeSetting::Plain } else { ThemeSetting::Molten };
                });
                cx.notify();
            }
            "settings.open" => {
                if self.settings_view.read(cx).is_open() {
                    self.settings_view.update(cx, |v, cx| v.close(cx));
                } else {
                    self.settings_view.update(cx, |v, cx| v.open(window, cx));
                }
            }
            "settings.pickShell" => {
                let current = app_state::settings(cx).shell.clone();
                let mut choices = vec![Choice::new("System default", |ws, _, cx| {
                    let _ = app_state::update_settings(cx, |s| s.shell = String::new());
                    ws.apply_settings(cx);
                })];
                for p in forge_term::shells::installed_shells() {
                    let id = p.id.clone();
                    let c = Choice::new(p.name.clone(), move |ws, _, cx| {
                        let id = id.clone();
                        let _ = app_state::update_settings(cx, |s| s.shell = id);
                        ws.apply_settings(cx);
                    })
                    .detail(p.file.clone());
                    choices.push(if p.id == current { c.hint("current") } else { c });
                }
                self.choose("Shell for new terminals", choices, window, cx);
            }
            "settings.project" => {
                if let Some(root) = self.root.clone() {
                    match ensure_project_settings(&root, &Default::default()) {
                        Ok(path) => {
                            self.file_tree.update(cx, |t, cx| t.refresh_all(cx));
                            self.open_path(path, window, cx);
                        }
                        Err(err) => self.alert(format!("Couldn't create the project settings file: {err}"), window, cx),
                    }
                }
            }
            "keys.openFile" => match app_state::state(cx).user.ensure_keybindings_file() {
                Ok(path) => self.open_path(path, window, cx),
                Err(err) => self.alert(format!("Couldn't create the shortcuts file: {err}"), window, cx),
            },
            "keys.show" => self.show_shortcuts(window, cx),

            _ => {
                let title = app_state::state(cx).keymap.get(id).map_or(id, |c| c.title).to_owned();
                self.status_note(format!("{title} isn't in the native app yet"), cx);
            }
        }
    }

    pub(crate) fn reveal_panel(&mut self, panel: &str, cx: &mut Context<Self>) {
        self.tasks.reveal(panel);
        self.layouts_changed(cx);
        cx.notify();
    }

    /// New File with no folder open: choose where to save it, then open it.
    fn new_file_anywhere(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = dirs_home();
        let picked = cx.prompt_for_new_path(&dir, Some("untitled.py"));
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(path))) = picked.await {
                this.update_in(cx, |ws, window, cx| {
                    if !path.exists() {
                        if let Err(err) = std::fs::write(&path, "") {
                            return ws.alert(format!("Couldn't create {}:\n{err}", path.display()), window, cx);
                        }
                    }
                    ws.open_path(path, window, cx);
                })
                .ok();
            }
        })
        .detach();
    }

    /// New Folder with no folder open: make a project folder somewhere and open it.
    fn new_project_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose where the new folder goes".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else { return };
            let Some(parent) = paths.into_iter().next() else { return };
            this.update_in(cx, |ws, window, cx| {
                let shown = parent.display().to_string();
                ws.palette.update(cx, |p, cx| {
                    p.prompt(
                        format!("Name of the new folder (inside {shown})"),
                        "",
                        "Create",
                        move |ws, name, window, cx| match create_entry(&parent, &name, true) {
                            Ok(path) => ws.open_folder(path, window, cx),
                            Err(err) => ws.alert(err.to_string(), window, cx),
                        },
                        window,
                        cx,
                    )
                });
            })
            .ok();
        })
        .detach();
    }

    fn change_line_endings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.active_doc() else { return };
        let current = doc.read(cx).doc.eol();
        let choices = [(Eol::Lf, "Unix / macOS style (\\n)"), (Eol::CrLf, "Windows style (\\r\\n)")]
            .into_iter()
            .map(|(eol, detail)| {
                let d = doc.clone();
                let c = Choice::new(eol.label(), move |ws, window, cx| {
                    d.update(cx, |d, cx| {
                        d.doc.set_eol(eol);
                        cx.emit(crate::document::DocumentEvent::Changed);
                        cx.notify();
                    });
                    ws.after_tabs_changed(window, cx);
                })
                .detail(detail);
                if eol == current {
                    c.hint("current")
                } else {
                    c
                }
            })
            .collect();
        self.choose("Line endings for this file", choices, window, cx);
    }

    fn reopen_with_encoding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.active_doc() else { return };
        let current = doc.read(cx).doc.encoding;
        let choices = Encoding::ALL
            .into_iter()
            .map(|enc| {
                let c = Choice::new(enc.label(), move |ws, window, cx| ws.reopen_as(enc, window, cx));
                if enc == current {
                    c.hint("current")
                } else {
                    c
                }
            })
            .collect();
        self.choose("Reopen with encoding (reads the file again)", choices, window, cx);
    }

    /// Read the active file again as `encoding`, replacing its tabs' contents.
    fn reopen_as(&mut self, encoding: Encoding, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.tabs.active() else { return };
        let Some(doc) = self.docs.get(&id).cloned() else { return };
        let (name, path, dirty) = {
            let d = &doc.read(cx).doc;
            (d.name().to_owned(), d.path().to_owned(), d.dirty())
        };
        let go = move |ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>| {
            match forge_sys::text_file::read_text_file(std::path::Path::new(&path), true, Some(encoding)) {
                Ok(forge_sys::text_file::TextFile::Text { text, encoding, size }) => {
                    let fresh = cx.new(|_| {
                        crate::document::Document::with_options(
                            path.clone(),
                            &text,
                            encoding,
                            size > forge_text::encoding::LARGE_FILE_BYTES,
                        )
                    });
                    let new_id = ws.adopt_doc(fresh, cx);
                    ws.tabs.replace(id, new_id);
                    ws.forget_doc(id, cx);
                    ws.after_tabs_changed(window, cx);
                }
                Ok(_) => ws.alert(format!("Couldn't reopen {name}: the file is too large."), window, cx),
                Err(err) => ws.alert(format!("Couldn't reopen {name}:\n{err}"), window, cx),
            }
        };
        if !dirty {
            return go(self, window, cx);
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("{} has unsaved changes.", doc.read(cx).doc.name()),
            Some("Reopen it and lose them?"),
            &["Reopen", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                this.update_in(cx, |ws, window, cx| go(ws, window, cx)).ok();
            }
        })
        .detach();
    }

    fn save_with_encoding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.active_doc() else { return };
        let current = doc.read(cx).doc.encoding;
        let choices = Encoding::ALL
            .into_iter()
            .map(|enc| {
                let d = doc.clone();
                let c = Choice::new(enc.label(), move |ws, window, cx| {
                    d.update(cx, |d, _| d.doc.encoding = enc);
                    ws.save(None, window, cx);
                });
                if enc == current {
                    c.hint("current")
                } else {
                    c
                }
            })
            .collect();
        self.choose("Save with encoding", choices, window, cx);
    }

    /// Keyboard Shortcuts…: every command with its keys; pick one to change it.
    pub(crate) fn show_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let keymap = app_state::state(cx).keymap.clone();
        let conflicts = keymap.conflicts();
        let title_of = |id: &str| keymap.get(id).map_or(id.to_owned(), |c| c.title.to_owned());
        let mut commands: Vec<_> = keymap.commands().to_vec();
        commands.sort_by_key(|c| c.title.to_lowercase());
        let choices = commands
            .into_iter()
            .map(|c| {
                let keys = keymap.keys_of(c.id);
                let mut notes = Vec::new();
                if keymap.overrides().contains_key(c.id) {
                    notes.push("changed".to_owned());
                }
                let mut others: Vec<String> = conflicts
                    .iter()
                    .filter(|(k, _)| keys.contains(k))
                    .flat_map(|(_, ids)| ids.iter().filter(|i| *i != c.id).map(|i| title_of(i)))
                    .collect();
                others.dedup();
                if !others.is_empty() {
                    notes.push(format!("⚠ also used by {}", others.join(", ")));
                }
                let id = c.id;
                let choice = Choice::new(c.title, move |ws, window, cx| ws.edit_shortcut(id, window, cx))
                    .hint(if keys.is_empty() { "—".to_owned() } else { keys.join(", ") });
                if notes.is_empty() {
                    choice
                } else {
                    choice.detail(notes.join(" · "))
                }
            })
            .collect();
        let n = conflicts.len();
        let summary =
            if n == 0 { "no clashes".to_owned() } else { format!("{n} clash{}", if n > 1 { "es" } else { "" }) };
        self.choose(format!("Keyboard shortcuts ({summary}): pick one to change it"), choices, window, cx);
    }

    fn edit_shortcut(&mut self, id: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let keymap = app_state::state(cx).keymap.clone();
        let Some(c) = keymap.get(id) else { return };
        let keys = keymap.keys_of(id);
        let defaults = keymap.default_keys(id);
        let show = |k: &[String]| if k.is_empty() { "no shortcut".to_owned() } else { k.join("  or  ") };
        let mut choices =
            vec![Choice::new("Change shortcut…", move |ws, window, cx| ws.capture_shortcut(id, false, window, cx))
                .detail(format!("now: {}", show(&keys)))];
        if !keys.is_empty() {
            choices.push(
                Choice::new("Add another shortcut…", move |ws, window, cx| ws.capture_shortcut(id, true, window, cx))
                    .detail("keep the current one too"),
            );
            choices.push(Choice::new("Remove shortcut", move |ws, window, cx| {
                ws.save_shortcut(id, Some(vec![]), &[], "", window, cx)
            }));
        }
        if keymap.overrides().contains_key(id) {
            choices.push(
                Choice::new("Reset to default", move |ws, window, cx| ws.save_shortcut(id, None, &[], "", window, cx))
                    .detail(show(&defaults)),
            );
        }
        choices.push(Choice::new("← Back to all shortcuts", |ws, window, cx| ws.show_shortcuts(window, cx)));
        self.choose(c.title, choices, window, cx);
    }

    fn capture_shortcut(&mut self, id: &'static str, add: bool, window: &mut Window, cx: &mut Context<Self>) {
        let title = app_state::state(cx).keymap.get(id).map_or(id, |c| c.title);
        self.palette.update(cx, |p, cx| {
            p.capture_key(
                format!("New shortcut for \"{title}\""),
                move |ws, key, window, cx| {
                    let keymap = app_state::state(cx).keymap.clone();
                    let current = keymap.keys_of(id);
                    let next: Vec<String> = if add {
                        current.into_iter().filter(|k| *k != key).chain([key.clone()]).collect()
                    } else {
                        vec![key.clone()]
                    };
                    let others = keymap.users_of(&key, id);
                    if others.is_empty() {
                        return ws.save_shortcut(id, Some(next), &[], "", window, cx);
                    }
                    let names = others.iter().map(|c| c.title).collect::<Vec<_>>().join(", ");
                    let other_ids: Vec<&'static str> = others.iter().map(|c| c.id).collect();
                    let (n1, n2, k1) = (next.clone(), next, key.clone());
                    let choices = vec![
                        Choice::new(
                            format!("Use {key} for \"{title}\" and remove it from {names}"),
                            move |ws, window, cx| ws.save_shortcut(id, Some(n1.clone()), &other_ids, &k1, window, cx),
                        ),
                        Choice::new(format!("Use {key} for both"), move |ws, window, cx| {
                            ws.save_shortcut(id, Some(n2.clone()), &[], "", window, cx)
                        })
                        .detail("whichever is available at the time runs; the first listed wins"),
                        Choice::new("Pick a different shortcut…", move |ws, window, cx| {
                            ws.capture_shortcut(id, add, window, cx)
                        }),
                        Choice::new("Cancel", |_, _, _| {}),
                    ];
                    ws.choose(format!("{key} is already used by {names}"), choices, window, cx);
                },
                window,
                cx,
            )
        });
    }

    /// Save `keys` for a command (None = back to the default), optionally taking `key` away from others.
    fn save_shortcut(
        &mut self,
        id: &str,
        keys: Option<Vec<String>>,
        take_from: &[&str],
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = app_state::state(cx).keymap.changed(id, keys, take_from, key);
        match app_state::state(cx).user.save_keybindings(&next) {
            Ok(()) => cx.global_mut::<AppState>().keymap.set_overrides(next),
            Err(err) => self.alert(format!("Couldn't save the shortcut: {err}"), window, cx),
        }
        cx.notify();
    }
}

/// "~/project" -> the home folder's project.
fn expand_home(text: &str) -> PathBuf {
    let t = text.trim();
    match t.strip_prefix('~') {
        Some(rest) => {
            let mut p = dirs_home();
            p.push(rest.trim_start_matches(['/', '\\']));
            p
        }
        None => PathBuf::from(t),
    }
}

fn dirs_home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::expand_home;

    #[test]
    fn a_leading_tilde_means_the_home_folder() {
        let p = expand_home("~/proj");
        assert!(p.ends_with("proj"));
        assert!(!p.to_string_lossy().contains('~'));
        assert_eq!(expand_home("  C:\\x  "), std::path::PathBuf::from("C:\\x"));
    }

    #[test]
    fn same_path_is_used_for_reloads() {
        assert!(forge_core::tabs::same_path("C:\\A\\b.json", "c:/a/B.json"));
    }
}
