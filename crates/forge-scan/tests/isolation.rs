//! Static inspection must not import files supplied by the inspected project.
use std::{collections::BTreeMap, time::Duration};

#[test]
fn static_helpers_ignore_project_modules_that_shadow_python_libraries() {
    let dir = tempfile::tempdir().unwrap();
    let Some(python) = forge_run::python::find_python(dir.path(), None) else { return };
    std::fs::write(
        dir.path().join("ast.py"),
        "open('executed.txt', 'w').write('project code ran')\nraise RuntimeError('shadowed ast')\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("main.py"), "def work():\n    return 1\n").unwrap();
    let scan = forge_scan::run_scan(&python, dir.path(), &BTreeMap::new(), Duration::from_secs(10));
    let flow =
        forge_scan::run_dataflow(&python, dir.path(), "def work():\n    return 1\n", "work", Duration::from_secs(10));
    assert!(!dir.path().join("executed.txt").exists(), "static inspection executed project ast.py");
    assert!(scan.is_ok(), "{scan:?}");
    assert!(flow.is_ok(), "{flow:?}");
}
