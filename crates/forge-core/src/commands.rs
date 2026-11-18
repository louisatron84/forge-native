//! Forge's commands: every action (save, run, open settings...) with a title
//! and default shortcuts, in one table that the key handler, the command
//! palette, the menus and the Keyboard Shortcuts list all use. Replaces
//! `src/renderer/keymap.ts` and the logic of `src/renderer/shortcuts.ts`;
//! the ids are the old app's, so keybindings.json files carry over.

use crate::keybindings::{find_conflicts, KeyOverrides};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandInfo {
    pub id: &'static str,
    pub title: &'static str,
    /// Default shortcuts like "Ctrl+S" (the first is the one shown).
    pub keys: &'static [&'static str],
}

const fn c(id: &'static str, title: &'static str, keys: &'static [&'static str]) -> CommandInfo {
    CommandInfo { id, title, keys }
}

/// Every command, in the order the old app registered them (the first command bound to a key wins).
pub const COMMANDS: &[CommandInfo] = &[
    c("nav.references", "Find All References", &["Shift+F12"]),
    c("editor.renameSymbol", "Rename Symbol…", &["F2"]),
    c("nav.symbol", "Go to Symbol in File…", &["Ctrl+Shift+O"]),
    c("view.outline", "Show Outline", &[]),
    c("nav.implementations", "Go to Implementations (subclasses / overrides)", &["Ctrl+F12"]),
    c("nav.typeDefinition", "Go to Type Definition", &[]),
    c("nav.projectSymbol", "Go to Symbol in Project…", &["Ctrl+T"]),
    c("nav.callHierarchy", "Show Call Hierarchy", &["Shift+Alt+H"]),
    c("panel.calls", "Show Calls Panel", &[]),
    c("panel.references", "Show References", &[]),
    c("file.open", "Open File…", &[]),
    c("file.openFolder", "Open Folder…", &["Ctrl+O"]),
    c("editor.toggleComment", "Toggle Comment", &[]),
    c("file.openFolderByPath", "Open Folder by Path…", &[]),
    c("file.openRecent", "Open Recent Folder…", &["Ctrl+R"]),
    c("file.clearRecent", "Clear Recent Folders", &[]),
    c("file.save", "Save", &["Ctrl+S"]),
    c("file.saveAll", "Save All", &[]),
    c("file.undoGenerated", "Undo Generated Write", &[]),
    c("file.newProject", "New Project from Template…", &[]),
    c("file.saveTemplate", "Save Project as Template…", &[]),
    c("file.new", "New File…", &["Ctrl+N"]),
    c("file.newFolder", "New Folder…", &[]),
    c("help.welcome", "Welcome", &[]),
    c("file.rename", "Rename Current File…", &[]),
    c("file.delete", "Delete Current File", &[]),
    c("tab.close", "Close Tab", &["Ctrl+W"]),
    c("tab.next", "Next Tab", &["Ctrl+Tab", "Ctrl+PageDown"]),
    c("tab.prev", "Previous Tab", &["Ctrl+Shift+Tab", "Ctrl+PageUp"]),
    c("find.open", "Find", &["Ctrl+F"]),
    c("find.replace", "Find and Replace", &["Ctrl+H"]),
    c("find.close", "Close Find", &["Escape"]),
    c("run.smart", "Run (or Debug if there are breakpoints) / Continue", &["F5"]),
    c("run.file", "Run Python File (without debugging)", &["Ctrl+F5"]),
    c("run.selectConfig", "Select Run Configuration…", &[]),
    c("run.editConfigs", "Edit Run Configurations and Tasks (run.json)", &[]),
    c("tasks.run", "Run Task…", &["Ctrl+Shift+B"]),
    c("tasks.rerun", "Run Last Task Again", &[]),
    c("run.stop", "Stop Running / Debugging", &["Shift+F5"]),
    c("debug.start", "Start Debugging", &[]),
    c("debug.restart", "Restart Debugging", &["Ctrl+Shift+F5"]),
    c("debug.stepOver", "Step Over", &["F10"]),
    c("debug.stepInto", "Step Into", &["F11"]),
    c("debug.stepOut", "Step Out", &["Shift+F11"]),
    c("debug.pause", "Pause", &[]),
    c("debug.toggleBreakpoint", "Toggle Breakpoint", &["F9"]),
    c("debug.editBreakpoint", "Edit Breakpoint (condition, hit count, log message)…", &[]),
    c("debug.toggleRaised", "Debug: Pause on Raised Exceptions (on / off)", &[]),
    c("debug.clearBreakpoints", "Remove All Breakpoints", &[]),
    c("editor.toggleMinimap", "Toggle Minimap", &[]),
    c("view.split", "Split Editor (or switch sides)", &["Ctrl+\\"]),
    c("view.moveToOtherGroup", "Move Tab to the Other Side", &[]),
    c("view.focusLeft", "Focus Left Editor", &["Ctrl+1"]),
    c("view.focusRight", "Focus Right Editor", &["Ctrl+2"]),
    c("editor.fold", "Fold Block (Ctrl+Shift+[)", &[]),
    c("editor.unfold", "Unfold Block (Ctrl+Shift+])", &[]),
    c("editor.foldAll", "Fold All", &[]),
    c("editor.unfoldAll", "Unfold All", &[]),
    c("panel.debug", "Show Debug Panel", &[]),
    c("panel.toggle", "Toggle Bottom Panel", &["Ctrl+J"]),
    c("panel.output", "Show Output", &[]),
    c("nav.definition", "Go to Definition", &["F12"]),
    c("nav.back", "Go Back", &["Alt+ArrowLeft"]),
    c("panel.problems", "Show Problems", &["Ctrl+Shift+M"]),
    c("python.restartServer", "Restart Python Language Server", &[]),
    c("terminal.toggle", "Toggle Terminal", &["Ctrl+`"]),
    c("settings.open", "Open Settings", &["Ctrl+,"]),
    c("view.zoomIn", "Increase Font Size", &["Ctrl+="]),
    c("view.zoomOut", "Decrease Font Size", &["Ctrl+-"]),
    c("view.toggleTheme", "Toggle Plain Theme", &[]),
    c("settings.project", "Open Project Settings", &[]),
    c("api.addOpenApi", "Add API from OpenAPI Description…", &[]),
    c("help.checkUpdates", "Check for Updates…", &[]),
    c("help.releaseNotes", "Release Notes (What's New)", &[]),
    c("nav.projectMap", "Project Map (find a file visually)", &["Ctrl+Alt+M"]),
    c("view.pipeline", "Show Pipeline (data flow of a function)", &[]),
    c("panel.memory", "Show Memory", &[]),
    c("run.memory", "Run with Memory Tracking", &[]),
    c("panel.network", "Show Network (web requests while running)", &[]),
    c("view.database", "Show Database (tables, keys and connections)", &[]),
    c("view.projectGraph", "Show Project Graph", &["Ctrl+Alt+G"]),
    c("connect.addApiBox", "Add API Box (Connect)", &[]),
    c("git.acceptCurrent", "Merge Conflict: Accept Current", &[]),
    c("git.acceptIncoming", "Merge Conflict: Accept Incoming", &[]),
    c("git.acceptBoth", "Merge Conflict: Accept Both", &[]),
    c("git.nextConflict", "Merge Conflict: Next", &[]),
    c("git.prevConflict", "Merge Conflict: Previous", &[]),
    c("git.history", "Git: Show History", &[]),
    c("git.fileHistory", "Git: File History", &[]),
    c("git.toggleBlame", "Git: Toggle Blame (who changed each line)", &[]),
    c("git.show", "Show Source Control (Git)", &["Ctrl+Shift+G"]),
    c("git.diffFile", "Git: Show Changes in This File", &[]),
    c("git.commit", "Git: Commit", &[]),
    c("git.branch", "Git: Switch Branch…", &[]),
    c("git.pull", "Git: Pull", &[]),
    c("git.push", "Git: Push", &[]),
    c("git.fetch", "Git: Fetch", &[]),
    c("git.stageAll", "Git: Stage All Changes", &[]),
    c("git.init", "Git: Initialize Repository", &[]),
    c("search.project", "Search in Project", &["Ctrl+Shift+F"]),
    c("view.files", "Show Files", &["Ctrl+Shift+E"]),
    c("python.selectInterpreter", "Select Python Interpreter…", &[]),
    c("python.createVenv", "Create Virtual Environment (.venv)…", &[]),
    c("python.packages", "Python Packages…", &[]),
    c("python.installRequirements", "Install Requirements (requirements.txt)", &[]),
    c("python.checkDeps", "Check Dependencies", &[]),
    c("tests.show", "Show Tests", &[]),
    c("tests.runAll", "Run All Tests", &[]),
    c("tests.runFailed", "Run Failed Tests Again", &[]),
    c("tests.runFile", "Run Tests in This File", &[]),
    c("tests.runAtCursor", "Run Test at Cursor", &[]),
    c("tests.stop", "Stop Tests", &[]),
    c("editor.quickFix", "Quick Fix…", &["Ctrl+."]),
    c("editor.format", "Format Document", &["Shift+Alt+F"]),
    c("editor.formatSelection", "Format Selection", &[]),
    c("editor.organizeImports", "Organize Imports", &["Shift+Alt+O"]),
    c("editor.fixAll", "Fix All Auto-fixable Problems (Ruff)", &[]),
    c("python.installRuff", "Install Ruff (linter and formatter)", &[]),
    c("keys.show", "Keyboard Shortcuts…", &[]),
    c("keys.openFile", "Edit Keyboard Shortcuts File (keybindings.json)", &[]),
    c("file.lineEndings", "Change Line Endings (LF / CRLF)…", &[]),
    c("file.reopenEncoding", "Reopen with Encoding…", &[]),
    c("file.saveEncoding", "Save with Encoding…", &[]),
    c("terminal.new", "New Terminal", &["Ctrl+Shift+`"]),
    c("terminal.newWithShell", "New Terminal with Shell…", &[]),
    c("terminal.kill", "Kill Terminal", &[]),
    c("terminal.next", "Next Terminal", &[]),
    c("palette.commands", "Show All Commands", &["Ctrl+Shift+P"]),
    c("palette.files", "Go to File…", &["Ctrl+P"]),
    c("palette.line", "Go to Line…", &["Ctrl+G"]),
    c("task.code", "Switch to Code", &["Ctrl+Alt+1"]),
    c("task.connect", "Switch to Connect", &["Ctrl+Alt+2"]),
    c("task.debug", "Switch to Debug", &["Ctrl+Alt+3"]),
    c("task.run", "Switch to Run", &["Ctrl+Alt+4"]),
    c("view.resetLayout", "Reset Layout of This Task", &[]),
    c("view.editLayout", "Edit Layout (move, close and add panels)", &[]),
];

