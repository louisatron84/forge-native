//! Folders and files for the file tree and quick open: listing (noise folders
//! hidden), creating, renaming, deleting. Replaces `src/main/dirEntries.ts`,
//! `listFiles.ts` and `fileOps.ts`. Names are checked here (not only in the
//! UI) so nothing odd reaches the disk.

use forge_core::names::name_problem;
use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};

/// Folders the tree, quick open and search never show.
pub const HIDDEN: [&str; 7] = [".git", "node_modules", "__pycache__", ".venv", "venv", ".mypy_cache", ".pytest_cache"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

/// Folders first, each sorted ignoring case; noise folders hidden.
pub fn sort_and_filter(dir: &Path, raw: Vec<(String, bool)>) -> Vec<DirEntry> {
    let mut entries: Vec<DirEntry> = raw
        .into_iter()
        .filter(|(name, _)| !HIDDEN.contains(&name.as_str()))
        .map(|(name, is_dir)| DirEntry { path: dir.join(&name), name, is_dir })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    entries
}

/// One folder's contents for the tree.
pub fn read_dir(dir: &Path) -> io::Result<Vec<DirEntry>> {
    let mut raw = Vec::new();
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false) || e.path().is_dir();
        raw.push((e.file_name().to_string_lossy().into_owned(), is_dir));
    }
    Ok(sort_and_filter(dir, raw))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectFile {
    pub path: PathBuf,
    /// Relative to the project root, with '/' separators.
    pub rel: String,
}

/// A project's files for quick open (Ctrl+P): breadth-first, so shallow
/// files are found first if the limit is reached.
pub fn list_files(root: &Path, limit: usize) -> Vec<ProjectFile> {
    let mut out = Vec::new();
    let mut queue: VecDeque<(PathBuf, String)> = VecDeque::from([(root.to_path_buf(), String::new())]);
    while let Some((dir, rel)) = queue.pop_front() {
        if out.len() >= limit {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue }; // unreadable folder: skip it
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            if HIDDEN.contains(&name.as_str()) {
                continue;
            }
            let child_rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let Ok(kind) = e.file_type() else { continue };
            if kind.is_dir() {
                queue.push_back((e.path(), child_rel));
            } else if kind.is_file() {
                out.push(ProjectFile { path: e.path(), rel: child_rel });
                if out.len() >= limit {
                    break;
                }
            }
        }
    }
    out
}

