//! Debounced generated-block and full CSV synchronisation, replacing
//! `boxes/controller.ts::syncCode/saveCsv`. All CSV construction runs on a worker.
use super::Workspace;
use forge_core::{
    api_fields::csv_from_sample,
    box_code::{block_hash, generate_block},
    box_sync::{block_state, replace_block, BlockState},
    boxes::ApiBox,
};
use gpui_kit::{Context, Task, Window};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
#[derive(Default)]
pub(super) struct SyncState {
    task: Option<Task<()>>,
    saved: HashMap<(String, String), (String, u64)>,
}
struct CsvInput {
    id: String,
    output: String,
    path: Option<Vec<String>>,
    ticked: Vec<String>,
    sample: Arc<serde_json::Value>,
}
struct Snapshot {
    api: ApiBox,
    csv: Vec<CsvInput>,
}
struct Change {
    path: PathBuf,
    relative: String,
    original: String,
    disk: Option<String>,
    updated: String,
    kind: ChangeKind,
}
enum ChangeKind {
    Code { id: String, hash: String },
    Csv { box_id: String, id: String, stamp: u64, count: usize },
}
fn read(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    match forge_sys::text_file::read_text_file(path, false, None).map_err(|e| e.to_string())? {
        forge_sys::text_file::TextFile::Text { text, .. } => Ok(Some(text)),
        _ => Err("The target isn't a normal text file.".into()),
    }
}
fn change(
    root: &Path,
    relative: &str,
    updated: String,
    kind: ChangeKind,
    open: &HashMap<String, String>,
) -> Result<Change, String> {
    let path = forge_sys::connect::project_path(root, relative).map_err(|e| e.to_string())?;
    let disk = read(&path)?;
    let original = open
        .iter()
        .find(|(p, _)| forge_core::tabs::same_path(p, &path.to_string_lossy()))
        .map(|(_, text)| text.clone())
        .unwrap_or_else(|| disk.clone().unwrap_or_default());
    Ok(Change { path, relative: relative.into(), original, disk, updated, kind })
}
fn prepare(
    root: &Path,
    snapshots: Vec<Snapshot>,
    open: HashMap<String, String>,
    saved: HashMap<(String, String), (String, u64)>,
) -> (Vec<Change>, Vec<String>) {
    let mut changes = Vec::new();
    let mut notes = Vec::new();
    let python_targets =
        snapshots.iter().map(|s| root.join(&s.api.target.file).to_string_lossy().to_string()).collect::<Vec<_>>();
    for Snapshot { api, csv } in snapshots {
        if api.hash.is_some() && !api.detached {
            let generated = generate_block(&api, api.target.mode == "new");
            let hash = block_hash(&generated);
            if api.hash.as_ref() != Some(&hash) {
                match change(
                    root,
                    &api.target.file,
                    String::new(),
                    ChangeKind::Code { id: api.id.clone(), hash },
                    &open,
                ) {
                    Ok(mut next) => {
                        if next.disk.is_some() && block_state(&next.original, &api) == BlockState::Ok {
                            match replace_block(&next.original, &api.id, &generated) {
                                Ok(updated) => {
                                    next.updated = updated;
                                    changes.push(next)
                                }
                                Err(e) => notes.push(e),
                            }
                        } else {
                            notes.push(format!("Kept the missing or hand-edited block in {}. Use Preview code to review a replacement.",api.target.file));
                        }
                    }
                    Err(e) => notes.push(format!("Couldn't update {}: {e}", api.target.file)),
                }
            }
        }
        for input in csv {
            let key = (api.id.clone(), input.id.clone());
            if python_targets
                .iter()
                .any(|p| forge_core::tabs::same_path(p, &root.join(&input.output).to_string_lossy()))
            {
                notes.push("CSV output must differ from generated Python files.".into());
                continue;
            }
            if !input.output.to_ascii_lowercase().ends_with(".csv") {
                notes.push("Choose a .csv output file inside the project.".into());
                continue;
            }
            let (text, count) = csv_from_sample(&input.sample, input.path.as_deref(), &input.ticked, usize::MAX);
            let mut hasher = std::hash::DefaultHasher::new();
            text.hash(&mut hasher);
            let stamp = hasher.finish();
            if saved.get(&key) == Some(&(input.output.clone(), stamp)) {
                continue;
            }
            let kind = ChangeKind::Csv { box_id: api.id.clone(), id: input.id, stamp, count };
            match change(root, &input.output, text, kind, &open) {
                Ok(next) => changes.push(next),
                Err(e) => notes.push(format!("Couldn't save {}: {e}", input.output)),
            }
        }
    }
    (changes, notes)
}
impl Workspace {
    pub(super) fn queue_connect_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.root.is_none() {
            return;
        }
        self.connect_state.sync.task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(300)).await;
            this.update_in(cx, |ws, window, cx| ws.start_connect_sync(window, cx)).ok();
        }));
    }
    fn start_connect_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        let generation = self.connect_state.generation;
        let revision = self.connect_state.revision;
        let snapshots = self
            .connect
            .read(cx)
            .boxes
            .iter()
            .map(|api| Snapshot {
                api: api.metadata(),
                csv: api
                    .requests
                    .iter()
                    .filter_map(|r| {
                        let test = r.test.as_ref().filter(|t| t.ok)?;
                        if r.ticked.is_empty() || r.output.is_empty() {
                            return None;
                        }
                        Some(CsvInput {
                            id: r.id.clone(),
                            output: r.output.clone(),
                            path: r.records_path.clone(),
                            ticked: r.ticked.clone(),
                            sample: test.sample.clone()?,
                        })
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();
        let wanted = snapshots
            .iter()
            .flat_map(|s| std::iter::once(s.api.target.file.as_str()).chain(s.csv.iter().map(|c| c.output.as_str())))
            .map(|p| root.join(p).to_string_lossy().to_string())
            .collect::<Vec<_>>();
        let open = self
            .docs
            .values()
            .filter_map(|d| {
                let d = d.read(cx);
                wanted
                    .iter()
                    .any(|p| forge_core::tabs::same_path(p, d.doc.path()))
                    .then(|| (d.doc.path().to_owned(), d.doc.text()))
            })
            .collect();
        let saved = self.connect_state.sync.saved.clone();
        let worker_root = root.clone();
        self.connect_state.sync.task = Some(cx.spawn_in(window, async move |this, cx| {
            let (changes, mut notes) =
                cx.background_executor().spawn(async move { prepare(&worker_root, snapshots, open, saved) }).await;
            this.update_in(cx, |ws, window, cx| {
                if ws.root.as_ref() != Some(&root)
                    || ws.connect_state.generation != generation
                    || ws.connect_state.revision != revision
                {
                    return;
                }
                let mut code = false;
                for change in changes {
                    match ws.apply_connect_change(&root, change, window, cx) {
                        Ok((note, is_code)) => {
                            notes.push(note);
                            code |= is_code
                        }
                        Err(e) => notes.push(e),
                    }
                }
                if code {
                    ws.save_boxes(cx);
                    ws.rebuild_graph(cx);
                    ws.queue_graph_scan(None, cx)
                }
                if !notes.is_empty() {
                    ws.connect.update(cx, |p, cx| {
                        p.status = notes.join("\n");
                        cx.notify();
                    });
                    ws.file_tree.update(cx, |t, cx| t.refresh_all(cx));
                }
            })
            .ok();
        }));
    }
    fn apply_connect_change(
        &mut self,
        root: &Path,
        change: Change,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(String, bool), String> {
        if forge_sys::connect::project_path(root, &change.relative).map_err(|e| e.to_string())? != change.path
            || read(&change.path)? != change.disk
        {
            return Err(format!("{} changed while synchronising; preview again before replacing it.", change.relative));
        }
        let existing = self.find_doc(&change.path.to_string_lossy(), cx);
        let current = existing
            .and_then(|id| self.docs.get(&id))
            .map(|d| d.read(cx).doc.text())
            .unwrap_or_else(|| change.disk.clone().unwrap_or_default());
        if current != change.original {
            return Err(format!("{} has newer edits; preview again before replacing them.", change.relative));
        }
        if !change.path.exists() {
            forge_sys::files::create_entry(root, &change.relative, false).map_err(|e| e.to_string())?;
        }
        let id = if let Some(id) = existing {
            id
        } else {
            self.load_doc(&change.path, false, cx)?.ok_or("Couldn't open the sync target.")?
        };
        let doc = self.docs[&id].clone();
        let dirty = doc.read(cx).doc.dirty();
        let is_csv = matches!(change.kind, ChangeKind::Csv { .. });
        doc.update(cx, |d, cx| {
            d.edit(None, cx, |doc| {
                if is_csv {
                    doc.set_eol(forge_text::Eol::CrLf)
                }
                let forge_text::Doc { buffer, history, .. } = doc;
                history.transact(buffer, forge_text::EditKind::Other, forge_text::Pos::new(0, 0), |b| {
                    b.set_text(&change.updated);
                    forge_text::Pos::new(0, 0)
                });
            })
        });
        if !dirty && !self.save(Some(id), window, cx) {
            return Err(format!("Couldn't save {}.", change.relative));
        }
        let suffix = if dirty { " (unsaved: the file already had other edits)" } else { "" };
        match change.kind {
            ChangeKind::Code { id, hash } => {
                self.connect.update(cx, |p, cx| {
                    if let Some(b) = p.boxes.iter_mut().find(|b| b.id == id) {
                        b.prev_hash = b.hash.take();
                        b.hash = Some(hash);
                    }
                    cx.notify();
                });
                Ok((format!("Updated {} from its API box{suffix}", change.relative), true))
            }
            ChangeKind::Csv { box_id, id, stamp, count } => {
                self.connect_state.sync.saved.insert((box_id, id), (change.relative.clone(), stamp));
                Ok((format!("Saved {count} rows to {}{suffix}", change.relative), false))
            }
        }
    }
}
