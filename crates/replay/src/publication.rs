//! Shared Windows POSIX rename and durable recording publication.

use std::{
    ffi::c_void,
    fs::{File, OpenOptions},
    io,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::{Component, Path, Prefix},
};
use windows_sys::Win32::{
    Foundation::GENERIC_WRITE,
    Storage::FileSystem::{
        DELETE, FILE_ATTRIBUTE_HIDDEN, FILE_DISPOSITION_INFO_EX, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FileDispositionInfoEx, FileRenameInfoEx, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        MoveFileExW, SetFileAttributesW, SetFileInformationByHandle,
    },
};

/// FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS.
const RENAME_REPLACE_POSIX: u32 = 0x1 | 0x2;

/// The temporary file beside `path` for `purpose`, named by a digest of `path`'s file name: any
/// legal file name leaves it room, and the same file, whatever the case of its name, always has
/// the same one.
pub fn temporary_beside(path: &Path, purpose: &str) -> std::path::PathBuf {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let digest = blake3::hash(name.as_bytes());
    let hex: String = digest.as_bytes()[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    path.with_file_name(format!("{hex}.{purpose}.tmp"))
}

/// Where a folder's recordings with unsaved hours wait, each as saving it would be.
pub fn unsaved_folder(folder: &Path) -> std::path::PathBuf {
    folder.join(".unsaved")
}

/// A campaign's unsaved recording in `folder`, by campaign id whatever its recording is called.
pub fn unsaved_path(folder: &Path, campaign: &str) -> std::path::PathBuf {
    unsaved_folder(folder).join(format!("{campaign}.battleplan"))
}

/// The unsaved folder of `folder`, created hidden so Explorer does not show it.
pub fn create_unsaved_folder(folder: &Path) -> io::Result<std::path::PathBuf> {
    let unsaved = unsaved_folder(folder);
    std::fs::create_dir_all(&unsaved)?;
    let wide = wide(&unsaved)?;
    // SAFETY: `wide` is a NUL-terminated path that outlives the call.
    if unsafe { SetFileAttributesW(wide.as_ptr(), FILE_ATTRIBUTE_HIDDEN) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsaved)
}

/// Remove a file's name now; delete-sharing readers retain its bytes until close.
/// Unsupported POSIX deletion is an error, with no deferred-delete fallback.
pub fn posix_delete(path: &Path) -> io::Result<()> {
    let directory = publication_directory(&std::path::absolute(path)?)?;
    let file = OpenOptions::new()
        .access_mode(DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)?;
    // FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS.
    let info = FILE_DISPOSITION_INFO_EX { Flags: 0x1 | 0x2 };
    // The file handle and correctly sized disposition remain live for the call.
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfoEx,
            (&info as *const FILE_DISPOSITION_INFO_EX).cast(),
            std::mem::size_of_val(&info) as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    drop(file);
    directory.sync_all()
}

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

/// A failed replacement, by whether `destination` already names the new file.
#[derive(Debug)]
pub enum ReplaceError {
    /// Nothing was renamed: `destination` still names its old file, if any.
    Unrenamed(io::Error),
    /// `destination` names the new file, but making it durable failed.
    Unflushed(io::Error),
}

impl From<ReplaceError> for io::Error {
    fn from(error: ReplaceError) -> Self {
        match error {
            ReplaceError::Unrenamed(error) | ReplaceError::Unflushed(error) => error,
        }
    }
}

impl std::fmt::Display for ReplaceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unrenamed(error) | Self::Unflushed(error) => {
                std::fmt::Display::fmt(error, formatter)
            }
        }
    }
}

/// Durably replace `destination` with the closed, synced sibling `source`, even
/// while delete-sharing readers hold the destination open.
pub fn replace_file(source: &Path, destination: &Path) -> Result<(), ReplaceError> {
    replace_file_with(source, destination, posix_replace, || Ok(()))
}

pub(crate) fn replace_file_with(
    source: &Path,
    destination: &Path,
    rename: impl FnOnce(&File, &[u16]) -> io::Result<bool>,
    after_rename: impl FnOnce() -> io::Result<()>,
) -> Result<(), ReplaceError> {
    let unrenamed = ReplaceError::Unrenamed;
    let destination = std::path::absolute(destination).map_err(unrenamed)?;
    // Acquire both flush-capable handles before changing the destination.
    let directory = publication_directory(&destination).map_err(unrenamed)?;
    let file = OpenOptions::new()
        .access_mode(DELETE | GENERIC_WRITE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(source)
        .map_err(unrenamed)?;
    if !rename(&file, &wide(&destination).map_err(unrenamed)?).map_err(unrenamed)? {
        drop(file);
        drop(directory);
        return classic_replace(source, &destination).map_err(unrenamed);
    }
    let unflushed = ReplaceError::Unflushed;
    after_rename().map_err(unflushed)?;
    // FileRenameInfoEx has no write-through flag. Success acknowledges both
    // the renamed file and its common parent namespace, after the temp sync.
    file.sync_all().map_err(unflushed)?;
    directory.sync_all().map_err(unflushed)
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

/// Durably rename within a volume, never replacing `target`; refused while `source` is held unshared.
pub fn rename_new(source: &Path, target: &Path) -> io::Result<()> {
    let (from, to) = (wide(source)?, wide(target)?);
    // Both paths stay alive and NUL-terminated for the call.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
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

fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let path = std::path::absolute(path)?;
    let (prefix, skip) = match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(_) => (r"\\?\", 0),
            Prefix::UNC(..) => (r"\\?\UNC\", 2),
            _ => ("", 0),
        },
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "missing path prefix",
            ));
        }
    };
    let mut wide: Vec<u16> = prefix
        .encode_utf16()
        .chain(path.as_os_str().encode_wide().skip(skip))
        .collect();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path contains NUL",
        ));
    }
    wide.push(0);
    Ok(wide)
}

fn classic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    let source = wide(source)?;
    let destination = wide(destination)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};

    #[test]
    fn win32_paths_are_absolute_and_verbatim_without_resolving_the_file() {
        for (input, expected) in [
            (
                r"C:\recordings\old\..\new.battleplan",
                r"\\?\C:\recordings\new.battleplan",
            ),
            (
                r"\\server\share\new.battleplan",
                r"\\?\UNC\server\share\new.battleplan",
            ),
            (
                r"\\?\C:\recordings\new.battleplan",
                r"\\?\C:\recordings\new.battleplan",
            ),
            (
                r"\\?\UNC\server\share\new.battleplan",
                r"\\?\UNC\server\share\new.battleplan",
            ),
        ] {
            assert_eq!(
                wide(Path::new(input)).unwrap(),
                expected.encode_utf16().chain([0]).collect::<Vec<_>>()
            );
        }
        let relative = Path::new("new.battleplan");
        assert_eq!(
            wide(relative).unwrap(),
            wide(&std::env::current_dir().unwrap().join(relative)).unwrap()
        );
        let invalid = OsString::from_wide(&[b'C' as u16, b':' as u16, b'\\' as u16, 0, 65]);
        assert_eq!(
            wide(Path::new(&invalid)).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
