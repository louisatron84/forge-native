//! Function data dependencies from the unchanged Python AST reader.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RawFlow {
    pub functions: Vec<Function>,
    pub error: Option<String>,
    pub func: Option<String>,
    pub line: u32,
    pub end_line: u32,
    pub params: Vec<Param>,
    pub returns: Option<String>,
    pub doc: String,
    pub steps: Option<Vec<Step>>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Function {
    pub qual: String,
    pub line: u32,
    pub end_line: u32,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Param {
    pub name: String,
    pub annotation: Option<String>,
    pub default: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Arg {
    pub kw: Option<String>,
    pub text: String,
    pub names: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Call {
    pub name: String,
    pub args: Vec<Arg>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Loop {
    pub targets: Vec<String>,
    pub over: String,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Step {
    pub line: u32,
    pub end_line: u32,
    pub kind: String,
    pub code: String,
    pub targets: Vec<String>,
    pub uses: Vec<String>,
    pub call: Option<Call>,
    #[serde(rename = "loop")]
    pub loop_: Option<Loop>,
    pub body: Option<Vec<Step>>,
}
#[derive(Clone, Debug)]
pub struct FlowNode {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub line: u32,
    pub end_line: u32,
    pub sets: Vec<String>,
    pub parent: Option<String>,
    pub depth: usize,
    pub call: Option<Call>,
}
impl FlowNode {
    pub fn container(&self) -> bool {
        matches!(self.kind.as_str(), "for" | "if" | "while" | "with" | "try")
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct FlowEdge {
    pub from: String,
    pub to: String,
    pub name: String,
}
#[derive(Clone, Debug)]
pub struct Flow {
    pub func: String,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
    pub origins: BTreeMap<String, BTreeMap<String, String>>,
}
pub fn build_flow(raw: &RawFlow) -> Option<Flow> {
    let func = raw.func.as_ref().filter(|s| !s.is_empty())?;
    let steps = raw.steps.as_ref()?;
    let mut flow = Flow { func: func.clone(), nodes: Vec::new(), edges: Vec::new(), origins: BTreeMap::new() };
    let mut last = HashMap::new();
    for p in &raw.params {
        if p.name == "self" || p.name == "cls" {
            continue;
        }
        let id = format!("param:{}", p.name);
        flow.nodes.push(FlowNode {
            id: id.clone(),
            kind: "param".into(),
            label: p
                .annotation
                .as_ref()
                .filter(|s| !s.is_empty())
                .map_or_else(|| p.name.clone(), |a| format!("{}: {a}", p.name)),
            line: raw.line,
            end_line: raw.line,
            sets: vec![p.name.clone()],
            parent: None,
            depth: 0,
            call: None,
        });
        last.insert(p.name.clone(), id);
    }
    // Enter/exit stack preserves Python statement order without recursive traversal,
    // repeated node searches or cloning each nested body's whole subtree.
    enum Visit<'a> {
        Enter(&'a Step, Option<String>, usize),
        Exit(&'a Step, String),
    }
    let mut todo: Vec<_> = steps.iter().rev().map(|s| Visit::Enter(s, None, 0)).collect();
    let mut ids: HashMap<u32, usize> = HashMap::new();
    while let Some(visit) = todo.pop() {
        match visit {
            Visit::Exit(s, id) => {
                for t in &s.targets {
                    last.insert(t.clone(), id.clone());
                }
            }
            Visit::Enter(s, parent, depth) => {
                let count = ids.entry(s.line).or_default();
                *count += 1;
                let id = if *count == 1 { format!("s{}", s.line) } else { format!("s{}_{}", s.line, count) };
                flow.nodes.push(FlowNode {
                    id: id.clone(),
                    kind: s.kind.clone(),
                    label: s.code.clone(),
                    line: s.line,
                    end_line: s.end_line,
                    sets: s.targets.clone(),
                    parent,
                    depth,
                    call: s.call.clone(),
                });
                let mut origins = BTreeMap::new();
                for name in &s.uses {
                    if let Some(origin) = last.get(name) {
                        if origins.insert(name.clone(), origin.clone()).is_none() {
                            flow.edges.push(FlowEdge { from: origin.clone(), to: id.clone(), name: name.clone() });
                        }
                    }
                }
                flow.origins.insert(id.clone(), origins);
                if let Some(loop_) = &s.loop_ {
                    for t in &loop_.targets {
                        last.insert(t.clone(), id.clone());
                    }
                }
                todo.push(Visit::Exit(s, id.clone()));
                if let Some(body) = &s.body {
                    todo.extend(body.iter().rev().map(|s| Visit::Enter(s, Some(id.clone()), depth + 1)));
                }
            }
        }
    }
    Some(flow)
}
#[derive(Clone, Copy)]
pub enum Direction {
    Up,
    Down,
}
pub fn dependencies(flow: &Flow, id: &str, direction: Direction) -> BTreeSet<String> {
    let mut adjacent: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in &flow.edges {
        let (from, to) = match direction {
            Direction::Up => (&e.to, &e.from),
            Direction::Down => (&e.from, &e.to),
        };
        adjacent.entry(from).or_default().push(to);
    }
    let mut out = BTreeSet::new();
    let mut todo = vec![id];
    while let Some(cur) = todo.pop() {
        for next in adjacent.get(cur).into_iter().flatten() {
            if out.insert((*next).to_string()) {
                todo.push(next);
            }
        }
    }
    out
}
pub fn guess_pipeline<'a>(text: &str, funcs: &'a [Function]) -> Option<&'a str> {
    let top = || funcs.iter().filter(|f| !f.qual.contains('.'));
    top()
        .find(|f| {
            let name = f.qual.to_lowercase();
            ["pipeline", "etl", "report", "process"].iter().any(|part| name.contains(part))
        })
        .or_else(|| if text.contains("forge_blocks") || text.contains("@block") { top().next_back() } else { None })
        .or_else(|| funcs.first())
        .map(|f| f.qual.as_str())
}
