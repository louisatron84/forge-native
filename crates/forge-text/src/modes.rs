//! Editor behaviour per language: highlighting, the line-comment marker for
//! Ctrl+/, and when Enter should indent one more level. Replaces
//! `src/renderer/editor/modes.ts`.

use crate::commands::python_indent_after;
use crate::config_tokenizer::tokenize_config;
use crate::json_tokenizer::tokenize_json;
use crate::languages::{self, language_for_path, LanguageInfo};
use crate::tokenizer::{tokenize_line, LineState, Token};

pub type TokenizeFn = fn(&str, LineState) -> (Vec<Token>, LineState);

#[derive(Debug)]
pub struct LanguageMode {
    pub language: &'static LanguageInfo,
    /// Tokens for one line, given the state at its start.
    pub tokenize: TokenizeFn,
    /// For Ctrl+/ (`None`: the language has no line comments).
    pub line_comment: Option<&'static str>,
    /// Should Enter indent one more level after this text (the line up to the cursor)?
    pub indent_after: fn(&str) -> bool,
    /// String prefixes like f"..." (Python), for auto-closing quotes after a letter.
    pub string_prefixes: bool,
}

/// An opening bracket at the end of the line (Enter right between brackets is handled separately).
fn opens_bracket(before: &str) -> bool {
    matches!(before.trim_end().chars().last(), Some('{' | '[' | '('))
}

fn yaml_indent_after(before: &str) -> bool {
    if opens_bracket(before) {
        return true;
    }
    // "key:" opens a nested block (ignoring a comment after it).
    let cut = before.char_indices().find(|&(i, c)| c == '#' && i > 0 && before[..i].ends_with(char::is_whitespace));
    let code = cut.map_or(before, |(i, _)| &before[..i]);
    code.trim_end().ends_with(':')
}

fn never(_: &str) -> bool {
    false
}

fn tokenize_json_plain(text: &str, state: LineState) -> (Vec<Token>, LineState) {
    tokenize_json(text, state, false)
}
fn tokenize_jsonc(text: &str, state: LineState) -> (Vec<Token>, LineState) {
    tokenize_json(text, state, true)
}
fn tokenize_toml(text: &str, _: LineState) -> (Vec<Token>, LineState) {
    (tokenize_config(text, crate::config_tokenizer::TOML), LineState::Normal)
}
fn tokenize_yaml(text: &str, _: LineState) -> (Vec<Token>, LineState) {
    (tokenize_config(text, crate::config_tokenizer::YAML), LineState::Normal)
}
fn tokenize_ini(text: &str, _: LineState) -> (Vec<Token>, LineState) {
    (tokenize_config(text, crate::config_tokenizer::INI), LineState::Normal)
}
fn tokenize_nothing(_: &str, _: LineState) -> (Vec<Token>, LineState) {
    (Vec::new(), LineState::Normal)
}

pub static PYTHON: LanguageMode = LanguageMode {
    language: &languages::PYTHON,
    tokenize: tokenize_line,
    line_comment: Some("#"),
    indent_after: python_indent_after,
    string_prefixes: true,
};
pub static JSON: LanguageMode = LanguageMode {
    language: &languages::JSON,
    tokenize: tokenize_json_plain,
    line_comment: None,
    indent_after: opens_bracket,
    string_prefixes: false,
};
pub static JSONC: LanguageMode = LanguageMode {
    language: &languages::JSONC,
    tokenize: tokenize_jsonc,
    line_comment: Some("//"),
    indent_after: opens_bracket,
    string_prefixes: false,
};
pub static TOML: LanguageMode = LanguageMode {
    language: &languages::TOML,
    tokenize: tokenize_toml,
    line_comment: Some("#"),
    indent_after: opens_bracket,
    string_prefixes: false,
};
pub static YAML: LanguageMode = LanguageMode {
    language: &languages::YAML,
    tokenize: tokenize_yaml,
    line_comment: Some("#"),
    indent_after: yaml_indent_after,
    string_prefixes: false,
};
pub static INI: LanguageMode = LanguageMode {
    language: &languages::INI,
    tokenize: tokenize_ini,
    line_comment: Some("#"),
    indent_after: opens_bracket,
    string_prefixes: false,
};
pub static PLAIN: LanguageMode = LanguageMode {
    language: &languages::PLAIN_TEXT,
    tokenize: tokenize_nothing,
    line_comment: None,
    indent_after: never,
    string_prefixes: false,
};

/// The editing mode for a file, by its path.
pub fn mode_for(path: &str) -> &'static LanguageMode {
    mode_by_id(language_for_path(path).id)
}

pub fn mode_by_id(id: &str) -> &'static LanguageMode {
    match id {
        "python" => &PYTHON,
        "json" => &JSON,
        "jsonc" => &JSONC,
        "toml" => &TOML,
        "yaml" => &YAML,
        "ini" => &INI,
        _ => &PLAIN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::newline;
    use crate::pairs::{toggle_comment, type_char};
    use crate::selection::Selection;
    use crate::text_buffer::{pos, TextBuffer};

    #[test]
    fn enter_indents_after_colon_in_python_after_brace_in_json_never_in_plain_text() {
        let enter = |path: &str, text: &str| {
            let mut b = TextBuffer::new(text);
            newline(&mut b, pos(0, text.chars().count()), 4, mode_for(path).indent_after);
            b.line(1).to_owned()
        };
        assert_eq!(enter("a.py", "if x:"), "    ");
        assert_eq!(enter("a.json", "\"a\": {"), "    ");
        assert_eq!(enter("a.json", "\"a\": 1,"), "");
        assert_eq!(enter("a.txt", "Note:"), "");
        assert_eq!(enter("a.yml", "jobs:"), "    ");
        assert_eq!(enter("a.yml", "jobs:  # all of them"), "    ");
    }

    #[test]
    fn ctrl_slash_uses_the_language_comment_marker() {
        let mut b = TextBuffer::new("\"a\": 1");
        let marker = mode_for("run.json").line_comment.expect("jsonc has comments");
        toggle_comment(&mut b, Selection::cursor(pos(0, 0)), marker);
        assert_eq!(b.text(), "// \"a\": 1");
        toggle_comment(&mut b, Selection::cursor(pos(0, 0)), "//");
        assert_eq!(b.text(), "\"a\": 1");
        assert_eq!(mode_for("data.json").line_comment, None);
        assert_eq!(mode_for("setup.cfg").line_comment, Some("#"));
    }

    #[test]
    fn docstring_quotes_and_f_prefixes_are_python_only() {
        let quote = |path: &str, text: &str| {
            let mut b = TextBuffer::new(text);
            type_char(&mut b, Selection::cursor(pos(0, text.chars().count())), '"', mode_for(path).string_prefixes);
            b.text()
        };
        assert_eq!(quote("a.py", "x = f"), "x = f\"\"");
        assert_eq!(quote("a.json", "\"a\": f"), "\"a\": f");
    }

    #[test]
    fn modes_are_shared_statics() {
        assert!(std::ptr::eq(mode_for("a.py"), mode_for("b.py")));
        assert_eq!(mode_for("README").language.id, "plaintext");
    }
}
