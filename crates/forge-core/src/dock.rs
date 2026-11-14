//! Forge's workspace layout: per task, a tree of splits whose leaves are tab
//! stacks of panels. Replaces `src/shared/layouts.ts` (the format and its
//! normalizing), `src/renderer/dock/model.ts` (pure edits; every function
//! returns a new tree) and the logic of `src/renderer/dock/tasks.ts`.
//! Saved as JSON (`layouts.json`) in the same shape the old app used.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

pub type TaskId = &'static str;
pub const TASKS: [TaskId; 4] = ["code", "connect", "debug", "run"];

/// Every panel the workspace knows.
pub const PANEL_IDS: [&str; 22] = [
    "files",
    "search",
    "outline",
    "git",
    "history",
    "editor",
    "where",
    "infile",
    "graph",
    "toolbox",
    "box",
    "terminal",
    "problems",
    "output",
    "debug",
    "tests",
    "references",
    "calls",
    "network",
    "pipeline",
    "database",
    "memory",
];

/// The code editor: can't be closed, always placed in the Code task.
pub const EDITOR: &str = "editor";
/// No part of a split smaller than this.
pub const MIN_SIZE: f64 = 0.08;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    /// Children side by side.
    Row,
    /// Children stacked top to bottom.
    Column,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum DockNode {
    Stack {
        id: String,
        panels: Vec<String>,
        active: String,
        /// Only the tab strip shows (e.g. the terminal drawer after Ctrl+J).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        collapsed: bool,
    },
    Split {
        id: String,
        dir: Dir,
        children: Vec<DockNode>,
        /// Fractions of the split, one per child, summing to 1.
        sizes: Vec<f64>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Left,
    Right,
    Top,
    Bottom,
    Center,
}

impl DockNode {
    pub fn id(&self) -> &str {
        match self {
            DockNode::Stack { id, .. } | DockNode::Split { id, .. } => id,
        }
    }

    pub fn stack(id: &str, panels: &[&str]) -> Self {
        DockNode::Stack {
            id: id.into(),
            panels: panels.iter().map(|p| (*p).to_owned()).collect(),
            active: panels.first().map(|p| (*p).to_owned()).unwrap_or_default(),
            collapsed: false,
        }
    }

    pub fn split(id: &str, dir: Dir, parts: Vec<(DockNode, f64)>) -> Self {
        let (children, sizes) = parts.into_iter().unzip();
        DockNode::Split { id: id.into(), dir, children, sizes }
    }
}

/// `n` sizes that are each at least MIN_SIZE and sum to 1 (bad input becomes equal parts).
pub fn fix_sizes(sizes: &[f64], n: usize) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }
    let mut s: Vec<f64> =
        (0..n).map(|i| sizes.get(i).copied().filter(|v| v.is_finite() && *v > 0.0).unwrap_or(0.0)).collect();
    let mut total: f64 = s.iter().sum();
    if total <= 0.0 {
        s = vec![1.0; n];
        total = n as f64;
    }
    s.iter_mut().for_each(|v| *v /= total);
    let min = MIN_SIZE.min(1.0 / n as f64);
    // Raise the small ones to the minimum and take the difference from the others, proportionally.
    for _ in 0..n {
        let small: Vec<bool> = s.iter().map(|v| *v < min - 1e-12).collect();
        let count = small.iter().filter(|b| **b).count();
        if count == 0 {
            break;
        }
        let fixed = count as f64 * min;
        let rest: f64 = s.iter().zip(&small).filter(|(_, sm)| !**sm).map(|(v, _)| v).sum();
        let others = (n - count).max(1) as f64;
        s = s
            .iter()
            .zip(&small)
            .map(|(v, sm)| {
                if *sm {
                    min
                } else if rest > 0.0 {
                    v / rest * (1.0 - fixed)
                } else {
                    (1.0 - fixed) / others
                }
            })
            .collect();
    }
    s
}

// ---------- default arrangements ----------

pub fn default_arrangement(task: &str) -> DockNode {
    use DockNode as N;
    let st = N::stack;
    match task {
        "connect" => N::split(
            "n9",
            Dir::Column,
            vec![
                (
                    N::split(
                        "n4",
                        Dir::Row,
                        vec![
                            (st("n1", &["files", "outline"]), 0.18),
                            (st("n2", &["graph", "pipeline", "database"]), 0.66),
                            (st("n3", &["toolbox"]), 0.16),
                        ],
                    ),
                    0.78,
                ),
                (st("n5", &["terminal", "network", "problems", "output"]), 0.22),
            ],
        ),
        "debug" => N::split(
            "n5",
            Dir::Column,
            vec![
                (N::split("n3", Dir::Row, vec![(st("n1", &["editor"]), 0.62), (st("n2", &["debug"]), 0.38)]), 0.75),
                (st("n4", &["terminal", "output", "problems"]), 0.25),
            ],
        ),
        "run" => N::split(
            "n5",
            Dir::Column,
            vec![
                (
                    N::split(
                        "n3",
                        Dir::Row,
                        vec![
                            (st("n1", &["editor"]), 0.6),
                            (st("n2", &["output", "network", "memory", "tests", "problems"]), 0.4),
                        ],
                    ),
                    0.72,
                ),
                (st("n4", &["terminal"]), 0.28),
            ],
        ),
        _ => N::split(
            "n9",
            Dir::Row,
            vec![
                (st("n1", &["files", "search", "outline", "git"]), 0.2),
                (
                    N::split(
                        "n4",
                        Dir::Column,
                        vec![
                            (st("n2", &["editor"]), 0.7),
                            (
                                st(
                                    "n3",
                                    &["terminal", "problems", "output", "tests", "references", "calls", "history"],
                                ),
                                0.3,
                            ),
                        ],
                    ),
                    0.58,
                ),
                (N::split("n7", Dir::Column, vec![(st("n5", &["where"]), 0.6), (st("n6", &["infile"]), 0.4)]), 0.22),
            ],
        ),
    }
}

