//! Git commands for Source Control, the file tree's badges, history and
//! blame: status, diffs, staging (whole files or single hunks via `git
//! apply`), commit, branches, fetch / pull / push, log, blame. git runs
//! without a shell and with fixed arguments; file paths always come after
//! `--`, so they can't be mistaken for options. Everything here blocks: call
//! it from a background thread. Replaces `src/main/gitOps.ts` and `git.ts`.

use crate::process::{run_captured, Captured};
use forge_core::git::{
    build_status_map, is_valid_branch_name, parse_branches, parse_porcelain, parse_status_v2, Branch, RepoStatus,
    StatusMap, BRANCH_FORMAT,
};
use forge_core::history::{parse_blame, parse_log, BlameLine, Commit, LOG_FORMAT};
use std::path::Path;
use std::time::Duration;

/// What a git command gave back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitResult {
    pub ok: bool,
    pub output: String,
    pub error: Option<String>,
}

fn run_git(cwd: &str, args: &[&str], input: &str) -> Captured {
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    // Never wait for a password on a terminal that isn't there; credential helpers still work.
    let env = [("GIT_TERMINAL_PROMPT", "0"), ("GIT_OPTIONAL_LOCKS", "0")];
    run_captured("git", &args, Path::new(cwd), &env, input, Duration::from_secs(120))
}

