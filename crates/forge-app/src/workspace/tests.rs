//! Workspace integration tests run only on GPUI's headless platform.
use super::*;
use gpui_kit::{TestAppContext, VisualTestContext};

#[gpui_kit::test]
fn cancelling_close_cancels_a_requested_update_restart(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let project = tempfile::tempdir().unwrap();
    let path = project.path().join("notes.txt");
    std::fs::write(&path, "before").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(project.path().into(), window, cx);
        ws.open_path(path, window, cx);
        let doc = ws.active_doc().unwrap().clone();
        doc.update(cx, |d, cx| {
            d.edit(None, cx, |doc| {
                let forge_text::Doc { buffer, history, .. } = doc;
                history.transact(buffer, forge_text::EditKind::Other, Pos::new(0, 0), |b| {
                    b.insert(Pos::new(0, 0), "changed")
                });
            })
        });
        cx.global_mut::<AppState>().restart_after_update = true;
        assert!(!ws.should_close(window, cx));
    });
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    ws.read_with(cx, |_, cx| assert!(!app_state::state(cx).restart_after_update));
}

#[gpui_kit::test]
fn release_notes_command_and_update_preferences_use_persisted_state(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    ws.update_in(cx, |ws, window, cx| {
        app_state::state(cx)
            .updater
            .save_notes(&forge_update::Release {
                version: "1.0.1".into(),
                notes: "Saved release improvements".into(),
                url: String::new(),
            })
            .unwrap();
        ws.run_command("help.releaseNotes", window, cx);
        assert!(ws.git_inspect.read(cx).contains_text("Saved release improvements"));
        app_state::update_settings(cx, |s| s.check_for_updates = false).unwrap();
        assert!(!app_state::state(cx).user.settings().0.check_for_updates);
        ws.update_status(cx);
        assert!(ws.settings_view.read(cx).update_info.contains(forge_update::CURRENT));
    });
}

#[gpui_kit::test]
fn project_map_keyboard_navigation_and_file_context_panels_follow_the_open_file(cx: &mut TestAppContext) {
    if forge_run::python::find_python(std::path::Path::new("."), None).is_none() {
        return;
    }
    cx.executor().allow_parking();
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("pkg")).unwrap();
    std::fs::write(dir.path().join("pkg/__init__.py"), "").unwrap();
    std::fs::write(dir.path().join("pkg/worker.py"), "def run():\n    return 1\n").unwrap();
    std::fs::write(dir.path().join("main.py"), "from pkg.worker import run\ndef main():\n    return run()\nmain()\n")
        .unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.open_path(dir.path().join("main.py"), window, cx);
        ws.run_command("nav.projectMap", window, cx);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| {
            ws.project_graph.read(cx).project_map.read(cx).files.iter().any(|(p, _)| p == "pkg/worker.py")
        }) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Project map command is not connected to analysis");
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(40));
    }
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.project_graph.read(cx).detail_mode, 0, "Escape should leave the Project Map")
    });
    ws.update_in(cx, |ws, window, cx| ws.run_command("nav.projectMap", window, cx));
    cx.simulate_input("worker");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    ws.update(cx, |ws, cx| {
        ws.reveal_panel("where", cx);
        ws.panel_view("where", cx);
        ws.panel_view("infile", cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert!(ws.active_doc().unwrap().read(cx).doc.path().ends_with("worker.py"));
        assert!(ws.where_panel.read(cx).rows.iter().any(|r| r.label == "main"));
        assert!(ws.infile.read(cx).rows.iter().any(|r| r.label.contains("run")));
        assert!(ws.infile.read(cx).rows.iter().any(|r| r.detail.contains("used by main")));
    });
}

#[gpui_kit::test]
fn traced_recursive_functions_have_inspectable_switchable_native_trees(cx: &mut TestAppContext) {
    if forge_run::python::find_python(std::path::Path::new("."), None).is_none() {
        return;
    }
    cx.executor().allow_parking();
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.py");
    std::fs::write(&file,"def factorial(n):\n    return 1 if n < 2 else n * factorial(n - 1)\ndef fib(n):\n    return n if n < 2 else fib(n - 1) + fib(n - 2)\nprint(factorial(4),fib(4))\n").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.open_path(file, window, cx);
        ws.run_command("view.projectGraph", window, cx);
        ws.run_command("run.file", window, cx);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| ws.run.current.is_none() && ws.project_graph.read(cx).recursion_trees.len() >= 2) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Traced recursion did not reach graph");
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(30));
    }
    let before = ws.read_with(cx, |ws, cx| ws.project_graph.read(cx).recursion.read(cx).nodes[0].title.clone());
    ws.update(cx, |ws, cx| {
        ws.project_graph.update(cx, |p, cx| {
            p.detail_mode = 2;
            cx.notify();
        })
    });
    cx.run_until_parked();
    let next =
        cx.debug_bounds("graph-recursion-next").expect("Every recorded recursive function needs to be inspectable");
    cx.simulate_click(next.center(), Default::default());
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        let panel = ws.project_graph.read(cx).recursion.read(cx);
        assert_ne!(panel.nodes[0].title, before);
        assert!(panel.nodes.iter().any(|n| n.fields.iter().any(|s| s.starts_with('→'))));
        assert!(!panel.edges.is_empty());
    });
}

#[gpui_kit::test]
fn project_and_run_changes_clear_object_and_recursion_snapshots(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    ws.update_in(cx, |ws, window, cx| {
        let panel = ws.project_graph.clone();
        panel.update(cx, |p, cx| {
            p.show_objects(
                forge_core::execution::ObjGraph {
                    roots: vec![("head".into(), "1".into())],
                    nodes: vec![forge_core::execution::ObjNode {
                        id: "1".into(),
                        type_: "PreviousProject".into(),
                        ..Default::default()
                    }],
                    edges: Vec::new(),
                },
                cx,
            );
            p.show_recursion(
                forge_core::execution::RecTree {
                    key: "old:f#1".into(),
                    func: "old:f".into(),
                    total: 1,
                    nodes: vec![forge_core::execution::RecNode { parent: -1, args: "private".into(), ret: None }],
                },
                cx,
            );
        });
        ws.open_folder(dir.path().into(), window, cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        let p = ws.project_graph.read(cx);
        assert!(p.objects.read(cx).nodes.is_empty(), "Previous-project object snapshots must be released");
        assert!(p.recursion.read(cx).nodes.is_empty(), "Previous-project recursion snapshots must be released");
    });
}

#[gpui_kit::test]
fn templates_create_a_project_save_it_and_offer_the_user_copy(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (data, ws, cx) = setup(cx);
    let parent = tempfile::tempdir().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    ws.update_in(cx, |ws, window, cx| ws.run_command("file.newProject", window, cx));
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| ws.palette.read(cx).is_open()) {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    cx.simulate_input("Script");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.simulate_path_prompt_response(|options| {
        assert!(options.directories);
        Some(vec![parent.path().into()])
    });
    cx.run_until_parked();
    cx.simulate_input("Demo One");
    cx.simulate_keystrokes("enter");
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, _| {
            ws.root.as_ref().is_some_and(|r| r.ends_with("Demo One")) && ws.tabs.active().is_some()
        }) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Template did not create/open project");
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(20));
    }
    assert!(parent.path().join("Demo One/main.py").exists());
    ws.update_in(cx, |ws, window, cx| ws.run_command("file.saveTemplate", window, cx));
    cx.simulate_input("My Starter");
    cx.simulate_keystrokes("enter");
    loop {
        cx.run_until_parked();
        if !forge_sys::templates::list_user_templates(&data.path().join("templates")).is_empty() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "User template was not saved");
        std::thread::sleep(Duration::from_millis(10));
    }
    ws.update_in(cx, |ws, window, cx| ws.run_command("file.newProject", window, cx));
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| ws.palette.read(cx).is_open()) {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    cx.simulate_input("My Starter");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.simulate_path_prompt_response(|_| Some(vec![parent.path().into()]));
    cx.run_until_parked();
    cx.simulate_input("Rain Alert");
    cx.simulate_keystrokes("enter");
    loop {
        cx.run_until_parked();
        if parent.path().join("Rain Alert/main.py").exists() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Saved template not offered in picker");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(std::fs::read_to_string(parent.path().join("Rain Alert/main.py")).unwrap().contains("Rain Alert"));
}

#[gpui_kit::test]
fn database_diagram_reads_schema_and_navigates_to_model_source(cx: &mut TestAppContext) {
    if forge_run::python::find_python(std::path::Path::new("."), None).is_none() {
        return;
    }
    cx.executor().allow_parking();
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models.py");
    std::fs::write(&path,"from sqlalchemy import Column, Integer, ForeignKey\nfrom sqlalchemy.orm import declarative_base\nBase = declarative_base()\nclass User(Base):\n    __tablename__ = 'users'\n    id = Column(Integer, primary_key=True)\nclass Post(Base):\n    __tablename__ = 'posts'\n    id = Column(Integer, primary_key=True)\n    author_id = Column(Integer, ForeignKey('users.id'))\n").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.run_command("view.database", window, cx);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| ws.database.read(cx).nodes.iter().any(|n| n.id == "posts")) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Schema did not reach the native diagram");
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(30));
    }
    ws.read_with(cx, |ws, cx| {
        let d = ws.database.read(cx);
        assert!(d.edges.iter().any(|e| e.from == "posts" && e.to == "users"));
        assert!(d.nodes.iter().find(|n| n.id == "posts").unwrap().fields.iter().any(|s| s.contains("author_id")));
    });
    let node = cx.debug_bounds("diagram-node-posts").unwrap();
    cx.simulate_click(node.center(), Default::default());
    cx.run_until_parked();
    let open = cx.debug_bounds("diagram-source").unwrap();
    cx.simulate_click(open.center(), Default::default());
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.active_doc().unwrap().read(cx).doc.path(),
            forge_sys::files::canonical_path(&path).to_string_lossy()
        )
    });
}