// ---------- queries ----------

pub fn placed_panels(root: &DockNode) -> Vec<String> {
    match root {
        DockNode::Stack { panels, .. } => panels.clone(),
        DockNode::Split { children, .. } => children.iter().flat_map(placed_panels).collect(),
    }
}

/// The stack holding a panel: (id, panels, active, collapsed).
pub fn find_stack<'a>(root: &'a DockNode, panel: &str) -> Option<&'a DockNode> {
    match root {
        DockNode::Stack { panels, .. } => panels.iter().any(|p| p == panel).then_some(root),
        DockNode::Split { children, .. } => children.iter().find_map(|c| find_stack(c, panel)),
    }
}

pub fn find_node<'a>(root: &'a DockNode, id: &str) -> Option<&'a DockNode> {
    if root.id() == id {
        return Some(root);
    }
    match root {
        DockNode::Split { children, .. } => children.iter().find_map(|c| find_node(c, id)),
        DockNode::Stack { .. } => None,
    }
}

/// Is the panel on screen: the active tab of a stack that isn't collapsed.
pub fn is_shown(root: &DockNode, panel: &str) -> bool {
    matches!(find_stack(root, panel), Some(DockNode::Stack { active, collapsed, .. }) if active == panel && !collapsed)
}

/// A fresh node id ("n" + one more than the highest number in use).
pub fn next_id(root: &DockNode) -> String {
    fn walk(n: &DockNode, max: &mut u64) {
        let digits: String =
            n.id().chars().rev().take_while(char::is_ascii_digit).collect::<Vec<_>>().into_iter().rev().collect();
        if let Ok(v) = digits.parse::<u64>() {
            *max = (*max).max(v);
        }
        if let DockNode::Split { children, .. } = n {
            children.iter().for_each(|c| walk(c, max));
        }
    }
    let mut max = 0;
    walk(root, &mut max);
    format!("n{}", max + 1)
}

fn parent_of<'a>(root: &'a DockNode, id: &str) -> Option<&'a DockNode> {
    match root {
        DockNode::Stack { .. } => None,
        DockNode::Split { children, .. } => {
            if children.iter().any(|c| c.id() == id) {
                Some(root)
            } else {
                children.iter().find_map(|c| parent_of(c, id))
            }
        }
    }
}

// ---------- edits ----------

/// Apply `f` to the node with this id (everything else is copied as is).
fn map_node(root: &DockNode, id: &str, f: &dyn Fn(&DockNode) -> DockNode) -> DockNode {
    if root.id() == id {
        return f(root);
    }
    match root {
        DockNode::Stack { .. } => root.clone(),
        DockNode::Split { id: sid, dir, children, sizes } => DockNode::Split {
            id: sid.clone(),
            dir: *dir,
            children: children.iter().map(|c| map_node(c, id, f)).collect(),
            sizes: sizes.clone(),
        },
    }
}

pub fn activate(root: &DockNode, panel: &str) -> DockNode {
    let Some(s) = find_stack(root, panel) else { return root.clone() };
    let panel = panel.to_owned();
    map_node(root, s.id(), &|n| match n {
        DockNode::Stack { id, panels, collapsed, .. } => {
            DockNode::Stack { id: id.clone(), panels: panels.clone(), active: panel.clone(), collapsed: *collapsed }
        }
        other => other.clone(),
    })
}

pub fn set_collapsed(root: &DockNode, stack_id: &str, collapsed: bool) -> DockNode {
    map_node(root, stack_id, &|n| match n {
        DockNode::Stack { id, panels, active, .. } => {
            DockNode::Stack { id: id.clone(), panels: panels.clone(), active: active.clone(), collapsed }
        }
        other => other.clone(),
    })
}

pub fn resize(root: &DockNode, split_id: &str, new_sizes: &[f64]) -> DockNode {
    map_node(root, split_id, &|n| match n {
        DockNode::Split { id, dir, children, .. } => DockNode::Split {
            id: id.clone(),
            dir: *dir,
            children: children.clone(),
            sizes: fix_sizes(new_sizes, children.len()),
        },
        other => other.clone(),
    })
}

