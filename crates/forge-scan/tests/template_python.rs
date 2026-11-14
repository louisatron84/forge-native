#[test]
fn every_builtin_template_renders_valid_python_and_exported_pipeline_runs_without_library() {
    let dir = tempfile::tempdir().unwrap();
    let Some(python) = forge_run::python::find_python(dir.path(), None) else {
        return;
    };
    let files = forge_core::templates::builtin_templates()
        .iter()
        .flat_map(|t| {
            forge_core::templates::render_template(t, "Demo App")
                .into_iter()
                .filter(|f| f.path.ends_with(".py"))
                .map(|f| (format!("{}/{}", t.id, f.path), f.content))
        })
        .collect::<Vec<_>>();
    let result = forge_sys::process::run_captured(
        &python,
        &[
            "-c".into(),
            "import ast,json,sys\nfor name,source in json.load(sys.stdin):\n    ast.parse(source, filename=name)"
                .into(),
        ],
        dir.path(),
        &[],
        &serde_json::to_string(&files).unwrap(),
        std::time::Duration::from_secs(20),
    );
    assert!(result.ok(), "{result:?}");
    let has_pandas = forge_sys::process::run_captured(
        &python,
        &["-c".into(), "import pandas".into()],
        dir.path(),
        &[],
        "",
        std::time::Duration::from_secs(15),
    )
    .ok();
    if !has_pandas {
        eprintln!("Optional pandas unavailable; AST checks ran, skipping exported runtime case.");
        return;
    }
    let source="from forge_blocks import read_csv, filter_rows, group_by, save_csv\ndef sales():\n    data = read_csv('input.csv')\n    paid = filter_rows(data, 'amount >= 10')\n    grouped = group_by(paid, ['region'], {'amount': 'sum'})\n    save_csv(grouped, 'output.csv')\n";
    let exported = forge_core::pipeline_edit::export_standalone(source, "sales").unwrap();
    std::fs::write(dir.path().join("pipeline.py"), exported).unwrap();
    std::fs::write(dir.path().join("input.csv"), "region,amount\nN,10\nN,20\nS,5\n").unwrap();
    let result = forge_sys::process::run_captured(
        &python,
        &["pipeline.py".into()],
        dir.path(),
        &[],
        "",
        std::time::Duration::from_secs(20),
    );
    assert!(result.ok(), "{result:?}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("output.csv")).unwrap().replace("\r\n", "\n"),
        "region,amount\nN,30\n"
    );
    assert!(!dir.path().join("forge_blocks.py").exists());
}
