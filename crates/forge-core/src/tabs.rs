//! The open files (tabs) and which one is active, in one or two groups (the
//! editor area can be split in two). Replaces the logic of
//! `src/renderer/workspace.ts`; reading and writing files is the app's job,
//! this only tracks document ids.

/// An open document (the app maps ids to its documents).
pub type DocId = u64;

/// One row of tabs with its own active tab.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabGroup {
    pub docs: Vec<DocId>,
    pub active: Option<DocId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tabs {
    /// Left to right; there's always at least one.
    pub groups: Vec<TabGroup>,
    /// The group with keyboard focus: commands and "the active file" refer to it.
    pub active_group: usize,
}

impl Default for Tabs {
    fn default() -> Self {
        Self { groups: vec![TabGroup::default()], active_group: 0 }
    }
}

fn is_win_path(p: &str) -> bool {
    let b = p.as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':') || p.starts_with("\\\\")
}

/// Compare paths; Windows paths (C:\...) ignore slash direction and case.
pub fn same_path(a: &str, b: &str) -> bool {
    if !(is_win_path(a) || is_win_path(b)) {
        return a == b;
    }
    let norm = |p: &str| p.replace('/', "\\").to_lowercase();
    norm(a) == norm(b)
}

/// True if `path` is `parent` itself or somewhere inside it.
pub fn is_inside(path: &str, parent: &str) -> bool {
    if same_path(path, parent) {
        return true;
    }
    let win = is_win_path(path) || is_win_path(parent);
    let norm = |p: &str| if win { p.replace('/', "\\").to_lowercase() } else { p.to_owned() };
    let sep = if win { '\\' } else { '/' };
    let mut base = norm(parent);
    if !base.ends_with(sep) {
        base.push(sep);
    }
    norm(path).starts_with(&base)
}

/// The path of something renamed from `old` to `new` (a folder rename moves what's inside).
pub fn renamed_path(path: &str, old: &str, new: &str) -> Option<String> {
    if !is_inside(path, old) {
        return None;
    }
    let rest: String = path.chars().skip(old.chars().count()).collect();
    Some(format!("{new}{rest}"))
}

impl Tabs {
    fn group(&self) -> &TabGroup {
        &self.groups[self.active_group]
    }

    /// Every open document once (a file open in both groups is one document).
    pub fn docs(&self) -> Vec<DocId> {
        let mut all = Vec::new();
        for g in &self.groups {
            for d in &g.docs {
                if !all.contains(d) {
                    all.push(*d);
                }
            }
        }
        all
    }

    /// The active document of the focused group.
    pub fn active(&self) -> Option<DocId> {
        self.group().active
    }

    pub fn contains(&self, doc: DocId) -> bool {
        self.groups.iter().any(|g| g.docs.contains(&doc))
    }

    /// Show a document in the focused group, adding a tab after the current one if needed.
    pub fn show(&mut self, doc: DocId) {
        let g = &mut self.groups[self.active_group];
        if !g.docs.contains(&doc) {
            let at = g.active.and_then(|a| g.docs.iter().position(|d| *d == a)).map_or(g.docs.len(), |i| i + 1);
            g.docs.insert(at, doc);
        }
        g.active = Some(doc);
    }

    fn group_holding(&self, doc: DocId, group: Option<usize>) -> Option<usize> {
        match group {
            Some(i) => (i < self.groups.len()).then_some(i),
            None if self.group().docs.contains(&doc) => Some(self.active_group),
            None => self.groups.iter().position(|g| g.docs.contains(&doc)),
        }
    }

    /// Make `doc` active: in the focused group if it has a tab there, else in the group that has it.
    pub fn activate(&mut self, doc: DocId, group: Option<usize>) {
        let Some(i) = self.group_holding(doc, group) else { return };
        self.active_group = i;
        let g = &mut self.groups[i];
        if !g.docs.contains(&doc) {
            g.docs.push(doc);
        }
        g.active = Some(doc);
    }