pub fn command(id: &str) -> Option<&'static CommandInfo> {
    COMMANDS.iter().find(|c| c.id == id)
}

/// A shortcut as the old app wrote it ("Ctrl+Shift+P", "F5", "Alt+ArrowLeft"),
/// from the pressed key's name (GPUI's names: "p", "f5", "left", "pageup"...)
/// and modifiers. Cmd counts as Ctrl. Empty for a bare modifier.
pub fn key_string(ctrl: bool, shift: bool, alt: bool, key: &str) -> String {
    let name = match key {
        "" | "control" | "shift" | "alt" | "platform" | "function" | "fn" | "cmd" | "super" => return String::new(),
        "up" => "ArrowUp".to_owned(),
        "down" => "ArrowDown".to_owned(),
        "left" => "ArrowLeft".to_owned(),
        "right" => "ArrowRight".to_owned(),
        "pageup" => "PageUp".to_owned(),
        "pagedown" => "PageDown".to_owned(),
        "escape" => "Escape".to_owned(),
        "enter" => "Enter".to_owned(),
        "tab" => "Tab".to_owned(),
        "space" => "Space".to_owned(),
        "backspace" => "Backspace".to_owned(),
        "delete" => "Delete".to_owned(),
        "insert" => "Insert".to_owned(),
        "home" => "Home".to_owned(),
        "end" => "End".to_owned(),
        k if k.len() > 1 && k.starts_with('f') && k[1..].parse::<u32>().is_ok() => k.to_uppercase(),
        k if k.chars().count() == 1 => k.to_uppercase(),
        k => {
            let mut c = k.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        }
    };
    let mut parts: Vec<&str> = Vec::new();
    if ctrl {
        parts.push("Ctrl");
    }
    if shift {
        parts.push("Shift");
    }
    if alt {
        parts.push("Alt");
    }
    let mut out = parts.join("+");
    if !out.is_empty() {
        out.push('+');
    }
    out.push_str(&name);
    out
}

