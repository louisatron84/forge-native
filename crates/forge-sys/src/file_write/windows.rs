//! Windows file security and write-through publication for staged writes.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{LocalFree, GENERIC_WRITE};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SetSecurityInfo, SDDL_REVISION_1,
    SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    EqualSid, GetSecurityDescriptorControl, GetSecurityDescriptorDacl, ACL, DACL_SECURITY_INFORMATION,
    OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
    UNPROTECTED_DACL_SECURITY_INFORMATION,
};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileAttributesW, MoveFileExW, SetFileAttributesW, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_TEMPORARY,
    FILE_FLAG_BACKUP_SEMANTICS, INVALID_FILE_ATTRIBUTES, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    READ_CONTROL, WRITE_DAC, WRITE_OWNER,
};

struct Descriptor(PSECURITY_DESCRIPTOR);

impl Drop for Descriptor {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // Windows allocated this descriptor with LocalAlloc.
            unsafe { LocalFree(self.0) };
        }
    }
}

fn security(file: &File) -> io::Result<(Descriptor, PSID, *mut ACL, u16)> {
    let mut descriptor = Descriptor(null_mut());
    let (mut owner, mut acl) = (null_mut(), null_mut());
    let flags = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    // The returned SID and ACL belong to descriptor, which the caller keeps alive.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            flags,
            &mut owner,
            null_mut(),
            &mut acl,
            null_mut(),
            &mut descriptor.0,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let (mut control, mut revision) = (0, 0);
    // The descriptor was initialized by GetSecurityInfo.
    if unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((descriptor, owner, acl, control))
}

pub(super) fn copy_security(stage: &Path, previous: Option<&Path>, secret: bool) -> io::Result<()> {
    if previous.is_none() && !secret {
        return Ok(());
    }
    let original = previous
        .map(|path| OpenOptions::new().access_mode(READ_CONTROL).open(path).and_then(|file| security(&file)))
        .transpose()?;
    let stage_file = OpenOptions::new().access_mode(READ_CONTROL | WRITE_DAC).open(stage)?;
    let (_current, current_owner, _, _) = security(&stage_file)?;
    let owner = original.as_ref().map_or(current_owner, |(_, owner, _, _)| *owner);
    // Both SIDs are backed by live descriptors.
    let owner_changes = unsafe { EqualSid(owner, current_owner) } == 0;
    let write_owner =
        owner_changes.then(|| OpenOptions::new().access_mode(WRITE_DAC | WRITE_OWNER).open(stage)).transpose()?;
    let mut private = Descriptor(null_mut());
    let (acl, protected) = if secret {
        let sddl = wide("D:P(A;;FA;;;OW)".as_ref());
        let (mut present, mut acl, mut defaulted) = (0, null_mut(), 0);
        // Protected ACL with a single allow entry for the file's owner.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut private.0,
                null_mut(),
            )
        } == 0
            || unsafe { GetSecurityDescriptorDacl(private.0, &mut present, &mut acl, &mut defaulted) } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if present == 0 || acl.is_null() {
            return Err(io::Error::other("Couldn't set owner-only access on the staged file"));
        }
        (acl, true)
    } else if let Some((_, _, acl, control)) = original.as_ref() {
        (*acl, control & SE_DACL_PROTECTED != 0)
    } else {
        return Ok(());
    };
    let flags = DACL_SECURITY_INFORMATION
        | if protected { PROTECTED_DACL_SECURITY_INFORMATION } else { UNPROTECTED_DACL_SECURITY_INFORMATION }
        | if owner_changes { OWNER_SECURITY_INFORMATION } else { 0 };
    // All input pointers remain valid until this call returns.
    let status = unsafe {
        SetSecurityInfo(
            write_owner.as_ref().unwrap_or(&stage_file).as_raw_handle(),
            SE_FILE_OBJECT,
            flags,
            owner,
            null_mut(),
            acl,
            null(),
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    Ok(())
}

fn wide(path: &std::ffi::OsStr) -> Vec<u16> {
    path.encode_wide().chain(std::iter::once(0)).collect()
}

pub(super) fn open_directory(path: &Path) -> io::Result<File> {
    OpenOptions::new().access_mode(GENERIC_WRITE).custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(path)
}

pub(super) fn commit(temp: tempfile::NamedTempFile, target: &Path, replace: bool) -> io::Result<()> {
    let mut temp = temp.into_temp_path();
    let source = wide(temp.as_os_str());
    let dest = wide(target.as_os_str());
    let flags = MOVEFILE_WRITE_THROUGH | if replace { MOVEFILE_REPLACE_EXISTING } else { 0 };
    // Drop the temporary attribute just as tempfile::persist does. Both paths
    // are NUL terminated and staging took place in the destination directory.
    let attributes = unsafe { GetFileAttributesW(source.as_ptr()) };
    if attributes == INVALID_FILE_ATTRIBUTES {
        return Err(io::Error::last_os_error());
    }
    let remaining = attributes & !FILE_ATTRIBUTE_TEMPORARY;
    let attributes = if remaining == 0 { FILE_ATTRIBUTE_NORMAL } else { remaining };
    if unsafe { SetFileAttributesW(source.as_ptr(), attributes) } == 0
        || unsafe { MoveFileExW(source.as_ptr(), dest.as_ptr(), flags) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    temp.disable_cleanup(true);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_writes_make_a_protected_owner_only_acl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        for value in ["X=first", "X=second"] {
            crate::file_write::replace(&path, value).unwrap();
            let file = File::open(&path).unwrap();
            let (_descriptor, _, acl, control) = security(&file).unwrap();
            assert_ne!(control & SE_DACL_PROTECTED, 0);
            assert!(!acl.is_null());
            assert_eq!(unsafe { (*acl).AceCount }, 1);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), value);
        }
    }
}
