use forge_core::{blocks::*, pipeline_edit::*};
use std::{collections::BTreeSet, path::Path, time::Duration};

#[test]
fn project_blocks_keep_type_hints_and_only_expose_marked_or_public_block_functions() {
    use forge_core::project_graph::{ModuleNode, ProjectGraph, RawParamSig, SymbolNode};
    let symbol = |name: &str, marked: bool| SymbolNode {
        name: name.into(),
        qual: name.into(),
        kind: "function".into(),
        decorators: if marked { vec!["tools.block".into()] } else { Vec::new() },
        line: 3,
        params: Some(vec![RawParamSig { name: "df".into(), annotation: Some("pd.DataFrame".into()), default: None }]),
        returns: Some("pd.DataFrame".into()),
        doc: Some("Tidy up.".into()),
        ..Default::default()
    };
    let graph = ProjectGraph {
        modules: vec![
            ModuleNode {
                id: "app.steps".into(),
                path: "app/steps.py".into(),
                symbols: vec![symbol("clean", true), symbol("other", false)],
                ..Default::default()
            },
            ModuleNode {
                id: "blocks".into(),
                path: "blocks.py".into(),
                symbols: vec![symbol("save_report", false), symbol("_helper", false)],
                ..Default::default()
            },
            ModuleNode { id: "forge_blocks".into(), symbols: vec![symbol("read_csv", true)], ..Default::default() },
        ],
        ..Default::default()
    };
    let blocks = project_blocks(&graph);
    assert_eq!(
        blocks.iter().map(|b| format!("{}.{}:{}", b.module, b.name, b.kind)).collect::<Vec<_>>(),
        ["app.steps.clean:transform", "blocks.save_report:save"]
    );
    assert!(is_data_param(&blocks[0].params[0], 0));
    assert_eq!(blocks[0].returns.as_deref(), Some("pd.DataFrame"));
    assert_eq!(blocks[0].path.as_deref(), Some("app/steps.py"));
    let params = parse_params("a: dict[str, str], b: str = 'x,y', *args: Any");
    assert_eq!(params.len(), 3);
    assert_eq!(params[1].default.as_deref(), Some("'x,y'"));
    assert!(!is_data_param(&params[2], 2));
}
#[test]
fn builtin_parameters_calls_settings_and_project_copies_match_the_original() {
    let blocks = builtin_blocks();
    assert_eq!(blocks.len(), 15);
    let get = |name: &str| blocks.iter().find(|b| b.name == name).unwrap();
    assert_eq!(block_call(get("read_csv"), &[], &BTreeSet::new()).0, "data = read_csv(path=\"\")");
    assert_eq!(
        block_call(get("filter_rows"), &["data".into()], &BTreeSet::from(["data".into()])).0,
        "filtered = filter_rows(data, condition=\"\")"
    );
    assert_eq!(
        block_call(get("merge"), &["a".into(), "b".into()], &BTreeSet::from(["merged".into()])).0,
        "merged_2 = merge(a, b, on=[])"
    );
    assert_eq!(placeholder(Some("dict[str, str]")), "{}");
    assert_eq!(placeholder(Some("Optional[str]")), "\"\"");
    let first = copy_builtin_to("", "filter_rows").unwrap();
    assert_eq!(copy_builtin_to(&first, "filter_rows").unwrap(), first);
    assert_eq!(copy_builtin_to(&first, "merge").unwrap().matches("def ").count(), 2);
    assert_eq!(
        import_from("from forge_blocks import read_csv\nx = 1\n", "forge_blocks", "save_csv"),
        "from forge_blocks import read_csv, save_csv\nx = 1\n"
    );
}
#[test]
fn builds_rewires_loops_and_exports_real_python() {
    let Some(python) = forge_run::python::find_python(Path::new("."), None) else {
        return;
    };
    let read =
        |text: &str| forge_scan::run_dataflow(&python, Path::new("."), text, "sales", Duration::from_secs(20)).unwrap();
    let mut text = new_pipeline("import os\n", "sales");
    let blocks = builtin_blocks();
    let mut last = Vec::new();
    for name in ["read_csv", "filter_rows", "group_by", "save_csv"] {
        let raw = read(&text);
        assert!(raw.error.is_none(), "{:?}", raw.error);
        let taken = raw.steps.as_ref().unwrap().iter().flat_map(|s| s.targets.clone()).collect();
        let block = blocks.iter().find(|b| b.name == name).unwrap();
        let (code, var) = block_call(block, &last, &taken);
        text = import_from(&insert_in_function(&text, &raw, &[code], None), "forge_blocks", name);
        if let Some(var) = var {
            last = vec![var];
        }
    }
    let raw = read(&text);
    let save =
        raw.steps.as_ref().unwrap().iter().find(|s| s.call.as_ref().is_some_and(|c| c.name == "save_csv")).unwrap();
    text = set_argument(&text, save, 0, "filtered");
    assert!(text.contains("save_csv(filtered"));
    let raw = read(&text);
    let (code, _) =
        for_each_code(blocks.iter().find(|b| b.name == "fill_missing").unwrap(), "grouped", &BTreeSet::new(), true);
    text = insert_in_function(&text, &raw, &code, None);
    assert!(read(&text).error.is_none());
    assert!(text.contains("for item in grouped.to_dict(\"records\"):"));
    let exported = export_standalone(&text, "sales").unwrap();
    assert!(!exported.contains("from forge_blocks import"));
    assert!(exported.contains("def group_by("));
    assert!(!exported.contains("def _records_table("));
    assert!(read(&exported).error.is_none());
    assert!(export_standalone("from forge_blocks import call_api\ndef p():\n    x = call_api('u')\n", "p")
        .unwrap()
        .contains("def _records_table("));
}
