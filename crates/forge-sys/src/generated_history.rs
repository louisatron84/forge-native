//! Private snapshots for generated writes. The committed bytes identify the
//! revision so an older snapshot cannot overwrite subsequent user changes.
use crate::{backups::backup_name, file_write, files::canonicalize_path, text_file};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_REVIEW_BYTES: usize = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
pub struct Revision {
    pub source: String,
    pub path: PathBuf,
    #[serde(with = "saved_bytes")]
    pub before: Option<Vec<u8>>,
    result_hash: [u8; 32],
}

mod saved_bytes {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(bytes) => serializer.serialize_some(&STANDARD.encode(bytes)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .map(|value| STANDARD.decode(value).map_err(serde::de::Error::custom))
            .transpose()
    }
}

pub struct History {
    dir: PathBuf,
}

/// Read a bounded snapshot of an existing plain-text destination.
pub fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    if let Err(error) = fs::symlink_metadata(path) {
        return if error.kind() == io::ErrorKind::NotFound { Ok(None) } else { Err(error) };
    }
    let checked = fs::canonicalize(path)?;
    if !fs::metadata(&checked)?.is_file() {
        return Err(io::Error::other("The destination is not a regular file."));
    }
    let mut bytes = Vec::new();
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options.open(checked)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("The destination is not a regular file."));
    }
    file.take(MAX_REVIEW_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_REVIEW_BYTES {
        return Err(io::Error::other("The file is too large to preview safely."));
    }
    Ok(Some(bytes))
}

pub fn is_text(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) || !text_file::looks_binary(bytes)
}

impl History {
    pub fn new(user_dir: &Path) -> Self {
        Self { dir: user_dir.join("generated-history") }
    }

    fn for_path(&self, path: &Path) -> PathBuf {
        self.dir.join(backup_name(&path.to_string_lossy()))
    }

    pub fn write(&self, path: &Path, before: Option<&[u8]>, after: &[u8], source: &str) -> io::Result<()> {
        if after.len() > MAX_REVIEW_BYTES {
            return Err(io::Error::other("The generated text is too large to keep in local history."));
        }
        let target = identity(path)?;
        let check = || {
            if identity(path)? != target || read(path)?.as_deref() != before {
                return Err(io::Error::other("The file changed after the preview. Preview the change again."));
            }
            Ok(())
        };
        check()?;
        if before == Some(after) {
            return Ok(());
        }
        let revision = Revision {
            source: source.to_owned(),
            path: target.clone(),
            before: before.map(<[u8]>::to_vec),
            result_hash: hash(after),
        };
        let record = self.for_path(&target).join(format!(
            "{:032}.json",
            SystemTime::now().duration_since(UNIX_EPOCH).map_err(io::Error::other)?.as_nanos()
        ));
        let state = serde_json::to_vec(&revision).map_err(io::Error::other)?;
        file_write::create_private(&record, &state)?;
        let result = file_write::publish_reviewed(path, before.is_some(), after, check);
        // A directory sync can fail after publication. Keep the snapshot if
        // the new file may already have reached disk.
        if result.is_err() && read(path).is_ok_and(|contents| contents.as_deref() != Some(after)) {
            let _ = fs::remove_file(record);
        }
        result
    }

    /// Undo the last generated write when the file still contains its result.
    /// Returns the restored bytes, or None if undo removed a newly created file.
    pub fn undo(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        let target = identity(path)?;
        let records = fs::read_dir(self.for_path(&target)).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                io::Error::other("This file has no generated changes to undo.")
            } else {
                error
            }
        })?;
        let mut latest = None;
        for entry in records {
            let entry = entry?;
            let name = entry.file_name();
            let valid = name
                .to_str()
                .and_then(|name| name.strip_suffix(".json"))
                .is_some_and(|stamp| stamp.len() == 32 && stamp.bytes().all(|b| b.is_ascii_digit()));
            if valid
                && entry.file_type()?.is_file()
                && !entry.path().with_extension("undone").try_exists()?
                && latest.as_ref().is_none_or(|old| name > *old)
            {
                latest = Some(name);
            }
        }
        let record = self
            .for_path(&target)
            .join(latest.ok_or_else(|| io::Error::other("This file has no generated changes to undo."))?);
        let revision: Revision = serde_json::from_reader(File::open(&record)?).map_err(io::Error::other)?;
        if revision.path != target {
            return Err(io::Error::other("The local history belongs to a different file."));
        }
        let check = || {
            if identity(path)? != target
                || read(path)?.as_deref().is_none_or(|bytes| hash(bytes) != revision.result_hash)
            {
                return Err(io::Error::other("The file has changed since the generated write. Its edits were kept."));
            }
            Ok(())
        };
        if let Some(previous) = &revision.before {
            check()?;
            file_write::publish_reviewed(path, true, previous, check)?;
        } else {
            file_write::remove_created(path, check)?;
        }
        file_write::create_private(&record.with_extension("undone"), b"")?;
        Ok(revision.before)
    }
}

/// Resolve through existing ancestors, including when creating a nested file.
pub fn identity(path: &Path) -> io::Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            // Reject dangling links instead of using the missing-path fallback.
            canonicalize_path(path)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
            let name = path.file_name().ok_or_else(|| io::Error::other("Invalid file path"))?;
            Ok(identity(parent)?.join(name))
        }
        Err(error) => Err(error),
    }
}

pub fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_restores_exact_bytes_after_reopening_history_and_preserves_newer_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.py");
        let original = b"caf\xe9\r\n";
        fs::write(&path, original).unwrap();
        History::new(dir.path()).write(&path, Some(original), b"updated\r\n", "Test").unwrap();
        let history = History::new(dir.path());
        fs::write(&path, b"newer").unwrap();
        assert!(history.undo(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"newer");
        fs::write(&path, b"updated\r\n").unwrap();
        assert_eq!(history.undo(&path).unwrap().unwrap(), original);
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(history.undo(&path).is_err());
    }

    #[test]
    fn stale_creation_is_refused_and_nested_creation_can_be_undone() {
        let dir = tempfile::tempdir().unwrap();
        let history = History::new(dir.path());
        let path = dir.path().join("nested/source.py");
        history.write(&path, None, b"generated", "Test").unwrap();
        assert!(history.write(&path, None, b"different", "Test").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"generated");
        assert_eq!(history.undo(&path).unwrap(), None);
        assert!(!path.exists());
    }
}
