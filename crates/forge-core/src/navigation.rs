//! Project treemap and file context, ported from treemap.ts and whereModel.ts.
use crate::project_graph::ProjectGraph;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}
pub struct MapLayout {
    pub groups: BTreeMap<String, Rect>,
    pub files: BTreeMap<String, Rect>,
}
fn worst(row: &[(String, f64)], side: f64) -> f64 {
    let sum = row.iter().map(|(_, a)| a).sum::<f64>();
    let max = row.iter().map(|(_, a)| *a).fold(0., f64::max);
    let min = row.iter().map(|(_, a)| *a).fold(f64::MAX, f64::min);
    (side * side * max / (sum * sum)).max(sum * sum / (side * side * min))
}
fn place_row(row: &[(String, f64)], r: &mut Rect, out: &mut BTreeMap<String, Rect>) {
    let sum = row.iter().map(|(_, a)| a).sum::<f64>();
    if r.w >= r.h {
        let w = sum / r.h;
        let mut y = r.y;
        for (id, a) in row {
            let h = a / w;
            out.insert(id.clone(), Rect { x: r.x, y, w, h });
            y += h;
        }
        r.x += w;
        r.w = (r.w - w).max(0.);
    } else {
        let h = sum / r.w;
        let mut x = r.x;
        for (id, a) in row {
            let w = a / h;
            out.insert(id.clone(), Rect { x, y: r.y, w, h });
            x += w;
        }
        r.y += h;
        r.h = (r.h - h).max(0.);
    }
}
pub fn squarify(items: &[(String, f64)], area: Rect) -> BTreeMap<String, Rect> {
    let mut out = BTreeMap::new();
    if area.w <= 0. || area.h <= 0. || !area.w.is_finite() || !area.h.is_finite() {
        return out;
    }
    let mut list = items.iter().filter(|(_, w)| *w > 0. && w.is_finite()).cloned().collect::<Vec<_>>();
    list.sort_by(|a, b| b.1.total_cmp(&a.1));
    let total = list.iter().map(|(_, w)| w).sum::<f64>();
    if total <= 0. {
        return out;
    }
    let scale = area.w * area.h / total;
    let mut rect = area;
    let mut row = Vec::new();
    for (id, w) in list {
        let a = w * scale;
        let side = rect.w.min(rect.h);
        if !row.is_empty() {
            let old = worst(&row, side);
            row.push((id.clone(), a));
            let new = worst(&row, side);
            row.pop();
            if new > old {
                place_row(&row, &mut rect, &mut out);
                row.clear();
            }
        }
        row.push((id, a));
    }
    if !row.is_empty() {
        place_row(&row, &mut rect, &mut out);
    }
    out
}
pub fn project_map(files: &[(String, usize)], area: Rect) -> MapLayout {
    let mut by_group: BTreeMap<String, Vec<(String, f64)>> = BTreeMap::new();
    for (path, lines) in files {
        let group = path.split_once('/').map(|(g, _)| g).unwrap_or("");
        by_group.entry(group.into()).or_default().push((path.clone(), (*lines).max(1) as f64));
    }
    let groups = squarify(
        &by_group.iter().map(|(g, items)| (g.clone(), items.iter().map(|(_, w)| w).sum())).collect::<Vec<_>>(),
        area,
    );
    let mut out = BTreeMap::new();
    for (g, r) in &groups {
        let inner =
            if r.w > 8. && r.h > 22. { Rect { x: r.x + 2., y: r.y + 16., w: r.w - 4., h: r.h - 18. } } else { *r };
        out.extend(squarify(&by_group[g], inner));
    }
    MapLayout { groups, files: out }
}
pub fn module_for_path<'a>(graph: &'a ProjectGraph, path: &str) -> Option<&'a str> {
    let path = path.replace('\\', "/");
    let path = path.strip_prefix("./").unwrap_or(&path);
    graph.modules.iter().find(|m| m.path == path).map(|m| m.id.as_str())
}
pub struct Neighbourhood {
    pub imports: Vec<String>,
    pub imported_by: Vec<String>,
    pub hosts: Vec<String>,
}
pub fn neighbourhood(graph: &ProjectGraph, module: &str) -> Neighbourhood {
    let sorted = |xs: Vec<String>| xs.into_iter().collect::<BTreeSet<_>>().into_iter().collect();
    Neighbourhood {
        imports: sorted(graph.imports.iter().filter(|e| e.from == module).map(|e| e.to.clone()).collect()),
        imported_by: sorted(graph.imports.iter().filter(|e| e.to == module).map(|e| e.from.clone()).collect()),
        hosts: sorted(graph.http.iter().filter(|h| h.module == module).filter_map(|h| h.host.clone()).collect()),
    }
}
pub struct SymbolSummary {
    pub qual: String,
    pub kind: String,
    pub line: usize,
    pub summary: String,
}
fn module_of(id: &str) -> &str {
    id.strip_prefix("module:").unwrap_or_else(|| id.split_once(':').map(|(m, _)| m).unwrap_or(id))
}
pub fn file_summary(graph: &ProjectGraph, module: &str) -> Vec<SymbolSummary> {
    let Some(m) = graph.modules.iter().find(|m| m.id == module) else {
        return Vec::new();
    };
    let mut outgoing: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut incoming: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for c in &graph.calls {
        if module_of(&c.to) != module {
            outgoing.entry(&c.from).or_default().insert(module_of(&c.to));
        }
        if module_of(&c.from) != module {
            incoming.entry(&c.to).or_default().insert(module_of(&c.from));
        }
    }
    let mut hosts: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for h in &graph.http {
        if h.module == module {
            if let Some((func, host)) = h.raw.func.as_deref().zip(h.host.as_deref()) {
                hosts.entry(func).or_default().insert(host);
            }
        }
    }
    let mut symbols = m.symbols.iter().collect::<Vec<_>>();
    symbols.sort_by_key(|s| s.line);
    symbols
        .into_iter()
        .map(|s| {
            let reaches = hosts
                .get(s.qual.as_str())
                .into_iter()
                .flatten()
                .chain(outgoing.get(s.id.as_str()).into_iter().flatten())
                .copied()
                .collect::<Vec<_>>();
            let users = incoming.get(s.id.as_str()).into_iter().flatten().copied().collect::<Vec<_>>();
            let mut parts = Vec::new();
            if !reaches.is_empty() {
                parts.push(format!("→ {}", reaches.join(", ")));
            }
            if !users.is_empty() {
                parts.push(format!("used by {}", users.join(", ")));
            }
            SymbolSummary { qual: s.qual.clone(), kind: s.kind.clone(), line: s.line, summary: parts.join(" · ") }
        })
        .collect()
}
