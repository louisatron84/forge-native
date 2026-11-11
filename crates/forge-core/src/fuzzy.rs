//! Fuzzy matching for the command palette and quick open: the query's letters
//! must appear in order, but not necessarily next to each other ("rpf" ->
//! "Run Python File"). Matches at word starts and in unbroken runs score
//! higher, so the most likely result comes first. Replaces
//! `src/renderer/fuzzy.ts`. Positions are char indices.

#[derive(Clone, Debug, PartialEq)]
pub struct FuzzyMatch {
    pub score: f64,
    /// Char index in the text of each matched character (for bold highlighting).
    pub positions: Vec<usize>,
}

fn is_boundary(ch: Option<char>) -> bool {
    ch.is_none_or(|c| c.is_whitespace() || matches!(c, '/' | '\\' | '_' | '-' | '.' | ':'))
}

fn is_upper(ch: char) -> bool {
    ch.is_uppercase()
}

fn lower(ch: char) -> char {
    ch.to_lowercase().next().unwrap_or(ch)
}

fn is_word_start(text: &[char], i: usize) -> bool {
    let prev = if i == 0 { None } else { text.get(i - 1).copied() };
    if is_boundary(prev) {
        return true;
    }
    // camelCase hump
    is_upper(text[i]) && prev.is_some_and(|p| !is_upper(p))
}

fn find(t: &[char], ch: char, from: usize) -> Option<usize> {
    t.get(from..)?.iter().position(|c| *c == ch).map(|p| p + from)
}

fn can_match_rest(q: &[char], mut qi: usize, t: &[char], mut from: usize) -> bool {
    while qi < q.len() {
        match find(t, q[qi], from) {
            Some(f) => from = f + 1,
            None => return false,
        }
        qi += 1;
    }
    true
}

/// Greedy left-to-right match, preferring word-start positions when one is
/// available later in the text for the same letter.
pub fn fuzzy_match(query: &str, text: &str) -> Option<FuzzyMatch> {
    if query.is_empty() {
        return Some(FuzzyMatch { score: 0.0, positions: Vec::new() });
    }
    let q: Vec<char> = query.chars().map(lower).collect();
    let orig: Vec<char> = text.chars().collect();
    let t: Vec<char> = orig.iter().copied().map(lower).collect();
    let mut positions: Vec<usize> = Vec::with_capacity(q.len());
    let mut from = 0;
    for (qi, &ch) in q.iter().enumerate() {
        let mut found = find(&t, ch, from)?;
        // If this char isn't continuing a run, try to land on a word start instead.
        let continues_run = positions.last().is_some_and(|p| found == p + 1);
        if !continues_run && !is_word_start(&orig, found) {
            for j in found + 1..t.len() {
                if t[j] == ch && is_word_start(&orig, j) {
                    // Only jump if the rest of the query can still match after j.
                    if can_match_rest(&q, qi + 1, &t, j + 1) {
                        found = j;
                    }
                    break;
                }
            }
        }
        positions.push(found);
        from = found + 1;
    }
    let mut score = 0.0;
    for (k, &p) in positions.iter().enumerate() {
        score += 1.0;
        if p == 0 {
            score += 8.0;
        } else if is_word_start(&orig, p) {
            score += 6.0;
        }
        if k > 0 && p == positions[k - 1] + 1 {
            score += 5.0;
        } else if k > 0 {
            score -= f64::min(3.0, (p - positions[k - 1] - 1) as f64 * 0.5);
        }
    }
    Some(FuzzyMatch { score, positions })
}

#[derive(Clone, Debug, PartialEq)]
pub struct FuzzyResult<T> {
    pub item: T,
    pub matched: FuzzyMatch,
}

/// Filter and sort items by how well they match. For file paths, matches in
/// the file name (after the last slash) get a bonus.
pub fn fuzzy_filter<T: Clone>(
    query: &str,
    items: &[T],
    text: impl Fn(&T) -> &str,
    limit: usize,
) -> Vec<FuzzyResult<T>> {
    let mut results: Vec<(FuzzyResult<T>, usize)> = Vec::new();
    for item in items {
        let s = text(item);
        let Some(mut m) = fuzzy_match(query, s) else { continue };
        let chars: Vec<char> = s.chars().collect();
        let name_start = chars.iter().rposition(|c| *c == '/' || *c == '\\').map_or(0, |i| i + 1);
        if name_start > 0 && m.positions.first().is_some_and(|p| *p >= name_start) {
            m.score += 10.0;
        }
        results.push((FuzzyResult { item: item.clone(), matched: m }, chars.len()));
    }
    // Stable sort: equal scores and lengths keep their order.
    results.sort_by(|a, b| b.0.matched.score.total_cmp(&a.0.matched.score).then(a.1.cmp(&b.1)));
    results.into_iter().take(limit).map(|(r, _)| r).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_characters_in_order_case_insensitively_and_reports_positions() {
        assert_eq!(fuzzy_match("rpf", "Run Python File").map(|m| m.positions), Some(vec![0, 4, 11]));
    }

    #[test]
    fn none_when_characters_are_missing_or_out_of_order() {
        assert_eq!(fuzzy_match("xyz", "Run Python File"), None);
        assert_eq!(fuzzy_match("fr", "Run File"), None);
    }

    #[test]
    fn an_empty_query_matches_everything_with_score_0() {
        assert_eq!(fuzzy_match("", "anything"), Some(FuzzyMatch { score: 0.0, positions: vec![] }));
    }

    #[test]
    fn scores_word_starts_and_runs_above_scattered_letters() {
        let score = |q: &str, t: &str| fuzzy_match(q, t).map(|m| m.score).unwrap_or(f64::MIN);
        assert!(score("sf", "Save File") > score("sf", "classify"));
        assert!(score("save", "Save File") > score("save", "Search And View Editor"));
    }

    #[test]
    fn path_separators_underscores_and_dots_are_word_boundaries() {
        assert_eq!(fuzzy_match("mfp", "src/main/file_parser.py").map(|m| m.positions), Some(vec![4, 9, 14]));
    }

    #[test]
    fn filter_sorts_by_score_and_prefers_shorter_text_on_ties() {
        let items = ["src/editor/editor.ts", "src/editor/editor.test.ts", "docs/readme.md"];
        let got: Vec<&str> = fuzzy_filter("editor", &items, |s| s, 100).into_iter().map(|r| r.item).collect();
        assert_eq!(got, ["src/editor/editor.ts", "src/editor/editor.test.ts"]);
    }

    #[test]
    fn a_match_in_the_file_name_beats_one_only_in_folders() {
        let items = ["find/notes.txt", "src/find.ts"];
        assert_eq!(fuzzy_filter("find", &items, |s| s, 100)[0].item, "src/find.ts");
    }
}
