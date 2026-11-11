//! Finding a Python interpreter: the one set in Settings, else the project's
//! virtualenv, else python on PATH. Replaces `pythonCandidates` in
//! `src/main/python.ts`.

use std::path::{Path, PathBuf};

/// Interpreters to try, best first.
pub fn python_candidates(root: &str, windows: bool, configured: Option<&str>) -> Vec<String> {
    let mut list = Vec::new();
    if let Some(c) = configured.filter(|c| !c.is_empty()) {
        list.push(c.to_owned());
    }
    let root = root.trim_end_matches(['/', '\\']);
    for dir in [".venv", "venv", "env"] {
        list.push(if windows {
            format!("{root}\\{dir}\\Scripts\\python.exe")
        } else {
            format!("{root}/{dir}/bin/python")
        });
    }
    if windows {
        list.extend(["python".to_owned(), "py".to_owned()]);
    } else {
        list.extend(["python3".to_owned(), "python".to_owned()]);
    }
    list
}

/// A command without a folder is looked up on PATH.
pub fn is_bare_command(p: &str) -> bool {
    !p.contains(['/', '\\'])
}

/// Where a bare command is on PATH (with .exe on Windows), if anywhere.
pub fn which(command: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".into());
        std::iter::once(String::new()).chain(pathext.split(';').map(str::to_lowercase)).collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let candidate = dir.join(format!("{command}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// The interpreter to use: an existing path, or a command found on PATH.
/// A configured one is used as-is (if it's wrong, the run says so).
pub fn find_python(root: &Path, configured: Option<&str>) -> Option<String> {
    if let Some(c) = configured.filter(|c| !c.is_empty()) {
        return Some(c.to_owned());
    }
    python_candidates(&root.to_string_lossy(), cfg!(windows), None).into_iter().find(|c| {
        if is_bare_command(c) {
            which(c).is_some()
        } else {
            Path::new(c).is_file()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_a_project_virtualenv_then_python_on_path_windows() {
        assert_eq!(
            python_candidates("C:\\proj", true, None),
            [
                "C:\\proj\\.venv\\Scripts\\python.exe",
                "C:\\proj\\venv\\Scripts\\python.exe",
                "C:\\proj\\env\\Scripts\\python.exe",
                "python",
                "py"
            ]
        );
    }

    #[test]
    fn uses_bin_python_and_python3_elsewhere() {
        assert_eq!(
            python_candidates("/proj", false, None),
            ["/proj/.venv/bin/python", "/proj/venv/bin/python", "/proj/env/bin/python", "python3", "python"]
        );
    }

    #[test]
    fn puts_a_configured_interpreter_first() {
        assert_eq!(python_candidates("/proj", false, Some("/usr/bin/python3.12"))[0], "/usr/bin/python3.12");
        assert_eq!(find_python(Path::new("/nowhere"), Some("/x/python")).as_deref(), Some("/x/python"));
    }

    #[test]
    fn a_project_venv_is_found() {
        let dir = tempfile::tempdir().expect("temp");
        let exe = if cfg!(windows) {
            dir.path().join(".venv").join("Scripts").join("python.exe")
        } else {
            dir.path().join(".venv/bin/python")
        };
        std::fs::create_dir_all(exe.parent().expect("parent")).expect("mkdir");
        std::fs::write(&exe, "").expect("write");
        assert_eq!(find_python(dir.path(), None).map(PathBuf::from), Some(exe));
    }
}