/// Take a panel out; empty stacks and one-child splits disappear. `None` if nothing is left.
pub fn remove_panel(root: &DockNode, panel: &str) -> Option<DockNode> {
    match root {
        DockNode::Stack { id, panels, active, collapsed } => {
            let Some(i) = panels.iter().position(|p| p == panel) else { return Some(root.clone()) };
            let rest: Vec<String> = panels.iter().filter(|p| *p != panel).cloned().collect();
            if rest.is_empty() {
                return None;
            }
            let active = if active == panel { rest[i.min(rest.len() - 1)].clone() } else { active.clone() };
            Some(DockNode::Stack { id: id.clone(), panels: rest, active, collapsed: *collapsed })
        }
        DockNode::Split { id, dir, children, sizes } => {
            let kept: Vec<(DockNode, f64)> =
                children.iter().zip(sizes).filter_map(|(c, s)| remove_panel(c, panel).map(|n| (n, *s))).collect();
            match kept.len() {
                0 => None,
                1 => kept.into_iter().next().map(|(n, _)| n),
                n => {
                    let raw: Vec<f64> = kept.iter().map(|(_, s)| *s).collect();
                    Some(DockNode::Split {
                        id: id.clone(),
                        dir: *dir,
                        children: kept.into_iter().map(|(c, _)| c).collect(),
                        sizes: fix_sizes(&raw, n),
                    })
                }
            }
        }
    }
}

/// Put a panel into a stack (centre) or beside it (an edge). A panel already placed is moved.
pub fn insert_panel(root: &DockNode, panel: &str, target_stack_id: &str, zone: Zone) -> DockNode {
    let base = if find_stack(root, panel).is_some() {
        match remove_panel(root, panel) {
            Some(b) => b,
            None => return root.clone(),
        }
    } else {
        root.clone()
    };
    if !matches!(find_node(&base, target_stack_id), Some(DockNode::Stack { .. })) {
        return root.clone();
    }
    let panel_s = panel.to_owned();
    if zone == Zone::Center {
        return map_node(&base, target_stack_id, &|n| match n {
            DockNode::Stack { id, panels, collapsed, .. } => {
                let mut panels = panels.clone();
                panels.push(panel_s.clone());
                DockNode::Stack { id: id.clone(), panels, active: panel_s.clone(), collapsed: *collapsed }
            }
            other => other.clone(),
        });
    }
    let dir = if matches!(zone, Zone::Left | Zone::Right) { Dir::Row } else { Dir::Column };
    let before = matches!(zone, Zone::Left | Zone::Top);
    let fresh_id = next_id(&base);
    let fresh =
        DockNode::Stack { id: fresh_id.clone(), panels: vec![panel_s.clone()], active: panel_s, collapsed: false };
    if let Some(DockNode::Split { id: parent_id, dir: parent_dir, .. }) = parent_of(&base, target_stack_id) {
        if *parent_dir == dir {
            // Become a sibling in the same split, sharing the target's share.
            let target = target_stack_id.to_owned();
            return map_node(&base, &parent_id.clone(), &|n| match n {
                DockNode::Split { id, dir, children, sizes } => {
                    let i = children.iter().position(|c| c.id() == target).unwrap_or(0);
                    let half = sizes[i] / 2.0;
                    let mut children = children.clone();
                    let mut sizes = sizes.clone();
                    children.insert(if before { i } else { i + 1 }, fresh.clone());
                    sizes.splice(i..=i, [half, half]);
                    DockNode::Split { id: id.clone(), dir: *dir, children, sizes }
                }
                other => other.clone(),
            });
        }
    }
    let wrap_id = format!("n{}", fresh_id[1..].parse::<u64>().unwrap_or(0) + 1);
    map_node(&base, target_stack_id, &|t| {
        let children = if before { vec![fresh.clone(), t.clone()] } else { vec![t.clone(), fresh.clone()] };
        DockNode::Split { id: wrap_id.clone(), dir, children, sizes: vec![0.5, 0.5] }
    })
}

/// Drag-and-drop: move a panel to a stack or beside it. Dropping onto itself changes nothing.
pub fn move_panel(root: &DockNode, panel: &str, target_stack_id: &str, zone: Zone) -> DockNode {
    if let Some(DockNode::Stack { id, panels, .. }) = find_stack(root, panel) {
        if id == target_stack_id && (zone == Zone::Center || panels.len() == 1) {
            return root.clone();
        }
    }
    insert_panel(root, panel, target_stack_id, zone)
}

/// Where a drop at (x, y) lands on a stack: its centre (middle 40% box) or the nearest edge.
pub fn zone_at(left: f64, top: f64, width: f64, height: f64, x: f64, y: f64) -> Zone {
    let fx = if width > 0.0 { (x - left) / width } else { 0.5 };
    let fy = if height > 0.0 { (y - top) / height } else { 0.5 };
    if (0.3..=0.7).contains(&fx) && (0.3..=0.7).contains(&fy) {
        return Zone::Center;
    }
    [(Zone::Left, fx), (Zone::Right, 1.0 - fx), (Zone::Top, fy), (Zone::Bottom, 1.0 - fy)]
        .into_iter()
        .fold((Zone::Left, f64::MAX), |a, b| if b.1 < a.1 { b } else { a })
        .0
}

