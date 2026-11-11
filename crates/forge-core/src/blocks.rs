//! Ordinary Python pipeline blocks, with the original bundled library unchanged.
use crate::{dataflow::Param, project_graph::ProjectGraph};
pub const BLOCKS_PY: &str = include_str!("../../../python/forge_blocks.py");
#[derive(Clone, Debug)]
pub struct BlockInfo {
    pub name: String,
    pub kind: String,
    pub doc: String,
    pub params: Vec<Param>,
    pub returns: Option<String>,
    pub module: String,
    pub path: Option<String>,
    pub line: usize,
}
pub fn kind_of(name: &str) -> &'static str {
    if name == "select_columns" {
        return "select";
    }
    if name == "retry" {
        return "retry";
    }
    for (kind, parts) in [
        ("read", vec!["read", "load", "open", "fetch_file"]),
        ("api", vec!["api", "fetch", "request", "download"]),
        ("filter", vec!["filter", "keep", "drop", "remove", "where"]),
        ("group", vec!["group", "aggregate", "total", "summar"]),
        ("merge", vec!["merge", "join", "combine"]),
        ("save", vec!["save", "write", "export", "store"]),
    ] {
        if parts.iter().any(|p| {
            if matches!(kind, "read" | "filter" | "save") {
                name.starts_with(p)
            } else {
                name.contains(p)
            }
        }) {
            return kind;
        }
    }
    "transform"
}
fn split_top(text: &str, sep: char) -> Vec<&str> {
    let (mut depth, mut quote, mut escaped, mut start) = (0, None, false, 0);
    let mut out = Vec::new();
    for (i, c) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if let Some(q) = quote {
            if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
        } else if "([{".contains(c) {
            depth += 1;
        } else if ")]}".contains(c) {
            depth -= 1;
        } else if c == sep && depth == 0 {
            out.push(&text[start..i]);
            start = i + c.len_utf8();
        }
    }
    out.push(&text[start..]);
    out
}
pub fn parse_params(text: &str) -> Vec<Param> {
    split_top(text, ',')
        .into_iter()
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            let eq = split_top(p, '=');
            let colon = split_top(eq[0], ':');
            Param {
                name: colon[0].trim().into(),
                annotation: (colon.len() > 1).then(|| colon[1..].join(":").trim().into()),
                default: (eq.len() > 1).then(|| eq[1..].join("=").trim().into()),
            }
        })
        .collect()
}
pub fn builtin_blocks() -> Vec<BlockInfo> {
    let re = regex::Regex::new(r#"(?m)^@block\ndef (\w+)\(([^\n]*)\)(?: -> ([^\n]+))?:\n    """([^\n]*)""""#).unwrap();
    re.captures_iter(BLOCKS_PY)
        .map(|c| BlockInfo {
            name: c[1].into(),
            kind: kind_of(&c[1]).into(),
            params: parse_params(&c[2]),
            returns: c.get(3).map(|s| s.as_str().trim().into()),
            doc: c[4].trim().into(),
            module: "forge_blocks".into(),
            path: None,
            line: 0,
        })
        .collect()
}
pub fn builtin_source(name: &str) -> Option<String> {
    let start = BLOCKS_PY.find(&format!("@block\ndef {name}("))?;
    let rest = &BLOCKS_PY[start..];
    let end = rest.find("\n\n\n").unwrap_or(rest.len());
    Some(format!("{}\n", rest[..end].trim_end()))
}
pub fn project_blocks(graph: &ProjectGraph) -> Vec<BlockInfo> {
    let mut out = Vec::new();
    for m in &graph.modules {
        if m.id == "forge_blocks" || m.id.ends_with(".forge_blocks") {
            continue;
        }
        for s in &m.symbols {
            if s.kind != "function"
                || s.qual.contains('.')
                || (!s.decorators.iter().any(|d| d == "block" || d.ends_with(".block"))
                    && !((m.id == "blocks" || m.id.ends_with(".blocks")) && !s.name.starts_with('_')))
            {
                continue;
            }
            out.push(BlockInfo {
                name: s.name.clone(),
                kind: kind_of(&s.name).into(),
                doc: s.doc.clone().unwrap_or_default(),
                params: s
                    .params
                    .iter()
                    .flatten()
                    .map(|p| Param {
                        name: p.name.clone(),
                        annotation: p.annotation.clone(),
                        default: p.default.clone(),
                    })
                    .collect(),
                returns: s.returns.clone(),
                module: m.id.clone(),
                path: Some(m.path.clone()),
                line: s.line,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}
pub fn is_data_param(p: &Param, index: usize) -> bool {
    if p.name.starts_with('*') {
        return false;
    }
    p.annotation.as_ref().is_some_and(|a| {
        ["DataFrame", "Series", "list[dict", "Iterable", "Sequence"].iter().any(|k| a.contains(k))
            && !["str", "int", "float", "bool"].iter().any(|k| a.starts_with(k))
    }) || (index == 0
        && ["table", "df", "data", "frame", "records", "rows", "items", "left", "right", "values"]
            .contains(&p.name.as_str()))
}
