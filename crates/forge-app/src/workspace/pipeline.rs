//! Lazy, debounced data-flow reading and Pipeline events.
use super::Workspace;
use crate::{palette::Choice, panels::pipeline::PipelineEvent};
use forge_core::dataflow::{build_flow, guess_pipeline};
use gpui_kit::{Context, Task, Window};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Clone, PartialEq)]
struct Key {
    path: String,
    version: u64,
    func: String,
    python: String,
}
#[derive(Default)]
pub(super) struct PipelineState {
    key: Option<Key>,
    busy: bool,
    timer: Option<Task<()>>,
    worker: Option<Task<()>>,
    generation: u64,
}
impl Workspace {
    pub(super) fn pipeline_is_current(&self, cx: &gpui_kit::App) -> bool {
        self.pipeline_state.key.as_ref() == self.pipeline_key(cx).as_ref() && !self.pipeline_state.busy
    }
    fn pipeline_key(&self, cx: &gpui_kit::App) -> Option<Key> {
        let doc = self.active_doc()?;
        let doc = &doc.read(cx).doc;
        if doc.large
            || !Path::new(doc.path())
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("py") || e.eq_ignore_ascii_case("pyw"))
        {
            return None;
        }
        let rel = self.pipeline.read(cx).rel.as_deref();
        let func = if rel == Some(doc.path()) { self.pipeline.read(cx).func.clone() } else { String::new() };
        Some(Key {
            path: doc.path().into(),
            version: doc.version,
            func,
            python: self.effective_settings(cx).python_path,
        })
    }
    pub(super) fn reset_pipeline(&mut self, cx: &mut Context<Self>) {
        let generation = self.pipeline_state.generation.wrapping_add(1);
        self.pipeline_state = PipelineState { generation, ..Default::default() };
        self.pipeline.update(cx, |p, cx| p.reset(cx));
    }
    pub(super) fn queue_pipeline(&mut self, cx: &mut Context<Self>) {
        if !self.panel_is_shown("pipeline") {
            return;
        }
        self.pipeline_state.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(350)).await;
            this.update(cx, |ws, cx| {
                ws.pipeline_state.timer = None;
                ws.ensure_pipeline(cx);
            })
            .ok();
        }));
    }
    pub(super) fn ensure_pipeline(&mut self, cx: &mut Context<Self>) {
        if self.root.is_some() {
            self.ensure_graph(cx);
        }
        // Rendering may call this every frame. Only compare document versions;
        // copy the rope and launch Python once per changed source/selection.
        let Some(key) = self.pipeline_key(cx) else {
            if self.pipeline.read(cx).rel.is_some() {
                self.reset_pipeline(cx);
            }
            return;
        };
        if self.pipeline_state.key.as_ref() == Some(&key)
            || self.pipeline_state.busy
            || self.pipeline_state.timer.is_some()
        {
            return;
        }
        let source = self.active_doc().unwrap().read(cx).doc.text();
        let root = self.root.clone().unwrap_or_else(|| Path::new(&key.path).parent().unwrap_or(Path::new(".")).into());
        let rel = Path::new(&key.path)
            .strip_prefix(&root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let module = forge_core::boxes::module_of_path(&rel);
        let generation = self.pipeline_state.generation;
        self.pipeline_state.key = Some(key.clone());
        self.pipeline_state.busy = true;
        self.pipeline.update(cx, |p, cx| {
            if p.rel.as_deref() != Some(key.path.as_str()) {
                p.set_flow(Default::default(), None, cx);
                p.selected = None;
            }
            p.rel = Some(key.path.clone());
            p.module = module;
            p.loading = true;
            cx.notify();
        });
        self.pipeline_state.worker = Some(cx.spawn(async move |this, cx| {
            let requested = key.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let python = forge_run::python::find_python(&root, Some(&requested.python))
                        .ok_or(forge_scan::Error::NoPython)?;
                    let list = forge_scan::run_dataflow(&python, &root, &source, "", Duration::from_secs(20))?;
                    if list.error.is_some() {
                        return Ok((list, None));
                    }
                    let func = list
                        .functions
                        .iter()
                        .find(|f| f.qual == requested.func)
                        .map(|f| f.qual.as_str())
                        .or_else(|| guess_pipeline(&source, &list.functions));
                    let raw = if let Some(func) = func {
                        forge_scan::run_dataflow(&python, &root, &source, func, Duration::from_secs(20))?
                    } else {
                        list
                    };
                    let flow = build_flow(&raw);
                    Ok::<_, forge_scan::Error>((raw, flow))
                })
                .await;
            this.update(cx, |ws, cx| {
                if ws.pipeline_state.generation != generation {
                    return;
                }
                ws.pipeline_state.busy = false;
                if ws.pipeline_key(cx).as_ref() != Some(&key) {
                    ws.ensure_pipeline(cx);
                    return;
                }
                ws.pipeline.update(cx, |p, cx| match result {
                    Ok((raw, flow)) => p.set_flow(raw, flow, cx),
                    Err(error) => {
                        p.loading = false;
                        p.status = error.to_string();
                        p.flow = None;
                        cx.notify();
                    }
                });
                ws.pipeline_state.key = ws.pipeline_key(cx);
            })
            .ok();
        }));
    }
    pub(super) fn on_pipeline_event(&mut self, event: &PipelineEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            PipelineEvent::AddBlock(_)
            | PipelineEvent::New
            | PipelineEvent::Export
            | PipelineEvent::Argument(_)
            | PipelineEvent::EditBlock => self.on_pipeline_author(event, window, cx),
            PipelineEvent::Refresh => {
                self.pipeline_state.key = None;
                self.ensure_pipeline(cx);
            }
            PipelineEvent::PickFunction => {
                let Some(raw) = &self.pipeline.read(cx).raw else {
                    return;
                };
                let path = self.pipeline.read(cx).rel.clone();
                let choices = raw
                    .functions
                    .iter()
                    .map(|f| {
                        let func = f.qual.clone();
                        let path = path.clone();
                        Choice::new(format!("{}()", f.qual), move |ws, _, cx| {
                            if ws.pipeline.read(cx).rel != path {
                                return;
                            }
                            ws.pipeline.update(cx, |p, cx| {
                                p.func = func.clone();
                                p.selected = None;
                                cx.notify();
                            });
                            ws.ensure_pipeline(cx);
                        })
                        .detail(format!("Line {}", f.line))
                    })
                    .collect();
                self.choose("Inspect function", choices, window, cx);
            }
            PipelineEvent::Open(line) => {
                let Some(path) = self.pipeline.read(cx).rel.clone() else {
                    return;
                };
                self.open_path_at(PathBuf::from(path), Some(line.saturating_sub(1) as usize), window, cx);
                self.reveal_panel("editor", cx);
                self.focus_editor(window, cx);
            }
            PipelineEvent::Run => {
                if self.pipeline_state.key.as_ref() != self.pipeline_key(cx).as_ref() {
                    self.ensure_pipeline(cx);
                    self.status_note("The function changed. Wait for its pipeline to refresh, then run again.", cx);
                    return;
                }
                let p = self.pipeline.read(cx);
                if p.func.is_empty() || p.loading {
                    return;
                }
                self.run.data_target = Some(format!("{}:{}", p.module, p.func));
                self.run_current_file(window, cx);
            }
        }
    }
}
