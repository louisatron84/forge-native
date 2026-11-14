//! Backups of unsaved files, so changes survive a crash or power cut.
//! Replaces `src/main/backups.ts` (the store) and `src/renderer/backups.ts`
//! (when to write and drop them).
//!
//! One JSON file per edited file in `<data>/backups`, named by a hash of its
//! path. They're removed when the file is saved or its changes are thrown
//! away, so whatever is left at startup came from a crash.

use forge_text::Encoding;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BackupData {
    pub path: String,
    /// Lines joined with \n; the line ending style is kept separately.
    pub text: String,
    pub encoding: Encoding,
    /// "LF" or "CRLF".
    pub eol: String,
    pub time: u64,
}

/// File name for a path's backup (Windows paths ignore case and slash direction).
/// The same name is used for each project's saved workspace state.
pub fn backup_name(path: &str) -> String {
    let b = path.as_bytes();
    let windows = b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':';
    let norm = if windows { path.replace('/', "\\").to_lowercase() } else { path.to_owned() };
    let mut hasher = sha1_smol::Sha1::new();
    hasher.update(norm.as_bytes());
    format!("{}.json", &hasher.digest().to_string()[..24])
}

pub struct Backups {
    dir: PathBuf,
}

impl Backups {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn write(&self, data: &BackupData) -> io::Result<()> {
        use std::io::Write;
        crate::text_file::write_atomic(&self.dir.join(backup_name(&data.path)), |file| {
            let mut writer = io::BufWriter::new(file);
            serde_json::to_writer(&mut writer, data).map_err(io::Error::other)?;
            writer.flush()
        })
    }

