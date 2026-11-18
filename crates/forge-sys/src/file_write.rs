//! Staged disk writes for Forge-owned files.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

#[cfg(windows)]
mod windows;

/// Replace a file after writing and flushing a temporary sibling.
pub fn replace(path: &Path, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    replace_with(path, |file| file.write_all(bytes.as_ref()))
}

/// Create a file without overwriting an existing path, even if one appears
/// while the data is being written.
pub fn create(path: &Path, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    create_with(path, |file| file.write_all(bytes.as_ref()))
}

pub(crate) fn create_with(path: &Path, write: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    stage(path, false, false, write, || Ok(()))
}

/// Save an owner-only file without replacing an existing entry.
pub(crate) fn create_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    stage(path, false, true, |file| file.write_all(bytes), || Ok(()))
}

/// Check that the reviewed contents still match immediately before publication.
pub(crate) fn publish_reviewed(
    path: &Path,
    replace: bool,
    bytes: &[u8],
    check: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    stage(path, replace, false, |file| file.write_all(bytes), check)
}

/// Undo creation only for the original regular, unlinked entry.
pub(crate) fn remove_created(path: &Path, check: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(io::Error::other("The generated file was replaced with a different type of entry."));
    }
    refuse_hard_links(path, &metadata)?;
    check()?;
    fs::remove_file(path)
}

/// Private scratch data handed to a helper, with automatic cleanup.
pub fn scratch(bytes: impl AsRef<[u8]>) -> io::Result<tempfile::NamedTempFile> {
    let mut file = tempfile::NamedTempFile::new()?;
    #[cfg(windows)]
    windows::copy_security(file.path(), None, true)?;
    file.write_all(bytes.as_ref())?;
    file.as_file().sync_all()?;
    Ok(file)
}

pub(crate) fn replace_with(path: &Path, write: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    stage(path, true, false, write, || Ok(()))
}

fn stage(
    path: &Path,
    replace: bool,
    private: bool,
    write: impl FnOnce(&mut File) -> io::Result<()>,
    check: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let previous = if replace {
        match fs::symlink_metadata(path) {
            Ok(meta) if meta.is_file() || meta.is_symlink() => {
                let target = fs::canonicalize(path)?;
                let meta = fs::metadata(&target)?;
                if !meta.is_file() {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, "The target is not a regular file"));
                }
                refuse_hard_links(&target, &meta)?;
                Some((target, meta))
            }
            Ok(_) => return Err(io::Error::new(io::ErrorKind::InvalidInput, "The target is not a regular file")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    let target = previous.as_ref().map_or(path, |(target, _)| target.as_path());
    let parent = target.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let parent = fs::canonicalize(parent)?;
    let destination = parent.join(target.file_name().ok_or_else(|| io::Error::other("Missing file name"))?);
    let mut temp = tempfile::NamedTempFile::new_in(&parent)?;
    let private = private || is_secret_file(path) || is_secret_file(target);
    #[cfg(windows)]
    windows::copy_security(temp.path(), previous.as_ref().map(|(target, _)| target.as_path()), private)?;
    write(temp.as_file_mut())?;
    set_metadata(private, temp.as_file(), previous.as_ref().map(|(_, meta)| meta))?;
    temp.as_file().sync_all()?;

    // Open the directory before committing so failure to open it leaves the
    // old file untouched. A sync failure after commit cannot undo the rename.
    #[cfg(unix)]
    let directory = File::open(&parent)?;
    #[cfg(windows)]
    let directory = windows::open_directory(&parent)?;
    if previous.is_some() {
        if fs::canonicalize(path)? != target {
            return Err(io::Error::other("The save target changed while Forge was writing it"));
        }
        refuse_hard_links(target, &fs::metadata(target)?)?;
    }
    check()?;
    #[cfg(windows)]
    windows::commit(temp, &destination, previous.is_some())?;
    #[cfg(not(windows))]
    if previous.is_some() {
        temp.persist(&destination).map_err(|error| error.error)?;
    } else {
        temp.persist_noclobber(&destination).map_err(|error| error.error)?;
    }
    #[cfg(any(unix, windows))]
    directory.sync_all().map_err(|error| io::Error::other(format!("File saved, but folder sync failed: {error}")))?;
    Ok(())
}

fn refuse_hard_links(_target: &Path, _meta: &fs::Metadata) -> io::Result<()> {
    #[cfg(unix)]
    let links = std::os::unix::fs::MetadataExt::nlink(_meta);
    #[cfg(windows)]
    let links = {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
        let file = File::open(_target)?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // The handle stays open across the call and info is writable.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        info.nNumberOfLinks
    };
    if links > 1 {
        return Err(io::Error::other("Forge cannot save a file with multiple hard links. Save to a new file instead."));
    }
    Ok(())
}

fn set_metadata(_private: bool, file: &File, previous: Option<&fs::Metadata>) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{fchown, MetadataExt, PermissionsExt};
        if let Some(previous) = previous {
            let fresh = file.metadata()?;
            let uid = (fresh.uid() != previous.uid()).then_some(previous.uid());
            let gid = (fresh.gid() != previous.gid()).then_some(previous.gid());
            if uid.is_some() || gid.is_some() {
                fchown(file, uid, gid)?;
            }
        }
        if _private {
            return file.set_permissions(fs::Permissions::from_mode(0o600));
        }
    }
    if let Some(previous) = previous {
        file.set_permissions(previous.permissions())?;
    }
    Ok(())
}

pub fn is_secret_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_ascii_lowercase();
    name == ".env"
        || name.starts_with(".env.")
        || name.ends_with(".pem")
        || matches!(name.as_str(), "credentials" | "credentials.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_keeps_a_file_added_while_staging() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.txt");
        let error = create_with(&path, |staged| {
            staged.write_all(b"ours")?;
            fs::write(&path, b"theirs")
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&path).unwrap(), b"theirs");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn saves_keep_mode_and_owner_except_that_secrets_become_private() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        for (name, mode) in [("run.sh", 0o754), (".env", 0o600), ("secret.pem", 0o600)] {
            let path = dir.path().join(name);
            fs::write(&path, b"before").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o754)).unwrap();
            let old = fs::metadata(&path).unwrap();
            replace(&path, b"after").unwrap();
            let new = fs::metadata(path).unwrap();
            assert_eq!((new.uid(), new.gid()), (old.uid(), old.gid()));
            assert_eq!(new.mode() & 0o7777, mode);
        }
    }
}
