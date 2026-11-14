//! Run (F5 / Ctrl+F5): running the open Python file or the chosen run
//! configuration with its output in the Output panel; run configurations and
//! tasks from `.forge/run.json`. Replaces the run parts of `main.ts`.

use super::Workspace;
use crate::palette::Choice;
use crate::panels::output::OutputEvent;
use forge_core::output::Stream;
use forge_core::run_config::{
    normalize_run_file, resolve_configuration, resolve_task, starter_run_file, LaunchSpec, RunFile, Task, Vars,
    RUN_FILE,
};
use forge_run::{PythonRun, RunEvent, RunSpec};
use forge_sys::user_state::{ensure_project_file, read_project_file};
use futures::StreamExt;
use gpui_kit::{Context, Entity, Window};
use std::path::{Path, PathBuf};

/// The run state of a window.
#[derive(Default)]
pub(crate) struct RunState {
    pub data_target: Option<String>,
    pub memory_next: bool,
    pub file: RunFile,
    pub file_path: Option<PathBuf>,
    /// The configuration Run and Debug use; None = the open file.
    pub config: Option<String>,
    pub last_task: Option<Task>,
    pub current: Option<PythonRun>,
    /// Goes up with every run, so late events from an old run are ignored.
    pub id: u64,
    pub _pump: Option<gpui_kit::Task<()>>,
    /// What to do when the current run ends (pip / venv runs re-check the environment).
    pub after: Option<super::pyenv::AfterRun>,
    pub last_failed: bool,
}

impl Workspace {
    pub(crate) fn on_output_event(
        &mut self,
        _p: &Entity<crate::panels::output::OutputPanel>,
        e: &OutputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match e {
            OutputEvent::OpenLink(path, line) => {
                self.open_path_at(PathBuf::from(path), Some(line.saturating_sub(1)), window, cx)
            }
            OutputEvent::SendInput(text) => {
                if let Some(r) = &self.run.current {
                    r.send_input(text);
                }
            }
            OutputEvent::Run => self.run_command("run.file", window, cx),
            OutputEvent::Stop => self.stop_run(cx),
        }
    }

    /// Read `.forge/run.json` (missing: no configurations).
    pub(crate) fn load_run_file(&mut self, folder: &Path, window: &mut Window, cx: &mut Context<Self>) {
        match read_project_file(folder, RUN_FILE) {
            Ok((f, path)) => {
                self.run.file = normalize_run_file(&f.value());
                self.run.file_path = Some(path.clone());
                if let Some(err) = f.error() {
                    self.alert(
                        format!(
                            "The project's run configurations couldn't be read, so none are used for now:\n{}\n\n{err}",
                            path.display()
                        ),
                        window,
                        cx,
                    );
                }
            }
            Err(_) => self.run.file = RunFile::default(),
        }
        if self.run.config.as_ref().is_some_and(|c| !self.run.file.configurations.iter().any(|x| &x.name == c)) {
            self.run.config = None;
        }
        cx.notify();
    }

    fn selected_config(&self) -> Option<&forge_core::run_config::RunConfiguration> {
        let name = self.run.config.as_ref()?;
        self.run.file.configurations.iter().find(|c| &c.name == name)
    }

    /// Can Run / Debug start something now?
    pub(crate) fn can_launch(&self) -> bool {
        self.tabs.active().is_some() || self.selected_config().is_some()
    }

    pub(crate) fn is_running(&self, cx: &gpui_kit::App) -> bool {
        self.output.read(cx).is_running()
    }