/// The commands and your shortcut changes (from keybindings.json).
#[derive(Clone, Debug)]
pub struct Keymap {
    commands: &'static [CommandInfo],
    overrides: KeyOverrides,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::new(COMMANDS)
    }
}

impl Keymap {
    pub fn new(commands: &'static [CommandInfo]) -> Self {
        Self { commands, overrides: KeyOverrides::new() }
    }

    pub fn commands(&self) -> &'static [CommandInfo] {
        self.commands
    }

    pub fn get(&self, id: &str) -> Option<&'static CommandInfo> {
        self.commands.iter().find(|c| c.id == id)
    }

    /// Use these keys instead of the defaults for the commands listed.
    pub fn set_overrides(&mut self, overrides: KeyOverrides) {
        self.overrides = overrides;
    }

    pub fn overrides(&self) -> &KeyOverrides {
        &self.overrides
    }

    /// The keys a command has when nothing is changed.
    pub fn default_keys(&self, id: &str) -> Vec<String> {
        self.get(id).map(|c| c.keys.iter().map(|k| (*k).to_owned()).collect()).unwrap_or_default()
    }

    /// The keys that run a command now (your change, or the default).
    pub fn keys_of(&self, id: &str) -> Vec<String> {
        self.overrides.get(id).cloned().unwrap_or_else(|| self.default_keys(id))
    }

    /// The command a key would run right now (`available` says whether a command can run now).
    pub fn command_for_key(&self, key: &str, available: impl Fn(&str) -> bool) -> Option<&'static CommandInfo> {
        if key.is_empty() {
            return None;
        }
        self.commands.iter().find(|c| self.keys_of(c.id).iter().any(|k| k == key) && available(c.id))
    }

    /// First shortcut of every command, for the menus (None = none).
    pub fn menu_keys(&self) -> BTreeMap<&'static str, Option<String>> {
        self.commands.iter().map(|c| (c.id, self.keys_of(c.id).into_iter().next())).collect()
    }

    /// Keys shared by several commands: key -> command ids.
    pub fn conflicts(&self) -> Vec<(String, Vec<String>)> {
        let all: Vec<(String, Vec<String>)> =
            self.commands.iter().map(|c| (c.id.to_owned(), self.keys_of(c.id))).collect();
        find_conflicts(&all)
    }

    /// Commands (other than `id`) that `key` runs now.
    pub fn users_of(&self, key: &str, except: &str) -> Vec<&'static CommandInfo> {
        self.commands.iter().filter(|c| c.id != except && self.keys_of(c.id).iter().any(|k| k == key)).collect()
    }

    /// The overrides after changing one command's keys (None = back to the
    /// default), optionally taking `key` away from other commands. Entries
    /// that equal the default aren't kept.
    pub fn changed(&self, id: &str, keys: Option<Vec<String>>, take_from: &[&str], key: &str) -> KeyOverrides {
        let mut next = self.overrides.clone();
        match keys {
            None => {
                next.remove(id);
            }
            Some(k) => {
                next.insert(id.to_owned(), k);
            }
        }
        for other in take_from {
            let kept: Vec<String> = self.keys_of(other).into_iter().filter(|k| k != key).collect();
            next.insert((*other).to_owned(), kept);
        }
        next.retain(|cid, k| *k != self.default_keys(cid));
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST: &[CommandInfo] = &[c("a", "A", &["Ctrl+R"]), c("b", "B", &["F1", "F2"])];

    #[test]
    fn key_strings_like_the_old_app() {
        assert_eq!(key_string(true, false, false, "s"), "Ctrl+S");
        assert_eq!(key_string(true, true, false, "p"), "Ctrl+Shift+P");
        assert_eq!(key_string(false, false, false, "f5"), "F5");
        assert_eq!(key_string(false, true, false, "f5"), "Shift+F5");
        assert_eq!(key_string(true, false, false, "`"), "Ctrl+`");
        assert_eq!(key_string(true, true, false, "tab"), "Ctrl+Shift+Tab");
        assert_eq!(key_string(true, false, false, ","), "Ctrl+,");
        assert_eq!(key_string(false, false, true, "left"), "Alt+ArrowLeft");
        assert_eq!(key_string(true, false, false, "\\"), "Ctrl+\\");
        assert_eq!(key_string(true, false, false, "control"), "");
    }

    #[test]
    fn finds_the_command_for_a_key_skipping_unavailable_ones() {
        let k = Keymap::new(TEST);
        assert_eq!(k.command_for_key("F2", |_| true).map(|c| c.id), Some("b"));
        assert_eq!(k.command_for_key("F3", |_| true), None);
        assert_eq!(k.command_for_key("F1", |id| id != "b"), None);
        assert_eq!(k.command_for_key("", |_| true), None);
    }

    #[test]
    fn replaces_the_default_keys_of_changed_commands_only() {
        let mut k = Keymap::new(TEST);
        k.set_overrides(KeyOverrides::from([("a".to_owned(), vec!["Ctrl+E".to_owned()])]));
        assert_eq!(k.keys_of("a"), ["Ctrl+E"]);
        assert_eq!(k.keys_of("b"), ["F1", "F2"]);
        assert_eq!(k.default_keys("a"), ["Ctrl+R"]);
        assert_eq!(k.command_for_key("Ctrl+R", |_| true), None);
        assert_eq!(k.command_for_key("Ctrl+E", |_| true).map(|c| c.id), Some("a"));
    }

    #[test]
    fn an_empty_list_removes_the_shortcut() {
        let mut k = Keymap::new(TEST);
        k.set_overrides(KeyOverrides::from([("b".to_owned(), vec![])]));
        assert_eq!(k.command_for_key("F1", |_| true), None);
        assert_eq!(k.menu_keys(), BTreeMap::from([("a", Some("Ctrl+R".to_owned())), ("b", None)]));
    }

    #[test]
    fn changing_keys_drops_entries_equal_to_the_default_and_can_take_a_key_away() {
        let k = Keymap::new(TEST);
        let o = k.changed("a", Some(vec!["F1".into()]), &["b"], "F1");
        assert_eq!(
            o,
            KeyOverrides::from([("a".to_owned(), vec!["F1".to_owned()]), ("b".to_owned(), vec!["F2".to_owned()])])
        );
        assert_eq!(k.changed("a", Some(vec!["Ctrl+R".into()]), &[], ""), KeyOverrides::new());
        let mut k2 = Keymap::new(TEST);
        k2.set_overrides(o);
        assert_eq!(k2.changed("a", None, &[], ""), KeyOverrides::from([("b".to_owned(), vec!["F2".to_owned()])]));
    }

    #[test]
    fn the_real_table_has_unique_ids_and_valid_default_keys() {
        let mut ids: Vec<&str> = COMMANDS.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n);
        for cmd in COMMANDS {
            for key in cmd.keys {
                assert_eq!(crate::keybindings::normalize_key(key).as_deref(), Some(*key), "{}", cmd.id);
            }
        }
        assert_eq!(command("file.save").map(|c| c.keys), Some(&["Ctrl+S"][..]));
        assert_eq!(command("task.run").map(|c| c.keys), Some(&["Ctrl+Alt+4"][..]));
        // The defaults don't clash with each other.
        assert_eq!(Keymap::default().conflicts(), Vec::<(String, Vec<String>)>::new());
    }
}
