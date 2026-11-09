//! Autocomplete helpers: what's being typed, filtering, and what to insert;
//! hover Markdown as plain sections. Replaces
//! `src/renderer/lsp/completionLogic.ts` and `src/renderer/lsp/markdown.ts`.

use crate::navigation::{range, LspPos};
use forge_core::fuzzy::fuzzy_match;
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompletionItem {
    pub label: String,
    /// LSP CompletionItemKind (3 function, 6 variable, 7 class, 9 module, 14 keyword...).
    pub kind: Option<u32>,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    pub insert_text: Option<String>,
    pub filter_text: Option<String>,
    pub sort_text: Option<String>,
    /// The server's own edit range (UTF-16) and text.
    pub text_edit: Option<(LspPos, LspPos, String)>,
    /// The raw item, for completionItem/resolve.
    pub raw: Value,
    /// Accepting it also adds an import (Pyright's auto-import suggestions).
    pub auto_import: bool,
}

/// LSP documentation is a string or { kind, value }.
pub fn doc_text(doc: &Value) -> Option<String> {
    match doc {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => o.get("value").map(|v| v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string())),
        _ => None,
    }
}

pub fn to_completion_item(raw: &Value) -> CompletionItem {
    let s = |k: &str| raw.get(k).and_then(Value::as_str).map(str::to_owned);
    let detail = s("detail").filter(|d| !d.is_empty());
    let desc = raw.get("labelDetails").and_then(|l| l.get("description")).and_then(Value::as_str).unwrap_or("");
    let auto_import = raw.get("data").and_then(|d| d.get("autoImportText")).is_some_and(|v| !v.is_null())
        || detail.as_deref().is_some_and(|d| d.to_lowercase().contains("auto-import"))
        || desc.to_lowercase().contains("auto-import");
    let text_edit = raw.get("textEdit").and_then(|te| {
        let (start, end) = range(te.get("range").or_else(|| te.get("insert")))?;
        Some((start, end, te.get("newText")?.as_str()?.to_owned()))
    });
    CompletionItem {
        label: s("label").unwrap_or_default(),
        kind: raw.get("kind").and_then(Value::as_u64).map(|k| k as u32),
        detail,
        documentation: raw.get("documentation").and_then(doc_text),
        insert_text: s("insertText"),
        filter_text: s("filterText"),
        sort_text: s("sortText"),
        text_edit,
        raw: raw.clone(),
        auto_import,
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Column (chars) where the identifier being typed (ending at `col`) starts.
pub fn word_start_at(line: &str, col: usize) -> usize {
    let chars: Vec<char> = line.chars().collect();
    let mut i = col.min(chars.len());
    while i > 0 && is_word_char(chars[i - 1]) {
        i -= 1;
    }
    i
}

/// Items matching what was typed so far, best first (ties keep the server's order).
pub fn filter_completions(items: &[CompletionItem], prefix: &str) -> Vec<CompletionItem> {
    let by_server = |a: &CompletionItem, b: &CompletionItem| {
        a.auto_import
            .cmp(&b.auto_import)
            .then_with(|| a.sort_text.as_ref().unwrap_or(&a.label).cmp(b.sort_text.as_ref().unwrap_or(&b.label)))
    };
    if prefix.is_empty() {
        let mut v = items.to_vec();
        v.sort_by(by_server);
        return v;
    }
    let mut scored: Vec<(CompletionItem, f64)> = Vec::new();
    for item in items {
        let text = item.filter_text.as_deref().unwrap_or(&item.label);
        let Some(m) = fuzzy_match(prefix, text) else { continue };
        let mut score = m.score;
        if text == prefix {
            score += 100.0;
        } else if text.starts_with(prefix) {
            score += 30.0; // same case as typed: "Do" prefers Dog over dog
        } else if text.to_lowercase().starts_with(&prefix.to_lowercase()) {
            score += 20.0;
        }
        if item.auto_import {
            score -= 25.0;
        }
        scored.push((item.clone(), score));
    }
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| by_server(&a.0, &b.0)));
    scored.into_iter().map(|(i, _)| i).collect()
}

/// The short mark and name shown next to each suggestion.
pub fn kind_icon(kind: Option<u32>) -> (&'static str, &'static str) {
    match kind {
        Some(2) => ("ƒ", "method"),
        Some(3) => ("ƒ", "function"),
        Some(4) => ("ƒ", "constructor"),
        Some(5 | 10) => ("◇", "property"),
        Some(6) => ("x", "variable"),
        Some(7 | 22) => ("C", "class"),
        Some(9) => ("M", "module"),
        Some(14) => ("k", "keyword"),
        Some(21) => ("π", "constant"),
        Some(13 | 20) => ("E", "enum"),
        _ => ("·", "text"),
    }
}

/// A piece of a hover: code (shown monospace) or text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoverSection {
    pub code: bool,
    pub text: String,
}