#[gpui_kit::test]
fn memory_run_captures_allocations_types_and_retained_paths(cx: &mut TestAppContext) {
    if forge_run::python::find_python(std::path::Path::new("."), None).is_none() {
        return;
    }
    cx.executor().allow_parking();
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mem.py");
    std::fs::write(&path,"import time\nclass Item:\n    def __init__(self,n):\n        self.n=n\n        self.data=[n]*50\nCACHE=[]\nfor i in range(2000):\n    CACHE.append(Item(i))\n    if i % 500 == 0:\n        time.sleep(0.15)\nprint(len(CACHE))\n").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.open_path(path, window, cx);
        ws.run_command("run.memory", window, cx);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| ws.run.current.is_none() && ws.memory.read(cx).log.count("mem.Item") == 2000.) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Memory records did not reach native panel");
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(40));
    }
    ws.read_with(cx, |ws, cx| {
        let log = &ws.memory.read(cx).log;
        assert!(log.peak > 100_000.);
        assert!(log.samples.len() > 3);
        assert!(log.sites.iter().any(|s| s.path == "mem.py"));
        assert!(log
            .kept
            .iter()
            .any(|k| k.type_ == "mem.Item" && k.count == 2000. && k.path.iter().any(|s| s == "mem.CACHE")));
        assert!(!log.running);
    });
}

#[gpui_kit::test]
fn debugger_graph_records_locals_history_and_shared_objects(cx: &mut TestAppContext) {
    if !std::process::Command::new("python").args(["-c", "import debugpy"]).status().is_ok_and(|s| s.success()) {
        return;
    }
    cx.executor().allow_parking();
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.py");
    std::fs::write(&path,"class Node:\n    def __init__(self,value):\n        self.value=value\ndef work():\n    head=Node(1)\n    tail=Node(2)\n    head.next=tail\n    items=[head,tail]\n    total=1\n    total=2\n    return total\nwork()\n").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.open_path(path.clone(), window, cx);
        ws.run_command("view.projectGraph", window, cx);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(35);
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| ws.project_graph.read(cx).graph.modules.iter().any(|m| m.id == "main")) {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(40));
    }
    ws.update_in(cx, |ws, window, cx| {
        ws.debug.breakpoints.toggle(&path.to_string_lossy(), 9);
        ws.start_debug(window, cx);
    });
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| {
            ws.project_graph.read(cx).trail.stops.last().is_some_and(|s| s.vars.get("total").is_some_and(|v| v == "1"))
        }) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Graph did not capture debugger locals");
        std::thread::sleep(Duration::from_millis(20));
        cx.executor().advance_clock(Duration::from_millis(30));
    }
    ws.update_in(cx, |ws, window, cx| ws.inspect_debug_objects(window, cx));
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| {
            ws.project_graph.read(cx).objects.read(cx).nodes.iter().filter(|n| n.title == "Node").count() == 2
        }) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Shared objects were not inspected");
        std::thread::sleep(Duration::from_millis(15));
        cx.executor().advance_clock(Duration::from_millis(30));
    }
    ws.update_in(cx, |ws, window, cx| ws.debug_control("next", window, cx));
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| {
            ws.project_graph.read(cx).trail.stops.last().is_some_and(|s| s.vars.get("total").is_some_and(|v| v == "2"))
        }) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Graph did not track variable change");
        std::thread::sleep(Duration::from_millis(15));
        cx.executor().advance_clock(Duration::from_millis(30));
    }
    ws.read_with(cx, |ws, cx| {
        let p = ws.project_graph.read(cx);
        assert!(p.trail.edges.contains_key(&("module:main".into(), "main:work".into())));
        let h = p.trail.history("main:work");
        assert!(h.rows.iter().find(|r| r.name == "total").unwrap().changed.iter().any(|b| *b));
        assert!(!p.objects.read(cx).edges.is_empty());
    });
    ws.update(cx, |ws, cx| ws.stop_debug(cx));
    ws.read_with(cx, |ws, cx| {
        assert!(
            !ws.project_graph.read(cx).status.starts_with("Paused"),
            "Stopping debug must clear the paused graph status"
        )
    });
}

#[gpui_kit::test]
fn pipeline_block_picker_inserts_python_imports_and_preserves_dirty_history(cx: &mut TestAppContext) {
    if forge_run::python::find_python(std::path::Path::new("."), None).is_none() {
        return;
    }
    cx.executor().allow_parking();
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.py");
    std::fs::write(&path, "def pipeline():\n    pass\n").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.open_path(path.clone(), window, cx);
    });
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input("# unsaved work\ndef pipeline():\n    pass\n");
    ws.update_in(cx, |ws, window, cx| ws.run_command("view.pipeline", window, cx));
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| ws.pipeline_is_current(cx) && ws.pipeline.read(cx).flow.is_some()) {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(40));
    }
    let add = cx.debug_bounds("pipeline-add").unwrap();
    cx.simulate_click(add.center(), Default::default());
    cx.simulate_input("read_csv");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        let d = ws.active_doc().unwrap();
        let d = &d.read(cx).doc;
        assert!(d.text().contains("data = read_csv(path=\"\")"));
        assert!(d.text().contains("from forge_blocks import read_csv"));
        assert!(d.text().contains("# unsaved work"));
        assert!(d.dirty());
    });
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "def pipeline():\n    pass\n");
    assert_eq!(std::fs::read_to_string(dir.path().join("forge_blocks.py")).unwrap(), forge_core::blocks::BLOCKS_PY);
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| ws.pipeline_is_current(cx)) {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(40));
    }
    ws.update_in(cx, |ws, window, cx| {
        let b = forge_core::blocks::builtin_blocks().into_iter().find(|b| b.name == "fill_missing").unwrap();
        ws.place_pipeline_block(&b, true, window, cx);
    });
    ws.read_with(cx, |ws, cx| {
        assert!(
            ws.active_doc().unwrap().read(cx).doc.text().contains("data.to_dict(\"records\")"),
            "For each must iterate DataFrame records, not column names"
        )
    });
    ws.update_in(cx, |ws, window, cx| {
        ws.reveal_panel("editor", cx);
        ws.focus_editor(window, cx);
    });
    cx.simulate_keystrokes("ctrl-z");
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-z");
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| assert!(!ws.active_doc().unwrap().read(cx).doc.text().contains("read_csv")));
}