    /// Give keyboard focus to a group (e.g. its editor was clicked).
    pub fn focus_group(&mut self, index: usize) {
        if index < self.groups.len() {
            self.active_group = index;
        }
    }

    /// Split: show the active file in a second group (or move to the other group if already split).
    pub fn split(&mut self) {
        if self.groups.len() > 1 {
            self.focus_group(if self.active_group == 0 { 1 } else { 0 });
            return;
        }
        let doc = self.active();
        self.groups.push(TabGroup { docs: doc.into_iter().collect(), active: doc });
        self.active_group = 1;
    }

    /// Move the active tab to the other group (splitting first if needed).
    pub fn move_to_other_group(&mut self) {
        let Some(doc) = self.active() else { return };
        if self.groups.len() == 1 {
            self.split();
            self.remove_from(0, doc);
        } else {
            let from = self.active_group;
            let to = if from == 0 { 1 } else { 0 };
            let target = &mut self.groups[to];
            if !target.docs.contains(&doc) {
                target.docs.push(doc);
            }
            target.active = Some(doc);
            self.remove_from(from, doc);
            self.active_group = to.min(self.groups.len() - 1);
        }
    }

    /// Would closing this tab close the document's last tab? (Then unsaved changes need asking about.)
    pub fn is_last_tab(&self, doc: DocId, group: Option<usize>) -> bool {
        let Some(i) = self.group_holding(doc, group) else { return false };
        !self.groups.iter().enumerate().any(|(j, g)| j != i && g.docs.contains(&doc))
    }

    /// Close a tab (in the focused group, or `group`). Asking about unsaved changes is the caller's job.
    pub fn close(&mut self, doc: DocId, group: Option<usize>) {
        if let Some(i) = self.group_holding(doc, group) {
            self.remove_from(i, doc);
        }
    }

    /// Remove a tab from a group; an emptied second group goes away.
    fn remove_from(&mut self, group: usize, doc: DocId) {
        let g = &mut self.groups[group];
        let Some(index) = g.docs.iter().position(|d| *d == doc) else { return };
        g.docs.remove(index);
        if g.active == Some(doc) {
            g.active = g.docs.get(index.min(g.docs.len().saturating_sub(1))).copied();
        }
        if g.docs.is_empty() && self.groups.len() > 1 {
            self.groups.remove(group);
            if self.active_group >= self.groups.len() || self.active_group == group {
                self.active_group = group.min(self.groups.len() - 1);
            }
        }
    }

    /// Close every tab (e.g. when opening another folder).
    pub fn close_all(&mut self) {
        *self = Self::default();
    }

    /// Switch to the next (+1) or previous (-1) tab in the focused group, wrapping around.
    pub fn next(&mut self, delta: i32) {
        let g = self.group();
        let Some(active) = g.active else { return };
        if g.docs.len() < 2 {
            return;
        }
        let n = g.docs.len() as i32;
        let i = g.docs.iter().position(|d| *d == active).unwrap_or(0) as i32;
        let doc = g.docs[((i + delta).rem_euclid(n)) as usize];
        self.activate(doc, Some(self.active_group));
    }

    /// Close tabs without asking (their files were deleted).
    pub fn discard(&mut self, docs: &[DocId]) {
        for i in (0..self.groups.len()).rev() {
            for doc in docs {
                if i < self.groups.len() {
                    self.remove_from(i, *doc);
                }
            }
        }
    }

    /// Put `new` wherever `old` is (the file was read again with another encoding).
    pub fn replace(&mut self, old: DocId, new: DocId) {
        for g in &mut self.groups {
            for d in &mut g.docs {
                if *d == old {
                    *d = new;
                }
            }
            if g.active == Some(old) {
                g.active = Some(new);
            }
        }
    }

