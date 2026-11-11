//! Highlighting for JSON (and JSON with comments). One line at a time, like
//! the Python tokenizer; the only state carried between lines is being inside
//! a block comment. Replaces `src/renderer/editor/jsonTokenizer.ts`.

use crate::tokenizer::{LineState, Token, TokenType};
use regex::Regex;
use std::sync::LazyLock;

static NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?").expect("valid regex"));

fn find(text: &[char], from: usize, pat: &[char]) -> Option<usize> {
    (from..text.len()).find(|&i| text[i..].starts_with(pat))
}

pub fn tokenize_json(line: &str, state: LineState, comments: bool) -> (Vec<Token>, LineState) {
    let text: Vec<char> = line.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    if state == LineState::BlockComment {
        match find(&text, 0, &['*', '/']) {
            None => {
                if !text.is_empty() {
                    tokens.push(Token::new(0, text.len(), TokenType::Comment));
                }
                return (tokens, LineState::BlockComment);
            }
            Some(end) => {
                tokens.push(Token::new(0, end + 2, TokenType::Comment));
                i = end + 2;
            }
        }
    }
    while i < text.len() {
        let ch = text[i];
        if ch == '"' {
            let mut j = i + 1;
            while j < text.len() && text[j] != '"' {
                j += if text[j] == '\\' { 2 } else { 1 };
            }
            let end = (j + 1).min(text.len());
            // A string followed by ':' is an object key.
            let mut k = end;
            while k < text.len() && (text[k] == ' ' || text[k] == '\t') {
                k += 1;
            }
            let kind = if text.get(k) == Some(&':') { TokenType::Property } else { TokenType::String };
            tokens.push(Token::new(i, end, kind));
            i = end;
            continue;
        }
        if comments && ch == '/' && text.get(i + 1) == Some(&'/') {
            tokens.push(Token::new(i, text.len(), TokenType::Comment));
            break;
        }
        if comments && ch == '/' && text.get(i + 1) == Some(&'*') {
            match find(&text, i + 2, &['*', '/']) {
                None => {
                    tokens.push(Token::new(i, text.len(), TokenType::Comment));
                    return (tokens, LineState::BlockComment);
                }
                Some(end) => {
                    tokens.push(Token::new(i, end + 2, TokenType::Comment));
                    i = end + 2;
                    continue;
                }
            }
        }
        if ch == '-' || ch.is_ascii_digit() {
            let rest: String = text[i..].iter().collect();
            if let Some(m) = NUMBER.find(&rest) {
                if m.as_str() != "-" && !m.as_str().is_empty() {
                    let len = m.as_str().len();
                    tokens.push(Token::new(i, i + len, TokenType::Number));
                    i += len;
                    continue;
                }
            }
        }
        if ch.is_ascii_alphabetic() || ch == '_' || ch == '$' {
            let mut j = i + 1;
            while j < text.len() && (text[j].is_ascii_alphanumeric() || text[j] == '_' || text[j] == '$') {
                j += 1;
            }
            let word: String = text[i..j].iter().collect();
            if word == "true" || word == "false" || word == "null" {
                tokens.push(Token::new(i, j, TokenType::Constant));
            }
            i = j;
            continue;
        }
        i += 1;
    }
    (tokens, LineState::Normal)
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
    fn colors_keys_strings_numbers_and_constants() {
        let text = r#"  "name": "x\"y", "n": -1.5e3, "ok": true, "v": null"#;
        assert_eq!(
            pieces(text, &tokenize_json(text, LineState::Normal, false).0),
            [
                r#"property:"name""#,
                r#"string:"x\"y""#,
                r#"property:"n""#,
                "number:-1.5e3",
                r#"property:"ok""#,
                "constant:true",
                r#"property:"v""#,
                "constant:null"
            ]
        );
    }

    #[test]
    fn jsonc_line_and_block_comments_carried_over_lines() {
        let one = tokenize_json("{ // note", LineState::Normal, true);
        assert_eq!(pieces("{ // note", &one.0), ["comment:// note"]);
        let a = tokenize_json("\"a\": 1 /* start", LineState::Normal, true);
        assert_eq!(a.1, LineState::BlockComment);
        let b = tokenize_json("still */ \"b\": 2", a.1, true);
        assert_eq!(pieces("still */ \"b\": 2", &b.0), ["comment:still */", "property:\"b\"", "number:2"]);
        assert_eq!(b.1, LineState::Normal);
        // Plain JSON has no comments.
        assert_eq!(tokenize_json("// x", LineState::Normal, false).0, vec![]);
    }
}
