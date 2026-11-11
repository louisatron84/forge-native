//! Ruff's lint results, and turning a tool's rewritten text (formatting,
//! sorted imports) into the smallest edit, so the cursor and undo stay
//! sensible. Positions are 0-based lines and character columns (Ruff counts
//! characters, like the editor). Replaces `src/shared/lint.ts`.

use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub start: Pos,
    pub end: Pos,
    pub text: String,
}

/// Ruff's fix for a problem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LintFix {
    pub title: String,
    pub edits: Vec<Edit>,
    /// Safe to apply without looking (Ruff's "safe" applicability).
    pub safe: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LintProblem {
    /// Rule code, e.g. F401 ("syntax" for syntax errors).
    pub code: String,
    pub message: String,
    pub start: Pos,
    pub end: Pos,
    pub fix: Option<LintFix>,
    pub url: Option<String>,
}

/// `ruff check --output-format=json` -> problems (0-based positions).
pub fn parse_ruff_json(json: &str) -> Vec<LintProblem> {
    let trimmed = json.trim();
    let data: Value = match serde_json::from_str(if trimmed.is_empty() { "[]" } else { trimmed }) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let Some(list) = data.as_array() else { return Vec::new() };
    let loc = |l: &Value| -> Option<Pos> {
        let row = l.get("row")?.as_u64()? as usize;
        let col = l.get("column")?.as_u64()? as usize;
        Some(Pos::new(row.saturating_sub(1), col.saturating_sub(1)))
    };
    let mut out = Vec::new();
    for d in list {
        let Some(start) = loc(&d["location"]) else { continue };
        let end = loc(&d["end_location"]).unwrap_or(start);
        let Some(message) = d["message"].as_str() else { continue };
        // Syntax errors have no code.
        let code = d["code"].as_str().unwrap_or("syntax").to_owned();
        let fix = d["fix"]["edits"].as_array().and_then(|edits| {
            let edits: Vec<Edit> = edits
                .iter()
                .filter_map(|e| {
                    Some(Edit {
                        start: loc(&e["location"])?,
                        end: loc(&e["end_location"])?,
                        text: e["content"].as_str()?.to_owned(),
                    })
                })
                .collect();
            if edits.is_empty() {
                return None;
            }
            let title = d["fix"]["message"]
                .as_str()
                .filter(|m| !m.is_empty())
                .map_or_else(|| format!("Fix {code}"), str::to_owned);
            let applicability = d["fix"]["applicability"].as_str().unwrap_or("");
            Some(LintFix { title, edits, safe: applicability != "unsafe" && applicability != "display-only" })
        });
        out.push(LintProblem {
            code,
            message: message.to_owned(),
            start,
            end,
            fix,
            url: d["url"].as_str().map(str::to_owned),
        });
    }
    out
}

/// The smallest whole-line edit that turns `before` into `after` (same lines
/// at the start and end are left alone), or None if they're the same.
/// Positions are in `before`'s lines.
pub fn line_diff(before: &str, after: &str) -> Option<Edit> {
    if before == after {
        return None;
    }
    let a: Vec<&str> = before.split('\n').collect();
    let b: Vec<&str> = after.split('\n').collect();
    let len = |s: &str| s.chars().count();
    let mut top = 0;
    while top < a.len() && top < b.len() && a[top] == b[top] {
        top += 1;
    }
    let mut bottom = 0;
    while bottom < a.len() - top && bottom < b.len() - top && a[a.len() - 1 - bottom] == b[b.len() - 1 - bottom] {
        bottom += 1;
    }
    let a_end = a.len() - bottom; // exclusive
    let b_end = b.len() - bottom;
    // Replace lines top..a_end-1 of `before` with lines top..b_end-1 of `after`.
    if a_end > top {
        // Lines removed and none added in their place: delete them with their line breaks.
        if b_end == top {
            return Some(delete_lines(&a, top, a_end));
        }
        // Otherwise replace the changed lines' text (the line break after them stays).
        return Some(Edit {
            start: Pos::new(top, 0),
            end: Pos::new(a_end - 1, len(a[a_end - 1])),
            text: b[top..b_end].join("\n"),
        });
    }
    // Only insertions: put the new lines before line `top`.
    if top < a.len() {
        return Some(Edit { start: Pos::new(top, 0), end: Pos::new(top, 0), text: b[top..b_end].join("\n") + "\n" });
    }
    // At the very end (after the last line).
    let last = a.len() - 1;
    let end = Pos::new(last, len(a[last]));
    Some(Edit { start: end, end, text: format!("\n{}", b[top..b_end].join("\n")) })
}

