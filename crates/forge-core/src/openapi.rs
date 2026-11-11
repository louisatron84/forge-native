//! OpenAPI 3 / Swagger 2 model, replacing `shared/openapi.ts`.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TypeRef {
    Str,
    Int,
    Float,
    Bool,
    Any,
    List { of: Box<TypeRef> },
    Dict { of: Box<TypeRef> },
    Model { name: String },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub py: String,
    #[serde(rename = "type")]
    pub ty: TypeRef,
    pub required: bool,
    pub description: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub name: String,
    pub description: String,
    pub fields: Vec<Field>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Param {
    #[serde(flatten)]
    pub field: Field,
    #[serde(rename = "in")]
    pub location: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Body {
    #[serde(rename = "type")]
    pub ty: TypeRef,
    pub required: bool,
    pub form: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Operation {
    pub id: String,
    pub method: String,
    pub path: String,
    pub summary: String,
    pub tag: String,
    pub params: Vec<Param>,
    pub body: Option<Body>,
    pub response: Option<TypeRef>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all_fields = "camelCase")]
pub enum AuthScheme {
    #[serde(rename = "apiKey")]
    ApiKey {
        #[serde(rename = "in")]
        location: String,
        name: String,
        env: String,
    },
    #[serde(rename = "bearer")]
    Bearer { env: String },
    #[serde(rename = "basic")]
    Basic { user_env: String, pass_env: String },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiSpec {
    pub title: String,
    pub version: String,
    pub description: String,
    pub base_url: String,
    pub slug: String,
    pub env_prefix: String,
    pub auth: Vec<AuthScheme>,
    pub operations: Vec<Operation>,
    pub models: Vec<Model>,
    pub warnings: Vec<String>,
}
const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
    "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal",
    "not", "or", "pass", "raise", "return", "try", "while", "with", "yield", "match", "case", "type",
];
fn camel() -> &'static regex::Regex {
    static R: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new("([a-z0-9])([A-Z])").expect("fixed regex"));
    &R
}
pub fn py_ident(text: &str) -> String {
    static R: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new("([A-Z]+)([A-Z][a-z])").expect("fixed regex"));
    let snake = camel().replace_all(text, "${1}_${2}");
    let snake = R.replace_all(&snake, "${1}_${2}");
    let mut name = crate::boxes::py_name(&snake);
    if name == "api" && !text.to_lowercase().contains("api") {
        name = "value".into()
    }
    if KEYWORDS.contains(&name.as_str()) {
        name.push('_')
    }
    name
}
pub fn py_class(text: &str) -> String {
    let separated = camel().replace_all(text, "$1 $2");
    let mut name = String::new();
    for part in separated.split(|c: char| !c.is_ascii_alphanumeric()).filter(|p| !p.is_empty()) {
        let mut chars = part.chars();
        if let Some(c) = chars.next() {
            name.extend(c.to_uppercase());
            name.extend(chars);
        }
    }
    if name.is_empty() {
        name = "Model".into()
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        name.insert(0, 'M')
    }
    if KEYWORDS.contains(&name.as_str()) {
        name.push('_')
    }
    name
}
fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").into()
}
fn arr(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn entries(v: &Value) -> impl Iterator<Item = (&String, &Value)> {
    v.as_object().into_iter().flatten()
}
fn object_schema(v: &Value) -> bool {
    v["properties"].is_object() || arr(&v["allOf"]).len() > 1
}
struct Reader<'a> {
    doc: &'a Value,
    prefix: &'static str,
    models: Vec<Model>,
    built: HashSet<String>,
    building: HashSet<String>,
    names: HashMap<String, String>,
    used: HashSet<String>,
    warnings: Vec<String>,
}
impl Reader<'_> {
    fn resolve(&mut self, raw: &Value) -> Value {
        let mut value = raw.clone();
        for _ in 0..21 {
            let Some(reference) = value["$ref"].as_str() else { break };
            if !reference.starts_with("#/") {
                self.warnings.push(format!("Skipped a reference to another file: {reference}"));
                return Value::Object(Map::new());
            }
            value = self.doc.pointer(&reference[1..]).cloned().unwrap_or(Value::Null);
        }
        value
    }
    fn name_for(&mut self, key: &str) -> String {
        if let Some(name) = self.names.get(key) {
            return name.clone();
        }
        let base = py_class(key);
        let mut name = base.clone();
        let mut n = 2;
        while self.used.contains(&name) {
            name = format!("{base}{n}");
            n += 1;
        }
        self.used.insert(name.clone());
        self.names.insert(key.into(), name.clone());
        name
    }
    fn type_of(&mut self, raw: &Value, depth: usize) -> TypeRef {
        if depth > 12 {
            return TypeRef::Any;
        }
        if let Some(key) = raw["$ref"].as_str().and_then(|r| r.strip_prefix(self.prefix)) {
            if self.building.len() >= 64 {
                self.warnings.push("Stopped a deeply nested model reference.".into());
                return TypeRef::Any;
            }
            let target = self.resolve(raw);
            if object_schema(&target) {
                self.build_model(key, &target);
                return TypeRef::Model { name: self.name_for(key) };
            }
            return self.type_of(&target, depth + 1);
        }
        let schema = self.resolve(raw);
        if !schema.is_object() {
            return TypeRef::Any;
        }
        if arr(&schema["allOf"]).len() == 1 {
            return self.type_of(&schema["allOf"][0], depth + 1);
        }
        if !schema["oneOf"].is_null() || !schema["anyOf"].is_null() {
            return TypeRef::Any;
        }
        let ty = if schema["type"].is_array() {
            arr(&schema["type"]).iter().find(|v| v.as_str() != Some("null")).and_then(Value::as_str)
        } else {
            schema["type"].as_str()
        };
        match ty {
            Some("string") => TypeRef::Str,
            Some("integer") => TypeRef::Int,
            Some("number") => TypeRef::Float,
            Some("boolean") => TypeRef::Bool,
            Some("array") => TypeRef::List { of: Box::new(self.type_of(&schema["items"], depth + 1)) },
            _ => {
                if schema["additionalProperties"].is_object() && !schema["properties"].is_object() {
                    TypeRef::Dict { of: Box::new(self.type_of(&schema["additionalProperties"], depth + 1)) }
                } else if ty == Some("object") || schema["properties"].is_object() {
                    TypeRef::Dict { of: Box::new(TypeRef::Any) }
                } else {
                    TypeRef::Any
                }
            }
        }
    }
    fn collect(
        &mut self,
        schema: &Value,
        props: &mut Map<String, Value>,
        required: &mut HashSet<String>,
        depth: usize,
        budget: &mut usize,
    ) {
        if depth > 32 || *budget == 0 {
            if !self.warnings.iter().any(|s| s == "Stopped a recursive allOf schema.") {
                self.warnings.push("Stopped a recursive allOf schema.".into());
            }
            return;
        }
        *budget -= 1;
        let schema = self.resolve(schema);
        for part in arr(&schema["allOf"]) {
            self.collect(part, props, required, depth + 1, budget)
        }
        for (key, value) in entries(&schema["properties"]) {
            props.insert(key.clone(), value.clone());
        }
        required.extend(arr(&schema["required"]).iter().filter_map(Value::as_str).map(str::to_owned));
    }
    fn build_model(&mut self, key: &str, schema: &Value) {
        if self.built.contains(key) || !self.building.insert(key.into()) {
            return;
        }
        let mut props = Map::new();
        let mut required = HashSet::new();
        self.collect(schema, &mut props, &mut required, 0, &mut 4096);
        let mut used = HashSet::new();
        let mut fields = Vec::new();
        for (name, value) in props {
            let base = py_ident(&name);
            let py = unique(&base, &mut used, "_");
            let ty = self.type_of(&value, 0);
            let description = s(&self.resolve(&value)["description"]);
            fields.push(Field { required: required.contains(&name), name, py, ty, description });
        }
        let name = self.name_for(key);
        self.models.push(Model { name, description: s(&schema["description"]), fields });
        self.built.insert(key.into());
        self.building.remove(key);
    }
}
fn unique(base: &str, used: &mut HashSet<String>, separator: &str) -> String {
    let mut name = base.to_owned();
    let mut n = 2;
    while used.contains(&name) {
        name = format!("{base}{separator}{n}");
        n += 1
    }
    used.insert(name.clone());
    name
}
pub fn read_spec(doc: &Value, from: &str) -> Result<ApiSpec, String> {
    if !doc.is_object() {
        return Err("This isn't an OpenAPI description (expected a JSON or YAML object).".into());
    }
    let v3 = doc["openapi"].is_string();
    let v2 = doc["swagger"] == "2.0" || doc["swagger"] == 2;
    if !v3 && !v2 {
        return Err(
            "This isn't an OpenAPI description: it has neither \"openapi\" nor \"swagger\": \"2.0\" at the top.".into(),
        );
    }
    let title = doc["info"]["title"].as_str().unwrap_or("API").to_owned();
    static API: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\bapi\b").expect("fixed regex"));
    let stripped = API.replace_all(&title, "");
    let slug = crate::boxes::py_name(if stripped.trim().is_empty() { &title } else { stripped.trim() });
    let env_prefix = slug.trim_start_matches('_').to_uppercase();
    let env_prefix = if env_prefix.is_empty() { "API".into() } else { env_prefix };
    let mut r = Reader {
        doc,
        prefix: if v3 { "#/components/schemas/" } else { "#/definitions/" },
        models: Vec::new(),
        built: HashSet::new(),
        building: HashSet::new(),
        names: HashMap::new(),
        used: HashSet::new(),
        warnings: Vec::new(),
    };
    let schemas = if v3 { &doc["components"]["schemas"] } else { &doc["definitions"] };
    for (key, schema) in entries(schemas) {
        if object_schema(schema) {
            r.build_model(key, schema)
        }
    }
    let mut base_url = if v3 {
        let server = &doc["servers"][0];
        let mut url = s(&server["url"]);
        for (key, value) in entries(&server["variables"]) {
            url = url.replacen(&format!("{{{key}}}"), &s(&value["default"]), 1);
        }
        url
    } else if let Some(host) = doc["host"].as_str() {
        let scheme = if arr(&doc["schemes"]).iter().any(|v| v == "https") {
            "https"
        } else {
            doc["schemes"][0].as_str().unwrap_or("https")
        };
        format!("{scheme}://{host}{}", s(&doc["basePath"]))
    } else {
        s(&doc["basePath"])
    };
    if !base_url.is_empty() && !base_url.contains("://") {
        if let Some(origin) = url::Url::parse(from).ok().filter(|u| matches!(u.scheme(), "http" | "https")) {
            if let Ok(full) = origin.join(&base_url) {
                base_url = full.to_string()
            }
        } else {
            r.warnings.push(format!(
                "The server address \"{base_url}\" is relative: set {env_prefix}_BASE_URL to the full address."
            ));
        }
    }
    base_url = base_url.trim_end_matches('/').into();
    let mut auth = Vec::new();
    let mut kinds = HashSet::new();
    let schemes = if v3 { &doc["components"]["securitySchemes"] } else { &doc["securityDefinitions"] };
    for (_, raw) in entries(schemes) {
        let a = r.resolve(raw);
        let ty = a["type"].as_str().unwrap_or("");
        if ty == "apiKey" && a["name"].is_string() {
            if kinds.insert("apiKey") {
                auth.push(AuthScheme::ApiKey {
                    location: match a["in"].as_str() {
                        Some("query") => "query",
                        Some("cookie") => "cookie",
                        _ => "header",
                    }
                    .into(),
                    name: s(&a["name"]),
                    env: format!("{env_prefix}_API_KEY"),
                });
            }
        } else if ty == "basic" || (ty == "http" && s(&a["scheme"]).eq_ignore_ascii_case("basic")) {
            if kinds.insert("basic") {
                auth.push(AuthScheme::Basic {
                    user_env: format!("{env_prefix}_USERNAME"),
                    pass_env: format!("{env_prefix}_PASSWORD"),
                });
            }
        } else if ["http", "oauth2", "openIdConnect"].contains(&ty) && kinds.insert("bearer") {
            auth.push(AuthScheme::Bearer { env: format!("{env_prefix}_TOKEN") });
        }
    }
    let mut operations = Vec::new();
    let mut used_ids = HashSet::new();
    static PLACEHOLDER: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\{[^}]*\}").expect("fixed regex"));
    for (path, raw) in entries(&doc["paths"]) {
        let item = r.resolve(raw);
        for method in ["get", "post", "put", "patch", "delete", "head", "options"] {
            let op = &item[method];
            if !op.is_object() {
                continue;
            }
            let fallback = PLACEHOLDER
                .replace_all(path, "by")
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>();
            let raw_id = s(&op["operationId"]);
            let base = py_ident(&if raw_id.is_empty() { format!("{method}_{fallback}") } else { raw_id });
            let id = unique(&base, &mut used_ids, "_");
            let mut by_key: Map<String, Value> = Map::new();
            for param in arr(&item["parameters"]).iter().chain(arr(&op["parameters"])) {
                let p = r.resolve(param);
                if p.is_object() {
                    by_key.insert(format!("{}:{}", s(&p["in"]), s(&p["name"])), p);
                }
            }
            let mut params = Vec::new();
            let mut body = None;
            let mut used = HashSet::new();
            let mut form = false;
            for (_, p) in by_key {
                let location = s(&p["in"]);
                if location == "body" {
                    body = Some(Body { ty: r.type_of(&p["schema"], 0), required: p["required"] == true, form: false });
                    continue;
                }
                if location == "formData" {
                    form = true;
                    continue;
                }
                if !["path", "query", "header"].contains(&location.as_str()) {
                    continue;
                }
                let name = s(&p["name"]);
                let py = unique(&py_ident(&name), &mut used, "_");
                let ty = r.type_of(if v3 { &p["schema"] } else { &p }, 0);
                params.push(Param {
                    field: Field {
                        name,
                        py,
                        ty,
                        required: location == "path" || p["required"] == true,
                        description: s(&p["description"]),
                    },
                    location,
                });
            }
            if form {
                body = Some(Body { ty: TypeRef::Dict { of: Box::new(TypeRef::Any) }, required: true, form: true });
            }
            if v3 && !op["requestBody"].is_null() {
                let rb = r.resolve(&op["requestBody"]);
                if rb["content"].is_object() {
                    let json = entries(&rb["content"]).find(|(k, _)| k.to_lowercase().contains("json"));
                    let form =
                        entries(&rb["content"]).find(|(k, _)| k.to_lowercase().contains("x-www-form-urlencoded"));
                    let chosen = json.or(form).or_else(|| entries(&rb["content"]).next());
                    body = Some(Body {
                        ty: r.type_of(chosen.map(|(_, v)| &v["schema"]).unwrap_or(&Value::Null), 0),
                        required: rb["required"] == true,
                        form: json.is_none() && form.is_some(),
                    });
                }
            }
            let mut success: Vec<_> = entries(&op["responses"]).filter(|(k, _)| k.starts_with('2')).collect();
            success.sort_by_key(|(key, _)| *key);
            let chosen = success.first().map(|(_, v)| *v).or_else(|| op["responses"].get("default"));
            let mut response = None;
            if let Some(raw) = chosen {
                let resp = r.resolve(raw);
                if v3 {
                    if let Some((_, media)) = entries(&resp["content"]).find(|(k, _)| k.to_lowercase().contains("json"))
                    {
                        response = Some(r.type_of(&media["schema"], 0));
                    }
                } else if !resp["schema"].is_null() {
                    response = Some(r.type_of(&resp["schema"], 0));
                }
            }
            if body.is_some() {
                for p in &mut params {
                    if p.field.py == "body" {
                        p.field.py = unique("body", &mut used, "_");
                    }
                }
            }
            let summary = op["summary"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| op["description"].as_str().unwrap_or("").split('\n').next().unwrap_or(""));
            operations.push(Operation {
                id,
                method: method.to_uppercase(),
                path: path.clone(),
                summary: summary.into(),
                tag: op["tags"][0].as_str().unwrap_or("other").into(),
                params,
                body,
                response,
            });
        }
    }
    if operations.is_empty() {
        r.warnings.push("No endpoints found in it.".into())
    }
    let mut seen = HashSet::new();
    r.warnings.retain(|w| seen.insert(w.clone()));
    Ok(ApiSpec {
        title,
        version: s(&doc["info"]["version"]),
        description: doc["info"]["description"].as_str().unwrap_or("").split('\n').next().unwrap_or("").into(),
        base_url,
        slug,
        env_prefix,
        auth,
        operations,
        models: r.models,
        warnings: r.warnings,
    })
}
pub fn env_keys(spec: &ApiSpec) -> Vec<String> {
    let mut keys = vec![format!("{}_BASE_URL", spec.env_prefix)];
    for a in &spec.auth {
        match a {
            AuthScheme::ApiKey { env, .. } | AuthScheme::Bearer { env } => keys.push(env.clone()),
            AuthScheme::Basic { user_env, pass_env } => {
                keys.push(user_env.clone());
                keys.push(pass_env.clone());
            }
        }
    }
    keys
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_specs_match_all_models_operations_auth_and_server_rules() {
        let cases: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/openapi.json")).unwrap();
        for c in cases.as_array().unwrap() {
            let spec = read_spec(&c["doc"], c["from"].as_str().unwrap()).unwrap();
            assert_eq!(serde_json::to_value(&spec).unwrap(), c["spec"]);
        }
    }
    #[test]
    fn python_names_and_invalid_descriptions() {
        for (raw, expected) in [
            ("petId", "pet_id"),
            ("photo-urls", "photo_urls"),
            ("class", "class_"),
            ("HTTPStatus", "http_status"),
            ("$$", "value"),
        ] {
            assert_eq!(py_ident(raw), expected);
        }
        assert_eq!(py_class("user-profile"), "UserProfile");
        assert_eq!(py_class("2fa"), "M2fa");
        assert!(read_spec(&serde_json::json!({"hello":1}), "").is_err());
        assert!(read_spec(&serde_json::json!("x"), "").is_err());
    }
    #[test]
    fn request_body_does_not_collide_with_a_parameter_named_body() {
        let spec=read_spec(&serde_json::json!({"openapi":"3.0.0","paths":{"/send":{"post":{"operationId":"send","parameters":[{"name":"body","in":"query","schema":{"type":"string"}}],"requestBody":{"content":{"application/json":{"schema":{"type":"string"}}}}}}}}),"").unwrap();
        assert_ne!(spec.operations[0].params[0].field.py, "body");
    }
    #[test]
    fn recursive_all_of_and_self_references_are_bounded() {
        let spec=read_spec(&serde_json::json!({"openapi":"3.0.0","components":{"schemas":{"Loop":{"allOf":[{"$ref":"#/components/schemas/Loop"},{"$ref":"#/components/schemas/Loop"}]},"Node":{"properties":{"next":{"$ref":"#/components/schemas/Node"}}}}}}),"").unwrap();
        assert_eq!(spec.models.len(), 2);
        assert!(spec.warnings.iter().any(|s| s.contains("recursive")));
    }
}
