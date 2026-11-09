//! A Forge window: one project folder, its open files (tabs, split in up to
//! two groups), the dock of panels arranged per task, the command palette,
//! and the commands every shortcut and menu runs. Replaces the shell parts of
//! `src/renderer/main.ts` and `src/renderer/workspace.ts`.

mod chrome;
mod commands;
mod dock;
mod editors;
mod pyenv;
mod run;

use crate::app_state::{self, AppState};
use crate::document::{Document, DocumentEvent};
use crate::editor::{EditorEvent, EditorView};
use crate::find_bar::{FindBar, FindOption};
use crate::palette::{Choice, CommandItem, Palette, PaletteEvent};
use crate::panels::files::{FileTree, FileTreeEvent};
use crate::panels::output::OutputPanel;
use crate::panels::search::{SearchEvent, SearchPanel};
use crate::panels::terminal::TerminalPanel;
use crate::panels::tests::{TestsEvent, TestsPanel, Which};
use crate::panels::{self, Placeholder};
use crate::settings_view::{project_note, SettingsEvent, SettingsView};
use forge_core::commands::key_string;
use forge_core::dock::{normalize_layouts, Tasks, Zone};
use forge_core::project_state::{DocState, GroupState, WorkspaceState};
use forge_core::settings::{effective_settings, ProjectSettings, Settings, PROJECT_SETTING_KEYS};
use forge_core::tabs::{is_inside, renamed_path, same_path, DocId, Tabs};
use forge_sys::backups::{BackupAction, BackupData, BackupKeeper, Backups, DocSnapshot};
use forge_sys::files::{list_files, trash_entry};
use forge_sys::project_dir::{prepare_project_dir, ProjectDirResult};
use forge_sys::text_file::{read_text_file, write_text_file, TextFile};
use forge_sys::user_state::read_project_settings;
use forge_text::{EditKind, Eol, Pos};
use gpui_kit::{
    div, prelude::FluentBuilder, px, AnyView, App, AppContext, Context, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeystrokeEvent, ParentElement, PromptLevel, Render, SharedString, Styled,
    Subscription, Task, WeakEntity, Window,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Shortcuts that work while the terminal has focus (the shell never sees these keys).
const TERMINAL_PASSTHROUGH: [&str; 25] = [
    "task.code",
    "task.connect",
    "task.debug",
    "task.run",
    "view.projectGraph",
    "nav.projectMap",
    "file.openFolder",
    "file.openRecent",
    "palette.files",
    "debug.start",
    "terminal.new",
    "terminal.toggle",
    "panel.toggle",
    "palette.commands",
    "tab.next",
    "tab.prev",
    "run.smart",
    "run.stop",
    "run.file",
    "settings.open",
    "debug.restart",
    "debug.toggleBreakpoint",
    "debug.stepOver",
    "debug.stepInto",
    "debug.stepOut",
];

/// Commands that still work while the palette is open (they switch its mode).
const PALETTE_TOGGLES: [&str; 3] = ["palette.commands", "palette.files", "palette.line"];

use forge_text::encoding::LARGE_FILE_BYTES;

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub struct Workspace {
    focus: FocusHandle,
    pub(crate) root: Option<PathBuf>,
    pub(crate) tasks: Tasks,
    pub(crate) editing_layout: bool,
    pub(crate) tabs: Tabs,
    pub(crate) docs: HashMap<DocId, Entity<Document>>,
    next_doc: DocId,
    /// One editor per tab group (left, right).
    pub(crate) editors: [Entity<EditorView>; 2],
    panels: HashMap<&'static str, AnyView>,
    pub(crate) file_tree: Entity<FileTree>,
    pub(crate) palette: Entity<Palette>,
    pub(crate) find_bar: Entity<FindBar>,
    pub(crate) settings_view: Entity<SettingsView>,
    pub(crate) terminal: Entity<TerminalPanel>,
    pub(crate) output: Entity<OutputPanel>,
    pub(crate) tests: Entity<TestsPanel>,
    pub(crate) search: Entity<SearchPanel>,
    pub(crate) run: run::RunState,
    pub(crate) pyenv: pyenv::PyEnvState,
    /// The interpreter setting last applied (a change re-checks the environment).
    python_path_seen: Option<String>,
    /// The open project's overrides (`.forge/settings.json`).
    pub(crate) project_settings: ProjectSettings,
    project_settings_path: Option<PathBuf>,
    /// A short message in the status bar.
    pub(crate) note: Option<SharedString>,
    note_task: Option<Task<()>>,
    /// Where a dragged panel would land (Edit Layout mode).
    pub(crate) drop_target: Option<(String, Zone)>,
    backups: BackupKeeper,
    backup_store: Backups,
    backup_task: Option<Task<()>>,
    state_task: Option<Task<()>>,
    layout_task: Option<Task<()>>,
    /// Bringing back a project's tabs: don't save state meanwhile.
    restoring: bool,
    /// Asked to close the window: close it once unsaved files are dealt with.
    closing: bool,
    /// Help → Welcome: the Welcome page shows even with files open.
    pub(crate) welcome_forced: bool,
    _subs: Vec<Subscription>,
}

/// Open a Forge window, optionally with a folder.
pub fn open_window(folder: Option<PathBuf>, cx: &mut App) {
    let bounds = gpui_kit::Bounds::centered(None, gpui_kit::size(px(1280.0), px(820.0)), cx);
    let options = gpui_kit::WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(bounds)),
        titlebar: Some(gpui_kit::TitlebarOptions { title: Some("Forge".into()), ..Default::default() }),
        window_min_size: Some(gpui_kit::size(px(480.0), px(320.0))),
        ..Default::default()
    };
    let result = gpui_kit::open_window(options, cx, |window, cx| {
        let ws = cx.new(|cx| Workspace::new(window, cx));
        ws.update(cx, |ws, cx| {
            match folder {
                Some(f) => ws.open_folder(f, window, cx),
                None => ws.reopen_last(window, cx),
            }
            ws.focus_editor(window, cx);
        });
        cx.new(|cx| gpui_kit::component::Root::new(ws, window, cx))
    });
    if let Err(err) = result {
        eprintln!("couldn't open a window: {err}");
    }
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editors = [cx.new(EditorView::new), cx.new(EditorView::new)];
        let file_tree = cx.new(FileTree::new);
        let palette = cx.new(|cx| Palette::new(window, cx));
        let find_bar = cx.new(|cx| FindBar::new(window, cx));
        let settings_view = cx.new(|cx| SettingsView::new(window, cx));
        let terminal = cx.new(TerminalPanel::new);
        let output = cx.new(|cx| OutputPanel::new(window, cx));
        let tests = cx.new(TestsPanel::new);
        let search = cx.new(|cx| SearchPanel::new(window, cx));
        let mut subs = Vec::new();
        for (i, ed) in editors.iter().enumerate() {
            subs.push(cx.subscribe_in(ed, window, move |this, _, event: &EditorEvent, window, cx| {
                this.on_editor_event(i, event, window, cx)
            }));
        }
        subs.push(cx.subscribe_in(&file_tree, window, Self::on_tree_event));
        subs.push(cx.subscribe_in(&output, window, Self::on_output_event));
        subs.push(cx.subscribe_in(&search, window, |this, _, e: &SearchEvent, window, cx| match e {
            SearchEvent::OpenMatch(path, line, col, len) => {
                this.open_path(path.clone(), window, cx);
                let ed = this.editors[this.tabs.active_group.min(1)].clone();
                let (line, col, len) = (*line, *col, *len);
                ed.update(cx, |e, cx| e.select(Pos::new(line, col), Pos::new(line, col + len), cx));
            }
        }));
        subs.push(cx.subscribe_in(&tests, window, |this, _, e: &TestsEvent, window, cx| match e {
            TestsEvent::Open(path, line) => this.open_path_at(path.clone(), Some(*line), window, cx),
            TestsEvent::Run(which) => this.run_tests(which.clone(), window, cx),
            TestsEvent::InstallPytest => this.pip_install("pytest", window, cx),
            TestsEvent::Stop => this.tests.update(cx, |t, _| t.stop()),
        }));
        subs.push(cx.subscribe_in(&settings_view, window, |this, _, event: &SettingsEvent, window, cx| match event {
            SettingsEvent::Changed => this.apply_settings(cx),
            SettingsEvent::Command(id) => this.run_command(id, window, cx),
            SettingsEvent::Closed => this.focus_editor(window, cx),
        }));
        subs.push(cx.subscribe_in(&palette, window, |this, _, event: &PaletteEvent, window, cx| match event {
            PaletteEvent::Run(action) => action(this, window, cx),
            PaletteEvent::Closed => cx.notify(),
        }));
        // App-wide shortcuts win over the editor's own keys (like the old capture-phase handler).
        let me: WeakEntity<Self> = cx.entity().downgrade();
        let handle = window.window_handle();
        subs.push(cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() != handle {
                return;
            }
            if let Some(ws) = me.upgrade() {
                ws.update(cx, |ws, cx| ws.on_keystroke(event, window, cx));
            }
        }));
        // Closing the window with unsaved files asks first.
        let me2 = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            me2.upgrade().is_none_or(|ws| ws.update(cx, |ws, cx| ws.should_close(window, cx)))
        });
        let st = app_state::state(cx);
        let saved = normalize_layouts(&st.user.layouts());
        let recent = st.user.recent();
        let backup_store = Backups::new(st.user.dir().join("backups"));
        file_tree.update(cx, |t, cx| t.set_recent(recent, cx));
        let mut ws = Self {
            focus: cx.focus_handle(),
            root: None,
            tasks: Tasks::new(&saved),
            editing_layout: false,
            tabs: Tabs::default(),
            docs: HashMap::new(),
            next_doc: 1,
            editors,
            panels: HashMap::new(),
            file_tree,
            palette,
            find_bar,
            settings_view,
            terminal,
            output,
            tests,
            search,
            run: run::RunState::default(),
            pyenv: pyenv::PyEnvState::default(),
            python_path_seen: None,
            project_settings: ProjectSettings::default(),
            project_settings_path: None,
            note: None,
            note_task: None,
            drop_target: None,
            backups: BackupKeeper::new(1500),
            backup_store,
            backup_task: None,
            state_task: None,
            layout_task: None,
            restoring: false,
            closing: false,
            welcome_forced: false,
            _subs: subs,
        };
        ws.apply_settings(cx);
        if let Some(err) = app_state::state(cx).settings_error.clone() {
            ws.status_note(format!("Your settings file couldn't be read, so the defaults are used: {err}"), cx);
        }
        ws.sync_editors(window, cx);
        ws
    }

    // ---------- status ----------

    /// A short message in the status bar for a few seconds.
    pub fn status_note(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.note = Some(text.into());
        self.note_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(6)).await;
            this.update(cx, |ws, cx| {
                ws.note = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Tell the user something went wrong (a message box).
    pub fn alert(&mut self, message: impl Into<String>, window: &mut Window, cx: &mut Context<Self>) {
        let message = message.into();
        drop(window.prompt(PromptLevel::Warning, &message, None, &["OK"], cx));
    }

    // ---------- settings ----------

    /// The settings in effect: yours, with the open project's overrides on top.
    pub fn effective_settings(&self, cx: &App) -> Settings {
        let mut project = self.project_settings.clone();
        if let (Some(p), Some(root)) = (&project.python_path, &self.root) {
            project.python_path = Some(resolve_in_project(p, root));
        }
        effective_settings(app_state::settings(cx), &project)
    }

    /// Read the open project's settings again.
    pub(crate) fn reload_project_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(root) = self.root.clone() {
            self.load_project_settings(&root, window, cx);
        }
    }

    /// Read `<project>/.forge/settings.json` (a missing file means no overrides).
    fn load_project_settings(&mut self, folder: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let f = read_project_settings(folder);
        if let Some(err) = &f.error {
            self.alert(
                format!(
                    "This project's settings file couldn't be read, so it's ignored for now:\n{}\n\n{err}",
                    f.path.display()
                ),
                window,
                cx,
            );
        }
        self.project_settings = f.settings;
        self.project_settings_path = Some(f.path);
        self.apply_settings(cx);
    }

    /// Apply the settings to the editors (font size, tab size...).
    pub fn apply_settings(&mut self, cx: &mut Context<Self>) {
        let s = self.effective_settings(cx);
        let set_keys: Vec<&str> = {
            let v = serde_json::to_value(&self.project_settings).unwrap_or_default();
            PROJECT_SETTING_KEYS.iter().copied().filter(|k| v.get(*k).is_some_and(|x| !x.is_null())).collect()
        };
        let note = project_note(&set_keys, self.root.is_some());
        let user_shell = app_state::settings(cx).shell.clone();
        let shell_name = if user_shell.is_empty() {
            "System default".to_owned()
        } else {
            forge_term::shells::installed_shells()
                .into_iter()
                .find(|p| p.id == user_shell)
                .map_or(user_shell, |p| p.name)
        };
        self.settings_view.update(cx, |v, cx| {
            v.project_note = note;
            v.shell_name = shell_name;
            cx.notify();
        });
        if self.root.is_some() && self.python_path_seen.as_deref() != Some(s.python_path.as_str()) {
            let first = self.python_path_seen.is_none();
            self.python_path_seen = Some(s.python_path.clone());
            if !first {
                self.refresh_python_env(cx);
            }
        }
        let shell = s.shell.clone();
        self.output.update(cx, |o, cx| {
            o.font_size = s.font_size as f32 - 1.0;
            cx.notify();
        });
        self.terminal.update(cx, |t, cx| {
            t.font_size = s.font_size as f32;
            t.default_shell = shell;
            cx.notify();
        });
        for ed in &self.editors {
            ed.update(cx, |e, cx| {
                e.font_size = s.font_size as f32;
                e.line_height = (s.font_size as f32 * 1.45).round();
                e.tab_size = s.tab_size as usize;
                e.layout = None;
                cx.notify();
            });
        }
        cx.notify();
    }

    // ---------- keys ----------

    fn on_keystroke(&mut self, event: &KeystrokeEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        let m = k.modifiers;
        let key = key_string(m.control || m.platform, m.shift, m.alt, &k.key);
        if self.palette.read(cx).is_open() {
            let handled = self.palette.update(cx, |p, cx| p.handle_key(&key, window, cx));
            if handled || self.palette.read(cx).capturing() {
                cx.stop_propagation();
                return;
            }
        }
        if key.is_empty() {
            return;
        }
        // The Settings dialog handles its own keys; Escape closes it, Ctrl+, toggles it.
        if self.settings_view.read(cx).is_open() {
            if key == "Escape" {
                self.settings_view.update(cx, |v, cx| v.close(cx));
                cx.stop_propagation();
                return;
            }
            let keymap = app_state::state(cx).keymap.clone();
            if keymap.command_for_key(&key, |id| id == "settings.open").is_some() {
                cx.stop_propagation();
                self.run_command("settings.open", window, cx);
            }
            return;
        }
        // Alt+C / Alt+W / Alt+R toggle the search options while typing in project search.
        if self.search.read(cx).has_focus(window, cx) {
            let option = match key.as_str() {
                "Alt+C" => Some(FindOption::Case),
                "Alt+W" => Some(FindOption::Word),
                "Alt+R" => Some(FindOption::Regex),
                _ => None,
            };
            if let Some(o) = option {
                self.search.update(cx, |s, cx| s.toggle(o, cx));
                cx.stop_propagation();
                return;
            }
        }
        // Alt+C / Alt+W / Alt+R toggle the find options while typing in the find bar.
        if self.find_bar.read(cx).has_focus(window, cx) {
            let option = match key.as_str() {
                "Alt+C" => Some(FindOption::Case),
                "Alt+W" => Some(FindOption::Word),
                "Alt+R" => Some(FindOption::Regex),
                _ => None,
            };
            if let Some(o) = option {
                self.find_bar.update(cx, |f, cx| f.toggle(o, window, cx));
                cx.stop_propagation();
                return;
            }
        }
        if key == "Escape" && self.editing_layout {
            self.set_editing_layout(false, cx);
            cx.stop_propagation();
            return;
        }
        let palette_open = self.palette.read(cx).is_open();
        let keymap = app_state::state(cx).keymap.clone();
        let Some(command) = keymap.command_for_key(&key, |id| self.available(id, window, cx)) else { return };
        if palette_open && !PALETTE_TOGGLES.contains(&command.id) {
            return;
        }
        // A focused terminal gets every key except the shortcuts that make sense from there.
        if self.terminal.read(cx).has_focus(window) && !TERMINAL_PASSTHROUGH.contains(&command.id) {
            return;
        }
        cx.stop_propagation();
        self.run_command(command.id, window, cx);
    }

    // ---------- events from the parts ----------

    fn on_editor_event(&mut self, group: usize, event: &EditorEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            EditorEvent::Focused => {
                if self.tabs.active_group != group && group < self.tabs.groups.len() {
                    self.tabs.focus_group(group);
                    self.after_tabs_changed(window, cx);
                }
            }
            EditorEvent::Edited => {
                self.find_bar.update(cx, |f, cx| f.refresh(window, cx));
                self.backup_soon(cx);
                self.update_title(window, cx);
                cx.notify();
            }
            EditorEvent::CursorMoved => {
                self.save_state_soon(cx);
                cx.notify();
            }
            EditorEvent::Typed(_) | EditorEvent::CtrlClick(_) | EditorEvent::GutterClick(_) => {}
        }
    }

    fn on_tree_event(
        &mut self,
        _tree: &Entity<FileTree>,
        event: &FileTreeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            FileTreeEvent::Open(path) => self.open_path(path.clone(), window, cx),
            FileTreeEvent::Created { path, is_dir } => {
                if !is_dir {
                    self.open_path(path.clone(), window, cx);
                }
            }
            FileTreeEvent::Renamed { old, new } => self.path_renamed(old, new, window, cx),
            FileTreeEvent::Delete { path, is_dir } => self.delete_path(path.clone(), *is_dir, window, cx),
            FileTreeEvent::OpenRecent(folder) => self.open_recent(folder.clone(), window, cx),
            FileTreeEvent::Error(message) => self.alert(message.clone(), window, cx),
        }
    }

    // ---------- folders ----------

    /// Open the last project again, if the settings say so and it's still there.
    fn reopen_last(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !app_state::settings(cx).reopen_last_project {
            return;
        }
        if let Some(last) = app_state::state(cx).user.recent().first().cloned() {
            let p = PathBuf::from(&last);
            if p.is_dir() {
                self.open_folder(p, window, cx);
            }
        }
    }

    pub fn open_folder_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Folder".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                if let Some(p) = paths.into_iter().next() {
                    this.update_in(cx, |ws, window, cx| ws.open_folder(p, window, cx)).ok();
                }
            }
        })
        .detach();
    }

    pub fn open_recent(&mut self, folder: String, window: &mut Window, cx: &mut Context<Self>) {
        let p = PathBuf::from(&folder);
        if p.is_dir() {
            return self.open_folder(p, window, cx);
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("{folder} doesn't exist any more."),
            Some("Remove it from the recent folders?"),
            &["Remove", "Keep"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                this.update(cx, |ws, cx| {
                    let _ = app_state::state(cx).user.remove_recent(&folder);
                    ws.refresh_recent(cx);
                })
                .ok();
            }
        })
        .detach();
    }

    fn refresh_recent(&mut self, cx: &mut Context<Self>) {
        let recent = app_state::state(cx).user.recent();
        self.file_tree.update(cx, |t, cx| t.set_recent(recent, cx));
        cx.notify();
    }

    /// Open a folder that exists, with the tabs and layout it had last time.
    pub fn open_folder(&mut self, folder: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let folder = std::fs::canonicalize(&folder).map(strip_verbatim).unwrap_or(folder);
        if !folder.is_dir() {
            return self.alert(format!("{} isn't a folder.", folder.display()), window, cx);
        }
        // Unsaved files of the project being left: ask first.
        if self.tabs.docs().iter().any(|d| self.is_dirty(*d, cx)) {
            let answer = window.prompt(
                PromptLevel::Warning,
                "Some files have unsaved changes.",
                Some("Open the other folder anyway? Your changes would be lost."),
                &["Discard and Open", "Cancel"],
                cx,
            );
            cx.spawn_in(window, async move |this, cx| {
                if answer.await == Ok(0) {
                    this.update_in(cx, |ws, window, cx| {
                        ws.close_all_docs(cx);
                        ws.open_folder(folder, window, cx);
                    })
                    .ok();
                }
            })
            .detach();
            return;
        }
        self.save_state_now(cx);
        self.close_all_docs(cx);
        let path_str = folder.to_string_lossy().into_owned();
        match prepare_project_dir(&folder) {
            ProjectDirResult::Migrated => self.status_note("Moved project settings from .ideforfun/ to .forge/", cx),
            ProjectDirResult::Copied => {
                self.status_note("Copied project settings to .forge/ (the old .ideforfun/ folder couldn't be moved)", cx)
            }
            ProjectDirResult::Error => self.alert(
                "This project's settings are in .ideforfun/, but Forge couldn't move or copy them to .forge/ (a file may be in use). They aren't used this time: close other programs using the folder and open it again.",
                window,
                cx,
            ),
            ProjectDirResult::None | ProjectDirResult::Both => {}
        }
        let _ = app_state::state(cx).user.add_recent(&path_str);
        self.root = Some(folder.clone());
        self.load_project_settings(&folder, window, cx);
        self.run.config = None;
        self.run.last_task = None;
        self.load_run_file(&folder, window, cx);
        self.python_path_seen = Some(self.effective_settings(cx).python_path);
        self.refresh_python_env(cx);
        self.file_tree.update(cx, |t, cx| t.set_root(Some(folder.clone()), cx));
        self.terminal.update(cx, |t, cx| t.move_idle_to(Some(folder.clone()), cx));
        self.tests.update(cx, |t, cx| t.set_root(Some(folder.clone()), cx));
        self.search.update(cx, |s, cx| s.set_root(Some(folder.clone()), cx));
        self.refresh_recent(cx);
        self.restore_layout(&path_str, window, cx);
        self.offer_restore(&path_str, window, cx);
        self.update_title(window, cx);
        cx.notify();
    }

    fn close_all_docs(&mut self, cx: &mut Context<Self>) {
        for id in self.tabs.docs() {
            self.forget_doc(id, cx);
        }
        self.tabs.close_all();
    }

    /// Bring back the tabs, split, cursor positions and task the project had last time.
    fn restore_layout(&mut self, folder: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = app_state::state(cx).user.workspace(folder) else {
            self.sync_editors(window, cx);
            return;
        };
        self.restoring = true;
        let mut groups = Vec::new();
        let mut opened: Vec<(DocId, String)> = Vec::new();
        for g in &state.groups {
            let mut ids = Vec::new();
            for tab in &g.tabs {
                // Only files inside the project come back (the state could be from a moved folder).
                if !is_inside(&tab.path, folder) {
                    continue;
                }
                let id = match opened.iter().find(|(_, p)| same_path(p, &tab.path)) {
                    Some((id, _)) => Some(*id),
                    None => self
                        .load_doc(Path::new(&tab.path), false, cx)
                        .ok()
                        .flatten()
                        .inspect(|id| opened.push((*id, tab.path.clone()))),
                };
                if let Some(id) = id {
                    ids.push(id);
                }
            }
            let active =
                g.active.as_ref().and_then(|a| opened.iter().find(|(_, p)| same_path(p, a)).map(|(id, _)| *id));
            groups.push((ids, active));
        }
        self.tabs = Tabs::restore(groups, state.active_group);
        // Cursor and scroll position of each tab (preferring the side it was on).
        for (i, g) in self.tabs.groups.clone().iter().enumerate() {
            for id in &g.docs {
                let Some(doc) = self.docs.get(id).cloned() else { continue };
                let path = doc.read(cx).doc.path().to_owned();
                let tab =
                    state.groups.get(i).and_then(|sg| sg.tabs.iter().find(|t| same_path(&t.path, &path))).or_else(
                        || state.groups.iter().flat_map(|sg| sg.tabs.iter()).find(|t| same_path(&t.path, &path)),
                    );
                if let Some(tab) = tab.cloned() {
                    self.editors[i].update(cx, |e, cx| {
                        e.show(doc.clone(), cx);
                        e.restore_view(Pos::new(tab.line as usize, tab.col as usize), tab.scroll_top as f32, cx);
                    });
                }
            }
        }
        self.tasks.switch_to(&state.task);
        if state.run_config.as_ref().is_some_and(|c| self.run.file.configurations.iter().any(|x| &x.name == c)) {
            self.run.config = state.run_config.clone();
        }
        self.terminal.update(cx, |t, _| t.set_pending(state.terminals.clone()));
        self.restoring = false;
        self.after_tabs_changed(window, cx);
    }

    /// Offer to bring back unsaved changes to files in `folder` that were lost in a crash.
    fn offer_restore(&mut self, folder: &str, window: &mut Window, cx: &mut Context<Self>) {
        let found: Vec<BackupData> =
            self.backup_store.list().into_iter().filter(|b| is_inside(&b.path, folder)).collect();
        if found.is_empty() {
            return;
        }
        let rel = |p: &str| p.get(folder.len()..).unwrap_or(p).trim_start_matches(['/', '\\']).to_owned();
        let mut list: Vec<String> = found.iter().take(8).map(|b| format!("  • {}", rel(&b.path))).collect();
        if found.len() > 8 {
            list.push(format!("  …and {} more", found.len() - 8));
        }
        let message = format!(
            "Forge didn't close properly last time, and {} file{} unsaved changes:",
            found.len(),
            if found.len() > 1 { "s had" } else { " had" }
        );
        let detail = format!(
            "{}\n\nRestore those changes? (They open as unsaved; Ctrl+Z goes back to the file on disk.) Discarding throws them away.",
            list.join("\n")
        );
        let answer = window.prompt(PromptLevel::Warning, &message, Some(&detail), &["Restore", "Discard"], cx);
        cx.spawn_in(window, async move |this, cx| {
            let restore = answer.await == Ok(0);
            this.update_in(cx, |ws, window, cx| {
                for b in found {
                    if !restore {
                        let _ = ws.backup_store.remove(&b.path);
                        continue;
                    }
                    ws.open_path(PathBuf::from(&b.path), window, cx);
                    let Some(id) = ws.find_doc(&b.path, cx) else { continue };
                    let Some(doc) = ws.docs.get(&id).cloned() else { continue };
                    doc.update(cx, |d, cx| {
                        d.edit(None, cx, |doc| {
                            // One undoable step from the file on disk to the backed-up text.
                            if doc.buffer.lines().join("\n") != b.text {
                                let end = doc.buffer.end();
                                doc.history.transact(&mut doc.buffer, EditKind::Other, Pos::default(), |buf| {
                                    buf.delete(Pos::default(), end);
                                    buf.insert(Pos::default(), &b.text)
                                });
                            }
                            doc.set_eol(if b.eol == "CRLF" { Eol::CrLf } else { Eol::Lf });
                            doc.encoding = b.encoding;
                        });
                        cx.emit(DocumentEvent::Changed);
                    });
                }
                ws.after_tabs_changed(window, cx);
            })
            .ok();
        })
        .detach();
    }

    // ---------- documents ----------

    fn find_doc(&self, path: &str, cx: &App) -> Option<DocId> {
        self.docs.iter().find(|(_, d)| same_path(d.read(cx).doc.path(), path)).map(|(id, _)| *id)
    }

    pub(crate) fn is_dirty(&self, id: DocId, cx: &App) -> bool {
        self.docs.get(&id).is_some_and(|d| d.read(cx).doc.dirty())
    }

    pub(crate) fn active_doc(&self) -> Option<Entity<Document>> {
        self.tabs.active().and_then(|id| self.docs.get(&id).cloned())
    }

    /// Read a file into a new document. Ok(None): binary or huge and not forced.
    fn load_doc(&mut self, path: &Path, force: bool, cx: &mut Context<Self>) -> Result<Option<DocId>, String> {
        let file = read_text_file(path, force, None).map_err(|e| e.to_string())?;
        match file {
            TextFile::Text { text, encoding, size } => {
                let path_str = path.to_string_lossy().into_owned();
                let doc = cx.new(|_| Document::with_options(path_str, &text, encoding, size > LARGE_FILE_BYTES));
                Ok(Some(self.adopt_doc(doc, cx)))
            }
            TextFile::TooLarge { size } => Err(format!("The file is too large ({} MB)", size / 1_048_576)),
            TextFile::Binary { .. } | TextFile::Huge { .. } => Ok(None),
        }
    }

    /// Keep a new document (it gets an id; tabs refer to it by that).
    pub(crate) fn adopt_doc(&mut self, doc: Entity<Document>, cx: &mut Context<Self>) -> DocId {
        let id = self.next_doc;
        self.next_doc += 1;
        let sub = cx.subscribe(&doc, |this, _, _: &DocumentEvent, cx| {
            this.backup_soon(cx);
            cx.notify();
        });
        self._subs.push(sub);
        self.docs.insert(id, doc);
        id
    }

    /// Open a file in a tab of the focused group (or switch to it).
    pub fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.open_path_at(path, None, window, cx);
    }

    pub fn open_path_at(&mut self, path: PathBuf, line: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let path_str = path.to_string_lossy().into_owned();
        if let Some(id) = self.find_doc(&path_str, cx) {
            self.tabs.show(id);
            self.after_tabs_changed(window, cx);
            if let Some(l) = line {
                self.go_to_line(l, window, cx);
            }
            self.focus_editor(window, cx);
            return;
        }
        match self.load_doc(&path, false, cx) {
            Ok(Some(id)) => {
                self.tabs.show(id);
                self.after_tabs_changed(window, cx);
                if let Some(l) = line {
                    self.go_to_line(l, window, cx);
                }
                self.focus_editor(window, cx);
            }
            Ok(None) => {
                // Binary and very big files ask first.
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let binary = matches!(read_text_file(&path, false, None), Ok(TextFile::Binary { .. }));
                let question = if binary {
                    format!("{name} looks like a binary file (not text), so it may show as gibberish.")
                } else {
                    format!("{name} is large ({:.1} MB) and may be slow to edit.", size as f64 / 1_048_576.0)
                };
                let detail = if binary { "Open it as text anyway?" } else { "Open it anyway?" };
                let answer = window.prompt(PromptLevel::Warning, &question, Some(detail), &["Open", "Cancel"], cx);
                cx.spawn_in(window, async move |this, cx| {
                    if answer.await == Ok(0) {
                        this.update_in(cx, |ws, window, cx| match ws.load_doc(&path, true, cx) {
                            Ok(Some(id)) => {
                                ws.tabs.show(id);
                                ws.after_tabs_changed(window, cx);
                                ws.focus_editor(window, cx);
                            }
                            Ok(None) => {}
                            Err(err) => ws.alert(format!("Can't open {}:\n{err}", path.display()), window, cx),
                        })
                        .ok();
                    }
                })
                .detach();
            }
            Err(err) => self.alert(format!("Can't open {path_str}:\n{err}"), window, cx),
        }
    }

    pub fn go_to_line(&mut self, line: usize, window: &mut Window, cx: &mut Context<Self>) {
        let ed = self.editors[self.tabs.active_group].clone();
        ed.update(cx, |e, cx| e.go_to(line, 0, cx));
        window.focus(&ed.focus_handle(cx), cx);
    }

    pub(crate) fn focus_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.active().is_some() {
            let ed = &self.editors[self.tabs.active_group.min(1)];
            window.focus(&ed.focus_handle(cx), cx);
        }
    }

    /// Show each group's active file in its editor, and update everything that follows the active file.
    pub(crate) fn after_tabs_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_editors(window, cx);
        let active_editor = self.editors[self.tabs.active_group.min(1)].downgrade();
        self.find_bar.update(cx, |f, cx| {
            f.switch_editor(Some(active_editor), cx);
            f.refresh(window, cx);
        });
        let active_path = self.active_doc().map(|d| PathBuf::from(d.read(cx).doc.path()));
        self.file_tree.update(cx, |t, cx| t.highlight(active_path.as_deref(), cx));
        self.update_title(window, cx);
        self.backup_soon(cx);
        self.save_state_soon(cx);
        cx.notify();
    }

    fn sync_editors(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        for (i, ed) in self.editors.clone().iter().enumerate() {
            let doc = self.tabs.groups.get(i).and_then(|g| g.active).and_then(|id| self.docs.get(&id).cloned());
            ed.update(cx, |e, cx| match doc {
                Some(d) => {
                    if e.document().map(|x| x.entity_id()) != Some(d.entity_id()) {
                        e.show(d, cx);
                    }
                }
                None => e.clear(None, cx),
            });
        }
    }

    fn update_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let doc = self.active_doc().map(|d| {
            let d = d.read(cx);
            format!("{}{}", if d.doc.dirty() { "● " } else { "" }, d.doc.name())
        });
        let folder = self.root.as_ref().and_then(|r| r.file_name()).map(|n| n.to_string_lossy().into_owned());
        let title: Vec<String> = [doc, folder, Some("Forge".to_owned())].into_iter().flatten().collect();
        window.set_window_title(&title.join(" — "));
    }

    fn forget_doc(&mut self, id: DocId, cx: &mut Context<Self>) {
        if let Some(doc) = self.docs.remove(&id) {
            for ed in &self.editors {
                ed.update(cx, |e, _| e.forget(&doc));
            }
        }
    }

    // ---------- saving ----------

    /// Save a document (the active one by default). Returns false if it failed.
    pub fn save(&mut self, id: Option<DocId>, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(id) = id.or(self.tabs.active()) else { return false };
        let Some(doc) = self.docs.get(&id).cloned() else { return false };
        let (path, text, encoding) = {
            let d = &doc.read(cx).doc;
            (d.path().to_owned(), d.text(), d.encoding)
        };
        match write_text_file(Path::new(&path), &text, encoding) {
            Ok(()) => {
                doc.update(cx, |d, cx| {
                    d.doc.mark_saved();
                    cx.emit(DocumentEvent::Changed);
                    cx.notify();
                });
                self.after_save(&path, window, cx);
                self.after_tabs_changed(window, cx);
                true
            }
            Err(err) => {
                self.alert(format!("Couldn't save {path}:\n{err}"), window, cx);
                false
            }
        }
    }

    /// A file was saved: reload what it configures.
    fn after_save(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let (Some(p), Some(root)) = (self.project_settings_path.clone(), self.root.clone()) {
            if same_path(path, &p.to_string_lossy()) {
                self.load_project_settings(&root, window, cx);
            }
        }
        if let Some(root) = self.root.clone() {
            let name = Path::new(path).file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            let at_root =
                Path::new(path).parent().is_some_and(|d| same_path(&d.to_string_lossy(), &root.to_string_lossy()));
            if at_root && (name == "requirements.txt" || name == "pyproject.toml") {
                self.refresh_python_env(cx);
            }
        }
        // Project search reads files from disk: refresh its results after saving.
        if self.tasks.is_shown("search") {
            self.search.update(cx, |s, cx| s.refresh(cx));
        }
        if forge_core::testing::is_test_file(path) {
            self.tests.update(cx, |t, cx| t.discover(cx));
        }
        if let (Some(p), Some(root)) = (self.run.file_path.clone(), self.root.clone()) {
            if same_path(path, &p.to_string_lossy()) {
                self.load_run_file(&root, window, cx);
            }
        }
        let st = app_state::state(cx);
        if same_path(path, &st.user.keybindings_path().to_string_lossy()) {
            let (overrides, err) = st.user.keybindings();
            cx.global_mut::<AppState>().keymap.set_overrides(overrides);
            if let Some(e) = err {
                self.status_note(
                    format!("Your keyboard shortcuts file couldn't be read, so the defaults are used: {e}"),
                    cx,
                );
            }
        } else if same_path(path, &st.user.settings_path().to_string_lossy()) {
            let (settings, err) = st.user.settings();
            let theme_changed = settings.theme != st.settings.theme;
            cx.global_mut::<AppState>().settings = settings;
            if theme_changed {
                app_state::apply_theme(cx);
            }
            self.apply_settings(cx);
            if let Some(e) = err {
                self.status_note(format!("Your settings file couldn't be read, so the defaults are used: {e}"), cx);
            }
        }
        if let Some(dir) = Path::new(path).parent() {
            self.file_tree.update(cx, |t, cx| t.refresh(dir, cx));
        }
    }

    // ---------- closing ----------

    /// Close a tab; a file with unsaved changes asks first (when its last tab closes).
    pub fn close_tab(&mut self, id: DocId, group: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_last_tab(id, group) && self.is_dirty(id, cx) {
            let name = self.docs.get(&id).map(|d| d.read(cx).doc.name().to_owned()).unwrap_or_default();
            let answer = window.prompt(
                PromptLevel::Warning,
                &format!("Save changes to {name}?"),
                Some("Your changes will be lost if you don't save them."),
                &["Save", "Don't Save", "Cancel"],
                cx,
            );
            cx.spawn_in(window, async move |this, cx| {
                let choice = answer.await;
                this.update_in(cx, |ws, window, cx| match choice {
                    Ok(0) => {
                        if ws.save(Some(id), window, cx) {
                            ws.close_tab_now(id, group, window, cx);
                        }
                    }
                    Ok(1) => ws.close_tab_now(id, group, window, cx),
                    _ => {}
                })
                .ok();
            })
            .detach();
            return;
        }
        self.close_tab_now(id, group, window, cx);
    }

    fn close_tab_now(&mut self, id: DocId, group: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.close(id, group);
        if !self.tabs.contains(id) {
            self.forget_doc(id, cx);
        }
        self.after_tabs_changed(window, cx);
        self.focus_editor(window, cx);
    }

    /// The window is asked to close: unsaved files ask first.
    fn should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.save_state_now(cx);
        self.save_layouts_now(cx);
        let dirty: Vec<DocId> = self.tabs.docs().into_iter().filter(|d| self.is_dirty(*d, cx)).collect();
        if dirty.is_empty() || self.closing {
            return true;
        }
        let names: Vec<String> =
            dirty.iter().filter_map(|id| self.docs.get(id)).map(|d| d.read(cx).doc.name().to_owned()).collect();
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!(
                "Save changes to {} before closing?",
                if names.len() == 1 { names[0].clone() } else { format!("{} files", names.len()) }
            ),
            Some(&names.join("\n")),
            &["Save All", "Don't Save", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let choice = answer.await;
            this.update_in(cx, |ws, window, cx| {
                let close = match choice {
                    Ok(0) => dirty.iter().all(|id| ws.save(Some(*id), window, cx)),
                    Ok(1) => true,
                    _ => false,
                };
                if close {
                    ws.closing = true;
                    // Thrown-away changes don't need their crash backups.
                    for id in &dirty {
                        if let Some(d) = ws.docs.get(id) {
                            let _ = ws.backup_store.remove(d.read(cx).doc.path());
                        }
                    }
                    window.remove_window();
                }
            })
            .ok();
        })
        .detach();
        false
    }

    // ---------- files renamed or deleted ----------

    /// A file or folder was renamed on disk: point the affected tabs at the new paths.
    fn path_renamed(&mut self, old: &Path, new: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let (old_s, new_s) = (old.to_string_lossy().into_owned(), new.to_string_lossy().into_owned());
        for doc in self.docs.values() {
            let path = doc.read(cx).doc.path().to_owned();
            if let Some(moved) = renamed_path(&path, &old_s, &new_s) {
                doc.update(cx, |d, cx| {
                    d.doc.set_path(moved);
                    cx.emit(DocumentEvent::Changed);
                    cx.notify();
                });
            }
        }
        self.after_tabs_changed(window, cx);
    }

    /// Ask, move to the Recycle Bin / Trash, and close its tabs.
    fn delete_path(&mut self, path: PathBuf, is_dir: bool, window: &mut Window, cx: &mut Context<Self>) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let what = if is_dir { format!("the folder {name} and everything in it") } else { name.clone() };
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Delete {what}?"),
            Some("It goes to the Recycle Bin / Trash, so you can get it back."),
            &["Delete", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update_in(cx, |ws, window, cx| {
                if let Err(err) = trash_entry(&path) {
                    return ws.alert(format!("Couldn't delete {}:\n{err}", path.display()), window, cx);
                }
                let p = path.to_string_lossy().into_owned();
                let gone: Vec<DocId> =
                    ws.docs.iter().filter(|(_, d)| is_inside(d.read(cx).doc.path(), &p)).map(|(id, _)| *id).collect();
                for id in &gone {
                    if let Some(d) = ws.docs.get(id) {
                        let _ = ws.backup_store.remove(d.read(cx).doc.path());
                    }
                }
                ws.tabs.discard(&gone);
                for id in gone {
                    ws.forget_doc(id, cx);
                }
                ws.file_tree.update(cx, |t, cx| t.deleted(&path, window, cx));
                ws.after_tabs_changed(window, cx);
            })
            .ok();
        })
        .detach();
    }

    // ---------- remembering the project's state ----------

    fn capture_state(&self, cx: &App) -> WorkspaceState {
        let groups = self
            .tabs
            .groups
            .iter()
            .enumerate()
            .map(|(i, g)| GroupState {
                tabs: g
                    .docs
                    .iter()
                    .filter_map(|id| self.docs.get(id))
                    .map(|d| {
                        let view = self.editors[i].read(cx).view_of(d);
                        DocState {
                            path: d.read(cx).doc.path().to_owned(),
                            line: view.as_ref().map_or(0, |v| v.cursor.line as u64),
                            col: view.as_ref().map_or(0, |v| v.cursor.col as u64),
                            scroll_top: view.as_ref().map_or(0, |v| v.scroll_top.max(0.0) as u64),
                        }
                    })
                    .collect(),
                active: g.active.and_then(|id| self.docs.get(&id)).map(|d| d.read(cx).doc.path().to_owned()),
            })
            .collect();
        WorkspaceState {
            groups,
            active_group: self.tabs.active_group,
            task: self.tasks.current().to_owned(),
            terminals: self.terminal.read(cx).shell_ids(),
            run_config: self.run.config.clone(),
            ..Default::default()
        }
    }

    /// Save the project's tabs and task a moment after they change.
    pub(crate) fn save_state_soon(&mut self, cx: &mut Context<Self>) {
        if self.root.is_none() || self.restoring {
            return;
        }
        self.state_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            this.update(cx, |ws, cx| ws.save_state_now(cx)).ok();
        }));
    }

    fn save_state_now(&mut self, cx: &mut Context<Self>) {
        self.state_task = None;
        let Some(root) = self.root.clone() else { return };
        if self.restoring {
            return;
        }
        let state = self.capture_state(cx);
        let _ = app_state::state(cx).user.save_workspace(&root.to_string_lossy(), &state);
    }

    /// The dock changed: save the arrangements a moment later.
    pub(crate) fn layouts_changed(&mut self, cx: &mut Context<Self>) {
        self.save_state_soon(cx);
        if !self.tasks.dirty {
            return;
        }
        self.layout_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            this.update(cx, |ws, cx| ws.save_layouts_now(cx)).ok();
        }));
        cx.notify();
    }

    fn save_layouts_now(&mut self, cx: &mut Context<Self>) {
        self.layout_task = None;
        if !self.tasks.dirty {
            return;
        }
        self.tasks.dirty = false;
        if let Ok(value) = serde_json::to_value(self.tasks.saved()) {
            let _ = app_state::state(cx).user.save_layouts(&value);
        }
    }

    // ---------- crash backups ----------

    fn backup_soon(&mut self, cx: &mut Context<Self>) {
        let open: Vec<DocSnapshot> = self
            .docs
            .iter()
            .map(|(id, d)| {
                let doc = &d.read(cx).doc;
                DocSnapshot {
                    id: *id,
                    path: doc.path().to_owned(),
                    dirty: doc.dirty(),
                    key: format!("{}|{}|{:?}|{:?}", doc.path(), doc.version, doc.eol(), doc.encoding),
                }
            })
            .collect();
        for action in self.backups.sync(&open, now_ms()) {
            match action {
                BackupAction::Remove { path } => {
                    let _ = self.backup_store.remove(&path);
                }
                BackupAction::Write { id, key } => {
                    let Some(d) = self.docs.get(&id) else { continue };
                    let doc = &d.read(cx).doc;
                    let data = BackupData {
                        path: doc.path().to_owned(),
                        text: doc.buffer.lines().join("\n"),
                        encoding: doc.encoding,
                        eol: if doc.eol() == Eol::CrLf { "CRLF".into() } else { "LF".into() },
                        time: now_ms(),
                    };
                    // Disk full or similar: try again on the next change.
                    if self.backup_store.write(&data).is_ok() {
                        self.backups.written(id, &data.path, &key);
                    }
                }
            }
        }
        if let Some(due) = self.backups.next_due() {
            let wait = due.saturating_sub(now_ms()) + 5;
            self.backup_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(wait)).await;
                this.update(cx, |ws, cx| ws.backup_soon(cx)).ok();
            }));
        }
    }

    // ---------- layout ----------

    pub(crate) fn set_editing_layout(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.editing_layout == on {
            return;
        }
        self.editing_layout = on;
        self.drop_target = None;
        if !on {
            self.save_layouts_now(cx);
        }
        cx.notify();
    }

    /// Run tests: save files, find the interpreter, then run.
    pub(crate) fn run_tests(&mut self, which: Which, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        if self.tests.read(cx).is_running() {
            return;
        }
        self.reveal_panel("tests", cx);
        if !self.save_all_dirty(window, cx) {
            self.tests.update(cx, |t, cx| t.say("Test run cancelled: a file could not be saved.", cx));
            return;
        }
        let configured = Some(self.effective_settings(cx).python_path).filter(|p| !p.is_empty());
        let known = self.pyenv.current.as_ref().map(|i| i.path.clone());
        cx.spawn_in(window, async move |this, cx| {
            let python = match known {
                Some(p) => Some(p),
                None => {
                    cx.background_executor()
                        .spawn(async move { forge_run::pyenv::resolve_python(&root, configured.as_deref()) })
                        .await
                }
            };
            this.update(cx, |ws, cx| {
                ws.tests.update(cx, |t, cx| match python {
                    Some(p) => t.run(which, p, cx),
                    None => t.say("No Python interpreter found: pick one with 🐍 in the status bar.", cx),
                });
            })
            .ok();
        })
        .detach();
    }

    /// The view of a panel (created the first time it's shown).
    pub(crate) fn panel_view(&mut self, id: &'static str, cx: &mut Context<Self>) -> AnyView {
        if id == "files" {
            return self.file_tree.clone().into();
        }
        if id == "output" {
            return self.output.clone().into();
        }
        if id == "search" {
            return self.search.clone().into();
        }
        if id == "tests" {
            self.tests.update(cx, |t, cx| t.shown(cx));
            return self.tests.clone().into();
        }
        if id == "terminal" {
            // The first time it shows, its terminals start.
            self.terminal.update(cx, |t, cx| t.ensure_started(cx));
            return self.terminal.clone().into();
        }
        self.panels
            .entry(id)
            .or_insert_with(|| cx.new(|_| Placeholder { title: panels::title(id).into() }).into())
            .clone()
    }

    /// The commands the palette lists now (those that can run).
    fn palette_commands(&self, window: &mut Window, cx: &mut Context<Self>) -> Vec<CommandItem> {
        let keymap = app_state::state(cx).keymap.clone();
        keymap
            .commands()
            .iter()
            .filter(|c| self.available(c.id, window, cx))
            .map(|c| CommandItem { id: c.id, title: c.title, key: keymap.keys_of(c.id).into_iter().next() })
            .collect()
    }

    pub fn open_palette(&mut self, prefix: &str, window: &mut Window, cx: &mut Context<Self>) {
        let commands = self.palette_commands(window, cx);
        let line_count = self.active_doc().map(|d| d.read(cx).doc.buffer.line_count());
        self.palette.update(cx, |p, cx| p.open_quick(prefix, commands, line_count, window, cx));
        if prefix.is_empty() {
            if let Some(root) = self.root.clone() {
                let palette = self.palette.downgrade();
                cx.spawn(async move |_, cx| {
                    let files = cx.background_executor().spawn(async move { list_files(&root, 20_000) }).await;
                    palette.update(cx, |p, cx| p.set_files(files, cx)).ok();
                })
                .detach();
            }
        }
        cx.notify();
    }

    pub fn choose(
        &mut self,
        prompt: impl Into<SharedString>,
        choices: Vec<Choice>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prompt = prompt.into();
        self.palette.update(cx, |p, cx| p.choose(prompt, choices, window, cx));
        cx.notify();
    }
}

/// A project's relative path ("venv/bin/python") made absolute inside the project.
fn resolve_in_project(path: &str, root: &Path) -> String {
    let b = path.as_bytes();
    let absolute = (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':') || path.starts_with(['/', '\\', '~']);
    if path.is_empty() || absolute || !path.contains(['/', '\\']) {
        return path.to_owned();
    }
    let rest = path.trim_start_matches("./").trim_start_matches(".\\");
    root.join(rest).to_string_lossy().into_owned()
}

/// `canonicalize` on Windows gives `\\?\C:\...`: use the plain form.
fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => p,
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<crate::theme::ForgeTheme>().0.clone();
        let dock = self.render_dock(window, cx);
        div()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .text_color(t.fg)
            .font_family(crate::theme::UI_FONT)
            .text_size(px(13.0))
            .child(self.render_toolbar(cx))
            .when(self.editing_layout, |d| d.child(self.render_edit_bar(cx)))
            .child(div().flex_1().min_h_0().flex().child(dock))
            .child(self.render_status_bar(window, cx))
            .child(self.settings_view.clone())
            .child(self.palette.clone())
    }
}
