//! Native pipeline authoring; source stays ordinary, undoable Python.
use super::Workspace;
use crate::{palette::Choice, panels::pipeline::PipelineEvent};
use forge_core::{
    blocks::{builtin_blocks, project_blocks, BlockInfo, BLOCKS_PY},
    dataflow::{RawFlow, Step},
    pipeline_edit::*,
};
use gpui_kit::{Context, Window};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
fn step_at(raw: &RawFlow, line: u32) -> Option<&Step> {
    let mut todo = raw.steps.as_ref()?.iter().collect::<Vec<_>>();
    while let Some(s) = todo.pop() {
        if s.line == line {
            return Some(s);
        }
        if let Some(body) = &s.body {
            todo.extend(body);
        }
    }
    None
}
impl Workspace {
    fn pipeline_library(&self, cx: &gpui_kit::App) -> Vec<BlockInfo> {
        let mut blocks = project_blocks(&self.project_graph.read(cx).graph);
        let project_names: BTreeSet<_> = blocks.iter().map(|b| b.name.clone()).collect();
        blocks.extend(builtin_blocks().into_iter().filter(|b| !project_names.contains(&b.name)));
        blocks
    }
    fn pipeline_snapshot(&self, cx: &gpui_kit::App) -> Option<(PathBuf, String, RawFlow)> {
        if !self.pipeline_is_current(cx) {
            return None;
        }
        let p = self.pipeline.read(cx);
        let path = p.rel.as_ref()?;
        let doc = self.active_doc()?;
        let d = &doc.read(cx).doc;
        if d.path() != path || p.loading {
            return None;
        }
        Some((PathBuf::from(path), d.text(), p.raw.clone()?))
    }
    fn source_still_matches(&self, path: &Path, source: &str, cx: &gpui_kit::App) -> bool {
        self.active_doc().is_some_and(|d| {
            let d = &d.read(cx).doc;
            Path::new(d.path()) == path && d.text() == source
        })
    }
    fn pipeline_write(&mut self, path: &Path, text: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !path.exists() {
            let Some(root) = &self.root else {
                return false;
            };
            let Ok(rel) = path.strip_prefix(root) else {
                return false;
            };
            if let Err(e) = forge_sys::files::create_entry(root, &rel.to_string_lossy(), false) {
                self.alert(e.to_string(), window, cx);
                return false;
            }
        }
        let id = match self.find_doc(&path.to_string_lossy(), cx) {
            Some(id) => Some(id),
            None => match self.load_doc(path, false, cx) {
                Ok(id) => id,
                Err(e) => {
                    self.alert(e, window, cx);
                    return false;
                }
            },
        };
        let Some(id) = id else {
            return false;
        };
        let doc = self.docs[&id].clone();
        let dirty = doc.read(cx).doc.dirty();
        doc.update(cx, |d, cx| {
            d.edit(None, cx, |doc| {
                let forge_text::Doc { buffer, history, .. } = doc;
                history.transact(buffer, forge_text::EditKind::Other, forge_text::Pos::new(0, 0), |b| {
                    b.set_text(text);
                    forge_text::Pos::new(0, 0)
                });
            })
        });
        if !dirty {
            self.save(Some(id), window, cx);
        }
        self.file_tree.update(cx, |p, cx| p.refresh_all(cx));
        true
    }
    fn project_text(&self, path: &Path, cx: &gpui_kit::App) -> String {
        self.find_doc(&path.to_string_lossy(), cx)
            .map(|id| self.docs[&id].read(cx).doc.text())
            .unwrap_or_else(|| std::fs::read_to_string(path).unwrap_or_default())
    }
    pub(super) fn place_pipeline_block(
        &mut self,
        b: &BlockInfo,
        each: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, text, raw)) = self.pipeline_snapshot(cx) else {
            return;
        };
        let Some(root) = self.root.clone() else {
            return;
        };
        // A fresh AST is required: the editor may have changed while a picker was open.
        if self.pipeline.read(cx).flow.as_ref().is_none_or(|f| raw.func.as_ref() != Some(&f.func)) {
            return;
        }
        let p = self.pipeline.read(cx);
        let node = p.selected.as_ref().and_then(|id| p.flow.as_ref()?.nodes.iter().find(|n| &n.id == id));
        let mut vars = raw
            .params
            .iter()
            .filter(|p| p.name != "self" && p.name != "cls")
            .map(|p| p.name.clone())
            .collect::<Vec<_>>();
        let mut after = None;
        for s in raw.steps.iter().flatten() {
            for name in &s.targets {
                vars.retain(|v| v != name);
                vars.push(name.clone());
            }
            if node.is_some_and(|n| n.line == s.line) {
                after = Some(s);
                break;
            }
        }
        let taken = vars.iter().cloned().collect();
        let inputs = vars.iter().rev().take(2).cloned().collect::<Vec<_>>();
        let code = if each {
            let Some(over) = inputs.first() else {
                self.status_note("Select a step with a value before using For each.", cx);
                return;
            };
            for_each_code(b, over, &taken, false).0
        } else {
            vec![block_call(b, &inputs, &taken).0]
        };
        let mut next = insert_in_function(&text, &raw, &code, after);
        let module = forge_core::boxes::module_of_path(&path.strip_prefix(&root).unwrap_or(&path).to_string_lossy());
        if b.module != module {
            next = import_from(&next, &b.module, &b.name);
        }
        if b.path.is_none() {
            let Ok(helper) = forge_sys::connect::project_path(&root, "forge_blocks.py") else {
                return;
            };
            if !helper.exists() && !self.pipeline_write(&helper, BLOCKS_PY, window, cx) {
                return;
            }
        }
        if self.source_still_matches(&path, &text, cx) {
            self.pipeline_write(&path, &next, window, cx);
        }
    }
    pub(super) fn on_pipeline_author(&mut self, event: &PipelineEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            PipelineEvent::AddBlock(each) => {
                self.ensure_graph(cx);
                let each = *each;
                let choices = self
                    .pipeline_library(cx)
                    .into_iter()
                    .map(|b| {
                        let detail = b.doc.clone();
                        Choice::new(format!("{} · {}", b.name, b.kind), move |ws, window, cx| {
                            ws.place_pipeline_block(&b, each, window, cx)
                        })
                        .detail(detail)
                    })
                    .collect();
                self.choose("Add a Python block", choices, window, cx);
            }
            PipelineEvent::New => {
                let Some(doc) = self.active_doc() else {
                    self.status_note("Open a Python file before adding a pipeline.", cx);
                    return;
                };
                let path = PathBuf::from(doc.read(cx).doc.path());
                let text = doc.read(cx).doc.text();
                self.palette.update(cx, |p, cx| {
                    p.prompt(
                        "Pipeline function name",
                        "pipeline",
                        "Create",
                        move |ws, name, window, cx| {
                            if name.is_empty()
                                || !name
                                    .chars()
                                    .enumerate()
                                    .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
                            {
                                ws.status_note("Use a Python function name.", cx);
                                return;
                            }
                            if ws.source_still_matches(&path, &text, cx) {
                                ws.pipeline_write(&path, &new_pipeline(&text, &name), window, cx);
                            }
                        },
                        window,
                        cx,
                    )
                });
            }
            PipelineEvent::Argument(index) => {
                let Some((path, text, raw)) = self.pipeline_snapshot(cx) else {
                    return;
                };
                let p = self.pipeline.read(cx);
                let Some(n) = p.selected.as_ref().and_then(|id| p.flow.as_ref()?.nodes.iter().find(|n| &n.id == id))
                else {
                    return;
                };
                let Some(step) = step_at(&raw, n.line).cloned() else {
                    return;
                };
                let Some(arg) = step.call.as_ref().and_then(|c| c.args.get(*index)) else {
                    return;
                };
                let initial = arg.text.clone();
                let index = *index;
                self.palette.update(cx, |p, cx| {
                    p.prompt(
                        "Input variable or Python setting expression",
                        &initial,
                        "Update",
                        move |ws, value, window, cx| {
                            if ws.source_still_matches(&path, &text, cx) {
                                ws.pipeline_write(&path, &set_argument(&text, &step, index, &value), window, cx);
                            }
                        },
                        window,
                        cx,
                    )
                });
            }
            PipelineEvent::Export => {
                let Some((path, text, raw)) = self.pipeline_snapshot(cx) else {
                    return;
                };
                let Some(root) = self.root.clone() else {
                    return;
                };
                let exported = match export_standalone(&text, raw.func.as_deref().unwrap_or("pipeline")) {
                    Ok(t) => t,
                    Err(e) => {
                        self.alert(e, window, cx);
                        return;
                    }
                };
                self.palette.update(cx, |p, cx| {
                    p.prompt(
                        "Export Python path (inside this project)",
                        "pipeline_export.py",
                        "Export",
                        move |ws, name, window, cx| {
                            if ws.root.as_ref() != Some(&root) || !ws.source_still_matches(&path, &text, cx) {
                                ws.status_note("The project or source changed. Export again.", cx);
                                return;
                            }
                            let target = match forge_sys::connect::project_path(&root, &name) {
                                Ok(p) => p,
                                Err(e) => {
                                    ws.alert(e.to_string(), window, cx);
                                    return;
                                }
                            };
                            if target.exists() {
                                ws.alert("Choose a new filename to preserve existing files.", window, cx);
                                return;
                            }
                            if ws.pipeline_write(&target, &exported, window, cx) {
                                ws.open_path(target, window, cx);
                            }
                        },
                        window,
                        cx,
                    )
                });
            }
            PipelineEvent::EditBlock => {
                let Some((path, text, raw)) = self.pipeline_snapshot(cx) else {
                    return;
                };
                let p = self.pipeline.read(cx);
                let Some(name) = p
                    .selected
                    .as_ref()
                    .and_then(|id| p.flow.as_ref()?.nodes.iter().find(|n| &n.id == id))
                    .and_then(|n| n.call.as_ref())
                    .map(|c| c.name.clone())
                else {
                    return;
                };
                let Some(b) = self.pipeline_library(cx).into_iter().find(|b| b.name == name) else {
                    return;
                };
                let Some(root) = self.root.clone() else {
                    return;
                };
                if let Some(rel) = b.path {
                    if let Ok(path) = forge_sys::connect::project_path(&root, &rel) {
                        self.open_path_at(path, Some(b.line.saturating_sub(1)), window, cx);
                        self.reveal_panel("editor", cx);
                    }
                    return;
                }
                let Ok(target) = forge_sys::connect::project_path(&root, "blocks.py") else {
                    return;
                };
                let content = self.project_text(&target, cx);
                let Some(copy) = copy_builtin_to(&content, &name) else {
                    return;
                };
                if !self.pipeline_write(&target, &copy, window, cx) {
                    return;
                }
                let prefix = "from forge_blocks import ";
                let mut next = Vec::new();
                for line in text.lines() {
                    if let Some(rest) = line.strip_prefix(prefix) {
                        let names = rest.split(',').map(str::trim).filter(|n| *n != name).collect::<Vec<_>>();
                        if !names.is_empty() {
                            next.push(format!("{prefix}{}", names.join(", ")));
                        }
                    } else {
                        next.push(line.to_string());
                    }
                }
                let next = import_from(&(next.join("\n") + "\n"), "blocks", &name);
                if self.source_still_matches(&path, &text, cx) {
                    self.pipeline_write(&path, &next, window, cx);
                }
                let line = copy.lines().position(|l| l.starts_with(&format!("def {name}("))).unwrap_or(0);
                self.open_path_at(target, Some(line), window, cx);
                self.reveal_panel("editor", cx);
                let _ = raw;
            }
            _ => {}
        }
    }
}
