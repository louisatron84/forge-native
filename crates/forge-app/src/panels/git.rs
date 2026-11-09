//! Source Control (the Git panel): the branch with fetch / pull / push, a
//! commit box, and the changed files in three groups (merge conflicts,
//! staged, changes). Click a file to see its diff; hover a row for open /
//! stage / unstage / discard. The workspace runs the git commands.
//! Replaces `src/renderer/gitView.ts`.

use crate::theme::{ForgeTheme, Theme};
use forge_core::git::{ChangeKind, FileChange, RepoStatus};
use forge_sys::git::SyncAction;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::{
    div, prelude::FluentBuilder, px, AnyElement, AppContext, Context, Entity, EventEmitter, FontWeight, Hsla,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window,
};
use std::path::PathBuf;

/// A repository: its top folder and status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    pub root: String,
    pub status: RepoStatus,
}

impl Repo {
    /// A repo-relative path (with /) as an absolute one.
    pub fn abs(&self, path: &str) -> PathBuf {
        let sep = if self.root.contains('\\') { "\\" } else { "/" };
        PathBuf::from(format!("{}{sep}{}", self.root.trim_end_matches(['\\', '/']), path.replace('/', sep)))
    }
}

/// What the panel asks the workspace to do.
pub enum GitPanelEvent {
    /// Show a file's diff (`staged`: what's in the index against the last commit).
    OpenDiff(FileChange, bool),
    OpenFile(PathBuf),
    Stage(Vec<String>),
    Unstage(Vec<String>),
    /// Discard changes (and delete new files), after asking.
    Discard(Vec<FileChange>),
    Sync(SyncAction),
    Commit {
        message: String,
        amend: bool,
    },
    /// Amend was ticked with an empty message: fill in the last one.
    WantLastMessage,
    PickBranch,
    Refresh,
    Init,
}

impl EventEmitter<GitPanelEvent> for GitPanel {}

pub struct GitPanel {
    has_folder: bool,
    loaded: bool,
    repo: Option<Repo>,
    busy: String,
    message: Entity<TextareaState>,
    amend: bool,
    _sub: Subscription,
}

fn kind_color(t: &Theme, k: ChangeKind) -> Hsla {
    match k {
        ChangeKind::Added | ChangeKind::Untracked | ChangeKind::Copied => t.ok,
        ChangeKind::Deleted | ChangeKind::Conflict => t.err,
        _ => t.git_modified,
    }
}

