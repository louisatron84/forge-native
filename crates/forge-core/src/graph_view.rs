//! Modules, Calls and Events projections and filters, replacing
//! `src/renderer/graph/viewModel.ts`. Source navigation uses zero-based lines.
use crate::project_graph::{ProjectGraph, SymbolNode};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
pub const MAX_CALL_NODES: usize = 400;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GraphMode {
    #[default]
    Modules,
    Calls,
    Events,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ViewOptions {
    pub mode: GraphMode,
    pub focus: Option<String>,
    pub show_external: bool,
    pub show_tests: bool,
    pub show_hosts: bool,
    pub query: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ViewNode {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub kind: String,
    pub group: String,
    pub module: Option<String>,
    pub line: Option<usize>,
    pub entry: bool,
    pub error: bool,
    pub test: bool,
    pub unused: bool,
    pub cyclic: bool,
    #[serde(rename = "match")]
    pub matches: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub box_rows: Vec<BoxRow>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BoxRow {
    pub text: String,
    pub ok: Option<bool>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ViewEdge {
    pub from: String,
    pub to: String,
    pub kind: String,
    pub cyclic: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GraphViewData {
    pub nodes: Vec<ViewNode>,
    pub edges: Vec<ViewEdge>,
    pub note: Option<String>,
}
fn edge(from: &str, to: &str, kind: &str) -> ViewEdge {
    ViewEdge { from: from.into(), to: to.into(), kind: kind.into(), cyclic: false }
}
fn group(module: &str) -> String {
    module.split('.').next().unwrap_or("").into()
}
fn module_of(id: &str) -> &str {
    id.strip_prefix("module:").unwrap_or_else(|| id.split(':').next().unwrap_or(id))
}
fn symbol_node(s: &SymbolNode, options: &ViewOptions, test: bool) -> ViewNode {
    ViewNode {
        id: s.id.clone(),
        label: if options.focus.as_ref() == Some(&s.module) {
            s.qual.clone()
        } else {
            format!("{}.{}", s.module, s.qual)
        },
        detail: format!("{} · line {}", s.kind, s.line),
        kind: s.kind.clone(),
        group: group(&s.module),
        module: Some(s.module.clone()),
        line: Some(s.line.saturating_sub(1)),
        test,
        ..Default::default()
    }
}
pub fn build_view(graph: &ProjectGraph, options: &ViewOptions) -> GraphViewData {
    let mut data = match options.mode {
        GraphMode::Modules => modules_view(graph, options),
        GraphMode::Calls => calls_view(graph, options),
        GraphMode::Events => events_view(graph, options),
    };
    let query = options.query.trim().to_lowercase();
    if query.is_empty() {
        return data;
    }
    let matches: HashSet<_> =
        data.nodes.iter().filter(|n| n.label.to_lowercase().contains(&query)).map(|n| n.id.clone()).collect();
    if matches.is_empty() {
        return GraphViewData {
            note: Some(format!("Nothing matches \"{}\".", options.query.trim())),
            ..Default::default()
        };
    }
    let mut keep = matches.clone();
    for e in &data.edges {
        if matches.contains(&e.from) {
            keep.insert(e.to.clone());
        }
        if matches.contains(&e.to) {
            keep.insert(e.from.clone());
        }
    }
    data.nodes.retain(|n| keep.contains(&n.id));
    for n in &mut data.nodes {
        n.matches = matches.contains(&n.id);
    }
    data.edges.retain(|e| keep.contains(&e.from) && keep.contains(&e.to));
    data.note = None;
    data
}
fn modules_view(g: &ProjectGraph, o: &ViewOptions) -> GraphViewData {
    let cyclic: HashSet<_> = g.cycles.iter().flatten().collect();
    let touched: HashSet<_> = g.imports.iter().flat_map(|e| [&e.from, &e.to]).collect();
    let mut nodes: Vec<_> = g
        .modules
        .iter()
        .filter(|m| {
            !(m.is_test && !o.show_tests
                || m.is_package && m.symbols.is_empty() && m.lines <= 2 && !touched.contains(&m.id))
        })
        .map(|m| {
            let mut detail = format!("{} line{}", m.lines, if m.lines == 1 { "" } else { "s" });
            if !m.routes.is_empty() {
                detail.push_str(&format!(" · {} route{}", m.routes.len(), if m.routes.len() == 1 { "" } else { "s" }));
            }
            if m.error.is_some() {
                detail.push_str(" · could not read");
            }
            ViewNode {
                id: m.id.clone(),
                label: m.id.clone(),
                detail,
                kind: "module".into(),
                group: group(&m.id),
                module: Some(m.id.clone()),
                line: Some(0),
                entry: m.is_entry,
                error: m.error.is_some(),
                test: m.is_test,
                cyclic: cyclic.contains(&m.id),
                ..Default::default()
            }
        })
        .collect();
    let shown: HashSet<_> = nodes.iter().map(|n| n.id.clone()).collect();
    let mut edges: Vec<_> = g
        .imports
        .iter()
        .filter(|e| shown.contains(&e.from) && shown.contains(&e.to))
        .map(|e| ViewEdge { cyclic: e.cyclic, ..edge(&e.from, &e.to, "import") })
        .collect();
    if o.show_external {
        for x in &g.externals {
            let by: Vec<_> = x.imported_by.iter().filter(|m| shown.contains(*m)).collect();
            if by.is_empty() {
                continue;
            }
            let id = format!("ext:{}", x.name);
            nodes.push(ViewNode {
                id: id.clone(),
                label: x.name.clone(),
                detail: "package".into(),
                kind: "external".into(),
                ..Default::default()
            });
            edges.extend(by.into_iter().map(|m| edge(m, &id, "external")));
        }
    }
    if o.show_hosts {
        let mut hosts: Vec<(String, BTreeSet<String>, BTreeSet<String>)> = Vec::new();
        let mut indices = HashMap::new();
        // Aggregate hosts, modules and methods together, avoiding one scan of
        // the entire request list per host as in the Electron implementation.
        for h in &g.http {
            let Some(host) = &h.host else {
                continue;
            };
            let i = *indices.entry(host.clone()).or_insert_with(|| {
                hosts.push((host.clone(), BTreeSet::new(), BTreeSet::new()));
                hosts.len() - 1
            });
            hosts[i].2.insert(h.raw.method.clone());
            if shown.contains(&h.module) {
                hosts[i].1.insert(h.module.clone());
            }
        }
        for (host, modules, methods) in hosts {
            if modules.is_empty() {
                continue;
            }
            let id = format!("host:{host}");
            nodes.push(ViewNode {
                id: id.clone(),
                label: host,
                detail: format!("web API · {}", methods.into_iter().collect::<Vec<_>>().join(" ")),
                kind: "host".into(),
                ..Default::default()
            });
            edges.extend(modules.iter().map(|m| edge(m, &id, "http")));
        }
    }
    let note = nodes.is_empty().then(|| "No Python modules found in this folder.".into());
    GraphViewData { nodes, edges, note }
}
fn calls_view(g: &ProjectGraph, o: &ViewOptions) -> GraphViewData {
    let modules: HashMap<_, _> = g.modules.iter().map(|m| (m.id.as_str(), m)).collect();
    let symbols: HashMap<_, _> = g.modules.iter().flat_map(|m| m.symbols.iter().map(|s| (s.id.as_str(), s))).collect();
    let visible = |id: &str| o.show_tests || !modules.get(module_of(id)).is_some_and(|m| m.is_test);
    let edges: Vec<_> = g
        .calls
        .iter()
        .filter(|c| {
            visible(&c.from)
                && visible(&c.to)
                && o.focus.as_ref().is_none_or(|f| module_of(&c.from) == f || module_of(&c.to) == f)
        })
        .collect();
    let targets: HashSet<_> = edges.iter().map(|e| e.to.as_str()).collect();
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |id: &str| {
        if seen.insert(id.to_owned()) {
            ids.push(id.to_owned());
        }
    };
    if let Some(m) = o.focus.as_ref().and_then(|f| modules.get(f.as_str())) {
        for s in &m.symbols {
            if s.kind != "class" || targets.contains(s.id.as_str()) {
                add(&s.id);
            }
        }
    }
    for c in &edges {
        add(&c.from);
        add(&c.to);
    }
    if o.focus.is_none() && ids.len() > MAX_CALL_NODES {
        return GraphViewData{note:Some(format!("This project has {} functions with calls: too many to draw at once. Pick a module (click it in the Modules view, then Calls).",ids.len())),..Default::default()};
    }
    let unused: HashSet<_> = g.unused.iter().map(|s| s.id.as_str()).collect();
    let nodes: Vec<_> = ids
        .iter()
        .filter_map(|id| {
            if let Some(m) = id.strip_prefix("module:") {
                return Some(ViewNode {
                    id: id.clone(),
                    label: m.into(),
                    detail: "top-level code".into(),
                    kind: "code".into(),
                    group: group(m),
                    module: Some(m.into()),
                    line: Some(0),
                    ..Default::default()
                });
            }
            symbols.get(id.as_str()).map(|s| {
                let mut node = symbol_node(s, o, modules.get(s.module.as_str()).is_some_and(|m| m.is_test));
                node.unused = unused.contains(id.as_str());
                node
            })
        })
        .collect();
    let note = nodes.is_empty().then(|| {
        o.focus
            .as_ref()
            .map(|f| format!("No functions or calls found in {f}."))
            .unwrap_or_else(|| "No calls between the project's functions were found.".into())
    });
    GraphViewData { nodes, edges: edges.iter().map(|c| edge(&c.from, &c.to, "call")).collect(), note }
}
fn events_view(g: &ProjectGraph, o: &ViewOptions) -> GraphViewData {
    let modules: HashMap<_, _> = g.modules.iter().map(|m| (m.id.as_str(), m)).collect();
    let symbols: HashMap<_, _> = g.modules.iter().flat_map(|m| m.symbols.iter().map(|s| (s.id.as_str(), s))).collect();
    let visible = |id: &str| o.show_tests || !modules.get(module_of(id)).is_some_and(|m| m.is_test);
    let mut nodes = Vec::new();
    let mut shown = HashSet::new();
    let mut add_function = |id: &str| {
        if shown.contains(id) {
            return;
        }
        let m = module_of(id);
        let node = if id.starts_with("module:") {
            Some(ViewNode {
                id: id.into(),
                label: m.into(),
                detail: "top-level code".into(),
                kind: "code".into(),
                group: group(m),
                module: Some(m.into()),
                line: Some(0),
                ..Default::default()
            })
        } else {
            symbols.get(id).map(|s| symbol_node(s, o, modules.get(m).is_some_and(|m| m.is_test)))
        };
        if let Some(node) = node {
            shown.insert(id.into());
            nodes.push(node);
        }
    };
    let mut edges = Vec::new();
    let mut edge_seen = HashSet::new();
    let mut add_edge = |from: &str, to: &str, kind: &str| {
        if edge_seen.insert((from.to_owned(), to.to_owned())) {
            edges.push(edge(from, to, kind));
        }
    };
    let mut events: Vec<(String, HashSet<String>, HashSet<String>)> = Vec::new();
    let mut indices = HashMap::new();
    let mut starts = HashSet::new();
    for u in &g.events {
        if !visible(&u.func) || u.target.as_ref().is_some_and(|t| !visible(t)) {
            continue;
        }
        if o.focus
            .as_ref()
            .is_some_and(|f| module_of(&u.func) != f && u.target.as_ref().is_none_or(|t| module_of(t) != f))
        {
            continue;
        }
        if u.kind == "spawns" {
            if let Some(target) = &u.target {
                add_function(&u.func);
                add_function(target);
                add_edge(&u.func, target, "spawns");
                starts.insert(target.clone());
            }
            continue;
        }
        let name = u.event.as_deref().unwrap_or("?");
        let id = format!("event:{name}");
        let i = *indices.entry(name.to_owned()).or_insert_with(|| {
            events.push((name.into(), HashSet::new(), HashSet::new()));
            events.len() - 1
        });
        add_function(&u.func);
        if u.kind == "handles" {
            events[i].2.insert(u.func.clone());
            add_edge(&id, &u.func, "handles");
            starts.insert(u.func.clone());
        } else {
            events[i].1.insert(u.func.clone());
            add_edge(&u.func, &id, "emits");
        }
    }
    for c in &g.calls {
        if starts.contains(&c.from) && visible(&c.to) && c.from != c.to {
            add_function(&c.to);
            add_edge(&c.from, &c.to, "call");
        }
    }
    for (name, emitters, handlers) in events {
        let from = if emitters.is_empty() {
            "from outside the code".into()
        } else {
            format!("emitted in {} place{}", emitters.len(), if emitters.len() == 1 { "" } else { "s" })
        };
        let id = format!("event:{name}");
        shown.insert(id.clone());
        nodes.push(ViewNode {
            id,
            label: name,
            detail: format!(
                "event · {from} · {} handler{}",
                handlers.len(),
                if handlers.len() == 1 { "" } else { "s" }
            ),
            kind: "event".into(),
            ..Default::default()
        });
    }
    edges.retain(|e| shown.contains(&e.from) && shown.contains(&e.to));
    let note=nodes.is_empty().then(||"No events found. Forge looks for handlers (@sio.on, @client.event, @receiver, .on(), .connect(), .bind(), command=), events emitted (emit(), send(), delay()), and work started with create_task(), Thread(target=...) or submit().".into());
    GraphViewData { nodes, edges, note }
}
pub fn neighbourhood(edges: &[ViewEdge], id: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    (
        edges.iter().filter(|e| e.from == id).map(|e| e.to.clone()).collect(),
        edges.iter().filter(|e| e.to == id).map(|e| e.from.clone()).collect(),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn module_call_event_filters_and_search_match_electron_fixtures() {
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../tests/fixtures/graph_views.json")).unwrap();
        for (index, case) in cases.iter().enumerate() {
            let scan = serde_json::from_value(case["scan"].clone()).unwrap();
            let graph = crate::project_graph::build_graph(&scan);
            let options = serde_json::from_value(case["options"].clone()).unwrap();
            let expected: GraphViewData = serde_json::from_value(case["expected"].clone()).unwrap();
            assert_eq!(build_view(&graph, &options), expected, "case {index}");
        }
    }
}
