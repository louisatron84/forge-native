use forge_core::execution::*;
use serde_json::json;
use std::collections::BTreeMap;
#[test]
fn recursion_parents_layout_and_run_store() {
    let raw = json!({"t":"rec","tree":{"key":"k","func":"a:f","nodes":[[-1,"r",null],[0,"a",null],[0,"b",null],[1,"c",null]]}});
    let tree = parse_rec(&raw).unwrap();
    let layout = tree_layout(&tree);
    assert_eq!(layout.positions, vec![(0.5, 0), (0., 1), (1., 1), (0., 2)]);
    assert_eq!((layout.columns, layout.depth), (2, 3));
    for nodes in [json!([]), json!([[0, "x", null]]), json!([[-1, "x", null], [5, "x", null]])] {
        assert!(parse_rec(&json!({"t":"rec","tree":{"key":"k","func":"f","nodes":nodes}})).is_none());
    }
}
#[test]
fn debug_trail_calls_history_changes_comparison_and_retention() {
    let mut trail = DebugTrail::default();
    let vars =
        |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>();
    trail.add("breakpoint".into(), vec!["a:f".into(), "module:a".into()], vars(&[("i", "0"), ("total", "0")]));
    trail.add("step".into(), vec!["a:g".into(), "a:f".into(), "module:a".into()], vars(&[("x", "1")]));
    trail.add("step".into(), vec!["a:f".into()], vars(&[("i", "1"), ("total", "0"), ("last", "3")]));
    let h = trail.history("a:f");
    assert_eq!(h.stops, [1, 3]);
    assert_eq!(h.rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["i", "total", "last"]);
    assert_eq!(h.rows[0].changed, [false, true]);
    let diff = diff_vars(&trail.stops[0].vars, &trail.stops[2].vars);
    assert_eq!(diff.changed, ["i"]);
    assert_eq!(diff.added, ["last"]);
    assert_eq!(trail.edges[&("module:a".into(), "a:f".into())], 2);
    for _ in 0..250 {
        trail.add("step".into(), Vec::new(), BTreeMap::new());
    }
    assert_eq!(trail.stops.len(), 200);
}
#[test]
fn object_graph_unrepr_filters_unknown_nodes_and_handles_escapes() {
    assert_eq!(unrepr_string("'hello\\nworld\\u00e9'"), "hello\nworldé");
    let g=parse_object_graph(r#"'{"roots":[["a","1"],["bad","9"]],"nodes":[{"id":"1","type":"list","fields":[["x","1"]],"more":0}],"edges":[["1","1","cycle"],["1","9","bad"]]}'"#).unwrap();
    assert_eq!(g.roots.len(), 1);
    assert_eq!(g.edges.len(), 1);
}
