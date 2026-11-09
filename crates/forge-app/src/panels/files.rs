//! The Files panel: the project's folders and files. Folder contents are read
//! the first time a folder is opened, so huge projects stay fast. Files and
//! folders are created, renamed and deleted from here (header buttons,
//! right-click, F2, Delete). Replaces `src/renderer/fileTree.ts`.

use crate::theme::{ForgeTheme, Theme};
use forge_core::git::{ChangeKind, StatusMap};
use forge_core::names::name_problem;
use forge_core::tabs::{is_inside, same_path};
use forge_sys::files::{create_entry, read_dir, rename_entry, DirEntry};
use futures::{channel::mpsc, FutureExt, StreamExt};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::{
    anchored, deferred, div, prelude::FluentBuilder, px, uniform_list, AnyElement, App, AppContext, ClipboardItem,
    Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, ParentElement, Pixels, Point, Render, ScrollStrategy, StatefulInteractiveElement,
    Styled, Subscription, UniformListScrollHandle, Window,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const INDENT: f32 = 12.0;
const ROW_HEIGHT: f32 = 22.0;

pub enum FileTreeEvent {
    OpenFolder,
    NewProject,
    Open(PathBuf),
    /// A file or folder was created (files are opened right away).
    Created {
        path: PathBuf,
        is_dir: bool,
    },
    Renamed {
        old: PathBuf,
        new: PathBuf,
    },
    /// Asked to delete (the workspace asks, moves it to the trash and closes tabs).
    Delete {
        path: PathBuf,
        is_dir: bool,
    },
    /// Open a recently used folder (shown while no folder is open).
    OpenRecent(String),
    Error(String),
}

impl EventEmitter<FileTreeEvent> for FileTree {}

/// Windows paths compare case-insensitively.
fn key(p: &Path) -> String {
    let s = p.to_string_lossy();
    let b = s.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        s.replace('/', "\\").to_lowercase()
    } else {
        s.into_owned()
    }
}

