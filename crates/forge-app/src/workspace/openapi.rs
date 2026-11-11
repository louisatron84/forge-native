//! OpenAPI loading and reviewable client/import/environment edits.
//! Replaces the OpenAPI controller in `renderer/main.ts`.
use super::Workspace;
use crate::panels::openapi::OpenApiEvent;
use forge_core::{
    openapi::{env_keys, read_spec},
    openapi_code::{add_import, generate_client, generated_state, import_line},
};
use gpui_kit::{Context, Task, Window};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};
#[derive(Default)]
pub(super) struct OpenApiState {
    generation: u64,
    worker: Option<Task<()>>,
    preview: Option<ImportPreview>,
}
struct SourceEdit {
    relative: String,
    path: PathBuf,
    original: String,
    disk: Option<String>,
    updated: String,
}
struct ImportPreview {
    root: PathBuf,
    serial: u64,
    chosen: HashSet<String>,
    target: String,
    import_current: bool,
    edits: Vec<SourceEdit>,
}
fn disk_text(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    match forge_sys::text_file::read_text_file(path, false, None).map_err(|e| e.to_string())? {
        forge_sys::text_file::TextFile::Text { text, .. } => Ok(Some(text)),
        _ => Err("Choose a normal text file.".into()),
    }
}
fn source_edit(root: &Path, relative: &str, open: &HashMap<String, String>) -> Result<SourceEdit, String> {
    let path = forge_sys::connect::project_path(root, relative).map_err(|e| e.to_string())?;
    let disk = disk_text(&path)?;
    let original = open
        .iter()
        .find(|(p, _)| forge_core::tabs::same_path(p, &path.to_string_lossy()))
        .map(|(_, text)| text.clone())
        .unwrap_or_else(|| disk.clone().unwrap_or_default());
    Ok(SourceEdit { relative: relative.into(), path, original: original.clone(), disk, updated: original })
}
fn prepare_import(
    mut plan: ImportPreview,
    spec: &forge_core::openapi::ApiSpec,
    import_into: Option<String>,
    open: &HashMap<String, String>,
) -> Result<(ImportPreview, Vec<String>, bool), String> {
    let mut client = source_edit(&plan.root, &plan.target, open)?;
    let replacement = !client.original.is_empty()
        && generated_state(&client.original).is_none_or(|s| s.edited || s.slug != spec.slug);
    client.updated = generate_client(spec, Some(&plan.chosen));
    let mut preview = Vec::new();
    if replacement {
        preview.push(format!(
            "WARNING: {} has hand edits or is not this generated client. Apply will replace the entire file.",
            plan.target
        ));
    }
    preview.push(format!("Client file: {}", plan.target));
    preview.extend(client.updated.lines().map(str::to_owned));
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
        edit.updated = add_import(&edit.original, &line);
        preview.push(format!("Import into {relative}: {line}"));
        if edit.updated != edit.original {
            plan.edits.push(edit)
        }
    }
    let mut env = source_edit(&plan.root, ".env", open)?;
    let known = forge_core::dotenv::parse_dotenv(&env.original);
    let names = env_keys(spec);
    let missing = names.iter().filter(|n| !known.contains_key(*n)).collect::<Vec<_>>();
    for name in &missing {
        env.updated = forge_core::dotenv::set_dotenv_var(
            &env.updated,
            name,
            if name.ends_with("_BASE_URL") { &spec.base_url } else { "" },
        );
    }
    if !missing.is_empty() {
        preview.push(format!(
            "Add .env names (server URL and empty credentials): {}",
            missing.into_iter().cloned().collect::<Vec<_>>().join(", ")
        ));
        plan.edits.push(env)
    }
    Ok((plan, preview, replacement))
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
        self.openapi_state.preview = None;
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
                    .then(|| (d.doc.path().to_owned(), d.doc.text()))
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
                    Ok((plan, lines, replacement)) => {
                        ws.openapi_state.preview = Some(plan);
                        ws.git.diff = None;
                        ws.git_inspect.update(cx, |p, cx| {
                            p.text("OpenAPI client preview".into(), lines, cx);
                            p.action = Some((
                                "openapi.apply",
                                if replacement { "Replace edited client and apply" } else { "Apply OpenAPI client" },
                            ));
                        });
                        ws.reveal_panel("editor", cx);
                        cx.notify();
                    }
                }
            })
            .ok();
        }));
    }
    pub(super) fn apply_openapi_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(plan) = self.openapi_state.preview.take() else { return };
        if self.root.as_ref() != Some(&plan.root) {
            return;
        }
        let panel = self.openapi.read(cx);
        if panel.serial != plan.serial
            || panel.chosen != plan.chosen
            || panel.import_current != plan.import_current
            || panel.target.read(cx).value().trim().replace('\\', "/") != plan.target
        {
            return self.alert("The endpoint selection or client settings changed. Preview again.", window, cx);
        }
        for edit in &plan.edits {
            if forge_sys::connect::project_path(&plan.root, &edit.relative).ok().as_ref() != Some(&edit.path)
                || disk_text(&edit.path).ok().as_ref() != Some(&edit.disk)
            {
                return self.alert(
                    "A target file changed after preview. Preview again to preserve those edits.",
                    window,
                    cx,
                );
            }
            let text = self
                .find_doc(&edit.path.to_string_lossy(), cx)
                .and_then(|id| self.docs.get(&id))
                .map(|d| d.read(cx).doc.text())
                .unwrap_or_else(|| edit.disk.clone().unwrap_or_default());
            if text != edit.original {
                return self.alert(
                    "A target buffer changed after preview. Preview again to preserve those edits.",
                    window,
                    cx,
                );
            }
        }
        for edit in plan.edits {
            if !edit.path.exists() {
                if let Err(e) = forge_sys::files::create_entry(&plan.root, &edit.relative, false) {
                    return self.alert(e.to_string(), window, cx);
                }
            }
            let loaded = if let Some(id) = self.find_doc(&edit.path.to_string_lossy(), cx) {
                Ok(Some(id))
            } else {
                self.load_doc(&edit.path, false, cx)
            };
            let id = match loaded {
                Ok(Some(id)) => id,
                Ok(None) => return,
                Err(e) => return self.alert(e, window, cx),
            };
            let doc = self.docs[&id].clone();
            let dirty = doc.read(cx).doc.dirty();
            doc.update(cx, |d, cx| {
                d.edit(None, cx, |doc| {
                    let forge_text::Doc { buffer, history, .. } = doc;
                    history.transact(buffer, forge_text::EditKind::Other, forge_text::Pos::new(0, 0), |b| {
                        b.set_text(&edit.updated);
                        forge_text::Pos::new(0, 0)
                    });
                })
            });
            if !dirty && !self.save(Some(id), window, cx) {
                return;
            }
        }
        self.openapi.update(cx, |p, cx| {
            p.status = "Applied client, imports and missing environment names. Source edits can be undone.".into();
            cx.notify();
        });
        self.openapi_state.preview = None;
        self.file_tree.update(cx, |t, cx| t.refresh_all(cx));
        self.queue_graph_scan(None, cx);
        self.open_path(plan.root.join(plan.target), window, cx);
        self.focus_editor(window, cx);
    }
}