fn result(r: Captured) -> GitResult {
    if r.ok() {
        let output = format!("{}{}", r.stdout, r.stderr).trim().to_owned();
        return GitResult { ok: true, output, error: None };
    }
    if r.not_found {
        return GitResult {
            ok: false,
            output: String::new(),
            error: Some("Git is not installed (or not on PATH).".into()),
        };
    }
    let text = if r.stderr.trim().is_empty() { r.stdout.trim() } else { r.stderr.trim() };
    let text = if text.is_empty() { "git failed" } else { text };
    let error = text
        .lines()
        .map(|l| l.strip_prefix("fatal: ").or_else(|| l.strip_prefix("error: ")).unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n");
    GitResult { ok: false, output: r.stdout.trim().to_owned(), error: Some(error) }
}

fn failed(error: impl Into<String>) -> GitResult {
    GitResult { ok: false, output: String::new(), error: Some(error.into()) }
}

/// The repository's top folder, in the opened folder's slash style.
fn toplevel(folder: &str) -> Option<String> {
    let top = run_git(folder, &["rev-parse", "--show-toplevel"], "");
    if !top.ok() {
        return None;
    }
    let t = top.stdout.trim();
    Some(if folder.contains('\\') { t.replace('/', "\\") } else { t.to_owned() })
}

fn is_sha(s: &str) -> bool {
    (4..=64).contains(&s.len()) && s.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// The repository containing `folder`: its root and status (None if there isn't one).
pub fn repo_status(folder: &str) -> Option<(String, RepoStatus)> {
    let root = toplevel(folder)?;
    let st = run_git(&root, &["status", "--porcelain=v2", "--branch", "-z", "--untracked-files=all"], "");
    st.ok().then(|| (root, parse_status_v2(&st.stdout)))
}

/// Status for the files under `folder` (the file tree's badges), or None if
/// it isn't in a repository (or git is missing).
pub fn tree_status(folder: &str) -> Option<StatusMap> {
    let root = toplevel(folder)?;
    let out = run_git(folder, &["status", "--porcelain=v1", "-z", "--untracked-files=all"], "");
    out.ok().then(|| build_status_map(&root, folder, &parse_porcelain(&out.stdout)))
}

/// The diff of one file: staged (index vs HEAD) or not (working tree vs index).
pub fn file_diff(root: &str, path: &str, staged: bool) -> GitResult {
    let mut args = vec!["-c", "core.quotepath=false", "diff", "--no-color", "--no-ext-diff", "-U3"];
    if staged {
        args.push("--cached");
    }
    args.extend(["--", path]);
    let r = run_git(root, &args, "");
    if r.ok() {
        return GitResult { ok: true, output: r.stdout, error: None };
    }
    result(r)
}

/// Let Git produce new-file patches, including quoted paths, binary detection
/// and the no-final-newline marker. `--no-index` returns 1 for a difference.
pub fn untracked_diff(root: &str, path: &str) -> GitResult {
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let captured = run_git(
        root,
        &["-c", "core.quotepath=false", "diff", "--no-index", "--no-color", "--no-ext-diff", "--", null, path],
        "",
    );
    if matches!(captured.code, Some(0 | 1)) && !captured.failed {
        GitResult { ok: true, output: captured.stdout, error: None }
    } else {
        result(captured)
    }
}

pub fn stage(root: &str, paths: &[&str]) -> GitResult {
    let mut args = vec!["add", "-A", "--"];
    args.extend(paths);
    result(run_git(root, &args, ""))
}

pub fn unstage(root: &str, paths: &[&str]) -> GitResult {
    // No commits yet: there's no HEAD to restore from, so just remove them from the index.
    let head = run_git(root, &["rev-parse", "--verify", "-q", "HEAD"], "");
    let mut args = if head.ok() { vec!["restore", "--staged", "--"] } else { vec!["rm", "--cached", "-r", "-q", "--"] };
    args.extend(paths);
    result(run_git(root, &args, ""))
}

/// Throw away unstaged changes to tracked files (untracked files are deleted
/// by the app, to the trash).
pub fn discard(root: &str, paths: &[&str]) -> GitResult {
    let mut args = vec!["restore", "--worktree", "--"];
    args.extend(paths);
    result(run_git(root, &args, ""))
}

/// Apply a patch (one hunk): to the index (`cached`: stage it, or unstage it
/// with `reverse`) or to the working tree (`reverse`: discard it).
pub fn apply_patch(root: &str, patch: &str, cached: bool, reverse: bool) -> GitResult {
    let mut args = vec!["apply", "--whitespace=nowarn"];
    if cached {
        args.push("--cached");
    }
    if reverse {
        args.push("--reverse");
    }
    args.push("-");
    result(run_git(root, &args, patch))
}

pub fn commit(root: &str, message: &str, amend: bool, all: bool) -> GitResult {
    if all {
        let added = run_git(root, &["add", "-A"], "");
        if !added.ok() {
            return result(added);
        }
    }
    let mut args = vec!["commit", "-F", "-"];
    if amend {
        args.push("--amend");
    }
    result(run_git(root, &args, message))
}

pub fn last_commit_message(root: &str) -> String {
    let r = run_git(root, &["log", "-1", "--format=%B"], "");
    if r.ok() {
        r.stdout.trim_end().to_owned()
    } else {
        String::new()
    }
}

pub fn branches(root: &str) -> Vec<Branch> {
    let format = format!("--format={BRANCH_FORMAT}");
    let r = run_git(root, &["for-each-ref", "--sort=-committerdate", &format, "refs/heads", "refs/remotes"], "");
    if r.ok() {
        parse_branches(&r.stdout)
    } else {
        Vec::new()
    }
}

/// Switch to a branch; a remote one (origin/x) gets a local tracking branch.
/// `create` makes a new one.
pub fn switch_branch(root: &str, name: &str, create: bool, remote: bool) -> GitResult {
    if !is_valid_branch_name(name) {
        return failed(format!("\"{name}\" isn't a valid branch name."));
    }
    if create {
        return result(run_git(root, &["switch", "-c", name], ""));
    }
    if remote {
        let local = name.split_once('/').map_or(name, |(_, l)| l);
        let existing = run_git(root, &["rev-parse", "--verify", "-q", &format!("refs/heads/{local}")], "");
        if existing.ok() {
            return result(run_git(root, &["switch", local], ""));
        }
        return result(run_git(root, &["switch", "--track", name], ""));
    }
    result(run_git(root, &["switch", name], ""))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncAction {
    Fetch,
    Pull,
    Push,
}

/// fetch / pull / push. A branch without an upstream is pushed to the first
/// remote and set to track it.
pub fn sync(root: &str, action: SyncAction) -> GitResult {
    match action {
        SyncAction::Fetch => return result(run_git(root, &["fetch", "--prune"], "")),
        SyncAction::Pull => return result(run_git(root, &["pull", "--no-rebase", "--no-edit"], "")),
        SyncAction::Push => {}
    }
    if run_git(root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"], "").ok() {
        return result(run_git(root, &["push"], ""));
    }
    let remotes = run_git(root, &["remote"], "").stdout;
    let remotes: Vec<&str> = remotes.lines().filter(|l| !l.is_empty()).collect();
    let Some(first) = remotes.first() else {
        return failed("This repository has no remote to push to. Add one with: git remote add origin <url>");
    };
    let remote = if remotes.contains(&"origin") { "origin" } else { first };
    result(run_git(root, &["push", "-u", remote, "HEAD"], ""))
}

pub fn init(folder: &str) -> GitResult {
    result(run_git(folder, &["init"], ""))
}

/// Commits, newest first (topological, so the graph draws cleanly). `all`
/// includes every branch; `path` limits it to one file's history (following renames).
pub fn log(root: &str, all: bool, path: Option<&str>, limit: usize, skip: usize) -> Result<Vec<Commit>, String> {
    let (format, n, skip) =
        (format!("--format={LOG_FORMAT}"), format!("-n{}", limit.clamp(1, 2000)), format!("--skip={skip}"));
    let mut args = vec!["-c", "core.quotepath=false", "log", "--topo-order", &format, &n, &skip];
    if all {
        args.push("--all");
    }
    if let Some(p) = path {
        args.extend(["--follow", "--", p]);
    }
    let r = run_git(root, &args, "");
    if r.ok() {
        return Ok(parse_log(&r.stdout));
    }
    // A repo without commits has no history yet (not an error worth showing).
    if r.stderr.contains("does not have any commits") || r.stderr.contains("bad default revision") {
        return Ok(Vec::new());
    }
    Err(r.stderr.trim().to_owned())
}

/// A file a commit changed: status letter (A, M, D, R...), path, and the old path for renames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitFile {
    pub status: String,
    pub path: String,
    pub from: Option<String>,
}

/// A commit's full message and the files it changed.
pub fn commit_details(root: &str, sha: &str) -> Result<(String, Vec<CommitFile>), String> {
    if !is_sha(sha) {
        return Err("bad commit id".into());
    }
    let r = run_git(
        root,
        &["-c", "core.quotepath=false", "show", "--no-color", "--format=%B%x00", "--name-status", "-z", "-M", sha],
        "",
    );
    if !r.ok() {
        return Err(r.stderr.trim().to_owned());
    }
    let (message, rest) = r.stdout.split_once('\0').unwrap_or((&r.stdout, ""));
    let parts: Vec<&str> = rest.trim_start().split('\0').filter(|p| !p.is_empty()).collect();
    let mut files = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let status = parts[i].trim();
        let valid = !status.is_empty()
            && status.as_bytes()[0].is_ascii_uppercase()
            && status[1..].chars().all(|c| c.is_ascii_digit());
        if !valid {
            i += 1;
            continue;
        }
        let letter = status[..1].to_owned();
        if letter == "R" || letter == "C" {
            files.push(CommitFile {
                status: letter,
                from: parts.get(i + 1).map(|s| (*s).to_owned()),
                path: parts.get(i + 2).copied().unwrap_or("").to_owned(),
            });
            i += 3;
        } else {
            files.push(CommitFile {
                status: status.to_owned(),
                path: parts.get(i + 1).copied().unwrap_or("").to_owned(),
                from: None,
            });
            i += 2;
        }
    }
    Ok((message.trim().to_owned(), files))
}

/// What one commit changed in one file (compared with its first parent).
pub fn commit_file_diff(root: &str, sha: &str, path: &str) -> GitResult {
    if !is_sha(sha) {
        return failed("bad commit id");
    }
    let r = run_git(
        root,
        &[
            "-c",
            "core.quotepath=false",
            "show",
            "--no-color",
            "--no-ext-diff",
            "--format=",
            "-M",
            "--first-parent",
            sha,
            "--",
            path,
        ],
        "",
    );
    if r.ok() {
        return GitResult { ok: true, output: r.stdout, error: None };
    }
    result(r)
}

/// Who last changed each line. `contents` is the editor's text (so unsaved edits line up).
pub fn blame(root: &str, path: &str, contents: Option<&str>) -> Result<Vec<BlameLine>, String> {
    let mut args = vec!["blame", "--porcelain"];
    if contents.is_some() {
        args.extend(["--contents", "-"]);
    }
    args.extend(["--", path]);
    let r = run_git(root, &args, contents.unwrap_or(""));
    if !r.ok() {
        return Err(r.stderr.trim().trim_start_matches("fatal: ").to_owned());
    }
    Ok(parse_blame(&r.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::git::{hunk_patch, parse_diff, ChangeKind, FileChange};

    fn has_git() -> bool {
        !run_captured("git", &["--version".into()], &std::env::temp_dir(), &[], "", Duration::from_secs(20)).not_found
    }

    #[test]
    fn an_untracked_file_patch_preserves_spaces_and_no_final_newline() {
        if !has_git() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        assert!(init(&root).ok);
        std::fs::write(dir.path().join("new file.py"), "print(1)").unwrap();
        let diff = untracked_diff(&root, "new file.py");
        assert!(diff.ok, "{:?}", diff.error);
        assert!(diff.output.contains("No newline at end of file"));
        let files = parse_diff(&diff.output);
        assert_eq!(files[0].hunks.len(), 1);
        let staged = apply_patch(&root, &hunk_patch(&files[0], &files[0].hunks[0]), true, false);
        assert!(staged.ok, "{:?}", staged.error);
        assert!(repo_status(&root).unwrap().1.staged.iter().any(|f| f.path == "new file.py"));
    }

    #[test]
    fn git_operations_on_a_real_repository() {
        if !has_git() {
            return eprintln!("git isn't installed: skipped");
        }
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_string_lossy().into_owned();
        let g = |args: &[&str]| assert!(run_git(&dir, args, "").ok(), "git {args:?}");
        g(&["init", "-q", "-b", "main"]);
        g(&["config", "user.name", "Test"]);
        g(&["config", "user.email", "test@example.com"]);
        g(&["config", "core.autocrlf", "false"]);
        let lines: Vec<String> = (1..=30).map(|i| format!("line {i}")).collect();
        let a = tmp.path().join("a.txt");

        // An empty repo, a first commit, then changes.
        std::fs::write(&a, lines.join("\n") + "\n").unwrap();
        let (root, st) = repo_status(&dir).expect("a repository");
        assert!(Path::new(&root).join("a.txt").is_file());
        assert_eq!((st.branch.as_deref(), st.initial), (Some("main"), true));
        assert_eq!(st.unstaged, vec![FileChange { path: "a.txt".into(), from: None, kind: ChangeKind::Untracked }]);
        assert!(tree_status(&dir).unwrap().files.values().all(|k| *k == ChangeKind::Untracked));
        assert!(commit(&dir, "first", false, true).ok);
        let (_, st) = repo_status(&dir).unwrap();
        assert!(!st.initial && st.staged.is_empty() && st.unstaged.is_empty());
        assert_eq!(last_commit_message(&dir), "first");

        // Stage and unstage a single hunk.
        let mut changed = lines.clone();
        changed[1] = "CHANGED 2".into();
        changed[25] = "CHANGED 26".into();
        std::fs::write(&a, changed.join("\n") + "\n").unwrap();
        let diff = parse_diff(&file_diff(&dir, "a.txt", false).output);
        assert_eq!(diff[0].hunks.len(), 2);
        assert!(apply_patch(&dir, &hunk_patch(&diff[0], &diff[0].hunks[1]), true, false).ok);
        let index = parse_diff(&file_diff(&dir, "a.txt", true).output);
        assert_eq!(index[0].hunks.len(), 1);
        assert!(index[0].hunks[0].lines.iter().any(|l| l.text == "CHANGED 26"));
        let work = parse_diff(&file_diff(&dir, "a.txt", false).output);
        assert_eq!(work[0].hunks.len(), 1);
        assert!(work[0].hunks[0].lines.iter().any(|l| l.text == "CHANGED 2"));
        assert!(apply_patch(&dir, &hunk_patch(&index[0], &index[0].hunks[0]), true, true).ok);
        assert_eq!(file_diff(&dir, "a.txt", true).output, "");

        // Discard a single hunk from the working tree.
        let diff = parse_diff(&file_diff(&dir, "a.txt", false).output);
        assert!(apply_patch(&dir, &hunk_patch(&diff[0], &diff[0].hunks[0]), false, true).ok);
        let text = std::fs::read_to_string(&a).unwrap();
        assert!(text.contains("line 2\n") && text.contains("CHANGED 26"));

        // Stage, unstage and discard whole files.
        assert!(stage(&dir, &["a.txt"]).ok);
        assert_eq!(repo_status(&dir).unwrap().1.staged.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.txt"]);
        assert!(unstage(&dir, &["a.txt"]).ok);
        assert!(repo_status(&dir).unwrap().1.staged.is_empty());
        assert!(discard(&dir, &["a.txt"]).ok);
        assert!(repo_status(&dir).unwrap().1.unstaged.is_empty());

        // Branches: create, switch, refuse bad names.
        assert!(switch_branch(&dir, "feature/x", true, false).ok);
        assert_eq!(repo_status(&dir).unwrap().1.branch.as_deref(), Some("feature/x"));
        assert!(branches(&dir).iter().any(|b| b.name == "feature/x" && b.current));
        assert!(switch_branch(&dir, "main", false, false).ok);
        let bad = switch_branch(&dir, "--force", true, false);
        assert!(!bad.ok && bad.error.unwrap().contains("isn't a valid branch name"));

        // History, a commit, its diff and blame.
        std::fs::write(tmp.path().join("b.txt"), "one\ntwo\n").unwrap();
        assert!(commit(&dir, "add b\n\nmore details", false, true).ok);
        let h = log(&dir, true, None, 50, 0).unwrap();
        assert_eq!(h.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(), ["add b", "first"]);
        assert!(h[0].refs.join(" ").contains("main"));
        assert_eq!(log(&dir, false, Some("b.txt"), 50, 0).unwrap().len(), 1);
        let (message, files) = commit_details(&dir, &h[0].sha).unwrap();
        assert_eq!(message, "add b\n\nmore details");
        assert_eq!(files, vec![CommitFile { status: "A".into(), path: "b.txt".into(), from: None }]);
        assert!(commit_file_diff(&dir, &h[0].sha, "b.txt").output.contains("+two"));
        let b = blame(&dir, "b.txt", Some("one\nchanged\ntwo\n")).unwrap();
        assert_eq!(b.iter().map(|l| l.uncommitted).collect::<Vec<_>>(), [false, true, false]);
        assert_eq!((b[0].author.as_str(), b[0].summary.as_str()), ("Test", "add b"));
        assert!(commit_details(&dir, "not-a-sha").is_err());

        // No remote to push to.
        let r = sync(&dir, SyncAction::Push);
        assert!(!r.ok && r.error.unwrap().contains("no remote"));
    }

    #[test]
    fn a_folder_outside_any_repository_has_no_status() {
        if !has_git() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_string_lossy().into_owned();
        // (The temp folder could itself be inside a repo on odd setups; only check when it isn't.)
        if run_git(&dir, &["rev-parse", "--show-toplevel"], "").ok() {
            return;
        }
        assert!(repo_status(&dir).is_none());
        assert!(tree_status(&dir).is_none());
        assert!(init(&dir).ok);
        // A fresh repo has no history (not an error).
        assert_eq!(log(&dir, true, None, 10, 0), Ok(Vec::new()));
    }
}
