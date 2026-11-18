use forge_sys::{
    generated_history::History,
    user_state::{finish_legacy_import, plan_legacy_import},
};
use std::{io, path::Path};

fn import_legacy(old: &Path, native: &Path) -> io::Result<()> {
    let Some(files) = plan_legacy_import(old, native)? else { return Ok(()) };
    for file in files {
        assert!(!native.join(&file.relative).exists(), "Planning must leave the destination untouched");
        History::new(native).write(&native.join(&file.relative), None, &file.contents, "Import Electron settings")?;
    }
    finish_legacy_import(native)
}

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

#[test]
fn failed_import_is_retryable_and_does_not_leave_a_partial_file() {
    let old = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let native = target.path().join("native");
    let path = old.path().join("templates/demo/files/large.txt");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::File::create(&path).unwrap().set_len(17 * 1024 * 1024).unwrap();
    assert!(import_legacy(old.path(), &native).is_err());
    assert!(!native.join(".electron-imported").exists());
    assert!(!native.join("templates/demo/files/large.txt").exists());
    std::fs::write(&path, "recovered").unwrap();
    import_legacy(old.path(), &native).unwrap();
    assert_eq!(std::fs::read_to_string(native.join("templates/demo/files/large.txt")).unwrap(), "recovered");
}
