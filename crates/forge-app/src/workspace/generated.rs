//! Review generated edits without changing an editor buffer or writing to disk.
use super::Workspace;
use crate::document::{Document, DocumentEvent};
use crate::panels::git_inspect::{GitInspect, InspectEvent};
use forge_core::tabs::DocId;
use forge_text::{Doc, EditKind, Encoding, Eol, Pos};
use gpui_kit::{Context, Entity, Window};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

type Decision = Box<dyn FnOnce(bool, &mut Workspace, &mut Window, &mut Context<Workspace>)>;

pub(super) struct ChangeReview {
    title: String,
    before: String,
    after: String,
    apply: &'static str,
    reject: &'static str,
    decision: Decision,
}

pub(super) struct PendingWrite {
    pub(super) before: Option<Vec<u8>>,
    pub(super) after: [u8; 32],
    pub(super) source: &'static str,
}

pub(super) struct NewFiles {
    pub directory: PathBuf,
    pub files: Vec<(String, Vec<u8>)>,
    pub source: &'static str,
    pub exclusive: bool,
}

#[derive(Clone)]
pub(super) struct TextChange {
    pub path: PathBuf,
    identity: PathBuf,
    pub disk: Option<Vec<u8>>,
    pub original: String,
    pub updated: String,
}

impl TextChange {
    pub(super) fn read(path: PathBuf, open: Option<(String, bool)>) -> Result<Self, String> {
        let identity = forge_sys::generated_history::identity(&path).map_err(|error| error.to_string())?;
        let disk = forge_sys::generated_history::read(&path).map_err(|error| error.to_string())?;
        if disk.as_deref().is_some_and(|bytes| !forge_sys::generated_history::is_text(bytes)) {
            return Err("Choose a normal text file.".into());
        }
        let disk_text = disk.as_deref().map(|bytes| forge_sys::text_file::decode(bytes).0).unwrap_or_default();
        let original = match open {
            Some((text, false)) if disk.is_none() || !matches_disk_text(&text, &disk_text) => {
                return Err(format!("{} changed on disk. Reopen it before previewing a change.", path.display()));
            }
            Some((text, _)) => text,
            None => disk_text,
        };
        Ok(Self { path, identity, disk, updated: original.clone(), original })
    }

    pub(super) fn check(&self, ws: &Workspace, cx: &gpui_kit::App) -> Result<Option<DocId>, String> {
        if forge_sys::generated_history::identity(&self.path).as_ref().ok() != Some(&self.identity)
            || forge_sys::generated_history::read(&self.path).map_err(|e| e.to_string())? != self.disk
        {
            return Err(format!("{} changed on disk. Preview the change again.", self.path.display()));
        }
        let id = ws.find_doc(&self.path.to_string_lossy(), cx);
        let current = id.map(|id| ws.docs[&id].read(cx).doc.text()).unwrap_or_else(|| {
            self.disk.as_deref().map(|bytes| forge_sys::text_file::decode(bytes).0).unwrap_or_default()
        });
        if current != self.original {
            return Err(format!("{} has newer edits. Preview the change again.", self.path.display()));
        }
        Ok(id)
    }

    pub(super) fn label(&self, ws: &Workspace, cx: &gpui_kit::App) -> &'static str {
        if ws.find_doc(&self.path.to_string_lossy(), cx).is_some_and(|id| ws.is_dirty(id, cx)) {
            "Apply to editor"
        } else if self.disk.is_some() {
            "Overwrite file"
        } else {
            "Create file"
        }
    }

    pub(super) fn diff(&self) -> (String, String) {
        if self.path.file_name().is_some_and(|name| name == ".env") {
            let keys = |text| {
                let vars = forge_core::dotenv::parse_dotenv(text);
                vars.keys().map(|name| format!("{name}=<hidden>\n")).collect::<String>()
            };
            (keys(&self.original), keys(&self.updated))
        } else if forge_sys::file_write::is_secret_file(&self.path) {
            (String::new(), "Secret file · contents hidden".into())
        } else {
            (self.original.clone(), self.updated.clone())
        }
    }
}

fn matches_disk_text(editor: &str, disk: &str) -> bool {
    editor == disk || editor.replace("\r\n", "\n").replace('\r', "\n") == disk.replace("\r\n", "\n").replace('\r', "\n")
}

