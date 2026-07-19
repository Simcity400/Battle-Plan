use super::{
    validate_binding, validate_mount_instance, DirAccessError, ListedObject, ObjectId, RootEntry,
};
use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::Read;
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

const RESOLVE_NO_XDEV: u64 = 0x01;
const RESOLVE_NO_SYMLINKS: u64 = 0x04;
const RESOLVE_BENEATH: u64 = 0x08;

#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}

#[derive(Debug, Clone, Copy)]
enum MountProof {
    Statx(u64),
    Openat2,
}

#[derive(Debug)]
pub(super) struct PlatformRoot {
    fd: OwnedFd,
    mount: MountProof,
}

impl PlatformRoot {
    pub(super) fn open(path: &Path) -> Result<Self, DirAccessError> {
        let path_c = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| DirAccessError::reject("store root contains a NUL byte"))?;
        let raw = unsafe {
            libc::open(
                path_c.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if raw < 0 {
            return Err(last_error(format!(
                "anchor directory root {}",
                path.display()
            )));
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let mount = match mount_id(fd.as_raw_fd())? {
            Some(id) => MountProof::Statx(id),
            None => {
                let probe = openat2_component(fd.as_raw_fd(), ".", true).map_err(|error| {
                    DirAccessError::reject(format!(
                        "Linux mount-instance proof unavailable: statx.stx_mnt_id absent and openat2 probe failed: {error}"
                    ))
                })?;
                drop(probe);
                MountProof::Openat2
            }
        };
        Ok(Self { fd, mount })
    }

    pub(super) fn root_entries(&self) -> Result<Vec<RootEntry>, DirAccessError> {
        Ok(list_directory(self.fd.as_raw_fd())?
            .into_iter()
            .map(|entry| RootEntry {
                name: entry.name,
                directory_or_link: entry.directory_or_link,
            })
            .collect())
    }

    pub(super) fn read_file(&self, components: &[&str]) -> Result<Vec<u8>, DirAccessError> {
        let mut opened_directories: Vec<OwnedFd> = Vec::new();
        let mut parent_fd = self.fd.as_raw_fd();
        let mut parent_mount = self.mount;

        for (index, expected) in components.iter().enumerate() {
            let directory = index + 1 != components.len();
            let opened = match open_component(parent_fd, expected, directory, parent_mount) {
                Ok(value) => value,
                Err(error) if error.not_found => {
                    reject_missing_lookalike(parent_fd, expected)?;
                    return Err(error);
                }
                Err(error) => return Err(error),
            };
            let entries = list_directory(parent_fd)?;
            let mut opened_id = object_id(opened.fd.as_raw_fd())?;
            // Within one already-verified parent mount, the atomic d_ino from
            // readdir is the binding key. Do not fold st_dev into this
            // same-parent comparison: overlayfs may report a different st_dev
            // for a child while statx proves one mount instance.
            opened_id.volume = 0;
            validate_binding(expected, expected, &opened_id, &listed_objects(&entries))?;

            if directory {
                parent_mount = opened.mount;
                opened_directories.push(opened.fd);
                parent_fd = opened_directories
                    .last()
                    .expect("stored parent")
                    .as_raw_fd();
            } else {
                let mut file = File::from(opened.fd);
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).map_err(|error| {
                    DirAccessError::reject(format!(
                        "single handle read of component {expected:?} failed: {error}"
                    ))
                })?;
                return Ok(bytes);
            }
        }
        unreachable!("validated nonempty components")
    }
}

struct Opened {
    fd: OwnedFd,
    mount: MountProof,
}

fn open_component(
    parent_fd: RawFd,
    expected: &str,
    directory: bool,
    parent_mount: MountProof,
) -> Result<Opened, DirAccessError> {
    let name = CString::new(expected)
        .map_err(|_| DirAccessError::reject(format!("component {expected:?} contains NUL")))?;
    match parent_mount {
        MountProof::Statx(parent_id) => {
            let flags = libc::O_RDONLY
                | libc::O_CLOEXEC
                | libc::O_NOFOLLOW
                | if directory { libc::O_DIRECTORY } else { 0 };
            let raw = unsafe { libc::openat(parent_fd, name.as_ptr(), flags) };
            if raw < 0 {
                return Err(open_error(parent_fd, expected, &name));
            }
            let fd = unsafe { OwnedFd::from_raw_fd(raw) };
            let child_id = mount_id(fd.as_raw_fd())?.ok_or_else(|| {
                DirAccessError::reject(format!(
                    "statx.stx_mnt_id disappeared while opening {expected:?}; refusing st_dev-only acceptance"
                ))
            })?;
            validate_mount_instance(parent_id, child_id)?;
            Ok(Opened {
                fd,
                mount: MountProof::Statx(child_id),
            })
        }
        MountProof::Openat2 => Ok(Opened {
            fd: openat2_component(parent_fd, expected, directory)?,
            mount: MountProof::Openat2,
        }),
    }
}

fn openat2_component(
    parent_fd: RawFd,
    expected: &str,
    directory: bool,
) -> Result<OwnedFd, DirAccessError> {
    let name = CString::new(expected)
        .map_err(|_| DirAccessError::reject(format!("component {expected:?} contains NUL")))?;
    let how = OpenHow {
        flags: (libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | if directory { libc::O_DIRECTORY } else { 0 }) as u64,
        mode: 0,
        resolve: RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_XDEV,
    };
    let raw = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            parent_fd,
            name.as_ptr(),
            &how,
            std::mem::size_of::<OpenHow>(),
        ) as RawFd
    };
    if raw < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EXDEV) {
            return Err(DirAccessError::reject(format!(
                "mount-instance containment rejected openat2 RESOLVE_NO_XDEV crossing at component {expected:?}: {error}"
            )));
        }
        return Err(open_error(parent_fd, expected, &name));
    }
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

