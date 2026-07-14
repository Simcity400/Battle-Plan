use super::{
    identity_bound_handle, validate_binding, validate_reopen_identity, DirAccessError,
    ListedObject, ObjectId, RootEntry,
};
use std::fs::File;
use std::io::Read;
use std::mem::{size_of, MaybeUninit};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    NtCreateFile, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN,
    FILE_OPEN_FOR_BACKUP_INTENT, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,
};
use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_NO_MORE_FILES, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE, UNICODE_STRING,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FileAttributeTagInfo, FileIdExtdDirectoryInfo, FileIdExtdDirectoryRestartInfo,
    FileIdInfo, FileNormalizedNameInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_GENERIC_READ, FILE_ID_EXTD_DIR_INFO, FILE_ID_INFO, FILE_NAME_INFO, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

const OBJ_CASE_INSENSITIVE: u32 = 0x40;
const STATUS_OBJECT_NAME_NOT_FOUND: u32 = 0xC000_0034;
const STATUS_OBJECT_PATH_NOT_FOUND: u32 = 0xC000_003A;
const STATUS_NO_SUCH_FILE: u32 = 0xC000_000F;

#[derive(Debug)]
pub(super) struct PlatformRoot {
    handle: OwnedHandle,
    id: ObjectId,
}

impl PlatformRoot {
    pub(super) fn open(path: &Path) -> Result<Self, DirAccessError> {
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        wide.push(0);
        let raw = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            return Err(win32_error(format!(
                "anchor directory root {}",
                path.display()
            )));
        }
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        let id = object_id(raw)?;
        Ok(Self { handle, id })
    }

    pub(super) fn open_child_dir(&self, component: &str) -> Result<Self, DirAccessError> {
        let opened = open_component(raw(&self.handle), &self.id, component, true)?;
        Ok(Self {
            handle: opened.handle,
            id: opened.id,
        })
    }

    pub(super) fn validate_child_dir(
        &self,
        component: &str,
        expected: &Self,
    ) -> Result<(), DirAccessError> {
        let current = open_component(raw(&self.handle), &self.id, component, true)?;
        validate_reopen_identity(&expected.id, &current.id).map_err(|error| {
            DirAccessError::reject(format!(
                "selected directory binding for {component:?} changed: {}",
                error.message
            ))
        })
    }

    pub(super) fn root_entries(&self) -> Result<Vec<RootEntry>, DirAccessError> {
        Ok(list_directory(raw(&self.handle), self.id.volume)?
            .into_iter()
            .map(|entry| RootEntry {
                name: entry.object.name,
                directory_or_link: entry.directory_or_link,
            })
            .collect())
    }

    pub(super) fn read_file(&self, components: &[&str]) -> Result<Vec<u8>, DirAccessError> {
        let mut opened_directories: Vec<OwnedHandle> = Vec::new();
        let mut parent = raw(&self.handle);
        let mut parent_id = self.id.clone();
        for (index, expected) in components.iter().enumerate() {
            let directory = index + 1 != components.len();
            let opened = open_component(parent, &parent_id, expected, directory)?;
            if directory {
                parent_id = opened.id;
                opened_directories.push(opened.handle);
                parent = raw(opened_directories.last().expect("stored parent"));
            } else {
                let mut file = File::from(opened.handle);
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
    handle: OwnedHandle,
    id: ObjectId,
}

fn open_component(
    parent: HANDLE,
    parent_id: &ObjectId,
    expected: &str,
    directory: bool,
) -> Result<Opened, DirAccessError> {
    let inspected = nt_open_relative(parent, expected, directory, true)?;
    let inspected_raw = raw(&inspected);
    let tag = reparse_tag(inspected_raw)?;
    let inspected_id = object_id(inspected_raw)?;
    if inspected_id.volume != parent_id.volume {
        return Err(DirAccessError::reject(format!(
            "Windows volume-serial containment rejected component {expected:?}: {} -> {}",
            parent_id.volume, inspected_id.volume
        )));
    }
    let actual = normalized_leaf_name(inspected_raw)?;
    let entries = list_directory(parent, parent_id.volume)?;
    validate_binding(
        expected,
        &actual,
        &inspected_id,
        &entries
            .iter()
            .map(|entry| entry.object.clone())
            .collect::<Vec<_>>(),
    )?;

    let (handle, id) = identity_bound_handle(tag, inspected, &inspected_id, || {
        let reopened = nt_open_relative(parent, expected, directory, false)?;
        let reopened_id = object_id(raw(&reopened))?;
        Ok((reopened, reopened_id))
    })
    .map_err(|error| {
        DirAccessError::reject(format!("{} at component {expected:?}", error.message))
    })?;
    Ok(Opened { handle, id })
}

fn nt_open_relative(
    parent: HANDLE,
    expected: &str,
    directory: bool,
    no_reparse: bool,
) -> Result<OwnedHandle, DirAccessError> {
    let mut wide: Vec<u16> = expected.encode_utf16().collect();
    if wide.is_empty() || wide.len() > (u16::MAX as usize / 2) {
        return Err(DirAccessError::reject(format!(
            "component length is unsupported for NtCreateFile: {expected:?}"
        )));
    }
    let unicode = UNICODE_STRING {
        Length: (wide.len() * 2) as u16,
        MaximumLength: (wide.len() * 2) as u16,
        Buffer: wide.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent,
        ObjectName: &unicode,
        Attributes: OBJ_CASE_INSENSITIVE,
        SecurityDescriptor: null(),
        SecurityQualityOfService: null(),
    };
    let mut io = MaybeUninit::<IO_STATUS_BLOCK>::zeroed();
    let mut handle: HANDLE = null_mut();
    let mut options = FILE_SYNCHRONOUS_IO_NONALERT
        | FILE_OPEN_FOR_BACKUP_INTENT
        | if directory {
            FILE_DIRECTORY_FILE
        } else {
            FILE_NON_DIRECTORY_FILE
        };
    if no_reparse {
        options |= FILE_OPEN_REPARSE_POINT;
    }
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            FILE_GENERIC_READ,
            &attributes,
            io.as_mut_ptr(),
            null(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            options,
            null(),
            0,
        )
    };
    if status < 0 {
        let status = status as u32;
        if matches!(
            status,
            STATUS_OBJECT_NAME_NOT_FOUND | STATUS_OBJECT_PATH_NOT_FOUND | STATUS_NO_SUCH_FILE
        ) {
            let entries = list_directory(parent, object_id(parent)?.volume)?;
            let variants: Vec<&str> = entries
                .iter()
                .map(|entry| entry.object.name.as_str())
                .filter(|name| name.eq_ignore_ascii_case(expected))
                .collect();
            if variants.iter().any(|name| *name != expected) {
                return Err(DirAccessError::reject(format!(
                    "ASCII-lookalike policy rejected missing component {expected:?}; variants: {variants:?}"
                )));
            }
            return Err(DirAccessError::missing(format!(
                "NtCreateFile component {expected:?} not found (NTSTATUS {status:#010x})"
            )));
        }
        return Err(DirAccessError::reject(format!(
            "NtCreateFile component {expected:?} relative to anchored parent failed with NTSTATUS {status:#010x}"
        )));
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

fn object_id(handle: HANDLE) -> Result<ObjectId, DirAccessError> {
    let mut info = MaybeUninit::<FILE_ID_INFO>::zeroed();
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            info.as_mut_ptr().cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(win32_error("query FileIdInfo for opened component"));
    }
    let info = unsafe { info.assume_init() };
    ObjectId::from_windows_file_id(info.VolumeSerialNumber, info.FileId.Identifier)
}

fn reparse_tag(handle: HANDLE) -> Result<u32, DirAccessError> {
    let mut info = MaybeUninit::<FILE_ATTRIBUTE_TAG_INFO>::zeroed();
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileAttributeTagInfo,
            info.as_mut_ptr().cast(),
            size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(win32_error(
            "query FileAttributeTagInfo for opened component",
        ));
    }
    let info = unsafe { info.assume_init() };
    Ok(if info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        info.ReparseTag
    } else {
        0
    })
}

