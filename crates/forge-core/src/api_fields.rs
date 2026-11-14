//! Response record selection, field trees and matching CSV previews,
//! replacing `src/shared/apiFields.ts`. Samples are memory-only.
use crate::boxes::FieldNode;
use serde_json::Value;
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq)]
pub struct RecordList {
    pub path: Vec<String>,
    pub count: usize,
}
pub fn record_lists(json: &Value) -> Vec<RecordList> {
    fn walk(value: &Value, path: &mut Vec<String>, out: &mut Vec<RecordList>) {
        if let Some(array) = value.as_array().filter(|a| !a.is_empty() && a.iter().all(Value::is_object)) {
            out.push(RecordList { path: path.clone(), count: array.len() });
        } else if path.len() < 4 {
            if let Some(object) = value.as_object() {
                for (k, v) in object {
                    path.push(k.clone());
                    walk(v, path, out);
                    path.pop();
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(json, &mut Vec::new(), &mut out);
    out.sort_by_key(|r| std::cmp::Reverse(r.count));
    out
}
fn at<'a>(json: &'a Value, path: &[String]) -> Option<&'a Value> {
    let mut value = json;
    for part in path {
        value = value.as_object()?.get(part)?;
    }
    Some(value)
}
fn record(value: &Value) -> Value {
    if value.is_object() {
        value.clone()
    } else {
        serde_json::json!({"value":value})
    }
}
pub fn pick_records(json: &Value, path: Option<&[String]>) -> (Vec<Value>, Option<Vec<String>>) {
    if let Some(path) = path {
        if let Some(array) = at(json, path).and_then(Value::as_array) {
            return (array.iter().map(record).collect(), Some(path.to_vec()));
        }
    }
    if let Some(array) = json.as_array() {
        return (array.iter().map(record).collect(), Some(Vec::new()));
    }
    if let Some(list) = record_lists(json).into_iter().next() {
        if let Some(array) = at(json, &list.path).and_then(Value::as_array) {
            return (array.iter().map(record).collect(), Some(list.path));
        }
    }
    (vec![record(json)], None)
}
pub fn py_json(json: &Value) -> String {
    match json {
        Value::String(s) => crate::box_code::s(s),
        Value::Array(a) => format!("[{}]", a.iter().map(py_json).collect::<Vec<_>>().join(", ")),
        Value::Object(o) => format!(
            "{{{}}}",
            o.iter().map(|(k, v)| format!("{}: {}", crate::box_code::s(k), py_json(v))).collect::<Vec<_>>().join(", ")
        ),
        Value::Number(n) => {
            n.as_f64().filter(|n| n.fract() == 0.0).map(|n| format!("{n:.0}")).unwrap_or_else(|| n.to_string())
        }
        _ => json.to_string(),
    }
}
pub fn field_tree(records: &[Value]) -> Vec<FieldNode> {
    fn merge(into: &mut Vec<FieldNode>, rec: &serde_json::Map<String, Value>, base: &[String]) {
        for (name, value) in rec {
            let index = into.iter().position(|n| &n.name == name).unwrap_or_else(|| {
                let mut path = base.to_vec();
                path.push(name.clone());
                into.push(FieldNode { path, name: name.clone(), kind: "value".into(), ..Default::default() });
                into.len() - 1
            });
            if value.is_null() {
                continue;
            }
            let node = &mut into[index];
            let known = !node.sample.is_empty() || !node.children.is_empty() || node.kind != "value";
            if let Some(object) = value.as_object().filter(|_| base.len() + 1 < 4) {
                if !known {
                    node.kind = "object".into();
                }
                if node.kind == "object" {
                    merge(&mut node.children, object, &node.path);
                }
            } else if !known {
                node.kind = if value.is_array() { "list" } else { "value" }.into();
                let sample = value.as_str().map(str::to_owned).unwrap_or_else(|| py_json(value));
                let mut units = 0;
                let shortened: String = sample
                    .chars()
                    .take_while(|c| {
                        units += c.len_utf16();
                        units <= 59
                    })
                    .collect();
                node.sample = if sample.encode_utf16().count() > 60 { shortened + "…" } else { sample };
            }
        }
    }
    let mut tree = Vec::new();
    for record in records {
        if let Some(object) = record.as_object() {
            merge(&mut tree, object, &[]);
        }
    }
    tree
}
pub fn default_ticks(tree: &[FieldNode]) -> Vec<String> {
    tree.iter().filter(|f| f.kind != "object").map(|f| f.path.join(".")).collect()
}
pub fn rows_for(records: &[Value], ticked: &[String]) -> Vec<BTreeMap<String, String>> {
    records
        .iter()
        .map(|r| {
            ticked
                .iter()
                .map(|key| {
                    let path: Vec<_> = key.split('.').map(str::to_owned).collect();
                    let value = at(r, &path);
                    let cell = match value {
                        None | Some(Value::Null) => String::new(),
                        Some(Value::String(text)) => text.clone(),
                        Some(other) => py_json(other),
                    };
                    (key.clone(), cell)
                })
                .collect()
        })
        .collect()
}
pub fn csv_preview(rows: &[BTreeMap<String, String>], columns: &[String], max: usize) -> String {
    fn cell(text: &str) -> String {
        if text.contains([',', '"', '\r', '\n']) {
            format!("\"{}\"", text.replace('"', "\"\""))
        } else {
            text.into()
        }
    }
    let mut out = columns.iter().map(|c| cell(c)).collect::<Vec<_>>().join(",") + "\r\n";
    for row in rows.iter().take(max) {
        out.push_str(
            &columns.iter().map(|c| cell(row.get(c).map(String::as_str).unwrap_or(""))).collect::<Vec<_>>().join(","),
        );
        out.push_str("\r\n");
    }
    out
}
fn sample_records<'a>(json: &'a Value, path: Option<&[String]>) -> &'a [Value] {
    let selected = path.and_then(|p| at(json, p)).and_then(Value::as_array).or_else(|| json.as_array());
    let lists = if selected.is_none() { record_lists(json) } else { Vec::new() };
    selected
        .or_else(|| lists.first().and_then(|list| at(json, &list.path)).and_then(Value::as_array))
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(json))
}
fn sample_cell(record: &Value, path: &[String]) -> String {
    let value = if record.is_object() {
        at(record, path)
    } else if path.first().is_some_and(|s| s == "value") {
        if path.len() == 1 {
            Some(record)
        } else {
            at(record, &path[1..])
        }
    } else {
        None
    };
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(value) => py_json(value),
    }
}
/// Return the selected columns for a bounded UI preview without cloning every
/// record or parsing the generated CSV back into cells.
pub fn table_from_sample(
    json: &Value,
    path: Option<&[String]>,
    columns: &[String],
    max: usize,
) -> (Vec<Vec<String>>, usize) {
    let records = sample_records(json, path);
    let paths = columns.iter().map(|key| key.split('.').map(str::to_owned).collect::<Vec<_>>()).collect::<Vec<_>>();
    let rows =
        records.iter().take(max).map(|record| paths.iter().map(|path| sample_cell(record, path)).collect()).collect();
    (rows, records.len())
}
/// Stream selected records directly into CSV, without cloning the response or
/// materialising a map for every row. `max` also serves the ten-row UI preview.
pub fn csv_from_sample(json: &Value, path: Option<&[String]>, columns: &[String], max: usize) -> (String, usize) {
    let records = sample_records(json, path);
    fn cell(text: &str) -> String {
        if text.contains([',', '"', '\r', '\n']) {
            format!("\"{}\"", text.replace('"', "\"\""))
        } else {
            text.into()
        }
    }
    let paths = columns.iter().map(|key| key.split('.').map(str::to_owned).collect::<Vec<_>>()).collect::<Vec<_>>();
    let mut csv = columns.iter().map(|c| cell(c)).collect::<Vec<_>>().join(",") + "\r\n";
    for record in records.iter().take(max) {
        for (i, path) in paths.iter().enumerate() {
            if i > 0 {
                csv.push(',')
            }
            csv.push_str(&cell(&sample_cell(record, path)));
        }
        csv.push_str("\r\n");
    }
    (csv, records.len())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streamed_csv_matches_record_selection_quoting_and_nested_scalar_values() {
        for sample in [
            serde_json::json!({"users":[{"id":1,"name":"a,b","plan":{"name":"pro"}},{"id":2,"name":"line\nquote\""}]}),
            serde_json::json!([true, null, 3, "a,b", [1, 2]]),
            serde_json::json!({"id":1}),
            serde_json::json!(null),
        ] {
            let columns = vec!["id".into(), "name".into(), "plan.name".into(), "value".into()];
            let (records, path) = pick_records(&sample, None);
            let expected = csv_preview(&rows_for(&records, &columns), &columns, usize::MAX);
            let (actual, count) = csv_from_sample(&sample, path.as_deref(), &columns, usize::MAX);
            assert_eq!(actual, expected);
            assert_eq!(count, records.len());
            let (preview, _) = csv_from_sample(&sample, path.as_deref(), &columns, 1);
            assert_eq!(preview, csv_preview(&rows_for(&records, &columns), &columns, 1));
        }
    }
    #[test]
    fn table_preview_matches_csv_cells_and_is_bounded() {
        let sample = json!({"users":[
            {"id":1,"name":"Leanne","address":{"city":"Gwenborough"}},
            {"id":2,"name":"Ervin","address":{"city":"Wisokyburgh"}}
        ]});
        let columns = vec!["id".into(), "name".into(), "address.city".into()];
        let path = vec!["users".into()];
        let (rows, count) = table_from_sample(&sample, Some(&path), &columns, 1);
        assert_eq!(count, 2);
        assert_eq!(rows, vec![vec!["1", "Leanne", "Gwenborough"]]);
    }
    use serde_json::json;
    fn fixture() -> Value {
        json!({"items":[{"a":1},{"a":2,"b":{"c":"x"}}],"meta":{"n":2}})
    }
    #[test]
    fn lists_of_objects_largest_first() {
        assert_eq!(record_lists(&fixture()), vec![RecordList { path: vec!["items".into()], count: 2 }]);
        assert_eq!(record_lists(&json!({"x":{"big":[{},{},{}]},"small":[{}]}))[0].count, 3);
        assert!(record_lists(&json!({"nums":[1,2]})).is_empty());
    }
    #[test]
    fn picks_largest_chosen_bare_object_and_plain_values() {
        let (rows, path) = pick_records(&fixture(), None);
        assert_eq!(path, Some(vec!["items".into()]));
        assert_eq!(rows.len(), 2);
        let (rows, path) = pick_records(&json!([1, 2]), None);
        assert_eq!(rows, vec![json!({"value":1}), json!({"value":2})]);
        assert_eq!(path, Some(Vec::new()));
        assert_eq!(pick_records(&json!("hello"), None).0, vec![json!({"value":"hello"})]);
        assert!(pick_records(&json!([]), None).0.is_empty());
    }
    #[test]
    fn merges_all_records_and_nests_objects() {
        let (records, _) = pick_records(&fixture(), None);
        let tree = field_tree(&records);
        assert_eq!(
            tree.iter().map(|n| (n.name.as_str(), n.kind.as_str(), n.sample.as_str())).collect::<Vec<_>>(),
            [("a", "value", "1"), ("b", "object", "")]
        );
        assert_eq!(tree[1].children[0].path, ["b", "c"]);
        assert_eq!(default_ticks(&tree), ["a"]);
    }
    #[test]
    fn initially_null_fields_take_their_kind_from_later_records() {
        let tree = field_tree(&[json!({"p":null}), json!({"p":{"q":1}})]);
        assert_eq!(tree[0].kind, "object");
        assert_eq!(tree[0].children[0].name, "q");
    }
    #[test]
    fn lists_are_single_fields_samples_are_short() {
        let tree = field_tree(&[json!({"tags":["a","b"],"long":"x".repeat(100)})]);
        assert_eq!(tree[0].kind, "list");
        assert_eq!(tree[0].sample, "[\"a\", \"b\"]");
        assert!(tree[1].sample.len() <= 62);
        assert_eq!(default_ticks(&field_tree(&[json!({"tags":[],"o":{"x":1}})])), ["tags"]);
    }
    #[test]
    fn ticked_fields_become_cells_with_python_json_defaults() {
        let (records, _) = pick_records(&fixture(), None);
        let columns = vec!["a".into(), "b.c".into()];
        let rows = rows_for(&records, &columns);
        assert_eq!(rows[0]["b.c"], "");
        assert_eq!(rows[1]["b.c"], "x");
        assert_eq!(py_json(&json!({"a":[1,"x",null,false],"b":{}})), "{\"a\": [1, \"x\", null, false], \"b\": {}}");
        let columns = vec!["n".into(), "t".into(), "l".into(), "o".into()];
        let rows = rows_for(&[json!({"n":null,"t":true,"l":["a","b"],"o":{"k":"é"}})], &columns);
        assert_eq!(rows[0]["n"], "");
        assert_eq!(rows[0]["t"], "true");
        assert_eq!(rows[0]["o"], "{\"k\": \"\\u00e9\"}");
    }
    #[test]
    fn csv_quotes_like_python_and_uses_crlf() {
        let rows = vec![
            BTreeMap::from([("a".into(), "a,b".into()), ("b".into(), "say \"hi\"".into())]),
            BTreeMap::from([("a".into(), "plain".into()), ("b".into(), "two\nlines".into())]),
        ];
        assert_eq!(
            csv_preview(&rows, &["a".into(), "b".into()], 5),
            "a,b\r\n\"a,b\",\"say \"\"hi\"\"\"\r\nplain,\"two\nlines\"\r\n"
        );
    }
}
