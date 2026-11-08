//! Simple highlighting for config files (TOML, YAML, INI/CFG): comments,
//! [section] headers, keys, strings, numbers and true/false. Line by line
//! with no state carried over. Replaces `src/renderer/editor/configTokenizer.ts`.

use crate::tokenizer::{Token, TokenType};
use regex::Regex;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug)]
pub struct ConfigSyntax {
    /// Characters that start a comment (at the start of a line or after a space).
    pub comment: &'static [char],
    /// Key separators: '=' (TOML, INI) and/or ':' (YAML, INI).
    pub separators: &'static [char],
}

pub const TOML: ConfigSyntax = ConfigSyntax { comment: &['#'], separators: &['='] };
pub const YAML: ConfigSyntax = ConfigSyntax { comment: &['#'], separators: &[':'] };
pub const INI: ConfigSyntax = ConfigSyntax { comment: &['#', ';'], separators: &['=', ':'] };

static NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[-+]?(?:0x[\da-fA-F_]+|\d[\d_]*(?:\.\d+)?(?:[eE][+-]?\d+)?)").expect("valid regex"));
static KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^("[^"]*"|'[^']*'|[A-Za-z0-9_.\-]+(?: [A-Za-z0-9_.\-]+)*)(\s*)([=:])"#).expect("valid regex")
});
const CONSTANTS: &[&str] = &["true", "false", "null", "True", "False", "yes", "no", "on", "off", "~"];

fn is_ascii_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

pub fn tokenize_config(line: &str, syntax: ConfigSyntax) -> Vec<Token> {
    let text: Vec<char> = line.chars().collect();
    let mut tokens = Vec::new();
    let indent = text.iter().take_while(|c| c.is_whitespace()).count();
    let Some(&first) = text.get(indent) else {
        return tokens;
    };
    if syntax.comment.contains(&first) {
        return vec![Token::new(indent, text.len(), TokenType::Comment)];
    }
    // [section] / [[array.of.tables]]
    if first == '[' && syntax.separators.contains(&'=') {
        if let Some(close) = text.iter().rposition(|&c| c == ']') {
            if close > indent {
                tokens.push(Token::new(indent, close + 1, TokenType::Class));
                if let Some(rest) = (close + 1..text.len()).find(|&i| syntax.comment.contains(&text[i])) {
                    tokens.push(Token::new(rest, text.len(), TokenType::Comment));
                }
                return tokens;
            }
        }
    }
    let mut i = indent;
    // key = / key: (YAML list items start with "- ")
    if text.get(i) == Some(&'-') && text.get(i + 1) == Some(&' ') {
        i += 2;
    }
    if let Some((end, sep)) = key_at(&text, i, syntax) {
        tokens.push(Token::new(i, end, TokenType::Property));
        i = sep + 1;
    }
    while i < text.len() {
        let ch = text[i];
        if ch == '"' || ch == '\'' {
            let mut j = i + 1;
            while j < text.len() && text[j] != ch {
                j += if ch == '"' && text[j] == '\\' { 2 } else { 1 };
            }
            let end = (j + 1).min(text.len());
            tokens.push(Token::new(i, end, TokenType::String));
            i = end;
            continue;
        }
        if syntax.comment.contains(&ch) && (i == 0 || text[i - 1].is_whitespace()) {
            tokens.push(Token::new(i, text.len(), TokenType::Comment));
            break;
        }
        let at_word_start = i == 0 || {
            let p = text[i - 1];
            p.is_whitespace() || "=:,[{(".contains(p)
        };
        if at_word_start {
            let rest: String = text[i..].iter().collect();
            if let Some(m) = NUMBER.find(&rest) {
                let len = m.as_str().len();
                // (?![\w.]): a number must not run into a word or a dot.
                let after = text.get(i + len).copied();
                if len > 0 && !after.is_some_and(|c| is_ascii_word(c) || c == '.') {
                    tokens.push(Token::new(i, i + len, TokenType::Number));
                    i += len;
                    continue;
                }
            }
            let w_len = text[i..].iter().take_while(|&&c| is_ascii_word(c) || c == '~').count();
            if w_len > 0 {
                let word: String = text[i..i + w_len].iter().collect();
                let after = text.get(i + w_len).copied();
                if CONSTANTS.contains(&word.as_str()) && after.is_none_or(|c| c.is_whitespace() || ",]}".contains(c)) {
                    tokens.push(Token::new(i, i + w_len, TokenType::Constant));
                }
                i += w_len;
                continue;
            }
        }
        i += 1;
    }
    tokens
}

/// A `key =` / `key:` starting at `from`: where the key text ends and where the separator is.
fn key_at(text: &[char], from: usize, syntax: ConfigSyntax) -> Option<(usize, usize)> {
    let rest: String = text[from..].iter().collect();
    let caps = KEY.captures(&rest)?;
    let sep_char = caps.get(3)?.as_str().chars().next()?;
    if !syntax.separators.contains(&sep_char) {
        return None;
    }
    let end = from + caps.get(1)?.as_str().chars().count();
    let sep = end + caps.get(2)?.as_str().chars().count();
    // YAML "key:" must be followed by a space or the end ("http://x" is not a key).
    if sep_char == ':' && !syntax.separators.contains(&'=') {
        if let Some(&next) = text.get(sep + 1) {
            if next != ' ' {
                return None;
            }
        }
    }
    Some((end, sep))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pieces(text: &str, tokens: &[Token]) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        tokens
            .iter()
            .map(|t| format!("{}:{}", t.kind.as_str(), chars[t.start..t.end].iter().collect::<String>()))
            .collect()
    }

    #[test]
    fn toml_sections_keys_strings_numbers_comments() {
        let s = "[tool.ruff]  # lint";
        assert_eq!(pieces(s, &tokenize_config(s, TOML)), ["class:[tool.ruff]", "comment:# lint"]);
        let line = "line-length = 100  # max";
        assert_eq!(pieces(line, &tokenize_config(line, TOML)), ["property:line-length", "number:100", "comment:# max"]);
        let s = "name = \"demo#1\"";
        assert_eq!(pieces(s, &tokenize_config(s, TOML)), ["property:name", "string:\"demo#1\""]);
    }

    #[test]
    fn yaml_keys_list_items_booleans_and_a_url_is_not_a_key() {
        assert_eq!(pieces("  - name: build", &tokenize_config("  - name: build", YAML)), ["property:name"]);
        assert_eq!(pieces("debug: true", &tokenize_config("debug: true", YAML)), ["property:debug", "constant:true"]);
        assert_eq!(
            pieces("# just a comment", &tokenize_config("# just a comment", YAML)),
            ["comment:# just a comment"]
        );
        assert_eq!(pieces("http://x.org", &tokenize_config("http://x.org", YAML)), Vec::<String>::new());
    }

    #[test]
    fn ini_semicolon_comments() {
        assert_eq!(pieces("; note", &tokenize_config("; note", INI)), ["comment:; note"]);
    }
}
