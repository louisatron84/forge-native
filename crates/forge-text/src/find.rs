//! Search logic for find/replace, shared by in-file find and project search.
//! Replaces `src/renderer/editor/find.ts` and `src/shared/regex.ts`.
//!
//! Matches never span lines (one regex run per line). Regexes use Rust
//! syntax, which matches JavaScript's for everyday patterns (no lookaround
//! or backreferences); replacements accept the JavaScript forms the old app
//! used (`$1`, `$&`, `$<name>`, `$$`).

use crate::chars;
use crate::text_buffer::{Pos, TextBuffer};
use regex::{Regex, RegexBuilder};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

/// Build the regex for a query, or an error message for an invalid regex.
pub fn build_regex(query: &str, opts: FindOptions) -> Result<Regex, String> {
    let mut source = if opts.regex { query.to_owned() } else { regex::escape(query) };
    if opts.whole_word {
        source = format!(r"\b(?:{source})\b");
    }
    RegexBuilder::new(&source)
        .case_insensitive(!opts.case_sensitive)
        .build()
        .map_err(|e| e.to_string().lines().last().unwrap_or("invalid regular expression").trim().to_owned())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub start: Pos,
    pub end: Pos,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindResult {
    pub matches: Vec<Match>,
    pub error: Option<String>,
}

pub const DEFAULT_LIMIT: usize = 10_000;

pub fn find_matches<S: AsRef<str>>(lines: &[S], query: &str, opts: FindOptions, limit: usize) -> FindResult {
    if query.is_empty() {
        return FindResult::default();
    }
    let re = match build_regex(query, opts) {
        Ok(re) => re,
        Err(error) => return FindResult { matches: Vec::new(), error: Some(error) },
    };
    let mut matches = Vec::new();
    'lines: for (line, text) in lines.iter().enumerate() {
        let text = text.as_ref();
        for m in re.find_iter(text) {
            if m.as_str().is_empty() {
                continue; // skip zero-length matches like /x*/
            }
            let start = chars::col_of_byte(text, m.start());
            let end = start + chars::len(m.as_str());
            matches.push(Match { start: Pos::new(line, start), end: Pos::new(line, end) });
            if matches.len() >= limit {
                break 'lines;
            }
        }
    }
    FindResult { matches, error: None }
}

/// Index of the match to go to from `p`: the first one starting at/after `p`
/// (forward) or the last one starting before `p` (backward), wrapping around.
pub fn match_index_from(matches: &[Match], p: Pos, forward: bool) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    if forward {
        return Some(matches.iter().position(|m| m.start >= p).unwrap_or(0));
    }
    Some(matches.iter().rposition(|m| m.start < p).unwrap_or(matches.len() - 1))
}

/// A JavaScript-style replacement string (`$1`, `$&`, `$<name>`, `$$`) in the
/// syntax the regex crate uses (`${1}`, `${0}`, `${name}`, `$$`).
pub fn js_replacement(replacement: &str) -> String {
    let chars: Vec<char> = replacement.chars().collect();
    let mut out = String::with_capacity(replacement.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '$' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        match chars.get(i + 1) {
            Some('$') => {
                out.push_str("$$");
                i += 2;
            }
            Some('&') => {
                out.push_str("${0}");
                i += 2;
            }
            Some(d) if d.is_ascii_digit() => {
                let mut j = i + 1;
                while j < chars.len() && j < i + 3 && chars[j].is_ascii_digit() {
                    j += 1;
                }
                let n: String = chars[i + 1..j].iter().collect();
                out.push_str(&format!("${{{n}}}"));
                i = j;
            }
            Some('<') => match chars[i + 2..].iter().position(|&c| c == '>') {
                Some(close) => {
                    let name: String = chars[i + 2..i + 2 + close].iter().collect();
                    out.push_str(&format!("${{{name}}}"));
                    i += 3 + close;
                }
                None => {
                    out.push_str("$$");
                    i += 1;
                }
            },
            // Anything else: a literal "$".
            _ => {
                out.push_str("$$");
                i += 1;
            }
        }
    }
    out
}

/// The text to put in place of `matched`. In regex mode, $1 etc. refer to capture groups.
pub fn replacement_for(matched: &str, query: &str, replacement: &str, opts: FindOptions) -> String {
    if !opts.regex {
        return replacement.to_owned();
    }
    match build_regex(query, opts) {
        Ok(re) => re.replace(matched, js_replacement(replacement).as_str()).into_owned(),
        Err(_) => replacement.to_owned(),
    }
}