impl GitPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let message = cx.new(|cx| TextareaState::new(window, cx));
        let sub = cx.subscribe_in(&message, window, |this, _, e: &InputEvent, _, cx| match e {
            InputEvent::PressEnter { secondary: true, .. } => this.commit(cx),
            InputEvent::Change => cx.notify(),
            _ => {}
        });
        Self { has_folder: false, loaded: false, repo: None, busy: String::new(), message, amend: false, _sub: sub }
    }

    pub fn repo(&self) -> Option<&Repo> {
        self.repo.as_ref()
    }

    /// A folder was opened or closed (the status follows).
    pub fn set_folder(&mut self, has_folder: bool, cx: &mut Context<Self>) {
        self.busy.clear();
        self.has_folder = has_folder;
        self.loaded = false;
        self.repo = None;
        cx.notify();
    }

    pub fn set_status(&mut self, repo: Option<Repo>, cx: &mut Context<Self>) {
        if !self.loaded || repo != self.repo {
            self.repo = repo;
            self.loaded = true;
            cx.notify();
        }
    }

    pub fn set_busy(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.busy = text.into();
        cx.notify();
    }

    pub fn focus_message(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.message.update(cx, |s, cx| s.focus(window, cx));
    }

    fn message_text(&self, cx: &gpui_kit::App) -> String {
        self.message.read(cx).value().trim().to_owned()
    }

    /// After a commit: empty the box and untick Amend.
    pub fn committed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.amend = false;
        self.message.update(cx, |s, cx| s.set_value("", window, cx));
        cx.notify();
    }

    /// The last commit's message (for Amend), unless something was typed meanwhile.
    pub fn set_last_message(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.amend && self.message_text(cx).is_empty() {
            self.message.update(cx, |s, cx| s.set_value(text, window, cx));
        }
    }

    /// Commit what's staged (the workspace offers to stage everything if nothing is).
    pub fn commit(&mut self, cx: &mut Context<Self>) {
        let message = self.message_text(cx);
        if !self.can_commit(cx) {
            return;
        }
        cx.emit(GitPanelEvent::Commit { message, amend: self.amend });
    }

    fn can_commit(&self, cx: &gpui_kit::App) -> bool {
        let Some(r) = &self.repo else { return false };
        let has_changes = !r.status.staged.is_empty() || !r.status.unstaged.is_empty() || self.amend;
        !self.message_text(cx).is_empty() && has_changes && self.busy.is_empty() && r.status.conflicts.is_empty()
    }

    #[allow(clippy::too_many_arguments)] // A section's data and its render context.
    fn section(
        &self,
        id: &'static str,
        title: &str,
        list: &[FileChange],
        staged: bool,
        actions: Vec<(&'static str, &'static str, GitPanelEvent)>,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut head = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(8.0))
            .pt(px(8.0))
            .pb(px(2.0))
            .text_size(px(11.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.fg_muted)
            .child(title.to_uppercase())
            .child(div().px(px(5.0)).rounded(px(6.0)).bg(t.shade_strong).child(list.len().to_string()));
        if !list.is_empty() {
            let mut buttons = div().ml_auto().flex().gap(px(2.0));
            for (i, (label, tip, event)) in actions.into_iter().enumerate() {
                let event = std::cell::RefCell::new(Some(event));
                buttons =
                    buttons.child(mini_button(SharedString::from(format!("{id}-act-{i}")), label, tip, t).on_click(
                        cx.listener(move |_, _, _, cx| {
                            if let Some(e) = event.borrow_mut().take() {
                                cx.emit(e);
                            }
                        }),
                    ));
            }
            head = head.child(buttons);
        }
        let rows = list.iter().enumerate().map(|(i, change)| self.file_row(id, i, change, staged, t, cx));
        div().flex().flex_col().child(head).children(rows).into_any_element()
    }

    fn file_row(
        &self,
        section: &'static str,
        i: usize,
        change: &FileChange,
        staged: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(repo) = &self.repo else { return div().into_any_element() };
        let (name, dir) = match change.path.rsplit_once('/') {
            Some((d, n)) => (n.to_owned(), d.to_owned()),
            None => (change.path.clone(), String::new()),
        };
        let dir = change.from.as_ref().map(|f| format!("← {f}")).unwrap_or(dir);
        let group = SharedString::from(format!("git-row-{section}-{i}"));
        let abs = repo.abs(&change.path);
        let c = change.clone();
        let mut buttons = div().flex().gap(px(2.0)).opacity(0.0).group_hover(group.clone(), |s| s.opacity(1.0));
        let bid = |what: &str| SharedString::from(format!("git-{section}-{i}-{what}"));
        if change.kind != ChangeKind::Deleted {
            let abs = abs.clone();
            buttons = buttons.child(mini_button(bid("open"), "📄", "Open file", t).on_click(cx.listener(
                move |_, _, _, cx| {
                    cx.stop_propagation();
                    cx.emit(GitPanelEvent::OpenFile(abs.clone()));
                },
            )));
        }
        if staged {
            let p = change.path.clone();
            buttons = buttons.child(mini_button(bid("unstage"), "−", "Unstage", t).on_click(cx.listener(
                move |_, _, _, cx| {
                    cx.stop_propagation();
                    cx.emit(GitPanelEvent::Unstage(vec![p.clone()]));
                },
            )));
        } else {
            let (c1, p) = (change.clone(), change.path.clone());
            let discard_tip =
                if change.kind == ChangeKind::Untracked { "Delete this new file" } else { "Discard changes" };
            let stage_tip = if change.kind == ChangeKind::Conflict { "Mark resolved (stage)" } else { "Stage" };
            buttons = buttons
                .child(mini_button(bid("discard"), "↶", discard_tip, t).on_click(cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    cx.emit(GitPanelEvent::Discard(vec![c1.clone()]));
                })))
                .child(mini_button(bid("stage"), "+", stage_tip, t).on_click(cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    cx.emit(GitPanelEvent::Stage(vec![p.clone()]));
                })));
        }
        div()
            .id(SharedString::from(format!("git-file-{section}-{i}")))
            .group(group)
            .flex()
            .items_center()
            .gap(px(6.0))
            .h(px(22.0))
            .pl(px(14.0))
            .pr(px(8.0))
            .whitespace_nowrap()
            .cursor_pointer()
            .hover(|s| s.bg(t.bg_hover))
            .on_click(cx.listener(move |_, _, _, cx| {
                if c.kind == ChangeKind::Conflict {
                    cx.emit(GitPanelEvent::OpenFile(abs.clone()));
                } else {
                    cx.emit(GitPanelEvent::OpenDiff(c.clone(), staged));
                }
            }))
            .child(div().flex_none().child(name))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_size(px(11.0))
                    .text_color(t.fg_muted)
                    .child(dir),
            )
            .child(buttons)
            .child(
                div()
                    .w(px(12.0))
                    .flex_none()
                    .text_center()
                    .font_family(crate::theme::CODE_FONT)
                    .text_size(px(11.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(kind_color(t, change.kind))
                    .child(change.kind.letter()),
            )
            .into_any_element()
    }
}

