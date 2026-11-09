//! Git history and blame: parsing `git log` / `git blame --porcelain`, laying
//! out the commit graph (which column each commit goes in and which lines
//! connect it to its parents), and merge conflict blocks in a file.
//! Replaces `src/shared/history.ts` and `src/shared/conflicts.ts`.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub parents: Vec<String>,
    pub author: String,
    /// Seconds since 1970.
    pub time: i64,
    /// Branch / tag names pointing here ("HEAD -> main", "origin/main", "tag: v1").
    pub refs: Vec<String>,
    pub subject: String,
}

/// The --format used with `parse_log`: fields separated by NUL, commits by RS (\x1e).
pub const LOG_FORMAT: &str = "%H%x00%P%x00%an%x00%at%x00%D%x00%s%x1e";

pub fn parse_log(out: &str) -> Vec<Commit> {
    let is_sha =
        |s: &str| (7..=64).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
    let mut commits = Vec::new();
    for rec in out.split('\x1e') {
        let r = rec.trim_start();
        if r.is_empty() {
            continue;
        }
        let f: Vec<&str> = r.split('\0').collect();
        let get = |i: usize| f.get(i).copied().unwrap_or("");
        if !is_sha(get(0)) {
            continue;
        }
        commits.push(Commit {
            sha: get(0).to_owned(),
            parents: get(1).split(' ').filter(|p| !p.is_empty()).map(str::to_owned).collect(),
            author: get(2).to_owned(),
            time: get(3).parse().unwrap_or(0),
            refs: get(4).split(", ").filter(|p| !p.is_empty()).map(str::to_owned).collect(),
            subject: get(5).to_owned(),
        });
    }
    commits
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphRow {
    /// Column of this commit's dot.
    pub col: usize,
    /// Which commit each column expects next, entering this row (None = empty column).
    pub before: Vec<Option<String>>,
    /// ...and leaving it (towards older commits).
    pub after: Vec<Option<String>>,
    /// Columns that end at this commit (branches merged into it from above).
    pub joins: Vec<usize>,
    /// Columns of this commit's parents in `after`.
    pub parent_cols: Vec<usize>,
}

/// Lay out the graph for commits listed newest first (as `git log
/// --topo-order` gives them). Each row says where the dot is and how lines continue.
pub fn graph_layout(commits: &[Commit]) -> Vec<GraphRow> {
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    // The first empty column (not `avoid`), or a new one.
    let free = |lanes: &[Option<String>], avoid: Option<usize>| {
        lanes.iter().enumerate().position(|(k, l)| l.is_none() && Some(k) != avoid).unwrap_or(lanes.len())
    };
    let set = |lanes: &mut Vec<Option<String>>, i: usize, v: Option<String>| {
        if i >= lanes.len() {
            lanes.resize(i + 1, None);
        }
        lanes[i] = v;
    };
    for c in commits {
        let col = match lanes.iter().position(|l| l.as_deref() == Some(c.sha.as_str())) {
            Some(i) => i,
            None => {
                let i = free(&lanes, None);
                set(&mut lanes, i, Some(c.sha.clone()));
                i
            }
        };
        let before = lanes.clone();
        let mut joins = Vec::new();
        for (i, lane) in lanes.iter_mut().enumerate() {
            if i != col && lane.as_deref() == Some(c.sha.as_str()) {
                joins.push(i);
                *lane = None;
            }
        }
        let mut parent_cols = Vec::new();
        match c.parents.split_first() {
            None => lanes[col] = None,
            Some((first, rest)) => {
                lanes[col] = Some(first.clone());
                parent_cols.push(col);
                for p in rest {
                    let pc = match lanes.iter().position(|l| l.as_deref() == Some(p.as_str())) {
                        Some(i) => i,
                        None => {
                            let i = free(&lanes, Some(col));
                            set(&mut lanes, i, Some(p.clone()));
                            i
                        }
                    };
                    parent_cols.push(pc);
                }
            }
        }
        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }
        rows.push(GraphRow { col, before, after: lanes.clone(), joins, parent_cols });
    }
    rows
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlameLine {
    pub sha: String,
    pub author: String,
    pub time: i64,
    pub summary: String,
    /// Not committed yet (sha of zeros).
    pub uncommitted: bool,
}

/// `git blame --porcelain` -> one entry per line of the file (index = line).
pub fn parse_blame(out: &str) -> Vec<BlameLine> {
    use std::collections::HashMap;
    let mut info: HashMap<String, (String, i64, String)> = HashMap::new();
    let mut lines: Vec<Option<BlameLine>> = Vec::new();
    let (mut sha, mut final_line) = (String::new(), 0usize);
    let is_header = |l: &str| -> Option<(String, usize)> {
        let mut f = l.split(' ');
        let s = f.next()?;
        if !(s.len() == 40 || s.len() == 64) || !s.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let _orig: usize = f.next()?.parse().ok()?;
        let fin: usize = f.next()?.parse().ok()?;
        match f.next() {
            Some(n) if n.parse::<usize>().is_err() => return None,
            _ => {}
        }
        f.next().is_none().then(|| (s.to_owned(), fin.saturating_sub(1)))
    };
    for raw in out.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some((s, fin)) = is_header(line) {
            info.entry(s.clone()).or_default();
            (sha, final_line) = (s, fin);
            continue;
        }
        if line.starts_with('\t') {
            let (author, time, summary) = info.get(&sha).cloned().unwrap_or_default();
            if final_line >= lines.len() {
                lines.resize(final_line + 1, None);
            }
            let uncommitted = sha.chars().all(|c| c == '0');
            lines[final_line] = Some(BlameLine { sha: sha.clone(), author, time, summary, uncommitted });
            continue;
        }
        let Some(meta) = info.get_mut(&sha) else { continue };
        if let Some(a) = line.strip_prefix("author ") {
            meta.0 = a.to_owned();
        } else if let Some(t) = line.strip_prefix("author-time ") {
            meta.1 = t.parse().unwrap_or(0);
        } else if let Some(s) = line.strip_prefix("summary ") {
            meta.2 = s.to_owned();
        }
    }
    // Fill any gap (shouldn't happen) so indexes line up with the file.
    lines.into_iter().map(|l| l.unwrap_or(BlameLine { uncommitted: true, ..Default::default() })).collect()
}