/// Replace every match (back to front so earlier positions stay valid). Returns the first match's start.
pub fn replace_all(b: &mut TextBuffer, matches: &[Match], replacement: &str, query: &str, opts: FindOptions) -> Pos {
    // Compile and translate once per operation, not once for every match.
    let regex = opts.regex.then(|| build_regex(query, opts).ok()).flatten();
    let expanded = regex.as_ref().map(|_| js_replacement(replacement));
    for m in matches.iter().rev() {
        let text = match (&regex, &expanded) {
            (Some(re), Some(expanded)) => re.replace(&b.range(m.start, m.end), expanded.as_str()).into_owned(),
            _ => replacement.to_owned(),
        };
        b.delete(m.start, m.end);
        b.insert(m.start, &text);
    }
    matches.first().map_or(Pos::new(0, 0), |m| m.start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{EditKind, History};
    use crate::text_buffer::pos;

    fn opts(case_sensitive: bool, whole_word: bool, regex: bool) -> FindOptions {
        FindOptions { case_sensitive, whole_word, regex }
    }

    fn m(a: (usize, usize), b: (usize, usize)) -> Match {
        Match { start: pos(a.0, a.1), end: pos(b.0, b.1) }
    }

    const LINES: [&str; 3] = ["Foo foo food", "bar.foo()", ""];

    #[test]
    #[ignore = "manual performance measurement"]
    fn benchmark_regex_replace_all() {
        let source = "get_café(1)\n".repeat(2000);
        let options = opts(false, false, true);
        let found = find_matches(&source.lines().collect::<Vec<_>>(), r"get_(\w+)", options, DEFAULT_LIMIT);
        let mut buffer = TextBuffer::new(&source);
        let start = std::time::Instant::now();
        replace_all(&mut buffer, &found.matches, "fetch_$1", r"get_(\w+)", options);
        eprintln!("replace_all 2000 regex matches: {:?}", start.elapsed());
        assert_eq!(buffer.text(), "fetch_café(1)\n".repeat(2000));
    }

    #[test]
    fn finds_plain_text_case_insensitive_by_default() {
        let r = find_matches(&LINES, "foo", FindOptions::default(), DEFAULT_LIMIT);
        assert_eq!(r.matches, [m((0, 0), (0, 3)), m((0, 4), (0, 7)), m((0, 8), (0, 11)), m((1, 4), (1, 7))]);
    }

    #[test]
    fn respects_case_sensitivity_and_whole_words() {
        assert_eq!(find_matches(&LINES, "Foo", opts(true, false, false), DEFAULT_LIMIT).matches.len(), 1);
        assert_eq!(find_matches(&LINES, "foo", opts(false, true, false), DEFAULT_LIMIT).matches.len(), 3);
    }

    #[test]
    fn treats_regex_characters_literally_unless_regex_mode_is_on() {
        assert_eq!(find_matches(&LINES, ".foo(", FindOptions::default(), DEFAULT_LIMIT).matches, [m((1, 3), (1, 8))]);
        assert_eq!(find_matches(&LINES, "fo+d?", opts(false, false, true), DEFAULT_LIMIT).matches.len(), 4);
    }

    #[test]
    fn reports_invalid_regexes_instead_of_failing() {
        let r = find_matches(&LINES, "(", opts(false, false, true), DEFAULT_LIMIT);
        assert!(r.matches.is_empty());
        assert!(r.error.is_some_and(|e| !e.is_empty()));
    }

    #[test]
    fn ignores_empty_queries_and_zero_length_regex_matches() {
        assert!(find_matches(&LINES, "", FindOptions::default(), DEFAULT_LIMIT).matches.is_empty());
        assert!(find_matches(&LINES, "x*", opts(false, false, true), DEFAULT_LIMIT).matches.is_empty());
    }

    #[test]
    fn stops_at_the_limit() {
        let many = vec!["aaaa"; 100];
        assert_eq!(find_matches(&many, "a", FindOptions::default(), 50).matches.len(), 50);
    }

    #[test]
    fn finds_the_next_match_at_or_after_a_position_wrapping_to_the_first() {
        let ms = find_matches(&["ab ab", "ab"], "ab", FindOptions::default(), DEFAULT_LIMIT).matches;
        assert_eq!(match_index_from(&ms, pos(0, 1), true), Some(1));
        assert_eq!(match_index_from(&ms, pos(0, 3), true), Some(1));
        assert_eq!(match_index_from(&ms, pos(1, 1), true), Some(0));
    }

    #[test]
    fn finds_the_previous_match_before_a_position_wrapping_to_the_last() {
        let ms = find_matches(&["ab ab", "ab"], "ab", FindOptions::default(), DEFAULT_LIMIT).matches;
        assert_eq!(match_index_from(&ms, pos(0, 3), false), Some(0));
        assert_eq!(match_index_from(&ms, pos(0, 0), false), Some(2));
    }

    #[test]
    fn returns_none_when_there_are_no_matches() {
        assert_eq!(match_index_from(&[], pos(0, 0), true), None);
    }

    #[test]
    fn builds_regex_replacements_with_capture_groups() {
        assert_eq!(replacement_for("get_user", r"get_(\w+)", "fetch_$1", opts(false, false, true)), "fetch_user");
        assert_eq!(replacement_for("a.b", "a.b", "$1x", FindOptions::default()), "$1x");
        assert_eq!(replacement_for("ab", "(a)(b)", "$2$1x", opts(false, false, true)), "bax");
        assert_eq!(replacement_for("ab", "ab", "[$&] $$5", opts(false, false, true)), "[ab] $5");
    }

    #[test]
    fn replaces_all_matches_as_one_undo_step() {
        let mut b = TextBuffer::new("foo = foo + 1\nprint(foo)");
        let mut h = History::new();
        let o = opts(false, true, false);
        let ms = find_matches(b.lines(), "foo", o, DEFAULT_LIMIT).matches;
        let n = h.transact(&mut b, EditKind::Other, pos(0, 0), |b| replace_all(b, &ms, "bar", "foo", o));
        assert_eq!(b.text(), "bar = bar + 1\nprint(bar)");
        assert_eq!(n, pos(0, 0));
        h.undo(&mut b);
        assert_eq!(b.text(), "foo = foo + 1\nprint(foo)");
    }

    #[test]
    fn match_columns_count_characters() {
        let r = find_matches(&["héllo wörld"], "wörld", FindOptions::default(), DEFAULT_LIMIT);
        assert_eq!(r.matches, [m((0, 6), (0, 11))]);
    }
}
