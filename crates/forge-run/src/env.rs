//! The environment for processes started for a project: the real
//! environment, plus the project's `.env` for names it doesn't have, plus
//! extras. Replaces `processEnvSync` in `src/main/dotenvFile.ts`, and the
//! `.env` file edits used by the API box.

use forge_core::dotenv::{merge_env, parse_dotenv, set_dotenv_var};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

pub fn env_path(root: &Path) -> PathBuf {
    root.join(".env")
}

/// The project's `.env` variables (none if there's no file).
pub fn read_dotenv(root: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(env_path(root)).map(|t| parse_dotenv(&t)).unwrap_or_default()
}

/// Set one variable in the project's `.env`, keeping every other line.
pub fn set_dotenv_value(root: &Path, name: &str, value: &str) -> io::Result<()> {
    let ok = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !ok {
        return Err(io::Error::other(format!("\"{name}\" isn't a valid variable name (letters, digits and _).")));
    }
    let text = std::fs::read_to_string(env_path(root)).unwrap_or_default();
    std::fs::write(env_path(root), set_dotenv_var(&text, name, value))
}

/// The variables that have a value (names only: the window never needs the secrets).
pub fn env_names(root: &Path) -> Vec<String> {
    read_dotenv(root).into_iter().filter(|(_, v)| !v.is_empty()).map(|(k, _)| k).collect()
}

const IGNORES_ENV: [&str; 6] = [".env", "/.env", "*.env", ".env*", "/.env*", "**/.env"];

pub fn gitignore_has_env(root: &Path) -> bool {
    std::fs::read_to_string(root.join(".gitignore")).is_ok_and(|t| t.lines().any(|l| IGNORES_ENV.contains(&l.trim())))
}

pub fn add_env_to_gitignore(root: &Path) -> io::Result<()> {
    let path = root.join(".gitignore");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let sep = if !text.is_empty() && !text.ends_with('\n') { eol } else { "" };
    std::fs::write(path, format!("{text}{sep}.env{eol}"))
}

/// The environment for a process: real environment + `.env` (for names it lacks) + `extra`.
pub fn process_env(root: Option<&Path>, extra: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let base: BTreeMap<String, String> = std::env::vars().collect();
    let dotenv = root.map(read_dotenv).unwrap_or_default();
    let mut out = merge_env(&base, &dotenv, cfg!(windows));
    for (k, v) in extra {
        // On Windows a different spelling of the same name replaces it.
        if cfg!(windows) {
            out.retain(|old, _| !old.eq_ignore_ascii_case(k) || old == k);
        }
        out.insert(k.clone(), v.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotenv_fills_missing_names_and_extras_win() {
        let dir = tempfile::tempdir().expect("temp");
        std::fs::write(dir.path().join(".env"), "FORGE_TEST_ONLY_IN_DOTENV=yes\nPATH=nope\n").expect("write");
        let extra = BTreeMap::from([("FORGE_EXTRA".to_owned(), "1".to_owned())]);
        let env = process_env(Some(dir.path()), &extra);
        assert_eq!(env.get("FORGE_TEST_ONLY_IN_DOTENV").map(String::as_str), Some("yes"));
        assert_ne!(env.iter().find(|(k, _)| k.eq_ignore_ascii_case("PATH")).map(|(_, v)| v.as_str()), Some("nope"));
        assert_eq!(env.get("FORGE_EXTRA").map(String::as_str), Some("1"));
    }

    #[test]
    fn edits_dotenv_and_gitignore() {
        let dir = tempfile::tempdir().expect("temp");
        set_dotenv_value(dir.path(), "API_KEY", "a b").expect("set");
        set_dotenv_value(dir.path(), "EMPTY", "").expect("set");
        assert_eq!(env_names(dir.path()), ["API_KEY"]);
        assert!(set_dotenv_value(dir.path(), "bad name", "x").is_err());
        assert!(!gitignore_has_env(dir.path()));
        std::fs::write(dir.path().join(".gitignore"), "target").expect("write");
        add_env_to_gitignore(dir.path()).expect("add");
        assert_eq!(std::fs::read_to_string(dir.path().join(".gitignore")).expect("read"), "target\n.env\n");
        assert!(gitignore_has_env(dir.path()));
    }
}
