//! Python environments and packages for the open project: which interpreter
//! it uses (status bar 🐍; pick another, or create a .venv), its packages
//! (install, upgrade, uninstall, freeze to requirements.txt), and packages the
//! project asks for that aren't installed. The chosen interpreter is saved in
//! `.forge/settings.json`; pip and venv run in the Output panel. Replaces
//! `src/renderer/pythonEnv.ts`.

use super::Workspace;
use crate::palette::Choice;
use forge_core::deps::{is_installable_spec, Requirement};
use forge_core::tabs::{is_inside, same_path};
use forge_run::pyenv::{
    check_dependencies, current_interpreter, find_interpreters, freeze, interpreter_label, list_packages,
    InterpreterInfo,
};
use forge_run::RunSpec;
use forge_sys::user_state::update_project_settings;
use gpui_kit::{Context, PromptLevel, Window};
use std::path::PathBuf;

/// What to do after a pip / venv run ends (its exit code).
pub(crate) type AfterRun = Box<dyn FnOnce(&mut Workspace, Option<i32>, &mut Window, &mut Context<Workspace>)>;

#[derive(Default)]
pub(crate) struct PyEnvState {
    /// The interpreter Run / Debug / the language server use (None: none found, or no folder).
    pub current: Option<InterpreterInfo>,
    /// Declared dependencies that aren't installed.
    pub missing: Vec<Requirement>,
    /// Which of requirements.txt / pyproject.toml the project has.
    pub sources: Vec<String>,
    checked: bool,
    generation: u64,
    task: Option<gpui_kit::Task<()>>,
}

impl Workspace {
    fn configured_python(&self, cx: &gpui_kit::App) -> Option<String> {
        Some(self.effective_settings(cx).python_path).filter(|p| !p.is_empty())
    }