#[gpui_kit::test]
fn pipeline_reads_unsaved_source_highlights_dependencies_and_captures_real_values(cx: &mut TestAppContext) {
    if forge_run::python::find_python(std::path::Path::new("."), None).is_none() {
        return;
    }
    cx.executor().allow_parking();
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.py");
    let source="def sales_pipeline(limit: int = 2):\n    orders = [{'amount': 10}, {'amount': 20}, {'amount': 30}]\n    paid = orders[:limit]\n    return paid\n\nprint(sales_pipeline())\n";
    std::fs::write(&path, source).unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.open_path(path.clone(), window, cx);
        ws.run_command("view.pipeline", window, cx);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| ws.pipeline.read(cx).flow.is_some()) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Pipeline analysis did not arrive");
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(30));
    }
    let node = cx.debug_bounds("pipeline-node-s3").unwrap();
    cx.simulate_click(node.center(), Default::default());
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        let p = ws.pipeline.read(cx);
        assert_eq!(p.selected.as_deref(), Some("s3"));
        assert!(p.upstream.contains("s2"));
        assert!(p.downstream.contains("s4"));
    });
    let run = cx.debug_bounds("pipeline-run").unwrap();
    cx.simulate_click(run.center(), Default::default());
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| {
            ws.run.current.is_none() && ws.pipeline.read(cx).log.io("main:sales_pipeline").is_some()
        }) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Runtime summaries did not reach Pipeline");
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(30));
    }
    ws.read_with(cx, |ws, cx| {
        let p = ws.pipeline.read(cx);
        let points = p.log.between("main:sales_pipeline", 3, 3);
        assert!(points.iter().any(|p| p.var == "paid" && p.summary.len == Some(2)));
        let io = p.log.io("main:sales_pipeline").unwrap();
        assert_eq!(io.calls[0].input["limit"].repr.as_deref(), Some("2"));
        assert_eq!(io.calls[0].output.as_ref().unwrap().len, Some(2));
    });
    assert!(
        cx.debug_bounds("pipeline-preview-paid").is_some(),
        "Captured record rows should render in the selected step"
    );
    let open = cx.debug_bounds("pipeline-open-source").unwrap();
    cx.simulate_click(open.center(), Default::default());
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input("def changed_pipeline():\n    return 9\n");
    ws.update_in(cx, |ws, window, cx| {
        ws.on_pipeline_event(&crate::panels::pipeline::PipelineEvent::Run, window, cx);
        assert!(ws.run.current.is_none(), "Run must wait until the changed source has been analysed");
    });
    ws.update_in(cx, |ws, window, cx| ws.run_command("view.pipeline", window, cx));
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| ws.pipeline.read(cx).func == "changed_pipeline") {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Unsaved Pipeline was not refreshed");
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(50));
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    ws.read_with(cx, |ws, cx| {
        assert!(!ws.pipeline.read(cx).log.has_data(), "An edited function must not show values from its old source")
    });
    let other = tempfile::tempdir().unwrap();
    ws.update_in(cx, |ws, window, cx| ws.open_folder(other.path().into(), window, cx));
    cx.run_until_parked();
    cx.simulate_prompt_answer("Discard and Open");
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert!(!ws.pipeline.read(cx).log.has_data());
        assert!(ws.pipeline.read(cx).flow.is_none());
    });
}

#[gpui_kit::test]
fn api_to_csv_workflow_uses_native_controls_and_runs_the_generated_python(cx: &mut TestAppContext) {
    if forge_run::python::find_python(std::path::Path::new("."), None).is_none() {
        return;
    }
    cx.executor().allow_parking();
    let (_data, ws, cx) = setup(cx);
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join(".env"), "PEOPLE_TOKEN=workflow-token\n").unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let stopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done = stopped.clone();
    let worker = std::thread::spawn(move || {
        use std::io::{Read, Write};
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let mut calls = 0;
        while calls < 2 && !done.load(std::sync::atomic::Ordering::Relaxed) {
            let mut socket = match listener.accept() {
                Ok((s, _)) => s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(std::time::Instant::now() < deadline, "API-to-CSV workflow did not make both requests");
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(e) => panic!("{e}"),
            };
            socket.set_nonblocking(false).unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut headers = Vec::new();
            let mut b = [0; 1];
            while socket.read(&mut b).unwrap_or(0) > 0 {
                headers.push(b[0]);
                if headers.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let headers = String::from_utf8(headers).unwrap();
            assert!(headers.starts_with("GET /users "));
            assert!(headers.to_lowercase().contains("authorization: bearer workflow-token"));
            let body = r#"[{"id":1,"name":"Ann","plan":{"name":"pro"}},{"id":2,"name":"Bo","plan":{"name":"free"}}]"#;
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            calls += 1;
        }
        calls
    });
    struct ServerGuard {
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        worker: Option<std::thread::JoinHandle<usize>>,
    }
    impl Drop for ServerGuard {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
    let mut server = ServerGuard { stop: stopped, worker: Some(worker) };
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(project.path().into(), window, cx);
        ws.run_command("task.connect", window, cx);
        ws.run_command("connect.addApiBox", window, cx);
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(80));
    cx.run_until_parked();
    let name = cx.debug_bounds("connect-field-name").unwrap();
    cx.simulate_click(name.center(), gpui_kit::Modifiers::default());
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input("People");
    cx.run_until_parked();
    let url = cx.debug_bounds("connect-field-url").unwrap();
    cx.simulate_click(url.center(), gpui_kit::Modifiers::default());
    cx.simulate_input(&format!("http://{address}/users"));
    cx.run_until_parked();
    let api_settings = cx.debug_bounds("connect-api-settings").unwrap();
    cx.simulate_click(api_settings.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    let auth = cx.debug_bounds("connect-auth").unwrap();
    cx.simulate_click(auth.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    cx.simulate_input("Bearer token");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert_eq!(forge_core::box_code::auth_env(&ws.connect.read(cx).boxes[0]), vec!["PEOPLE_TOKEN"])
    });
    ws.update_in(cx, |ws, window, cx| {
        ws.on_connect_event(&crate::panels::connect::ConnectEvent::IgnoreEnv, window, cx)
    });
    cx.run_until_parked();
    assert!(std::fs::read_to_string(project.path().join(".gitignore")).unwrap().contains(".env"));
    let test = cx.debug_bounds("connect-test-call").unwrap();
    cx.simulate_click(test.center(), gpui_kit::Modifiers::default());
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| ws.connect.read(cx).boxes[0].requests[0].test.as_ref().is_some_and(|t| t.ok)) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Test request failed: {}",
            ws.read_with(cx, |ws, cx| ws.connect.read(cx).status.clone())
        );
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(30));
    }
    for selector in ["connect-response-field-0", "connect-response-field-2"] {
        let field = cx.debug_bounds(selector).unwrap();
        cx.simulate_click(field.center(), gpui_kit::Modifiers::default());
        cx.run_until_parked();
    }
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    let csv = project.path().join("people_data.csv");
    let expected = "name,plan.name\r\nAnn,pro\r\nBo,free\r\n";
    assert_eq!(std::fs::read_to_string(&csv).unwrap(), expected);
    let review = cx.debug_bounds("connect-preview").unwrap();
    cx.simulate_click(review.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(!project.path().join("api/people.py").exists());
    let apply = cx.debug_bounds("apply-generated-code").expect("Reviewable Python before writing");
    cx.simulate_click(apply.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    let generated = std::fs::read_to_string(project.path().join("api/people.py")).unwrap();
    assert!(generated.contains("def people_data("));
    assert!(!generated.contains("workflow-token"));
    std::fs::remove_file(&csv).unwrap();
    ws.update_in(cx, |ws, window, cx| ws.run_command("run.file", window, cx));
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, _| ws.run.current.is_none()) && csv.is_file() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Generated client did not finish");
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(30));
    }
    assert_eq!(std::fs::read_to_string(&csv).unwrap(), expected);
    assert_eq!(server.worker.take().unwrap().join().unwrap(), 2);
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    let metadata = std::fs::read_to_string(project.path().join(".forge/boxes.json")).unwrap();
    assert!(!metadata.contains("workflow-token"));
    assert!(!metadata.contains("\"sample\""));
}