fn error(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

/// Create an empty file or a folder named `name` inside `dir` ("pkg/mod.py"
/// creates "pkg" on the way). Returns its full path; never overwrites.
pub fn create_entry(dir: &Path, name: &str, is_dir: bool) -> io::Result<PathBuf> {
    let target = crate::connect::project_path(dir, name)?;
    if target.exists() {
        return Err(error(format!("\"{name}\" already exists.")));
    }
    if is_dir {
        std::fs::create_dir_all(&target)?;
    } else {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::OpenOptions::new().write(true).create_new(true).open(&target)?;
    }
    Ok(target)
}

/// Rename a file or folder in place (same parent). Returns the new full path.
pub fn rename_entry(path: &Path, new_name: &str) -> io::Result<PathBuf> {
    if let Some(problem) = name_problem(new_name, false) {
        return Err(error(problem));
    }
    let target = path.with_file_name(new_name);
    if target == path {
        return Ok(target);
    }
    // On Windows "a.py" -> "A.py" is the same file: allowed, it only changes the case.
    let case_only = cfg!(windows) && target.to_string_lossy().to_lowercase() == path.to_string_lossy().to_lowercase();
    if !case_only && target.exists() {
        return Err(error(format!("\"{new_name}\" already exists.")));
    }
    std::fs::rename(path, &target)?;
    Ok(target)
}

/// Move a file or folder to the Recycle Bin / Trash.
pub fn trash_entry(path: &Path) -> io::Result<()> {
    trash::delete(path).map_err(|e| error(e.to_string()))
}

/// Delete a file or folder for good (when the trash isn't available).
pub fn delete_entry(path: &Path) -> io::Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, is_dir: bool) -> (String, bool) {
        (name.to_owned(), is_dir)
    }

    #[test]
    fn nested_creation_cannot_escape_through_a_directory_link() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = root.path().join("linked");
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        #[cfg(windows)]
        {
            // Junctions require no symlink privilege and exercise canonical ancestry on Windows.
            let mut command = std::process::Command::new("cmd");
            command.args(["/c", "mklink", "/J"]).arg(&link).arg(outside.path());
            crate::process::hide_window(&mut command);
            assert!(command.output().unwrap().status.success());
        }
        let created = create_entry(root.path(), "linked/escape.py", false);
        assert!(created.is_err(), "nested creation escaped the chosen directory");
        assert!(!outside.path().join("escape.py").exists());
    }

    #[test]
    fn puts_folders_before_files_each_sorted_case_insensitively() {
        let out = sort_and_filter(
            Path::new("/p"),
            vec![e("b.py", false), e("Zeta", true), e("A.txt", false), e("alpha", true)],
        );
        assert_eq!(out.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["alpha", "Zeta", "A.txt", "b.py"]);
    }

    #[test]
    fn hides_noise_folders() {
        let out = sort_and_filter(
            Path::new("/p"),
            vec![e(".git", true), e("node_modules", true), e("__pycache__", true), e(".venv", true), e("src", true)],
        );
        assert_eq!(out.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["src"]);
    }

    #[test]
    fn builds_full_paths() {
        let out = sort_and_filter(Path::new("/proj"), vec![e("main.py", false)]);
        assert_eq!(out[0].path, Path::new("/proj").join("main.py"));
        assert!(!out[0].is_dir);
    }

    fn project() -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("temp dir");
        let r = root.path();
        std::fs::create_dir_all(r.join("pkg/sub")).expect("mkdir");
        std::fs::create_dir_all(r.join("node_modules/x")).expect("mkdir");
        std::fs::create_dir_all(r.join(".git")).expect("mkdir");
        for f in ["main.py", "pkg/a.py", "pkg/sub/b.py", "node_modules/x/junk.js", ".git/HEAD"] {
            std::fs::write(r.join(f), "").expect("write");
        }
        root
    }

    #[test]
    fn lists_files_recursively_with_relative_paths_skipping_hidden_folders() {
        let root = project();
        let files = list_files(root.path(), 20_000);
        let mut rels: Vec<&str> = files.iter().map(|f| f.rel.as_str()).collect();
        rels.sort_unstable();
        assert_eq!(rels, ["main.py", "pkg/a.py", "pkg/sub/b.py"]);
        assert_eq!(
            files.iter().find(|f| f.rel == "pkg/a.py").map(|f| f.path.clone()),
            Some(root.path().join("pkg").join("a.py"))
        );
    }

    #[test]
    fn stops_at_the_limit_shallow_files_first() {
        let root = project();
        assert_eq!(
            list_files(root.path(), 2).iter().map(|f| f.rel.as_str()).collect::<Vec<_>>(),
            ["main.py", "pkg/a.py"]
        );
    }

    #[test]
    fn creates_files_folders_and_missing_parents_never_overwriting() {
        let dir = tempfile::tempdir().expect("temp dir");
        let d = dir.path();
        let p = create_entry(d, "hello.py", false).expect("create");
        assert_eq!(p, d.join("hello.py"));
        assert_eq!(std::fs::read_to_string(&p).expect("read"), "");
        let nested = create_entry(d, "pkg/sub/mod.py", false).expect("create");
        assert_eq!(nested, d.join("pkg").join("sub").join("mod.py"));
        assert!(create_entry(d, "data", true).expect("create").is_dir());
        std::fs::write(d.join("a.py"), "keep me").expect("write");
        assert!(create_entry(d, "a.py", false).unwrap_err().to_string().contains("already exists"));
        assert!(create_entry(d, "a.py", true).unwrap_err().to_string().contains("already exists"));
        assert_eq!(std::fs::read_to_string(d.join("a.py")).expect("read"), "keep me");
        assert!(create_entry(d, "../escape.py", false).is_err());
        assert!(!d.parent().expect("parent").join("escape.py").exists());
    }

    #[test]
    fn renames_in_place_but_never_onto_another_name_or_into_a_folder() {
        let dir = tempfile::tempdir().expect("temp dir");
        let d = dir.path();
        std::fs::write(d.join("a.py"), "a").expect("write");
        assert_eq!(rename_entry(&d.join("a.py"), "b.py").expect("rename"), d.join("b.py"));
        std::fs::write(d.join("c.py"), "c").expect("write");
        assert!(rename_entry(&d.join("c.py"), "b.py").unwrap_err().to_string().contains("already exists"));
        assert_eq!(std::fs::read_to_string(d.join("b.py")).expect("read"), "a");
        assert!(rename_entry(&d.join("c.py"), "sub/c.py").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn allows_a_case_only_rename_on_windows() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("a.py"), "a").expect("write");
        rename_entry(&dir.path().join("a.py"), "A.py").expect("rename");
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .expect("dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["A.py"]);
    }

    #[test]
    fn deletes_files_and_folders() {
        let dir = tempfile::tempdir().expect("temp dir");
        let d = dir.path();
        std::fs::create_dir_all(d.join("x/y")).expect("mkdir");
        std::fs::write(d.join("x/y/z.py"), "").expect("write");
        delete_entry(&d.join("x")).expect("delete");
        assert!(!d.join("x").exists());
    }
}