    /// What Run / Debug start: the selected configuration, or the open file.
    pub(crate) fn launch_spec(&self, cx: &gpui_kit::App) -> Result<LaunchSpec, String> {
        let file = self.active_doc().map(|d| d.read(cx).doc.path().to_owned());
        let root = self.root.as_ref().map(|r| r.to_string_lossy().into_owned());
        if let (Some(cfg), Some(root)) = (self.selected_config(), &root) {
            return resolve_configuration(cfg, &Vars { project: root.clone(), file });
        }
        let Some(file) = file else {
            return Err(if root.is_some() {
                "Open a Python file first (pick one in the tree, or Ctrl+P), or pick a run configuration.".into()
            } else {
                "Open a folder (Ctrl+O) and a Python file first.".into()
            });
        };
        let cwd = root
            .unwrap_or_else(|| Path::new(&file).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
        Ok(LaunchSpec { program: Some(file), module: None, args: Vec::new(), env: Default::default(), cwd })
    }

    /// "app.py --port 8000" or "-m flask run", for the Output header.
    fn describe_launch(&self, spec: &LaunchSpec) -> String {
        let quote = |a: &String| if a.is_empty() || a.contains([' ', '"']) { format!("{a:?}") } else { a.clone() };
        let what = spec.program.clone().unwrap_or_else(|| format!("-m {}", spec.module.clone().unwrap_or_default()));
        let mut parts: Vec<String> = Vec::new();
        if let Some(c) = self.selected_config() {
            parts.push(format!("[{}]", c.name));
        }
        parts.push(what);
        parts.extend(spec.args.iter().map(quote));
        parts.join(" ")
    }

    /// Save every unsaved file (Run sees what's in the editor). False if one failed.
    pub(crate) fn save_all_dirty(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let dirty: Vec<_> = self.tabs.docs().into_iter().filter(|d| self.is_dirty(*d, cx)).collect();
        dirty.into_iter().all(|id| self.save(Some(id), window, cx))
    }

    /// Run the open file (or the selected configuration) with its output in the Output panel.
    pub(crate) fn run_current_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let data_target = self.run.data_target.take();
        let memory = std::mem::take(&mut self.run.memory_next) || self.panel_is_shown("memory");
        let trace = memory
            || data_target.is_some()
            || self.panel_is_shown("network")
            || (self.effective_settings(cx).show_runs_on_graph && self.panel_is_shown("graph"));
        self.reveal_panel("output", cx);
        let spec = match self.launch_spec(cx) {
            Ok(s) => s,
            Err(e) => {
                self.output.update(cx, |o, cx| {
                    o.clear(cx);
                    o.error(&e, cx);
                });
                return;
            }
        };
        let what = self.describe_launch(&spec);
        self.output.update(cx, |o, cx| o.begin(&what, cx));
        if spec.program.as_ref().is_some_and(|p| !p.to_lowercase().ends_with(".py")) {
            self.output.update(cx, |o, cx| {
                o.error(
                    "Run works with Python files (.py). To run something else, add a task (Run Task… → Configure).",
                    cx,
                )
            });
            return;
        }
        if !self.save_all_dirty(window, cx) {
            self.output.update(cx, |o, cx| o.error("Run cancelled: a file couldn't be saved.", cx));
            return;
        }
        let root = self.root.clone().unwrap_or_else(|| PathBuf::from(&spec.cwd));
        let python = Some(self.effective_settings(cx).python_path).filter(|p| !p.is_empty());
        let mut run_spec = RunSpec {
            root,
            python,
            program: spec.program,
            module: spec.module,
            args: spec.args,
            env: spec.env,
            cwd: PathBuf::from(&spec.cwd),
            prefix: Vec::new(),
            trace,
        };
        if let Some(target) = data_target {
            run_spec.env.insert("FORGE_TRACE_DATA".into(), target);
        }
        if memory {
            run_spec.env.insert("FORGE_TRACE_MEMORY".into(), "1".into());
        }
        self.start_run(run_spec, window, cx);
        self.focus_editor(window, cx);
    }