#[gpui_kit::test]
fn connect_toolbox_can_drag_a_new_api_onto_the_canvas(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.run_command("task.connect", window, cx);
    });
    cx.run_until_parked();
    let source = cx.debug_bounds("connect-add-box").unwrap();
    let target = cx.debug_bounds("graph-canvas").expect("Connect canvas is a drop target");
    cx.simulate_mouse_down(source.center(), gpui_kit::MouseButton::Left, gpui_kit::Modifiers::default());
    cx.simulate_mouse_move(
        source.center() + gpui_kit::point(px(12.), px(0.)),
        Some(gpui_kit::MouseButton::Left),
        gpui_kit::Modifiers::default(),
    );
    cx.run_until_parked();
    cx.simulate_mouse_move(target.center(), Some(gpui_kit::MouseButton::Left), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    cx.simulate_mouse_up(target.center(), gpui_kit::MouseButton::Left, gpui_kit::Modifiers::default());
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| assert_eq!(ws.connect.read(cx).boxes.len(), 1));
}

#[gpui_kit::test]
fn connect_graph_source_navigation_returns_to_code_task(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.py"), "x=1\n").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.run_command("task.connect", window, cx);
        ws.project_graph.update(cx, |p, _| {
            p.graph = std::sync::Arc::new(forge_core::project_graph::ProjectGraph {
                modules: vec![forge_core::project_graph::ModuleNode {
                    id: "main".into(),
                    path: "main.py".into(),
                    ..Default::default()
                }],
                ..Default::default()
            })
        });
        ws.on_graph_event(&crate::panels::graph::GraphEvent::Open { module: "main".into(), line: 0 }, window, cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert_eq!(ws.tasks.current(), "code"));
}

#[gpui_kit::test]
fn connect_gitignore_offer_preserves_dirty_buffers_and_existing_disk_text(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".gitignore");
    std::fs::write(&path, "target/\n").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.open_path(path.clone(), window, cx);
        ws.active_doc().unwrap().update(cx, |d, cx| {
            d.edit(None, cx, |doc| {
                let forge_text::Doc { buffer, history, .. } = doc;
                history.transact(buffer, forge_text::EditKind::Other, Pos::new(0, 0), |b| {
                    b.set_text("target/\n# unsaved comment\n");
                    Pos::new(0, 0)
                });
            })
        });
        ws.on_connect_event(&crate::panels::connect::ConnectEvent::IgnoreEnv, window, cx);
    });
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(path).unwrap(), "target/\n");
    ws.read_with(cx, |ws, cx| {
        let doc = ws.active_doc().unwrap().read(cx);
        assert!(doc.doc.dirty());
        assert!(doc.doc.text().contains("# unsaved comment\n.env\n"));
        assert!(!ws.connect.read(cx).env_ignored);
    });
}

#[gpui_kit::test]
fn connect_request_removal_confirms_and_preserves_the_remaining_request(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.run_command("connect.addApiBox", window, cx);
        ws.connect.update(cx, |p, cx| {
            let next = forge_core::boxes::new_request(&p.boxes[0], "second");
            p.boxes[0].requests.push(next);
            p.request = 1;
            p.refresh_form(window, cx);
        });
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, window, cx| {
        ws.on_connect_event(&crate::panels::connect::ConnectEvent::RemoveRequest, window, cx)
    });
    cx.run_until_parked();
    cx.simulate_prompt_answer("Remove request");
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        let p = ws.connect.read(cx);
        assert_eq!(p.boxes[0].requests.len(), 1);
        assert_eq!(p.boxes[0].requests[0].name, "data");
        assert_eq!(p.request, 0);
    });
}

#[gpui_kit::test]
fn network_visible_before_run_captures_requests_even_when_output_is_revealed(cx: &mut TestAppContext) {
    if forge_run::python::find_python(std::path::Path::new("."), None).is_none() {
        return;
    }
    cx.executor().allow_parking();
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let worker = std::thread::spawn(move || {
        use std::io::{Read, Write};
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut socket = loop {
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("{e}"),
            }
        };
        socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut b = [0; 4096];
        let _ = socket.read(&mut b);
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").unwrap();
    });
    let path = dir.path().join("main.py");
    std::fs::write(&path,format!("import urllib.request\ndef fetch():\n    with urllib.request.urlopen('http://{address}/users') as r:\n        print(r.read().decode())\nfetch()\n")).unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.open_path(path.clone(), window, cx);
        ws.run_command("panel.network", window, cx);
        assert!(ws.panel_is_shown("network"));
        ws.run_command("run.file", window, cx);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| {
            ws.run.current.is_none()
                && ws.network.read(cx).log.current().iter().any(|r| r.status == Some(200) && r.url.ends_with("/users"))
        }) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Real request never reached Network");
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(30));
    }
    worker.join().unwrap();
    ws.update_in(cx, |ws, window, cx| ws.run_command("panel.network", window, cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("network-record-1").is_some());
}

#[gpui_kit::test]
fn connect_keeps_hand_edited_code_and_does_not_rewrite_an_unchanged_csv(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("client.py");
    let mut api = forge_core::boxes::new_api_box(&[], "Test");
    api.target.file = "client.py".into();
    api.target.mode = "append".into();
    let block = forge_core::box_code::generate_block(&api, false);
    api.hash = Some(forge_core::box_code::block_hash(&block));
    let edited = block.replace("def test_data(", "def my_edited_function(");
    std::fs::write(&path, &edited).unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.connect.update(cx, |p, cx| {
            p.set_boxes(vec![api], true, window, cx);
            let id = p.boxes[0].id.clone();
            let rid = p.boxes[0].requests[0].id.clone();
            p.boxes[0].requests[0].output = "result.csv".into();
            p.boxes[0].requests[0].url = "/changed".into();
            p.test_result(
                &id,
                &rid,
                forge_sys::connect::TestCallResult {
                    ok: true,
                    json: Some(serde_json::json!([{"a":1}])),
                    ..Default::default()
                },
                cx,
            );
        });
        ws.on_connect_event(&crate::panels::connect::ConnectEvent::Changed, window, cx);
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
    let csv = dir.path().join("result.csv");
    assert!(csv.exists());
    std::fs::write(&csv, "manual CSV edit\n").unwrap();
    ws.update_in(cx, |ws, window, cx| ws.on_connect_event(&crate::panels::connect::ConnectEvent::Changed, window, cx));
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(csv).unwrap(), "manual CSV edit\n");
}

#[gpui_kit::test]
fn connect_exports_all_response_rows_and_updates_the_selected_columns(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.run_command("connect.addApiBox", window, cx);
        ws.connect.update(cx, |p, cx| {
            let id = p.boxes[0].id.clone();
            let rid = p.boxes[0].requests[0].id.clone();
            p.boxes[0].requests[0].output = "result.csv".into();
            let rows = (0..25).map(|i| serde_json::json!({"id":i,"name":format!("row {i}")})).collect::<Vec<_>>();
            p.test_result(
                &id,
                &rid,
                forge_sys::connect::TestCallResult {
                    ok: true,
                    json: Some(serde_json::json!(rows)),
                    ..Default::default()
                },
                cx,
            );
        });
        ws.on_connect_event(&crate::panels::connect::ConnectEvent::Changed, window, cx);
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    let csv = std::fs::read_to_string(dir.path().join("result.csv")).expect("A successful test response exports CSV");
    assert_eq!(csv.lines().count(), 26);
    assert!(csv.contains("24,row 24"));
    ws.update_in(cx, |ws, window, cx| {
        ws.connect.update(cx, |p, _| p.boxes[0].requests[0].ticked = vec!["name".into()]);
        ws.on_connect_event(&crate::panels::connect::ConnectEvent::Changed, window, cx);
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    let csv = std::fs::read_to_string(dir.path().join("result.csv")).unwrap();
    assert!(csv.starts_with("name\r\n"));
    assert!(!csv.contains("24,row"));
}

#[gpui_kit::test]
fn connect_synchronises_untouched_generated_blocks_without_saving_other_dirty_edits(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("client.py");
    let mut api = forge_core::boxes::new_api_box(&[], "Test");
    api.target.file = "client.py".into();
    api.target.mode = "append".into();
    api.requests[0].url = "/before".into();
    let code = forge_core::box_code::generate_block(&api, false);
    api.hash = Some(forge_core::box_code::block_hash(&code));
    std::fs::write(&path, &code).unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.open_path(path.clone(), window, cx);
        ws.connect.update(cx, |p, cx| p.set_boxes(vec![api], true, window, cx));
        ws.active_doc().unwrap().update(cx, |d, cx| {
            d.edit(None, cx, |doc| {
                let forge_text::Doc { buffer, history, .. } = doc;
                history.transact(buffer, forge_text::EditKind::Other, Pos::new(0, 0), |b| {
                    b.set_text(&(code.clone() + "# other unsaved edits\n"));
                    Pos::new(0, 0)
                });
            })
        });
        ws.connect.update(cx, |p, _| p.boxes[0].requests[0].url = "/after".into());
        ws.on_connect_event(&crate::panels::connect::ConnectEvent::Changed, window, cx);
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        let doc = ws.active_doc().unwrap().read(cx);
        assert!(doc.doc.text().contains("/after"));
        assert!(doc.doc.text().contains("# other unsaved edits"));
        assert!(doc.doc.dirty());
    });
    assert_eq!(std::fs::read_to_string(path).unwrap(), code);
}