#[derive(Clone, Debug, PartialEq)]
enum RowKind {
    Entry(DirEntry),
    Message(String),
    /// The name box for a new file or folder.
    NewName {
        is_dir: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct Row {
    kind: RowKind,
    depth: usize,
}

/// The tree's contents and which folders are open: everything but the drawing.
#[derive(Default)]
struct TreeState {
    root: Option<PathBuf>,
    /// Folder contents read so far (by folder key); Err = couldn't be read.
    children: HashMap<String, Result<Vec<DirEntry>, String>>,
    /// Folders you have open (keys), so refreshing keeps them open.
    expanded: HashSet<String>,
    selected: Option<String>,
    rows: Arc<Vec<Row>>,
    /// A name box for a new file (false) or folder (true) is showing in this folder.
    creating: Option<(PathBuf, bool)>,
}

impl TreeState {
    fn set_root(&mut self, root: Option<PathBuf>) {
        *self = TreeState { root, ..Default::default() };
        if let Some(r) = self.root.clone() {
            self.load(&r);
        }
        self.rebuild();
    }

    fn load(&mut self, dir: &Path) {
        let result = read_dir(dir).map_err(|e| e.to_string());
        self.children.insert(key(dir), result);
    }

    fn entries(&self, dir: &Path) -> Vec<DirEntry> {
        self.children.get(&key(dir)).and_then(|r| r.as_ref().ok()).cloned().unwrap_or_default()
    }

    /// Read a folder again (if it was read before), and the open folders inside it.
    fn refresh(&mut self, dir: &Path) {
        if !self.children.contains_key(&key(dir)) {
            return;
        }
        self.load(dir);
        for e in self.entries(dir) {
            if e.is_dir && self.expanded.contains(&key(&e.path)) {
                self.refresh(&e.path);
            }
        }
    }

    fn refresh_all(&mut self) {
        let Some(root) = self.root.clone() else { return };
        self.children.clear();
        self.load(&root);
        let mut stack = vec![root];
        while let Some(d) = stack.pop() {
            for e in self.entries(&d) {
                if e.is_dir && self.expanded.contains(&key(&e.path)) {
                    self.load(&e.path);
                    stack.push(e.path);
                }
            }
        }
    }

    fn set_expanded(&mut self, dir: &Path, open: bool) {
        let k = key(dir);
        if open {
            self.expanded.insert(k.clone());
            if !self.children.contains_key(&k) {
                self.load(dir);
            }
        } else {
            self.expanded.remove(&k);
        }
    }

    /// Open every folder above `path` (inside the root). False if it isn't in the project.
    fn open_parents(&mut self, path: &Path) -> bool {
        let Some(root) = self.root.clone() else { return false };
        let (ps, rs) = (path.to_string_lossy(), root.to_string_lossy());
        if !is_inside(&ps, &rs) || same_path(&ps, &rs) {
            return false;
        }
        let mut dirs: Vec<PathBuf> = path
            .ancestors()
            .skip(1)
            .take_while(|a| !same_path(&a.to_string_lossy(), &rs))
            .map(Path::to_path_buf)
            .collect();
        dirs.reverse();
        for d in dirs {
            self.set_expanded(&d, true);
        }
        true
    }

    /// The flattened list of visible rows.
    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        if let Some(root) = self.root.clone() {
            self.push_children(&root, 0, &mut rows);
        }
        self.rows = Arc::new(rows);
    }

    fn push_children(&self, dir: &Path, depth: usize, rows: &mut Vec<Row>) {
        let creating = self.creating.as_ref().filter(|(d, _)| key(d) == key(dir)).map(|(_, is_dir)| *is_dir);
        if let Some(is_dir) = creating {
            rows.push(Row { kind: RowKind::NewName { is_dir }, depth });
        }
        match self.children.get(&key(dir)) {
            None => {}
            Some(Err(e)) => rows.push(Row { kind: RowKind::Message(format!("Can't read folder: {e}")), depth }),
            Some(Ok(entries)) if entries.is_empty() => {
                if creating.is_none() {
                    rows.push(Row { kind: RowKind::Message("(empty)".into()), depth });
                }
            }
            Some(Ok(entries)) => {
                for e in entries {
                    rows.push(Row { kind: RowKind::Entry(e.clone()), depth });
                    if e.is_dir && self.expanded.contains(&key(&e.path)) {
                        self.push_children(&e.path, depth + 1, rows);
                    }
                }
            }
        }
    }

    fn selected_index(&self) -> Option<usize> {
        let sel = self.selected.as_ref()?;
        self.rows.iter().position(|r| matches!(&r.kind, RowKind::Entry(e) if key(&e.path) == *sel))
    }

    fn selected_entry(&self) -> Option<DirEntry> {
        match &self.rows.get(self.selected_index()?)?.kind {
            RowKind::Entry(e) => Some(e.clone()),
            _ => None,
        }
    }

    /// Where "New File" goes: the selected folder, the selected file's folder, or the root.
    fn target_folder(&self, entry: Option<&DirEntry>) -> Option<PathBuf> {
        let root = self.root.clone()?;
        Some(match entry {
            None => root,
            Some(e) if e.is_dir => e.path.clone(),
            Some(e) => e.path.parent().map(Path::to_path_buf).unwrap_or(root),
        })
    }

    /// A folder was renamed: open folders inside it stay open under the new name.
    fn moved(&mut self, old: &Path, new: &Path) {
        let old_key = key(old);
        let moved: Vec<String> = self.expanded.iter().filter(|k| is_inside(k, &old_key)).cloned().collect();
        for k in moved {
            self.expanded.remove(&k);
            let rest: String = k.chars().skip(old_key.chars().count()).collect();
            self.expanded.insert(format!("{}{rest}", key(new)));
        }
    }
}

enum EditMode {
    Create { dir: PathBuf, is_dir: bool },
    Rename { path: PathBuf, old_name: String },
}

struct Editing {
    input: Entity<InputState>,
    mode: EditMode,
    problem: Option<String>,
    _subs: Vec<Subscription>,
}

struct Menu {
    position: Point<Pixels>,
    entry: Option<DirEntry>,
}

#[derive(Clone)]
enum MenuAction {
    NewFile(Option<PathBuf>),
    NewFolder(Option<PathBuf>),
    Rename(PathBuf),
    Delete(DirEntry),
    Copy(String),
}

pub struct FileTree {
    focus: FocusHandle,
    st: TreeState,
    /// Git status: changed files (by key) and the folders holding them.
    git_files: HashMap<String, ChangeKind>,
    git_folders: HashSet<String>,
    /// One worker per project; requests are coalesced while git is running.
    git_refresh: Option<mpsc::Sender<()>>,
    git_task: Option<gpui_kit::Task<()>>,
    editing: Option<Editing>,
    menu: Option<Menu>,
    scroll: UniformListScrollHandle,
    recent: Vec<String>,
}

impl Focusable for FileTree {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FileTree {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            st: TreeState::default(),
            git_files: HashMap::new(),
            git_folders: HashSet::new(),
            git_refresh: None,
            git_task: None,
            editing: None,
            menu: None,
            scroll: UniformListScrollHandle::new(),
            recent: Vec::new(),
        }
    }

    pub fn root(&self) -> Option<&Path> {
        self.st.root.as_deref()
    }

    /// Show git status: coloured names and a letter for changed files, a dot on folders.
    pub fn set_git(&mut self, status: Option<&StatusMap>, cx: &mut Context<Self>) {
        let files: HashMap<String, ChangeKind> =
            status.map(|s| s.files.iter().map(|(p, k)| (key(Path::new(p)), *k)).collect()).unwrap_or_default();
        let folders: HashSet<String> =
            status.map(|s| s.folders.iter().map(|p| key(Path::new(p))).collect()).unwrap_or_default();
        if files != self.git_files || folders != self.git_folders {
            self.git_files = files;
            self.git_folders = folders;
            cx.notify();
        }
    }

    /// Request a new status after saving, file operations or regaining focus.
    pub fn refresh_git(&mut self) {
        if let Some(sender) = &mut self.git_refresh {
            // A full channel already has a refresh pending.
            let _ = sender.try_send(());
        }
    }