/// "3 days ago", "2 months ago", "just now".
pub fn time_ago(seconds: i64, now_seconds: i64) -> String {
    let s = (now_seconds - seconds).max(0);
    const UNITS: [(i64, &str); 6] = [
        (31_536_000, "year"),
        (2_592_000, "month"),
        (604_800, "week"),
        (86_400, "day"),
        (3_600, "hour"),
        (60, "minute"),
    ];
    for (size, name) in UNITS {
        let n = s / size;
        if n >= 1 {
            return format!("{n} {name}{} ago", if n > 1 { "s" } else { "" });
        }
    }
    "just now".into()
}

// ---------- merge conflicts ----------

/// A merge conflict block in a file:
///
/// ```text
/// <<<<<<< HEAD            (current: your side)
/// ...
/// ||||||| base            (optional: the common ancestor, with diff3 style)
/// ...
/// =======
/// ...                     (incoming: the side being merged in)
/// >>>>>>> feature
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// Line of <<<<<<< (0-based).
    pub start: usize,
    /// Line of |||||||, if any.
    pub base: Option<usize>,
    /// Line of =======.
    pub separator: usize,
    /// Line of >>>>>>>.
    pub end: usize,
    pub current_label: String,
    pub incoming_label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Current,
    Incoming,
    Both,
    None,
}