/// Remove whole lines top..end-1, including their line breaks.
fn delete_lines(a: &[&str], top: usize, end: usize) -> Edit {
    let len = |s: &str| s.chars().count();
    if end < a.len() {
        return Edit { start: Pos::new(top, 0), end: Pos::new(end, 0), text: String::new() };
    }
    // Deleting through the last line: take the line break before them instead.
    if top == 0 {
        return Edit { start: Pos::new(0, 0), end: Pos::new(end - 1, len(a[end - 1])), text: String::new() };
    }
    Edit { start: Pos::new(top - 1, len(a[top - 1])), end: Pos::new(end - 1, len(a[end - 1])), text: String::new() }
}

/// The edit that makes Ruff ignore `code` on a line: append `# noqa: CODE`,
/// or add the code to the line's noqa comment. None if it's already ignored.
pub fn noqa_edit(text: &str, line: usize, code: &str) -> Option<Edit> {
    static NOQA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)#\s*noqa(?::\s*([\w,\s]+))?\s*$").expect("regex"));
    let end = text.chars().count();
    let Some(m) = NOQA.captures(text) else {
        return Some(Edit { start: Pos::new(line, end), end: Pos::new(line, end), text: format!("  # noqa: {code}") });
    };
    // A bare "# noqa" already ignores everything.
    let listed = m.get(1)?;
    let mut codes: Vec<&str> = listed.as_str().split([',', ' ', '\t']).filter(|c| !c.is_empty()).collect();
    if codes.contains(&code) {
        return None;
    }
    codes.push(code);
    let start = text[..m.get(0)?.start()].chars().count();
    Some(Edit { start: Pos::new(line, start), end: Pos::new(line, end), text: format!("# noqa: {}", codes.join(", ")) })
}