    /// Cancel the old project's worker, clear its decorations, and read the
    /// new status off the UI thread. Polling also catches terminal commands.
    fn start_git(&mut self, cx: &mut Context<Self>) {
        self.git_task = None;
        self.git_refresh = None;
        self.set_git(None, cx);
        let Some(folder) = self.st.root.clone() else { return };
        let (sender, mut requests) = mpsc::channel(1);
        self.git_refresh = Some(sender);
        self.git_task = Some(cx.spawn(async move |this, cx| loop {
            let path = folder.to_string_lossy().into_owned();
            let status = cx.background_executor().spawn(async move { forge_sys::git::tree_status(&path) }).await;
            let applied = this.update(cx, |tree, cx| {
                if tree.st.root.as_ref() != Some(&folder) {
                    return false;
                }
                tree.set_git(status.as_ref(), cx);
                true
            });
            if !matches!(applied, Ok(true)) {
                break;
            }
            let timer = cx.background_executor().timer(Duration::from_secs(5)).fuse();
            let request = requests.next().fuse();
            futures::pin_mut!(timer, request);
            futures::select! {
                _ = timer => {},
                next = request => if next.is_none() { break },
            }
        }));
    }

    /// With no folder open: recently opened folders to reopen with one click.
    pub fn set_recent(&mut self, recent: Vec<String>, cx: &mut Context<Self>) {
        self.recent = recent;
        cx.notify();
    }

