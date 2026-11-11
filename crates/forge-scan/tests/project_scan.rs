//! Port of the real-Python cases in `src/main/projectScan.test.ts`.
use forge_core::project_graph::{build_graph, describe_unreachable, ProjectGraph};
use std::{collections::BTreeMap, path::Path, time::Duration};

fn python(root: &Path) -> Option<String> {
    forge_run::python::find_python(root, None)
}
#[test]
fn generated_clients_are_marked_edits_ignore_eol_and_base_addresses_reach_hosts() {
    let dir = tempfile::tempdir().unwrap();
    let Some(python) = python(dir.path()) else { return };
    let spec=forge_core::openapi::read_spec(&serde_json::json!({"swagger":"2.0","info":{"title":"Café Shop","version":"1"},"host":"shop.example.com","basePath":"/api","paths":{"/items":{"get":{"operationId":"listItems","responses":{"200":{"description":"ok"}}}}}}),"").unwrap();
    let code = forge_core::openapi_code::generate_client(&spec, None);
    std::fs::write(dir.path().join("shop_client.py"), &code).unwrap();
    std::fs::write(dir.path().join("edited_client.py"), code.replace('\n', "\r\n") + "# my change\r\n").unwrap();
    std::fs::write(dir.path().join("raw.py"),"import urllib.request\nBASE_URL = 'https://data.example.org/v2'\ndef get():\n    return urllib.request.urlopen(BASE_URL + '/x')\n").unwrap();
    let raw = forge_scan::run_scan(&python, dir.path(), &BTreeMap::new(), Duration::from_secs(30)).unwrap();
    let graph = build_graph(&raw);
    let module = |id: &str| graph.modules.iter().find(|m| m.id == id).unwrap();
    assert_eq!(
        module("shop_client").generated,
        Some(serde_json::json!({"kind":"openapi","slug":"caf_shop","edited":false}))
    );
    assert_eq!(
        module("edited_client").generated,
        Some(serde_json::json!({"kind":"openapi","slug":"caf_shop","edited":true}))
    );
    assert!(module("raw").generated.is_none());
    assert_eq!(
        graph
            .http
            .iter()
            .filter(|h| h.module == "raw")
            .map(|h| format!("{} {} {}", h.raw.library, h.raw.method, h.host.as_deref().unwrap_or_default()))
            .collect::<Vec<_>>(),
        vec!["urllib HTTP data.example.org"]
    );
    assert_eq!(
        graph.http.iter().filter(|h| h.module == "shop_client").filter_map(|h| h.host.as_deref()).collect::<Vec<_>>(),
        vec!["shop.example.com"]
    );
}
fn fixture() -> Option<(tempfile::TempDir, ProjectGraph)> {
    let dir = tempfile::tempdir().unwrap();
    let python = python(dir.path())?;
    let files: BTreeMap<String, String> = serde_json::from_str(include_str!("fixtures/project_files.json")).unwrap();
    for (path, text) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let scan = forge_scan::run_scan(&python, dir.path(), &BTreeMap::new(), Duration::from_secs(60)).unwrap();
    Some((dir, build_graph(&scan)))
}
#[test]
fn finds_every_module_skipping_virtual_environments_with_packages_tests_and_entries() {
    let Some((_dir, g)) = fixture() else {
        return;
    };
    let mut ids: Vec<_> = g.modules.iter().map(|m| m.id.as_str()).collect();
    ids.sort();
    assert_eq!(
        ids,
        [
            "app",
            "app.api",
            "app.api.routes",
            "app.broken",
            "app.main",
            "app.service",
            "app.util",
            "tests.test_dead",
            "tests.test_service"
        ]
    );
    assert!(g.modules.iter().find(|m| m.id == "app").unwrap().is_package);
    assert!(g.modules.iter().find(|m| m.id == "tests.test_service").unwrap().is_test);
    assert!(g.modules.iter().find(|m| m.id == "app.main").unwrap().is_entry);
}
#[test]
fn resolves_absolute_relative_and_submodule_imports_and_external_packages() {
    let Some((_dir, g)) = fixture() else {
        return;
    };
    let mut edges: Vec<_> = g.imports.iter().map(|e| format!("{} -> {}", e.from, e.to)).collect();
    edges.sort();
    assert_eq!(
        edges,
        [
            "app.main -> app.api.routes",
            "app.main -> app.service",
            "app.service -> app.util",
            "app.util -> app.service",
            "tests.test_service -> app.service"
        ]
    );
    assert_eq!(g.externals.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["fastapi", "httpx", "os", "requests"]);
}
#[test]
fn finds_the_import_cycle_and_marks_its_edges() {
    let Some((_dir, g)) = fixture() else {
        return;
    };
    assert_eq!(g.cycles, vec![vec!["app.service", "app.util"]]);
    assert_eq!(g.imports.iter().filter(|e| e.cyclic).count(), 2);
}
#[test]
fn links_calls_to_the_functions_they_reach_without_recursive_self_edges() {
    let Some((_dir, g)) = fixture() else {
        return;
    };
    let edges: Vec<_> = g.calls.iter().map(|c| format!("{} -> {}", c.from, c.to)).collect();
    for edge in [
        "module:app.main -> app.main:main",
        "app.main:main -> app.service:fetch_user",
        "app.service:fetch_user -> app.service:Client",
        "app.service:fetch_user -> app.service:Client.get",
        "app.service:Client.get -> app.service:Client._call",
        "app.service:never_used -> app.util:helper",
        "tests.test_service:test_it -> app.service:fetch_user",
    ] {
        assert!(edges.iter().any(|e| e == edge), "Missing {edge}: {edges:?}");
    }
    assert!(!edges.iter().any(|c| c.contains("recursive -> ")));
}
#[test]
fn reports_functions_nothing_uses_except_tests_routes_and_hooks() {
    let Some((_dir, g)) = fixture() else {
        return;
    };
    let mut unused: Vec<_> = g.unused.iter().map(|s| s.id.as_str()).collect();
    unused.sort();
    assert_eq!(unused, ["app.service:never_used", "app.service:recursive"]);
}
#[test]
fn finds_code_that_can_never_run() {
    let Some((_dir, g)) = fixture() else {
        return;
    };
    let entries: Vec<_> = g
        .unreachable
        .iter()
        .map(|u| {
            format!(
                "{}:{}-{} {} {}",
                u.module,
                u.raw.line,
                u.raw.end_line,
                u.raw.func.as_deref().unwrap_or(""),
                u.raw.reason
            )
        })
        .collect();
    assert_eq!(
        entries,
        [
            "tests.test_dead:3-3 early return",
            "tests.test_dead:7-7 loop continue",
            "tests.test_dead:13-14 both branches",
            "tests.test_dead:17-17 guarded constant"
        ]
    );
    assert_eq!(describe_unreachable(&g.unreachable[0].raw), "line 3, after a return");
    assert_eq!(describe_unreachable(&g.unreachable[2].raw), "lines 13–14, every branch above it stops");
}
#[test]
fn finds_routes_http_hosts_and_environment_variables_and_reports_syntax_errors() {
    let Some((_dir, g)) = fixture() else {
        return;
    };
    assert_eq!(g.routes[0].raw.path, "/users/{id}");
    assert_eq!(g.routes[0].module, "app.api.routes");
    let mut http: Vec<_> = g
        .http
        .iter()
        .map(|h| {
            format!(
                "{} {} {} in {}:{}",
                h.raw.library,
                h.raw.method,
                h.host.as_deref().unwrap_or(""),
                h.module,
                h.raw.func.as_deref().unwrap_or("")
            )
        })
        .collect();
    http.sort();
    assert_eq!(
        http,
        [
            "httpx GET example.org in app.api.routes:read_user",
            "httpx POST hooks.slack.com in app.api.routes:read_user",
            "requests GET api.github.com in app.service:Client._call"
        ]
    );
    assert_eq!(g.env.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["API_KEY", "BASE_URL"]);
    assert!(g.env[1].uses[0].default);
    assert_eq!(g.errors[0].module, "app.broken");
    assert!(g.errors[0].error.contains("SyntaxError"));
}
#[test]
fn overrides_replace_disk_text_and_only_scans_the_requested_files_without_executing_code() {
    let dir = tempfile::tempdir().unwrap();
    let Some(python) = python(dir.path()) else {
        return;
    };
    std::fs::write(dir.path().join("a.py"), "def old_name():\n    pass\n").unwrap();
    std::fs::write(dir.path().join("b.py"), "open('executed', 'w').write('bad')\n").unwrap();
    let overrides = BTreeMap::from([("a.py".into(), "def new_name():\n    pass\n".into())]);
    let scan = forge_scan::run_scan(&python, dir.path(), &overrides, Duration::from_secs(30)).unwrap();
    assert_eq!(
        scan.files.iter().find(|f| f.path == "a.py").unwrap().defs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
        ["<module>", "new_name"]
    );
    assert!(!dir.path().join("executed").exists());
    let overrides = BTreeMap::from([("a.py".into(), "def broken(:".into())]);
    let files =
        forge_scan::run_scan_files(&python, dir.path(), &["a.py".into()], &overrides, Duration::from_secs(30)).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "a.py");
    assert!(files[0].error.as_ref().unwrap().contains("SyntaxError"));
}
