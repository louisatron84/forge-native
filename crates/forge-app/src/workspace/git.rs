//! Source control orchestration. Replaces the Git sections of `main.ts`.
use super::Workspace;
use crate::palette::Choice;
use crate::panels::git::{GitPanel, GitPanelEvent, Repo};
use crate::panels::git_inspect::{GitInspect, InspectEvent};
use forge_core::git::{ChangeKind, FileChange};
use forge_core::history::{conflict_near, find_conflicts, resolve_conflict, Resolution};
use forge_sys::git::{self, GitResult, SyncAction};
use gpui_kit::{Context, Entity, PromptLevel, Task, Window};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Default)]
pub(crate) struct GitState {
    generation: u64,
    poll: Option<Task<()>>,
    busy: bool,
    pub inspect_path: Option<PathBuf>,
    pub(super) diff: Option<(FileChange, bool)>,
}

impl Workspace {
    pub(crate) fn reset_git(&mut self) {
        self.git = GitState { generation: self.git.generation.wrapping_add(1), ..Default::default() };
    }

    pub(crate) fn refresh_git(&mut self, cx: &mut Context<Self>) {
        self.git.poll = None;
        let Some(folder) = self.root.clone() else { return };
        let generation = self.git.generation;
        self.git.poll = Some(cx.spawn(async move |this, cx| loop {
            let path = folder.to_string_lossy().into_owned();
            let repo = cx
                .background_executor()
                .spawn(async move { git::repo_status(&path).map(|(root, status)| Repo { root, status }) })
                .await;
            if this
                .update(cx, |ws, cx| {
                    if ws.git.generation != generation || ws.root.as_ref() != Some(&folder) {
                        return false;
                    }
                    let changed =
                        ws.git_panel.read(cx).repo().is_some() && ws.git_panel.read(cx).repo() != repo.as_ref();
                    ws.git_panel.update(cx, |p, cx| p.set_status(repo, cx));
                    if ws.project_graph.read(cx).detail_mode == 3 {
                        ws.refresh_map_markers(cx);
                    }
                    if changed {
                        ws.reload_clean_git_docs(cx);
                        ws.file_tree.update(cx, |t, cx| t.refresh_all(cx));
                    }
                    cx.notify();
                    true
                })
                .ok()
                != Some(true)
            {
                break;
            }
            cx.background_executor().timer(Duration::from_secs(5)).await;
        }));
    }

    fn git_root(&self, cx: &gpui_kit::App) -> Option<String> {
        self.git_panel.read(cx).repo().map(|r| r.root.clone())
    }