    /// Start a program with its output in the Output panel.
    pub(crate) fn start_run(&mut self, spec: RunSpec, window: &mut Window, cx: &mut Context<Self>) {
        self.stop_run(cx);
        self.run.after = None;
        self.run.last_failed = false;
        self.project_graph.update(cx, |p, cx| p.begin_run(cx));
        self.run.id += 1;
        let id = self.run.id;
        self.network.update(cx, |p, cx| p.begin(id, cx));
        self.pipeline.update(cx, |p, cx| p.begin(id, cx));
        self.memory.update(cx, |p, cx| p.begin(id, cx));
        let project = self.root.clone();
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<RunEvent>();
        self.run.current = PythonRun::start(spec, move |e| {
            let _ = tx.unbounded_send(e);
        });
        self.run._pump = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(e) = rx.next().await {
                let ok = this.update_in(cx, |ws, window, cx| {
                    if ws.run.id != id {
                        return; // a late event from a run that was replaced
                    }
                    let out = ws.output.clone();
                    match e {
                        RunEvent::Trace(message) => {
                            if ws.root == project {
                                ws.on_trace_message(&message, cx)
                            }
                        }
                        RunEvent::Start { python, .. } => out.update(cx, |o, cx| o.started(&python, cx)),
                        RunEvent::Output { stream, text } => {
                            out.update(cx, |o, cx| o.append(stream == Stream::Stderr, &text, cx))
                        }
                        RunEvent::Error(m) => out.update(cx, |o, cx| o.error(&m, cx)),
                        RunEvent::Exit { code, ms } => {
                            if ws.root == project {
                                ws.project_graph.update(cx, |p, cx| p.end_run(cx));
                                ws.network.update(cx, |p, cx| p.end(cx));
                                ws.memory.update(cx, |p, cx| {
                                    p.log.running = false;
                                    cx.notify();
                                });
                                ws.run.last_failed = code.is_some_and(|code| code != 0);
                            }
                            out.update(cx, |o, cx| o.finished(code, ms, cx));
                            ws.run.current = None;
                            if let Some(after) = ws.run.after.take() {
                                after(ws, code, window, cx);
                            }
                        }
                    }
                    cx.notify();
                });
                if ok.is_err() {
                    break;
                }
            }
        }));
        cx.notify();
    }

    pub(crate) fn stop_run(&mut self, cx: &mut Context<Self>) {
        if let Some(r) = &self.run.current {
            r.stop();
        }
        cx.notify();
    }

    /// Open run.json, creating it with examples first if needed.
    pub(crate) fn edit_run_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        let has_requirements = root.join("requirements.txt").is_file();
        let starter = serde_json::to_value(starter_run_file(has_requirements)).unwrap_or_default();
        match ensure_project_file(&root, RUN_FILE, &starter) {
            Ok(path) => {
                self.file_tree.update(cx, |t, cx| t.refresh_all(cx));
                self.open_path(path, window, cx);
                self.load_run_file(&root, window, cx);
            }
            Err(err) => self.alert(format!("Couldn't create the run configurations file: {err}"), window, cx),
        }
    }

    pub(crate) fn pick_run_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.run.config.clone();
        let mut choices = vec![{
            let c = Choice::new("Current file", |ws, _, cx| ws.set_run_config(None, cx))
                .detail("run whichever Python file is open");
            if current.is_none() {
                c.hint("✓")
            } else {
                c
            }
        }];
        for cfg in &self.run.file.configurations {
            let name = cfg.name.clone();
            let mut detail = vec![cfg
                .module
                .as_ref()
                .map_or_else(|| cfg.file.clone().unwrap_or_else(|| "${file}".into()), |m| format!("-m {m}"))];
            detail.extend(cfg.args.iter().cloned());
            let c = Choice::new(cfg.name.clone(), move |ws, _, cx| ws.set_run_config(Some(name.clone()), cx))
                .detail(detail.join(" "));
            choices.push(if current.as_ref() == Some(&cfg.name) { c.hint("✓") } else { c });
        }
        let label = if self.run.file.configurations.is_empty() {
            "Add a run configuration…"
        } else {
            "Edit run configurations…"
        };
        choices.push(Choice::new(label, |ws, window, cx| ws.edit_run_file(window, cx)).detail(".forge/run.json"));
        self.choose("Run configuration for F5 / Ctrl+F5", choices, window, cx);
    }

    fn set_run_config(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        self.run.config = name;
        self.save_state_soon(cx);
        cx.notify();
    }

    pub(crate) fn run_task(&mut self, task: Task, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        let file = self.active_doc().map(|d| d.read(cx).doc.path().to_owned());
        match resolve_task(&task, &Vars { project: root.to_string_lossy().into_owned(), file }) {
            Ok((command, cwd)) => {
                self.run.last_task = Some(task.clone());
                self.reveal_panel("terminal", cx);
                self.terminal.update(cx, |t, cx| {
                    t.ensure_started(cx);
                    t.run_command(&task.name, &command, PathBuf::from(cwd), cx);
                    t.focus(window, cx);
                });
            }
            Err(e) => self.alert(e, window, cx),
        }
    }

    pub(crate) fn pick_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut choices: Vec<Choice> = self
            .run
            .file
            .tasks
            .iter()
            .map(|t| {
                let task = t.clone();
                Choice::new(t.name.clone(), move |ws, window, cx| ws.run_task(task.clone(), window, cx))
                    .detail(t.command.clone())
            })
            .collect();
        let none = choices.is_empty();
        choices.push(
            Choice::new(if none { "Add a task…" } else { "Edit tasks…" }, |ws, window, cx| {
                ws.edit_run_file(window, cx)
            })
            .detail(".forge/run.json"),
        );
        self.choose(
            if none { "No tasks yet: add some to .forge/run.json" } else { "Run a task (in a new terminal)" },
            choices,
            window,
            cx,
        );
    }
}