#[gpui_kit::test]
fn openapi_picker_updates_an_already_floating_box_panel(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("../../../forge-core/tests/fixtures/openapi.json")).unwrap();
    let spec = forge_core::openapi::read_spec(&cases[0]["doc"], "").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.float_panel("box", window, cx);
        ws.openapi.update(cx, |p, cx| p.loaded(spec, window, cx));
        ws.run_command("api.addOpenApi", window, cx);
    });
    cx.run_until_parked();
    let floating = ws.read_with(cx, |ws, _| ws.floating["box"]);
    let mut floating_cx = VisualTestContext::from_window(floating, cx);
    floating_cx.run_until_parked();
    assert!(floating_cx.debug_bounds("openapi-preview").is_some(), "OpenAPI should appear in the existing Box window");
}

#[gpui_kit::test]
fn openapi_loading_preview_and_apply_preserve_source_and_environment_values(cx: &mut TestAppContext) {
    use crate::panels::openapi::OpenApiEvent;
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main.py");
    std::fs::write(&main, "import os\nprint('keep')\n").unwrap();
    std::fs::write(dir.path().join(".env"), "PETSTORE_TOKEN=keep-private-value\n").unwrap();
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("../../../forge-core/tests/fixtures/openapi.json")).unwrap();
    std::fs::write(dir.path().join("openapi.json"), cases[0]["doc"].to_string()).unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.open_path(main.clone(), window, cx);
        ws.on_openapi_event(&OpenApiEvent::Load("openapi.json".into()), window, cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| assert_eq!(ws.openapi.read(cx).spec.as_ref().unwrap().operations.len(), 4));
    let preview = cx.debug_bounds("openapi-preview").expect("Native endpoint picker shows Preview");
    cx.simulate_click(preview.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(!dir.path().join("api/petstore_client.py").exists());
    ws.read_with(cx, |ws, cx| assert!(!ws.git_inspect.read(cx).contains_text("keep-private-value")));
    let apply = cx.debug_bounds("apply-generated-code").expect("Client preview has Apply action");
    cx.simulate_click(apply.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(std::fs::read_to_string(dir.path().join("api/petstore_client.py")).unwrap().contains("def list_pets("));
    assert!(std::fs::read_to_string(&main)
        .unwrap()
        .contains("from api.petstore_client import list_pets, create_pet, show_pet_by_id, delete_pets_by"));
    let env = std::fs::read_to_string(dir.path().join(".env")).unwrap();
    assert!(env.contains("PETSTORE_TOKEN=keep-private-value"));
    assert!(env.contains("PETSTORE_BASE_URL=https://eu.pets.example.com/v1"));
    assert!(env.contains("PETSTORE_API_KEY=\"\""));
}

#[gpui_kit::test]
fn openapi_apply_rejects_stale_selection_before_creating_any_files(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("../../../forge-core/tests/fixtures/openapi.json")).unwrap();
    let spec = forge_core::openapi::read_spec(&cases[0]["doc"], "").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.openapi.update(cx, |p, cx| p.loaded(spec, window, cx));
        ws.on_openapi_event(&crate::panels::openapi::OpenApiEvent::Preview, window, cx);
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, window, cx| {
        ws.openapi.update(cx, |p, _| p.chosen.clear());
        ws.apply_openapi_preview(window, cx);
    });
    cx.run_until_parked();
    assert!(!dir.path().join("api").exists());
    assert!(!dir.path().join(".env").exists());
}
#[gpui_kit::test]
fn openapi_apply_validates_all_targets_before_writing_the_client(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let env = dir.path().join(".env");
    std::fs::write(&env, "# initial\n").unwrap();
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("../../../forge-core/tests/fixtures/openapi.json")).unwrap();
    let spec = forge_core::openapi::read_spec(&cases[0]["doc"], "").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.openapi.update(cx, |p, cx| p.loaded(spec, window, cx));
        ws.on_openapi_event(&crate::panels::openapi::OpenApiEvent::Preview, window, cx);
    });
    cx.run_until_parked();
    std::fs::write(&env, "# changed after preview\n").unwrap();
    ws.update_in(cx, |ws, window, cx| ws.apply_openapi_preview(window, cx));
    cx.run_until_parked();
    assert!(!dir.path().join("api").exists());
    assert_eq!(std::fs::read_to_string(env).unwrap(), "# changed after preview\n");
}

#[gpui_kit::test]
fn api_box_graph_connection_selects_source_for_preview_without_writing_it(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.py");
    std::fs::write(&path, "# keep\n").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.run_command("connect.addApiBox", window, cx);
        ws.project_graph.update(cx, |p, _| {
            p.graph = std::sync::Arc::new(forge_core::project_graph::ProjectGraph {
                modules: vec![forge_core::project_graph::ModuleNode {
                    id: "source".into(),
                    path: "source.py".into(),
                    ..Default::default()
                }],
                ..Default::default()
            })
        });
        let id = ws.connect.read(cx).boxes[0].id.clone();
        ws.on_graph_event(
            &crate::panels::graph::GraphEvent::Attach { box_id: id, module: "source".into() },
            window,
            cx,
        );
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        let api = &ws.connect.read(cx).boxes[0];
        assert_eq!(api.target.file, "source.py");
        assert_eq!(api.target.mode, "append");
        assert_eq!(api.attach.as_ref().unwrap().module, "source");
    });
    assert_eq!(std::fs::read_to_string(path).unwrap(), "# keep\n");
}

#[gpui_kit::test]
fn trace_from_previous_project_cannot_populate_the_new_projects_graph(cx: &mut TestAppContext) {
    let Some(python) = forge_run::python::find_python(std::path::Path::new("."), None) else { return };
    cx.executor().allow_parking();
    let (_data, ws, cx) = setup(cx);
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    let path = old.path().join("old.py");
    std::fs::write(&path, "import time\ndef old_work():\n    time.sleep(0.1)\nold_work()\n").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(old.path().into(), window, cx);
        let spec = forge_run::RunSpec {
            python: Some(python),
            program: Some(path.to_string_lossy().into()),
            cwd: old.path().into(),
            trace: true,
            ..Default::default()
        };
        ws.start_run(spec, window, cx);
        ws.open_folder(new.path().into(), window, cx);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, _| ws.run.current.is_none()) {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(20));
    }
    ws.read_with(cx, |ws, cx| {
        assert!(
            ws.project_graph.read(cx).profile.fns.is_empty(),
            "Old project profiler results leaked into the new graph"
        )
    });
}