    pub fn set_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        self.editing = None;
        self.menu = None;
        self.st.set_root(root);
        self.start_git(cx);
        cx.notify();
    }

    /// Re-read one folder (or the root) from disk, keeping open folders open.
    pub fn refresh(&mut self, dir: &Path, cx: &mut Context<Self>) {
        self.refresh_git();
        self.st.refresh(dir);
        self.st.rebuild();
        cx.notify();
    }

    pub fn refresh_all(&mut self, cx: &mut Context<Self>) {
        self.refresh_git();
        self.st.refresh_all();
        self.st.rebuild();
        cx.notify();
    }

    pub fn collapse_all(&mut self, cx: &mut Context<Self>) {
        self.st.expanded.clear();
        self.st.rebuild();
        cx.notify();
    }

    /// Open every folder above `path` so its row is visible; select it and scroll to it.
    pub fn reveal(&mut self, path: &Path, cx: &mut Context<Self>) {
        if !self.st.open_parents(path) {
            return;
        }
        self.st.selected = Some(key(path));
        self.st.rebuild();
        self.scroll_to_selected();
        cx.notify();
    }

    /// Select the row for this file if it's visible (the active tab changed).
    pub fn highlight(&mut self, path: Option<&Path>, cx: &mut Context<Self>) {
        let Some(p) = path else { return };
        let k = key(p);
        let visible = self.st.rows.iter().any(|r| matches!(&r.kind, RowKind::Entry(e) if key(&e.path) == k));
        if visible && self.st.selected.as_ref() != Some(&k) {
            self.st.selected = Some(k);
            self.scroll_to_selected();
            cx.notify();
        }
    }

    fn scroll_to_selected(&self) {
        if let Some(i) = self.st.selected_index() {
            self.scroll.scroll_to_item(i, ScrollStrategy::Top);
        }
    }

    fn click_row(&mut self, entry: &DirEntry, cx: &mut Context<Self>) {
        self.menu = None;
        self.st.selected = Some(key(&entry.path));
        if entry.is_dir {
            let open = !self.st.expanded.contains(&key(&entry.path));
            self.st.set_expanded(&entry.path, open);
            self.st.rebuild();
        } else {
            cx.emit(FileTreeEvent::Open(entry.path.clone()));
        }
        cx.notify();
    }

    // ---------- create / rename / delete ----------

    /// Show a name box in `dir` (default: the selected folder); creates the file or folder on Enter.
    pub fn start_create(&mut self, is_dir: bool, dir: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dir) = dir.or_else(|| self.st.target_folder(self.st.selected_entry().as_ref())) else { return };
        self.menu = None;
        self.st.open_parents(&dir.join("x"));
        self.st.creating = Some((dir.clone(), is_dir));
        let placeholder = if is_dir { "folder name" } else { "file name (e.g. app.py or pkg/app.py)" };
        self.begin_edit(EditMode::Create { dir, is_dir }, "", placeholder, window, cx);
    }

    /// Turn a row's name into a box for typing the new name (default: the selected row).
    pub fn start_rename(&mut self, path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = path.or_else(|| self.st.selected_entry().map(|e| e.path)) else { return };
        self.menu = None;
        self.reveal(&path, cx);
        let old_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.begin_edit(EditMode::Rename { path, old_name: old_name.clone() }, &old_name, "", window, cx);
    }

    fn begin_edit(
        &mut self,
        mode: EditMode,
        initial: &str,
        placeholder: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(placeholder.to_owned()).default_value(initial.to_owned())
        });
        // Select the name without its extension, like other editors.
        let dot = initial.rfind('.').filter(|d| *d > 0).unwrap_or(initial.len());
        input.update(cx, |s, cx| {
            s.focus(window, cx);
            s.set_selected_range(0..dot, cx);
        });
        let sub = cx.subscribe_in(&input, window, |this, input, event: &InputEvent, window, cx| match event {
            InputEvent::Change => {
                let name = input.read(cx).value().trim().to_owned();
                if let Some(ed) = &mut this.editing {
                    let nested = matches!(ed.mode, EditMode::Create { .. });
                    ed.problem = if name.is_empty() { None } else { name_problem(&name, nested) };
                }
                cx.notify();
            }
            InputEvent::PressEnter { .. } => this.submit(false, window, cx),
            InputEvent::Blur => this.submit(true, window, cx),
            InputEvent::Focus => {}
        });
        self.editing = Some(Editing { input, mode, problem: None, _subs: vec![sub] });
        self.st.rebuild();
        let row = self
            .st
            .rows
            .iter()
            .position(|r| matches!(r.kind, RowKind::NewName { .. }))
            .or_else(|| self.st.selected_index());
        if let Some(i) = row {
            self.scroll.scroll_to_item(i, ScrollStrategy::Top);
        }
        cx.notify();
    }

    fn finish_edit(&mut self, refocus: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = None;
        self.st.creating = None;
        self.st.rebuild();
        if refocus {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    fn submit(&mut self, from_blur: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ed) = &self.editing else { return };
        let name = ed.input.read(cx).value().trim().to_owned();
        let (initial, nested) = match &ed.mode {
            EditMode::Rename { old_name, .. } => (old_name.clone(), false),
            EditMode::Create { .. } => (String::new(), true),
        };
        // Clicking away from an empty or unchanged box just cancels it.
        if name.is_empty() || name == initial {
            return self.finish_edit(!from_blur, window, cx);
        }
        if let Some(problem) = name_problem(&name, nested) {
            if from_blur {
                return self.finish_edit(false, window, cx);
            }
            if let Some(ed) = &mut self.editing {
                ed.problem = Some(problem);
            }
            cx.notify();
            return;
        }
        let result = match &ed.mode {
            EditMode::Create { dir, is_dir } => {
                let (dir, is_dir) = (dir.clone(), *is_dir);
                create_entry(&dir, &name, is_dir).map(|path| {
                    self.st.creating = None;
                    self.st.refresh(&dir);
                    // "pkg/app.py" may have created folders: open the way down to the new item.
                    self.st.open_parents(&path);
                    self.st.selected = Some(key(&path));
                    cx.emit(FileTreeEvent::Created { path, is_dir });
                })
            }
            EditMode::Rename { path, .. } => {
                let path = path.clone();
                rename_entry(&path, &name).map(|new| {
                    self.st.moved(&path, &new);
                    if let Some(parent) = path.parent() {
                        self.st.refresh(parent);
                    }
                    self.st.selected = Some(key(&new));
                    cx.emit(FileTreeEvent::Renamed { old: path, new });
                })
            }
        };
        match result {
            Ok(()) => {
                self.refresh_git();
                self.finish_edit(!from_blur, window, cx);
                self.scroll_to_selected();
            }
            Err(err) => {
                if from_blur {
                    self.finish_edit(false, window, cx);
                    cx.emit(FileTreeEvent::Error(err.to_string()));
                } else if let Some(ed) = &mut self.editing {
                    ed.problem = Some(err.to_string());
                    cx.notify();
                }
            }
        }
    }

    pub fn request_delete(&mut self, entry: Option<DirEntry>, cx: &mut Context<Self>) {
        self.menu = None;
        if let Some(e) = entry.or_else(|| self.st.selected_entry()) {
            cx.emit(FileTreeEvent::Delete { path: e.path, is_dir: e.is_dir });
        }
        cx.notify();
    }

    /// Something was deleted (after asking): forget it and read its folder again.
    pub fn deleted(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let k = key(path);
        if self.st.selected.as_ref() == Some(&k) {
            self.st.selected = None;
        }
        self.st.expanded.retain(|p| !is_inside(p, &k));
        if let Some(parent) = path.parent() {
            self.refresh(parent, cx);
        }
        window.focus(&self.focus, cx);
    }

    fn relative(&self, path: &Path) -> String {
        match &self.st.root {
            Some(r) => path.strip_prefix(r).unwrap_or(path).to_string_lossy().into_owned(),
            None => path.to_string_lossy().into_owned(),
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() || !self.focus.is_focused(window) {
            return;
        }
        match event.keystroke.key.as_str() {
            "f2" => self.start_rename(None, window, cx),
            "delete" => self.request_delete(None, cx),
            "enter" => {
                if let Some(e) = self.st.selected_entry() {
                    self.click_row(&e, cx);
                }
            }
            "escape" if self.menu.is_some() => {
                self.menu = None;
                cx.notify();
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn open_menu(&mut self, position: Point<Pixels>, entry: Option<DirEntry>, cx: &mut Context<Self>) {
        if self.st.root.is_none() {
            return;
        }
        if let Some(e) = &entry {
            self.st.selected = Some(key(&e.path));
        }
        self.menu = Some(Menu { position, entry });
        cx.notify();
    }

    fn run_menu(&mut self, action: MenuAction, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        match action {
            MenuAction::NewFile(dir) => self.start_create(false, dir, window, cx),
            MenuAction::NewFolder(dir) => self.start_create(true, dir, window, cx),
            MenuAction::Rename(path) => self.start_rename(Some(path), window, cx),
            MenuAction::Delete(entry) => self.request_delete(Some(entry), cx),
            MenuAction::Copy(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
        }
        cx.notify();
    }

    // ---------- drawing ----------

    fn render_row(&self, row: &Row, index: usize, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let pad = px(4.0 + row.depth as f32 * INDENT);
        match &row.kind {
            RowKind::Message(text) => div()
                .h(px(ROW_HEIGHT))
                .flex()
                .items_center()
                .pl(pad + px(16.0))
                .text_color(t.fg_muted)
                .child(text.clone())
                .into_any_element(),
            RowKind::NewName { is_dir } => self.render_name_box(*is_dir, pad, t),
            RowKind::Entry(entry) => {
                let renaming = matches!(&self.editing, Some(Editing { mode: EditMode::Rename { path, .. }, .. }) if key(path) == key(&entry.path));
                if renaming {
                    return self.render_name_box(entry.is_dir, pad, t);
                }
                let selected = self.st.selected.as_ref() == Some(&key(&entry.path));
                let open = entry.is_dir && self.st.expanded.contains(&key(&entry.path));
                let chevron = match (entry.is_dir, open) {
                    (false, _) => "",
                    (true, true) => "▾",
                    (true, false) => "▸",
                };
                let (e1, e2) = (entry.clone(), entry.clone());
                let git = (!entry.is_dir).then(|| self.git_files.get(&key(&entry.path)).copied()).flatten();
                let dirty_folder = entry.is_dir && self.git_folders.contains(&key(&entry.path));
                let git_color = git.map(|k| match k {
                    ChangeKind::Added | ChangeKind::Untracked | ChangeKind::Copied => t.git_added,
                    ChangeKind::Deleted => t.git_deleted,
                    ChangeKind::Conflict => t.git_conflict,
                    _ => t.git_modified,
                });
                div()
                    .id(("tree-row", index))
                    .h(px(ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .pl(pad)
                    .pr(px(8.0))
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(t.bg_active))
                    .when(!selected, |d| d.hover(|s| s.bg(t.bg_hover)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        window.focus(&this.focus, cx);
                        this.click_row(&e1, cx);
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.open_menu(ev.position, Some(e2.clone()), cx);
                        }),
                    )
                    .child(div().w(px(16.0)).flex_none().text_center().text_color(t.fg_muted).child(chevron))
                    .child(
                        div()
                            .overflow_hidden()
                            .text_ellipsis()
                            .when(entry.is_dir, |d| d.font_weight(FontWeight::MEDIUM))
                            .when_some(git_color, |d, c| d.text_color(c))
                            .when(git == Some(ChangeKind::Deleted), |d| d.line_through())
                            .child(entry.name.clone()),
                    )
                    .when(dirty_folder, |d| d.child(div().pl(px(4.0)).text_color(t.git_modified).child("•")))
                    .when_some(git.zip(git_color), |d, (k, c)| {
                        d.child(
                            div()
                                .ml_auto()
                                .pl(px(8.0))
                                .text_size(px(11.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(c)
                                .child(k.letter()),
                        )
                    })
                    .into_any_element()
            }
        }
    }

    fn render_name_box(&self, is_dir: bool, pad: Pixels, t: &Theme) -> AnyElement {
        let Some(ed) = &self.editing else { return div().into_any_element() };
        div()
            .debug_selector(|| "tree-name-row".into())
            .w_full()
            .flex()
            .flex_col()
            .pl(pad)
            .pr(px(8.0))
            .py_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(div().w(px(16.0)).flex_none().text_center().text_color(t.fg_muted).child(if is_dir {
                        "▸"
                    } else {
                        ""
                    }))
                    .child(
                        div()
                            .debug_selector(|| "tree-name-frame".into())
                            .flex_1()
                            .min_w_0()
                            .border_1()
                            .border_color(if ed.problem.is_some() { t.err } else { t.heat_2 })
                            // Input::h configures multiline inputs only. Override
                            // the frame style so this fits the 22px virtual row.
                            .child(
                                Styled::h(Input::new(&ed.input).appearance(false).cleanable(false), px(20.0))
                                    .py_0()
                                    .px(px(4.0)),
                            ),
                    ),
            )
            .when_some(ed.problem.clone(), |d, p| {
                d.child(div().pl(px(16.0)).text_size(px(11.0)).text_color(t.err).child(p))
            })
            .into_any_element()
    }

    fn render_menu(&self, menu: &Menu, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let dir = self.st.target_folder(menu.entry.as_ref());
        let mut items: Vec<Option<(&'static str, &'static str, MenuAction)>> = vec![
            Some(("New File…", "Ctrl+N", MenuAction::NewFile(dir.clone()))),
            Some(("New Folder…", "", MenuAction::NewFolder(dir))),
        ];
        if let Some(e) = &menu.entry {
            items.extend([
                None,
                Some(("Rename…", "F2", MenuAction::Rename(e.path.clone()))),
                Some(("Delete", "Delete", MenuAction::Delete(e.clone()))),
                None,
                Some(("Copy Path", "", MenuAction::Copy(e.path.to_string_lossy().into_owned()))),
                Some(("Copy Relative Path", "", MenuAction::Copy(self.relative(&e.path)))),
            ]);
        }
        let rows = items.into_iter().enumerate().map(|(i, item)| match item {
            None => div().h(px(1.0)).my(px(4.0)).bg(t.border).into_any_element(),
            Some((label, keys, action)) => div()
                .id(("tree-menu", i))
                .flex()
                .justify_between()
                .gap(px(24.0))
                .px(px(10.0))
                .py(px(4.0))
                .rounded(px(2.0))
                .cursor_pointer()
                .hover(|s| s.bg(t.bg_active))
                .on_click(cx.listener(move |this, _, window, cx| this.run_menu(action.clone(), window, cx)))
                .child(label)
                .child(div().text_color(t.fg_muted).child(keys))
                .into_any_element(),
        });
        deferred(
            anchored().position(menu.position).snap_to_window().child(
                div()
                    .id("tree-menu")
                    .min_w(px(200.0))
                    .p(px(4.0))
                    .bg(t.bg_panel)
                    .border_1()
                    .border_color(t.line_strong)
                    .rounded(px(2.0))
                    .shadow_lg()
                    .text_size(px(13.0))
                    .text_color(t.fg)
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.menu = None;
                        cx.notify();
                    }))
                    .children(rows),
            ),
        )
        .with_priority(1)
    }

    fn render_recent(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.recent.iter().enumerate().map(|(i, folder)| {
            let name = folder.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(folder).to_owned();
            let f = folder.clone();
            div()
                .id(("recent", i))
                .flex()
                .flex_col()
                .px(px(12.0))
                .py(px(4.0))
                .cursor_pointer()
                .hover(|s| s.bg(t.bg_hover))
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(FileTreeEvent::OpenRecent(f.clone()))))
                .child(div().child(name))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(t.fg_muted)
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(folder.clone()),
                )
        });
        div()
            .flex()
            .flex_col()
            .py(px(4.0))
            .child(div().px(px(12.0)).py(px(6.0)).text_color(t.fg_muted).child("Choose a project folder to work in."))
            .children(
                [("Open Folder…", "tree-open-folder", false), ("New Project Folder…", "tree-new-project", true)]
                    .into_iter()
                    .map(|(label, id, new)| {
                        div()
                            .id(id)
                            .debug_selector(|| id.to_owned())
                            .px(px(12.0))
                            .py(px(6.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(t.bg_hover))
                            .child(label)
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(if new { FileTreeEvent::NewProject } else { FileTreeEvent::OpenFolder })
                            }))
                    }),
            )
            .when(!self.recent.is_empty(), |d| {
                d.child(
                    div().px(px(12.0)).py(px(6.0)).text_size(px(10.0)).text_color(t.fg_muted).child("RECENT FOLDERS"),
                )
            })
            .children(rows)
    }
}

