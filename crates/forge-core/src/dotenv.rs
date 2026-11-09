//! The project's `.env` file as text: reading variables, changing one while
//! keeping every other line exactly as it was, and merging with the real
//! environment (which always wins). Replaces `src/shared/dotenv.ts`.

use regex::Regex;
use std::collections::BTreeMap;
use std::sync::LazyLock;

static LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=(.*)$").expect("regex"));

fn unquote(raw: &str) -> String {
    let v = raw.trim();
    if let Some(rest) = v.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = rest.chars();
        while let Some(ch) = chars.next() {
            match ch {
                '\\' => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some('t') => out.push('\t'),
                    Some(c) => out.push(c),
                    None => out.push('\\'),
                },
                '"' => return out,
                c => out.push(c),
            }
        }
        return out;
    }
    if let Some(rest) = v.strip_prefix('\'') {
        return match rest.find('\'') {
            Some(end) => rest[..end].to_owned(),
            None => rest.to_owned(),
        };
    }
    // An unquoted value ends at " #" (a comment).
    let cut =
        v.char_indices().find(|(i, c)| *c == '#' && *i > 0 && v[..*i].ends_with(char::is_whitespace)).map(|(i, _)| i);
    match cut {
        Some(i) => v[..i].trim().to_owned(),
        None => v.to_owned(),
    }
}

pub fn parse_dotenv(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        if let Some(c) = LINE.captures(line) {
            out.insert(c[2].to_owned(), unquote(&c[3]));
        }
    }
    out
}

fn quote(value: &str) -> String {
    let needs = value.is_empty() || value.chars().any(|c| c.is_whitespace() || matches!(c, '#' | '"' | '\'' | '\\'));
    if !value.is_empty() && !needs {
        return value.to_owned();
    }
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r");
    format!("\"{escaped}\"")
}

/// Set one variable: replaces its first line (keeping `export`), else appends it.
pub fn set_dotenv_var(text: &str, name: &str, value: &str) -> String {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_owned()).collect();
    for line in &mut lines {
        if line.trim_start().starts_with('#') {
            continue;
        }
        if let Some(c) = LINE.captures(line) {
            if &c[2] == name {
                let export = c.get(1).map_or("", |m| m.as_str()).to_owned();
                *line = format!("{export}{name}={}", quote(value));
                return lines.join(eol);
            }
        }
    }
    let body = if text.is_empty() || text.ends_with('\n') { text.to_owned() } else { format!("{text}{eol}") };
    format!("{body}{name}={}{eol}", quote(value))
}

/// The environment for a process: `.env` only fills names the real environment
/// doesn't have (compared without case on Windows, where PATH and Path are one).
pub fn merge_env(
    base: &BTreeMap<String, String>,
    dotenv: &BTreeMap<String, String>,
    windows: bool,
) -> BTreeMap<String, String> {
    let mut out = base.clone();
    for (k, v) in dotenv {
        let present = if windows { base.keys().any(|b| b.eq_ignore_ascii_case(k)) } else { base.contains_key(k) };
        if !present {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> String {
        [
            "# keys for the API box",
            "",
            "export A=1",
            "B=\"x y\"",
            "C='z # not a comment'",
            "D=a=b",
            "E=v # note",
            "F=\"line\\nbreak\"",
            "not a variable line",
            "",
        ]
        .join("\r\n")
    }

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn parses_the_usual_forms() {
        assert_eq!(
            parse_dotenv(&sample()),
            map(&[
                ("A", "1"),
                ("B", "x y"),
                ("C", "z # not a comment"),
                ("D", "a=b"),
                ("E", "v"),
                ("F", "line\nbreak")
            ])
        );
    }

    #[test]
    fn replaces_one_variable_and_keeps_every_other_line() {
        assert_eq!(set_dotenv_var(&sample(), "B", "new"), sample().replace("B=\"x y\"", "B=new"));
    }

    #[test]
    fn keeps_export_and_appends_new_variables() {
        assert!(set_dotenv_var(&sample(), "A", "2").contains("\r\nexport A=2\r\n"));
        assert_eq!(set_dotenv_var("X=1\n", "Y", "2"), "X=1\nY=2\n");
        assert_eq!(set_dotenv_var("X=1", "Y", "2"), "X=1\nY=2\n");
        assert_eq!(set_dotenv_var("", "Y", "2"), "Y=2\n");
    }

    #[test]
    fn quotes_values_that_need_it() {
        assert_eq!(set_dotenv_var("", "K", "a b"), "K=\"a b\"\n");
        assert_eq!(set_dotenv_var("", "K", "say \"hi\""), "K=\"say \\\"hi\\\"\"\n");
        assert_eq!(parse_dotenv(&set_dotenv_var("", "K", "say \"hi\" # x")), map(&[("K", "say \"hi\" # x")]));
    }

    #[test]
    fn the_real_environment_wins() {
        assert_eq!(
            merge_env(&map(&[("A", "1")]), &map(&[("A", "2"), ("B", "3")]), false),
            map(&[("A", "1"), ("B", "3")])
        );
        assert_eq!(merge_env(&map(&[("Path", "x")]), &map(&[("PATH", "y")]), true), map(&[("Path", "x")]));
    }
}
