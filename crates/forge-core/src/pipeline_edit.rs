//! Text edits used by visual pipelines; all results remain normal Python.
use crate::{
    blocks::*,
    dataflow::{RawFlow, Step},
};
use std::collections::BTreeSet;
fn eol(text: &str) -> &str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}
fn lines(text: &str) -> Vec<String> {
    text.replace("\r\n", "\n").split('\n').map(str::to_owned).collect()
}
fn indent(text: &str) -> String {
    text.chars().take_while(|c| c.is_whitespace()).collect()
}
pub fn fresh_name(base: &str, taken: &BTreeSet<String>) -> String {
    if !taken.contains(base) {
        return base.into();
    }
    (2..).map(|i| format!("{base}_{i}")).find(|s| !taken.contains(s)).unwrap()
}
pub fn placeholder(annotation: Option<&str>) -> &'static str {
    let a = annotation.unwrap_or("");
    if a.starts_with("str") || a.starts_with("Optional[str]") {
        "\"\""
    } else if a.starts_with("int") || a.starts_with("float") {
        "0"
    } else if a.starts_with("bool") {
        "False"
    } else if ["list", "tuple", "List", "Sequence"].iter().any(|s| a.starts_with(s)) {
        "[]"
    } else if ["dict", "Dict", "Mapping"].iter().any(|s| a.starts_with(s)) {
        "{}"
    } else {
        "None"
    }
}
fn out_name(kind: &str) -> &str {
    match kind {
        "read" => "data",
        "api" => "api_data",
        "filter" => "filtered",
        "select" => "selected",
        "transform" => "table",
        "group" => "grouped",
        "merge" => "merged",
        "save" => "saved",
        _ => "result",
    }
}
pub fn block_call(b: &BlockInfo, inputs: &[String], taken: &BTreeSet<String>) -> (String, Option<String>) {
    let mut args = Vec::new();
    let mut next = 0;
    for (i, p) in b.params.iter().enumerate() {
        if p.name.starts_with('*') {
            continue;
        }
        if is_data_param(p, i) {
            args.push(inputs.get(next).cloned().unwrap_or_else(|| placeholder(p.annotation.as_deref()).into()));
            next += 1;
        } else if p.default.is_none() {
            args.push(format!("{}={}", p.name, placeholder(p.annotation.as_deref())));
        }
    }
    let call = format!("{}({})", b.name, args.join(", "));
    if b.kind == "save" && b.returns.is_none() {
        return (call, None);
    }
    let var = fresh_name(out_name(&b.kind), taken);
    (format!("{var} = {call}"), Some(var))
}
pub fn insert_in_function(text: &str, raw: &RawFlow, code: &[String], after: Option<&Step>) -> String {
    let mut lines = lines(text);
    let steps = raw.steps.as_deref().unwrap_or_default();
    let whitespace =
        steps.first().and_then(|s| lines.get(s.line.saturating_sub(1) as usize)).map(|s| indent(s)).unwrap_or_else(
            || format!("{}    ", lines.get(raw.line.saturating_sub(1) as usize).map(|s| indent(s)).unwrap_or_default()),
        );
    let at = after.map(|s| s.end_line).unwrap_or_else(|| {
        steps.last().filter(|s| s.kind == "return").map(|s| s.line.saturating_sub(1)).unwrap_or(raw.end_line)
    }) as usize;
    let at = at.min(lines.len());
    lines.splice(at..at, code.iter().map(|s| if s.is_empty() { String::new() } else { format!("{whitespace}{s}") }));
    lines.join(eol(text))
}
pub fn import_from(text: &str, module: &str, name: &str) -> String {
    let mut ls = lines(text);
    let prefix = format!("from {module} import ");
    for l in &mut ls {
        if let Some(rest) = l.strip_prefix(&prefix) {
            if rest.contains(['(', ')', '#']) {
                continue;
            }
            let mut names: Vec<_> = rest.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            if names.iter().any(|s| s == name || s == "*") {
                return text.into();
            }
            names.push(name.into());
            *l = format!("{prefix}{}", names.join(", "));
            return ls.join(eol(text));
        }
    }
    crate::openapi_code::add_import(text, &format!("{prefix}{name}"))
}
pub fn set_argument(text: &str, step: &Step, index: usize, value: &str) -> String {
    let Some(call) = step.call.as_ref().filter(|c| index < c.args.len()) else {
        return text.into();
    };
    let args = call
        .args
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let t = if i == index { value } else { &a.text };
            a.kw.as_ref().map_or_else(|| t.to_string(), |kw| format!("{kw}={t}"))
        })
        .collect::<Vec<_>>();
    let call = format!("{}({})", call.name, args.join(", "));
    let stmt = if step.kind == "assign" && !step.targets.is_empty() {
        format!("{} = {call}", step.targets.join(", "))
    } else if step.kind == "return" {
        format!("return {call}")
    } else {
        call
    };
    let mut ls = lines(text);
    let start = step.line.saturating_sub(1) as usize;
    if start >= ls.len() || step.end_line as usize > ls.len() {
        return text.into();
    }
    let whitespace = indent(&ls[start]);
    ls.splice(start..step.end_line as usize, [format!("{whitespace}{stmt}")]);
    ls.join(eol(text))
}
pub fn for_each_code(b: &BlockInfo, over: &str, taken: &BTreeSet<String>, table: bool) -> (Vec<String>, String) {
    let item = fresh_name("item", taken);
    let out = fresh_name(&format!("{}_each", out_name(&b.kind)), taken);
    let mut used = taken.clone();
    used.extend([item.clone(), out.clone()]);
    let (call, _) = block_call(b, std::slice::from_ref(&item), &used);
    let call = call.split_once(" = ").map(|(_, s)| s).unwrap_or(&call);
    let iter = if table { format!("{over}.to_dict(\"records\")") } else { over.into() };
    (vec![format!("{out} = []"), format!("for {item} in {iter}:"), format!("    {out}.append({call})")], out)
}
pub fn new_pipeline(text: &str, name: &str) -> String {
    let sep = eol(text);
    let body = text.trim_end();
    let block = format!(
        "{}def {name}():{sep}    \"\"\"A pipeline: add steps from the block library (Pipeline view).\"\"\"{sep}",
        if body.is_empty() { String::new() } else { sep.repeat(3) }
    );
    let re = regex::Regex::new(r#"(?m)^if\s+__name__\s*==\s*["']__main__["']\s*:"#).unwrap();
    if let Some(m) = re.find(body) {
        format!("{}{}{}{}{sep}", body[..m.start()].trim_end(), block, sep.repeat(2), &body[m.start()..])
    } else {
        format!("{body}{block}{sep}{sep}if __name__ == \"__main__\":{sep}    {name}(){sep}")
    }
}
pub fn copy_builtin_to(text: &str, name: &str) -> Option<String> {
    let src = builtin_source(name)?;
    if text.lines().any(|s| s.starts_with(&format!("def {name}("))) {
        return Some(text.into());
    }
    let base = if text.trim().is_empty() {
        "\"\"\"Your pipeline blocks. Functions here appear in Forge's block library.\"\"\"\nfrom forge_blocks import block\nimport pandas as pd\nfrom typing import Any, Callable, Optional"
    } else {
        text.trim_end()
    };
    Some(format!("{base}\n\n\n{src}").replace("\r\n", "\n").replace('\n', eol(text)))
}
pub fn export_standalone(text: &str, func: &str) -> Result<String, String> {
    let mut used = BTreeSet::new();
    let mut kept = Vec::new();
    for line in lines(text) {
        if let Some(names) = line.strip_prefix("from forge_blocks import ") {
            if names.contains(['(', ')', '*', '#']) {
                return Err("Use simple named imports before exporting the pipeline.".into());
            }
            for name in names.split(',').map(str::trim) {
                if name != "block" {
                    if builtin_source(name).is_none() {
                        return Err(format!("Can't export unknown built-in {name}."));
                    }
                    used.insert(name.to_string());
                }
            }
        } else {
            kept.push(line);
        }
    }
    if used.is_empty() {
        return Ok(text.into());
    }
    let mut bodies = used.iter().filter_map(|n| builtin_source(n)).collect::<Vec<_>>().join("\n\n");
    if bodies.contains("_records_table(") {
        bodies.push_str(&BLOCKS_PY[BLOCKS_PY.find("def _records_table(").unwrap()..]);
    }
    let helpers=format!("# Copied from Forge's forge_blocks.py, so this script runs on its own.\nimport json\nimport time\nimport urllib.parse\nimport urllib.request\nfrom typing import Any, Callable, Optional\nimport pandas as pd\n\ndef block(fn):\n    return fn\n\n{bodies}\n");
    let current = kept.join(eol(text));
    let mut out = crate::openapi_code::add_import(&current, &helpers.replace('\n', eol(text)));
    if !out.lines().any(|l| l.starts_with("if __name__")) {
        out.push_str(&format!("\n\nif __name__ == \"__main__\":\n    {func}()\n").replace('\n', eol(text)));
    }
    Ok(out)
}
