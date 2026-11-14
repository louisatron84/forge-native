use forge_sys::user_state::import_legacy;

#[test]
fn imports_compatible_state_and_templates_once_without_overwriting_native_files() {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    for name in [
        "settings.json",
        "keybindings.json",
        "layouts.json",
        "recent.json",
        "workspaces/abc.json",
        "templates/demo/template.json",
        "templates/demo/files/main.py",
    ] {
        let path = old.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, if name.ends_with(".py") { "print(1)" } else { "{}" }).unwrap();
    }
    std::fs::write(new.path().join("settings.json"), "{\"tabSize\":2}").unwrap();
    std::fs::write(old.path().join("token.txt"), "secret").unwrap();
    import_legacy(old.path(), new.path()).unwrap();
    assert_eq!(std::fs::read_to_string(new.path().join("settings.json")).unwrap(), "{\"tabSize\":2}");
    assert_eq!(std::fs::read_to_string(new.path().join("templates/demo/files/main.py")).unwrap(), "print(1)");
    assert!(new.path().join("workspaces/abc.json").exists());
    assert!(!new.path().join("token.txt").exists());
    std::fs::remove_file(new.path().join("recent.json")).unwrap();
    import_legacy(old.path(), new.path()).unwrap();
    assert!(!new.path().join("recent.json").exists(), "first-run import must not revive cleared state");
    assert!(old.path().join("recent.json").exists());
}