#[gpui_kit::test]
fn api_box_env_preparation_adds_missing_names_without_overwriting_values(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env"), "# credentials\nEXISTING=keep\n").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.run_command("connect.addApiBox", window, cx);
        ws.connect.update(cx, |p, _| {
            p.boxes[0].auth =
                forge_core::boxes::BoxAuth::Basic { user_env: "EXISTING".into(), pass_env: "MISSING".into() }
        });
        ws.on_connect_event(&crate::panels::connect::ConnectEvent::Env, window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".env")).unwrap(),
        "# credentials\nEXISTING=keep\nMISSING=\"\"\n"
    );
}

#[gpui_kit::test]
fn api_box_append_preserves_dirty_buffers_and_removal_is_previewed_and_undoable(cx: &mut TestAppContext) {
    use crate::panels::connect::ConnectEvent;
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("client.py");
    std::fs::write(&path, "# disk\n").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.open_path(path.clone(), window, cx);
        ws.focus_editor(window, cx);
        ws.run_command("connect.addApiBox", window, cx);
        ws.connect.update(cx, |p, cx| {
            p.boxes[0].target.file = "client.py".into();
            p.boxes[0].target.mode = "append".into();
            cx.notify();
        });
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, window, cx| {
        ws.reveal_panel("editor", cx);
        ws.focus_editor(window, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input("# unsaved π\n");
    cx.run_until_parked();
    let before = ws.read_with(cx, |ws, cx| ws.active_doc().unwrap().read(cx).doc.text());
    assert!(before.contains("unsaved"));
    ws.update_in(cx, |ws, window, cx| ws.on_connect_event(&ConnectEvent::Preview, window, cx));
    cx.run_until_parked();
    ws.update_in(cx, |ws, window, cx| ws.on_connect_event(&ConnectEvent::Apply, window, cx));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# disk\n");
    let generated = ws.read_with(cx, |ws, cx| {
        let d = ws.active_doc().unwrap().read(cx);
        assert!(d.doc.dirty());
        d.doc.text()
    });
    assert!(generated.starts_with(&before));
    assert!(generated.contains("def api_data("));
    ws.update_in(cx, |ws, window, cx| ws.on_connect_event(&ConnectEvent::Remove, window, cx));
    cx.run_until_parked();
    cx.simulate_prompt_answer("Preview code removal");
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| assert_eq!(ws.active_doc().unwrap().read(cx).doc.text(), generated));
    ws.update_in(cx, |ws, window, cx| ws.on_connect_event(&ConnectEvent::Apply, window, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert!(ws.connect.read(cx).boxes.is_empty());
        assert_eq!(ws.active_doc().unwrap().read(cx).doc.text(), before);
    });
    cx.simulate_keystrokes("ctrl-z");
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| assert_eq!(ws.active_doc().unwrap().read(cx).doc.text(), generated));
}

#[gpui_kit::test]
fn api_box_test_results_pick_fields_and_never_persist_response_samples(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.run_command("connect.addApiBox", window, cx);
        ws.connect.update(cx, |p, cx| {
            let id = p.boxes[0].id.clone();
            let rid = p.boxes[0].requests[0].id.clone();
            p.test_result(
                &id,
                &rid,
                forge_sys::connect::TestCallResult {
                    ok: true,
                    status: Some(200),
                    json: Some(serde_json::json!({"users":[{"name":"sample-private","id":1}]})),
                    ..Default::default()
                },
                cx,
            );
            let r = &p.boxes[0].requests[0];
            assert_eq!(r.records_path, Some(vec!["users".into()]));
            assert_eq!(r.ticked, vec!["name", "id"]);
            assert_eq!(r.test.as_ref().unwrap().count, Some(1.0));
            let saved = serde_json::to_string(&forge_core::boxes::serialize_boxes(&p.boxes)).unwrap();
            assert!(!saved.contains("sample-private"));
            assert!(!saved.contains("\"fields\""));
        });
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn api_box_apply_refuses_a_source_changed_after_preview(cx: &mut TestAppContext) {
    use crate::panels::connect::ConnectEvent;
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("client.py");
    std::fs::write(&path, "# initial\n").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.open_path(path.clone(), window, cx);
        ws.run_command("connect.addApiBox", window, cx);
        ws.connect.update(cx, |p, _| {
            p.boxes[0].target.file = "client.py".into();
            p.boxes[0].target.mode = "append".into();
        });
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, window, cx| ws.on_connect_event(&ConnectEvent::Preview, window, cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("apply-generated-code").is_some());
    std::fs::write(&path, "# changed outside\n").unwrap();
    ws.update_in(cx, |ws, window, cx| ws.on_connect_event(&ConnectEvent::Apply, window, cx));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# changed outside\n");
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("OK");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn api_box_preview_creates_no_file_until_apply_and_generated_edits_can_be_undone(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.run_command("connect.addApiBox", window, cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| assert_eq!(ws.connect.read(cx).boxes.len(), 1));
    ws.update_in(cx, |ws, window, cx| ws.on_connect_event(&crate::panels::connect::ConnectEvent::Preview, window, cx));
    cx.run_until_parked();
    assert!(!dir.path().join("api/api.py").exists());
    let apply = cx.debug_bounds("apply-generated-code").expect("Reviewable code preview has Apply button");
    cx.simulate_click(apply.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(std::fs::read_to_string(dir.path().join("api/api.py")).unwrap().contains("def api_data("));
    cx.simulate_keystrokes("ctrl-z");
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| assert_eq!(ws.active_doc().unwrap().read(cx).doc.text(), ""));
}

#[gpui_kit::test]
fn traced_run_updates_the_native_graph_profile(cx: &mut TestAppContext) {
    if forge_run::python::find_python(std::path::Path::new("."), None).is_none() {
        return;
    }
    cx.executor().allow_parking();
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.py");
    std::fs::write(&path, "import time\ndef slow():\n    time.sleep(0.04)\nslow()\nprint('done')\n").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.open_path(path, window, cx);
        ws.run_command("view.projectGraph", window, cx);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| !ws.project_graph.read(cx).data.nodes.is_empty()) {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(30));
    }
    ws.update_in(cx, |ws, window, cx| ws.run_command("run.file", window, cx));
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| {
            ws.project_graph.read(cx).profile.fns.contains_key("main:slow") && ws.run.current.is_none()
        }) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Profile never reached the native graph");
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(30));
    }
    ws.read_with(cx, |ws, cx| assert!(ws.project_graph.read(cx).profile.fns["main:slow"].total_ms >= 20.0));
}

#[gpui_kit::test]
fn a_project_switch_discards_previous_graph_work(cx: &mut TestAppContext) {
    if forge_run::python::find_python(std::path::Path::new("."), None).is_none() {
        return;
    }
    cx.executor().allow_parking();
    let (_data, ws, cx) = setup(cx);
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    std::fs::write(first.path().join("old_project.py"), "pass\n").unwrap();
    std::fs::write(second.path().join("new_project.py"), "pass\n").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(first.path().into(), window, cx);
        ws.run_command("view.projectGraph", window, cx);
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, window, cx| ws.open_folder(second.path().into(), window, cx));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| ws.project_graph.read(cx).data.nodes.iter().any(|n| n.id == "new_project")) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "New project was not drawn");
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(30));
    }
    ws.read_with(cx, |ws, cx| assert!(!ws.project_graph.read(cx).graph.modules.iter().any(|m| m.id == "old_project")));
}

#[gpui_kit::test]
fn real_python_project_graph_scans_unsaved_changes_and_navigates_to_source(cx: &mut TestAppContext) {
    if forge_run::python::find_python(std::path::Path::new("."), None).is_none() {
        return;
    }
    cx.executor().allow_parking();
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.py");
    std::fs::write(&path, "def first():\n    pass\n").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(dir.path().into(), window, cx);
        ws.run_command("view.projectGraph", window, cx);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| ws.project_graph.read(cx).graph.modules.iter().any(|m| m.id == "main")) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Graph scan did not complete");
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(20));
    }
    ws.update_in(cx, |ws, window, cx| {
        ws.on_graph_event(&crate::panels::graph::GraphEvent::Open { module: "main".into(), line: 0 }, window, cx)
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input("def unsaved_name():\n    pass\n");
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| {
            ws.project_graph.read(cx).graph.modules.iter().any(|m| m.symbols.iter().any(|s| s.name == "unsaved_name"))
        }) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Unsaved source was not rescanned");
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(50));
    }
    assert!(std::fs::read_to_string(&path).unwrap().contains("first"));
}