/// Every complete conflict block, top to bottom.
pub fn find_conflicts<S: AsRef<str>>(lines: &[S]) -> Vec<Conflict> {
    // "<<<<<<<" alone or followed by a space (a label).
    let marker = |l: &str, m: &str| l.starts_with(m) && (l.len() == 7 || l.as_bytes()[7] == b' ');
    let label = |l: &str| l.get(8..).unwrap_or("").trim().to_owned();
    let mut out = Vec::new();
    // (start, label, base, separator)
    let mut open: Option<(usize, String, Option<usize>, Option<usize>)> = None;
    for (i, l) in lines.iter().map(AsRef::as_ref).enumerate() {
        if marker(l, "<<<<<<<") {
            open = Some((i, label(l), None, None));
            continue;
        }
        let Some((start, current, base, separator)) = open.as_mut() else { continue };
        if marker(l, "|||||||") && separator.is_none() {
            *base = Some(i);
        } else if l.starts_with("=======") && l[7..].trim().is_empty() {
            separator.get_or_insert(i);
        } else if marker(l, ">>>>>>>") {
            if let Some(sep) = *separator {
                out.push(Conflict {
                    start: *start,
                    base: *base,
                    separator: sep,
                    end: i,
                    current_label: current.clone(),
                    incoming_label: label(l),
                });
                open = None;
            }
        }
    }
    out
}

/// The lines that replace a conflict block (start..=end) for a choice.
pub fn resolve_conflict<S: AsRef<str>>(lines: &[S], c: &Conflict, choice: Resolution) -> Vec<String> {
    let take =
        |from: usize, to: usize| lines[from.min(to)..to].iter().map(|l| l.as_ref().to_owned()).collect::<Vec<_>>();
    let current = take(c.start + 1, c.base.unwrap_or(c.separator));
    let incoming = take(c.separator + 1, c.end);
    match choice {
        Resolution::Current => current,
        Resolution::Incoming => incoming,
        Resolution::Both => [current, incoming].concat(),
        Resolution::None => Vec::new(),
    }
}

