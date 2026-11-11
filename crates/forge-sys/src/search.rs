//! Search across every file in the project (Ctrl+Shift+F). Uses the same
//! options and regex rules as in-file find, skips the folders the file tree
//! hides, binary files and very large files, and stops after a maximum number
//! of results so a too-broad search can't hang the IDE. Replaces
//! `src/main/search.ts`. Columns and lengths count chars.

use crate::files::list_files;
use forge_text::find::{build_regex, FindOptions};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    /// The line, shortened for display.
    pub text: String,
    /// Where the match begins in `text` (chars).
    pub start: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    /// 0-based line and column (chars).
    pub line: usize,
    pub col: usize,
    pub length: usize,
    pub preview: Preview,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileResult {
    pub path: PathBuf,
    pub rel: String,
    pub matches: Vec<SearchMatch>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchResult {
    pub files: Vec<FileResult>,
    pub total: usize,
    pub truncated: bool,
    /// Files not searched because they're too big (relative paths).
    pub skipped: Vec<String>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct SearchLimits {
    pub max_results: usize,
    pub max_file_bytes: u64,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self { max_results: 5000, max_file_bytes: 20_000_000 }
    }
}

const PREVIEW_BEFORE: usize = 40;
const PREVIEW_MAX: usize = 120;

/// Shorten a line for the results list, keeping the match visible.
pub fn make_preview(line: &str, col: usize, length: usize) -> Preview {
    let chars: Vec<char> = line.chars().collect();
    let indent = chars.iter().take_while(|c| c.is_whitespace()).count();
    let mut from = indent.min(col);
    let mut prefix = "";
    if col - from > PREVIEW_BEFORE {
        from = col - PREVIEW_BEFORE;
        prefix = "…";
    }
    let mut to = chars.len();
    let mut suffix = "";
    if to - from > PREVIEW_MAX {
        to = (col + length).max(from + PREVIEW_MAX).min(chars.len());
        suffix = if to < chars.len() { "…" } else { "" };
    }
    let body: String = chars[from..to].iter().collect();
    Preview { text: format!("{prefix}{body}{suffix}"), start: prefix.chars().count() + col - from }
}

/// A file is treated as binary if its first 8 KB contain a NUL byte.
fn is_binary(buf: &[u8]) -> bool {
    buf[..buf.len().min(8192)].contains(&0)
}

/// Search the project. `cancelled` is checked between files (a newer search started).
pub fn search_project(
    root: &Path,
    query: &str,
    opts: FindOptions,
    limits: SearchLimits,
    cancelled: &dyn Fn() -> bool,
) -> SearchResult {
    let mut result = SearchResult::default();
    if query.is_empty() {
        return result;
    }
    let re = match build_regex(query, opts) {
        Ok(re) => re,
        Err(e) => {
            result.error = Some(e);
            return result;
        }
    };
    let mut files = list_files(root, 200_000);
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    'files: for file in files {
        if cancelled() {
            break;
        }
        let Ok(meta) = std::fs::metadata(&file.path) else { continue };
        if meta.len() > limits.max_file_bytes {
            result.skipped.push(file.rel.clone());
            continue;
        }
        let Ok(buf) = std::fs::read(&file.path) else { continue };
        if is_binary(&buf) {
            continue;
        }
        let text = String::from_utf8_lossy(&buf);
        let mut matches = Vec::new();
        for (line_no, line) in text.split('\n').enumerate() {
            let line = line.strip_suffix('\r').unwrap_or(line);
            for m in re.find_iter(line) {
                if m.as_str().is_empty() {
                    continue;
                }
                let col = line[..m.start()].chars().count();
                let length = m.as_str().chars().count();
                matches.push(SearchMatch { line: line_no, col, length, preview: make_preview(line, col, length) });
                result.total += 1;
                if result.total >= limits.max_results {
                    break;
                }
            }
            if result.total >= limits.max_results {
                break;
            }
        }
        if !matches.is_empty() {
            result.files.push(FileResult { path: file.path, rel: file.rel, matches });
        }
        if result.total >= limits.max_results {
            result.truncated = true;
            break 'files;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(case_sensitive: bool, whole_word: bool, regex: bool) -> FindOptions {
        FindOptions { case_sensitive, whole_word, regex }
    }

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp");
        let r = dir.path();
        std::fs::create_dir_all(r.join("pkg")).expect("mkdir");
        std::fs::create_dir_all(r.join("node_modules")).expect("mkdir");
        std::fs::write(r.join("main.py"), "import pkg\nuser = get_user(1)\nprint(user)\n").expect("write");
        std::fs::write(r.join("pkg").join("users.py"), "def get_user(id):\n    return User(id)\n").expect("write");
        std::fs::write(r.join("node_modules").join("skip.js"), "get_user").expect("write");
        std::fs::write(
            r.join("image.png"),
            [0x89, 0x50, 0x4e, 0x47, 0, 0, 0, 0x67, 0x65, 0x74, 0x5f, 0x75, 0x73, 0x65, 0x72],
        )
        .expect("write");
        dir
    }

    fn search(root: &Path, q: &str, o: FindOptions) -> SearchResult {
        search_project(root, q, o, SearchLimits::default(), &|| false)
    }

    #[test]
    fn finds_matches_grouped_by_file_with_line_and_column() {
        let p = project();
        let r = search(p.path(), "get_user", opts(false, false, false));
        assert_eq!(r.files.iter().map(|f| f.rel.as_str()).collect::<Vec<_>>(), ["main.py", "pkg/users.py"]);
        assert_eq!(
            r.files[0].matches,
            vec![SearchMatch { line: 1, col: 7, length: 8, preview: make_preview("user = get_user(1)", 7, 8) }]
        );
        assert_eq!(r.total, 2);
        assert!(!r.truncated);
    }

    #[test]
    fn skips_hidden_folders_and_binary_files() {
        let p = project();
        let r = search(p.path(), "get_user", opts(false, false, false));
        assert!(!r.files.iter().any(|f| f.rel.contains("node_modules") || f.rel.ends_with(".png")));
    }

    #[test]
    fn respects_case_whole_word_and_regex() {
        let p = project();
        assert_eq!(search(p.path(), "user", opts(true, false, false)).total, 4);
        assert_eq!(search(p.path(), "user", opts(false, true, false)).total, 3);
        assert_eq!(search(p.path(), r"get_\w+\(", opts(false, false, true)).total, 2);
    }

    #[test]
    fn reports_invalid_regexes_and_ignores_empty_queries() {
        let p = project();
        let r = search(p.path(), "(", opts(false, false, true));
        assert!(r.error.is_some());
        assert!(r.files.is_empty());
        assert!(search(p.path(), "", opts(false, false, false)).files.is_empty());
    }

    #[test]
    fn stops_at_the_limit_and_skips_big_files() {
        let p = project();
        let r = search_project(
            p.path(),
            "u",
            opts(false, false, false),
            SearchLimits { max_results: 3, ..Default::default() },
            &|| false,
        );
        assert_eq!(r.total, 3);
        assert!(r.truncated);
        let r = search_project(
            p.path(),
            "get_user",
            opts(false, false, false),
            SearchLimits { max_file_bytes: 40, ..Default::default() },
            &|| false,
        );
        assert_eq!(r.files.iter().map(|f| f.rel.as_str()).collect::<Vec<_>>(), ["pkg/users.py"]);
        assert_eq!(r.skipped, ["main.py"]);
    }

    #[test]
    fn previews_trim_indentation_and_cut_long_lines() {
        assert_eq!(make_preview("    return User(id)", 11, 4), Preview { text: "return User(id)".into(), start: 7 });
        let line = format!("{}NEEDLE{}", "x".repeat(200), "y".repeat(200));
        let p = make_preview(&line, 200, 6);
        assert!(p.text.starts_with('…') && p.text.ends_with('…'));
        let chars: Vec<char> = p.text.chars().collect();
        assert_eq!(chars[p.start..p.start + 6].iter().collect::<String>(), "NEEDLE");
        assert!(chars.len() < 140);
    }
}
