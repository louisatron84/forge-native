//! OpenAPI loading and reviewable client/import/environment edits.
//! Replaces the OpenAPI controller in `renderer/main.ts`.
use super::{
    generated::{ChangeReview, TextChange},
    Workspace,
};
use crate::panels::openapi::OpenApiEvent;
use forge_core::{
    openapi::{env_keys, read_spec},
    openapi_code::{add_import, generate_client, generated_state, import_line},
};
use gpui_kit::{Context, Task, Window};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
};
#[derive(Default)]
pub(super) struct OpenApiState {
    generation: u64,
    worker: Option<Task<()>>,
}
struct SourceEdit {
    relative: String,
    change: TextChange,
}
struct ImportPreview {
    root: PathBuf,
    serial: u64,
    chosen: HashSet<String>,
    target: String,
    import_current: bool,
    edits: Vec<SourceEdit>,
}
fn source_edit(root: &Path, relative: &str, open: &HashMap<String, (String, bool)>) -> Result<SourceEdit, String> {
    let path = forge_sys::connect::project_path(root, relative).map_err(|e| e.to_string())?;
    let open = open
        .iter()
        .find(|(p, _)| forge_core::tabs::same_path(p, &path.to_string_lossy()))
        .map(|(_, text)| text.clone());
    Ok(SourceEdit { relative: relative.into(), change: TextChange::read(path, open)? })
}
fn prepare_import(
    mut plan: ImportPreview,
    spec: &forge_core::openapi::ApiSpec,
    import_into: Option<String>,
    open: &HashMap<String, (String, bool)>,
) -> Result<(ImportPreview, bool), String> {
    let mut client = source_edit(&plan.root, &plan.target, open)?;
    let replacement = !client.change.original.is_empty()
        && generated_state(&client.change.original).is_none_or(|s| s.edited || s.slug != spec.slug);
    client.change.updated = generate_client(spec, Some(&plan.chosen));
    plan.edits.push(client);
    if let Some(relative) = import_into {
        let mut edit = source_edit(&plan.root, &relative, open)?;
        let names =
            spec.operations.iter().filter(|o| plan.chosen.contains(&o.id)).map(|o| o.id.clone()).collect::<Vec<_>>();
        let line = if names.len() <= 6 {
            import_line(&plan.target, &names)
        } else {
            format!("import {} as {}", plan.target.trim_end_matches(".py").replace('/', "."), spec.slug)
        };
        edit.change.updated = add_import(&edit.change.original, &line);
        if edit.change.updated != edit.change.original {
            plan.edits.push(edit)
        }
    }
    let mut env = source_edit(&plan.root, ".env", open)?;
    let known = forge_core::dotenv::parse_dotenv(&env.change.original);
    let names = env_keys(spec);
    let missing = names.iter().filter(|n| !known.contains_key(*n)).collect::<Vec<_>>();
    for name in &missing {
        env.change.updated = forge_core::dotenv::set_dotenv_var(
            &env.change.updated,
            name,
            if name.ends_with("_BASE_URL") { &spec.base_url } else { "" },
        );
    }
    if !missing.is_empty() {
        plan.edits.push(env)
    }
    Ok((plan, replacement))
}
impl Workspace {
    pub(super) fn reset_openapi(&mut self, cx: &mut Context<Self>) {
        let generation = self.openapi_state.generation.wrapping_add(1);
        self.openapi_state = OpenApiState { generation, ..Default::default() };
        self.openapi.update(cx, |p, cx| {
            p.open = false;
            p.spec = None;
            p.chosen.clear();
            p.status.clear();
            p.serial = p.serial.wrapping_add(1);
            cx.notify();
        });
    }
    pub(super) fn on_openapi_event(&mut self, event: &OpenApiEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            OpenApiEvent::Close => {
                self.openapi.update(cx, |p, cx| {
                    p.open = false;
                    cx.notify();
                });
                cx.notify();
            }
            OpenApiEvent::Load(source) => self.load_openapi(source.clone(), window, cx),
            OpenApiEvent::Preview => self.preview_openapi(window, cx),
            OpenApiEvent::Pick => {
                let picked = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: Some("OpenAPI description".into()),
                });
                let generation = self.openapi_state.generation;
                cx.spawn_in(window, async move |this, cx| {
                    if let Ok(Ok(Some(paths))) = picked.await {
                        if let Some(path) = paths.first() {
                            this.update_in(cx, |ws, window, cx| {
                                if ws.openapi_state.generation == generation {
                                    ws.load_openapi(path.to_string_lossy().into(), window, cx)
                                }
                            })
                            .ok();
                        }
                    }
                })
                .detach();
            }
        }
    }
    fn load_openapi(&mut self, source: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return self.alert("Open a project folder first.", window, cx) };
        self.openapi_state.generation = self.openapi_state.generation.wrapping_add(1);
        let generation = self.openapi_state.generation;
        self.openapi.update(cx, |p, cx| {
            p.open = true;
            p.spec = None;
            p.status = "Loading description…".into();
            p.source.update(cx, |s, cx| s.set_value(source.clone(), window, cx));
            cx.notify();
        });
        self.reveal_panel("box", cx);
        self.openapi_state.worker = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let source = source.trim();
                    let source = if source.is_empty() || source.contains("://") || Path::new(source).is_absolute() {
                        source.to_owned()
                    } else {
                        root.join(source).to_string_lossy().into()
                    };
                    let loaded = forge_sys::openapi::load_spec(&source)?;
                    read_spec(&loaded.doc, &loaded.from)
                })
                .await;
            this.update_in(cx, |ws, window, cx| {
                if ws.openapi_state.generation != generation {
                    return;
                }
                ws.openapi.update(cx, |p, cx| match result {
                    Ok(spec) => p.loaded(spec, window, cx),
                    Err(e) => {
                        p.status = e;
                        cx.notify();
                    }
                });
                cx.notify();
            })
            .ok();
        }));
    }
    fn preview_openapi(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        let panel = self.openapi.read(cx);
        let Some(spec) = panel.spec.clone() else { return };
        let chosen = panel.chosen.clone();
        let target = panel.target.read(cx).value().trim().replace('\\', "/");
        let serial = panel.serial;
        let import_current = panel.import_current;
        if chosen.is_empty() {
            return self.alert("Tick at least one endpoint.", window, cx);
        }
        if !target.ends_with(".py") {
            return self.alert("The client file must be a .py path inside the project.", window, cx);
        }
        let import_into = if import_current {
            self.active_doc()
                .and_then(|d| {
                    let path = PathBuf::from(d.read(cx).doc.path());
                    path.strip_prefix(&root)
                        .ok()
                        .filter(|r| r.extension().is_some_and(|s| s.eq_ignore_ascii_case("py")))
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                })
                .filter(|p| p != &target)
        } else {
            None
        };
        let wanted = [Some(target.clone()), Some(".env".into()), import_into.clone()]
            .into_iter()
            .flatten()
            .map(|rel| root.join(rel).to_string_lossy().to_string())
            .collect::<Vec<_>>();
        let open = self
            .docs
            .values()
            .filter_map(|d| {
                let d = d.read(cx);
                wanted
                    .iter()
                    .any(|path| forge_core::tabs::same_path(path, d.doc.path()))
                    .then(|| (d.doc.path().to_owned(), (d.doc.text(), d.doc.dirty())))
            })
            .collect::<HashMap<_, _>>();
        let generation = self.openapi_state.generation;
        self.openapi_state.worker = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    prepare_import(
                        ImportPreview { root, serial, chosen, target, import_current, edits: Vec::new() },
                        &spec,
                        import_into,
                        &open,
                    )
                })
                .await;
            this.update_in(cx, |ws, window, cx| {
                if ws.openapi_state.generation != generation {
                    return;
                }
                match result {
                    Err(e) => ws.alert(e, window, cx),
                    Ok((plan, replacement)) => ws.review_openapi_plan(plan, replacement, cx),
                }
            })
            .ok();
        }));
    }

    fn review_openapi_plan(&mut self, plan: ImportPreview, replacement: bool, cx: &mut Context<Self>) {
        let plan = Rc::new(plan);
        let checked = Rc::new(Cell::new(None));
        let client_written = Rc::new(Cell::new(false));
        for (index, edit) in plan.edits.iter().enumerate() {
            let first = index == 0;
            let last = index + 1 == plan.edits.len();
            let (before, after) = edit.change.diff();
            let label = if first && replacement { "Replace edited client" } else { edit.change.label(self, cx) };
            let (plan, checked, client_written) = (plan.clone(), checked.clone(), client_written.clone());
            self.review_change(
                ChangeReview::new(
                    format!("OpenAPI client: {}", edit.relative),
                    before,
                    after,
                    label,
                    "Reject",
                    move |apply, ws, window, cx| {
                        if apply {
                            if let Err(error) = ws.check_openapi_plan(&plan, cx) {
                                return ws.alert(error, window, cx);
                            }
                            if checked.get().is_none() {
                                let result =
                                    plan.edits.iter().try_for_each(|edit| edit.change.check(ws, cx).map(|_| ()));
                                checked.set(Some(result.is_ok()));
                                if let Err(error) = result {
                                    return ws.alert(error, window, cx);
                                }
                            }
                            if checked.get() == Some(false) {
                                return ws.alert("A file changed. Preview the OpenAPI client again.", window, cx);
                            }
                            let edit = &plan.edits[index];
                            if forge_sys::connect::project_path(&plan.root, &edit.relative).ok().as_ref()
                                != Some(&edit.change.path)
                            {
                                return ws.alert("A target moved. Preview the OpenAPI client again.", window, cx);
                            }
                            let applied = ws.apply_reviewed_text(edit.change.clone(), "OpenAPI client", window, cx);
                            if let Err(error) = applied {
                                return ws.alert(error, window, cx);
                            }
                            if first {
                                client_written.set(true);
                            }
                            ws.file_tree.update(cx, |t, cx| t.refresh_all(cx));
                        }
                        if last && client_written.get() {
                            ws.reveal_panel("editor", cx);
                            ws.open_path(plan.root.join(&plan.target), window, cx);
                        }
                    },
                ),
                cx,
            );
        }
    }

    fn check_openapi_plan(&self, plan: &ImportPreview, cx: &gpui_kit::App) -> Result<(), String> {
        let panel = self.openapi.read(cx);
        if self.root.as_ref() != Some(&plan.root)
            || panel.serial != plan.serial
            || panel.chosen != plan.chosen
            || panel.import_current != plan.import_current
            || panel.target.read(cx).value().trim().replace('\\', "/") != plan.target
        {
            return Err("The project, endpoints or client settings changed. Preview the client again.".into());
        }
        Ok(())
    }
}