#[derive(Clone, Copy, Default)]
pub(super) struct WriteSummary {
    pub written: usize,
    pub failed: bool,
    reserved: bool,
}

impl ChangeReview {
    pub(super) fn new(
        title: String,
        before: String,
        after: String,
        apply: &'static str,
        reject: &'static str,
        decision: impl FnOnce(bool, &mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
    ) -> Self {
        Self { title, before, after, apply, reject, decision: Box::new(decision) }
    }
}

impl Workspace {
    pub(super) fn open_config_or_review(
        &mut self,
        target: PathBuf,
        content: serde_json::Value,
        project: Option<(PathBuf, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut change = match self.generate_text_change(target, cx) {
            Ok(change) => change,
            Err(error) => return self.alert(error, window, cx),
        };
        if change.disk.is_some() {
            return self.open_path(change.path, window, cx);
        }
        match serde_json::to_string_pretty(&content) {
            Ok(text) => change.updated = format!("{text}\n"),
            Err(error) => return self.alert(error.to_string(), window, cx),
        }
        self.review_text_change(
            change,
            "Create configuration",
            project,
            |ws, path, window, cx| ws.open_path(path, window, cx),
            cx,
        );
    }

    pub(super) fn generate_text_change(&self, path: PathBuf, cx: &gpui_kit::App) -> Result<TextChange, String> {
        let open = self.find_doc(&path.to_string_lossy(), cx).map(|id| {
            let doc = &self.docs[&id].read(cx).doc;
            (doc.text(), doc.dirty())
        });
        TextChange::read(path, open)
    }

    pub(super) fn review_text_change(
        &mut self,
        change: TextChange,
        source: &'static str,
        project: Option<(PathBuf, String)>,
        done: impl FnOnce(&mut Workspace, PathBuf, &mut Window, &mut Context<Workspace>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let (before, after) = change.diff();
        let name = change.path.file_name().unwrap_or_default().to_string_lossy();
        let title = format!("{source} · {name}");
        self.review_change(
            ChangeReview::new(title, before, after, change.label(self, cx), "Reject", move |apply, ws, window, cx| {
                if !apply {
                    return;
                }
                if let Some((root, relative)) = project {
                    if ws.root.as_ref() != Some(&root)
                        || forge_sys::connect::project_path(&root, &relative).as_ref().ok() != Some(&change.path)
                    {
                        return ws.alert("The project or destination changed. Preview the change again.", window, cx);
                    }
                }
                let path = change.path.clone();
                if let Err(error) = ws.apply_reviewed_text(change, source, window, cx) {
                    return ws.alert(error, window, cx);
                }
                ws.file_tree.update(cx, |t, cx| t.refresh_all(cx));
                done(ws, path, window, cx);
            }),
            cx,
        );
    }

    pub(super) fn remember_reviewed_edit(
        &mut self,
        id: DocId,
        before: Option<Vec<u8>>,
        source: &'static str,
        cx: &gpui_kit::App,
    ) {
        let doc = &self.docs[&id].read(cx).doc;
        let after = forge_sys::generated_history::hash(&forge_sys::text_file::encode(&doc.text(), doc.encoding));
        self.pending_reviewed_writes
            .entry(id)
            .and_modify(|write| {
                write.after = after;
                write.source = source;
            })
            .or_insert(PendingWrite { before, after, source });
    }

    /// A tool preview is based on a specific version of the editor buffer.
    pub(super) fn review_tool_edit(
        &mut self,
        doc: Entity<Document>,
        version: u64,
        difference: (String, String),
        source: &'static str,
        save_now: bool,
        cx: &mut Context<Self>,
    ) {
        let (before, after) = difference;
        let data = &doc.read(cx).doc;
        let path = data.path().to_owned();
        let settings = (data.encoding, data.eol());
        if data.version != version {
            self.status_note("The editor changed while the tool was running. Run the tool again.", cx);
            return;
        }
        let Some(id) = self.find_doc(&path, cx) else { return };
        let disk = match forge_sys::generated_history::read(Path::new(&path)) {
            Ok(disk) => disk,
            Err(error) => {
                self.status_note(format!("Could not read the file for review: {error}"), cx);
                return;
            }
        };
        if !data.dirty()
            && disk.as_deref().is_none_or(|bytes| !matches_disk_text(&before, &forge_sys::text_file::decode(bytes).0))
        {
            self.status_note("The file changed on disk. Reopen it before running the tool.", cx);
            return;
        }
        let Some(edit) = forge_core::lint::line_diff(&before, &after) else { return };
        self.review_change(
            ChangeReview::new(
                format!("{source} · {}", data.name()),
                before,
                after,
                if save_now { "Apply and save" } else { "Apply to editor" },
                if save_now { "Save without formatting" } else { "Reject" },
                move |apply, ws, window, cx| {
                    if !apply && !save_now {
                        return;
                    }
                    let data = &doc.read(cx).doc;
                    if ws.docs.get(&id) != Some(&doc)
                        || data.path() != path
                        || data.version != version
                        || settings != (data.encoding, data.eol())
                    {
                        return ws.status_note("The editor changed. Run the tool again to preview its new result.", cx);
                    }
                    if !apply {
                        if forge_sys::generated_history::read(Path::new(&path)).ok().as_ref() != Some(&disk) {
                            return ws.alert("The file on disk changed. Review that edit before saving.", window, cx);
                        }
                        ws.save(Some(id), window, cx);
                        return;
                    }
                    ws.apply_doc_edits(&doc, super::code_tools::char_edits(&[edit]), cx);
                    ws.remember_reviewed_edit(id, disk, source, cx);
                    if save_now {
                        ws.save(Some(id), window, cx);
                    }
                    ws.lint_soon(path, std::time::Duration::from_millis(100), cx);
                },
            ),
            cx,
        );
    }

    pub(super) fn review_change(&mut self, change: ChangeReview, cx: &mut Context<Self>) {
        self.generated_changes.push_back(change);
        if self.generated_changes.len() == 1 {
            self.show_change_review(cx);
        }
    }

    pub(super) fn review_new_files(
        &mut self,
        plan: NewFiles,
        done: impl FnOnce(WriteSummary, &mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if plan.files.is_empty() {
            return done(WriteSummary::default(), self, window, cx);
        }
        let directory = Rc::new(plan.directory);
        let identity = match forge_sys::generated_history::identity(&directory) {
            Ok(path) => Rc::new(path),
            Err(error) => return self.alert(error.to_string(), window, cx),
        };
        let summary = Rc::new(Cell::new(WriteSummary::default()));
        let len = plan.files.len();
        let mut done = Some(done);
        for (index, (relative, bytes)) in plan.files.into_iter().enumerate() {
            let directory = directory.clone();
            let identity = identity.clone();
            let summary = summary.clone();
            let complete = (index + 1 == len).then(|| done.take().unwrap());
            let source = plan.source;
            let exclusive = plan.exclusive;
            let proposed = if forge_sys::file_write::is_secret_file(Path::new(&relative)) {
                "Secret file · contents hidden".into()
            } else if forge_sys::generated_history::is_text(&bytes) {
                forge_sys::text_file::decode(&bytes).0
            } else {
                format!("Binary file · {} bytes", bytes.len())
            };
            self.review_change(
                ChangeReview::new(
                    format!("{source} · {relative} ({}/{len})", index + 1),
                    String::new(),
                    proposed,
                    "Create file",
                    "Reject",
                    move |apply, ws, window, cx| {
                        let mut result = summary.get();
                        if apply && !(exclusive && result.failed && !result.reserved) {
                            let write = (|| -> std::io::Result<()> {
                                if forge_sys::generated_history::identity(&directory)? != *identity {
                                    return Err(std::io::Error::other("The destination changed. Preview again."));
                                }
                                if !result.reserved {
                                    if exclusive {
                                        if let Some(parent) = directory.parent() {
                                            std::fs::create_dir_all(parent)?;
                                        }
                                        std::fs::create_dir(&*directory)?;
                                    } else {
                                        std::fs::create_dir_all(&*directory)?;
                                    }
                                    result.reserved = true;
                                }
                                let path = forge_sys::connect::project_path(&directory, &relative)?;
                                ws.write_reviewed(&path, None, &bytes, source, cx)?;
                                ws.after_save(&path.to_string_lossy(), window, cx);
                                result.written += 1;
                                Ok(())
                            })();
                            if let Err(error) = write {
                                result.failed = true;
                                ws.alert(format!("Could not create {relative}: {error}"), window, cx);
                            }
                            summary.set(result);
                        }
                        if let Some(complete) = complete {
                            complete(result, ws, window, cx);
                        }
                    },
                ),
                cx,
            );
        }
    }

    pub(super) fn offer_legacy_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if super::app_state::state(cx).legacy_import_offered {
            return;
        }
        let Some(legacy) = forge_sys::user_state::legacy_data_dir() else { return };
        let native = super::app_state::state(cx).user.dir().to_path_buf();
        cx.global_mut::<super::AppState>().legacy_import_offered = true;
        cx.spawn_in(window, async move |this, cx| {
            let output = native.clone();
            let plan = cx
                .background_executor()
                .spawn(async move { forge_sys::user_state::plan_legacy_import(&legacy, &output) })
                .await;
            this.update_in(cx, |ws, window, cx| {
                let files = match plan {
                    Ok(Some(files)) => files.into_iter().map(|file| (file.relative, file.contents)).collect(),
                    Ok(None) => return,
                    Err(error) => return ws.alert(format!("Could not review Electron settings: {error}"), window, cx),
                };
                ws.review_new_files(
                    NewFiles { directory: native.clone(), files, source: "Import Electron settings", exclusive: false },
                    move |summary, ws, window, cx| {
                        if !summary.failed {
                            if let Err(error) = forge_sys::user_state::finish_legacy_import(&native) {
                                return ws.alert(format!("Could not finish the import: {error}"), window, cx);
                            }
                        }
                        ws.refresh_recent(cx);
                        if ws.root.is_none() && summary.written > 0 {
                            ws.reopen_last(window, cx);
                        }
                    },
                    window,
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn offer_legacy_project_import(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if !root.join(forge_core::settings::LEGACY_PROJECT_DIR).is_dir() {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let project = root.clone();
            let plan = cx
                .background_executor()
                .spawn(async move { forge_sys::project_dir::plan_legacy_project_import(&project) })
                .await;
            this.update_in(cx, |ws, window, cx| {
                if ws.root.as_ref() != Some(&root) {
                    return;
                }
                let files = match plan {
                    Ok(Some(files)) => files.into_iter().map(|file| (file.relative, file.contents)).collect(),
                    Ok(None) => return,
                    Err(error) => {
                        return ws.alert(format!("Could not review legacy project settings: {error}"), window, cx)
                    }
                };
                ws.review_new_files(
                    NewFiles {
                        directory: root.join(forge_core::settings::PROJECT_DIR),
                        files,
                        source: "Import project settings",
                        exclusive: true,
                    },
                    move |summary, ws, window, cx| {
                        if summary.written > 0 {
                            ws.load_project_settings(&root, window, cx);
                            ws.load_run_file(&root, window, cx);
                            ws.load_boxes(&root, window, cx);
                            ws.rebuild_graph(cx);
                            ws.refresh_python_env(cx);
                            ws.file_tree.update(cx, |p, cx| p.refresh_all(cx));
                        }
                    },
                    window,
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    fn show_change_review(&mut self, cx: &mut Context<Self>) {
        if let Some(change) = self.generated_changes.front() {
            self.change_review.update(cx, |p, cx| {
                p.review(change.title.clone(), &change.before, &change.after, change.apply, change.reject, cx);
            });
        } else {
            self.change_review.update(cx, |p, cx| {
                p.text(String::new(), Vec::new(), cx);
                p.open = false;
            });
        }
        cx.notify();
    }

    pub(super) fn clear_change_reviews(&mut self, cx: &mut Context<Self>) {
        if !self.generated_changes.is_empty() {
            self.generated_changes.clear();
            self.show_change_review(cx);
        }
    }

    pub(super) fn on_change_review(
        &mut self,
        _: &Entity<GitInspect>,
        event: &InspectEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let apply = match event {
            InspectEvent::ApplyReviewed => true,
            InspectEvent::Close => false,
            _ => return,
        };
        if let Some(change) = self.generated_changes.pop_front() {
            (change.decision)(apply, self, window, cx);
            self.show_change_review(cx);
        }
    }

    pub(super) fn review_project_export(
        &mut self,
        root: PathBuf,
        relative: String,
        after: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = match forge_sys::connect::project_path(&root, &relative) {
            Ok(path) => path,
            Err(error) => return self.alert(error.to_string(), window, cx),
        };
        let mut change = match self.generate_text_change(target, cx) {
            Ok(change) => change,
            Err(error) => return self.alert(format!("Could not preview {relative}: {error}"), window, cx),
        };
        change.updated = after;
        self.review_text_change(
            change,
            "Pipeline export",
            Some((root, relative)),
            |ws, path, window, cx| ws.open_path(path, window, cx),
            cx,
        );
    }

    pub(super) fn undo_generated_write(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.tabs.active() else { return };
        if self.is_dirty(id, cx) {
            return self.alert("Save your current edits before undoing the generated write.", window, cx);
        }
        let path = PathBuf::from(self.docs[&id].read(cx).doc.path());
        let history = forge_sys::generated_history::History::new(super::app_state::state(cx).user.dir());
        match history.undo(&path) {
            Ok(Some(bytes)) => {
                let (text, encoding) = forge_sys::text_file::decode(&bytes);
                self.sync_reviewed_doc(id, &path, &text, encoding, window, cx);
            }
            Ok(None) => {
                self.close_tab_now(id, None, window, cx);
                self.after_save(&path.to_string_lossy(), window, cx);
            }
            Err(error) => return self.alert(error.to_string(), window, cx),
        }
        self.file_tree.update(cx, |p, cx| p.refresh_all(cx));
        self.status_note("Undid the generated write.", cx);
    }

    pub(super) fn apply_reviewed_text(
        &mut self,
        change: TextChange,
        source: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(String, Encoding), String> {
        let id = change.check(self, cx)?;
        let TextChange { path, disk, original, updated, .. } = change;
        let encoding = disk.as_deref().map(|bytes| forge_sys::text_file::decode(bytes).1).unwrap_or(Encoding::Utf8);
        let updated =
            if original.contains("\r\n") { updated.replace("\r\n", "\n").replace('\n', "\r\n") } else { updated };
        if let Some(id) = id.filter(|id| self.is_dirty(*id, cx)) {
            let encoding = self.docs[&id].read(cx).doc.encoding;
            self.update_reviewed_doc(id, &updated, encoding, false, cx);
            self.remember_reviewed_edit(id, disk, source, cx);
            return Ok((updated, encoding));
        }
        if encoding == Encoding::Latin1 && updated.chars().any(|character| character as u32 > 255) {
            return Err(
                "The generated text does not fit the file's encoding. Save the target as UTF-8 and try again.".into()
            );
        }
        let bytes = forge_sys::text_file::encode(&updated, encoding);
        self.write_reviewed(&path, disk.as_deref(), &bytes, source, cx).map_err(|error| error.to_string())?;
        let actual = forge_sys::text_file::decode_as(&bytes, encoding);
        if let Some(id) = id {
            self.sync_reviewed_doc(id, &path, &actual, encoding, window, cx);
        } else {
            self.after_save(&path.to_string_lossy(), window, cx);
        }
        Ok((actual, encoding))
    }

    pub(super) fn sync_reviewed_doc(
        &mut self,
        id: DocId,
        path: &Path,
        text: &str,
        encoding: Encoding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_reviewed_doc(id, text, encoding, true, cx);
        self.after_save(&path.to_string_lossy(), window, cx);
    }

    pub(super) fn update_reviewed_doc(
        &mut self,
        id: DocId,
        text: &str,
        encoding: Encoding,
        saved: bool,
        cx: &mut Context<Self>,
    ) {
        self.docs[&id].update(cx, |doc, cx| {
            doc.edit(None, cx, |doc| {
                doc.encoding = encoding;
                doc.set_eol(if text.contains("\r\n") { Eol::CrLf } else { Eol::Lf });
                if doc.text() != text {
                    let Doc { buffer, history, .. } = doc;
                    history.transact(buffer, EditKind::Other, Pos::default(), |buffer| {
                        buffer.set_text(text);
                        Pos::default()
                    });
                }
            });
            if saved {
                doc.doc.mark_saved();
            }
            cx.emit(DocumentEvent::Changed);
            cx.notify();
        });
    }

    pub(super) fn write_reviewed(
        &self,
        path: &Path,
        before: Option<&[u8]>,
        after: &[u8],
        source: &str,
        cx: &gpui_kit::App,
    ) -> std::io::Result<()> {
        let history = forge_sys::generated_history::History::new(super::app_state::state(cx).user.dir());
        history.write(path, before, after, source)
    }
}
