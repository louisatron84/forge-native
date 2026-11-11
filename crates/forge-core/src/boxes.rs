//! Connect API boxes and compatible `.forge/boxes.json`, replacing
//! `src/shared/boxes.ts`. Only environment-variable names are persisted.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
pub const HTTP_METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct KeyValue {
    pub key: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub arg: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", rename_all_fields = "camelCase")]
pub enum BoxAuth {
    #[default]
    None,
    Header {
        name: String,
        env: String,
    },
    Query {
        name: String,
        env: String,
    },
    Bearer {
        env: String,
    },
    Basic {
        user_env: String,
        pass_env: String,
    },
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FieldNode {
    pub path: Vec<String>,
    pub name: String,
    pub kind: String,
    pub sample: String,
    pub children: Vec<FieldNode>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TestSummary {
    pub ok: bool,
    pub at: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<FieldNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample: Option<std::sync::Arc<Value>>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ApiRequest {
    pub id: String,
    pub name: String,
    pub method: String,
    pub url: String,
    pub query: Vec<KeyValue>,
    pub headers: Vec<KeyValue>,
    pub body: String,
    pub test_args: BTreeMap<String, String>,
    pub records_path: Option<Vec<String>>,
    pub ticked: Vec<String>,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test: Option<TestSummary>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BoxTarget {
    pub file: String,
    pub mode: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BoxAttach {
    pub module: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ApiBox {
    pub id: String,
    #[serde(rename = "type")]
    pub box_type: String,
    pub name: String,
    pub base_url: String,
    pub auth: BoxAuth,
    pub target: BoxTarget,
    pub attach: Option<BoxAttach>,
    pub requests: Vec<ApiRequest>,
    pub hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
    pub detached: bool,
}
impl ApiBox {
    /// Snapshot editable metadata without cloning potentially megabytes of
    /// response samples. Samples and inferred fields are deliberately transient.
    pub fn metadata(&self) -> Self {
        Self {
            id: self.id.clone(),
            box_type: self.box_type.clone(),
            name: self.name.clone(),
            base_url: self.base_url.clone(),
            auth: self.auth.clone(),
            target: self.target.clone(),
            attach: self.attach.clone(),
            hash: self.hash.clone(),
            prev_hash: self.prev_hash.clone(),
            detached: self.detached,
            requests: self
                .requests
                .iter()
                .map(|r| ApiRequest {
                    id: r.id.clone(),
                    name: r.name.clone(),
                    method: r.method.clone(),
                    url: r.url.clone(),
                    query: r.query.clone(),
                    headers: r.headers.clone(),
                    body: r.body.clone(),
                    test_args: r.test_args.clone(),
                    records_path: r.records_path.clone(),
                    ticked: r.ticked.clone(),
                    output: r.output.clone(),
                    function_name: r.function_name.clone(),
                    test: r.test.as_ref().map(|t| TestSummary {
                        ok: t.ok,
                        at: t.at,
                        status: t.status,
                        status_text: t.status_text.clone(),
                        count: t.count,
                        ms: t.ms,
                        error: t.error.clone(),
                        fields: None,
                        sample: None,
                    }),
                })
                .collect(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoxesFile {
    pub version: u32,
    pub boxes: Vec<ApiBox>,
}
pub fn py_name(text: &str) -> String {
    let mut result = String::new();
    for ch in text.to_lowercase().chars() {
        let c = if ch.is_ascii_alphanumeric() || ch == '_' { ch } else { '_' };
        if c != '_' || !result.ends_with('_') {
            result.push(c);
        }
    }
    let mut result = result.trim_matches('_').to_owned();
    if result.is_empty() {
        return "api".into();
    }
    if result.as_bytes()[0].is_ascii_digit() {
        result.insert(0, '_');
    }
    result
}
pub fn env_name(text: &str, suffix: &str) -> String {
    format!("{}_{suffix}", py_name(text).trim_start_matches('_').to_uppercase())
}
pub fn function_name_for(api: &ApiBox, request: &ApiRequest) -> String {
    request
        .function_name
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(py_name)
        .unwrap_or_else(|| py_name(&format!("{}_{}", api.name, request.name)))
}
fn fresh_id<'a>(prefix: &str, taken: impl Iterator<Item = &'a str>) -> String {
    let taken: HashSet<_> = taken.collect();
    let mut number = 1;
    loop {
        let id = format!("{prefix}{number}");
        if !taken.contains(id.as_str()) {
            return id;
        }
        number += 1;
    }
}
pub fn new_api_box(existing: &[String], name: &str) -> ApiBox {
    let mut api = ApiBox {
        id: fresh_id(
            &format!("{}-", py_name(name).replace('_', "-").trim_start_matches('-')),
            existing.iter().map(String::as_str),
        ),
        box_type: "api".into(),
        name: name.into(),
        target: BoxTarget { file: format!("api/{}.py", py_name(name)), mode: "new".into() },
        ..Default::default()
    };
    api.requests.push(new_request(&api, "data"));
    api
}
pub fn new_request(api: &ApiBox, name: &str) -> ApiRequest {
    ApiRequest {
        id: fresh_id("r", api.requests.iter().map(|r| r.id.as_str())),
        name: name.into(),
        method: "GET".into(),
        output: format!("{}_{}.csv", py_name(&api.name), py_name(name)),
        ..Default::default()
    }
}
pub fn module_of_path(path: &str) -> String {
    let mut path = path.replace('\\', "/");
    let lower = path.to_lowercase();
    if lower.ends_with(".pyi") {
        path.truncate(path.len() - 4);
    } else if lower.ends_with(".py") {
        path.truncate(path.len() - 3);
    }
    if path.ends_with("/__init__") {
        path.truncate(path.len() - 9);
    }
    path.split('/').filter(|p| !p.is_empty()).collect::<Vec<_>>().join(".")
}
pub fn rename_box(api: &mut ApiBox, name: &str) {
    let old = std::mem::replace(&mut api.name, name.into());
    let old_file = format!("api/{}.py", py_name(&old));
    if api.hash.is_none() && api.target.mode == "new" && api.target.file == old_file {
        api.target.file = format!("api/{}.py", py_name(name));
        if api.attach.as_ref().is_none_or(|a| a.module == module_of_path(&old_file)) {
            api.attach = Some(BoxAttach { module: module_of_path(&api.target.file) });
        }
    }
    for r in &mut api.requests {
        if r.output == format!("{}_{}.csv", py_name(&old), py_name(&r.name)) {
            r.output = format!("{}_{}.csv", py_name(name), py_name(&r.name));
        }
    }
}
fn string(raw: &Value, key: &str) -> String {
    raw[key].as_str().unwrap_or_default().into()
}
fn js_string(raw: &Value) -> String {
    match raw {
        Value::String(s) => s.clone(),
        Value::Object(_) => "[object Object]".into(),
        Value::Array(a) => {
            a.iter().map(|v| if v.is_null() { String::new() } else { js_string(v) }).collect::<Vec<_>>().join(",")
        }
        _ => raw.to_string(),
    }
}
fn norm_kv(raw: &Value) -> Vec<KeyValue> {
    raw.as_array()
        .into_iter()
        .flatten()
        .filter(|v| v.is_object())
        .map(|v| KeyValue { key: string(v, "key"), value: string(v, "value"), arg: v["arg"] == true })
        .collect()
}
fn norm_auth(raw: &Value) -> BoxAuth {
    match raw["kind"].as_str() {
        Some("header") => BoxAuth::Header { name: string(raw, "name"), env: string(raw, "env") },
        Some("query") => BoxAuth::Query { name: string(raw, "name"), env: string(raw, "env") },
        Some("bearer") => BoxAuth::Bearer { env: string(raw, "env") },
        Some("basic") => BoxAuth::Basic { user_env: string(raw, "userEnv"), pass_env: string(raw, "passEnv") },
        _ => BoxAuth::None,
    }
}
fn norm_fields(raw: &Value, depth: usize) -> Vec<FieldNode> {
    if depth >= 32 {
        return Vec::new();
    }
    raw.as_array()
        .into_iter()
        .flatten()
        .filter(|v| v.is_object())
        .map(|v| FieldNode {
            path: v["path"].as_array().into_iter().flatten().map(js_string).collect(),
            name: string(v, "name"),
            kind: if matches!(v["kind"].as_str(), Some("object" | "list")) {
                string(v, "kind")
            } else {
                "value".into()
            },
            sample: string(v, "sample"),
            children: norm_fields(&v["children"], depth + 1),
        })
        .collect()
}
fn norm_test(raw: &Value) -> Option<TestSummary> {
    if !raw.is_object() {
        return None;
    }
    Some(TestSummary {
        ok: raw["ok"] == true,
        at: raw["at"].as_f64().unwrap_or(0.0),
        status: raw["status"].as_f64(),
        status_text: raw["statusText"].as_str().map(str::to_owned),
        count: raw["count"].as_f64(),
        ms: raw["ms"].as_f64(),
        error: raw["error"].as_str().map(str::to_owned),
        fields: raw["fields"].is_array().then(|| norm_fields(&raw["fields"], 0)),
        sample: None,
    })
}
pub fn normalize_boxes(raw: &Value) -> Vec<ApiBox> {
    let mut result: Vec<ApiBox> = Vec::new();
    for item in raw.get("boxes").and_then(Value::as_array).into_iter().flatten() {
        if !item.is_object() || item["type"] != "api" {
            continue;
        }
        let ids: Vec<_> = result.iter().map(|b| b.id.clone()).collect();
        let mut api = new_api_box(&ids, item["name"].as_str().unwrap_or("API"));
        if let Some(id) = item["id"].as_str().filter(|id| !id.is_empty() && !ids.iter().any(|x| x == id)) {
            api.id = id.into();
        }
        api.base_url = string(item, "baseUrl");
        api.auth = norm_auth(&item["auth"]);
        if let Some(file) = item["target"]["file"].as_str().filter(|s| !s.is_empty()) {
            api.target = BoxTarget {
                file: file.into(),
                mode: if item["target"]["mode"] == "append" { "append" } else { "new" }.into(),
            };
        }
        api.attach = item["attach"]["module"].as_str().map(|m| BoxAttach { module: m.into() });
        api.hash = item["hash"].as_str().map(str::to_owned);
        api.prev_hash = item["prevHash"].as_str().map(str::to_owned);
        api.detached = item["detached"] == true;
        if let Some(requests) = item["requests"].as_array() {
            api.requests.clear();
            for raw in requests.iter().filter(|v| v.is_object()) {
                let mut r = new_request(&api, raw["name"].as_str().unwrap_or("data"));
                if let Some(id) =
                    raw["id"].as_str().filter(|s| !s.is_empty() && !api.requests.iter().any(|r| r.id == *s))
                {
                    r.id = id.into();
                }
                if let Some(method) = raw["method"].as_str().filter(|m| HTTP_METHODS.contains(m)) {
                    r.method = method.into();
                }
                r.url = string(raw, "url");
                r.query = norm_kv(&raw["query"]);
                r.headers = norm_kv(&raw["headers"]);
                r.body = string(raw, "body");
                r.test_args =
                    raw["testArgs"].as_object().into_iter().flatten().map(|(k, v)| (k.clone(), js_string(v))).collect();
                r.records_path = raw["recordsPath"].as_array().map(|a| a.iter().map(js_string).collect());
                r.ticked = raw["ticked"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect();
                if let Some(output) = raw["output"].as_str().filter(|s| !s.is_empty()) {
                    r.output = output.into();
                }
                r.function_name = raw["functionName"].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
                r.test = norm_test(&raw["test"]);
                api.requests.push(r);
            }
        }
        if api.requests.is_empty() {
            api.requests.push(new_request(&api, "data"));
        }
        result.push(api);
    }
    result
}
pub fn serialize_boxes(boxes: &[ApiBox]) -> BoxesFile {
    BoxesFile { version: 1, boxes: boxes.iter().map(ApiBox::metadata).collect() }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn python_and_environment_names_match_electron() {
        for (text, expected) in [
            ("My API!", "my_api"),
            ("2nd list", "_2nd_list"),
            ("  GitHub -- Users ", "github_users"),
            ("", "api"),
            ("!!!", "api"),
        ] {
            assert_eq!(py_name(text), expected);
        }
        assert_eq!(env_name("GitHub", "TOKEN"), "GITHUB_TOKEN");
        assert_eq!(env_name("My API!", "KEY"), "MY_API_KEY");
    }
    #[test]
    fn function_names_join_box_and_request_unless_overridden() {
        let b = new_api_box(&[], "GitHub");
        let mut r = b.requests[0].clone();
        r.name = "users".into();
        assert_eq!(function_name_for(&b, &r), "github_users");
        r.function_name = Some("Get Users".into());
        assert_eq!(function_name_for(&b, &r), "get_users");
    }
    #[test]
    fn new_boxes_have_sensible_defaults_and_unique_ids() {
        let b = new_api_box(&[], "GitHub");
        assert_eq!(b.box_type, "api");
        assert_eq!(b.auth, BoxAuth::None);
        assert_eq!(b.target, BoxTarget { file: "api/github.py".into(), mode: "new".into() });
        assert_eq!(b.requests.len(), 1);
        assert_eq!(b.requests[0].output, "github_data.csv");
        assert_eq!(b.requests[0].method, "GET");
        assert!(b.hash.is_none() && !b.detached);
        assert_ne!(new_api_box(std::slice::from_ref(&b.id), "GitHub").id, b.id);
        assert_ne!(new_request(&b, "data").id, b.requests[0].id);
    }
    #[test]
    fn module_paths_and_renaming_default_paths_follow_the_name_until_written() {
        for (path, module) in
            [("api/github.py", "api.github"), ("pkg\\sub\\__init__.py", "pkg.sub"), ("main.py", "main")]
        {
            assert_eq!(module_of_path(path), module);
        }
        let mut b = new_api_box(&[], "API");
        b.attach = Some(BoxAttach { module: "api.api".into() });
        rename_box(&mut b, "GitHub");
        assert_eq!(b.target.file, "api/github.py");
        assert_eq!(b.attach.as_ref().unwrap().module, "api.github");
        assert_eq!(b.requests[0].output, "github_data.csv");
        b.requests[0].output = "mine.csv".into();
        b.hash = Some("abc".into());
        rename_box(&mut b, "Hub");
        assert_eq!(b.target.file, "api/github.py");
        assert_eq!(b.requests[0].output, "mine.csv");
    }
    #[test]
    fn normalization_drops_junk_and_fixes_fields_and_duplicate_ids() {
        let good = serde_json::to_value(new_api_box(&[], "GitHub")).unwrap();
        let mut edited = good.clone();
        edited["hash"] = json!("abc");
        edited["detached"] = json!(true);
        edited["requests"][0]["method"] = json!("FETCH");
        let boxes = normalize_boxes(&json!({"version":1,"boxes":[42,{"type":"database","id":"x"},edited,good]}));
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[0].requests[0].method, "GET");
        assert_eq!(boxes[0].hash.as_deref(), Some("abc"));
        assert!(boxes[0].detached);
        assert_ne!(boxes[0].id, boxes[1].id);
        assert!(normalize_boxes(&Value::Null).is_empty());
        assert!(normalize_boxes(&json!({"boxes":"no"})).is_empty());
    }
    #[test]
    fn serialization_round_trips_without_response_samples_or_fields() {
        let mut b = new_api_box(&[], "GitHub");
        b.auth = BoxAuth::Bearer { env: "GITHUB_TOKEN".into() };
        b.requests[0].test = Some(TestSummary {
            ok: true,
            at: 1.0,
            status: Some(200.0),
            sample: Some(json!([{"secret":"x"}]).into()),
            fields: Some(vec![FieldNode {
                name: "email".into(),
                sample: "ann@private.example".into(),
                ..Default::default()
            }]),
            ..Default::default()
        });
        let value = serde_json::to_value(serialize_boxes(&[b])).unwrap();
        let text = value.to_string();
        assert!(!text.contains("secret") && !text.contains("ann@private.example"));
        let back = normalize_boxes(&value);
        assert_eq!(back[0].auth, BoxAuth::Bearer { env: "GITHUB_TOKEN".into() });
        assert!(back[0].requests[0].test.as_ref().unwrap().ok);
    }
}
