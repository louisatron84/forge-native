//! Typed standard-library clients and import edits, replacing `shared/openapiCode.ts`.
use crate::{
    box_code::block_hash,
    openapi::{ApiSpec, AuthScheme, Model, Operation, TypeRef},
};
use std::collections::{HashMap, HashSet};
fn q(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}
fn doc(text: &str) -> String {
    text.replace('\\', "\\\\").replace("\"\"\"", "\\\"\\\"\\\"").split_whitespace().collect::<Vec<_>>().join(" ")
}
pub fn py_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Str => "str".into(),
        TypeRef::Int => "int".into(),
        TypeRef::Float => "float".into(),
        TypeRef::Bool => "bool".into(),
        TypeRef::Any => "Any".into(),
        TypeRef::List { of } => format!("list[{}]", py_type(of)),
        TypeRef::Dict { of } => format!("dict[str, {}]", py_type(of)),
        TypeRef::Model { name } => name.clone(),
    }
}
fn from_json(ty: &TypeRef, expr: &str) -> String {
    match ty {
        TypeRef::Model { name } => format!("{name}.from_dict({expr}) if isinstance({expr}, dict) else {expr}"),
        TypeRef::List { of } => {
            if let TypeRef::Model { name } = of.as_ref() {
                format!("[{name}.from_dict(x) if isinstance(x, dict) else x for x in ({expr} or [])]")
            } else {
                expr.into()
            }
        }
        _ => expr.into(),
    }
}
fn model_code(model: &Model) -> Vec<String> {
    let mut lines = vec![
        "".into(),
        "".into(),
        "@dataclass".into(),
        format!("class {}:", model.name),
        format!(
            "    \"\"\"{}\"\"\"",
            if model.description.is_empty() { format!("The {} object.", model.name) } else { doc(&model.description) }
        ),
        "".into(),
    ];
    let ordered: Vec<_> =
        model.fields.iter().filter(|f| f.required).chain(model.fields.iter().filter(|f| !f.required)).collect();
    for f in &ordered {
        let comment = if f.description.is_empty() {
            String::new()
        } else {
            format!("  # {}", doc(&f.description).chars().take(80).collect::<String>())
        };
        let ty = py_type(&f.ty);
        lines.push(if f.required {
            format!("    {}: {ty}{comment}", f.py)
        } else {
            format!("    {}: Optional[{ty}] = None{comment}", f.py)
        });
    }
    if ordered.is_empty() {
        lines.push("    pass".into())
    }
    lines.extend([
        "".into(),
        "    @classmethod".into(),
        format!("    def from_dict(cls, data: dict) -> {}:", q(&model.name)),
    ]);
    if ordered.is_empty() {
        lines.push("        return cls()".into())
    } else {
        lines.push("        return cls(".into());
        for f in &ordered {
            lines.push(format!("            {}={},", f.py, from_json(&f.ty, &format!("data.get({})", q(&f.name)))))
        }
        lines.push("        )".into());
    }
    lines.extend(["".into(), "    def to_dict(self) -> dict:".into(), "        out = {}".into()]);
    for f in &ordered {
        lines.extend([
            format!("        if self.{} is not None:", f.py),
            format!("            out[{}] = _plain(self.{})", q(&f.name), f.py),
        ]);
    }
    lines.push("        return out".into());
    lines
}
fn operation_code(op: &Operation) -> Vec<String> {
    let path: Vec<_> = op.params.iter().filter(|p| p.location == "path").collect();
    let mut sig = Vec::new();
    for p in path.iter().copied().chain(op.params.iter().filter(|p| p.location != "path" && p.field.required)) {
        sig.push(format!("{}: {}", p.field.py, py_type(&p.field.ty)))
    }
    if let Some(body) = op.body.as_ref().filter(|b| b.required) {
        sig.push(format!("body: {}", py_type(&body.ty)))
    }
    for p in op.params.iter().filter(|p| p.location != "path" && !p.field.required) {
        sig.push(format!("{}: Optional[{}] = None", p.field.py, py_type(&p.field.ty)))
    }
    if let Some(body) = op.body.as_ref().filter(|b| !b.required) {
        sig.push(format!("body: Optional[{}] = None", py_type(&body.ty)))
    }
    let mapping = |location: &str| {
        op.params
            .iter()
            .filter(|p| p.location == location)
            .map(|p| format!("{}: {}", q(&p.field.name), p.field.py))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let path_expr =
        if path.is_empty() { q(&op.path) } else { format!("_path({}, {{{}}})", q(&op.path), mapping("path")) };
    let mut args = vec![q(&op.method), path_expr];
    for (location, name) in [("query", "params"), ("header", "headers")] {
        let values = mapping(location);
        if !values.is_empty() {
            args.push(format!("{name}={{{values}}}"));
        }
    }
    if op.body.is_some() {
        args.push("body=body".into())
    }
    if op.body.as_ref().is_some_and(|b| b.form) {
        args.push("form=True".into())
    }
    vec![
        "".into(),
        "".into(),
        format!("def {}({}) -> {}:", op.id, sig.join(", "), op.response.as_ref().map(py_type).unwrap_or("Any".into())),
        format!(
            "    \"\"\"{} {}{}\"\"\"",
            op.method,
            doc(&op.path),
            if op.summary.is_empty() { String::new() } else { format!(": {}", doc(&op.summary)) }
        ),
        format!("    data = _request({})", args.join(", ")),
        format!("    return {}", op.response.as_ref().map(|t| from_json(t, "data")).unwrap_or("data".into())),
    ]
}
pub fn generate_client(spec: &ApiSpec, only: Option<&HashSet<String>>) -> String {
    let ops: Vec<_> = spec.operations.iter().filter(|o| only.is_none_or(|s| s.contains(&o.id))).collect();
    let by_name: HashMap<_, _> = spec.models.iter().map(|m| (m.name.as_str(), m)).collect();
    let mut needed = HashSet::new();
    fn reach<'a>(ty: &'a TypeRef, models: &HashMap<&str, &'a Model>, needed: &mut HashSet<String>) {
        match ty {
            TypeRef::List { of } | TypeRef::Dict { of } => reach(of, models, needed),
            TypeRef::Model { name } if needed.insert(name.clone()) => {
                if let Some(model) = models.get(name.as_str()) {
                    for field in &model.fields {
                        reach(&field.ty, models, needed)
                    }
                }
            }
            _ => {}
        }
    }
    for op in &ops {
        for ty in op.response.iter().chain(op.body.iter().map(|b| &b.ty)).chain(op.params.iter().map(|p| &p.field.ty)) {
            reach(ty, &by_name, &mut needed)
        }
    }
    let mut lines = vec![
        "\"\"\"".into(),
        format!(
            "Client for {}{}, generated by Forge from its OpenAPI description.",
            doc(&spec.title),
            if spec.version.is_empty() { String::new() } else { format!(" {}", doc(&spec.version)) }
        ),
        "".into(),
        format!(
            "Settings come from the environment (Forge loads .env): {}_BASE_URL{}.",
            spec.env_prefix,
            if spec.auth.is_empty() { "" } else { " and the credentials below" }
        ),
        "It is ordinary Python: edit it freely. Forge notices edits and asks before regenerating over them.".into(),
        "\"\"\"".into(),
        "from __future__ import annotations".into(),
        "".into(),
        "import base64".into(),
        "import json".into(),
        "import os".into(),
        "import urllib.parse".into(),
        "import urllib.request".into(),
        "from dataclasses import dataclass".into(),
        "from typing import Any, Optional".into(),
        "".into(),
        format!("BASE_URL = os.environ.get({}, {})", q(&format!("{}_BASE_URL", spec.env_prefix)), q(&spec.base_url)),
        "".into(),
        "".into(),
        "def _auth(headers: dict, query: dict) -> None:".into(),
        "    \"\"\"Add the credentials from the environment (or .env), when they are set.\"\"\"".into(),
    ];
    if spec.auth.is_empty() {
        lines.push("    pass".into())
    }
    for auth in &spec.auth {
        match auth {
            AuthScheme::ApiKey { location, name, env } => {
                lines.extend([
                    format!("    key = os.environ.get({})", q(env)),
                    "    if key:".into(),
                    match location.as_str() {
                        "query" => format!("        query[{}] = key", q(name)),
                        "cookie" => format!("        headers[\"Cookie\"] = {} + key", q(&format!("{name}="))),
                        _ => format!("        headers[{}] = key", q(name)),
                    },
                ]);
            }
            AuthScheme::Bearer { env } => lines.extend([
                format!("    token = os.environ.get({})", q(env)),
                "    if token:".into(),
                "        headers[\"Authorization\"] = \"Bearer \" + token".into(),
            ]),
            AuthScheme::Basic { user_env, pass_env } => lines.extend([
                format!("    user = os.environ.get({})", q(user_env)),
                "    if user:".into(),
                format!("        pair = (user + \":\" + os.environ.get({}, \"\")).encode(\"utf-8\")", q(pass_env)),
                "        headers[\"Authorization\"] = \"Basic \" + base64.b64encode(pair).decode(\"ascii\")".into(),
            ]),
        }
    }
    lines.extend(["".into(), "".into()]);
    lines.extend(
        include_str!("openapi_runtime.py")
            .replace("    # FORGE_SAFE_OPENER", include_str!("http_redirect.py").trim_end())
            .lines()
            .map(str::to_owned),
    );
    for model in spec.models.iter().filter(|m| needed.contains(&m.name)) {
        lines.extend(model_code(model))
    }
    for op in ops {
        lines.extend(operation_code(op))
    }
    let body = lines.join("\n") + "\n";
    format!("# forge:openapi {} {}\n{body}", spec.slug, block_hash(&body))
}
#[derive(Clone, Debug, PartialEq)]
pub struct GeneratedState {
    pub slug: String,
    pub edited: bool,
}
pub fn generated_state(text: &str) -> Option<GeneratedState> {
    let text = text.replace("\r\n", "\n");
    let (first, body) = text.split_once('\n').unwrap_or((&text, &text));
    let mut parts = first.strip_prefix("# forge:openapi ")?.split(' ');
    let slug = parts.next()?;
    let hash = parts.next()?;
    if slug.is_empty()
        || hash.len() != 8
        || !hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || parts.next().is_some()
    {
        return None;
    }
    Some(GeneratedState { slug: slug.into(), edited: block_hash(body) != hash })
}
pub fn import_line(module_path: &str, names: &[String]) -> String {
    let path = module_path.replace('\\', "/");
    let path = path.strip_suffix(".py").unwrap_or(&path);
    let path = path.strip_suffix("/__init__").unwrap_or(path);
    format!("from {} import {}", path.replace('/', "."), names.join(", "))
}
pub fn add_import(text: &str, line: &str) -> String {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<_> = text.split('\n').map(|s| s.trim_end_matches('\r').to_owned()).collect();
    if lines.iter().any(|s| s.trim() == line) {
        return text.into();
    }
    let mut at = 0;
    let mut i = 0;
    while lines.get(i).is_some_and(|s| s.starts_with('#') || (s.trim().is_empty() && i + 1 < lines.len())) {
        i += 1;
        at = i;
    }
    let first = lines.get(i).map(String::as_str).unwrap_or("").trim_start();
    let first = first.strip_prefix(['r', 'u', 'b', 'R', 'U', 'B']).unwrap_or(first);
    if let Some(quote) = ["\"\"\"", "'''"].into_iter().find(|q| first.starts_with(q)) {
        i = if first[3..].contains(quote) {
            i + 1
        } else {
            lines.iter().enumerate().skip(i + 1).find(|(_, s)| s.contains(quote)).map(|(i, _)| i + 1).unwrap_or(i)
        };
        at = i;
    }
    let mut depth = 0isize;
    let mut continued = false;
    for (index, text) in lines.iter().enumerate().skip(i) {
        let code = text.split('#').next().unwrap_or("").trim_end();
        let import = ["import", "from"]
            .iter()
            .any(|prefix| text.strip_prefix(prefix).is_some_and(|rest| rest.starts_with(char::is_whitespace)));
        if import || depth > 0 || continued {
            depth += code
                .chars()
                .map(|c| match c {
                    '(' => 1,
                    ')' => -1,
                    _ => 0,
                })
                .sum::<isize>();
            continued = code.ends_with('\\');
            at = index + 1;
        } else if !text.trim().is_empty() && !text.starts_with('#') {
            break;
        }
    }
    lines.insert(at, line.into());
    lines.join(eol)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_clients_and_operation_subsets_match_hardened_fixtures() {
        let cases: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/openapi.json")).unwrap();
        for c in cases.as_array().unwrap() {
            let spec = crate::openapi::read_spec(&c["doc"], c["from"].as_str().unwrap()).unwrap();
            let code = generate_client(&spec, None);
            assert_eq!(code, c["code"].as_str().unwrap());
            let only = std::collections::HashSet::from(["delete_pets_by".to_owned()]);
            assert_eq!(generate_client(&spec, Some(&only)), c["subset"].as_str().unwrap());
            assert!(!generated_state(&code).unwrap().edited);
            assert!(!generated_state(&code.replace('\n', "\r\n")).unwrap().edited);
            assert!(generated_state(&(code + "# edit\n")).unwrap().edited);
        }
        assert!(generated_state("import os\n").is_none());
    }
    #[test]
    fn imports_follow_imports_or_docstring_and_preserve_crlf_without_duplicates() {
        let line = "from api.x import f";
        assert_eq!(
            import_line("api/petstore_client.py", &["list_pets".into(), "Pet".into()]),
            "from api.petstore_client import list_pets, Pet"
        );
        for (before, after) in [
            ("import os\nimport sys\n\nprint(1)\n", "import os\nimport sys\nfrom api.x import f\n\nprint(1)\n"),
            ("\"\"\"Doc.\"\"\"\nx = 1\n", "\"\"\"Doc.\"\"\"\nfrom api.x import f\nx = 1\n"),
            ("x = 1\n", "from api.x import f\nx = 1\n"),
            ("import os\r\nx = 1\r\n", "import os\r\nfrom api.x import f\r\nx = 1\r\n"),
        ] {
            assert_eq!(add_import(before, line), after);
            assert_eq!(add_import(after, line), after);
        }
    }
    #[test]
    fn imports_stay_outside_parenthesized_imports_and_follow_python_file_headers() {
        let text = "#!/usr/bin/env python\n# coding: utf-8\nfrom things import (\n    a,\n    b,\n)\nprint(a)\n";
        assert_eq!(add_import(text,"from api.x import f"),"#!/usr/bin/env python\n# coding: utf-8\nfrom things import (\n    a,\n    b,\n)\nfrom api.x import f\nprint(a)\n");
        assert_eq!(
            add_import("#!/usr/bin/env python\nprint(1)\n", "import api.x"),
            "#!/usr/bin/env python\nimport api.x\nprint(1)\n"
        );
    }
}