fn normalized_leaf_name(handle: HANDLE) -> Result<String, DirAccessError> {
    let mut storage = vec![0u64; 4096];
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileNormalizedNameInfo,
            storage.as_mut_ptr().cast(),
            (storage.len() * size_of::<u64>()) as u32,
        )
    };
    if ok == 0 {
        return Err(win32_error(
            "namespace binding FileNormalizedNameInformation unsupported",
        ));
    }
    let info = unsafe { &*(storage.as_ptr().cast::<FILE_NAME_INFO>()) };
    let units = info.FileNameLength as usize / 2;
    let name = unsafe { std::slice::from_raw_parts(info.FileName.as_ptr(), units) };
    let leaf_start = name
        .iter()
        .rposition(|unit| *unit == 92u16 || *unit == b'/' as u16)
        .map_or(0, |index| index + 1);
    String::from_utf16(&name[leaf_start..]).map_err(|_| {
        DirAccessError::reject("namespace binding normalized stored name is not valid UTF-16")
    })
}

#[derive(Debug)]
struct WindowsEntry {
    object: ListedObject,
    directory_or_link: bool,
}

fn list_directory(handle: HANDLE, volume: u64) -> Result<Vec<WindowsEntry>, DirAccessError> {
    let mut output = Vec::new();
    let mut restart = true;
    loop {
        let mut storage = vec![0u64; 8192];
        let class = if restart {
            FileIdExtdDirectoryRestartInfo
        } else {
            FileIdExtdDirectoryInfo
        };
        let ok = unsafe {
            GetFileInformationByHandleEx(
                handle,
                class,
                storage.as_mut_ptr().cast(),
                (storage.len() * size_of::<u64>()) as u32,
            )
        };
        if ok == 0 {
            let code = unsafe { GetLastError() };
            if code == ERROR_NO_MORE_FILES {
                return Ok(output);
            }
            return Err(win32_error(
                "enumerate directory through the same parent handle",
            ));
        }
        restart = false;
        let mut offset = 0usize;
        loop {
            if offset + size_of::<FILE_ID_EXTD_DIR_INFO>() > storage.len() * size_of::<u64>() {
                return Err(DirAccessError::reject(
                    "FileIdExtdDirectoryInfo returned a truncated record",
                ));
            }
            let entry = unsafe {
                &*(storage
                    .as_ptr()
                    .cast::<u8>()
                    .add(offset)
                    .cast::<FILE_ID_EXTD_DIR_INFO>())
            };
            let units = entry.FileNameLength as usize / 2;
            let name_units = unsafe { std::slice::from_raw_parts(entry.FileName.as_ptr(), units) };
            let name = String::from_utf16(name_units).map_err(|_| {
                DirAccessError::reject("directory stored-name enumeration returned invalid UTF-16")
            })?;
            if name != "." && name != ".." {
                let id = ObjectId::from_windows_file_id(volume, entry.FileId.Identifier)?;
                output.push(WindowsEntry {
                    object: ListedObject { name, id },
                    directory_or_link: entry.FileAttributes
                        & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
                        != 0,
                });
            }
            if entry.NextEntryOffset == 0 {
                break;
            }
            offset = offset
                .checked_add(entry.NextEntryOffset as usize)
                .ok_or_else(|| DirAccessError::reject("FileIdExtdDirectoryInfo offset overflow"))?;
        }
    }
}

fn raw(handle: &OwnedHandle) -> HANDLE {
    handle.as_raw_handle()
}

fn win32_error(context: impl AsRef<str>) -> DirAccessError {
    DirAccessError::reject(format!(
        "{}: {}",
        context.as_ref(),
        std::io::Error::last_os_error()
    ))
}