#[gpui_kit::test]
fn context_tools_follow_the_file_and_clicks_reveal_the_suggested_panel(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test_example.py");
    std::fs::write(&file, "# test file\n").unwrap();
    ws.update_in(cx, |ws, window, cx| ws.open_path(file, window, cx));
    cx.run_until_parked();
    let bounds = cx.debug_bounds("suggest-tests").expect("Tests suggestion for a test file");
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert!(ws.panel_is_shown("tests")));
    assert!(cx.debug_bounds("suggest-tests").is_none(), "Already visible tools must not be suggested");
}

#[gpui_kit::test]
fn enabling_animations_does_not_replay_a_task_change_made_while_disabled(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    ws.update_in(cx, |ws, window, cx| {
        app_state::update_settings(cx, |s| s.animations = false).unwrap();
        ws.run_command("task.connect", window, cx);
    });
    cx.run_until_parked();
    let before = cx.debug_bounds("dock-body").unwrap().origin.y;
    ws.update(cx, |_, cx| app_state::update_settings(cx, |s| s.animations = true).unwrap());
    cx.run_until_parked();
    let after = cx.debug_bounds("dock-body").unwrap().origin.y;
    assert_eq!(after, before, "Changing the preference must not replay an old transition");
}

#[gpui_kit::test]
fn panel_popout_and_back_buttons_work_across_windows(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let project = tempfile::tempdir().unwrap();
    ws.update_in(cx, |ws, window, cx| ws.open_folder(project.path().into(), window, cx));
    cx.run_until_parked();
    let popout = cx.debug_bounds("float-files").expect("Files popout button");
    assert!(popout.size.width > px(8.0));
    cx.simulate_click(popout.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    let floating = ws.read_with(cx, |ws, _| ws.floating["files"]);
    let mut float_cx = VisualTestContext::from_window(floating, cx);
    float_cx.run_until_parked();
    let new_file = float_cx.debug_bounds("tree-new-file").expect("New File in floating Files panel");
    float_cx.simulate_click(new_file.center(), gpui_kit::Modifiers::default());
    float_cx.run_until_parked();
    float_cx.simulate_input("floating.txt");
    float_cx.simulate_keystrokes("enter");
    float_cx.run_until_parked();
    assert!(project.path().join("floating.txt").is_file());
    let back = float_cx.debug_bounds("float-back").expect("Back to the dock");
    float_cx.simulate_click(back.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    ws.read_with(cx, |ws, _| assert!(ws.floating.is_empty()));
    assert!(cx.debug_bounds("float-files").is_some());
}

#[gpui_kit::test]
fn resizing_with_a_floating_stack_preserves_its_saved_share(cx: &mut TestAppContext) {
    use forge_core::dock::{Dir, DockNode};
    let (_data, ws, cx) = setup(cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.tasks.changed(DockNode::split(
            "row",
            Dir::Row,
            vec![
                (DockNode::stack("a", &["files"]), 0.2),
                (DockNode::stack("b", &["editor"]), 0.4),
                (DockNode::stack("c", &["output"]), 0.4),
            ],
        ));
        ws.float_panel("files", window, cx);
        ws.resize_docked("row", &[0.25, 0.75]);
        let DockNode::Split { sizes, .. } = ws.tasks.root() else { panic!("split") };
        for (actual, expected) in sizes.iter().zip([0.2, 0.2, 0.6]) {
            assert!((actual - expected).abs() < 0.0001);
        }
        ws.close_floating(cx);
    });
}

#[gpui_kit::test]
fn density_changes_panel_geometry_without_resizing_the_editor(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let project = tempfile::tempdir().unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(project.path().into(), window, cx);
        ws.run_command("file.new", window, cx);
    });
    cx.run_until_parked();
    let normal = cx.debug_bounds("tree-name-row").unwrap().size.height;
    let font = ws.read_with(cx, |ws, cx| ws.editors[0].read(cx).font_size);
    for (density, expected) in
        [(forge_core::settings::Density::Compact, 0.88), (forge_core::settings::Density::Roomy, 1.12)]
    {
        ws.update(cx, |ws, cx| {
            app_state::update_settings(cx, |s| s.density = density).unwrap();
            ws.apply_settings(cx);
        });
        cx.run_until_parked();
        let height = cx.debug_bounds("tree-name-row").unwrap().size.height;
        assert!(
            (f32::from(height) - f32::from(normal) * expected).abs() < 1.0,
            "Panel row must follow {density:?}: normal={normal:?}, actual={height:?}"
        );
        ws.read_with(cx, |ws, cx| assert_eq!(ws.editors[0].read(cx).font_size, font));
    }
    cx.simulate_input("density.txt");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(project.path().join("density.txt").is_file());
}

#[gpui_kit::test]
fn floating_panel_keeps_its_entity_and_returns_when_its_window_closes(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let saved = ws.read_with(cx, |ws, _| ws.tasks.saved().clone());
    let panel = ws.read_with(cx, |ws, _| ws.file_tree.entity_id());
    ws.update_in(cx, |ws, window, cx| ws.float_panel("files", window, cx));
    cx.run_until_parked();
    let floating = ws.read_with(cx, |ws, _| {
        assert_eq!(ws.floating.len(), 1);
        assert_eq!(ws.file_tree.entity_id(), panel);
        assert!(!forge_core::dock::placed_panels(&ws.docked_root().unwrap()).contains(&"files".into()));
        assert_eq!(ws.tasks.saved(), saved);
        ws.floating["files"]
    });
    ws.update_in(cx, |ws, window, cx| {
        ws.float_panel("files", window, cx);
        ws.run_command("task.run", window, cx);
        ws.reveal_panel("files", cx);
        assert_eq!(ws.floating.len(), 1);
        assert_eq!(ws.tasks.current(), "run");
    });
    floating.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
    ws.update_in(cx, |ws, window, cx| ws.run_command("task.code", window, cx));
    ws.read_with(cx, |ws, _| {
        assert!(ws.floating.is_empty());
        assert_eq!(ws.file_tree.entity_id(), panel);
        assert!(forge_core::dock::placed_panels(&ws.docked_root().unwrap()).contains(&"files".into()));
        assert_eq!(ws.tasks.saved(), saved);
    });
}

#[gpui_kit::test]
fn editor_and_terminal_stay_docked_and_owner_closes_floating_windows(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.float_panel("editor", window, cx);
        ws.float_panel("terminal", window, cx);
        assert!(ws.floating.is_empty());
        ws.float_panel("output", window, cx);
        assert_eq!(ws.floating.len(), 1);
        assert!(ws.should_close(window, cx));
        assert!(ws.floating.is_empty());
    });
    cx.run_until_parked();
}

fn setup(cx: &mut TestAppContext) -> (tempfile::TempDir, Entity<Workspace>, &mut VisualTestContext) {
    // Workspace opening starts real filesystem/process I/O (including LSP).
    cx.executor().allow_parking();
    let data = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        let settings = Settings { check_for_updates: false, ..Default::default() };
        cx.set_global(AppState {
            user: forge_sys::user_state::UserState::new(data.path()),
            settings,
            settings_error: None,
            keymap: Default::default(),
            keys_error: None,
            updater: std::sync::Arc::new(forge_update::Updater::new(data.path().into())),
            restart_after_update: false,
        });
        app_state::apply_theme(cx);
        crate::editor::bind_keys(cx);
    });
    let mut workspace = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let ws = cx.new(|cx| {
            let mut ws = Workspace::new(window, cx);
            // These tests exercise workspace editing and Git, without starting an
            // unrelated shell whose native PTY thread wakes GPUI's test scheduler.
            for task in forge_core::dock::TASKS {
                ws.tasks.switch_to(task);
                ws.tasks.hide("terminal");
            }
            ws.tasks = Tasks::new(&ws.tasks.saved());
            ws
        });
        workspace = Some(ws.clone());
        gpui_kit::component::Root::new(ws, window, cx)
    });
    (data, workspace.expect("workspace created"), cx)
}

