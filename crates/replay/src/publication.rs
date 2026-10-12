//! Shared Windows POSIX rename and durable recording publication.

use std::{
    ffi::c_void,
    fs::{File, OpenOptions},
    io,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
};
use windows_sys::Win32::{
    Foundation::GENERIC_WRITE,
    Storage::FileSystem::{
        DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FileRenameInfoEx, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        MoveFileExW, SetFileInformationByHandle,
    },
};

/// FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS.
const RENAME_REPLACE_POSIX: u32 = 0x1 | 0x2;

/// Rename an open source with DELETE access to a NUL-terminated Windows path.
/// Delete-sharing readers retain the old destination. `Ok(false)` means the
/// volume does not support POSIX rename; all other failures propagate.
/// This performs no flush: each publisher owns its durability policy.
pub fn posix_replace(source: &File, target: &[u16]) -> io::Result<bool> {
    posix_replace_with(target, |info, size| {
        // The source handle and aligned, variable-length buffer remain live.
        if unsafe {
            SetFileInformationByHandle(source.as_raw_handle(), FileRenameInfoEx, info, size)
        } == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    })
}

pub(crate) fn posix_replace_with(
    target: &[u16],
    rename: impl FnOnce(*const c_void, u32) -> io::Result<()>,
) -> io::Result<bool> {
    let name = target
        .strip_suffix(&[0])
        .filter(|name| !name.is_empty() && !name.contains(&0))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid rename target"))?;
    // The name excludes its terminator; the struct keeps one u16 for it.
    let name_bytes = std::mem::size_of_val(name);
    let size = std::mem::offset_of!(FILE_RENAME_INFO, FileName) + name_bytes + 2;
    let size_u32 = u32::try_from(size).map_err(|_| io::Error::other("rename name too long"))?;
    let mut buffer = vec![0u64; size.div_ceil(8)];
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // u64 storage supplies the struct's alignment and the entire UTF-16 tail.
    unsafe {
        (*info).Anonymous.Flags = RENAME_REPLACE_POSIX;
        (*info).RootDirectory = std::ptr::null_mut();
        (*info).FileNameLength = name_bytes as u32;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            std::ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
            name.len(),
        );
    }
    match rename(info.cast(), size_u32) {
        // Keep the collector's unsupported-volume policy. Sharing and ACL
        // errors must not be disguised by trying another replacement API.
        Err(error) if matches!(error.raw_os_error(), Some(1 | 50 | 87)) => Ok(false),
        Ok(()) => Ok(true),
        Err(error) => Err(error),
    }
}

pub(crate) fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    replace_file_with(source, destination, posix_replace, || Ok(()))
}

pub(crate) fn replace_file_with(
    source: &Path,
    destination: &Path,
    rename: impl FnOnce(&File, &[u16]) -> io::Result<bool>,
    after_rename: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let destination = std::path::absolute(destination)?;
    // Acquire both flush-capable handles before changing the destination.
    let directory = publication_directory(&destination)?;
    let file = OpenOptions::new()
        .access_mode(DELETE | GENERIC_WRITE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(source)?;
    if !rename(&file, &wide(&destination))? {
        drop(file);
        drop(directory);
        return classic_replace(source, &destination);
    }
    after_rename()?;
    // FileRenameInfoEx has no write-through flag. Success acknowledges both
    // the renamed file and its common parent namespace, after the temp sync.
    file.sync_all()?;
    directory.sync_all()
}

/// Complete a visible recording's durability barrier without rewriting it.
/// A matching revision may be visible after an interrupted or failed flush.
pub fn sync_file(destination: &Path) -> io::Result<()> {
    let destination = std::path::absolute(destination)?;
    let directory = publication_directory(&destination)?;
    let file = OpenOptions::new()
        .access_mode(GENERIC_WRITE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(&destination)?;
    file.sync_all()?;
    directory.sync_all()
}

fn publication_directory(destination: &Path) -> io::Result<File> {
    let parent = destination.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "missing publication directory")
    })?;
    OpenOptions::new()
        .access_mode(GENERIC_WRITE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(parent)
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn classic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    let source = wide(source);
    let destination = wide(destination);
    // The source is a closed, fully synced sibling file on the same volume.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