impl Render for FileTree {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<ForgeTheme>().0.clone();
        let base = div()
            .id("file-tree")
            .key_context("FileTree")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .text_size(px(13.0))
            .text_color(t.fg)
            .on_key_down(cx.listener(Self::on_key_down))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                if this.editing.is_some() {
                    this.finish_edit(true, window, cx);
                    cx.stop_propagation();
                }
            }));
        if self.st.root.is_none() {
            return base.child(self.render_recent(&t, cx));
        }
        type Run = fn(&mut FileTree, &mut Window, &mut Context<FileTree>);
        let buttons: [(&'static str, &'static str, Run); 5] = [
            ("Open…", "tree-open-folder", |_, _, cx| cx.emit(FileTreeEvent::OpenFolder)),
            ("+ File", "tree-new-file", |this, w, cx| this.start_create(false, None, w, cx)),
            ("+ Folder", "tree-new-folder", |this, w, cx| this.start_create(true, None, w, cx)),
            ("⟳", "tree-refresh", |this, _, cx| this.refresh_all(cx)),
            ("⊟", "tree-collapse", |this, _, cx| this.collapse_all(cx)),
        ];
        let header = div()
            .flex()
            .flex_wrap()
            .flex_none()
            .gap(px(2.0))
            .px(px(6.0))
            .py(px(3.0))
            .border_b_1()
            .border_color(t.border)
            .text_size(px(12.0))
            .children(buttons.into_iter().map(|(label, id, run)| {
                div()
                    .id(id)
                    .debug_selector(|| id.to_owned())
                    .px(px(6.0))
                    .py(px(1.0))
                    .rounded(px(2.0))
                    .text_color(t.fg_muted)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.bg_hover).text_color(t.fg))
                    .on_click(cx.listener(move |this, _, window, cx| run(this, window, cx)))
                    .child(label)
            }));
        let rows = self.st.rows.clone();
        let list = uniform_list(
            "tree-list",
            rows.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let t = cx.global::<ForgeTheme>().0.clone();
                range.map(|i| this.render_row(&rows[i], i, &t, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .size_full()
        .py(px(4.0));
        let project = self.st.root.as_ref().map(|root| root.to_string_lossy().into_owned()).unwrap_or_default();
        base.child(
            div()
                .px(px(10.0))
                .py(px(5.0))
                .text_size(px(11.0))
                .text_color(t.fg_muted)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(project),
        )
        .child(header)
        .child(
            div()
                .id("tree-body")
                .flex_1()
                .min_h_0()
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, ev: &MouseDownEvent, _, cx| this.open_menu(ev.position, None, cx)),
                )
                .child(list),
        )
        .when_some(self.menu.as_ref(), |d, m| d.child(self.render_menu(m, &t, cx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    fn git(dir: &Path, args: &[&str]) {
        let output = std::process::Command::new("git").args(args).current_dir(dir).output().expect("git installed");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }

    #[gpui_kit::test]
    fn narrow_tree_name_input_has_a_visible_clickable_text_area(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(ForgeTheme(Arc::new(Theme::molten())));
        });
        let mut tree = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(FileTree::new);
            tree = Some(view.clone());
            gpui_kit::component::Root::new(view, window, cx)
        });
        let tree = tree.unwrap();
        cx.simulate_resize(gpui_kit::size(px(200.0), px(500.0)));
        tree.update_in(cx, |tree, window, cx| {
            tree.set_root(Some(dir.path().into()), cx);
            tree.start_create(false, None, window, cx);
        });
        cx.run_until_parked();
        let bounds = tree.read_with(cx, |t, cx| t.editing.as_ref().unwrap().input.read(cx).input_bounds());
        assert!(bounds.size.width >= px(120.0), "Name text viewport is too narrow: {bounds:?}");
        assert!(bounds.size.height >= px(16.0), "Name text viewport is too short: {bounds:?}");
        assert!(bounds.size.height <= px(ROW_HEIGHT), "Name input must fit the file tree row: {bounds:?}");
        let row = cx.debug_bounds("tree-name-row").expect("name row rendered");
        assert!(row.size.height <= px(ROW_HEIGHT), "Name row overlaps the following files: {row:?}");
        cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
        cx.simulate_input("visible.txt");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(dir.path().join("visible.txt").is_file());
        tree.update_in(cx, |tree, window, cx| tree.start_create(true, Some(dir.path().into()), window, cx));
        cx.run_until_parked();
        let bounds = tree.read_with(cx, |t, cx| t.editing.as_ref().unwrap().input.read(cx).input_bounds());
        assert!(bounds.size.width >= px(120.0));
        cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
        cx.simulate_input("new-folder");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(dir.path().join("new-folder").is_dir());
        tree.update_in(cx, |tree, window, cx| tree.start_rename(Some(dir.path().join("new-folder")), window, cx));
        cx.run_until_parked();
        let bounds = tree.read_with(cx, |t, cx| t.editing.as_ref().unwrap().input.read(cx).input_bounds());
        assert!(bounds.size.width >= px(120.0));
        cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
        cx.simulate_keystrokes("ctrl-a");
        cx.simulate_input("renamed-folder");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(dir.path().join("renamed-folder").is_dir());
    }

    #[gpui_kit::test]
    fn naming_a_new_file_then_renaming_it_with_the_keyboard(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(ForgeTheme(Arc::new(Theme::molten())));
        });
        let (tree, cx) = cx.add_window_view(|_, cx| FileTree::new(cx));
        tree.update_in(cx, |tree, window, cx| {
            tree.set_root(Some(dir.path().into()), cx);
            tree.start_create(false, None, window, cx);
        });
        cx.run_until_parked();
        cx.simulate_input("hello.txt");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(dir.path().join("hello.txt").is_file(), "Typing a name and Enter must create the file");
        tree.update_in(cx, |tree, window, cx| tree.start_rename(Some(dir.path().join("hello.txt")), window, cx));
        cx.run_until_parked();
        cx.simulate_input("renamed");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(
            dir.path().join("renamed.txt").is_file(),
            "Rename should replace the selected name and keep the extension"
        );
        assert!(!dir.path().join("hello.txt").exists());
    }

    #[gpui_kit::test]
    fn git_badges_load_on_open_and_refresh_after_external_changes(cx: &mut TestAppContext) {
        let dir = project();
        git(dir.path(), &["init", "-q"]);
        cx.update(|cx| cx.set_global(ForgeTheme(Arc::new(Theme::molten()))));
        let (tree, cx) = cx.add_window_view(|_, cx| FileTree::new(cx));
        tree.update(cx, |t, cx| t.set_root(Some(dir.path().to_path_buf()), cx));
        cx.run_until_parked();
        tree.read_with(cx, |t, _| {
            assert_eq!(t.git_files.get(&key(&dir.path().join("main.py"))), Some(&ChangeKind::Untracked));
            assert!(t.git_folders.contains(&key(&dir.path().join("pkg"))));
        });

        git(dir.path(), &["add", "main.py"]);
        cx.executor().advance_clock(std::time::Duration::from_secs(5));
        cx.run_until_parked();
        tree.read_with(cx, |t, _| {
            assert_eq!(t.git_files.get(&key(&dir.path().join("main.py"))), Some(&ChangeKind::Added));
        });

        std::fs::write(dir.path().join("new.py"), "").expect("write");
        tree.update(cx, |t, cx| t.refresh_all(cx));
        cx.run_until_parked();
        tree.read_with(cx, |t, _| {
            assert_eq!(t.git_files.get(&key(&dir.path().join("new.py"))), Some(&ChangeKind::Untracked));
        });
    }

    #[gpui_kit::test]
    fn changing_the_project_clears_git_badges_and_stops_the_old_refresh(cx: &mut TestAppContext) {
        let dir = project();
        let other = tempfile::tempdir().expect("temp");
        cx.update(|cx| cx.set_global(ForgeTheme(Arc::new(Theme::molten()))));
        let (tree, cx) = cx.add_window_view(|_, cx| FileTree::new(cx));
        tree.update(cx, |t, cx| {
            t.set_root(Some(dir.path().to_path_buf()), cx);
            let mut status = StatusMap::default();
            status.files.insert(dir.path().join("main.py").to_string_lossy().into_owned(), ChangeKind::Modified);
            status.folders.push(dir.path().join("pkg").to_string_lossy().into_owned());
            t.set_git(Some(&status), cx);
            t.set_root(Some(other.path().to_path_buf()), cx);
            assert!(t.git_files.is_empty());
            assert!(t.git_folders.is_empty());
        });
        cx.run_until_parked();
        cx.executor().advance_clock(std::time::Duration::from_secs(5));
        cx.run_until_parked();
        tree.read_with(cx, |t, _| assert!(t.git_files.is_empty() && t.git_folders.is_empty()));
        tree.update(cx, |t, cx| t.set_root(None, cx));
    }

    fn names(t: &TreeState) -> Vec<String> {
        t.rows
            .iter()
            .map(|r| {
                let pad = "  ".repeat(r.depth);
                match &r.kind {
                    RowKind::Entry(e) => format!("{pad}{}", e.name),
                    RowKind::Message(m) => format!("{pad}{m}"),
                    RowKind::NewName { .. } => format!("{pad}<new>"),
                }
            })
            .collect()
    }

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp");
        let r = dir.path();
        std::fs::create_dir_all(r.join("pkg/sub")).expect("mkdir");
        std::fs::create_dir_all(r.join("empty")).expect("mkdir");
        std::fs::write(r.join("main.py"), "").expect("write");
        std::fs::write(r.join("pkg/a.py"), "").expect("write");
        dir
    }

    #[test]
    fn lists_folders_first_and_expands_lazily() {
        let dir = project();
        let r = dir.path();
        let mut t = TreeState::default();
        t.set_root(Some(r.to_path_buf()));
        assert_eq!(names(&t), ["empty", "pkg", "main.py"]);
        assert!(!t.children.contains_key(&key(&r.join("pkg"))));
        t.set_expanded(&r.join("pkg"), true);
        t.set_expanded(&r.join("empty"), true);
        t.rebuild();
        assert_eq!(names(&t), ["empty", "  (empty)", "pkg", "  sub", "  a.py", "main.py"]);
        t.set_expanded(&r.join("pkg"), false);
        t.rebuild();
        assert_eq!(names(&t), ["empty", "  (empty)", "pkg", "main.py"]);
    }

    #[test]
    fn revealing_opens_the_folders_above_and_selects() {
        let dir = project();
        let r = dir.path();
        let mut t = TreeState::default();
        t.set_root(Some(r.to_path_buf()));
        let target = r.join("pkg").join("sub");
        assert!(t.open_parents(&target));
        t.selected = Some(key(&target));
        t.rebuild();
        assert_eq!(names(&t), ["empty", "pkg", "  sub", "  a.py", "main.py"]);
        assert_eq!(t.selected_index(), Some(2));
        assert!(!t.open_parents(Path::new("/elsewhere/x.py")));
    }

    #[test]
    fn a_new_name_box_shows_at_the_top_of_its_folder_instead_of_empty() {
        let dir = project();
        let r = dir.path();
        let mut t = TreeState::default();
        t.set_root(Some(r.to_path_buf()));
        t.set_expanded(&r.join("empty"), true);
        t.creating = Some((r.join("empty"), false));
        t.rebuild();
        assert_eq!(names(&t), ["empty", "  <new>", "pkg", "main.py"]);
    }

    #[test]
    fn refreshing_picks_up_new_files_and_keeps_folders_open() {
        let dir = project();
        let r = dir.path();
        let mut t = TreeState::default();
        t.set_root(Some(r.to_path_buf()));
        t.set_expanded(&r.join("pkg"), true);
        std::fs::write(r.join("pkg/b.py"), "").expect("write");
        std::fs::write(r.join("z.py"), "").expect("write");
        t.refresh_all();
        t.rebuild();
        assert_eq!(names(&t), ["empty", "pkg", "  sub", "  a.py", "  b.py", "main.py", "z.py"]);
    }

    #[test]
    fn renaming_a_folder_keeps_open_folders_inside_open() {
        let mut t = TreeState::default();
        t.expanded.insert("/p/pkg".into());
        t.expanded.insert("/p/pkg/sub".into());
        t.expanded.insert("/p/other".into());
        t.moved(Path::new("/p/pkg"), Path::new("/p/lib"));
        let mut open: Vec<&str> = t.expanded.iter().map(String::as_str).collect();
        open.sort_unstable();
        assert_eq!(open, ["/p/lib", "/p/lib/sub", "/p/other"]);
    }

    #[test]
    fn new_files_go_in_the_selected_folder_or_next_to_the_selected_file() {
        let dir = project();
        let r = dir.path();
        let mut t = TreeState::default();
        t.set_root(Some(r.to_path_buf()));
        let file = DirEntry { name: "main.py".into(), path: r.join("main.py"), is_dir: false };
        let folder = DirEntry { name: "pkg".into(), path: r.join("pkg"), is_dir: true };
        assert_eq!(t.target_folder(None), Some(r.to_path_buf()));
        assert_eq!(t.target_folder(Some(&file)), Some(r.to_path_buf()));
        assert_eq!(t.target_folder(Some(&folder)), Some(r.join("pkg")));
    }
}