#[gpui_kit::test]
fn startup_shows_the_welcome_page_until_a_project_is_chosen(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let previous = tempfile::tempdir().unwrap();
    ws.update_in(cx, |ws, window, cx| {
        app_state::state(cx).user.add_recent(&previous.path().to_string_lossy()).unwrap();
        ws.reopen_last(window, cx);
        assert!(ws.root.is_none(), "A previous development/test project must not become the default workspace");
    });
}

#[gpui_kit::test]
fn reopening_the_last_project_still_works_when_explicitly_enabled(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let project = tempfile::tempdir().unwrap();
    ws.update_in(cx, |ws, window, cx| {
        cx.global_mut::<AppState>().settings.reopen_last_project = true;
        app_state::state(cx).user.add_recent(&project.path().to_string_lossy()).unwrap();
        ws.reopen_last(window, cx);
        assert_eq!(ws.root, Some(forge_sys::files::canonical_path(project.path())));
    });
}

#[gpui_kit::test]
fn a_new_project_folder_is_created_in_the_location_the_user_chooses(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let parent = tempfile::tempdir().unwrap();
    cx.run_until_parked();
    let bounds = cx.debug_bounds("tree-new-project").expect("New Project Folder button visible without a project");
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    cx.simulate_path_prompt_response(|options| {
        assert!(options.directories);
        Some(vec![parent.path().into()])
    });
    cx.run_until_parked();
    cx.simulate_input("my-project");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(parent.path().join("my-project").is_dir());
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.root, Some(forge_sys::files::canonical_path(&parent.path().join("my-project"))))
    });
}

#[gpui_kit::test]
fn a_new_file_without_a_project_uses_the_selected_save_location(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let parent = tempfile::tempdir().unwrap();
    let path = parent.path().join("standalone.txt");
    ws.update_in(cx, |ws, window, cx| ws.run_command("file.new", window, cx));
    cx.simulate_new_path_selection(|_| Some(path.clone()));
    cx.run_until_parked();
    assert!(path.is_file());
    ws.read_with(cx, |ws, cx| {
        assert_eq!(
            ws.active_doc().unwrap().read(cx).doc.path(),
            forge_sys::files::canonical_path(&path).to_string_lossy()
        )
    });
}

#[gpui_kit::test]
fn new_file_command_accepts_a_name_and_opens_an_editable_document(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let project = tempfile::tempdir().unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(project.path().into(), window, cx);
        ws.run_command("file.new", window, cx);
    });
    cx.run_until_parked();
    cx.simulate_input("hello.txt");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(project.path().join("hello.txt").is_file(), "New File must accept the typed name");
    cx.simulate_input("some text");
    ws.read_with(cx, |ws, cx| assert_eq!(ws.active_doc().unwrap().read(cx).doc.text(), "some text"));
    ws.update_in(cx, |ws, window, cx| ws.run_command("file.rename", window, cx));
    cx.run_until_parked();
    cx.simulate_input("renamed");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(project.path().join("renamed.txt").is_file());
}

#[gpui_kit::test]
fn new_file_button_accepts_typing_after_a_mouse_click(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let project = tempfile::tempdir().unwrap();
    ws.update_in(cx, |ws, window, cx| ws.open_folder(project.path().into(), window, cx));
    cx.run_until_parked();
    let bounds = cx.debug_bounds("tree-new-file").expect("New File button visible");
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    cx.simulate_input("clicked.txt");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(project.path().join("clicked.txt").is_file(), "Clicking New File must keep focus in the name input");
}

#[gpui_kit::test]
fn breakpoint_shortcut_gutter_markers_edits_and_workspace_persistence(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let project = tempfile::tempdir().unwrap();
    let file = project.path().join("a.txt");
    std::fs::write(&file, "first\nsecond\n").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(project.path().into(), window, cx);
        ws.open_path(file.clone(), window, cx);
        ws.go_to_line(1, window, cx);
        ws.run_command("debug.toggleBreakpoint", window, cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.debug.breakpoints.saved[0].line, 1);
        assert_eq!(ws.editors[0].read(cx).debug_lines, [(1, true)]);
    });
    ws.update(cx, |ws, cx| {
        let doc = ws.active_doc().unwrap();
        doc.update(cx, |d, cx| d.edit(None, cx, |d| d.buffer.insert(Pos::new(0, 0), "inserted\n")));
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.debug.breakpoints.saved[0].line, 2);
        let state = ws.capture_state(cx);
        assert_eq!(state.breakpoints[0].line, 2);
        assert_eq!(state.breakpoints[0].path, forge_sys::files::canonical_path(&file).to_string_lossy());
    });
}

#[gpui_kit::test]
fn source_control_stages_a_file_and_updates_the_panel(cx: &mut TestAppContext) {
    let (_data, ws, cx) = setup(cx);
    let project = tempfile::tempdir().unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(project.path())
        .status()
        .unwrap()
        .success());
    std::fs::write(project.path().join("a.txt"), "one\n").unwrap();
    ws.update_in(cx, |ws, window, cx| ws.open_folder(project.path().into(), window, cx));
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| assert!(ws.git_panel.read(cx).repo().is_some()));
    ws.update_in(cx, |ws, window, cx| {
        ws.on_git_event(
            &ws.git_panel.clone(),
            &crate::panels::git::GitPanelEvent::Stage(vec!["a.txt".into()]),
            window,
            cx,
        )
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert!(ws.git_panel.read(cx).repo().unwrap().status.staged.iter().any(|f| f.path == "a.txt"))
    });
}

#[gpui_kit::test]
fn real_debugpy_updates_the_native_panel_and_reads_terminal_input(cx: &mut TestAppContext) {
    if !std::process::Command::new("python").args(["-c", "import debugpy"]).status().is_ok_and(|s| s.success()) {
        return;
    }
    cx.executor().allow_parking();
    let (_data, ws, cx) = setup(cx);
    let project = tempfile::tempdir().unwrap();
    let file = project.path().join("debug.txt");
    std::fs::write(&file, "name = input('Name: ')\nupper = name.upper()\nprint(upper)\n").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_folder(project.path().into(), window, cx);
        ws.open_path(file.clone(), window, cx);
        // Let runInTerminal create the debuggee's PTY. A pre-existing shell
        // would make the session-count wait below race the adapter request.
        ws.tasks.switch_to("debug");
        ws.tasks.hide("terminal");
        ws.tasks.switch_to("code");
        ws.debug.breakpoints.toggle(&file.to_string_lossy(), 1);
        ws.debug.watches.push("name".into());
        ws.start_debug(window, cx);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| ws.terminal.read(cx).session_count() > 0) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Debugger did not start a terminal");
        std::thread::sleep(Duration::from_millis(20));
        cx.executor().advance_clock(Duration::from_millis(20));
    }
    ws.update_in(cx, |ws, window, cx| ws.terminal.update(cx, |t, cx| t.focus(window, cx)));
    cx.run_until_parked();
    cx.simulate_input("Ada");
    cx.simulate_keystrokes("enter");
    loop {
        cx.run_until_parked();
        if ws.read_with(cx, |ws, cx| {
            ws.debug_panel.read(cx).watches.iter().any(|(name, value, _)| name == "name" && value.contains("Ada"))
        }) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Debugger did not show the watch value");
        std::thread::sleep(Duration::from_millis(20));
        cx.executor().advance_clock(Duration::from_millis(20));
    }
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.tasks.current(), "debug", "Debugging should use the Debug task when enabled");
        assert_eq!(ws.debug.stopped.as_ref().map(|(_, line)| *line), Some(1));
        assert!(!ws.debug_panel.read(cx).frames.is_empty());
        assert!(ws.debug_panel.read(cx).variables.iter().any(|v| v.name == "name" && v.value.contains("Ada")));
    });
    ws.update(cx, |ws, cx| ws.stop_debug(cx));
    ws.read_with(cx, |ws, _| assert_eq!(ws.tasks.current(), "code"));
    cx.run_until_parked();
}
