//! Reading the language server's answers: locations, workspace edits,
//! document symbols, call hierarchy, highlights, workspace symbols; applying
//! edits to a whole file's text; and the class/method helpers used to find
//! implementations. Replaces `src/renderer/lsp/navigation.ts`.
//!
//! Positions here are the server's: `col` counts UTF-16 code units. The app
//! converts them to char columns with the line's text.

use crate::uri::uri_to_path;
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::LazyLock;

/// A position as the language server counts it (UTF-16 columns).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LspPos {
    pub line: usize,
    pub col: usize,
}

impl LspPos {
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }

    pub fn to_json(self) -> Value {
        serde_json::json!({ "line": self.line, "character": self.col })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub path: String,
    pub start: LspPos,
    pub end: LspPos,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub start: LspPos,
    pub end: LspPos,
    pub text: String,
}

/// A rename's changes: file path -> edits in that file.
pub type FileEdits = BTreeMap<String, Vec<TextEdit>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocSymbol {
    pub name: String,
    /// LSP SymbolKind (5 class, 6 method, 12 function, 13 variable, 14 constant...).
    pub kind: u32,
    pub detail: Option<String>,
    /// The whole thing (e.g. the full function).
    pub start: LspPos,
    pub end: LspPos,
    /// Just the name (where to put the cursor).
    pub name_start: LspPos,
    pub children: Vec<DocSymbol>,
}

pub fn pos(v: &Value) -> Option<LspPos> {
    Some(LspPos { line: v.get("line")?.as_u64()? as usize, col: v.get("character")?.as_u64()? as usize })
}

pub fn range(v: Option<&Value>) -> Option<(LspPos, LspPos)> {
    let v = v?;
    Some((pos(v.get("start")?)?, pos(v.get("end")?)?))
}

/// Location[] / LocationLink[] as locations, sorted by file then position.
pub fn parse_locations(res: &Value) -> Vec<Location> {
    let list: Vec<&Value> = match res {
        Value::Array(a) => a.iter().collect(),
        Value::Null => Vec::new(),
        v => vec![v],
    };
    let mut out: Vec<Location> = list
        .into_iter()
        .filter_map(|l| {
            let uri = l.get("uri").or_else(|| l.get("targetUri"))?.as_str()?;
            let (start, end) =
                range(l.get("range").or_else(|| l.get("targetSelectionRange")).or_else(|| l.get("targetRange")))?;
            Some(Location { path: uri_to_path(uri), start, end })
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path).then(a.start.cmp(&b.start)));
    out
}

/// A WorkspaceEdit (changes or documentChanges) as edits per file. File
/// create/rename/delete operations are left out.
pub fn parse_workspace_edit(res: &Value) -> FileEdits {
    let mut out = FileEdits::new();
    let mut add = |uri: Option<&Value>, edits: Option<&Value>| {
        let (Some(uri), Some(Value::Array(edits))) = (uri.and_then(Value::as_str), edits) else { return };
        let path = uri_to_path(uri);
        let list: Vec<TextEdit> = edits
            .iter()
            .filter_map(|e| {
                let (start, end) = range(e.get("range"))?;
                Some(TextEdit { start, end, text: e.get("newText")?.as_str()?.to_owned() })
            })
            .collect();
        if !list.is_empty() {
            out.entry(path).or_default().extend(list);
        }
    };
    if let Some(Value::Array(dcs)) = res.get("documentChanges") {
        for dc in dcs {
            if let Some(td) = dc.get("textDocument") {
                add(td.get("uri"), dc.get("edits"));
            }
        }
    } else if let Some(Value::Object(changes)) = res.get("changes") {
        for (uri, edits) in changes {
            add(Some(&Value::String(uri.clone())), Some(edits));
        }
    }
    out
}