fn button(id: impl Into<SharedString>, label: impl Into<SharedString>, t: &Theme) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id.into())
        .px(px(8.0))
        .py(px(2.0))
        .border_1()
        .border_color(t.border)
        .rounded(px(2.0))
        .text_size(px(12.0))
        .cursor_pointer()
        .hover(|s| s.bg(t.bg_hover))
        .child(label.into())
}

fn mini_button(
    id: SharedString,
    label: &'static str,
    tip: &'static str,
    t: &Theme,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let _ = tip;
    div()
        .id(id)
        .px(px(4.0))
        .rounded(px(2.0))
        .text_size(px(12.0))
        .text_color(t.fg_muted)
        .cursor_pointer()
        .hover(|s| s.bg(t.bg_active).text_color(t.fg))
        .child(label)
}

fn note(t: &Theme, text: impl Into<SharedString>) -> gpui_kit::Div {
    div().px(px(10.0)).py(px(8.0)).text_color(t.fg_muted).child(text.into())
}

impl Render for GitPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let root =
            div().id("git-panel").size_full().overflow_y_scroll().text_size(px(13.0)).text_color(t.fg).pb(px(8.0));
        if !self.has_folder {
            return root.child(note(&t, "Open a folder to use git."));
        }
        if !self.loaded {
            return root.child(note(&t, "Reading git status…"));
        }
        let Some(repo) = self.repo.clone() else {
            return root.child(
                note(&t, "This folder isn't a git repository.").child(
                    div().pt(px(8.0)).child(
                        button("git-init", "Initialize Repository", &t)
                            .bg(t.accent)
                            .text_color(t.on_heat)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(GitPanelEvent::Init))),
                    ),
                ),
            );
        };
        let st = &repo.status;
        let name =
            if st.detached { "detached HEAD".to_owned() } else { st.branch.clone().unwrap_or_else(|| "?".into()) };
        let ab = [(st.ahead > 0).then(|| format!("↑{}", st.ahead)), (st.behind > 0).then(|| format!("↓{}", st.behind))]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        let head = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(4.0))
            .px(px(8.0))
            .pt(px(8.0))
            .child(
                div()
                    .id("git-branch")
                    .flex_1()
                    .min_w(px(80.0))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(2.0))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.bg_hover))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(GitPanelEvent::PickBranch)))
                    .child(format!("⎇ {name}{}", if ab.is_empty() { String::new() } else { format!("  {ab}") })),
            )
            .child(button("git-refresh", "⟳", &t).on_click(cx.listener(|_, _, _, cx| cx.emit(GitPanelEvent::Refresh))))
            .child(
                button("git-fetch", "Fetch", &t)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(GitPanelEvent::Sync(SyncAction::Fetch)))),
            )
            .child(
                button("git-pull", if st.behind > 0 { format!("Pull ↓{}", st.behind) } else { "Pull".into() }, &t)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(GitPanelEvent::Sync(SyncAction::Pull)))),
            )
            .child(
                button("git-push", if st.ahead > 0 { format!("Push ↑{}", st.ahead) } else { "Push".into() }, &t)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(GitPanelEvent::Sync(SyncAction::Push)))),
            );
        let placeholder = format!("Message (Ctrl+Enter to commit on \"{name}\")");
        self.message.update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
        let can_commit = self.can_commit(cx);
        let amend = self.amend;
        let initial = st.initial;
        let commit_box = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .px(px(8.0))
            .pt(px(6.0))
            .child(
                div()
                    .h(px(64.0))
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(2.0))
                    .bg(t.bg)
                    .child(Textarea::new(&self.message).appearance(false).h_full()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        button("git-commit", if amend { "Amend Commit" } else { "Commit" }, &t)
                            .when(can_commit, |d| d.bg(t.accent).text_color(t.on_heat).border_color(t.accent))
                            .when(!can_commit, |d| d.opacity(0.5))
                            .on_click(cx.listener(|this, _, _, cx| this.commit(cx))),
                    )
                    .child(
                        div()
                            .id("git-amend")
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .text_size(px(12.0))
                            .text_color(if initial { t.fg_muted } else { t.fg })
                            .when(!initial, |d| d.cursor_pointer())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if initial {
                                    return;
                                }
                                this.amend = !this.amend;
                                // Start from the last message when amending.
                                if this.amend && this.message_text(cx).is_empty() {
                                    cx.emit(GitPanelEvent::WantLastMessage);
                                }
                                cx.notify();
                            }))
                            .child(if amend { "☑" } else { "☐" })
                            .child("Amend"),
                    ),
            );
        let mut out: Vec<AnyElement> = vec![head.into_any_element(), commit_box.into_any_element()];
        if !self.busy.is_empty() {
            out.push(note(&t, self.busy.clone()).text_color(t.accent).into_any_element());
        }
        if !st.conflicts.is_empty() {
            let all: Vec<String> = st.conflicts.iter().map(|c| c.path.clone()).collect();
            out.push(self.section(
                "conflicts",
                "Merge Conflicts",
                &st.conflicts,
                false,
                vec![("Stage all", "Mark all as resolved (stage them)", GitPanelEvent::Stage(all))],
                &t,
                cx,
            ));
        }
        if !st.staged.is_empty() {
            let all: Vec<String> = st.staged.iter().map(|c| c.path.clone()).collect();
            out.push(self.section(
                "staged",
                "Staged Changes",
                &st.staged,
                true,
                vec![("−", "Unstage all", GitPanelEvent::Unstage(all))],
                &t,
                cx,
            ));
        }
        let all: Vec<String> = st.unstaged.iter().map(|c| c.path.clone()).collect();
        out.push(self.section(
            "changes",
            "Changes",
            &st.unstaged,
            false,
            vec![
                ("↶", "Discard all changes", GitPanelEvent::Discard(st.unstaged.clone())),
                ("+", "Stage all", GitPanelEvent::Stage(all)),
            ],
            &t,
            cx,
        ));
        if st.staged.is_empty() && st.unstaged.is_empty() && st.conflicts.is_empty() {
            out.push(
                note(
                    &t,
                    if st.initial {
                        "No commits yet. Create or change files, then commit them."
                    } else {
                        "No changes. Everything is committed."
                    },
                )
                .into_any_element(),
            );
        }
        root.children(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    #[gpui_kit::test]
    fn keyboard_commit_cannot_bypass_unresolved_conflicts(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(ForgeTheme(std::sync::Arc::new(Theme::molten())));
        });
        let (panel, cx) = cx.add_window_view(GitPanel::new);
        let events = std::rc::Rc::new(std::cell::Cell::new(0));
        let count = events.clone();
        let _sub = cx.update(|_, cx| cx.subscribe(&panel, move |_, _: &GitPanelEvent, _| count.set(count.get() + 1)));
        panel.update_in(cx, |p, window, cx| {
            p.set_status(
                Some(Repo {
                    root: "/p".into(),
                    status: RepoStatus {
                        conflicts: vec![FileChange { path: "a.py".into(), from: None, kind: ChangeKind::Conflict }],
                        ..Default::default()
                    },
                }),
                cx,
            );
            p.message.update(cx, |s, cx| s.set_value("resolve", window, cx));
            p.commit(cx);
        });
        assert_eq!(events.get(), 0);
    }
}