    /// Find the interpreter in use and check the dependencies (after opening a folder,
    /// changing settings, pip runs...).
    pub(crate) fn refresh_python_env(&mut self, cx: &mut Context<Self>) {
        self.pyenv.generation += 1;
        let generation = self.pyenv.generation;
        let Some(root) = self.root.clone() else {
            self.pyenv = PyEnvState { generation, ..Default::default() };
            cx.notify();
            return;
        };
        let configured = self.configured_python(cx);
        self.pyenv.task = Some(cx.spawn(async move |this, cx| {
            let r = root.clone();
            let found =
                cx.background_executor().spawn(async move { current_interpreter(&r, configured.as_deref()) }).await;
            let path = found.as_ref().map(|i| i.path.clone());
            let ok = this.update(cx, |ws, cx| {
                if ws.pyenv.generation != generation {
                    return false;
                }
                ws.pyenv.current = found;
                ws.pyenv.checked = true;
                if ws.pyenv.current.is_none() {
                    ws.pyenv.missing.clear();
                }
                cx.notify();
                true
            });
            let (Ok(true), Some(python)) = (ok, path) else { return };
            let deps = cx.background_executor().spawn(async move { check_dependencies(&root, &python) }).await;
            this.update(cx, |ws, cx| {
                if ws.pyenv.generation == generation {
                    ws.pyenv.missing = deps.missing;
                    ws.pyenv.sources = deps.sources;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// Status bar text for the interpreter.
    pub(crate) fn python_status(&self) -> Option<String> {
        self.root.as_ref()?;
        Some(match &self.pyenv.current {
            Some(i) => format!("🐍 {}", interpreter_label(i).trim_start_matches("Python ")),
            None if self.pyenv.checked => "🐍 No Python".into(),
            None => "🐍 …".into(),
        })
    }

    /// Status bar text for missing packages (None when there are none).
    pub(crate) fn missing_status(&self) -> Option<String> {
        let n = self.pyenv.missing.len();
        (n > 0).then(|| format!("⚠ {n} missing package{}", if n > 1 { "s" } else { "" }))
    }

    /// Run python -m pip / venv / ensurepip in the Output panel (in the project folder).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_tool(
        &mut self,
        title: &str,
        python: String,
        module: &str,
        args: Vec<String>,
        after: Option<AfterRun>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.root.clone() else { return };
        self.reveal_panel("output", cx);
        self.output.update(cx, |o, cx| o.begin(title, cx));
        let spec = RunSpec {
            root: root.clone(),
            python: Some(python),
            module: Some(module.into()),
            args,
            cwd: root,
            ..Default::default()
        };
        self.start_run(spec, window, cx);
        self.run.after = after;
    }

    fn pip(&mut self, title: &str, args: &[&str], window: &mut Window, cx: &mut Context<Self>) {
        let Some(python) = self.pyenv.current.as_ref().map(|i| i.path.clone()) else {
            return self.alert(
                "No Python interpreter was found for this project. Pick one first (click 🐍 in the status bar).",
                window,
                cx,
            );
        };
        let mut a: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        a.push("--disable-pip-version-check".into());
        self.run_tool(title, python, "pip", a, Some(Box::new(|ws, _, _, cx| ws.refresh_python_env(cx))), window, cx);
    }

    pub(crate) fn pip_install(&mut self, spec: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.pip(&format!("pip install {spec}"), &["install", spec], window, cx);
    }

    pub(crate) fn install_requirements(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pyenv.sources.iter().any(|s| s == "requirements.txt")
            && !self.root.as_ref().is_some_and(|r| r.join("requirements.txt").is_file())
        {
            return self.alert("This project has no requirements.txt.", window, cx);
        }
        self.pip("pip install -r requirements.txt", &["install", "-r", "requirements.txt"], window, cx);
    }

    fn install_missing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names: Vec<String> = self
            .pyenv
            .missing
            .iter()
            .map(|r| {
                let spec = r.spec.split(';').next().unwrap_or("");
                let spec = if spec.trim_start().starts_with('@') { "" } else { spec };
                format!("{}{}", r.name, spec.split_whitespace().collect::<String>())
            })
            .collect();
        if names.is_empty() {
            return;
        }
        let mut args = vec!["install"];
        args.extend(names.iter().map(String::as_str));
        self.pip(&format!("pip install {}", names.join(" ")), &args, window, cx);
    }

    fn install_choices(&self) -> Vec<Choice> {
        let mut out = Vec::new();
        if self.pyenv.sources.iter().any(|s| s == "requirements.txt") {
            out.push(
                Choice::new("Install from requirements.txt", |ws, w, cx| ws.install_requirements(w, cx))
                    .detail("pip install -r requirements.txt"),
            );
        }
        if self.pyenv.sources.iter().any(|s| s == "pyproject.toml") {
            out.push(
                Choice::new("Install this project (editable)", |ws, w, cx| {
                    ws.pip("pip install -e .", &["install", "-e", "."], w, cx)
                })
                .detail("pip install -e ."),
            );
        }
        out
    }

    /// Clicking "⚠ n missing packages".
    pub(crate) fn show_missing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pyenv.missing.is_empty() {
            return;
        }
        let names: Vec<String> = self.pyenv.missing.iter().map(|r| r.name.clone()).collect();
        let n = names.len();
        let mut choices =
            vec![Choice::new(format!("Install the missing package{}", if n > 1 { "s" } else { "" }), |ws, w, cx| {
                ws.install_missing(w, cx)
            })
            .detail(names.join(", "))];
        choices.extend(self.install_choices());
        let from = format!("from {}", self.pyenv.sources.join(" / "));
        for r in &self.pyenv.missing {
            choices.push(
                Choice::new(r.name.clone(), |_, _, _| {})
                    .detail(if r.spec.is_empty() { "any version".to_owned() } else { r.spec.clone() })
                    .hint(from.clone()),
            );
        }
        let what = self.pyenv.current.as_ref().map_or("this interpreter".to_owned(), interpreter_label);
        self.choose(format!("Not installed in {what}"), choices, window, cx);
    }

    pub(crate) fn check_deps_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        let configured = self.configured_python(cx);
        cx.spawn_in(window, async move |this, cx| {
            let r = root.clone();
            let found =
                cx.background_executor().spawn(async move { current_interpreter(&r, configured.as_deref()) }).await;
            let deps = match &found {
                Some(i) => {
                    let (root, p) = (root.clone(), i.path.clone());
                    Some(cx.background_executor().spawn(async move { check_dependencies(&root, &p) }).await)
                }
                None => None,
            };
            this.update_in(cx, |ws, window, cx| {
                ws.pyenv.current = found;
                ws.pyenv.checked = true;
                if let Some(d) = deps {
                    ws.pyenv.missing = d.missing;
                    ws.pyenv.sources = d.sources;
                }
                if ws.pyenv.missing.is_empty() {
                    ws.alert("Every package in requirements.txt / pyproject.toml is installed.", window, cx);
                } else {
                    ws.show_missing(window, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ---------- interpreter ----------

    pub(crate) fn pick_interpreter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        self.choose("Looking for Python interpreters…", Vec::new(), window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let r = root.clone();
            let found = cx.background_executor().spawn(async move { find_interpreters(Some(&r)) }).await;
            this.update_in(cx, |ws, window, cx| {
                if !ws.palette.read(cx).is_open() {
                    return; // closed while searching
                }
                let configured = ws.configured_python(cx);
                let root_s = root.to_string_lossy().into_owned();
                let mut choices = vec![{
                    let c = Choice::new("Automatic", |ws, w, cx| ws.choose_python(None, w, cx))
                        .detail("the project's .venv if it has one, else python on PATH");
                    if configured.is_none() {
                        c.hint("✓")
                    } else {
                        c
                    }
                }];
                for info in &found {
                    let path = info.path.clone();
                    let current = configured.is_some()
                        && ws.pyenv.current.as_ref().is_some_and(|c| same_path(&c.path, &info.path));
                    let c = Choice::new(interpreter_label(info), move |ws, w, cx| {
                        ws.choose_python(Some(path.clone()), w, cx)
                    })
                    .detail(info.path.clone());
                    choices.push(if current {
                        c.hint("✓")
                    } else if is_inside(&info.path, &root_s) {
                        c.hint("project")
                    } else {
                        c
                    });
                }
                let known = found.clone();
                choices.push(
                    Choice::new("Create a virtual environment (.venv)…", move |ws, w, cx| {
                        ws.create_venv(Some(known.clone()), w, cx)
                    })
                    .detail("python -m venv .venv in this project"),
                );
                choices.push(Choice::new("Enter interpreter path…", |ws, window, cx| {
                    ws.palette.update(cx, |p, cx| {
                        p.prompt(
                            "Path to python (e.g. C:\\Python312\\python.exe or .venv/bin/python)",
                            "",
                            "Use",
                            |ws, text, w, cx| ws.choose_python(Some(text), w, cx),
                            window,
                            cx,
                        )
                    })
                }));
                ws.choose(
                    "Python interpreter for this project (Run, Debug, errors and completion)",
                    choices,
                    window,
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    /// Save the choice (relative to the project when it's inside it) and re-check.
    fn choose_python(&mut self, path: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        let root_s = root.to_string_lossy().into_owned();
        let value = path.as_ref().map(|p| {
            if is_inside(p, &root_s) {
                p.get(root_s.trim_end_matches(['/', '\\']).len()..)
                    .unwrap_or(p)
                    .trim_start_matches(['/', '\\'])
                    .replace('\\', "/")
            } else {
                p.clone()
            }
        });
        if let Err(err) = self.set_project_python(value, window, cx) {
            return self.alert(format!("Couldn't save the interpreter choice: {err}"), window, cx);
        }
        if let Some(p) = path {
            // Say so if it doesn't run.
            cx.spawn_in(window, async move |this, cx| {
                let q = p.clone();
                let ok = cx
                    .background_executor()
                    .spawn(async move { forge_run::pyenv::describe_python(&q).is_some() })
                    .await;
                if !ok {
                    this.update_in(cx, |ws, window, cx| {
                        ws.alert(format!("That interpreter didn't run:\n{p}"), window, cx)
                    })
                    .ok();
                }
            })
            .detach();
        }
    }

    fn set_project_python(
        &mut self,
        value: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> std::io::Result<()> {
        let Some(root) = self.root.clone() else { return Ok(()) };
        let mut patch = serde_json::Map::new();
        patch.insert("pythonPath".into(), value.map_or(serde_json::Value::Null, serde_json::Value::String));
        update_project_settings(&root, &patch)?;
        self.reload_project_settings(window, cx);
        self.file_tree.update(cx, |t, cx| t.refresh_all(cx));
        self.refresh_python_env(cx);
        Ok(())
    }

    pub(crate) fn create_venv(
        &mut self,
        known: Option<Vec<InterpreterInfo>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.root.clone() else { return };
        let Some(found) = known else {
            cx.spawn_in(window, async move |this, cx| {
                let r = root.clone();
                let found = cx.background_executor().spawn(async move { find_interpreters(Some(&r)) }).await;
                this.update_in(cx, |ws, window, cx| ws.create_venv(Some(found), window, cx)).ok();
            })
            .detach();
            return;
        };
        let root_s = root.to_string_lossy().into_owned();
        let existing = found.iter().find(|i| {
            i.is_venv && is_inside(&i.prefix, &root_s) && i.prefix.trim_end_matches(['/', '\\']).ends_with(".venv")
        });
        if let Some(e) = existing.cloned() {
            let label = format!("Use it ({})", interpreter_label(&e));
            let path = e.path.clone();
            let choices = vec![
                Choice::new(label, move |ws, w, cx| ws.choose_python(Some(path.clone()), w, cx)),
                Choice::new("Cancel", |_, _, _| {}),
            ];
            return self.choose("This project already has a .venv", choices, window, cx);
        }
        let bases: Vec<InterpreterInfo> = found.into_iter().filter(|i| !i.is_venv).collect();
        if bases.is_empty() {
            return self.alert("No Python installation was found to create the environment from. Install Python from python.org first.", window, cx);
        }
        let create =
            move |ws: &mut Workspace, base: InterpreterInfo, window: &mut Window, cx: &mut Context<Workspace>| {
                let target = root.join(".venv").to_string_lossy().into_owned();
                let after: AfterRun = Box::new(|ws, code, window, cx| {
                    if code != Some(0) {
                        return;
                    }
                    let python = if cfg!(windows) { ".venv/Scripts/python.exe" } else { ".venv/bin/python" };
                    if ws.set_project_python(Some(python.into()), window, cx).is_ok() {
                        ws.offer_install_after_create(window, cx);
                    }
                });
                ws.run_tool(
                    &format!("Creating .venv with {}", interpreter_label(&base)),
                    base.path.clone(),
                    "venv",
                    vec![target],
                    Some(after),
                    window,
                    cx,
                );
            };
        if bases.len() == 1 {
            let b = bases[0].clone();
            return create(self, b, window, cx);
        }
        let create = std::rc::Rc::new(create);
        let choices = bases
            .into_iter()
            .map(|b| {
                let create = create.clone();
                let label = interpreter_label(&b);
                let detail = b.path.clone();
                Choice::new(label, move |ws, w, cx| create(ws, b.clone(), w, cx)).detail(detail)
            })
            .collect();
        self.choose("Create the .venv from which Python?", choices, window, cx);
    }

    fn offer_install_after_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        let has_req = root.join("requirements.txt").is_file();
        let has_py = root.join("pyproject.toml").is_file();
        if !has_req && !has_py {
            return;
        }
        let mut choices = Vec::new();
        if has_req {
            choices.push(
                Choice::new("Install from requirements.txt", |ws, w, cx| ws.install_requirements(w, cx))
                    .detail("pip install -r requirements.txt"),
            );
        }
        if has_py {
            choices.push(
                Choice::new("Install this project (editable)", |ws, w, cx| {
                    ws.pip("pip install -e .", &["install", "-e", "."], w, cx)
                })
                .detail("pip install -e ."),
            );
        }
        choices.push(Choice::new("Not now", |_, _, _| {}));
        self.choose("The .venv is ready and selected. Install the project's packages into it?", choices, window, cx);
    }

    // ---------- packages ----------

    /// "Python Packages…": installed packages and what to do with them.
    pub(crate) fn show_packages(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(info) = self.pyenv.current.clone() else {
            return self.alert(
                "No Python interpreter was found for this project. Pick one first (click 🐍 in the status bar).",
                window,
                cx,
            );
        };
        self.choose("Reading installed packages…", Vec::new(), window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let p = info.path.clone();
            let res = cx.background_executor().spawn(async move { list_packages(&p) }).await;
            this.update_in(cx, |ws, window, cx| {
                if !ws.palette.read(cx).is_open() {
                    return;
                }
                let packages = match res {
                    Ok(p) => p,
                    Err(e) => {
                        let python = info.path.clone();
                        let choices = vec![Choice::new("Install pip", move |ws, w, cx| {
                            let after: AfterRun = Box::new(|ws, _, _, cx| ws.refresh_python_env(cx));
                            ws.run_tool("python -m ensurepip --upgrade", python.clone(), "ensurepip", vec!["--upgrade".into()], Some(after), w, cx)
                        })
                        .detail("python -m ensurepip --upgrade")];
                        return ws.choose(format!("Couldn't list packages: {e}"), choices, window, cx);
                    }
                };
                let mut choices = vec![Choice::new("Install a package…", |ws, window, cx| {
                    ws.palette.update(cx, |p, cx| {
                        p.prompt("Package to install, e.g. requests or \"fastapi>=0.110\"", "", "Install", |ws, text, w, cx| {
                            let spec = text.trim().trim_matches(['"', '\'']).to_owned();
                            if !is_installable_spec(&spec) {
                                return ws.alert(format!("\"{spec}\" doesn't look like a package name. Use a name like requests, uvicorn[standard] or numpy==1.26.4."), w, cx);
                            }
                            let compact: String = spec.split_whitespace().collect();
                            ws.pip_install(&compact, w, cx);
                        }, window, cx)
                    })
                })
                .detail("pip install <name>")];
                choices.extend(ws.install_choices());
                if !ws.pyenv.missing.is_empty() {
                    let names: Vec<String> = ws.pyenv.missing.iter().map(|r| r.name.clone()).collect();
                    choices.push(Choice::new(format!("Install {} missing", names.len()), |ws, w, cx| ws.install_missing(w, cx)).detail(names.join(", ")));
                }
                choices.push(Choice::new("Save installed packages to requirements.txt", |ws, w, cx| ws.freeze_to_file(w, cx)).detail("pip freeze > requirements.txt"));
                choices.push(Choice::new("Upgrade pip", |ws, w, cx| ws.pip("pip install --upgrade pip", &["install", "--upgrade", "pip"], w, cx)).detail("pip install --upgrade pip"));
                let mut sorted = packages;
                sorted.sort_by_key(|p| p.name.to_lowercase());
                let n = sorted.len();
                for p in sorted {
                    let (name, version) = (p.name.clone(), p.version.clone());
                    choices.push(Choice::new(p.name.clone(), move |ws, w, cx| ws.package_actions(name.clone(), version.clone(), w, cx)).hint(p.version.clone()));
                }
                ws.choose(format!("{}: {n} packages installed", interpreter_label(&info)), choices, window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn package_actions(&mut self, name: String, version: String, window: &mut Window, cx: &mut Context<Self>) {
        let (n1, n2) = (name.clone(), name.clone());
        let choices = vec![
            Choice::new(format!("Upgrade {name}"), move |ws, w, cx| {
                ws.pip(&format!("pip install --upgrade {n1}"), &["install", "--upgrade", &n1], w, cx)
            })
            .detail(format!("pip install --upgrade {name}")),
            Choice::new(format!("Uninstall {name}"), move |ws, w, cx| {
                ws.pip(&format!("pip uninstall {n2}"), &["uninstall", "-y", &n2], w, cx)
            })
            .detail(format!("pip uninstall -y {name}")),
            Choice::new("← Back to packages", |ws, w, cx| ws.show_packages(w, cx)),
        ];
        self.choose(format!("{name} {version}"), choices, window, cx);
    }

    fn freeze_to_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(root), Some(python)) = (self.root.clone(), self.pyenv.current.as_ref().map(|i| i.path.clone()))
        else {
            return;
        };
        let has = root.join("requirements.txt").is_file();
        cx.spawn_in(window, async move |this, cx| {
            let text = match cx.background_executor().spawn(async move { freeze(&python) }).await {
                Ok(t) => t,
                Err(e) => {
                    this.update_in(cx, |ws, w, cx| ws.alert(format!("pip freeze failed:\n{e}"), w, cx)).ok();
                    return;
                }
            };
            let path: PathBuf = root.join("requirements.txt");
            let write = move |ws: &mut Workspace, w: &mut Window, cx: &mut Context<Workspace>| match std::fs::write(
                &path, &text,
            ) {
                Ok(()) => {
                    ws.file_tree.update(cx, |t, cx| t.refresh_all(cx));
                    ws.open_path(path.clone(), w, cx);
                    ws.refresh_python_env(cx);
                }
                Err(e) => ws.alert(format!("Couldn't write requirements.txt: {e}"), w, cx),
            };
            this.update_in(cx, |ws, window, cx| {
                if !has {
                    return write(ws, window, cx);
                }
                let answer = window.prompt(
                    PromptLevel::Warning,
                    "Replace requirements.txt with the packages installed now?",
                    None,
                    &["Replace", "Cancel"],
                    cx,
                );
                cx.spawn_in(window, async move |this, cx| {
                    if answer.await == Ok(0) {
                        this.update_in(cx, |ws, w, cx| write(ws, w, cx)).ok();
                    }
                })
                .detach();
            })
            .ok();
        })
        .detach();
    }
}