/// DocumentSymbol[] (hierarchical) or SymbolInformation[] (flat) as a symbol tree.
pub fn parse_symbols(res: &Value) -> Vec<DocSymbol> {
    fn one(s: &Value) -> Option<DocSymbol> {
        let name = s.get("name")?.as_str()?.to_owned();
        let kind = s.get("kind")?.as_u64()? as u32;
        let (start, end) = range(s.get("range")).or_else(|| range(s.get("location").and_then(|l| l.get("range"))))?;
        let name_start = range(s.get("selectionRange")).map_or(start, |r| r.0);
        let mut children: Vec<DocSymbol> =
            s.get("children").and_then(Value::as_array).map(|c| c.iter().filter_map(one).collect()).unwrap_or_default();
        children.sort_by_key(|c| c.start);
        let detail = s.get("detail").and_then(Value::as_str).filter(|d| !d.is_empty()).map(str::to_owned);
        Some(DocSymbol { name, kind, detail, start, end, name_start, children })
    }
    let mut list: Vec<DocSymbol> = res.as_array().map(|a| a.iter().filter_map(one).collect()).unwrap_or_default();
    list.sort_by_key(|c| c.start);
    list
}

/// The innermost symbol containing `p` (a path from the outermost), for the outline.
pub fn symbol_path_at(symbols: &[DocSymbol], p: LspPos) -> Vec<DocSymbol> {
    for s in symbols {
        if s.start <= p && p <= s.end {
            let mut out = vec![s.clone()];
            out.extend(symbol_path_at(&s.children, p));
            return out;
        }
    }
    Vec::new()
}

/// Apply edits to a file's whole text (for files that aren't open). Keeps the
/// file's own line endings; columns are UTF-16 units.
pub fn apply_edits(text: &str, edits: &[TextEdit]) -> String {
    // Byte offset where each line starts, and where it ends (before its line break).
    let mut starts = vec![0usize];
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            starts.push(i + 2);
            i += 2;
            continue;
        }
        if bytes[i] == b'\n' || bytes[i] == b'\r' {
            starts.push(i + 1);
        }
        i += 1;
    }
    let line_end = |line: usize| -> usize {
        if line + 1 >= starts.len() {
            return text.len();
        }
        let next = starts[line + 1];
        if next >= 2 && &bytes[next - 2..next] == b"\r\n" {
            next - 2
        } else {
            next - 1
        }
    };
    let offset = |p: LspPos| -> usize {
        if p.line >= starts.len() {
            return text.len();
        }
        let (from, to) = (starts[p.line], line_end(p.line));
        let mut units = 0;
        for (b, ch) in text[from..to].char_indices() {
            if units >= p.col {
                return from + b;
            }
            units += ch.len_utf16();
        }
        to
    };
    let mut sorted: Vec<(usize, usize, &str)> =
        edits.iter().map(|e| (offset(e.start), offset(e.end), e.text.as_str())).collect();
    // Apply from the end so earlier offsets stay valid.
    sorted.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let mut out = text.to_owned();
    for (from, to, t) in sorted {
        out.replace_range(from..to.max(from), t);
    }
    out
}

/// Short marks for symbol kinds (shown in the outline): (icon, name).
pub fn symbol_kind(kind: u32) -> (&'static str, &'static str) {
    match kind {
        2 => ("▣", "module"),
        5 => ("C", "class"),
        6 => ("ƒ", "method"),
        7 => ("·", "property"),
        8 => ("·", "field"),
        9 => ("ƒ", "constructor"),
        12 => ("ƒ", "function"),
        13 => ("x", "variable"),
        14 => ("K", "constant"),
        _ => ("·", "symbol"),
    }
}

/// A function or method in a call hierarchy (the server's item is kept for follow-up requests).
#[derive(Clone, Debug, PartialEq)]
pub struct CallItem {
    pub name: String,
    pub kind: u32,
    pub detail: Option<String>,
    pub path: String,
    pub start: LspPos,
    pub end: LspPos,
    pub name_start: LspPos,
    pub raw: Value,
}