/// Add a panel in a new stack along one side of the whole layout, taking `size` of it.
pub fn insert_at_edge(root: &DockNode, panel: &str, zone: Zone, size: f64) -> DockNode {
    let base = if find_stack(root, panel).is_some() {
        remove_panel(root, panel).unwrap_or_else(|| root.clone())
    } else {
        root.clone()
    };
    let fresh_id = next_id(&base);
    let fresh = DockNode::Stack {
        id: fresh_id.clone(),
        panels: vec![panel.to_owned()],
        active: panel.to_owned(),
        collapsed: false,
    };
    let before = matches!(zone, Zone::Left | Zone::Top);
    let s = size.clamp(0.1, 0.9);
    DockNode::Split {
        id: format!("n{}", fresh_id[1..].parse::<u64>().unwrap_or(0) + 1),
        dir: if matches!(zone, Zone::Left | Zone::Right) { Dir::Row } else { Dir::Column },
        children: if before { vec![fresh, base] } else { vec![base, fresh] },
        sizes: if before { vec![s, 1.0 - s] } else { vec![1.0 - s, s] },
    }
}

// ---------- saved layouts ----------

/// What's saved per user: an arrangement per task (missing = that task's default).
pub type SavedLayouts = BTreeMap<String, DockNode>;

/// Turn a saved layouts file (any version, hand-edited, or corrupt) into valid
/// arrangements: unknown panels and duplicates go, sizes are fixed, empty
/// parts collapse. A task whose arrangement is unusable is left out (it gets
/// its default), and so is a Code arrangement without the editor.
pub fn normalize_layouts(raw: &Value) -> SavedLayouts {
    let mut out = SavedLayouts::new();
    let Some(obj) = raw.as_object() else { return out };
    for task in TASKS {
        let mut seen = HashSet::new();
        let mut ids = HashSet::new();
        let Some(tree) = obj.get(task).and_then(|v| normalize_node(v, &mut seen, 0, &mut ids)) else { continue };
        if task == "code" && !seen.contains(EDITOR) {
            continue;
        }
        out.insert(task.to_owned(), tree);
    }
    out
}

fn normalize_node(
    raw: &Value,
    seen: &mut HashSet<String>,
    depth: usize,
    ids: &mut HashSet<String>,
) -> Option<DockNode> {
    let r = raw.as_object()?;
    if depth > 12 {
        return None;
    }
    let raw_id = r.get("id")?.as_str()?;
    if raw_id.is_empty()
        || raw_id.len() > 32
        || !raw_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    // Node ids must be unique (edits find nodes by id): rename repeats.
    let mut id = raw_id.to_owned();
    let mut k = 2;
    while ids.contains(&id) {
        id = format!("{}-{k}", &raw_id[..raw_id.len().min(24)]);
        k += 1;
    }
    ids.insert(id.clone());
    match r.get("kind")?.as_str()? {
        "stack" => {
            let mut panels = Vec::new();
            for p in r.get("panels").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                if PANEL_IDS.contains(&p) && !seen.contains(p) {
                    seen.insert(p.to_owned());
                    panels.push(p.to_owned());
                }
            }
            if panels.is_empty() {
                return None;
            }
            let active =
                r.get("active").and_then(Value::as_str).filter(|a| panels.iter().any(|p| p == a)).map(str::to_owned);
            Some(DockNode::Stack {
                active: active.unwrap_or_else(|| panels[0].clone()),
                panels,
                id,
                collapsed: r.get("collapsed") == Some(&Value::Bool(true)),
            })
        }
        "split" => {
            let dir = match r.get("dir")?.as_str()? {
                "row" => Dir::Row,
                "column" => Dir::Column,
                _ => return None,
            };
            let raw_sizes: Vec<f64> = r
                .get("sizes")
                .and_then(Value::as_array)
                .map(|s| s.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect())
                .unwrap_or_default();
            let mut kept = Vec::new();
            for (i, c) in r.get("children").and_then(Value::as_array).into_iter().flatten().enumerate() {
                if let Some(n) = normalize_node(c, seen, depth + 1, ids) {
                    kept.push((n, raw_sizes.get(i).copied().unwrap_or(0.0)));
                }
            }
            match kept.len() {
                0 => None,
                1 => kept.into_iter().next().map(|(n, _)| n),
                n => {
                    let sizes: Vec<f64> = kept.iter().map(|(_, s)| *s).collect();
                    Some(DockNode::Split {
                        id,
                        dir,
                        children: kept.into_iter().map(|(c, _)| c).collect(),
                        sizes: fix_sizes(&sizes, n),
                    })
                }
            }
        }
        _ => None,
    }
}

// ---------- tasks ----------

/// Panels that live at the side (with Files); everything else goes in the drawer (with Terminal).
const SIDE_PANELS: [&str; 6] = ["files", "search", "outline", "git", "where", "infile"];
const DRAWER_ANCHOR: &str = "terminal";
/// Panels that belong with the Graph: revealing one goes to Connect.
const CONNECT_PANELS: [&str; 2] = ["toolbox", "box"];
const SIDE_ANCHOR: &str = "files";

/// The tasks (Code, Connect, Debug, Run): one arrangement each. Switching
/// task swaps the whole arrangement; revealing a panel makes it visible in
/// the current task, placing it if this task doesn't have it yet.
#[derive(Clone, Debug)]
pub struct Tasks {
    arrangements: BTreeMap<String, DockNode>,
    task: TaskId,
    before_debug: Option<TaskId>,
    /// Something changed that should be saved (the owner saves, debounced).
    pub dirty: bool,
    transition: Option<Transition>,
}

