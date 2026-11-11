//! Persistent breakpoints, conditions, hit counts and logpoints.
//! Replaces `src/renderer/debug/breakpoints.ts`.
use forge_core::project_state::{path_key, SavedBreakpointState};
use forge_core::tabs::{is_inside, renamed_path};
use forge_text::text_buffer::Change;

pub type Breakpoint = SavedBreakpointState;

#[derive(Default)]
pub struct Breakpoints {
    pub saved: Vec<Breakpoint>,
}

impl Breakpoints {
    pub fn for_file(&self, path: &str) -> Vec<Breakpoint> {
        self.saved.iter().filter(|b| path_key(&b.path) == path_key(path)).cloned().collect()
    }
    pub fn toggle(&mut self, path: &str, line: usize) {
        if self.saved.iter().any(|b| path_key(&b.path) == path_key(path) && b.line == line as u64) {
            self.remove(path, line);
        } else {
            self.set(Breakpoint { path: path.into(), line: line as u64, enabled: true, ..Default::default() });
        }
    }
    pub fn set(&mut self, mut breakpoint: Breakpoint) {
        let clean = |s: Option<String>| s.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        breakpoint.condition = clean(breakpoint.condition);
        breakpoint.hit_condition = clean(breakpoint.hit_condition);
        breakpoint.log_message = clean(breakpoint.log_message);
        self.remove(&breakpoint.path, breakpoint.line as usize);
        self.saved.push(breakpoint);
        self.saved.sort_by(|a, b| path_key(&a.path).cmp(&path_key(&b.path)).then(a.line.cmp(&b.line)));
    }
    pub fn remove(&mut self, path: &str, line: usize) {
        self.saved.retain(|b| path_key(&b.path) != path_key(path) || b.line != line as u64);
    }
    pub fn count(&self) -> usize {
        self.saved.iter().filter(|b| b.enabled).count()
    }
    pub fn apply_change(&mut self, path: &str, change: &Change) {
        let old = self.for_file(path);
        self.saved.retain(|b| path_key(&b.path) != path_key(path));
        for mut b in old {
            let shifted = forge_text::lines::shift_lines(&[b.line as usize], change);
            if let Some(line) = shifted.first() {
                if self.for_file(path).iter().any(|b| b.line == *line as u64) {
                    continue;
                }
                b.line = *line as u64;
                self.set(b);
            }
        }
    }
    pub fn renamed(&mut self, old: &str, new: &str) {
        for b in &mut self.saved {
            if let Some(path) = renamed_path(&b.path, old, new) {
                b.path = path;
            }
        }
    }
    pub fn remove_under(&mut self, path: &str) {
        self.saved.retain(|b| !is_inside(&b.path, path));
    }
    pub fn load(&mut self, saved: Vec<Breakpoint>) {
        self.saved.clear();
        for b in saved {
            self.set(b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_text::Pos;
    #[test]
    fn toggles_per_file_and_windows_paths_ignore_case_and_slashes() {
        let mut s = Breakpoints::default();
        s.toggle("C:\\p\\a.py", 4);
        s.toggle("c:/p/A.py", 1);
        assert_eq!(s.for_file("C:\\p\\a.py").iter().map(|b| b.line).collect::<Vec<_>>(), [1, 4]);
        s.toggle("C:\\p\\a.py", 4);
        assert_eq!(s.count(), 1);
    }
    #[test]
    fn conditions_follow_edits_renames_and_deletes_and_round_trip() {
        let mut s = Breakpoints::default();
        s.set(Breakpoint {
            path: "/p/pkg/a.py".into(),
            line: 4,
            enabled: true,
            condition: Some(" x > 1 ".into()),
            ..Default::default()
        });
        s.apply_change("/p/pkg/a.py", &Change::insert(Pos::new(0, 0), "a\nb\n"));
        assert_eq!(s.saved[0].line, 6);
        assert_eq!(s.saved[0].condition.as_deref(), Some("x > 1"));
        s.renamed("/p/pkg", "/p/lib");
        assert_eq!(s.saved[0].path, "/p/lib/a.py");
        let saved = s.saved.clone();
        let mut t = Breakpoints::default();
        t.load(saved.clone());
        assert_eq!(t.saved, saved);
        s.remove_under("/p/lib");
        assert!(s.saved.is_empty());
    }
    #[test]
    fn deleted_lines_remove_breakpoints_and_disabled_breakpoints_are_not_counted() {
        let mut s = Breakpoints::default();
        s.toggle("/a.py", 2);
        s.toggle("/a.py", 3);
        s.toggle("/a.py", 6);
        s.apply_change("/a.py", &Change::delete(Pos::new(2, 0), "line\n"));
        assert_eq!(s.saved.iter().map(|b| b.line).collect::<Vec<_>>(), [2, 5]);
        for b in &mut s.saved {
            b.enabled = false;
        }
        assert_eq!(s.count(), 0);
    }
}