/// The conflict the cursor is in, else the next one below it (wrapping to the first).
pub fn conflict_near(conflicts: &[Conflict], line: usize) -> Option<&Conflict> {
    conflicts
        .iter()
        .find(|c| line >= c.start && line <= c.end)
        .or_else(|| conflicts.iter().find(|c| c.start > line))
        .or_else(|| conflicts.first())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_reads_commits_with_parents_refs_and_subjects() {
        let out = "aaaaaaa1\0bbbbbbb2 ccccccc3\0Ada\x001700000000\0HEAD -> main, origin/main\0Merge branch x\x1e\nbbbbbbb2\0\0Bob\x001690000000\0\0first: with, commas\x1e\n";
        assert_eq!(
            parse_log(out),
            vec![
                Commit {
                    sha: "aaaaaaa1".into(),
                    parents: vec!["bbbbbbb2".into(), "ccccccc3".into()],
                    author: "Ada".into(),
                    time: 1_700_000_000,
                    refs: vec!["HEAD -> main".into(), "origin/main".into()],
                    subject: "Merge branch x".into()
                },
                Commit {
                    sha: "bbbbbbb2".into(),
                    parents: vec![],
                    author: "Bob".into(),
                    time: 1_690_000_000,
                    refs: vec![],
                    subject: "first: with, commas".into()
                },
            ]
        );
    }

    fn c(sha: &str, parents: &[&str]) -> Commit {
        Commit { sha: sha.into(), parents: parents.iter().map(|p| (*p).to_owned()).collect(), ..Default::default() }
    }

    fn lanes(l: &[&str]) -> Vec<Option<String>> {
        l.iter().map(|s| (!s.is_empty()).then(|| (*s).to_owned())).collect()
    }

    #[test]
    fn graph_keeps_a_straight_line_for_linear_history() {
        let rows = graph_layout(&[c("c", &["b"]), c("b", &["a"]), c("a", &[])]);
        assert_eq!(rows.iter().map(|r| r.col).collect::<Vec<_>>(), [0, 0, 0]);
        assert!(rows[2].after.is_empty());
    }

    #[test]
    fn graph_opens_a_column_for_a_merged_branch_and_joins_it_back() {
        // m merges f into main: m(b, f), f(a), b(a), a
        let rows = graph_layout(&[c("m", &["b", "f"]), c("f", &["a"]), c("b", &["a"]), c("a", &[])]);
        assert_eq!(
            (rows[0].col, rows[0].parent_cols.clone(), rows[0].after.clone()),
            (0, vec![0, 1], lanes(&["b", "f"]))
        );
        assert_eq!((rows[1].col, rows[1].after.clone()), (1, lanes(&["b", "a"])));
        // b also leads to a: the side column (also waiting for a) joins at a.
        assert_eq!((rows[2].col, rows[2].after.clone()), (0, lanes(&["a", "a"])));
        assert_eq!((rows[3].col, rows[3].joins.clone(), rows[3].after.clone()), (0, vec![1], vec![]));
    }

    #[test]
    fn blame_reads_author_time_and_summary_for_every_line() {
        let a = "a".repeat(40);
        let z = "0".repeat(40);
        let out = [
            format!("{a} 1 1 2"),
            "author Ada".into(),
            "author-time 1700000000".into(),
            "summary First commit".into(),
            "filename x.py".into(),
            "\tline one".into(),
            format!("{a} 2 2"),
            "\tline two".into(),
            format!("{z} 3 3 1"),
            "author Not Committed Yet".into(),
            "author-time 1800000000".into(),
            "summary Version of x.py from x.py".into(),
            "filename x.py".into(),
            "\tnew line".into(),
            String::new(),
        ]
        .join("\n");
        let b = parse_blame(&out);
        assert_eq!(b.len(), 3);
        assert_eq!(
            (b[0].sha.as_str(), b[0].author.as_str(), b[0].summary.as_str(), b[0].uncommitted),
            (a.as_str(), "Ada", "First commit", false)
        );
        assert_eq!(b[0].time, 1_700_000_000);
        assert_eq!(b[1].sha, a);
        assert!(b[2].uncommitted);
    }

    #[test]
    fn time_ago_says_roughly_how_long_ago() {
        let now = 1_700_000_000;
        assert_eq!(time_ago(now - 30, now), "just now");
        assert_eq!(time_ago(now - 3 * 86400, now), "3 days ago");
        assert_eq!(time_ago(now - 400 * 86400, now), "1 year ago");
        assert_eq!(time_ago(now + 50, now), "just now");
    }

    const LINES: [&str; 15] = [
        "a = 1",
        "<<<<<<< HEAD",
        "b = 2",
        "||||||| base",
        "b = 0",
        "=======",
        "b = 3",
        "b2 = 3",
        ">>>>>>> feature",
        "c = 4",
        "<<<<<<< ours",
        "=======",
        "d = 5",
        ">>>>>>> theirs",
        "<<<<<<< never closed",
    ];

    #[test]
    fn conflicts_are_found_as_complete_blocks() {
        assert_eq!(
            find_conflicts(&LINES),
            vec![
                Conflict {
                    start: 1,
                    base: Some(3),
                    separator: 5,
                    end: 8,
                    current_label: "HEAD".into(),
                    incoming_label: "feature".into()
                },
                Conflict {
                    start: 10,
                    base: None,
                    separator: 11,
                    end: 13,
                    current_label: "ours".into(),
                    incoming_label: "theirs".into()
                },
            ]
        );
    }

    #[test]
    fn conflicts_resolve_either_way_or_both() {
        let cs = find_conflicts(&LINES);
        assert_eq!(resolve_conflict(&LINES, &cs[0], Resolution::Current), ["b = 2"]);
        assert_eq!(resolve_conflict(&LINES, &cs[0], Resolution::Incoming), ["b = 3", "b2 = 3"]);
        assert_eq!(resolve_conflict(&LINES, &cs[0], Resolution::Both), ["b = 2", "b = 3", "b2 = 3"]);
        assert_eq!(resolve_conflict(&LINES, &cs[1], Resolution::Current), Vec::<String>::new());
        assert_eq!(resolve_conflict(&LINES, &cs[1], Resolution::None), Vec::<String>::new());
    }

    #[test]
    fn the_conflict_at_or_after_the_cursor_is_picked() {
        let cs = find_conflicts(&LINES);
        assert_eq!(conflict_near(&cs, 4).map(|c| c.start), Some(1));
        assert_eq!(conflict_near(&cs, 9).map(|c| c.start), Some(10));
        assert_eq!(conflict_near(&cs, 14).map(|c| c.start), Some(1)); // wraps
        assert_eq!(conflict_near(&[], 0), None);
    }
}
