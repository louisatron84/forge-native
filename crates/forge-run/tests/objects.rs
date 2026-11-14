use std::time::Duration;
#[test]
fn original_python_object_helper_preserves_aliases_cycles_fields_and_limits() {
    let dir = tempfile::tempdir().unwrap();
    let Some(python) = forge_run::python::find_python(dir.path(), None) else {
        return;
    };
    let source=format!("{}\nclass Node:\n    def __init__(self, value):\n        self.value=value\nhead=Node(1)\ntail=Node(2)\nhead.next=tail\nitems=[head,tail]\na=[]\na.append(a)\nbig=[[i] for i in range(100)]\nprint(forge_objects(locals()))\n",forge_core::execution::OBJECT_HELPER);
    let result =
        forge_sys::process::run_captured(&python, &["-c".into(), source], dir.path(), &[], "", Duration::from_secs(20));
    assert!(result.ok(), "{result:?}");
    let g = forge_core::execution::parse_object_graph(&result.stdout).unwrap();
    assert!(g.nodes.len() <= 40);
    assert_eq!(g.nodes.iter().filter(|n| n.type_ == "Node").count(), 2);
    let root = |name: &str| g.roots.iter().find(|(n, _)| n == name).unwrap().1.clone();
    assert!(g.edges.contains(&(root("head"), root("tail"), "next".into())));
    assert!(g.edges.contains(&(root("items"), root("head"), "[0]".into())));
    assert!(g.edges.contains(&(root("a"), root("a"), "[0]".into())));
}