/// Transient presentation event; never part of the saved layout format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transition {
    pub revision: u64,
    pub kind: TransitionKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionKind {
    Task,
    Layout,
}

impl Tasks {
    pub fn new(saved: &SavedLayouts) -> Self {
        let arrangements = TASKS
            .iter()
            .map(|t| ((*t).to_owned(), saved.get(*t).cloned().unwrap_or_else(|| default_arrangement(t))))
            .collect();
        Self { arrangements, task: "code", before_debug: None, dirty: false, transition: None }
    }

    pub fn current(&self) -> TaskId {
        self.task
    }

    pub fn root(&self) -> &DockNode {
        &self.arrangements[self.task]
    }

    pub fn arrangement(&self, task: &str) -> Option<&DockNode> {
        self.arrangements.get(task)
    }

    pub fn saved(&self) -> SavedLayouts {
        self.arrangements.clone()
    }

    pub fn switch_to(&mut self, task: &str) {
        if let Some(t) = TASKS.iter().find(|t| **t == task) {
            if self.task != *t {
                self.task = t;
                self.animate(TransitionKind::Task);
            }
        }
    }

    pub fn transition(&self) -> Option<Transition> {
        self.transition
    }

    fn animate(&mut self, kind: TransitionKind) {
        let revision = self.transition.map_or(1, |t| t.revision.wrapping_add(1));
        self.transition = Some(Transition { revision, kind });
    }

    fn update_animated(&mut self, root: DockNode) {
        if &root != self.root() {
            self.animate(TransitionKind::Layout);
            self.update(root);
        }
    }

    /// Saved arrangements (they load after startup): use them.
    pub fn use_saved(&mut self, saved: &SavedLayouts) {
        for (t, n) in saved {
            self.arrangements.insert(t.clone(), n.clone());
        }
    }

    /// The view changed it (tab click, divider drag).
    pub fn changed(&mut self, root: DockNode) {
        self.update(root);
    }

    fn update(&mut self, root: DockNode) {
        self.arrangements.insert(self.task.to_owned(), root);
        self.dirty = true;
    }

    pub fn is_shown(&self, panel: &str) -> bool {
        is_shown(self.root(), panel)
    }

    /// Make a panel visible in the current task (placing it if needed).
    pub fn reveal(&mut self, panel: &str) {
        let root = self.root().clone();
        if let Some(DockNode::Stack { id, collapsed, .. }) = find_stack(&root, panel) {
            let mut next = activate(&root, panel);
            if *collapsed {
                next = set_collapsed(&next, id, false);
            }
            if next != root {
                self.update(next);
            }
            return;
        }
        if panel == EDITOR {
            return self.switch_to("code");
        }
        if panel == "graph" {
            return self.switch_to("connect");
        }
        if CONNECT_PANELS.contains(&panel) && self.task != "connect" {
            self.switch_to("connect");
            return self.reveal(panel);
        }
        let placed = self.place(&root, panel);
        self.update(placed);
    }

    /// Where a panel goes when it isn't placed: with Files, in the drawer, or along an edge.
    fn place(&self, root: &DockNode, panel: &str) -> DockNode {
        if SIDE_PANELS.contains(&panel) {
            return match find_stack(root, SIDE_ANCHOR) {
                Some(side) => insert_panel(root, panel, side.id(), Zone::Center),
                None => insert_at_edge(root, panel, Zone::Left, 0.2),
            };
        }
        if [EDITOR, "graph", "pipeline", "database"].contains(&panel) {
            return insert_at_edge(root, panel, Zone::Right, 0.6);
        }
        if CONNECT_PANELS.contains(&panel) {
            return insert_at_edge(root, panel, Zone::Right, if panel == "box" { 0.6 } else { 0.26 });
        }
        match find_stack(root, DRAWER_ANCHOR) {
            Some(drawer) => {
                let next = insert_panel(root, panel, drawer.id(), Zone::Center);
                match find_stack(&next, panel) {
                    Some(DockNode::Stack { id, collapsed: true, .. }) => set_collapsed(&next, id, false),
                    _ => next,
                }
            }
            None => insert_at_edge(root, panel, Zone::Bottom, 0.3),
        }
    }

    /// Collapse the drawer a panel is in (other panels are left alone).
    pub fn hide(&mut self, panel: &str) {
        if let Some(DockNode::Stack { id, panels, collapsed: false, .. }) = find_stack(self.root(), panel) {
            if panels.iter().any(|p| p == DRAWER_ANCHOR) {
                let next = set_collapsed(self.root(), &id.clone(), true);
                self.update(next);
            }
        }
    }

    /// Ctrl+J: collapse or expand the drawer (the stack holding the terminal).
    pub fn toggle_drawer(&mut self) {
        match find_stack(self.root(), DRAWER_ANCHOR) {
            Some(DockNode::Stack { id, collapsed, .. }) => {
                let next = set_collapsed(self.root(), &id.clone(), !collapsed);
                self.update(next);
            }
            _ => self.reveal(DRAWER_ANCHOR),
        }
    }

    pub fn reset_current(&mut self) {
        self.update_animated(default_arrangement(self.task));
    }

