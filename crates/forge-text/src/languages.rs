//! Which language a file is, by its name. Replaces `src/shared/languages.ts`.
//! The editor uses this to pick highlighting, comments and indentation
//! (modes.rs), and the language services use `server` to send the file to
//! the right language server.

#[derive(Debug, PartialEq, Eq)]
pub struct LanguageInfo {
    pub id: &'static str,
    /// Shown in the status bar.
    pub name: &'static str,
    /// Lower-case, with the dot.
    pub extensions: &'static [&'static str],
    /// Exact file names (lower-case), for files without a telling extension.
    pub file_names: &'static [&'static str],
    /// The language server that handles it, if any.
    pub server: Option<&'static str>,
}

impl LanguageInfo {
    /// The languageId sent to the language server.
    pub fn lsp_id(&self) -> &'static str {
        self.id
    }
}

pub static PYTHON: LanguageInfo = LanguageInfo {
    id: "python",
    name: "Python",
    extensions: &[".py", ".pyi", ".pyw"],
    file_names: &[],
    server: Some("pyright"),
};
pub static JSON: LanguageInfo =
    LanguageInfo { id: "json", name: "JSON", extensions: &[".json"], file_names: &[], server: None };
/// JSON with comments: editor and tool settings files that allow them.
pub static JSONC: LanguageInfo = LanguageInfo {
    id: "jsonc",
    name: "JSON with Comments",
    extensions: &[".jsonc"],
    file_names: &["settings.json", "keybindings.json", "run.json", "tsconfig.json", "jsconfig.json", ".eslintrc.json"],
    server: None,
};
pub static TOML: LanguageInfo =
    LanguageInfo { id: "toml", name: "TOML", extensions: &[".toml"], file_names: &["pipfile"], server: None };
pub static YAML: LanguageInfo =
    LanguageInfo { id: "yaml", name: "YAML", extensions: &[".yaml", ".yml"], file_names: &[], server: None };
pub static INI: LanguageInfo = LanguageInfo {
    id: "ini",
    name: "INI",
    extensions: &[".ini", ".cfg", ".conf"],
    file_names: &[".editorconfig", ".flake8", ".pylintrc", ".coveragerc"],
    server: None,
};
pub static PLAIN_TEXT: LanguageInfo =
    LanguageInfo { id: "plaintext", name: "Plain Text", extensions: &[], file_names: &[], server: None };

pub static LANGUAGES: [&LanguageInfo; 7] = [&PYTHON, &JSON, &JSONC, &TOML, &YAML, &INI, &PLAIN_TEXT];

/// The language of a file (Plain Text if nothing matches).
pub fn language_for_path(path: &str) -> &'static LanguageInfo {
    let name = path.rsplit(['/', '\\']).next().unwrap_or("").to_lowercase();
    if let Some(lang) = LANGUAGES.iter().find(|l| l.file_names.contains(&name.as_str())) {
        return lang;
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => {
            let ext = &name[dot..];
            LANGUAGES.iter().find(|l| l.extensions.contains(&ext)).copied().unwrap_or(&PLAIN_TEXT)
        }
        _ => &PLAIN_TEXT,
    }
}

pub fn language_by_id(id: &str) -> &'static LanguageInfo {
    LANGUAGES.iter().find(|l| l.id == id).copied().unwrap_or(&PLAIN_TEXT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_language_by_extension_or_file_name() {
        assert_eq!(language_for_path("C:\\proj\\main.py").id, "python");
        assert_eq!(language_for_path("/proj/stubs/x.PYI").id, "python");
        assert_eq!(language_for_path("/proj/data.json").id, "json");
        assert_eq!(language_for_path("/proj/.ideforfun/settings.json").id, "jsonc");
        assert_eq!(language_for_path("/proj/pyproject.toml").id, "toml");
        assert_eq!(language_for_path("/proj/.github/ci.yml").id, "yaml");
        assert_eq!(language_for_path("/proj/setup.cfg").id, "ini");
        assert_eq!(language_for_path("/proj/README").id, "plaintext");
        assert_eq!(language_for_path("/proj/notes.txt").id, "plaintext");
        assert_eq!(language_for_path("/proj/.gitignore").id, "plaintext");
    }

    #[test]
    fn only_python_has_a_language_server_so_far() {
        assert_eq!(language_for_path("a.py").server, Some("pyright"));
        assert_eq!(language_for_path("a.json").server, None);
    }
}
