//! Port of Electron main/eventScan.test.ts against the unchanged AST helper.
use forge_core::{
    graph_view::{build_view, GraphMode, ViewOptions},
    project_graph::build_graph,
};
use std::{collections::BTreeMap, time::Duration};
#[test]
fn real_async_signal_tk_task_and_thread_flows_reach_the_events_view_without_importing_code() {
    let dir = tempfile::tempdir().unwrap();
    let Some(python) = forge_run::python::find_python(dir.path(), None) else {
        return;
    };
    let files: BTreeMap<String, String> = serde_json::from_str(include_str!("fixtures/event_files.json")).unwrap();
    for (path, text) in files {
        let p = dir.path().join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let raw = forge_scan::run_scan(&python, dir.path(), &BTreeMap::new(), Duration::from_secs(30)).unwrap();
    let graph = build_graph(&raw);
    assert_eq!(forge_core::navigation::module_for_path(&graph, "chat\\server.py"), Some("chat.server"));
    assert_eq!(forge_core::navigation::neighbourhood(&graph, "chat.server").imports, ["chat.store"]);
    assert!(forge_core::navigation::file_summary(&graph, "chat.server")
        .iter()
        .any(|s| s.qual == "on_message" && s.summary.contains("chat.store")));
    let uses = |kind: &str| {
        let mut rows = graph
            .events
            .iter()
            .filter(|e| e.kind == kind)
            .map(|e| {
                format!(
                    "{} {}{}",
                    e.event.as_deref().unwrap_or(""),
                    e.func,
                    e.target.as_ref().map(|t| format!(" → {t}")).unwrap_or_default()
                )
            })
            .collect::<Vec<_>>();
        rows.sort();
        rows
    };
    assert_eq!(
        uses("handles"),
        [
            "<Key> gui:on_key",
            "Button command gui:clicked",
            "message chat.server:on_message",
            "post_save chat.server:audit",
            "reply chat.server:on_reply"
        ]
    );
    assert_eq!(uses("emits"), ["<Key> module:gui", "reply chat.server:on_message"]);
    assert_eq!(uses("spawns"), [" chat.server:main → chat.server:heartbeat", " chat.server:main → chat.server:worker"]);
    let view = build_view(&graph, &ViewOptions { mode: GraphMode::Events, ..Default::default() });
    let edges = view.edges.iter().map(|e| format!("{} {} {}", e.from, e.kind, e.to)).collect::<Vec<_>>();
    for expected in [
        "chat.server:on_message emits event:reply",
        "event:reply handles chat.server:on_reply",
        "event:message handles chat.server:on_message",
        "chat.server:on_message call chat.store:save_message",
        "chat.server:main spawns chat.server:heartbeat",
    ] {
        assert!(edges.iter().any(|e| e == expected), "Missing {expected}");
    }
    let message = view.nodes.iter().find(|n| n.id == "event:message").unwrap();
    assert_eq!(message.detail, "event · from outside the code · 1 handler");
    assert!(!view.nodes.iter().any(|n| n.id == "plain:add"));
}