    pub fn remove(&self, path: &str) -> io::Result<()> {
        match std::fs::remove_file(self.dir.join(backup_name(path))) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Every readable backup (unreadable or half-written ones are skipped).
    pub fn list(&self) -> Vec<BackupData> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        entries
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| serde_json::from_str::<BackupData>(&std::fs::read_to_string(e.path()).ok()?).ok())
            .collect()
    }

    pub fn clear(&self) -> io::Result<()> {
        match std::fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// What an open document looks like to the keeper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocSnapshot {
    /// A stable id for the open document (it may be renamed).
    pub id: u64,
    pub path: String,
    pub dirty: bool,
    /// Changes whenever a backup would contain something different (path, version, eol, encoding).
    pub key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackupAction {
    /// Write a backup of this document now (its state is `key`).
    Write { id: u64, key: String },
    /// Delete the backup written under this path.
    Remove { path: String },
}

/// Decides when backups are written and dropped: an unsaved document is
/// backed up `delay_ms` after its last change; saving, closing or renaming
/// drops its backup.
pub struct BackupKeeper {
    delay_ms: u64,
    /// id -> (path written under, state it holds)
    written: HashMap<u64, (String, String)>,
    /// id -> when to write (ms)
    due: HashMap<u64, u64>,
}

impl BackupKeeper {
    pub fn new(delay_ms: u64) -> Self {
        Self { delay_ms, written: HashMap::new(), due: HashMap::new() }
    }

    /// Call whenever the open documents or their state change, and on a timer.
    /// Returns what to do now.
    pub fn sync(&mut self, open: &[DocSnapshot], now: u64) -> Vec<BackupAction> {
        let mut actions = Vec::new();
        let is_open = |id: u64| open.iter().find(|d| d.id == id);
        // Files closed while their backup was still waiting to be written.
        self.due.retain(|id, _| is_open(*id).is_some_and(|d| d.dirty));
        // Drop backups of files that were closed, saved, or renamed.
        let stale: Vec<u64> = self
            .written
            .iter()
            .filter(|(id, (path, _))| is_open(**id).is_none_or(|d| !d.dirty || d.path != *path))
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            if let Some((path, _)) = self.written.remove(&id) {
                actions.push(BackupAction::Remove { path });
            }
        }
        // Schedule (or write) backups of unsaved files that changed since their last backup.
        for d in open.iter().filter(|d| d.dirty) {
            if self.written.get(&d.id).is_some_and(|(_, key)| *key == d.key) {
                self.due.remove(&d.id);
                continue;
            }
            let at = *self.due.entry(d.id).or_insert(now + self.delay_ms);
            if now >= at {
                self.due.remove(&d.id);
                actions.push(BackupAction::Write { id: d.id, key: d.key.clone() });
            }
        }
        actions
    }

    /// A backup was written (record what it holds).
    pub fn written(&mut self, id: u64, path: &str, key: &str) {
        self.written.insert(id, (path.to_owned(), key.to_owned()));
    }

    /// When `sync` next needs to run to write a due backup.
    pub fn next_due(&self) -> Option<u64> {
        self.due.values().copied().min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(path: &str, text: &str) -> BackupData {
        BackupData { path: path.into(), text: text.into(), encoding: Encoding::Utf8, eol: "LF".into(), time: 1 }
    }

    #[test]
    fn writing_a_backup_does_not_modify_a_hardlinked_file() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("private.txt");
        std::fs::write(&victim, "keep me").unwrap();
        let store = dir.path().join("backups");
        std::fs::create_dir(&store).unwrap();
        std::fs::hard_link(&victim, store.join(backup_name("/project/main.py"))).unwrap();
        let backups = Backups::new(store);
        backups.write(&data("/project/main.py", "edited")).unwrap();
        assert_eq!(std::fs::read_to_string(victim).unwrap(), "keep me");
        assert_eq!(backups.list()[0].text, "edited");
    }

    #[test]
    fn writes_lists_and_removes_backups() {
        let dir = tempfile::tempdir().expect("temp");
        let b = Backups::new(dir.path().join("backups"));
        assert!(b.list().is_empty());
        b.write(&data("C:\\p\\a.py", "x = 1")).expect("write");
        b.write(&data("C:\\p\\b.py", "y")).expect("write");
        b.write(&data("C:\\p\\a.py", "x = 2")).expect("write"); // replaces the first
        let mut texts: Vec<String> = b.list().into_iter().map(|d| d.text).collect();
        texts.sort();
        assert_eq!(texts, ["x = 2", "y"]);
        b.remove("c:/p/A.py").expect("remove");
        assert_eq!(b.list().into_iter().map(|d| d.path).collect::<Vec<_>>(), ["C:\\p\\b.py"]);
    }

    #[test]
    fn skips_broken_files_and_clear_removes_everything() {
        let dir = tempfile::tempdir().expect("temp");
        let b = Backups::new(dir.path().join("backups"));
        b.write(&data("/p/a.py", "ok")).expect("write");
        std::fs::write(dir.path().join("backups").join("junk.json"), "{ half").expect("write");
        assert_eq!(b.list().len(), 1);
        b.clear().expect("clear");
        assert!(b.list().is_empty());
    }

    #[test]
    fn names_ignore_case_and_slashes_on_windows_paths() {
        assert_eq!(backup_name("C:\\P\\A.py"), backup_name("c:/p/a.py"));
        assert_ne!(backup_name("/p/A.py"), backup_name("/p/a.py"));
        assert_eq!(backup_name("x").len(), 29);
    }

    fn snap(id: u64, path: &str, dirty: bool, key: &str) -> DocSnapshot {
        DocSnapshot { id, path: path.into(), dirty, key: key.into() }
    }

    #[test]
    fn backs_up_an_unsaved_file_a_moment_after_it_changes() {
        let mut k = BackupKeeper::new(1000);
        assert!(k.sync(&[snap(1, "/p/a.py", false, "v0")], 0).is_empty());
        assert!(k.sync(&[snap(1, "/p/a.py", true, "v1")], 100).is_empty());
        assert!(k.sync(&[snap(1, "/p/a.py", true, "v1")], 600).is_empty());
        assert_eq!(k.next_due(), Some(1100));
        assert_eq!(k.sync(&[snap(1, "/p/a.py", true, "v1")], 1100), [BackupAction::Write { id: 1, key: "v1".into() }]);
    }

    #[test]
    fn does_not_rewrite_an_unchanged_backup() {
        let mut k = BackupKeeper::new(1000);
        k.sync(&[snap(1, "/p/a.py", true, "v1")], 0);
        let a = k.sync(&[snap(1, "/p/a.py", true, "v1")], 1000);
        assert_eq!(a.len(), 1);
        k.written(1, "/p/a.py", "v1");
        assert!(k.sync(&[snap(1, "/p/a.py", true, "v1")], 5000).is_empty());
    }

    #[test]
    fn drops_the_backup_when_saved_closed_or_renamed() {
        let mut k = BackupKeeper::new(1000);
        for (id, p) in [(1, "/p/a.py"), (2, "/p/b.py"), (3, "/p/c.py")] {
            k.written(id, p, "v1");
        }
        // a saved, b closed, c renamed (still unsaved).
        let actions = k.sync(&[snap(1, "/p/a.py", false, "v2"), snap(3, "/p/renamed.py", true, "v2")], 0);
        let mut removed: Vec<String> = actions
            .iter()
            .filter_map(|a| if let BackupAction::Remove { path } = a { Some(path.clone()) } else { None })
            .collect();
        removed.sort();
        assert_eq!(removed, ["/p/a.py", "/p/b.py", "/p/c.py"]);
        // The renamed file is backed up again under its new name.
        let later = k.sync(&[snap(1, "/p/a.py", false, "v2"), snap(3, "/p/renamed.py", true, "v2")], 1000);
        assert_eq!(later, [BackupAction::Write { id: 3, key: "v2".into() }]);
    }

    #[test]
    fn a_file_closed_before_its_backup_was_written_is_never_written() {
        let mut k = BackupKeeper::new(1000);
        k.sync(&[snap(1, "/p/a.py", true, "v1")], 0);
        k.sync(&[], 10);
        assert!(k.sync(&[], 2000).is_empty());
        assert_eq!(k.next_due(), None);
    }
}