fn mount_id(fd: RawFd) -> Result<Option<u64>, DirAccessError> {
    let mut statx = MaybeUninit::<libc::statx>::zeroed();
    let empty = b"\0";
    let rc = unsafe {
        libc::statx(
            fd,
            empty.as_ptr().cast(),
            libc::AT_EMPTY_PATH | libc::AT_SYMLINK_NOFOLLOW,
            libc::STATX_MNT_ID,
            statx.as_mut_ptr(),
        )
    };
    if rc == 0 {
        let statx = unsafe { statx.assume_init() };
        return Ok((statx.stx_mask & libc::STATX_MNT_ID != 0).then_some(statx.stx_mnt_id));
    }
    let error = std::io::Error::last_os_error();
    if matches!(
        error.raw_os_error(),
        Some(libc::ENOSYS) | Some(libc::EINVAL)
    ) {
        Ok(None)
    } else {
        Err(DirAccessError::reject(format!(
            "query statx.stx_mnt_id on opened handle: {error}"
        )))
    }
}

#[derive(Debug)]
struct LinuxEntry {
    name: String,
    id: ObjectId,
    directory_or_link: bool,
}

fn list_directory(fd: RawFd) -> Result<Vec<LinuxEntry>, DirAccessError> {
    let duplicated = unsafe { libc::dup(fd) };
    if duplicated < 0 {
        return Err(last_error(
            "duplicate parent fd for handle-relative enumeration",
        ));
    }
    let directory = unsafe { libc::fdopendir(duplicated) };
    if directory.is_null() {
        unsafe { libc::close(duplicated) };
        return Err(last_error("fdopendir parent handle"));
    }
    let mut entries = Vec::new();
    loop {
        unsafe { *libc::__errno_location() = 0 };
        let raw = unsafe { libc::readdir(directory) };
        if raw.is_null() {
            let errno = unsafe { *libc::__errno_location() };
            unsafe { libc::closedir(directory) };
            if errno == 0 {
                return Ok(entries);
            }
            return Err(DirAccessError::reject(format!(
                "readdir through parent handle failed: {}",
                std::io::Error::from_raw_os_error(errno)
            )));
        }
        let entry = unsafe { &*raw };
        let name_bytes = unsafe { CStr::from_ptr(entry.d_name.as_ptr()) }.to_bytes();
        if name_bytes == b"." || name_bytes == b".." {
            continue;
        }
        let name = String::from_utf8(name_bytes.to_vec())
            .unwrap_or_else(|_| format!("<non-utf8:{}>", hex(name_bytes)));
        let mut file = [0u8; 16];
        file[..8].copy_from_slice(&(entry.d_ino as u64).to_le_bytes());
        let directory_or_link = entry.d_type == libc::DT_DIR
            || entry.d_type == libc::DT_LNK
            || entry.d_type == libc::DT_UNKNOWN;
        entries.push(LinuxEntry {
            name,
            id: ObjectId { volume: 0, file },
            directory_or_link,
        });
    }
}

fn listed_objects(entries: &[LinuxEntry]) -> Vec<ListedObject> {
    entries
        .iter()
        .map(|entry| ListedObject {
            name: entry.name.clone(),
            id: entry.id.clone(),
        })
        .collect()
}

fn reject_missing_lookalike(parent_fd: RawFd, expected: &str) -> Result<(), DirAccessError> {
    let entries = list_directory(parent_fd)?;
    let variants: Vec<&str> = entries
        .iter()
        .map(|entry| entry.name.as_str())
        .filter(|name| name.eq_ignore_ascii_case(expected))
        .collect();
    if variants.iter().any(|name| *name != expected) {
        Err(DirAccessError::reject(format!(
            "ASCII-lookalike policy rejected missing component {expected:?}; variants: {variants:?}"
        )))
    } else {
        Ok(())
    }
}

fn object_id(fd: RawFd) -> Result<ObjectId, DirAccessError> {
    let mut stat = MaybeUninit::<libc::stat>::zeroed();
    if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
        return Err(last_error("fstat opened component identity"));
    }
    let stat = unsafe { stat.assume_init() };
    let mut file = [0u8; 16];
    file[..8].copy_from_slice(&(stat.st_ino as u64).to_le_bytes());
    Ok(ObjectId {
        volume: stat.st_dev as u64,
        file,
    })
}

fn open_error(parent_fd: RawFd, expected: &str, name: &CString) -> DirAccessError {
    let error = std::io::Error::last_os_error();
    let mut stat = MaybeUninit::<libc::stat>::zeroed();
    let is_symlink = unsafe {
        libc::fstatat(
            parent_fd,
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        ) == 0
            && stat.assume_init().st_mode & libc::S_IFMT == libc::S_IFLNK
    };
    if error.raw_os_error() == Some(libc::ELOOP) || is_symlink {
        DirAccessError::reject(format!(
            "name-surrogate symlink rejected at component {expected:?}"
        ))
    } else if error.kind() == std::io::ErrorKind::NotFound {
        DirAccessError::missing(format!("component {expected:?} not found: {error}"))
    } else {
        DirAccessError::reject(format!(
            "openat component {expected:?} relative to anchored parent: {error}"
        ))
    }
}

fn last_error(context: impl AsRef<str>) -> DirAccessError {
    let error = std::io::Error::last_os_error();
    DirAccessError::reject(format!("{}: {error}", context.as_ref()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
