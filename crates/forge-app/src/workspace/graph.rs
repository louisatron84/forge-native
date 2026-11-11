//! Project scan scheduling, unsaved overrides and native graph navigation.
//! Replaces scanner/graph orchestration in `src/renderer/main.ts`.
use super::Workspace;
use crate::panels::graph::{prepare_layout, GraphEvent};
use forge_core::{
    graph_view::build_view,
    project_graph::{build_graph, keep_last_good, replace_files, RawScan},
};
use gpui_kit::{Context, Task, Window};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};

#[derive(Default)]
pub(super) struct GraphState {
    raw: Option<Arc<RawScan>>,
    generation: u64,
    view_revision: u64,
    busy: bool,
    full: bool,
    attempted: bool,
    pending: BTreeSet<String>,
    timer: Option<Task<()>>,
    worker: Option<Task<()>>,
    view_worker: Option<Task<()>>,
}
impl Workspace {
    pub(super) fn on_trace_message(&mut self, message: &serde_json::Value, cx: &mut Context<Self>) {
        if let Some(message) = forge_core::data_summary::parse_data_message(message) {
            self.pipeline.update(cx, |p, cx| p.add(message, cx));
            return;
        }
        if let Some(record) = forge_core::network::parse_net(message) {
            let visible = self.panel_is_shown("graph");
            self.project_graph.update(cx, |p, cx| p.network_record(&record, visible, cx));
            self.network.update(cx, |p, cx| p.upsert(Arc::new(record), cx));
            return;
        }
        if let Some(profile) = forge_core::profile::parse_profile(message) {
            self.project_graph.update(cx, |p, cx| p.set_profile(profile, cx));
            return;
        }
        match forge_core::live_run::parse_trace(message) {
            Some(forge_core::live_run::TraceMessage::Tick(tick)) => {
                let visible = self.panel_is_shown("graph");
                self.project_graph.update(cx, |p, cx| p.tick(tick, visible, cx))
            }
            Some(forge_core::live_run::TraceMessage::End) => self.project_graph.update(cx, |p, cx| p.end_run(cx)),
            None => {}
        }
    }
    pub(super) fn reset_graph(&mut self, cx: &mut Context<Self>) {
        let generation = self.graph.generation.wrapping_add(1);
        self.graph = GraphState { generation, ..Default::default() };
        self.project_graph.update(cx, |p, cx| p.reset(cx));
        self.network.update(cx, |p, cx| p.reset(cx));
        self.reset_pipeline(cx);
    }
    pub(super) fn ensure_graph(&mut self, cx: &mut Context<Self>) {
        if !self.graph.attempted && !self.graph.busy && self.graph.timer.is_none() && self.root.is_some() {
            self.graph.full = true;
            self.start_graph_scan(cx);
        }
    }
    pub(super) fn queue_graph_scan(&mut self, path: Option<&str>, cx: &mut Context<Self>) {
        if self.graph.raw.is_none() && !self.panel_is_shown("graph") {
            return;
        }
        if let Some(path) = path {
            let Some(root) = &self.root else {
                return;
            };
            let Ok(rel) = std::path::Path::new(path).strip_prefix(root) else {
                return;
            };
            if !rel.extension().is_some_and(|e| e.eq_ignore_ascii_case("py") || e.eq_ignore_ascii_case("pyw")) {
                return;
            }
            self.graph.pending.insert(rel.to_string_lossy().replace('\\', "/"));
        } else {
            self.graph.full = true;
        }
        if self.graph.busy {
            return;
        }
        self.graph.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(350)).await;
            this.update(cx, |ws, cx| {
                ws.graph.timer = None;
                ws.start_graph_scan(cx);
            })
            .ok();
        }));
    }
    fn start_graph_scan(&mut self, cx: &mut Context<Self>) {
        if self.graph.busy {
            return;
        }
        let Some(root) = self.root.clone() else {
            return;
        };
        let full = std::mem::take(&mut self.graph.full) || self.graph.raw.is_none();
        let files: Vec<_> = std::mem::take(&mut self.graph.pending).into_iter().collect();
        if !full && files.is_empty() {
            return;
        }
        let mut overrides = BTreeMap::new();
        for entity in self.docs.values() {
            let doc = &entity.read(cx).doc;
            if !doc.dirty() || doc.large {
                continue;
            }
            if let Ok(rel) = std::path::Path::new(doc.path()).strip_prefix(&root) {
                overrides.insert(rel.to_string_lossy().replace('\\', "/"), doc.text());
            }
        }
        let configured = self.effective_settings(cx).python_path;
        let generation = self.graph.generation;
        let previous = self.graph.raw.clone();
        self.graph.busy = true;
        self.graph.attempted = true;
        self.graph.view_revision = self.graph.view_revision.wrapping_add(1);
        self.graph.view_worker = None;
        self.project_graph.update(cx, |p, cx| {
            p.scanning = true;
            p.status.clear();
            cx.notify();
        });
        self.graph.worker = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let next = if full {
                        forge_scan::scan_project(&root, Some(&configured), &overrides)?
                    } else {
                        let fresh = forge_scan::scan_project_files(&root, Some(&configured), &files, &overrides)?;
                        replace_files(previous.as_deref().unwrap_or(&RawScan::default()), fresh)
                    };
                    let scan = keep_last_good(next, previous.as_deref());
                    let graph = Arc::new(build_graph(&scan));
                    Ok::<_, forge_scan::Error>((Arc::new(scan), graph))
                })
                .await;
            this.update(cx, |ws, cx| {
                if ws.graph.generation != generation {
                    return;
                }
                ws.graph.busy = false;
                ws.graph.worker = None;
                match result {
                    Ok((scan, graph)) => {
                        ws.graph.raw = Some(scan);
                        ws.project_graph.update(cx, |p, _| p.graph = graph);
                        ws.rebuild_graph(cx);
                    }
                    Err(error) => ws.project_graph.update(cx, |p, cx| {
                        p.scanning = false;
                        p.status = format!("Error: {error}");
                        cx.notify();
                    }),
                }
                if ws.graph.full || !ws.graph.pending.is_empty() {
                    ws.start_graph_scan(cx);
                }
            })
            .ok();
        }));
    }
    pub(super) fn rebuild_graph(&mut self, cx: &mut Context<Self>) {
        self.graph.view_revision = self.graph.view_revision.wrapping_add(1);
        let revision = self.graph.view_revision;
        let generation = self.graph.generation;
        let panel = self.project_graph.read(cx);
        let graph = panel.graph.clone();
        let options = panel.options.clone();
        let boxes = self.connect.read(cx).boxes.iter().map(forge_core::boxes::ApiBox::metadata).collect::<Vec<_>>();
        self.graph.view_worker = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(40)).await;
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut data = build_view(&graph, &options);
                    let mut layout = prepare_layout(&data);
                    forge_core::box_layer::add_boxes(&mut data, &mut layout, &boxes, options.mode);
                    (graph, data, layout, boxes)
                })
                .await;
            this.update(cx, |ws, cx| {
                if ws.graph.generation != generation || ws.graph.view_revision != revision {
                    return;
                }
                ws.project_graph.update(cx, |p, cx| {
                    p.set_view(result.0, result.1, result.2, cx);
                    p.attach_boxes(&result.3, cx);
                });
            })
            .ok();
        }));
    }
    pub(super) fn on_graph_event(&mut self, event: &GraphEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            GraphEvent::AddApi(module) => {
                self.on_connect_event(&crate::panels::connect::ConnectEvent::Add, window, cx);
                if let Some(module) = module {
                    if let Some(api) = self.connect.read(cx).current() {
                        let id = api.id.clone();
                        self.on_graph_event(&GraphEvent::Attach { box_id: id, module: module.clone() }, window, cx);
                    }
                }
            }
            GraphEvent::Attach { box_id, module } => {
                let Some(file) =
                    self.project_graph.read(cx).graph.modules.iter().find(|m| &m.id == module).map(|m| m.path.clone())
                else {
                    return;
                };
                self.connect.update(cx, |p, cx| {
                    if let Some(index) = p.boxes.iter().position(|b| &b.id == box_id) {
                        if p.boxes[index].hash.is_some() {
                            p.status =
                                "This box already has generated code. Remove its connection before moving it.".into();
                            cx.notify();
                            return;
                        }
                        let api = &mut p.boxes[index];
                        api.target.file = file;
                        api.target.mode = "append".into();
                        api.attach = Some(forge_core::boxes::BoxAttach { module: module.clone() });
                        p.select(index, window, cx);
                        cx.emit(crate::panels::connect::ConnectEvent::Changed);
                    }
                });
                self.reveal_panel("box", cx);
            }
            GraphEvent::Box(id) => {
                if let Some(index) = self.connect.read(cx).boxes.iter().position(|b| &b.id == id) {
                    self.connect.update(cx, |p, cx| p.select(index, window, cx));
                    self.reveal_panel("box", cx);
                }
            }
            GraphEvent::Refresh => {
                self.graph.full = true;
                self.start_graph_scan(cx);
            }
            GraphEvent::View(options) => {
                self.project_graph.update(cx, |p, _| p.options = options.clone());
                self.rebuild_graph(cx);
            }
            GraphEvent::Open { module, line } => {
                let path =
                    self.project_graph.read(cx).graph.modules.iter().find(|m| &m.id == module).map(|m| m.path.clone());
                if let (Some(root), Some(path)) = (&self.root, path) {
                    self.open_path_at(root.join(path), Some(*line), window, cx);
                    self.reveal_panel("editor", cx);
                    self.focus_editor(window, cx);
                }
            }
        }
    }
}
