//! Workspace integration tests run only on GPUI's headless platform.
use super::*;
use gpui_kit::{TestAppContext, VisualTestContext};

fn setup(cx: &mut TestAppContext) -> (tempfile::TempDir, Entity<Workspace>, &mut VisualTestContext) {
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
            ws.tasks.hide("terminal");
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
        assert_eq!(ws.root.as_deref(), Some(project.path()));
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
    ws.read_with(cx, |ws, _| assert_eq!(ws.root, Some(parent.path().join("my-project"))));
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
    ws.read_with(cx, |ws, cx| assert_eq!(ws.active_doc().unwrap().read(cx).doc.path(), path.to_string_lossy()));
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
        assert_eq!(state.breakpoints[0].path, file.to_string_lossy());
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
        assert_eq!(ws.debug.stopped.as_ref().map(|(_, line)| *line), Some(1));
        assert!(!ws.debug_panel.read(cx).frames.is_empty());
        assert!(ws.debug_panel.read(cx).variables.iter().any(|v| v.name == "name" && v.value.contains("Ada")));
    });
    ws.update(cx, |ws, cx| ws.stop_debug(cx));
    cx.run_until_parked();
}
