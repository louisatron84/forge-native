use forge_core::templates::*;
use forge_sys::templates::*;
#[test]
fn builtins_render_names_paths_and_keep_foreign_template_syntax() {
    assert_eq!(module_name("My Tool!"), "my_tool");
    assert_eq!(module_name("2048 game"), "game_2048");
    assert_eq!(module_name("2048"), "p_2048");
    assert_eq!(title_of("weather-bot"), "Weather Bot");
    assert!(project_name_problem("").is_some());
    assert!(project_name_problem("a/b").is_some());
    for p in ["../x", "/etc/x", "C:/x", "a//b", "a/../b", "CON/file"] {
        assert!(!safe_template_path(p), "{p}");
    }
    for t in builtin_templates() {
        let files = render_template(&t, "Demo App");
        assert!(files.iter().any(|f| f.path == "README.md"));
        for f in files.iter().filter(|f| f.path.ends_with(".json")) {
            assert!(serde_json::from_str::<serde_json::Value>(&f.content).is_ok(), "{}", f.path);
        }
    }
    let t = builtin_templates().into_iter().find(|t| t.id == "flask").unwrap();
    assert!(render_template(&t, "x").iter().any(|f| f.content.contains("{{ note }}")));
}
#[test]
fn creates_once_and_round_trips_user_templates_without_secrets_or_binary_files() {
    let dir = tempfile::tempdir().unwrap();
    let t = builtin_templates().into_iter().find(|t| t.id == "cli").unwrap();
    let made = create_project(dir.path(), "Weather Bot", &render_template(&t, "Weather Bot")).unwrap();
    assert!(made.join("weather_bot.py").exists());
    assert!(made.join(".forge/run.json").exists());
    assert!(create_project(dir.path(), "Weather Bot", &[]).is_err());
    assert!(create_project(dir.path(), "Bad", &[TemplateFile { path: "../escape.py".into(), content: String::new() }])
        .is_err());
    assert!(!dir.path().join("Bad").exists());
    let root = dir.path().join("weather_bot");
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::create_dir_all(root.join(".venv")).unwrap();
    for (p, s) in [
        ("weather_bot.py", "print('weather_bot')\n"),
        ("tests/test_weather_bot.py", "import weather_bot\n"),
        (".env", "TOKEN=secret"),
        (".env.production", "TOKEN=secret"),
        (".venv/pyvenv.cfg", "x"),
    ] {
        std::fs::write(root.join(p), s).unwrap();
    }
    std::fs::write(root.join("logo.png"), [0x89, 0x50, 0, 1]).unwrap();
    let store = dir.path().join("templates");
    assert_eq!(save_template(&root, &store, "My bot", "").unwrap(), "my_bot");
    assert_eq!(save_template(&root, &store, "My bot", "").unwrap(), "my_bot_2");
    let users = list_user_templates(&store);
    assert_eq!(users.len(), 2);
    assert!(users[0].user);
    let mut paths = render_template(&users[0], "Rain Alert").iter().map(|f| f.path.clone()).collect::<Vec<_>>();
    paths.sort();
    assert_eq!(paths, ["rain_alert.py", "tests/test_rain_alert.py"]);
    assert!(!serde_json::to_string(&users).unwrap().contains("secret"));
}