    pub fn close_panel(&mut self, id: &str) {
        if id == EDITOR {
            return;
        }
        if let Some(next) = remove_panel(self.root(), id) {
            self.update_animated(next);
        }
    }

    /// Put a closed panel back (Edit Layout's "Add panel"), without switching task.
    pub fn add_panel(&mut self, id: &str) {
        if find_stack(self.root(), id).is_some() {
            return self.reveal(id);
        }
        let next = self.place(&self.root().clone(), id);
        self.update_animated(next);
    }

    /// Panels the current task doesn't place.
    pub fn hidden_panels(&self) -> Vec<&'static str> {
        let placed: HashSet<String> = placed_panels(self.root()).into_iter().collect();
        PANEL_IDS.iter().copied().filter(|p| !placed.contains(*p)).collect()
    }

    /// Drag and drop in Edit Layout mode.
    pub fn drop_panel(&mut self, panel: &str, target_stack_id: &str, zone: Zone) {
        let next = move_panel(self.root(), panel, target_stack_id, zone);
        if &next != self.root() {
            self.update_animated(next);
        }
    }

    pub fn begin_debug(&mut self, enabled: bool) {
        if !enabled || self.task == "debug" {
            return;
        }
        self.before_debug = Some(self.task);
        self.switch_to("debug");
    }

    pub fn end_debug(&mut self) {
        if let Some(back) = self.before_debug.take() {
            if self.task == "debug" {
                self.switch_to(back);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use DockNode as N;

    #[test]
    fn task_and_structural_changes_animate_but_tab_clicks_and_resizes_do_not() {
        let mut tasks = Tasks::new(&SavedLayouts::new());
        assert!(tasks.transition().is_none());
        tasks.switch_to("connect");
        let task = tasks.transition().unwrap();
        assert_eq!(task.kind, TransitionKind::Task);
        tasks.switch_to("connect");
        assert_eq!(tasks.transition(), Some(task));
        tasks.close_panel("toolbox");
        let layout = tasks.transition().unwrap();
        assert_eq!(layout.kind, TransitionKind::Layout);
        assert!(layout.revision > task.revision);
        tasks.changed(activate(tasks.root(), "files"));
        assert_eq!(tasks.transition(), Some(layout));
        let root = tasks.root().clone();
        tasks.changed(resize(&root, root.id(), &[0.3, 0.7]));
        assert_eq!(tasks.transition(), Some(layout));
        tasks.add_panel("toolbox");
        assert!(tasks.transition().unwrap().revision > layout.revision);
        let added = tasks.transition();
        tasks.add_panel("toolbox");
        tasks.close_panel("editor");
        assert_eq!(tasks.transition(), added);
        tasks.reset_current();
        assert_eq!(tasks.transition().unwrap().kind, TransitionKind::Layout);
    }

    /// row [ stack s1 (files, git) | column [ stack s2 (editor) | stack s3 (terminal, problems) ] ]
    fn tree() -> DockNode {
        N::split(
            "n1",
            Dir::Row,
            vec![
                (N::stack("s1", &["files", "git"]), 0.25),
                (
                    N::split(
                        "n2",
                        Dir::Column,
                        vec![(N::stack("s2", &["editor"]), 0.7), (N::stack("s3", &["terminal", "problems"]), 0.3)],
                    ),
                    0.75,
                ),
            ],
        )
    }

    fn stack_of(root: &DockNode, panel: &str) -> (String, Vec<String>, String, bool) {
        match find_stack(root, panel) {
            Some(DockNode::Stack { id, panels, active, collapsed }) => {
                (id.clone(), panels.clone(), active.clone(), *collapsed)
            }
            _ => panic!("{panel} not placed"),
        }
    }

    #[test]
    fn default_arrangements_place_only_known_panels_each_once() {
        for task in TASKS {
            let placed = placed_panels(&default_arrangement(task));
            let set: HashSet<&String> = placed.iter().collect();
            assert_eq!(set.len(), placed.len(), "{task}");
            assert!(placed.iter().all(|p| PANEL_IDS.contains(&p.as_str())), "{task}");
        }
        assert!(placed_panels(&default_arrangement("code")).contains(&EDITOR.to_owned()));
        let connect = placed_panels(&default_arrangement("connect"));
        assert!(connect.contains(&"graph".to_owned()) && connect.contains(&"toolbox".to_owned()));
        assert!(!connect.contains(&EDITOR.to_owned()) && !connect.contains(&"box".to_owned()));
    }

    #[test]
    fn api_box_editor_gets_room_for_its_test_result_preview() {
        let mut tasks = Tasks::new(&SavedLayouts::new());
        tasks.switch_to("connect");
        tasks.reveal("box");
        let DockNode::Split { dir: Dir::Row, sizes, .. } = tasks.root() else {
            panic!("API box should open beside the Connect workspace");
        };
        assert_eq!(sizes, &[0.4, 0.6]);
        assert_eq!(stack_of(tasks.root(), "box").2, "box");
    }

    #[test]
    fn centre_joins_the_target_stack_and_becomes_active() {
        let r = insert_panel(&tree(), "output", "s3", Zone::Center);
        assert_eq!(
            stack_of(&r, "output"),
            ("s3".into(), vec!["terminal".into(), "problems".into(), "output".into()], "output".into(), false)
        );
    }

    #[test]
    fn left_right_wrap_in_a_row_top_bottom_in_a_column() {
        let r = insert_panel(&N::stack("s9", &["editor"]), "where", "s9", Zone::Right);
        let N::Split { dir, children, sizes, .. } = &r else { panic!("split") };
        assert_eq!((*dir, sizes.clone()), (Dir::Row, vec![0.5, 0.5]));
        assert_eq!(
            children.iter().map(placed_panels).collect::<Vec<_>>(),
            [vec!["editor".to_owned()], vec!["where".to_owned()]]
        );
        let t = insert_panel(&N::stack("s9", &["editor"]), "terminal", "s9", Zone::Top);
        let N::Split { dir, children, .. } = &t else { panic!("split") };
        assert_eq!(*dir, Dir::Column);
        assert_eq!(placed_panels(&children[0]), ["terminal"]);
    }

    #[test]
    fn next_to_a_stack_in_a_split_of_the_same_direction_becomes_a_sibling() {
        let r = insert_panel(&tree(), "where", "s3", Zone::Bottom);
        let N::Split { children, .. } = &r else { panic!("split") };
        let N::Split { children: col, sizes, .. } = &children[1] else { panic!("column") };
        assert_eq!(col.len(), 3);
        assert_eq!(placed_panels(&col[2]), ["where"]);
        assert!((sizes[0] - 0.7).abs() < 1e-9 && (sizes[1] - 0.15).abs() < 1e-9 && (sizes[2] - 0.15).abs() < 1e-9);
    }

    #[test]
    fn removing_the_last_panel_of_a_stack_removes_it_and_a_one_child_split_collapses() {
        let r = remove_panel(&tree(), "editor").expect("left");
        let N::Split { dir, children, sizes, .. } = &r else { panic!("split") };
        assert_eq!(*dir, Dir::Row);
        assert_eq!(children[1].id(), "s3");
        assert_eq!(sizes, &vec![0.25, 0.75]);
    }

    #[test]
    fn the_active_panel_moves_to_a_neighbour_when_removed() {
        let r = remove_panel(&activate(&tree(), "problems"), "problems").expect("left");
        assert_eq!(stack_of(&r, "terminal").1, ["terminal"]);
        assert_eq!(stack_of(&r, "terminal").2, "terminal");
        assert_eq!(remove_panel(&N::stack("s1", &["files"]), "files"), None);
    }

    #[test]
    fn moving_onto_itself_changes_nothing_and_moves_clean_up() {
        let t = tree();
        assert_eq!(move_panel(&t, "terminal", "s3", Zone::Center), t);
        assert_eq!(move_panel(&t, "editor", "s2", Zone::Left), t);
        let r = move_panel(&t, "git", "s2", Zone::Right);
        assert_eq!(stack_of(&r, "files").1, ["files"]);
        assert_eq!(stack_of(&r, "git").1, ["git"]);
        let mut placed = placed_panels(&r);
        placed.sort();
        assert_eq!(placed, ["editor", "files", "git", "problems", "terminal"]);
    }

    #[test]
    fn resize_clamps_to_the_minimum_and_keeps_the_sum_at_one() {
        let N::Split { sizes, .. } = resize(&tree(), "n1", &[0.99, 0.01]) else { panic!("split") };
        assert!((sizes[0] - 0.92).abs() < 1e-9 && (sizes[1] - 0.08).abs() < 1e-9);
        let N::Split { sizes, .. } = resize(&tree(), "n1", &[-5.0, 2.0]) else { panic!("split") };
        assert!((sizes[0] - 0.08).abs() < 1e-9 && ((sizes[0] + sizes[1]) - 1.0).abs() < 1e-9);
        assert_eq!(resize(&tree(), "nope", &[0.5, 0.5]), tree());
    }

    #[test]
    fn activate_and_collapse() {
        assert_eq!(stack_of(&activate(&tree(), "problems"), "problems").2, "problems");
        assert!(stack_of(&set_collapsed(&tree(), "s3", true), "terminal").3);
    }

    #[test]
    fn drop_zones_centre_box_else_the_nearest_edge() {
        assert_eq!(zone_at(0.0, 0.0, 100.0, 100.0, 50.0, 50.0), Zone::Center);
        assert_eq!(zone_at(0.0, 0.0, 100.0, 100.0, 5.0, 50.0), Zone::Left);
        assert_eq!(zone_at(0.0, 0.0, 100.0, 100.0, 95.0, 40.0), Zone::Right);
        assert_eq!(zone_at(0.0, 0.0, 100.0, 100.0, 50.0, 3.0), Zone::Top);
        assert_eq!(zone_at(0.0, 0.0, 100.0, 100.0, 50.0, 97.0), Zone::Bottom);
    }

    #[test]
    fn insert_at_edge_wraps_the_whole_layout() {
        let r = insert_at_edge(&tree(), "output", Zone::Bottom, 0.3);
        let N::Split { dir, children, sizes, .. } = &r else { panic!("split") };
        assert_eq!((*dir, sizes.clone()), (Dir::Column, vec![0.7, 0.3]));
        assert_eq!(placed_panels(&children[1]), ["output"]);
        let l = insert_at_edge(&N::stack("s1", &["editor"]), "files", Zone::Left, 0.2);
        let N::Split { children, sizes, .. } = &l else { panic!("split") };
        assert_eq!(sizes, &vec![0.2, 0.8]);
        assert_eq!(placed_panels(&children[0]), ["files"]);
    }

    #[test]
    fn layouts_json_round_trips_in_the_old_format() {
        let json = serde_json::to_value(tree()).expect("json");
        assert_eq!(json["kind"], "split");
        assert_eq!(json["children"][0]["kind"], "stack");
        assert!(json["children"][0].get("collapsed").is_none());
        let mut raw = serde_json::Map::new();
        raw.insert("code".into(), json);
        let saved = normalize_layouts(&Value::Object(raw));
        assert_eq!(saved.get("code"), Some(&tree()));
    }

    #[test]
    fn normalizing_drops_unknown_panels_duplicates_and_codes_without_the_editor() {
        let raw = serde_json::json!({
            "code": { "kind": "stack", "id": "s1", "panels": ["files", "files", "nope"], "active": "x" },
            "run": { "kind": "split", "id": "a", "dir": "row", "sizes": [3, 1], "children": [
                { "kind": "stack", "id": "a", "panels": ["editor"] },
                { "kind": "stack", "id": "b", "panels": [] }
            ]},
            "debug": "garbage"
        });
        let saved = normalize_layouts(&raw);
        assert!(!saved.contains_key("code")); // no editor
        assert!(!saved.contains_key("debug"));
        // The split lost its empty child and collapsed to the remaining stack (id made unique).
        assert_eq!(saved.get("run").map(placed_panels), Some(vec!["editor".to_owned()]));
        assert_eq!(saved.get("run").map(|n| n.id().to_owned()), Some("a-2".to_owned()));
    }

    #[test]
    fn tasks_switch_and_reveal() {
        let mut t = Tasks::new(&SavedLayouts::new());
        t.switch_to("connect");
        assert!(t.is_shown("graph"));
        // A panel the task does not place appears in the drawer.
        t.reveal("tests");
        assert!(t.is_shown("tests"));
        assert!(stack_of(t.root(), "tests").1.contains(&"terminal".to_owned()));
        // Side panels join the files stack.
        t.reveal("git");
        assert!(stack_of(t.root(), "git").1.contains(&"files".to_owned()));
        // The editor switches to Code, the graph to Connect, when not placed.
        t.reveal(EDITOR);
        assert_eq!(t.current(), "code");
        t.reveal("graph");
        assert_eq!(t.current(), "connect");
    }

    #[test]
    fn toolbox_and_box_settings_switch_to_connect() {
        let mut t = Tasks::new(&SavedLayouts::new());
        t.reveal("box");
        assert_eq!(t.current(), "connect");
        assert!(t.is_shown("box"));
        t.close_panel("toolbox");
        t.switch_to("code");
        t.reveal("toolbox");
        assert_eq!(t.current(), "connect");
        assert!(t.is_shown("toolbox"));
        assert!(!stack_of(t.root(), "toolbox").1.contains(&"terminal".to_owned()));
    }

    #[test]
    fn drawer_toggles_and_reveal_opens_a_collapsed_drawer() {
        let mut t = Tasks::new(&SavedLayouts::new());
        t.switch_to("debug");
        t.toggle_drawer();
        assert!(!t.is_shown("terminal"));
        t.reveal("problems");
        assert!(t.is_shown("problems"));
        let mut saved = SavedLayouts::new();
        saved.insert("run".into(), N::stack("s1", &["editor"]));
        let mut t = Tasks::new(&saved);
        t.switch_to("run");
        t.toggle_drawer();
        assert!(t.is_shown("terminal"));
        t.toggle_drawer();
        assert!(!t.is_shown("terminal"));
    }

    #[test]
    fn the_editor_cannot_be_closed_other_panels_can_and_come_back() {
        let mut t = Tasks::new(&SavedLayouts::new());
        t.close_panel(EDITOR);
        assert!(placed_panels(t.root()).contains(&EDITOR.to_owned()));
        t.close_panel("where");
        assert!(t.hidden_panels().contains(&"where"));
        t.add_panel("where");
        assert!(placed_panels(t.root()).contains(&"where".to_owned()));
        assert!(t.dirty);
    }

    #[test]
    fn debugging_switches_to_debug_and_back_unless_turned_off_or_moved_on() {
        let mut t = Tasks::new(&SavedLayouts::new());
        t.switch_to("run");
        t.begin_debug(true);
        assert_eq!(t.current(), "debug");
        t.end_debug();
        assert_eq!(t.current(), "run");
        t.begin_debug(false);
        assert_eq!(t.current(), "run");
        t.begin_debug(true);
        t.switch_to("connect");
        t.end_debug();
        assert_eq!(t.current(), "connect");
    }

    #[test]
    fn saved_arrangements_replace_defaults_and_reset_restores_them() {
        let mut t = Tasks::new(&SavedLayouts::new());
        let mut saved = SavedLayouts::new();
        saved.insert("code".into(), N::stack("s1", &["editor", "files"]));
        t.use_saved(&saved);
        assert_eq!(placed_panels(t.root()), ["editor", "files"]);
        t.reset_current();
        assert_eq!(t.root(), &default_arrangement("code"));
    }
}
