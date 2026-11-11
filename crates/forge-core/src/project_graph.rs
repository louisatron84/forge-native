//! Static project intelligence, replacing `src/shared/projectGraph.ts`.
//! Scanner messages keep their Electron JSON names; dynamic targets stay
//! unresolved rather than guessing. All source positions here are 1-based.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RawScan {
    pub root: String,
    pub python: String,
    pub truncated: bool,
    pub files: Vec<RawFile>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RawFile {
    pub path: String,
    pub module: String,
    pub is_package: bool,
    pub imports: Vec<RawImport>,
    pub defs: Vec<RawDef>,
    pub routes: Vec<RawRoute>,
    pub http: Vec<RawHttp>,
    pub env: Vec<RawEnv>,
    pub unreachable: Vec<RawUnreachable>,
    pub lines: usize,
    pub error: Option<String>,
    pub all: Vec<String>,
    pub main: bool,
    pub generated: Option<serde_json::Value>,
    pub db: Option<serde_json::Value>,
    pub events: Vec<RawEvent>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RawImport {
    pub module: String,
    pub names: Vec<String>,
    pub alias: Option<String>,
    pub level: usize,
    pub line: usize,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RawCall {
    pub name: String,
    pub line: usize,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RawParamSig {
    pub name: String,
    pub annotation: Option<String>,
    pub default: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RawDef {
    pub name: String,
    pub qual: String,
    pub kind: String,
    pub line: usize,
    pub end_line: usize,
    pub calls: Vec<RawCall>,
    pub refs: Vec<String>,
    pub decorators: Vec<String>,
    pub params: Option<Vec<RawParamSig>>,
    pub returns: Option<String>,
    pub doc: Option<String>,
    pub bases: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RawRoute {
    pub methods: Vec<String>,
    pub path: String,
    pub func: String,
    pub line: usize,
    pub framework: String,
    pub owner: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RawHttp {
    pub library: String,
    pub method: String,
    pub url: Option<String>,
    pub line: usize,
    pub func: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RawEnv {
    pub name: String,
    pub line: usize,
    pub func: Option<String>,
    pub default: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RawUnreachable {
    pub line: usize,
    pub end_line: usize,
    pub func: Option<String>,
    pub reason: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RawEvent {
    pub kind: String,
    pub event: Option<String>,
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub def: Option<String>,
    pub line: usize,
    pub via: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SymbolNode {
    pub id: String,
    pub module: String,
    pub qual: String,
    pub name: String,
    pub kind: String,
    pub line: usize,
    pub end_line: usize,
    pub decorators: Vec<String>,
    pub params: Option<Vec<RawParamSig>>,
    pub returns: Option<String>,
    pub doc: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ModuleNode {
    pub id: String,
    pub path: String,
    pub is_package: bool,
    pub is_test: bool,
    pub is_entry: bool,
    pub generated: Option<serde_json::Value>,
    pub lines: usize,
    pub error: Option<String>,
    pub symbols: Vec<SymbolNode>,
    pub routes: Vec<RawRoute>,
    pub http: Vec<RawHttp>,
    pub env: Vec<RawEnv>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImportEdge {
    pub from: String,
    pub to: String,
    pub line: usize,
    pub cyclic: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CallEdge {
    pub from: String,
    pub to: String,
    pub line: usize,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct External {
    pub name: String,
    pub imported_by: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EventUse {
    pub kind: String,
    pub event: Option<String>,
    pub func: String,
    pub target: Option<String>,
    pub module: String,
    pub line: usize,
    pub via: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HttpUse {
    #[serde(flatten)]
    pub raw: RawHttp,
    pub module: String,
    pub host: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RouteUse {
    #[serde(flatten)]
    pub raw: RawRoute,
    pub module: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Unreachable {
    #[serde(flatten)]
    pub raw: RawUnreachable,
    pub module: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EnvUse {
    pub module: String,
    pub line: usize,
    pub func: Option<String>,
    pub default: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EnvName {
    pub name: String,
    pub uses: Vec<EnvUse>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ScanError {
    pub module: String,
    pub path: String,
    pub error: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProjectGraph {
    pub modules: Vec<ModuleNode>,
    pub imports: Vec<ImportEdge>,
    pub externals: Vec<External>,
    pub calls: Vec<CallEdge>,
    pub cycles: Vec<Vec<String>>,
    pub unused: Vec<SymbolNode>,
    pub unreachable: Vec<Unreachable>,
    pub routes: Vec<RouteUse>,
    pub http: Vec<HttpUse>,
    pub events: Vec<EventUse>,
    pub env: Vec<EnvName>,
    pub errors: Vec<ScanError>,
    pub truncated: bool,
}

#[derive(Clone)]
enum Binding {
    Module(String),
    Symbol(String),
}

fn absolute(file: &RawFile, import: &RawImport) -> String {
    if import.level == 0 {
        return import.module.clone();
    }
    let mut parts: Vec<_> = file.module.split('.').collect();
    if !file.is_package {
        parts.pop();
    }
    parts.truncate(parts.len().saturating_sub(import.level - 1));
    if !import.module.is_empty() {
        parts.push(&import.module);
    }
    parts.join(".")
}

struct Resolver<'a> {
    files: HashMap<&'a str, &'a RawFile>,
    symbols: HashMap<String, &'a SymbolNode>,
}

impl<'a> Resolver<'a> {
    fn symbol(&self, module: &str, qual: &str) -> Option<&'a SymbolNode> {
        self.symbols.get(&format!("{module}:{qual}")).copied()
    }
    fn longest_module(&self, dotted: &str) -> Option<String> {
        let mut candidate = dotted;
        loop {
            if self.files.contains_key(candidate) {
                return Some(candidate.into());
            }
            candidate = candidate.rsplit_once('.')?.0;
        }
    }
    fn call(
        &self,
        name: &str,
        file: &RawFile,
        def: &RawDef,
        bindings: &HashMap<String, Binding>,
    ) -> Option<&'a SymbolNode> {
        let parts: Vec<_> = name.split('.').collect();
        if parts.len() == 2 {
            if let Some(instance) = parts[0].strip_suffix("()").filter(|s| !s.contains('(')) {
                let class = self.call(instance, file, def, bindings)?;
                return (class.kind == "class")
                    .then(|| self.symbol(&class.module, &format!("{}.{}", class.qual, parts[1])))
                    .flatten();
            }
        }
        if parts.iter().any(|p| p.contains('(')) {
            return None;
        }
        if parts.len() == 1 {
            if def.kind != "module" {
                if let Some(nested) = self.symbol(&file.module, &format!("{}.{name}", def.qual)) {
                    return Some(nested);
                }
            }
            return self.symbol(&file.module, name).or_else(|| match bindings.get(name) {
                Some(Binding::Symbol(id)) => self.symbols.get(id).copied(),
                _ => None,
            });
        }
        if matches!(parts[0], "self" | "cls") && parts.len() == 2 && def.kind == "method" {
            let class = def.qual.rsplit_once('.')?.0;
            return self.symbol(&file.module, &format!("{class}.{}", parts[1]));
        }
        let head = bindings.get(parts[0]);
        let class = self.symbol(&file.module, parts[0]).or_else(|| match head {
            Some(Binding::Symbol(id)) => self.symbols.get(id).copied(),
            _ => None,
        });
        if let Some(class) = class.filter(|c| c.kind == "class" && parts.len() == 2) {
            return self.symbol(&class.module, &format!("{}.{}", class.qual, parts[1]));
        }
        let Some(Binding::Module(module)) = head else {
            return None;
        };
        let mut target = module.clone();
        let mut i = 1;
        while i + 1 < parts.len() && self.files.contains_key(format!("{target}.{}", parts[i]).as_str()) {
            target.push('.');
            target.push_str(parts[i]);
            i += 1;
        }
        self.symbol(&target, &parts[i..].join("."))
    }
}

pub fn build_graph(scan: &RawScan) -> ProjectGraph {
    let modules: Vec<_> = scan
        .files
        .iter()
        .map(|f| ModuleNode {
            id: f.module.clone(),
            path: f.path.clone(),
            is_package: f.is_package,
            is_test: is_test_path(&f.path),
            is_entry: f.main,
            generated: f.generated.clone(),
            lines: f.lines,
            error: f.error.clone(),
            symbols: f
                .defs
                .iter()
                .filter(|d| d.kind != "module")
                .map(|d| SymbolNode {
                    id: format!("{}:{}", f.module, d.qual),
                    module: f.module.clone(),
                    qual: d.qual.clone(),
                    name: d.name.clone(),
                    kind: d.kind.clone(),
                    line: d.line,
                    end_line: d.end_line,
                    decorators: d.decorators.clone(),
                    params: d.params.clone(),
                    returns: d.returns.clone(),
                    doc: d.params.as_ref().map(|_| d.doc.clone().unwrap_or_default()),
                })
                .collect(),
            routes: f.routes.clone(),
            http: f.http.clone(),
            env: f.env.clone(),
        })
        .collect();
    let resolver = Resolver {
        files: scan.files.iter().map(|f| (f.module.as_str(), f)).collect(),
        symbols: modules.iter().flat_map(|m| m.symbols.iter().map(|s| (s.id.clone(), s))).collect(),
    };
    let mut imports = Vec::new();
    let mut import_seen = HashSet::new();
    let mut externals: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut bindings = HashMap::new();
    for file in &scan.files {
        let mut bound = HashMap::new();
        let mut edge = |target: &str, line| {
            if target != file.module && import_seen.insert((file.module.clone(), target.to_owned())) {
                imports.push(ImportEdge { from: file.module.clone(), to: target.into(), line, cyclic: false });
            }
        };
        let mut external = |name: &str| {
            if let Some(top) = name.split('.').next().filter(|s| !s.is_empty()) {
                externals.entry(top.into()).or_default().insert(file.module.clone());
            }
        };
        for import in &file.imports {
            let base = absolute(file, import);
            if import.names.is_empty() {
                let target = resolver.longest_module(&base);
                match &target {
                    Some(target) => edge(target, import.line),
                    None => external(&base),
                }
                if let Some(alias) = &import.alias {
                    if target.as_ref() == Some(&base) {
                        bound.insert(alias.clone(), Binding::Module(base));
                    }
                } else if let Some(top) = base.split('.').next().filter(|t| resolver.files.contains_key(*t)) {
                    bound.insert(top.into(), Binding::Module(top.into()));
                }
                continue;
            }
            let mut base_used = false;
            for name in &import.names {
                if name == "*" {
                    base_used = true;
                    continue;
                }
                let sub = if base.is_empty() { name.clone() } else { format!("{base}.{name}") };
                if resolver.files.contains_key(sub.as_str()) {
                    edge(&sub, import.line);
                    bound.insert(name.clone(), Binding::Module(sub));
                } else {
                    base_used = true;
                    if let Some(symbol) = resolver.symbol(&base, name) {
                        bound.insert(name.clone(), Binding::Symbol(symbol.id.clone()));
                    }
                }
            }
            if base_used {
                if let Some(target) = resolver.longest_module(&base) {
                    edge(&target, import.line);
                } else if import.level == 0 {
                    external(&base);
                }
            }
        }
        bindings.insert(file.module.as_str(), bound);
    }
    let ids: Vec<_> = modules.iter().map(|m| m.id.clone()).collect();
    let cycles: Vec<_> = strongly_connected(&ids, &imports).into_iter().filter(|c| c.len() > 1).collect();
    let in_cycle: HashMap<_, _> =
        cycles.iter().enumerate().flat_map(|(i, c)| c.iter().map(move |id| (id.as_str(), i))).collect();
    for edge in &mut imports {
        edge.cyclic = in_cycle.get(edge.from.as_str()).is_some_and(|i| in_cycle.get(edge.to.as_str()) == Some(i));
    }
    let mut calls = Vec::new();
    let mut seen_calls = HashSet::new();
    let mut events = Vec::new();
    let mut seen_events = HashSet::new();
    let mut referenced_by: HashMap<&str, HashSet<String>> = HashMap::new();
    for file in &scan.files {
        let bound = &bindings[file.module.as_str()];
        for def in &file.defs {
            let from = if def.kind == "module" {
                format!("module:{}", file.module)
            } else {
                format!("{}:{}", file.module, def.qual)
            };
            for call in &def.calls {
                if let Some(target) = resolver.call(&call.name, file, def, bound) {
                    if target.id != from && seen_calls.insert((from.clone(), target.id.clone())) {
                        calls.push(CallEdge { from: from.clone(), to: target.id.clone(), line: call.line });
                    }
                }
            }
            for reference in &def.refs {
                referenced_by.entry(reference).or_default().insert(format!("{}:{}", file.module, def.qual));
            }
        }
        for event in &file.events {
            let ctx = event
                .def
                .as_ref()
                .and_then(|q| file.defs.iter().find(|d| &d.qual == q))
                .or_else(|| file.defs.iter().find(|d| d.kind == "module"))
                .or(file.defs.first());
            let here = ctx
                .filter(|d| d.kind != "module")
                .map(|d| format!("{}:{}", file.module, d.qual))
                .unwrap_or_else(|| format!("module:{}", file.module));
            let resolved = event.reference.as_ref().and_then(|name| {
                resolver.symbol(&file.module, name).or_else(|| ctx.and_then(|d| resolver.call(name, file, d, bound)))
            });
            if event.kind != "emits" && resolved.is_none_or(|r| r.kind == "class") {
                continue;
            }
            let function =
                if event.kind == "handles" { resolved.map(|s| s.id.clone()).unwrap_or_default() } else { here };
            let target = (event.kind == "spawns").then(|| resolved.map(|s| s.id.clone())).flatten();
            let event_name = if event.kind == "spawns" { None } else { event.event.clone() };
            if seen_events.insert((event.kind.clone(), event_name.clone(), function.clone(), target.clone())) {
                events.push(EventUse {
                    kind: event.kind.clone(),
                    event: event_name,
                    func: function,
                    target,
                    module: file.module.clone(),
                    line: event.line,
                    via: event.via.clone(),
                });
            }
        }
    }
    const HOOKS: &[&str] = &[
        "setUp",
        "tearDown",
        "setUpClass",
        "tearDownClass",
        "run",
        "main",
        "handle",
        "get",
        "post",
        "put",
        "patch",
        "delete",
        "dispatch",
        "save",
        "clean",
        "validate",
    ];
    let mut unused = Vec::new();
    for module in &modules {
        if module.is_test {
            continue;
        }
        let file = resolver.files[module.id.as_str()];
        for symbol in &module.symbols {
            if symbol.name.starts_with("__")
                || symbol.name.starts_with("test")
                || symbol.name == "main"
                || file.all.contains(&symbol.name)
                || module.routes.iter().any(|r| r.func == symbol.qual)
                || symbol.decorators.iter().any(|d| !matches!(d.as_str(), "staticmethod" | "classmethod"))
            {
                continue;
            }
            if symbol.kind == "method" {
                let class = symbol.qual.rsplit_once('.').map_or("", |(c, _)| c);
                if HOOKS.contains(&symbol.name.as_str())
                    || file.defs.iter().any(|d| d.qual == class && !d.bases.is_empty())
                {
                    continue;
                }
            }
            if !referenced_by.get(symbol.name.as_str()).is_some_and(|users| {
                users.iter().any(|u| u != &symbol.id && !u.starts_with(&format!("{}.", symbol.id)))
            }) {
                unused.push(symbol.clone());
            }
        }
    }
    let mut unreachable: Vec<_> = scan
        .files
        .iter()
        .flat_map(|f| f.unreachable.iter().map(|u| Unreachable { raw: u.clone(), module: f.module.clone() }))
        .collect();
    unreachable.sort_by(|a, b| a.module.cmp(&b.module).then_with(|| a.raw.line.cmp(&b.raw.line)));
    let mut env_map: BTreeMap<String, Vec<EnvUse>> = BTreeMap::new();
    for module in &modules {
        for env in &module.env {
            env_map.entry(env.name.clone()).or_default().push(EnvUse {
                module: module.id.clone(),
                line: env.line,
                func: env.func.clone(),
                default: env.default,
            });
        }
    }
    let routes = modules
        .iter()
        .flat_map(|m| m.routes.iter().map(|r| RouteUse { raw: r.clone(), module: m.id.clone() }))
        .collect();
    let http = modules
        .iter()
        .flat_map(|m| {
            m.http.iter().map(|h| HttpUse { raw: h.clone(), module: m.id.clone(), host: host_of(h.url.as_deref()) })
        })
        .collect();
    let errors = modules
        .iter()
        .filter_map(|m| {
            m.error.as_ref().map(|error| ScanError { module: m.id.clone(), path: m.path.clone(), error: error.clone() })
        })
        .collect();
    ProjectGraph {
        modules,
        imports,
        externals: externals
            .into_iter()
            .map(|(name, imported_by)| External { name, imported_by: imported_by.into_iter().collect() })
            .collect(),
        calls,
        cycles,
        unused,
        unreachable,
        routes,
        http,
        events,
        env: env_map.into_iter().map(|(name, uses)| EnvName { name, uses }).collect(),
        errors,
        truncated: scan.truncated,
    }
}

pub fn keep_last_good(mut next: RawScan, prev: Option<&RawScan>) -> RawScan {
    let Some(prev) = prev else {
        return next;
    };
    let before: HashMap<_, _> = prev.files.iter().map(|f| (f.path.as_str(), f)).collect();
    for file in &mut next.files {
        if file.error.as_ref().is_some_and(|e| e.contains("SyntaxError")) {
            if let Some(old) = before.get(file.path.as_str()).filter(|f| f.error.is_none()) {
                let mut kept = (*old).clone();
                kept.error = file.error.clone();
                if file.lines != 0 {
                    kept.lines = file.lines;
                }
                *file = kept;
            }
        }
    }
    next
}
pub fn replace_files(scan: &RawScan, files: Vec<RawFile>) -> RawScan {
    let mut fresh: HashMap<_, _> = files.iter().map(|f| (f.path.as_str(), f)).collect();
    let mut next = scan.clone();
    for file in &mut next.files {
        if let Some(new) = fresh.remove(file.path.as_str()) {
            *file = new.clone();
        }
    }
    next.files.extend(files.iter().filter(|f| fresh.contains_key(f.path.as_str())).cloned());
    next
}
pub fn strongly_connected(nodes: &[String], edges: &[ImportEdge]) -> Vec<Vec<String>> {
    let ids: HashMap<_, _> = nodes.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
    let count = nodes.len();
    let mut out = vec![Vec::new(); count];
    for edge in edges {
        if let (Some(&a), Some(&b)) = (ids.get(edge.from.as_str()), ids.get(edge.to.as_str())) {
            out[a].push(b);
        }
    }
    let mut index = vec![None; count];
    let mut low = vec![0; count];
    let mut on_stack = vec![false; count];
    let mut stack = Vec::new();
    let mut work = Vec::new();
    let mut next = 0;
    let mut result = Vec::new();
    for start in 0..count {
        if index[start].is_some() {
            continue;
        }
        index[start] = Some(next);
        low[start] = next;
        next += 1;
        on_stack[start] = true;
        stack.push(start);
        work.push((start, 0));
        while let Some(&(node, cursor)) = work.last() {
            if cursor < out[node].len() {
                if let Some(top) = work.last_mut() {
                    top.1 += 1;
                }
                let target = out[node][cursor];
                if let Some(i) = index[target] {
                    if on_stack[target] {
                        low[node] = low[node].min(i);
                    }
                } else {
                    index[target] = Some(next);
                    low[target] = next;
                    next += 1;
                    stack.push(target);
                    on_stack[target] = true;
                    work.push((target, 0));
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[node]);
            }
            if Some(low[node]) == index[node] {
                let mut component = Vec::new();
                while let Some(member) = stack.pop() {
                    on_stack[member] = false;
                    component.push(nodes[member].clone());
                    if member == node {
                        break;
                    }
                }
                component.sort();
                result.push(component);
            }
        }
    }
    result
}
pub fn host_of(url: Option<&str>) -> Option<String> {
    let (scheme, rest) = url?.split_once("://")?;
    if !scheme.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        || !scheme.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-'))
    {
        return None;
    }
    let host = rest.split(['/', '?', '#', '{', '…']).next()?.rsplit('@').next()?;
    (!host.is_empty()).then(|| host.to_lowercase())
}
pub fn is_test_path(path: &str) -> bool {
    let parts: Vec<_> = path.split('/').collect();
    parts.iter().take(parts.len().saturating_sub(1)).any(|p| matches!(*p, "test" | "tests" | "testing"))
        || parts
            .last()
            .and_then(|p| p.strip_suffix(".py"))
            .is_some_and(|p| p.starts_with("test_") || p.ends_with("_test") || p == "conftest")
}

pub fn describe_unreachable(raw: &RawUnreachable) -> String {
    let location = if raw.line == raw.end_line {
        format!("line {}", raw.line)
    } else {
        format!("lines {}–{}", raw.line, raw.end_line)
    };
    let reason = match raw.reason.as_str() {
        "return" => "after a return",
        "raise" => "after a raise",
        "break" => "after a break",
        "continue" => "after a continue",
        "branches" => "every branch above it stops",
        "constant" => "its condition is always false",
        _ => "unreachable",
    };
    format!("{location}, {reason}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graph_matches_the_electron_view_model_fixture() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/project_graph.json")).unwrap();
        let scan: RawScan = serde_json::from_value(fixture["scan"].clone()).unwrap();
        let expected: ProjectGraph = serde_json::from_value(fixture["expected"].clone()).unwrap();
        assert_eq!(build_graph(&scan), expected);
    }
    #[test]
    fn syntax_errors_keep_last_good_definitions_and_other_errors_do_not() {
        let old = RawFile {
            path: "a.py".into(),
            defs: vec![RawDef { name: "total".into(), ..Default::default() }],
            ..Default::default()
        };
        let prev = RawScan { files: vec![old.clone()], ..Default::default() };
        let mut bad = RawFile {
            path: "a.py".into(),
            error: Some("SyntaxError: invalid syntax (line 2)".into()),
            ..Default::default()
        };
        let next = RawScan { files: vec![bad.clone()], ..Default::default() };
        assert_eq!(keep_last_good(next, Some(&prev)).files[0].defs, old.defs);
        bad.error = Some("too large to analyse".into());
        assert!(keep_last_good(RawScan { files: vec![bad], ..Default::default() }, Some(&prev)).files[0]
            .defs
            .is_empty());
    }
    #[test]
    fn rescanned_files_replace_existing_and_add_new_paths() {
        let old = RawFile { path: "a.py".into(), ..Default::default() };
        let scan = RawScan {
            files: vec![old.clone(), RawFile { path: "b.py".into(), ..Default::default() }],
            ..Default::default()
        };
        let fresh = RawFile { lines: 42, ..old };
        let result = replace_files(&scan, vec![fresh, RawFile { path: "c.py".into(), ..Default::default() }]);
        assert_eq!(result.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.py", "b.py", "c.py"]);
        assert_eq!(result.files[0].lines, 42);
    }
    #[test]
    fn host_and_test_path_helpers_match_electron() {
        assert_eq!(host_of(Some("https://api.github.com/users/{…}")), Some("api.github.com".into()));
        assert_eq!(host_of(Some("http://user:pw@Example.ORG:8080/x")), Some("example.org:8080".into()));
        assert_eq!(host_of(Some("/relative")), None);
        assert_eq!(host_of(None), None);
        for path in ["tests/test_a.py", "pkg/a_test.py", "conftest.py"] {
            assert!(is_test_path(path));
        }
        assert!(!is_test_path("pkg/testing_utils.py"));
    }
    #[test]
    fn strongly_connected_handles_a_twenty_thousand_module_loop() {
        let nodes: Vec<_> = (0..20_000).map(|i| format!("m{i}")).collect();
        let edges: Vec<_> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| ImportEdge {
                from: n.clone(),
                to: nodes[(i + 1) % nodes.len()].clone(),
                ..Default::default()
            })
            .collect();
        assert_eq!(strongly_connected(&nodes, &edges)[0].len(), 20_000);
    }
}
