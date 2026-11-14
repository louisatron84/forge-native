//! Debug pauses, variable history, recursive call trees and bounded object graphs.
use crate::project_graph::ProjectGraph;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
pub type Vars = BTreeMap<String, String>;
#[derive(Clone, Debug)]
pub struct Stop {
    pub n: u64,
    pub reason: String,
    pub stack: Vec<String>,
    pub vars: Vars,
    pub new_edges: Vec<(String, String)>,
}
#[derive(Default)]
pub struct DebugTrail {
    pub stops: Vec<Stop>,
    pub edges: BTreeMap<(String, String), u64>,
    counter: u64,
}
pub struct HistoryRow {
    pub name: String,
    pub values: Vec<Option<String>>,
    pub changed: Vec<bool>,
}
pub struct History {
    pub stops: Vec<u64>,
    pub rows: Vec<HistoryRow>,
}
#[derive(Default)]
pub struct VarDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
    pub same: Vec<String>,
}
pub fn diff_vars(a: &Vars, b: &Vars) -> VarDiff {
    let mut out = VarDiff::default();
    for (k, v) in a {
        match b.get(k) {
            None => out.removed.push(k.clone()),
            Some(w) if w == v => out.same.push(k.clone()),
            _ => out.changed.push(k.clone()),
        }
    }
    for k in b.keys() {
        if !a.contains_key(k) {
            out.added.push(k.clone());
        }
    }
    out
}
impl DebugTrail {
    pub fn add(&mut self, reason: String, stack: Vec<String>, vars: Vars) -> &Stop {
        let mut new_edges = Vec::new();
        for pair in stack.windows(2).rev() {
            if pair[0] == pair[1] {
                continue;
            }
            let key = (pair[1].clone(), pair[0].clone());
            let count = self.edges.entry(key.clone()).or_default();
            if *count == 0 {
                new_edges.push(key);
            }
            *count += 1;
        }
        self.counter += 1;
        self.stops.push(Stop { n: self.counter, reason, stack, vars, new_edges });
        if self.stops.len() > 200 {
            self.stops.remove(0);
        }
        self.stops.last().unwrap()
    }
    pub fn history(&self, node: &str) -> History {
        let stops = self.stops.iter().filter(|s| s.stack.first().is_some_and(|n| n == node)).collect::<Vec<_>>();
        let mut names = Vec::new();
        for s in &stops {
            for k in s.vars.keys() {
                if !names.contains(k) {
                    names.push(k.clone());
                }
            }
        }
        let rows = names
            .into_iter()
            .map(|name| {
                let values = stops.iter().map(|s| s.vars.get(&name).cloned()).collect::<Vec<_>>();
                let changed = values.iter().enumerate().map(|(i, v)| i > 0 && *v != values[i - 1]).collect();
                HistoryRow { name, values, changed }
            })
            .collect();
        History { stops: stops.iter().map(|s| s.n).collect(), rows }
    }
}
pub fn frame_node(path: &str, line0: usize, graph: &ProjectGraph, root: &str) -> Option<String> {
    let file = path.replace('\\', "/");
    let base = format!("{}/", root.replace('\\', "/").trim_end_matches('/'));
    let fold = file.as_bytes().get(1) == Some(&b':') || base.as_bytes().get(1) == Some(&b':');
    let (file, base) = if fold { (file.to_lowercase(), base.to_lowercase()) } else { (file, base) };
    let rel = file.strip_prefix(&base)?;
    let module = graph.modules.iter().find(|m| if fold { m.path.to_lowercase() == rel } else { m.path == rel })?;
    module
        .symbols
        .iter()
        .filter(|s| s.line <= line0 + 1 && s.end_line > line0)
        .min_by_key(|s| s.end_line - s.line)
        .map(|s| s.id.clone())
        .or_else(|| Some(format!("module:{}", module.id)))
}
#[derive(Clone, Debug)]
pub struct RecNode {
    pub parent: i64,
    pub args: String,
    pub ret: Option<String>,
}
#[derive(Clone, Debug)]
pub struct RecTree {
    pub key: String,
    pub func: String,
    pub total: u64,
    pub nodes: Vec<RecNode>,
}
pub struct TreeLayout {
    pub positions: Vec<(f64, usize)>,
    pub columns: usize,
    pub depth: usize,
}
pub fn parse_rec(v: &Value) -> Option<RecTree> {
    if v.get("t")?.as_str()? != "rec" {
        return None;
    }
    let t = v.get("tree")?;
    let raw = t.get("nodes")?.as_array()?;
    if raw.is_empty() || raw.len() > 10_000 {
        return None;
    }
    let mut nodes = Vec::new();
    for (i, n) in raw.iter().enumerate() {
        let n = n.as_array()?;
        if n.len() != 3 {
            return None;
        }
        let parent = n[0].as_i64()?;
        if if i == 0 { parent != -1 } else { parent < 0 || parent as usize >= i } {
            return None;
        }
        nodes.push(RecNode {
            parent,
            args: n[1].as_str()?.into(),
            ret: if n[2].is_null() { None } else { Some(n[2].as_str()?.into()) },
        });
    }
    Some(RecTree {
        key: t.get("key")?.as_str()?.into(),
        func: t.get("func")?.as_str()?.into(),
        total: t.get("total").and_then(Value::as_u64).unwrap_or(nodes.len() as u64),
        nodes,
    })
}
pub fn tree_layout(tree: &RecTree) -> TreeLayout {
    let mut kids = vec![Vec::new(); tree.nodes.len()];
    for (i, n) in tree.nodes.iter().enumerate() {
        if n.parent >= 0 {
            kids[n.parent as usize].push(i);
        }
    }
    let mut positions = vec![(0., 0); tree.nodes.len()];
    let (mut next, mut depth) = (0, 0);
    let mut todo = vec![(0, 0, false)];
    while let Some((i, d, done)) = todo.pop() {
        positions[i].1 = d;
        depth = depth.max(d + 1);
        if !done && !kids[i].is_empty() {
            todo.push((i, d, true));
            todo.extend(kids[i].iter().rev().map(|k| (*k, d + 1, false)));
            continue;
        }
        positions[i].0 = if kids[i].is_empty() {
            let x = next;
            next += 1;
            x as f64
        } else {
            (positions[kids[i][0]].0 + positions[*kids[i].last().unwrap()].0) / 2.
        };
    }
    TreeLayout { positions, columns: next, depth }
}
pub const OBJECT_HELPER: &str = include_str!("../../../python/objectGraph.py");
pub fn object_expression() -> String {
    format!(
        "(lambda g,L:(exec({},g),g['forge_objects'](L))[1])({{}},locals())",
        serde_json::to_string(OBJECT_HELPER).unwrap()
    )
}
pub fn unrepr_string(s: &str) -> String {
    let s = s.trim();
    if !((s.starts_with('\'') && s.ends_with('\'')) || (s.starts_with('"') && s.ends_with('"'))) || s.len() < 2 {
        return s.into();
    }
    let mut out = String::new();
    let mut chars = s[1..s.len() - 1].chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(e) = chars.next() else {
            break;
        };
        match e {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '0' => out.push('\0'),
            'x' | 'u' | 'U' => {
                let n = match e {
                    'x' => 2,
                    'u' => 4,
                    _ => 8,
                };
                let hex = chars.by_ref().take(n).collect::<String>();
                if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(c);
                }
            }
            _ => out.push(e),
        }
    }
    out
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ObjNode {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub fields: Vec<(String, String)>,
    pub more: usize,
    pub len: Option<usize>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ObjGraph {
    pub roots: Vec<(String, String)>,
    pub nodes: Vec<ObjNode>,
    pub edges: Vec<(String, String, String)>,
}
pub fn parse_object_graph(result: &str) -> Option<ObjGraph> {
    let mut graph: ObjGraph = serde_json::from_str(&unrepr_string(result)).ok()?;
    graph.nodes.truncate(40);
    for n in &mut graph.nodes {
        n.fields.truncate(16);
    }
    let ids: BTreeSet<_> = graph.nodes.iter().map(|n| n.id.clone()).collect();
    graph.roots.retain(|(_, id)| ids.contains(id));
    graph.roots.truncate(100);
    graph.edges.retain(|(a, b, _)| ids.contains(a) && ids.contains(b));
    graph.edges.truncate(640);
    Some(graph)
}
