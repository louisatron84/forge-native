//! Reading git's output: `git status` (porcelain v2 for Source Control, v1
//! for the file tree's badges), unified diffs (split into hunks you can stage
//! one by one), and branch lists. Replaces `src/shared/git.ts` and the pure
//! part of `src/main/git.ts`.

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    Untracked,
    Conflict,
    Typechange,
}

impl ChangeKind {
    /// The badge letter (M, A, D, R, C, U, !, T).
    pub fn letter(self) -> &'static str {
        match self {
            ChangeKind::Modified => "M",
            ChangeKind::Added => "A",
            ChangeKind::Deleted => "D",
            ChangeKind::Renamed => "R",
            ChangeKind::Copied => "C",
            ChangeKind::Untracked => "U",
            ChangeKind::Conflict => "!",
            ChangeKind::Typechange => "T",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    /// Repo-relative, with / separators.
    pub path: String,
    /// For renames / copies: the old path.
    pub from: Option<String>,
    pub kind: ChangeKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoStatus {
    pub branch: Option<String>,
    /// e.g. origin/main (None if the branch doesn't track one).
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// HEAD isn't on a branch.
    pub detached: bool,
    /// No commits yet.
    pub initial: bool,
    /// The current commit id (None before the first commit).
    pub head: Option<String>,
    pub staged: Vec<FileChange>,
    pub unstaged: Vec<FileChange>,
    pub conflicts: Vec<FileChange>,
}

/// Parse `git status --porcelain=v2 --branch -z --untracked-files=all`.
pub fn parse_status_v2(out: &str) -> RepoStatus {
    let mut s = RepoStatus::default();
    let kind = |c: char| match c {
        'A' => ChangeKind::Added,
        'D' => ChangeKind::Deleted,
        'R' => ChangeKind::Renamed,
        'C' => ChangeKind::Copied,
        'T' => ChangeKind::Typechange,
        _ => ChangeKind::Modified,
    };
    let parts: Vec<&str> = out.split('\0').collect();
    let mut i = 0;
    while i < parts.len() {
        let p = parts[i];
        i += 1;
        if p.is_empty() {
            continue;
        }
        if let Some(header) = p.strip_prefix("# ") {
            let (key, val) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.head" if val == "(detached)" => s.detached = true,
                "branch.head" => s.branch = Some(val.to_owned()),
                "branch.oid" if val == "(initial)" => s.initial = true,
                "branch.oid" => s.head = Some(val.to_owned()),
                "branch.upstream" => s.upstream = Some(val.to_owned()),
                "branch.ab" => {
                    if let Some((a, b)) = val.strip_prefix('+').and_then(|v| v.split_once(" -")) {
                        if let (Ok(a), Ok(b)) = (a.parse(), b.parse()) {
                            (s.ahead, s.behind) = (a, b);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let chars: Vec<char> = p.chars().take(4).collect();
        match chars.first() {
            Some('?') => s.unstaged.push(FileChange {
                path: p.get(2..).unwrap_or("").to_owned(),
                from: None,
                kind: ChangeKind::Untracked,
            }),
            // u XY sub m1 m2 m3 mW h1 h2 h3 path
            Some('u') => {
                s.conflicts.push(FileChange { path: fields(p, 10).to_owned(), from: None, kind: ChangeKind::Conflict })
            }
            Some(t @ ('1' | '2')) if chars.len() == 4 => {
                let (x, y) = (chars[2], chars[3]);
                // 1 XY sub mH mI mW hH hI path   |   2 XY sub mH mI mW hH hI Xscore path \0 origPath
                let path = fields(p, if *t == '1' { 8 } else { 9 }).to_owned();
                let from = (*t == '2').then(|| {
                    i += 1;
                    parts.get(i - 1).map(|s| (*s).to_owned())
                });
                let from = from.flatten();
                if x != '.' {
                    let from = from.clone().filter(|_| x == 'R' || x == 'C');
                    s.staged.push(FileChange { path: path.clone(), from, kind: kind(x) });
                }
                if y != '.' {
                    s.unstaged.push(FileChange { path, from: None, kind: kind(y) });
                }
            }
            _ => {}
        }
    }
    s
}

/// Everything after the first `n` space-separated fields (the path may contain spaces).
fn fields(line: &str, n: usize) -> &str {
    let mut rest = line;
    for _ in 0..n {
        rest = rest.split_once(' ').map_or("", |(_, r)| r);
    }
    rest
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

impl LineKind {
    pub fn prefix(self) -> char {
        match self {
            LineKind::Context => ' ',
            LineKind::Added => '+',
            LineKind::Removed => '-',
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
    /// 1-based line numbers in the old / new file (None where the line doesn't exist).
    pub old_no: Option<usize>,
    pub new_no: Option<usize>,
    /// "\ No newline at end of file" follows this line.
    pub no_newline: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    /// The "@@ -a,b +c,d @@ context" line.
    pub header: String,
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    /// The lines before the first hunk (diff --git, index, ---, +++).
    pub head: Vec<String>,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub binary: bool,
    pub hunks: Vec<Hunk>,
}

/// Split a unified diff (`git diff`) into files and hunks.
pub fn parse_diff(text: &str) -> Vec<FileDiff> {
    static HUNK: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@").expect("regex"));
    let mut files: Vec<FileDiff> = Vec::new();
    let (mut old_no, mut new_no) = (0usize, 0usize);
    let mut in_hunk = false;
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    for raw in lines {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.starts_with("diff --git ") {
            files.push(FileDiff { head: vec![line.to_owned()], ..Default::default() });
            in_hunk = false;
            continue;
        }
        let Some(file) = files.last_mut() else { continue };
        if let Some(h) = HUNK.captures(line) {
            let num = |i: usize, default: usize| h.get(i).map_or(default, |m| m.as_str().parse().unwrap_or(default));
            let hunk = Hunk {
                header: line.to_owned(),
                old_start: num(1, 0),
                old_lines: num(2, 1),
                new_start: num(3, 0),
                new_lines: num(4, 1),
                lines: Vec::new(),
            };
            (old_no, new_no) = (hunk.old_start, hunk.new_start);
            file.hunks.push(hunk);
            in_hunk = true;
            continue;
        }
        if !in_hunk {
            file.head.push(line.to_owned());
            if let Some(p) = line.strip_prefix("--- ") {
                file.old_path = (line != "--- /dev/null").then(|| p.strip_prefix("a/").unwrap_or(p).to_owned());
            } else if let Some(p) = line.strip_prefix("+++ ") {
                file.new_path = (line != "+++ /dev/null").then(|| p.strip_prefix("b/").unwrap_or(p).to_owned());
            } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
                file.binary = true;
            }
            continue;
        }
        let Some(hunk) = file.hunks.last_mut() else { continue };
        if line.starts_with('\\') {
            if let Some(last) = hunk.lines.last_mut() {
                last.no_newline = true;
            }
            continue;
        }
        let text = line.get(1..).unwrap_or("").to_owned();
        let l = match line.chars().next() {
            Some('+') => {
                new_no += 1;
                DiffLine { kind: LineKind::Added, text, old_no: None, new_no: Some(new_no - 1), no_newline: false }
            }
            Some('-') => {
                old_no += 1;
                DiffLine { kind: LineKind::Removed, text, old_no: Some(old_no - 1), new_no: None, no_newline: false }
            }
            _ => {
                old_no += 1;
                new_no += 1;
                DiffLine {
                    kind: LineKind::Context,
                    text,
                    old_no: Some(old_no - 1),
                    new_no: Some(new_no - 1),
                    no_newline: false,
                }
            }
        };
        hunk.lines.push(l);
    }
    files
}

/// A patch with just one hunk of a file (for `git apply` to stage / unstage / discard it).
pub fn hunk_patch(file: &FileDiff, hunk: &Hunk) -> String {
    let mut out: Vec<String> = file.head.clone();
    out.push(hunk.header.clone());
    for l in &hunk.lines {
        out.push(format!("{}{}", l.kind.prefix(), l.text));
        if l.no_newline {
            out.push("\\ No newline at end of file".into());
        }
    }
    out.join("\n") + "\n"
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub current: bool,
    pub remote: bool,
    pub upstream: Option<String>,
}

/// The `for-each-ref` format `parse_branches` reads.
pub const BRANCH_FORMAT: &str = "%(refname)%00%(HEAD)%00%(upstream:short)";

/// Parse `git for-each-ref --format=<BRANCH_FORMAT> refs/heads refs/remotes`.
pub fn parse_branches(out: &str) -> Vec<Branch> {
    let mut list = Vec::new();
    for line in out.lines() {
        let mut f = line.split('\0');
        let (Some(r), head, upstream) = (f.next(), f.next().unwrap_or(""), f.next().unwrap_or("")) else { continue };
        if let Some(name) = r.strip_prefix("refs/heads/") {
            list.push(Branch {
                name: name.to_owned(),
                current: head == "*",
                remote: false,
                upstream: (!upstream.is_empty()).then(|| upstream.to_owned()),
            });
        } else if let Some(name) = r.strip_prefix("refs/remotes/").filter(|_| !r.ends_with("/HEAD")) {
            list.push(Branch { name: name.to_owned(), current: false, remote: true, upstream: None });
        }
    }
    list
}

/// A branch name git would accept (roughly `git check-ref-format --branch`).
pub fn is_valid_branch_name(name: &str) -> bool {
    let bad_char = |c: char| c.is_whitespace() || "~^:?*[\\".contains(c) || (c as u32) < 0x20 || c == '\u{7f}';
    !name.is_empty()
        && name != "@"
        && !name.starts_with('-')
        && !name.starts_with('/')
        && !name.ends_with('/')
        && !name.ends_with('.')
        && !name.ends_with(".lock")
        && !name.contains("..")
        && !name.contains("//")
        && !name.contains("@{")
        && !name.chars().any(bad_char)
}

// ---------- the file tree's badges (porcelain v1) ----------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PorcelainEntry {
    pub x: char,
    pub y: char,
    pub path: String,
    pub from: Option<String>,
}

/// Parse `git status --porcelain=v1 -z`.
pub fn parse_porcelain(out: &str) -> Vec<PorcelainEntry> {
    let parts: Vec<&str> = out.split('\0').collect();
    let mut entries = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let p = parts[i];
        i += 1;
        let mut chars = p.chars();
        let (Some(x), Some(y)) = (chars.next(), chars.next()) else { continue };
        if p.len() < 4 {
            continue;
        }
        let from = if x == 'R' || x == 'C' {
            i += 1;
            parts.get(i - 1).map(|s| (*s).to_owned())
        } else {
            None
        };
        entries.push(PorcelainEntry { x, y, path: p[3..].to_owned(), from });
    }
    entries
}

/// A file's state for the tree, from git's two status letters.
pub fn classify(x: char, y: char) -> ChangeKind {
    match (x, y) {
        ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D') => ChangeKind::Conflict,
        ('?', '?') => ChangeKind::Untracked,
        ('A', _) => ChangeKind::Added,
        ('R' | 'C', _) => ChangeKind::Renamed,
        ('D', _) | (_, 'D') => ChangeKind::Deleted,
        _ => ChangeKind::Modified,
    }
}

/// Changed files (absolute paths) and the folders that contain them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusMap {
    pub files: BTreeMap<String, ChangeKind>,
    pub folders: Vec<String>,
}

/// Turn repo-relative entries into absolute file paths, plus the folders
/// (inside the opened folder) that contain changes, for the dot on folders.
pub fn build_status_map(repo_root: &str, opened_folder: &str, entries: &[PorcelainEntry]) -> StatusMap {
    let windows = repo_root.contains('\\');
    let sep = if windows { '\\' } else { '/' };
    let trim = |p: &str| p.trim_end_matches(['\\', '/']).to_owned();
    let (root, opened) = (trim(repo_root), trim(opened_folder));
    let lower = |p: &str| if windows { p.to_lowercase() } else { p.to_owned() };
    let opened_prefix = lower(&format!("{opened}{sep}"));
    let mut map = StatusMap::default();
    for e in entries {
        let abs = format!("{root}{sep}{}", e.path.replace('/', &sep.to_string()));
        if !lower(&abs).starts_with(&opened_prefix) {
            continue; // outside the opened folder
        }
        map.files.insert(abs.clone(), classify(e.x, e.y));
        let mut dir = abs[..abs.rfind(sep).unwrap_or(0)].to_owned();
        while dir.len() > opened.len() && lower(&dir).starts_with(&opened_prefix) {
            if !map.folders.contains(&dir) {
                map.folders.push(dir.clone());
            }
            dir.truncate(dir.rfind(sep).unwrap_or(0));
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fc(path: &str, kind: ChangeKind) -> FileChange {
        FileChange { path: path.into(), from: None, kind }
    }

    #[test]
    fn status_v2_reads_the_branch_and_staged_unstaged_untracked_conflicted_files() {
        let out = [
            "# branch.oid 1234567890abcdef",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -1",
            "1 M. N... 100644 100644 100644 aaa bbb src/app.py",
            "1 .M N... 100644 100644 100644 aaa bbb README.md",
            "1 MM N... 100644 100644 100644 aaa bbb both.py",
            "1 A. N... 000000 100644 100644 000 bbb new file.py",
            "2 R. N... 100644 100644 100644 aaa bbb R100 lib/new.py",
            "lib/old.py",
            "u UU N... 100644 100644 100644 100644 a b c clash.py",
            "? notes.txt",
            "",
        ]
        .join("\0");
        let s = parse_status_v2(&out);
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!((s.ahead, s.behind, s.detached, s.initial), (2, 1, false, false));
        assert_eq!(s.head.as_deref(), Some("1234567890abcdef"));
        assert_eq!(
            s.staged,
            vec![
                fc("src/app.py", ChangeKind::Modified),
                fc("both.py", ChangeKind::Modified),
                fc("new file.py", ChangeKind::Added),
                FileChange { path: "lib/new.py".into(), from: Some("lib/old.py".into()), kind: ChangeKind::Renamed },
            ]
        );
        assert_eq!(
            s.unstaged,
            vec![
                fc("README.md", ChangeKind::Modified),
                fc("both.py", ChangeKind::Modified),
                fc("notes.txt", ChangeKind::Untracked)
            ]
        );
        assert_eq!(s.conflicts, vec![fc("clash.py", ChangeKind::Conflict)]);
    }

    #[test]
    fn status_v2_knows_a_detached_head_and_a_repo_with_no_commits() {
        let s = parse_status_v2("# branch.oid abc\0# branch.head (detached)\0");
        assert!(s.detached && s.branch.is_none());
        let s = parse_status_v2("# branch.oid (initial)\0# branch.head main\0");
        assert!(s.initial && s.head.is_none());
        assert_eq!(s.branch.as_deref(), Some("main"));
    }

    const DIFF: &str = "diff --git a/app.py b/app.py
index 1111111..2222222 100644
--- a/app.py
+++ b/app.py
@@ -1,4 +1,4 @@
 import os
-x = 1
+x = 2
 y = 3
 z = 4
@@ -10,3 +10,4 @@ def main():
     a()
     b()
+    c()
 end
\\ No newline at end of file
diff --git a/new.py b/new.py
new file mode 100644
index 0000000..3333333
--- /dev/null
+++ b/new.py
@@ -0,0 +1 @@
+print(\"hi\")
";

    #[test]
    fn diff_splits_files_and_hunks_with_line_numbers() {
        let files = parse_diff(DIFF);
        assert_eq!(files.len(), 2);
        let (app, added) = (&files[0], &files[1]);
        assert_eq!(
            (app.old_path.as_deref(), app.new_path.as_deref(), app.binary),
            (Some("app.py"), Some("app.py"), false)
        );
        assert_eq!(app.hunks.len(), 2);
        let rows: Vec<(char, Option<usize>, Option<usize>)> =
            app.hunks[0].lines.iter().map(|l| (l.kind.prefix(), l.old_no, l.new_no)).collect();
        assert_eq!(
            rows,
            vec![
                (' ', Some(1), Some(1)),
                ('-', Some(2), None),
                ('+', None, Some(2)),
                (' ', Some(3), Some(3)),
                (' ', Some(4), Some(4))
            ]
        );
        let h = &app.hunks[1];
        assert_eq!((h.old_start, h.old_lines, h.new_start, h.new_lines), (10, 3, 10, 4));
        assert_eq!(h.lines[3].text, "end");
        assert!(h.lines[3].no_newline);
        assert_eq!((added.old_path.as_deref(), added.new_path.as_deref()), (None, Some("new.py")));
        let h = &added.hunks[0];
        assert_eq!((h.old_start, h.old_lines, h.new_start, h.new_lines), (0, 0, 1, 1));
    }

    #[test]
    fn diff_notices_binary_files_and_windows_line_ends() {
        let f = parse_diff("diff --git a/x.png b/x.png\nindex 1..2 100644\nBinary files a/x.png and b/x.png differ\n");
        assert!(f[0].binary);
        let f = parse_diff("diff --git a/a b/a\r\n--- a/a\r\n+++ b/a\r\n@@ -1 +1 @@\r\n-x\r\n+y\r\n");
        assert_eq!(f[0].hunks[0].lines[1].text, "y");
    }

    #[test]
    fn hunk_patch_keeps_the_file_header() {
        let files = parse_diff(DIFF);
        let app = &files[0];
        let expected = [
            "diff --git a/app.py b/app.py",
            "index 1111111..2222222 100644",
            "--- a/app.py",
            "+++ b/app.py",
            "@@ -10,3 +10,4 @@ def main():",
            "     a()",
            "     b()",
            "+    c()",
            " end",
            "\\ No newline at end of file",
            "",
        ]
        .join("\n");
        assert_eq!(hunk_patch(app, &app.hunks[1]), expected);
    }

    #[test]
    fn branches_are_read_local_and_remote() {
        let out = [
            "refs/heads/main\0*\0origin/main",
            "refs/heads/feature/x\0 \0",
            "refs/remotes/origin/HEAD\0 \0",
            "refs/remotes/origin/main\0 \0",
            "",
        ]
        .join("\n");
        assert_eq!(
            parse_branches(&out),
            vec![
                Branch { name: "main".into(), current: true, remote: false, upstream: Some("origin/main".into()) },
                Branch { name: "feature/x".into(), current: false, remote: false, upstream: None },
                Branch { name: "origin/main".into(), current: false, remote: true, upstream: None },
            ]
        );
    }

    #[test]
    fn branch_names_are_checked() {
        for ok in ["main", "feature/login", "fix-1.2", "user/louis/thing"] {
            assert!(is_valid_branch_name(ok), "{ok}");
        }
        for bad in
            ["", "-x", "a..b", "a b", "a~1", "x.lock", "a/", "a//b", "@", "a:b", "what?", "/a", "a.", "a@{b", "a\u{7f}"]
        {
            assert!(!is_valid_branch_name(bad), "{bad:?}");
        }
    }

    fn z(entries: &[&str]) -> String {
        entries.join("\0") + "\0"
    }

    #[test]
    fn porcelain_v1_reads_codes_and_paths_with_spaces_and_renames() {
        let e = |x, y, path: &str| PorcelainEntry { x, y, path: path.into(), from: None };
        assert_eq!(
            parse_porcelain(&z(&[" M src/app.py", "?? new file.py", "A  added.py", " D gone.py"])),
            vec![
                e(' ', 'M', "src/app.py"),
                e('?', '?', "new file.py"),
                e('A', ' ', "added.py"),
                e(' ', 'D', "gone.py")
            ]
        );
        assert_eq!(
            parse_porcelain(&z(&["R  new.py", "old.py", " M other.py"])),
            vec![
                PorcelainEntry { x: 'R', y: ' ', path: "new.py".into(), from: Some("old.py".into()) },
                e(' ', 'M', "other.py")
            ]
        );
        assert_eq!(parse_porcelain(""), Vec::new());
    }

    #[test]
    fn classify_maps_git_codes_to_simple_states() {
        use ChangeKind::*;
        let cases = [
            (' ', 'M', Modified),
            ('M', ' ', Modified),
            ('?', '?', Untracked),
            ('A', ' ', Added),
            ('A', 'M', Added),
            (' ', 'D', Deleted),
            ('R', ' ', Renamed),
            ('U', 'U', Conflict),
            ('A', 'A', Conflict),
            ('D', 'D', Conflict),
        ];
        for (x, y, want) in cases {
            assert_eq!(classify(x, y), want, "{x}{y}");
        }
    }

    #[test]
    fn status_map_gives_absolute_paths_and_marks_parent_folders() {
        let map = build_status_map(
            "C:\\repo",
            "C:\\repo\\sub",
            &parse_porcelain(&z(&[" M sub/pkg/a.py", "?? sub/b.py", " M outside.py"])),
        );
        let files: Vec<(&str, ChangeKind)> = map.files.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        assert_eq!(
            files,
            vec![("C:\\repo\\sub\\b.py", ChangeKind::Untracked), ("C:\\repo\\sub\\pkg\\a.py", ChangeKind::Modified)]
        );
        assert_eq!(map.folders, vec!["C:\\repo\\sub\\pkg".to_owned()]);
        let map = build_status_map("/r", "/r", &parse_porcelain(&z(&["A  x/y.py"])));
        assert_eq!(map.files.get("/r/x/y.py"), Some(&ChangeKind::Added));
        assert_eq!(map.folders, vec!["/r/x".to_owned()]);
    }
}