    /// Bring back a saved layout: per group its documents (those that could
    /// be opened) and the one that was showing, plus the focused group. A
    /// group left with nothing goes away.
    pub fn restore(groups: Vec<(Vec<DocId>, Option<DocId>)>, active_group: usize) -> Self {
        let mut all: Vec<TabGroup> = Vec::new();
        for (docs, active) in groups.into_iter().take(2) {
            let mut unique: Vec<DocId> = Vec::new();
            for d in docs {
                if !unique.contains(&d) {
                    unique.push(d);
                }
            }
            let active = active.filter(|a| unique.contains(a)).or_else(|| unique.first().copied());
            all.push(TabGroup { docs: unique, active });
        }
        let focused = all.get(active_group).cloned();
        let kept: Vec<TabGroup> = all.into_iter().filter(|g| !g.docs.is_empty()).collect();
        let active_group = focused.and_then(|f| kept.iter().position(|g| *g == f)).unwrap_or(0);
        if kept.is_empty() {
            return Self::default();
        }
        Self { groups: kept, active_group }
    }

    /// Which group shows this document's tab (the focused one first).
    pub fn group_of(&self, doc: DocId) -> Option<usize> {
        self.group_holding(doc, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(t: &mut Tabs, docs: &[DocId]) {
        for d in docs {
            t.show(*d);
        }
    }

    #[test]
    fn opens_tabs_and_reuses_an_open_tab() {
        let mut t = Tabs::default();
        open(&mut t, &[1, 2]);
        assert_eq!(t.docs(), [1, 2]);
        assert_eq!(t.active(), Some(2));
        t.show(1);
        assert_eq!(t.docs(), [1, 2]);
        assert_eq!(t.active(), Some(1));
    }

    #[test]
    fn opens_a_new_tab_right_after_the_active_one() {
        let mut t = Tabs::default();
        open(&mut t, &[1, 2]);
        t.activate(1, None);
        t.show(3);
        assert_eq!(t.docs(), [1, 3, 2]);
    }

    #[test]
    fn closing_the_active_tab_activates_its_neighbour() {
        let mut t = Tabs::default();
        open(&mut t, &[1, 2, 3]);
        t.activate(2, None);
        t.close(2, None);
        assert_eq!(t.active(), Some(3));
        t.close(3, None);
        assert_eq!(t.active(), Some(1));
        t.close(1, None);
        assert_eq!(t.active(), None);
    }

    #[test]
    fn cycles_through_tabs_wrapping_around() {
        let mut t = Tabs::default();
        open(&mut t, &[1, 2, 3]);
        t.next(1);
        assert_eq!(t.active(), Some(1));
        t.next(-1);
        assert_eq!(t.active(), Some(3));
    }

    #[test]
    fn same_path_ignores_slashes_and_case_for_windows_paths_only() {
        assert!(same_path("C:\\X\\a.py", "c:/x/A.py"));
        assert!(!same_path("/home/a.py", "/home/A.py"));
    }

    #[test]
    fn is_inside_matches_the_path_itself_and_anything_below() {
        assert!(is_inside("C:\\p\\pkg", "C:\\p\\pkg"));
        assert!(is_inside("C:\\p\\pkg\\a.py", "c:/p/PKG"));
        assert!(!is_inside("C:\\p\\pkg2\\a.py", "C:\\p\\pkg"));
        assert!(is_inside("/home/u/pkg/a.py", "/home/u/pkg"));
        assert!(!is_inside("/home/u/pkg2", "/home/u/pkg"));
    }

    #[test]
    fn renaming_a_folder_moves_the_paths_inside_it() {
        assert_eq!(renamed_path("C:\\p\\pkg\\a.py", "C:\\p\\pkg", "C:\\p\\lib").as_deref(), Some("C:\\p\\lib\\a.py"));
        assert_eq!(renamed_path("C:\\p\\main.py", "C:\\p\\pkg", "C:\\p\\lib"), None);
        assert_eq!(renamed_path("C:\\p\\main.py", "C:\\p\\main.py", "C:\\p\\app.py").as_deref(), Some("C:\\p\\app.py"));
    }

    #[test]
    fn deleting_closes_the_affected_tabs() {
        let mut t = Tabs::default();
        open(&mut t, &[1, 2, 3]);
        t.activate(2, None);
        t.discard(&[1, 2]);
        assert_eq!(t.docs(), [3]);
        assert_eq!(t.active(), Some(3));
    }

    #[test]
    fn split_shows_the_active_file_in_a_second_group_that_gets_focus() {
        let mut t = Tabs::default();
        open(&mut t, &[1]);
        t.split();
        assert_eq!(t.groups.len(), 2);
        assert_eq!(t.active_group, 1);
        assert_eq!(t.groups[1].docs, [1]);
        assert_eq!(t.docs(), [1]);
    }

    #[test]
    fn opening_goes_into_the_focused_group() {
        let mut t = Tabs::default();
        open(&mut t, &[1]);
        t.split();
        t.show(2);
        assert_eq!(t.groups[0].docs, [1]);
        assert_eq!(t.groups[1].docs, [1, 2]);
        t.focus_group(0);
        assert_eq!(t.active(), Some(1));
    }

    #[test]
    fn a_file_open_in_both_groups_is_only_last_when_its_final_tab_closes() {
        let mut t = Tabs::default();
        open(&mut t, &[1, 2]);
        t.split();
        t.activate(1, Some(1));
        assert!(t.groups[1].docs.contains(&1));
        assert!(!t.is_last_tab(1, Some(0)));
        t.close(1, Some(0));
        assert!(t.is_last_tab(1, Some(1)));
    }

    #[test]
    fn closing_the_last_tab_of_the_second_group_removes_the_split() {
        let mut t = Tabs::default();
        open(&mut t, &[1]);
        t.split();
        t.close(1, Some(1));
        assert_eq!(t.groups.len(), 1);
        assert_eq!(t.active_group, 0);
        assert_eq!(t.active(), Some(1));
    }

    #[test]
    fn move_to_the_other_group() {
        let mut t = Tabs::default();
        open(&mut t, &[1, 2]);
        t.move_to_other_group();
        assert_eq!(t.groups[0].docs, [1]);
        assert_eq!(t.groups[1].docs, [2]);
        assert_eq!(t.active(), Some(2));
    }

    #[test]
    fn deleted_files_disappear_from_both_groups() {
        let mut t = Tabs::default();
        open(&mut t, &[1]);
        t.split();
        t.show(3);
        t.discard(&[1]);
        assert_eq!(t.groups.len(), 1);
        assert_eq!(t.docs(), [3]);
    }

    #[test]
    fn restores_tabs_per_side_with_the_right_active_tab_and_focus() {
        let t = Tabs::restore(vec![(vec![1, 2], Some(2)), (vec![2, 3], Some(3))], 1);
        assert_eq!(t.groups.iter().map(|g| g.docs.clone()).collect::<Vec<_>>(), [vec![1, 2], vec![2, 3]]);
        assert_eq!(t.groups[0].active, Some(2));
        assert_eq!(t.active(), Some(3));
    }

    #[test]
    fn restoring_drops_a_side_left_empty() {
        let t = Tabs::restore(vec![(vec![], None), (vec![1], None)], 1);
        assert_eq!(t.groups.len(), 1);
        assert_eq!(t.active(), Some(1));
        assert_eq!(Tabs::restore(vec![(vec![], None)], 0), Tabs::default());
    }

    #[test]
    fn replacing_a_document_keeps_its_place() {
        let mut t = Tabs::default();
        open(&mut t, &[1, 2]);
        t.replace(2, 9);
        assert_eq!(t.docs(), [1, 9]);
        assert_eq!(t.active(), Some(9));
    }
}