    pub(crate) fn on_git_event(
        &mut self,
        _: &Entity<GitPanel>,
        event: &GitPanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            GitPanelEvent::Refresh => self.refresh_git(cx),
            GitPanelEvent::Init => {
                if let Some(root) = self.root.clone() {
                    let root = root.to_string_lossy().into_owned();
                    self.git_job("Initialize repository", move |_| git::init(&root), false, window, cx);
                }
            }
            GitPanelEvent::OpenFile(path) => self.open_path(path.clone(), window, cx),
            GitPanelEvent::OpenDiff(file, staged) => self.git_diff(file.clone(), *staged, window, cx),
            GitPanelEvent::Stage(paths) | GitPanelEvent::Unstage(paths) => {
                let paths = paths.clone();
                let unstage = matches!(event, GitPanelEvent::Unstage(_));
                self.git_job(
                    if unstage { "Unstage" } else { "Stage" },
                    move |root| {
                        let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
                        if unstage {
                            git::unstage(root, &paths)
                        } else {
                            git::stage(root, &paths)
                        }
                    },
                    false,
                    window,
                    cx,
                );
            }
            GitPanelEvent::Sync(action) => {
                let action = *action;
                self.git_job("Synchronize", move |root| git::sync(root, action), false, window, cx);
            }
            GitPanelEvent::Commit { message, amend } => self.git_commit(message.clone(), *amend, window, cx),
            GitPanelEvent::WantLastMessage => {
                if let Some(root) = self.git_root(cx) {
                    cx.spawn_in(window, async move |this, cx| {
                        let text = cx.background_executor().spawn(async move { git::last_commit_message(&root) }).await;
                        this.update_in(cx, |ws, window, cx| {
                            ws.git_panel.update(cx, |p, cx| p.set_last_message(text, window, cx))
                        })
                        .ok();
                    })
                    .detach();
                }
            }
            GitPanelEvent::PickBranch => self.git_branches(window, cx),
            GitPanelEvent::Discard(files) => {
                let files = files.clone();
                let answer = window.prompt(PromptLevel::Warning, "Discard these changes?", Some("Tracked files will be restored; new files go to the Trash. This cannot be undone in the editor."), &["Discard", "Cancel"], cx);
                cx.spawn_in(window, async move |this, cx| {
                    if answer.await != Ok(0) {
                        return;
                    }
                    this.update_in(cx, |ws, window, cx| {
                        ws.git_job(
                            "Discard",
                            move |root| {
                                for file in &files {
                                    if file.kind == ChangeKind::Untracked {
                                        if let Err(e) =
                                            forge_sys::files::trash_entry(&PathBuf::from(root).join(&file.path))
                                        {
                                            return GitResult { error: Some(e.to_string()), ..Default::default() };
                                        }
                                    } else {
                                        let result = git::discard(root, &[&file.path]);
                                        if !result.ok {
                                            return result;
                                        }
                                    }
                                }
                                GitResult { ok: true, ..Default::default() }
                            },
                            false,
                            window,
                            cx,
                        )
                    })
                    .ok();
                })
                .detach();
            }
        }
    }

    fn git_job(
        &mut self,
        label: &str,
        action: impl FnOnce(&str) -> GitResult + Send + 'static,
        committed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.git.busy {
            return;
        }
        let Some(folder) = self.root.clone() else { return };
        let root = self.git_root(cx).unwrap_or_else(|| folder.to_string_lossy().into_owned());
        self.git.busy = true;
        let generation = self.git.generation;
        self.git_panel.update(cx, |p, cx| p.set_busy(label, cx));
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move { action(&root) }).await;
            this.update_in(cx, |ws, window, cx| {
                if ws.git.generation != generation || ws.root.as_ref() != Some(&folder) {
                    return;
                }
                ws.git.busy = false;
                ws.git_panel.update(cx, |p, cx| {
                    p.set_busy("", cx);
                    if committed && result.ok {
                        p.committed(window, cx);
                    }
                });
                if let Some(error) = result.error {
                    ws.alert(error, window, cx);
                } else if !result.output.is_empty() {
                    ws.status_note(result.output, cx);
                }
                ws.reload_clean_git_docs(cx);
                ws.file_tree.update(cx, |t, cx| t.refresh_all(cx));
                ws.refresh_git(cx);
                if let Some((file, staged)) = ws.git.diff.clone().filter(|_| ws.git_inspect.read(cx).open) {
                    ws.git_diff(file, staged, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn reload_clean_git_docs(&mut self, cx: &mut Context<Self>) {
        let docs = self
            .docs
            .iter()
            .filter_map(|(id, doc)| {
                let d = &doc.read(cx).doc;
                (!d.dirty()).then(|| (*id, d.path().to_owned(), d.version))
            })
            .collect::<Vec<_>>();
        let generation = self.git.generation;
        cx.spawn(async move |this, cx| {
            let texts = cx
                .background_executor()
                .spawn(async move {
                    docs.into_iter()
                        .filter_map(|(id, path, version)| {
                            if let Ok(forge_sys::text_file::TextFile::Text { text, .. }) =
                                forge_sys::text_file::read_text_file(std::path::Path::new(&path), false, None)
                            {
                                Some((id, path, version, text))
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            this.update(cx, |ws, cx| {
                if ws.git.generation != generation {
                    return;
                }
                for (id, path, version, text) in texts {
                    let Some(doc) = ws.docs.get(&id).cloned() else { continue };
                    let d = &doc.read(cx).doc;
                    if d.dirty() || d.version != version || d.path() != path || d.text() == text {
                        continue;
                    }
                    doc.update(cx, |d, cx| {
                        d.edit(None, cx, |d| {
                            d.buffer.set_text(&text);
                            d.mark_saved();
                        })
                    });
                }
            })
            .ok();
        })
        .detach();
    }

    fn git_commit(&mut self, message: String, amend: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.git_panel.read(cx).repo() else { return };
        if !repo.status.conflicts.is_empty() {
            return self.alert("Resolve and stage merge conflicts first.", window, cx);
        }
        if repo.status.staged.is_empty() && !repo.status.unstaged.is_empty() {
            let answer = window.prompt(
                PromptLevel::Warning,
                "No staged changes. Stage all changes and commit?",
                None,
                &["Stage All and Commit", "Cancel"],
                cx,
            );
            cx.spawn_in(window, async move |this, cx| {
                if answer.await != Ok(0) {
                    return;
                }
                this.update_in(cx, |ws, window, cx| {
                    ws.git_job("Commit", move |root| git::commit(root, &message, amend, true), true, window, cx)
                })
                .ok();
            })
            .detach();
        } else {
            self.git_job("Commit", move |root| git::commit(root, &message, amend, false), true, window, cx);
        }
    }

    fn git_diff(&mut self, file: FileChange, staged: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.git_root(cx) else { return };
        self.git.inspect_path = Some(PathBuf::from(&root).join(&file.path));
        self.git.diff = Some((file.clone(), staged));
        let folder = self.root.clone();
        let path = file.path.clone();
        self.reveal_panel("editor", cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if file.kind == ChangeKind::Untracked {
                        return git::untracked_diff(&root, &file.path);
                    }
                    git::file_diff(&root, &file.path, staged)
                })
                .await;
            this.update_in(cx, |ws, window, cx| {
                if ws.root != folder || ws.git.diff.as_ref().map(|(f, s)| (&f.path, *s)) != Some((&path, staged)) {
                    return;
                }
                if let Some(e) = result.error {
                    ws.alert(e, window, cx);
                    return;
                }
                ws.git_inspect.update(cx, |p, cx| {
                    p.diff(
                        format!("{} — {}", path, if staged { "Staged" } else { "Changes" }),
                        &result.output,
                        staged,
                        false,
                        cx,
                    )
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn on_git_inspect(
        &mut self,
        _: &Entity<GitInspect>,
        event: &InspectEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InspectEvent::ApplyReviewed => {}
            InspectEvent::Close => {
                self.git.diff = None;
                self.git_inspect.update(cx, |p, _| p.open = false);
                cx.notify();
            }
            InspectEvent::OpenLine(line) => {
                if let Some(path) = self.git.inspect_path.clone() {
                    self.open_path_at(path, Some(*line), window, cx);
                }
            }
            InspectEvent::Patch { patch, cached, reverse } => {
                let (patch, cached, reverse) = (patch.clone(), *cached, *reverse);
                if !cached {
                    let answer =
                        window.prompt(PromptLevel::Warning, "Discard this hunk?", None, &["Discard", "Cancel"], cx);
                    cx.spawn_in(window, async move |this, cx| {
                        if answer.await == Ok(0) {
                            this.update_in(cx, |ws, window, cx| {
                                ws.git_job(
                                    "Discard hunk",
                                    move |root| git::apply_patch(root, &patch, cached, reverse),
                                    false,
                                    window,
                                    cx,
                                )
                            })
                            .ok();
                        }
                    })
                    .detach();
                } else {
                    self.git_job(
                        "Apply hunk",
                        move |root| git::apply_patch(root, &patch, cached, reverse),
                        false,
                        window,
                        cx,
                    );
                }
            }
            InspectEvent::Commit(sha) => self.git_commit_details(sha.clone(), window, cx),
        }
    }

    fn git_branches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.git_root(cx) else { return };
        cx.spawn_in(window, async move |this, cx| {
            let branches = cx.background_executor().spawn(async move { git::branches(&root) }).await;
            this.update_in(cx, |ws, window, cx| {
                let mut choices = vec![Choice::new("Create branch…", |ws, window, cx| {
                    ws.palette.update(cx, |p, cx| {
                        p.prompt(
                            "New branch name",
                            "",
                            "Create",
                            |ws, name, window, cx| {
                                ws.git_job(
                                    "Create branch",
                                    move |root| git::switch_branch(root, &name, true, false),
                                    false,
                                    window,
                                    cx,
                                )
                            },
                            window,
                            cx,
                        )
                    });
                })];
                choices.extend(branches.into_iter().map(|b| {
                    Choice::new(format!("{}{}", if b.current { "✓ " } else { "" }, b.name), move |ws, window, cx| {
                        let b = b.clone();
                        ws.git_job(
                            "Switch branch",
                            move |root| git::switch_branch(root, &b.name, false, b.remote),
                            false,
                            window,
                            cx,
                        );
                    })
                }));
                ws.choose("Switch branch", choices, window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn git_history(&mut self, file: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.git_root(cx) else { return };
        self.git.diff = None;
        self.git.inspect_path = None;
        self.reveal_panel("editor", cx);
        cx.spawn_in(window, async move |this, cx| {
            let title =
                file.as_ref().map(|f| format!("History: {f}")).unwrap_or_else(|| "Git history (all branches)".into());
            let result = cx
                .background_executor()
                .spawn(async move { git::log(&root, file.is_none(), file.as_deref(), 2000, 0) })
                .await;
            this.update_in(cx, |ws, window, cx| match result {
                Ok(commits) => {
                    ws.git_inspect.update(cx, |p, cx| p.history(title, &commits, cx));
                    cx.notify();
                }
                Err(e) => ws.alert(e, window, cx),
            })
            .ok();
        })
        .detach();
    }

    fn git_commit_details(&mut self, sha: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.git_root(cx) else { return };
        cx.spawn_in(window, async move |this, cx| {
            let r = root.clone();
            let s = sha.clone();
            let result = cx.background_executor().spawn(async move { git::commit_details(&r, &s) }).await;
            this.update_in(cx, |ws, window, cx| match result {
                Err(e) => ws.alert(e, window, cx),
                Ok((message, files)) => {
                    let choices = files
                        .into_iter()
                        .map(|f| {
                            let root = root.clone();
                            let sha = sha.clone();
                            Choice::new(format!("{} {}", f.status, f.path), move |ws, window, cx| {
                                let (root, sha, path) = (root.clone(), sha.clone(), f.path.clone());
                                ws.git.diff = None;
                                ws.git.inspect_path = Some(PathBuf::from(&root).join(&path));
                                cx.spawn_in(window, async move |this, cx| {
                                    let title = format!("{} — {}", path, &sha[..sha.len().min(8)]);
                                    let result = cx
                                        .background_executor()
                                        .spawn(async move { git::commit_file_diff(&root, &sha, &path) })
                                        .await;
                                    this.update_in(cx, |ws, window, cx| {
                                        if let Some(e) = result.error {
                                            ws.alert(e, window, cx);
                                        } else {
                                            ws.git_inspect
                                                .update(cx, |p, cx| p.diff(title, &result.output, false, true, cx));
                                            cx.notify();
                                        }
                                    })
                                    .ok();
                                })
                                .detach();
                            })
                        })
                        .collect();
                    ws.choose(message, choices, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn git_available(&self, id: &str, cx: &gpui_kit::App) -> bool {
        match id {
            "git.show" => true,
            "git.init" => self.root.is_some() && !self.git.busy,
            "git.acceptCurrent" | "git.acceptIncoming" | "git.acceptBoth" | "git.nextConflict" | "git.prevConflict" => {
                self.tabs.active().is_some()
            }
            _ => self.git_root(cx).is_some() && !self.git.busy,
        }
    }

    pub(crate) fn git_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            "git.show" => {
                self.reveal_panel("git", cx);
                self.refresh_git(cx);
            }
            "git.init" => self.on_git_event(&self.git_panel.clone(), &GitPanelEvent::Init, window, cx),
            "git.commit" => {
                self.reveal_panel("git", cx);
                self.git_panel.update(cx, |p, cx| p.focus_message(window, cx));
            }
            "git.branch" => self.git_branches(window, cx),
            "git.fetch" | "git.pull" | "git.push" => {
                let action = match id {
                    "git.fetch" => SyncAction::Fetch,
                    "git.pull" => SyncAction::Pull,
                    _ => SyncAction::Push,
                };
                self.on_git_event(&self.git_panel.clone(), &GitPanelEvent::Sync(action), window, cx);
            }
            "git.stageAll" => self.git_job("Stage all", |root| git::stage(root, &["."]), false, window, cx),
            "git.history" => self.git_history(None, window, cx),
            "git.fileHistory" | "git.diffFile" | "git.toggleBlame" => {
                let Some(doc) = self.active_doc() else { return };
                let path = doc.read(cx).doc.path().to_owned();
                let text = doc.read(cx).doc.text();
                let Some(root) = self.git_root(cx) else { return };
                let Ok(relative) = std::path::Path::new(&path).strip_prefix(&root) else { return };
                let relative = relative.to_string_lossy().replace('\\', "/");
                if id == "git.fileHistory" {
                    self.git_history(Some(relative), window, cx);
                } else if id == "git.diffFile" {
                    let repo = self.git_panel.read(cx).repo().cloned();
                    if let Some(repo) = repo {
                        if let Some(f) = repo.status.unstaged.iter().find(|f| f.path == relative) {
                            self.git_diff(f.clone(), false, window, cx);
                        } else if let Some(f) = repo.status.staged.iter().find(|f| f.path == relative) {
                            self.git_diff(f.clone(), true, window, cx);
                        } else {
                            self.status_note("No changes in this file.", cx);
                        }
                    }
                } else {
                    self.git.diff = None;
                    self.git.inspect_path = Some(PathBuf::from(&path));
                    cx.spawn_in(window, async move |this, cx| {
                        let contents = text.clone();
                        let result = cx
                            .background_executor()
                            .spawn(async move { git::blame(&root, &relative, Some(&contents)) })
                            .await;
                        this.update_in(cx, |ws, window, cx| match result {
                            Err(e) => ws.alert(e, window, cx),
                            Ok(blame) => {
                                let lines = text
                                    .lines()
                                    .enumerate()
                                    .map(|(i, text)| {
                                        let b = blame.get(i);
                                        format!(
                                            "{:>5}  {:8}  {:20}  {}  | {}",
                                            i + 1,
                                            b.map(|b| &b.sha[..b.sha.len().min(8)]).unwrap_or(""),
                                            b.map(|b| b.author.as_str()).unwrap_or(""),
                                            b.map(|b| b.summary.as_str()).unwrap_or(""),
                                            text
                                        )
                                    })
                                    .collect();
                                ws.git_inspect.update(cx, |p, cx| p.text(format!("Blame: {path}"), lines, cx));
                                cx.notify();
                            }
                        })
                        .ok();
                    })
                    .detach();
                }
            }
            "git.acceptCurrent" | "git.acceptIncoming" | "git.acceptBoth" | "git.nextConflict" | "git.prevConflict" => {
                let ed = self.editors[self.tabs.active_group.min(1)].clone();
                let Some(doc) = ed.read(cx).document().cloned() else { return };
                let lines = doc.read(cx).doc.buffer.lines().to_vec();
                let conflicts = find_conflicts(&lines);
                let line = ed.read(cx).cursor().line;
                if id.ends_with("Conflict") {
                    let conflict = if id == "git.prevConflict" {
                        conflicts.iter().rev().find(|c| c.start < line).or_else(|| conflicts.last())
                    } else {
                        conflicts.iter().find(|c| c.start > line).or_else(|| conflicts.first())
                    };
                    if let Some(c) = conflict {
                        ed.update(cx, |e, cx| e.go_to(c.start, 0, cx));
                    }
                } else if let Some(conflict) = conflict_near(&conflicts, line) {
                    let resolution = match id {
                        "git.acceptCurrent" => Resolution::Current,
                        "git.acceptIncoming" => Resolution::Incoming,
                        _ => Resolution::Both,
                    };
                    let replacement = resolve_conflict(&lines, conflict, resolution).join("\n");
                    let (start, end) = (conflict.start, conflict.end);
                    ed.update(cx, |e, cx| {
                        e.run_edit(cx, |b| {
                            let from = forge_text::Pos::new(start, 0);
                            let to = if end + 1 < b.line_count() { forge_text::Pos::new(end + 1, 0) } else { b.end() };
                            let replacement =
                                if end + 1 < b.line_count() { format!("{replacement}\n") } else { replacement };
                            b.delete(from, to);
                            b.insert(from, &replacement)
                        })
                    });
                }
            }
            _ => {}
        }
    }
}
