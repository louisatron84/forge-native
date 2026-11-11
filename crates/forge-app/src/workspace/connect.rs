//! API-box persistence, preview and undoable generated source edits.
//! Replaces the controller in `src/renderer/boxes/controller.ts`.
use super::Workspace;
use crate::{palette::Choice, panels::connect::ConnectEvent};
use forge_core::{
    box_code::{block_hash, generate_block},
    box_sync::{block_state, find_block, insert_block, replace_block, Block, BlockState},
    boxes::{normalize_boxes, serialize_boxes, HTTP_METHODS},
};
use gpui_kit::{Context, PromptLevel, Task, Window};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Default)]
pub(super) struct ConnectState {
    pub(super) generation: u64,
    pub(super) revision: u64,
    pub(super) sync: super::connect_sync::SyncState,
    save: Option<Task<()>>,
    prepare: Option<Task<()>>,
    test: Option<Task<()>>,
    preview: Option<Preview>,
}
struct Preview {
    root: PathBuf,
    path: PathBuf,
    relative: String,
    id: String,
    block: String,
    original: String,
    disk_original: Option<String>,
    updated: String,
    remove: bool,
}
impl Workspace {
    pub(super) fn load_boxes(&mut self, root: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.reset_openapi(cx);
        let generation = self.connect_state.generation.wrapping_add(1);
        self.connect_state = ConnectState { generation, ..Default::default() };
        let (result, status) = match forge_sys::user_state::read_project_file(root, "boxes.json") {
            Ok((file, _)) => (normalize_boxes(&file.value()), file.error().unwrap_or_default().to_owned()),
            Err(e) => (Vec::new(), e.to_string()),
        };
        self.connect.update(cx, |p, cx| {
            p.set_boxes(result, true, window, cx);
            p.env_ignored = forge_run::env::gitignore_has_env(root);
            p.status = status;
        });
    }
    pub(super) fn save_boxes(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let metadata = serialize_boxes(&self.connect.read(cx).boxes);
        let generation = self.connect_state.generation;
        self.connect_state.save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(300)).await;
            let result = cx
                .background_executor()
                .spawn(async move {
                    let value = serde_json::to_value(metadata).map_err(std::io::Error::other)?;
                    forge_sys::user_state::write_project_file(&root, "boxes.json", &value)
                })
                .await;
            this.update(cx, |ws, cx| {
                if ws.connect_state.generation != generation {
                    return;
                }
                if let Err(error) = result {
                    ws.connect.update(cx, |p, cx| {
                        p.status = format!("Couldn't save API boxes: {error}");
                        cx.notify();
                    });
                }
            })
            .ok();
        }));
    }
    pub(super) fn on_connect_event(&mut self, event: &ConnectEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            ConnectEvent::IgnoreEnv => self.ignore_box_env(window, cx),
            ConnectEvent::OpenCsv => {
                if let Some((root, output)) = self.root.clone().zip(
                    self.connect
                        .read(cx)
                        .current()
                        .and_then(|b| b.requests.get(self.connect.read(cx).request))
                        .map(|r| r.output.clone()),
                ) {
                    match forge_sys::connect::project_path(&root, &output) {
                        Ok(path) if path.is_file() => {
                            self.open_path(path, window, cx);
                            self.reveal_panel("editor", cx);
                            self.focus_editor(window, cx);
                        }
                        Ok(_) => self.alert("The CSV hasn't been saved yet. Test the request first.", window, cx),
                        Err(e) => self.alert(e.to_string(), window, cx),
                    }
                }
            }
            ConnectEvent::RemoveRequest => {
                let Some(api) = self.connect.read(cx).current() else { return };
                if api.requests.len() <= 1 {
                    return;
                }
                let Some(request) = api.requests.get(self.connect.read(cx).request) else { return };
                let id = api.id.clone();
                let rid = request.id.clone();
                let generation = self.connect_state.generation;
                let revision = self.connect_state.revision;
                let answer=window.prompt(PromptLevel::Warning,&format!("Remove request {} from {}?",request.name,api.name),Some("Its generated function will be removed when its unchanged block synchronises. Existing CSV files stay in the project."),&["Remove request","Cancel"],cx);
                cx.spawn_in(window, async move |this, cx| {
                    if answer.await != Ok(0) {
                        return;
                    }
                    this.update_in(cx, |ws, window, cx| {
                        if ws.connect_state.generation != generation || ws.connect_state.revision != revision {
                            return;
                        }
                        ws.connect.update(cx, |p, cx| {
                            if let Some(api) = p.boxes.iter_mut().find(|b| b.id == id) {
                                if api.requests.len() > 1 {
                                    api.requests.retain(|r| r.id != rid);
                                    p.request = 0;
                                    p.refresh_form(window, cx);
                                    cx.emit(ConnectEvent::Changed);
                                    cx.notify();
                                }
                            }
                        });
                    })
                    .ok();
                })
                .detach();
            }
            ConnectEvent::OpenApi => {
                self.openapi.update(cx, |p, cx| {
                    p.open = true;
                    cx.notify();
                });
                self.reveal_panel("box", cx);
            }
            ConnectEvent::Add => {
                self.openapi.update(cx, |p, cx| {
                    p.open = false;
                    cx.notify();
                });
                if self.root.is_some() {
                    self.connect.update(cx, |p, cx| p.add(window, cx));
                    self.reveal_panel("box", cx);
                }
            }
            ConnectEvent::Selected => {
                self.openapi.update(cx, |p, cx| {
                    p.open = false;
                    cx.notify();
                });
                self.reveal_panel("box", cx)
            }
            ConnectEvent::Changed => {
                self.connect_state.revision = self.connect_state.revision.wrapping_add(1);
                self.save_boxes(cx);
                self.rebuild_graph(cx);
                self.queue_connect_sync(window, cx);
            }
            ConnectEvent::Preview => self.preview_box(false, false, window, cx),
            ConnectEvent::Apply => self.apply_box_preview(window, cx),
            ConnectEvent::PickMethod => {
                let choices = HTTP_METHODS
                    .into_iter()
                    .map(|method| {
                        Choice::new(method, move |ws, window, cx| {
                            ws.connect.update(cx, |p, cx| p.method(method, window, cx))
                        })
                    })
                    .collect();
                self.choose("HTTP method", choices, window, cx);
            }
            ConnectEvent::PickAuth => {
                let choices = [
                    ("None", "none"),
                    ("Bearer token", "bearer"),
                    ("API key in header", "header"),
                    ("API key in query", "query"),
                    ("Basic", "basic"),
                ]
                .into_iter()
                .map(|(label, kind)| {
                    Choice::new(label, move |ws, window, cx| ws.connect.update(cx, |p, cx| p.auth(kind, window, cx)))
                })
                .collect();
                self.choose("Authentication", choices, window, cx);
            }
            ConnectEvent::Env => {
                self.prepare_box_env(window, cx);
            }
            ConnectEvent::Remove => self.remove_api_box(window, cx),
            ConnectEvent::TestCall => self.test_api_request(window, cx),
        }
    }
    fn prepare_box_env(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        let Some(api) = self.connect.read(cx).current() else { return };
        let names = forge_core::box_code::auth_env(api);
        if names.iter().any(|name| {
            name.is_empty()
                || !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }) {
            return self.alert("Use valid environment-variable names (letters, digits and underscores).", window, cx);
        }
        let path = match forge_sys::connect::project_path(&root, ".env") {
            Ok(path) => path,
            Err(e) => return self.alert(e.to_string(), window, cx),
        };
        if !path.exists() {
            if let Err(e) = forge_sys::files::create_entry(&root, ".env", false) {
                return self.alert(e.to_string(), window, cx);
            }
        }
        let loaded = if let Some(id) = self.find_doc(&path.to_string_lossy(), cx) {
            Ok(Some(id))
        } else {
            self.load_doc(&path, false, cx)
        };
        let id = match loaded {
            Ok(Some(id)) => id,
            Ok(None) => return,
            Err(e) => return self.alert(e, window, cx),
        };
        let doc = self.docs[&id].clone();
        let dirty = doc.read(cx).doc.dirty();
        let original = doc.read(cx).doc.text();
        let known = forge_core::dotenv::parse_dotenv(&original);
        let mut updated = original.clone();
        for name in names {
            if !known.contains_key(&name) {
                updated = forge_core::dotenv::set_dotenv_var(&updated, &name, "");
            }
        }
        if updated != original {
            doc.update(cx, |d, cx| {
                d.edit(None, cx, |doc| {
                    let forge_text::Doc { buffer, history, .. } = doc;
                    history.transact(buffer, forge_text::EditKind::Other, forge_text::Pos::new(0, 0), |b| {
                        b.set_text(&updated);
                        forge_text::Pos::new(0, 0)
                    });
                })
            });
            if !dirty {
                self.save(Some(id), window, cx);
            }
        }
        self.open_path(path, window, cx);
        self.reveal_panel("editor", cx);
        self.focus_editor(window, cx);
    }
    fn ignore_box_env(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        let path = match forge_sys::connect::project_path(&root, ".gitignore") {
            Ok(p) => p,
            Err(e) => return self.alert(e.to_string(), window, cx),
        };
        if !path.exists() {
            if let Err(e) = forge_sys::files::create_entry(&root, ".gitignore", false) {
                return self.alert(e.to_string(), window, cx);
            }
        }
        let loaded = if let Some(id) = self.find_doc(&path.to_string_lossy(), cx) {
            Ok(Some(id))
        } else {
            self.load_doc(&path, false, cx)
        };
        let id = match loaded {
            Ok(Some(id)) => id,
            Ok(None) => return,
            Err(e) => return self.alert(e, window, cx),
        };
        let doc = self.docs[&id].clone();
        let dirty = doc.read(cx).doc.dirty();
        let text = doc.read(cx).doc.text();
        if !text.lines().any(|line| line.trim() == ".env") {
            let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
            let updated = format!("{text}{}.env{eol}", if text.is_empty() || text.ends_with('\n') { "" } else { eol });
            doc.update(cx, |d, cx| {
                d.edit(None, cx, |doc| {
                    let forge_text::Doc { buffer, history, .. } = doc;
                    history.transact(buffer, forge_text::EditKind::Other, forge_text::Pos::new(0, 0), |b| {
                        b.set_text(&updated);
                        forge_text::Pos::new(0, 0)
                    });
                })
            });
        }
        let saved = !dirty && self.save(Some(id), window, cx);
        self.connect.update(cx, |p, cx| {
            p.env_ignored = saved;
            p.status = if saved {
                "Added .env to .gitignore."
            } else {
                "Added .env to the unsaved .gitignore buffer. Save that file to finish."
            }
            .into();
            cx.notify();
        });
        if dirty {
            self.open_path(path, window, cx);
            self.reveal_panel("editor", cx);
            self.focus_editor(window, cx);
        }
        self.refresh_git(cx);
    }
    fn preview_box(&mut self, force: bool, remove: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let Some(api) = self.connect.read(cx).current().map(forge_core::boxes::ApiBox::metadata) else {
            return;
        };
        if api.detached && !remove {
            return self.alert(
                "This box keeps your code edits. Re-enable code updates before generating a replacement.",
                window,
                cx,
            );
        }
        let path = match forge_sys::connect::project_path(&root, &api.target.file) {
            Ok(p) => p,
            Err(e) => return self.alert(e.to_string(), window, cx),
        };
        let open = self
            .docs
            .values()
            .find(|d| forge_core::tabs::same_path(d.read(cx).doc.path(), &path.to_string_lossy()))
            .map(|d| d.read(cx).doc.text());
        let generation = self.connect_state.generation;
        let revision = self.connect_state.revision;
        self.connect_state.prepare = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let disk_original = if path.exists() {
                        match forge_sys::text_file::read_text_file(&path, false, None).map_err(|e| e.to_string())? {
                            forge_sys::text_file::TextFile::Text { text, .. } => Some(text),
                            _ => return Err("Choose a normal text Python file.".to_owned()),
                        }
                    } else {
                        None
                    };
                    let original = open.unwrap_or_else(|| disk_original.clone().unwrap_or_default());
                    let state = block_state(&original, &api);
                    if state == BlockState::Broken {
                        return Err(
                            "The generated-code markers are damaged. Repair them before updating this box.".into()
                        );
                    }
                    if state == BlockState::Edited && !force && !remove {
                        return Ok(Err((api.id.clone(), "The generated block was edited by hand.".to_owned())));
                    }
                    let block = generate_block(&api, api.target.mode == "new");
                    let updated = if remove {
                        if !matches!(find_block(&original, &api.id), Block::Found(_)) {
                            return Err(
                                "The generated block is missing. Choose Keep code to remove only the box.".into()
                            );
                        }
                        forge_core::box_sync::remove_block(&original, &api.id)
                    } else {
                        match find_block(&original, &api.id) {
                            Block::Found(_) => replace_block(&original, &api.id, &block)?,
                            Block::Missing => {
                                if api.target.mode == "new" && !original.is_empty() {
                                    return Err(
                                        "The target file already exists. Choose Append or a new Python file.".into()
                                    );
                                }
                                insert_block(&original, &block)
                            }
                            Block::Broken => return Err("Damaged block markers".into()),
                        }
                    };
                    Ok(Ok(Preview {
                        root,
                        path,
                        relative: api.target.file,
                        id: api.id,
                        block,
                        original,
                        disk_original,
                        updated,
                        remove,
                    }))
                })
                .await;
            this.update_in(cx, |ws, window, cx| {
                if ws.connect_state.generation != generation || ws.connect_state.revision != revision {
                    return;
                }
                match result {
                    Err(error) => ws.alert(error, window, cx),
                    Ok(Err((id, message))) => {
                        let answer = window.prompt(
                            PromptLevel::Warning,
                            &message,
                            Some("Your edits stay untouched until you review and apply a replacement."),
                            &["Keep my edits", "Preview replacement", "Cancel"],
                            cx,
                        );
                        cx.spawn_in(window, async move |this, cx| {
                            let choice = answer.await;
                            this.update_in(cx, |ws, window, cx| {
                                if ws.connect_state.generation != generation
                                    || ws.connect_state.revision != revision
                                    || ws.connect.read(cx).current().is_none_or(|b| b.id != id)
                                {
                                    return;
                                }
                                match choice {
                                    Ok(0) => {
                                        ws.connect.update(cx, |p, cx| {
                                            if let Some(i) = p.selected {
                                                p.boxes[i].detached = true;
                                                cx.notify();
                                            }
                                        });
                                        ws.save_boxes(cx);
                                    }
                                    Ok(1) => ws.preview_box(true, false, window, cx),
                                    _ => {}
                                }
                            })
                            .ok();
                        })
                        .detach();
                    }
                    Ok(Ok(preview)) => {
                        let title = format!("Generated Python — {}", preview.relative);
                        let lines = preview.updated.lines().map(str::to_owned).collect();
                        let label = if preview.remove { "Remove generated code" } else { "Apply generated code" };
                        ws.connect_state.preview = Some(preview);
                        ws.git.diff = None;
                        ws.git_inspect.update(cx, |p, cx| {
                            p.text(title, lines, cx);
                            p.action = Some(("connect.apply", label));
                        });
                        ws.reveal_panel("editor", cx);
                        cx.notify();
                    }
                }
            })
            .ok();
        }));
    }
    pub(super) fn apply_box_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(preview) = self.connect_state.preview.take() else {
            return;
        };
        if self.root.as_ref() != Some(&preview.root) {
            return;
        }
        let Some(api) = self.connect.read(cx).boxes.iter().find(|b| b.id == preview.id) else {
            return;
        };
        if api.target.file != preview.relative
            || (!preview.remove && (api.detached || generate_block(api, api.target.mode == "new") != preview.block))
        {
            return self.alert("The box changed. Preview its code again before applying.", window, cx);
        }
        let path = match forge_sys::connect::project_path(&preview.root, &preview.relative) {
            Ok(p) => p,
            Err(e) => return self.alert(e.to_string(), window, cx),
        };
        if path != preview.path {
            return self.alert("The target moved. Preview again.", window, cx);
        }
        let disk_current = if path.exists() {
            match forge_sys::text_file::read_text_file(&path, false, None) {
                Ok(forge_sys::text_file::TextFile::Text { text, .. }) => Some(text),
                _ => return self.alert("The target cannot be read. Preview again.", window, cx),
            }
        } else {
            None
        };
        if disk_current != preview.disk_original {
            return self.alert(
                "The file changed on disk after the preview. Preview again to preserve those edits.",
                window,
                cx,
            );
        }
        let current = self
            .docs
            .values()
            .find(|d| forge_core::tabs::same_path(d.read(cx).doc.path(), &path.to_string_lossy()))
            .map(|d| d.read(cx).doc.text());
        let current = match current {
            Some(s) => s,
            None => disk_current.unwrap_or_default(),
        };
        if current != preview.original {
            return self.alert(
                "The source changed after the preview. Preview again to preserve those edits.",
                window,
                cx,
            );
        }
        if !path.exists() {
            if let Err(e) = forge_sys::files::create_entry(&preview.root, &preview.relative, false) {
                return self.alert(e.to_string(), window, cx);
            }
        }
        let loaded = if let Some(id) = self.find_doc(&path.to_string_lossy(), cx) {
            Ok(Some(id))
        } else {
            self.load_doc(&path, false, cx)
        };
        let id = match loaded {
            Ok(Some(id)) => id,
            Ok(None) => return,
            Err(e) => return self.alert(e, window, cx),
        };
        let doc = self.docs[&id].clone();
        let was_dirty = doc.read(cx).doc.dirty();
        doc.update(cx, |d, cx| {
            d.edit(None, cx, |doc| {
                let forge_text::Doc { buffer, history, .. } = doc;
                history.transact(buffer, forge_text::EditKind::Other, forge_text::Pos::new(0, 0), |b| {
                    b.set_text(&preview.updated);
                    forge_text::Pos::new(0, 0)
                });
            })
        });
        self.open_path(path, window, cx);
        if !was_dirty {
            self.save(Some(id), window, cx);
        }
        let actual = doc.read(cx).doc.text();
        let hash = match find_block(&actual, &preview.id) {
            Block::Found(found) => block_hash(&found.text),
            _ => block_hash(&preview.block),
        };
        self.connect.update(cx, |p, cx| {
            if preview.remove {
                p.boxes.retain(|b| b.id != preview.id);
                p.selected =
                    if p.boxes.is_empty() { None } else { Some(p.selected.unwrap_or(0).min(p.boxes.len() - 1)) };
                p.request = 0;
                p.refresh_form(window, cx);
                p.status = "Removed the box and its generated block. The source edit can be undone.".into();
                cx.notify();
                return;
            }
            if let Some(b) = p.boxes.iter_mut().find(|b| b.id == preview.id) {
                b.prev_hash = b.hash.clone();
                b.hash = Some(hash);
                b.attach =
                    Some(forge_core::boxes::BoxAttach { module: forge_core::boxes::module_of_path(&preview.relative) });
            }
            p.status = "Generated code applied. Edits can be undone in the editor.".into();
            cx.notify();
        });
        self.save_boxes(cx);
        self.rebuild_graph(cx);
        self.file_tree.update(cx, |t, cx| t.refresh_all(cx));
        self.queue_graph_scan(None, cx);
        self.focus_editor(window, cx);
    }
    fn remove_api_box(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(api) = self.connect.read(cx).current() else { return };
        let id = api.id.clone();
        let generation = self.connect_state.generation;
        let answer = window.prompt(
            PromptLevel::Warning,
            "Remove this API box?",
            Some("Keep its Python code, or preview removing the marked generated block. Other source stays untouched."),
            &["Keep code", "Preview code removal", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let choice = answer.await;
            this.update_in(cx, |ws, window, cx| {
                if ws.connect_state.generation != generation || ws.connect.read(cx).current().is_none_or(|b| b.id != id)
                {
                    return;
                }
                match choice {
                    Ok(0) => {
                        ws.connect.update(cx, |p, cx| {
                            p.boxes.retain(|b| b.id != id);
                            p.selected = if p.boxes.is_empty() { None } else { Some(0) };
                            p.request = 0;
                            p.refresh_form(window, cx);
                            cx.notify();
                        });
                        ws.save_boxes(cx);
                        ws.rebuild_graph(cx);
                    }
                    Ok(1) => ws.preview_box(false, true, window, cx),
                    _ => {}
                }
            })
            .ok();
        })
        .detach();
    }
    fn test_api_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connect.read(cx).testing {
            return;
        }
        let Some(root) = self.root.clone() else { return };
        let Some(api) = self.connect.read(cx).current().map(forge_core::boxes::ApiBox::metadata) else { return };
        let Some(request) = api.requests.get(self.connect.read(cx).request) else { return };
        let request_id = request.id.clone();
        let testing_id = api.id.clone();
        self.project_graph.update(cx, |p, cx| p.box_testing(&testing_id, true, cx));
        let generation = self.connect_state.generation;
        let revision = self.connect_state.revision;
        self.connect.update(cx, |p, cx| {
            p.testing = true;
            p.status = "Calling the API…".into();
            cx.notify();
        });
        self.connect_state.test = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let result = forge_sys::connect::test_call(&root, &api, &request_id);
                    (api.id, request_id, result)
                })
                .await;
            this.update_in(cx, |ws, window, cx| {
                if ws.connect_state.generation != generation {
                    return;
                }
                ws.project_graph.update(cx, |p, cx| p.box_testing(&testing_id, false, cx));
                if ws.connect_state.revision != revision {
                    ws.connect.update(cx, |p, cx| {
                        p.testing = false;
                        p.status = "Request settings changed. Test again to use the new values.".into();
                        cx.notify();
                    });
                    return;
                }
                ws.connect.update(cx, |p, cx| p.test_result(&result.0, &result.1, result.2, cx));
                ws.save_boxes(cx);
                ws.rebuild_graph(cx);
                ws.queue_connect_sync(window, cx);
            })
            .ok();
        }));
    }
}