/// Hover Markdown as simple sections: code blocks and plain text (never full Markdown).
pub fn hover_sections(markdown: &str) -> Vec<HoverSection> {
    static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)```[\w-]*\n(.*?)```").expect("regex"));
    static RULE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(-{3,}|\*{3,}|_{3,})\s*$").expect("regex"));
    static ESCAPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\\([\\`*_{}\[\]()#+\-.!|<>])").expect("regex"));
    let mut out = Vec::new();
    let push_text = |raw: &str, out: &mut Vec<HoverSection>| {
        let kept: Vec<&str> = raw.split('\n').filter(|l| !RULE.is_match(l)).collect();
        let text = ESCAPE.replace_all(&kept.join("\n"), "$1").trim().to_owned();
        if !text.is_empty() {
            out.push(HoverSection { code: false, text });
        }
    };
    let mut last = 0;
    for m in FENCE.captures_iter(markdown) {
        let whole = m.get(0).expect("match");
        push_text(&markdown[last..whole.start()], &mut out);
        let code = m[1].trim();
        if !code.is_empty() {
            out.push(HoverSection { code: true, text: code.to_owned() });
        }
        last = whole.end();
    }
    push_text(&markdown[last..], &mut out);
    out
}

/// The hover result's contents as Markdown.
pub fn hover_markdown(res: &Value) -> Option<String> {
    let c = res.get("contents")?;
    let parts: Vec<&Value> = match c {
        Value::Array(a) => a.iter().collect(),
        v => vec![v],
    };
    let text = parts
        .into_iter()
        .map(|p| match p {
            Value::String(s) => s.clone(),
            v => match (v.get("language").and_then(Value::as_str), v.get("value").and_then(Value::as_str)) {
                (Some(lang), Some(value)) => format!("```{lang}\n{value}\n```"),
                (None, Some(value)) => value.to_owned(),
                _ => String::new(),
            },
        })
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_owned();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str) -> CompletionItem {
        CompletionItem { label: label.into(), kind: Some(6), ..Default::default() }
    }

    fn sorted(label: &str, sort: &str) -> CompletionItem {
        CompletionItem { sort_text: Some(sort.into()), ..item(label) }
    }

    fn labels(v: &[CompletionItem]) -> Vec<&str> {
        v.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn finds_where_the_identifier_starts() {
        assert_eq!(word_start_at("    result = os.pa", 18), 16);
        assert_eq!(word_start_at("x = my_var", 10), 4);
        assert_eq!(word_start_at("foo.", 4), 4);
        assert_eq!(word_start_at("", 0), 0);
    }

    #[test]
    fn filters_and_orders_completions() {
        let items = [sorted("path", "09"), sorted("pathsep", "10"), sorted("getpid", "01"), sorted("_private", "99")];
        assert_eq!(labels(&filter_completions(&items, "")), ["getpid", "path", "pathsep", "_private"]);
        assert_eq!(labels(&filter_completions(&items, "pa")), ["path", "pathsep", "_private"]);
        let ft = CompletionItem { filter_text: Some("zzz".into()), ..item("display") };
        assert_eq!(labels(&filter_completions(&[ft], "zz")), ["display"]);
        assert_eq!(filter_completions(&[item("dog"), item("Dog")], "Do")[0].label, "Dog");
        assert_eq!(filter_completions(&[item("Dog"), item("dog")], "do")[0].label, "dog");
        assert_eq!(filter_completions(&[item("items_list"), item("items")], "items")[0].label, "items");
    }

    #[test]
    fn auto_imports_come_after_names_in_scope() {
        let items = [
            CompletionItem { auto_import: true, sort_text: Some("01".into()), ..item("datetime") },
            CompletionItem { sort_text: Some("09".into()), ..item("data") },
            CompletionItem { auto_import: true, ..item("dataclass") },
        ];
        assert_eq!(labels(&filter_completions(&items, "dat")), ["data", "datetime", "dataclass"]);
        assert_eq!(labels(&filter_completions(&items, "")), ["data", "datetime", "dataclass"]);
    }

    #[test]
    fn reads_server_items() {
        let raw = serde_json::json!({ "label": "append", "kind": 2, "detail": "Auto-import", "textEdit": { "range": { "start": { "line": 2, "character": 8 }, "end": { "line": 2, "character": 9 } }, "newText": "append" }, "documentation": { "kind": "markdown", "value": "Add." } });
        let i = to_completion_item(&raw);
        assert!(i.auto_import);
        assert_eq!(i.text_edit, Some((LspPos::new(2, 8), LspPos::new(2, 9), "append".into())));
        assert_eq!(i.documentation.as_deref(), Some("Add."));
    }

    #[test]
    fn splits_hover_markdown_into_code_and_text() {
        let md = "```python\n(function) def greet(name: str) -> str\n```\n---\nSay hello to someone.\n\nArgs: name";
        assert_eq!(
            hover_sections(md),
            vec![
                HoverSection { code: true, text: "(function) def greet(name: str) -> str".into() },
                HoverSection { code: false, text: "Say hello to someone.\n\nArgs: name".into() },
            ]
        );
        assert_eq!(hover_sections("just text"), vec![HoverSection { code: false, text: "just text".into() }]);
        assert_eq!(
            hover_sections("```python\nx: int\n```\n\n---\n"),
            vec![HoverSection { code: true, text: "x: int".into() }]
        );
        assert_eq!(hover_sections("a\\_b \\*c\\*"), vec![HoverSection { code: false, text: "a_b *c*".into() }]);
    }

    #[test]
    fn reads_hover_contents() {
        assert_eq!(
            hover_markdown(&serde_json::json!({ "contents": { "kind": "markdown", "value": "x" } })).as_deref(),
            Some("x")
        );
        assert_eq!(
            hover_markdown(&serde_json::json!({ "contents": [{ "language": "python", "value": "y" }, "z"] }))
                .as_deref(),
            Some("```python\ny\n```\n\nz")
        );
        assert_eq!(hover_markdown(&serde_json::json!({ "contents": "" })), None);
    }
}