/// Apply an edit to text (for checking `line_diff`).
pub fn apply_edit(text: &str, e: &Edit) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    // Byte offset of a (line, char column).
    let off = |p: Pos| -> usize {
        let before: usize = lines.iter().take(p.line).map(|l| l.len() + 1).sum();
        let line = lines.get(p.line).copied().unwrap_or("");
        before + line.char_indices().nth(p.col).map_or(line.len(), |(b, _)| b)
    };
    let (s, t) = (off(e.start).min(text.len()), off(e.end).min(text.len()));
    format!("{}{}{}", &text[..s], e.text, &text[t..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_problems_and_their_fixes_with_0_based_positions() {
        let text = json!([
            {
                "code": "F401", "message": "`os` imported but unused", "url": "https://docs.astral.sh/ruff/rules/unused-import",
                "location": { "row": 1, "column": 8 }, "end_location": { "row": 1, "column": 10 },
                "fix": { "applicability": "safe", "message": "Remove unused import: `os`", "edits": [{ "content": "", "location": { "row": 1, "column": 1 }, "end_location": { "row": 2, "column": 1 } }] }
            },
            { "code": "E501", "message": "Line too long", "location": { "row": 3, "column": 89 }, "end_location": { "row": 3, "column": 120 }, "fix": null },
            { "code": null, "message": "SyntaxError: Expected an expression", "location": { "row": 5, "column": 4 }, "end_location": { "row": 5, "column": 5 } },
            { "code": "X", "message": "bad", "location": null }
        ])
        .to_string();
        let p = parse_ruff_json(&text);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].code, "F401");
        assert_eq!((p[0].start, p[0].end), (Pos::new(0, 7), Pos::new(0, 9)));
        assert_eq!(p[0].url.as_deref(), Some("https://docs.astral.sh/ruff/rules/unused-import"));
        assert_eq!(
            p[0].fix,
            Some(LintFix {
                title: "Remove unused import: `os`".into(),
                safe: true,
                edits: vec![Edit { start: Pos::new(0, 0), end: Pos::new(1, 0), text: String::new() }],
            })
        );
        assert_eq!(p[1].fix, None);
        assert_eq!(p[2].code, "syntax");
        assert_eq!(parse_ruff_json("not json"), Vec::new());
        assert_eq!(parse_ruff_json(""), Vec::new());
    }

    #[test]
    fn unsafe_and_display_only_fixes_are_not_safe() {
        let fix = |applicability: &str| {
            json!([{ "code": "B", "message": "m", "location": { "row": 1, "column": 1 }, "end_location": { "row": 1, "column": 2 },
                "fix": { "applicability": applicability, "message": "", "edits": [{ "content": "x", "location": { "row": 1, "column": 1 }, "end_location": { "row": 1, "column": 2 } }] } }])
            .to_string()
        };
        assert!(!parse_ruff_json(&fix("unsafe"))[0].fix.as_ref().unwrap().safe);
        assert!(!parse_ruff_json(&fix("display-only"))[0].fix.as_ref().unwrap().safe);
        // No message: a title made from the code.
        assert_eq!(parse_ruff_json(&fix("safe"))[0].fix.as_ref().unwrap().title, "Fix B");
    }

    #[test]
    fn noqa_is_appended_or_joins_an_existing_comment() {
        let end = "import os".chars().count();
        assert_eq!(
            noqa_edit("import os", 3, "F401"),
            Some(Edit { start: Pos::new(3, end), end: Pos::new(3, end), text: "  # noqa: F401".into() })
        );
        let line = "x = 1  # noqa: E501";
        assert_eq!(
            noqa_edit(line, 0, "F841"),
            Some(Edit { start: Pos::new(0, 7), end: Pos::new(0, line.len()), text: "# noqa: E501, F841".into() })
        );
        // Already listed, or a bare noqa (which ignores everything): nothing to do.
        assert_eq!(noqa_edit(line, 0, "E501"), None);
        assert_eq!(noqa_edit("x = 1  # noqa", 0, "F841"), None);
        // Columns are characters.
        let line = "é = 1  # NOQA:E501";
        assert_eq!(noqa_edit(line, 2, "F841").map(|e| e.start), Some(Pos::new(2, 7)));
    }

    fn check(a: &str, b: &str) -> Option<Edit> {
        let e = line_diff(a, b);
        let got = match &e {
            None => a.to_owned(),
            Some(e) => apply_edit(a, e),
        };
        assert_eq!(got, b, "from {a:?}");
        e
    }

    #[test]
    fn line_diff_is_none_when_nothing_changed() {
        assert_eq!(line_diff("a\nb", "a\nb"), None);
    }

    #[test]
    fn line_diff_touches_only_the_changed_lines() {
        let e = check("x = 1\ny=2\nz = 3\n", "x = 1\ny = 2\nz = 3\n");
        assert_eq!(e, Some(Edit { start: Pos::new(1, 0), end: Pos::new(1, 3), text: "y = 2".into() }));
    }

    #[test]
    fn line_diff_handles_inserted_and_removed_lines_anywhere() {
        check("import b\nimport a\n\nx\n", "import a\nimport b\n\nx\n");
        check("a\nb\nc\n", "a\nc\n"); // remove a middle line
        check("a\nb\n", "a\nb\nc\n"); // add at the end
        check("a\nb", "a\nb\nc"); // add after a last line without newline
        check("a\nb\nc", "a"); // remove the last lines
        check("a\nb\nc", "c"); // remove the first lines
        check("b\n", "a\nb\n"); // insert at the start
        check("a\nb\n", "x\ny\nz\n"); // replace everything
        check("", "print(1)\n");
        check("x\n\n\n\ny\n", "x\n\n\ny\n");
        check("é = 1\nü=2\n", "é = 1\nü = 2\n"); // columns are characters
    }
}
