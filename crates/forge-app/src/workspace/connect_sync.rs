//! Debounced generated-block and full CSV synchronisation, replacing
//! `boxes/controller.ts::syncCode/saveCsv`. All CSV construction runs on a worker.
use super::{
    generated::{ChangeReview, TextChange},
    Workspace,
};
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
    path::Path,
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
    relative: String,
    text: TextChange,
    kind: ChangeKind,
}
enum ChangeKind {
    Code { id: String, hash: String },
    Csv { box_id: String, id: String, stamp: u64, count: usize },
}
fn change(
    root: &Path,
    relative: &str,
    updated: String,
    kind: ChangeKind,
    open: &HashMap<String, (String, bool)>,
) -> Result<Change, String> {
    let path = forge_sys::connect::project_path(root, relative).map_err(|e| e.to_string())?;
    let original = open
        .iter()
        .find(|(p, _)| forge_core::tabs::same_path(p, &path.to_string_lossy()))
        .map(|(_, text)| text.clone());
    let mut text = TextChange::read(path, original)?;
    text.updated = updated;
    Ok(Change { relative: relative.into(), text, kind })
}
fn prepare(
    root: &Path,
    snapshots: Vec<Snapshot>,
    open: HashMap<String, (String, bool)>,
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
                        if next.text.disk.is_some() && block_state(&next.text.original, &api) == BlockState::Ok {
                            match replace_block(&next.text.original, &api.id, &generated) {
                                Ok(updated) => {
                                    next.text.updated = updated;
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
                    .then(|| (d.doc.path().to_owned(), (d.doc.text(), d.doc.dirty())))
            })
            .collect();
        let saved = self.connect_state.sync.saved.clone();
        let worker_root = root.clone();
        self.connect_state.sync.task = Some(cx.spawn_in(window, async move |this, cx| {
            let (changes, notes) =
                cx.background_executor().spawn(async move { prepare(&worker_root, snapshots, open, saved) }).await;
            this.update(cx, |ws, cx| {
                if ws.root.as_ref() != Some(&root)
                    || ws.connect_state.generation != generation
                    || ws.connect_state.revision != revision
                {
                    return;
                }
                for change in changes {
                    let root = root.clone();
                    let label = change.text.label(ws, cx);
                    let (before, after) = change.text.diff();
                    let source = match change.kind {
                        ChangeKind::Code { .. } => "Connect generated code",
                        ChangeKind::Csv { .. } => "Connect Test call CSV",
                    };
                    ws.review_change(
                        ChangeReview::new(
                            format!("{source}: {}", change.relative),
                            before,
                            after,
                            label,
                            "Reject",
                            move |apply, ws, window, cx| {
                                if !apply {
                                    return;
                                }
                                if ws.root.as_ref() != Some(&root)
                                    || ws.connect_state.generation != generation
                                    || ws.connect_state.revision != revision
                                {
                                    return ws.status_note("The API settings changed. Run the request again.", cx);
                                }
                                match ws.apply_connect_change(&root, change, window, cx) {
                                    Ok((note, code)) => {
                                        if code {
                                            ws.save_boxes(cx);
                                            ws.rebuild_graph(cx);
                                            ws.queue_graph_scan(None, cx);
                                        }
                                        ws.connect.update(cx, |p, cx| {
                                            p.status = note;
                                            cx.notify();
                                        });
                                        ws.file_tree.update(cx, |t, cx| t.refresh_all(cx));
                                    }
                                    Err(error) => ws.alert(error, window, cx),
                                }
                            },
                        ),
                        cx,
                    );
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
        let text = change.text;
        if forge_sys::connect::project_path(root, &change.relative).map_err(|e| e.to_string())? != text.path {
            return Err(format!("{} moved. Preview the change again.", change.relative));
        }
        let dirty = self.find_doc(&text.path.to_string_lossy(), cx).is_some_and(|id| self.is_dirty(id, cx));
        let source = match change.kind {
            ChangeKind::Code { .. } => "Connect generated code",
            ChangeKind::Csv { .. } => "Connect Test call CSV",
        };
        self.apply_reviewed_text(text, source, window, cx)?;
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
