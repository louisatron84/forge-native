use forge_core::dataflow::*;
use serde_json::json;
#[test]
fn same_line_steps_get_unique_ids_and_reassignments_use_the_previous_definition() {
    let raw: RawFlow = serde_json::from_value(json!({"func":"f","params":[],"steps":[
        {"line":2,"kind":"assign","code":"x = 1","targets":["x"]},
        {"line":2,"kind":"assign","code":"x = x + 1","targets":["x"],"uses":["x","x"]},
        {"line":3,"kind":"return","code":"return x","uses":["x"]}
    ]}))
    .unwrap();
    let flow = build_flow(&raw).unwrap();
    assert_eq!(flow.nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["s2", "s2_2", "s3"]);
    assert_eq!(flow.edges.len(), 2);
    assert_eq!(flow.origins["s2_2"]["x"], "s2");
    assert_eq!(flow.origins["s3"]["x"], "s2_2");
}
#[test]
fn guesses_pipeline_name_else_last_block_function_else_first() {
    let functions = |names: &[&str]| {
        names.iter().map(|s| Function { qual: s.to_string(), ..Default::default() }).collect::<Vec<_>>()
    };
    assert_eq!(guess_pipeline("", &functions(&["load", "sales_pipeline", "save"])), Some("sales_pipeline"));
    assert_eq!(guess_pipeline("from forge_blocks import read_csv", &functions(&["helper", "main"])), Some("main"));
    assert_eq!(guess_pipeline("", &functions(&["a", "A.m"])), Some("a"));
    assert_eq!(guess_pipeline("", &[]), None);
    assert_eq!(guess_pipeline("", &functions(&["A.pipeline", "load", "etl_job"])), Some("etl_job"));
    assert_eq!(guess_pipeline("@block", &functions(&["helper", "process_report", "A.run"])), Some("process_report"));
    assert_eq!(guess_pipeline("from forge_blocks import x", &functions(&["helper", "main", "A.run"])), Some("main"));
}
#[test]
fn dependency_walk_terminates_on_cycles_and_handles_long_chains() {
    let raw = RawFlow {
        func: Some("f".into()),
        steps: Some(
            (1..=5000)
                .map(|line| Step {
                    line,
                    end_line: line,
                    kind: "assign".into(),
                    targets: vec!["x".into()],
                    uses: vec!["x".into()],
                    ..Default::default()
                })
                .collect(),
        ),
        ..Default::default()
    };
    let mut flow = build_flow(&raw).unwrap();
    assert_eq!(dependencies(&flow, "s1", Direction::Down).len(), 4999);
    flow.edges.push(FlowEdge { from: "s5000".into(), to: "s1".into(), name: "x".into() });
    assert_eq!(dependencies(&flow, "s1", Direction::Down).len(), 5000);
}
