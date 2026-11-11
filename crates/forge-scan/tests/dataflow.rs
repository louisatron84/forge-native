//! Cases ported from Electron's main/dataflow.test.ts; unchanged Python helper.
use forge_core::dataflow::{build_flow, dependencies, Direction};
use std::{path::Path, time::Duration};
const SOURCE: &str = "import pandas as pd\nfrom forge_blocks import read_csv, filter_rows, group_by, save_csv\n\ndef sales(path: str, limit=10):\n    \"\"\"Totals by region.\"\"\"\n    orders = read_csv(path)\n    paid = filter_rows(orders, \"status == 'paid'\")\n    totals = group_by(paid, [\"region\"], {\"amount\": \"sum\"})\n    for row in totals.itertuples():\n        print(row.region, [x for x in range(limit)])\n        orders = orders.head(1)\n    save_csv(totals, \"out.csv\")\n    return totals\n\nclass Job:\n    def run(self):\n        return 1";
fn read(source: &str, func: &str) -> Option<forge_core::dataflow::RawFlow> {
    let python = forge_run::python::find_python(Path::new("."), None)?;
    Some(forge_scan::run_dataflow(&python, Path::new("."), source, func, Duration::from_secs(20)).unwrap())
}
#[test]
fn lists_functions_without_running_or_importing_project_code() {
    let Some(raw) = read(SOURCE, "") else { return };
    assert_eq!(raw.functions.iter().map(|f| f.qual.as_str()).collect::<Vec<_>>(), ["sales", "Job.run"]);
    assert!(raw.steps.is_none());
    assert!(read("raise RuntimeError('must not execute')\ndef f():\n    return 1", "f").unwrap().error.is_none());
}
#[test]
fn reads_params_steps_calls_and_comprehension_scope() {
    let Some(raw) = read(SOURCE, "sales") else { return };
    assert_eq!(
        serde_json::to_value(&raw.params).unwrap(),
        serde_json::json!([
            {"name":"path","annotation":"str","default":null}, {"name":"limit","annotation":null,"default":"10"}
        ])
    );
    assert_eq!(raw.doc, "Totals by region.");
    let steps = raw.steps.unwrap();
    assert_eq!(
        steps.iter().map(|s| format!("{}@{}", s.kind, s.line)).collect::<Vec<_>>(),
        ["assign@6", "assign@7", "assign@8", "for@9", "call@12", "return@13"]
    );
    assert_eq!(steps[1].targets, ["paid"]);
    assert_eq!(steps[1].uses, ["filter_rows", "orders"]);
    assert_eq!(
        serde_json::to_value(&steps[1].call).unwrap(),
        serde_json::json!({"name":"filter_rows","args":[{"kw":null,"text":"orders","names":["orders"]},{"kw":null,"text":"\"status == 'paid'\"","names":[]}]})
    );
    assert_eq!(steps[3].body.as_ref().unwrap()[0].uses, ["print", "row", "range", "limit"]);
}
#[test]
fn wires_last_definitions_loop_variables_and_dependencies() {
    let Some(raw) = read(SOURCE, "sales") else { return };
    let flow = build_flow(&raw).unwrap();
    assert_eq!(
        flow.edges.iter().map(|e| format!("{}→{}:{}", e.from, e.to, e.name)).collect::<Vec<_>>(),
        [
            "param:path→s6:path",
            "s6→s7:orders",
            "s7→s8:paid",
            "s8→s9:totals",
            "s9→s10:row",
            "param:limit→s10:limit",
            "s6→s11:orders",
            "s8→s12:totals",
            "s8→s13:totals"
        ]
    );
    assert_eq!(flow.nodes.iter().find(|n| n.id == "s10").unwrap().parent.as_deref(), Some("s9"));
    assert_eq!(flow.origins["s7"]["orders"], "s6");
    assert_eq!(
        dependencies(&flow, "s7", Direction::Down).into_iter().collect::<Vec<_>>(),
        ["s10", "s12", "s13", "s8", "s9"]
    );
    assert_eq!(dependencies(&flow, "s8", Direction::Up).into_iter().collect::<Vec<_>>(), ["param:path", "s6", "s7"]);
}
#[test]
fn methods_drop_self_and_errors_are_reported() {
    let Some(raw) = read(SOURCE, "Job.run") else { return };
    assert_eq!(build_flow(&raw).unwrap().nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["s17"]);
    assert!(read("def (:", "x").unwrap().error.unwrap().contains("SyntaxError"));
    assert!(read(SOURCE, "nope").unwrap().error.unwrap().contains("No function nope"));
}