pub fn parse_call_item(raw: &Value) -> Option<CallItem> {
    let name = raw.get("name")?.as_str()?.to_owned();
    let uri = raw.get("uri")?.as_str()?;
    let (start, end) = range(raw.get("range"))?;
    let name_start = range(raw.get("selectionRange")).map_or(start, |r| r.0);
    Some(CallItem {
        name,
        kind: raw.get("kind").and_then(Value::as_u64).unwrap_or(12) as u32,
        detail: raw.get("detail").and_then(Value::as_str).filter(|d| !d.is_empty()).map(str::to_owned),
        path: uri_to_path(uri),
        start,
        end,
        name_start,
        raw: raw.clone(),
    })
}

/// Incoming (`from`) or outgoing (`to`) calls: the other end, with where the calls are.
pub fn parse_calls(res: &Value, side: &str) -> Vec<(CallItem, Vec<LspPos>)> {
    let mut out: Vec<(CallItem, Vec<LspPos>)> = res
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let item = parse_call_item(c.get(side)?)?;
            let at = c
                .get("fromRanges")
                .and_then(Value::as_array)
                .map(|rs| rs.iter().filter_map(|r| range(Some(r)).map(|x| x.0)).collect())
                .unwrap_or_default();
            Some((item, at))
        })
        .collect();
    out.sort_by(|a, b| a.0.path.cmp(&b.0.path).then(a.0.name_start.line.cmp(&b.0.name_start.line)));
    out
}

/// documentHighlight as ranges (kind 3 = write).
pub fn parse_highlights(res: &Value) -> Vec<(LspPos, LspPos, bool)> {
    res.as_array()
        .into_iter()
        .flatten()
        .filter_map(|h| {
            let (s, e) = range(h.get("range"))?;
            Some((s, e, h.get("kind").and_then(Value::as_u64) == Some(3)))
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceSymbol {
    pub name: String,
    pub kind: u32,
    pub container: Option<String>,
    pub location: Location,
}

/// workspace/symbol as symbols with locations.
pub fn parse_workspace_symbols(res: &Value) -> Vec<WorkspaceSymbol> {
    res.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let location = parse_locations(s.get("location")?).into_iter().next()?;
            Some(WorkspaceSymbol {
                name: s.get("name")?.as_str()?.to_owned(),
                kind: s.get("kind")?.as_u64()? as u32,
                container: s.get("containerName").and_then(Value::as_str).filter(|c| !c.is_empty()).map(str::to_owned),
                location,
            })
        })
        .collect()
}

static CLASS_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*)class\s+([A-Za-z_]\w*)\s*\(").expect("regex"));
static METACLASS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bmetaclass\s*=\s*$").expect("regex"));

/// If the name at `col` (chars) is one of the bases in a `class Name(Base, ...)`
/// line: the new class's name and its column. (For finding subclasses.)
pub fn subclass_at(line: &str, col: usize) -> Option<(String, usize)> {
    let m = CLASS_HEAD.captures(line)?;
    let chars: Vec<char> = line.chars().collect();
    let open = m[0].chars().count() - 1;
    // The bases end at the matching ")" (they can contain calls like Generic[T]).
    let mut depth = 0i32;
    let mut close = None;
    for (i, c) in chars.iter().enumerate().skip(open) {
        if "([{".contains(*c) {
            depth += 1;
        } else if ")]}".contains(*c) {
            depth -= 1;
            if depth == 0 {
                close = Some(i);
                break;
            }
        }
    }
    if col <= open || close.is_some_and(|c| col >= c) {
        return None;
    }
    // metaclass=Base is not inheritance.
    let between: String = chars[open + 1..col.min(chars.len())].iter().collect();
    if METACLASS.is_match(&between) {
        return None;
    }
    let indent = m[1].chars().count();
    let name = m[2].to_owned();
    let from: String = chars[indent + 5..].iter().collect();
    let at = from.find(&name).map(|b| indent + 5 + from[..b].chars().count())?;
    Some((name, at))
}

/// The `def method` directly inside the class defined on `class_line`: (line, col).
pub fn method_in_class(lines: &[&str], class_line: usize, method: &str) -> Option<(usize, usize)> {
    let indent_of = |s: &str| s.len() - s.trim_start().len();
    let class_indent = indent_of(lines.get(class_line)?);
    let clean: String = method.chars().filter(|c| c.is_alphanumeric() || *c == '_').collect();
    let def = Regex::new(&format!(r"^(\s*)(?:async\s+)?def\s+({clean})\s*\(")).ok()?;
    let mut body_indent = None;
    for (i, l) in lines.iter().enumerate().skip(class_line + 1) {
        let t = l.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let ind = indent_of(l);
        if ind <= class_indent {
            break; // end of the class
        }
        let body = *body_indent.get_or_insert(ind);
        if ind != body {
            continue; // inside a method
        }
        if let Some(m) = def.captures(l) {
            let lead = m[1].len();
            let b = l[lead..].find(&m[2]).map(|x| x + lead)?;
            return Some((i, l[..b].chars().count()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn r(l1: usize, c1: usize, l2: usize, c2: usize) -> Value {
        json!({ "start": { "line": l1, "character": c1 }, "end": { "line": l2, "character": c2 } })
    }

    #[test]
    fn reads_locations_and_links_sorted() {
        let locs = parse_locations(&json!([
            { "uri": "file:///p/b.py", "range": r(3, 0, 3, 4) },
            { "targetUri": "file:///p/a.py", "targetSelectionRange": r(1, 2, 1, 5), "targetRange": r(0, 0, 9, 0) },
            { "uri": "file:///p/b.py", "range": r(1, 0, 1, 4) },
            { "nonsense": true }
        ]));
        let got: Vec<String> = locs.iter().map(|l| format!("{}:{}:{}", l.path, l.start.line, l.start.col)).collect();
        assert_eq!(got, ["/p/a.py:1:2", "/p/b.py:1:0", "/p/b.py:3:0"]);
        assert!(parse_locations(&Value::Null).is_empty());
    }

    #[test]
    fn reads_workspace_edits() {
        let e = parse_workspace_edit(&json!({ "documentChanges": [
            { "textDocument": { "uri": "file:///p/a.py", "version": 3 }, "edits": [{ "range": r(0, 4, 0, 7), "newText": "bar" }] },
            { "kind": "rename", "oldUri": "file:///p/x.py", "newUri": "file:///p/y.py" }
        ] }));
        assert_eq!(
            e,
            FileEdits::from([(
                "/p/a.py".to_owned(),
                vec![TextEdit { start: LspPos::new(0, 4), end: LspPos::new(0, 7), text: "bar".into() }]
            )])
        );
        let c = parse_workspace_edit(
            &json!({ "changes": { "file:///p/a.py": [{ "range": r(1, 0, 1, 1), "newText": "z" }], "file:///p/b.py": [] } }),
        );
        assert_eq!(c.keys().collect::<Vec<_>>(), ["/p/a.py"]);
    }

    #[test]
    fn reads_a_symbol_tree_sorted_by_position() {
        let syms = parse_symbols(&json!([
            { "name": "helper", "kind": 12, "range": r(10, 0, 12, 0), "selectionRange": r(10, 4, 10, 10), "children": [] },
            { "name": "Dog", "kind": 5, "range": r(0, 0, 8, 0), "selectionRange": r(0, 6, 0, 9), "children": [
                { "name": "bark", "kind": 6, "detail": "(self) -> None", "range": r(4, 4, 6, 0), "selectionRange": r(4, 8, 4, 12) },
                { "name": "__init__", "kind": 6, "range": r(1, 4, 3, 0), "selectionRange": r(1, 8, 1, 16) }
            ] }
        ]));
        assert_eq!(syms.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Dog", "helper"]);
        assert_eq!(syms[0].children.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["__init__", "bark"]);
        assert_eq!(syms[0].children[1].detail.as_deref(), Some("(self) -> None"));
        assert_eq!(syms[0].children[1].name_start, LspPos::new(4, 8));
        assert_eq!(
            symbol_path_at(&syms, LspPos::new(5, 2)).iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["Dog", "bark"]
        );
        assert!(symbol_path_at(&syms, LspPos::new(9, 0)).is_empty());
        let flat = parse_symbols(
            &json!([{ "name": "x", "kind": 13, "location": { "uri": "file:///p/a.py", "range": r(2, 0, 2, 5) } }]),
        );
        assert_eq!((flat[0].name.as_str(), flat[0].kind, flat[0].start), ("x", 13, LspPos::new(2, 0)));
    }

    #[test]
    fn applies_edits_keeping_line_endings() {
        let text = "def foo():\r\n    return foo\r\nfoo()\r\n";
        let e =
            |l1, c1, l2, c2, t: &str| TextEdit { start: LspPos::new(l1, c1), end: LspPos::new(l2, c2), text: t.into() };
        let out = apply_edits(text, &[e(0, 4, 0, 7, "bar"), e(1, 11, 1, 14, "bar"), e(2, 0, 2, 3, "bar")]);
        assert_eq!(out, "def bar():\r\n    return bar\r\nbar()\r\n");
        assert_eq!(apply_edits("a\nb\nc", &[e(0, 1, 2, 0, "-")]), "a-c");
        assert_eq!(apply_edits("ab", &[e(0, 99, 0, 99, "!")]), "ab!");
        assert_eq!(apply_edits("ab", &[e(5, 0, 5, 0, "\nc")]), "ab\nc");
        // Columns are UTF-16 units: "☕" is one, "𝔸" is two.
        assert_eq!(apply_edits("𝔸x", &[e(0, 2, 0, 3, "y")]), "𝔸y");
    }

    #[test]
    fn reads_calls_highlights_and_workspace_symbols() {
        let calls = parse_calls(
            &json!([
                { "from": { "name": "main", "kind": 12, "uri": "file:///p/app.py", "range": r(10, 0, 20, 0), "selectionRange": r(10, 4, 10, 8) }, "fromRanges": [r(12, 4, 12, 9), r(15, 4, 15, 9)] },
                { "from": { "nonsense": 1 }, "fromRanges": [] }
            ]),
            "from",
        );
        assert_eq!(calls.len(), 1);
        assert_eq!(
            (calls[0].0.name.as_str(), calls[0].0.path.as_str(), calls[0].0.name_start),
            ("main", "/p/app.py", LspPos::new(10, 4))
        );
        assert_eq!(calls[0].1, vec![LspPos::new(12, 4), LspPos::new(15, 4)]);
        let h =
            parse_highlights(&json!([{ "range": r(1, 0, 1, 3), "kind": 3 }, { "range": r(4, 2, 4, 5), "kind": 2 }]));
        assert_eq!(h.iter().map(|x| x.2).collect::<Vec<_>>(), [true, false]);
        let ws = parse_workspace_symbols(
            &json!([{ "name": "Dog", "kind": 5, "containerName": "animals", "location": { "uri": "file:///p/a.py", "range": r(0, 6, 0, 9) } }]),
        );
        assert_eq!(
            (ws[0].name.as_str(), ws[0].container.as_deref(), ws[0].location.path.as_str()),
            ("Dog", Some("animals"), "/p/a.py")
        );
    }

    #[test]
    fn finds_subclasses_and_methods() {
        let line = "class Dog(Animal, Generic[T], metaclass=Meta):";
        assert_eq!(subclass_at(line, line.find("Animal").expect("x")), Some(("Dog".into(), 6)));
        assert_eq!(subclass_at(line, line.find("Meta").expect("x")), None);
        assert_eq!(subclass_at("x = Animal()", 4), None);
        assert_eq!(subclass_at("class Animal:", 6), None);
        assert_eq!(subclass_at("    class Inner(Base): pass", 16), Some(("Inner".into(), 10)));
        let lines = [
            "class Dog(Animal):",
            "    \"\"\"Docs.\"\"\"",
            "    def helper(self):",
            "        def speak(): pass",
            "    async def speak(self):",
            "        pass",
            "def speak(): pass",
        ];
        assert_eq!(method_in_class(&lines, 0, "speak"), Some((4, 14)));
        assert_eq!(method_in_class(&lines, 0, "missing"), None);
    }
}
