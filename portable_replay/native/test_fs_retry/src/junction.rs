//! Native NTFS junction (mount-point reparse point) creation for test
//! fixtures.
//!
//! Every former call site built its junction by shelling out to the cmd link
//! builtin through a console-subsystem child. A harness process without a
//! console cannot pass one down, so Windows allocated a fresh console per call
//! and (with Windows Terminal as the default host) each allocation flashed a
//! blank terminal window on the desktop. This module performs the identical
//! operation in-process -- CreateFileW on the new directory followed by
//! FSCTL_SET_REPARSE_POINT with an IO_REPARSE_TAG_MOUNT_POINT reparse data
//! buffer -- so no child process, and no console, ever exists.
//!
//! Kept dependency-free on purpose (see the crate manifest): the few Win32
//! entry points are declared here directly against kernel32.

/// The reparse tag every junction carries.
pub const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;

#[cfg(windows)]
mod imp {
    use super::IO_REPARSE_TAG_MOUNT_POINT;
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::path::Path;

    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_SHARE_DELETE: u32 = 0x0000_0004;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00A4;
    // FILE_INFO_BY_HANDLE_CLASS::FileAttributeTagInfo == 9 (windows-sys
    // 0.61.2 is the local authority; not the FileStandardInfo slot 1).
    const FILE_ATTRIBUTE_TAG_INFO_CLASS: i32 = 9;
    const INVALID_HANDLE_VALUE: *mut c_void = -1isize as *mut c_void;

    #[allow(non_snake_case)]
    #[repr(C)]
    struct FILE_ATTRIBUTE_TAG_INFO {
        FileAttributes: u32,
        ReparseTag: u32,
    }

    extern "system" {
        #[link_name = "CreateFileW"]
        fn create_file_w(
            file_name: *const u16,
            desired_access: u32,
            share_mode: u32,
            security_attributes: *const c_void,
            creation_disposition: u32,
            flags_and_attributes: u32,
            template_file: *mut c_void,
        ) -> *mut c_void;

        #[link_name = "DeviceIoControl"]
        fn device_io_control(
            device: *mut c_void,
            control_code: u32,
            in_buffer: *const c_void,
            in_buffer_size: u32,
            out_buffer: *mut c_void,
            out_buffer_size: u32,
            bytes_returned: *mut u32,
            overlapped: *const c_void,
        ) -> i32;

        #[link_name = "GetFileInformationByHandleEx"]
        fn get_file_information_by_handle_ex(
            file: *mut c_void,
            info_class: i32,
            info: *mut c_void,
            info_size: u32,
        ) -> i32;
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }

    /// `\??\`-prefixed substitute name for the junction's absolute target.
    ///
    /// canonicalize hands back a verbatim `\\?\` path; the mount-point
    /// substitute form is the same absolute path under the NT namespace
    /// prefix (`\??\C:\...`; a UNC verbatim `\\?\UNC\...` keeps its `UNC`
    /// segment, so prefixing the stripped remainder is correct either way).
    fn substitute_name_wide(target: &Path) -> Result<Vec<u16>, String> {
        let verbatim = std::fs::canonicalize(target)
            .map_err(|error| format!("resolve junction target {}: {error}", target.display()))?;
        let wide: Vec<u16> = verbatim.as_os_str().encode_wide().collect();
        let device_prefix = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
        if !wide.starts_with(&device_prefix) {
            return Err(format!(
                "junction target {} did not canonicalize to a verbatim path",
                target.display()
            ));
        }
        let mut name = vec![b'\\' as u16, b'?' as u16, b'?' as u16, b'\\' as u16];
        name.extend_from_slice(&wide[4..]);
        Ok(name)
    }

    /// Build the little-endian REPARSE_DATA_BUFFER for a mount point.
    ///
    /// Layout: ReparseTag(u32), ReparseDataLength(u16), Reserved(u16),
    /// SubstituteNameOffset(u16), SubstituteNameLength(u16),
    /// PrintNameOffset(u16), PrintNameLength(u16), then the path buffer.
    fn mount_point_reparse_buffer(substitute: &[u16], print: &[u16]) -> Vec<u8> {
        let substitute_bytes: Vec<u8> = substitute.iter().flat_map(|u| u.to_le_bytes()).collect();
        let print_bytes: Vec<u8> = print.iter().flat_map(|u| u.to_le_bytes()).collect();
        let substitute_padded_bytes = substitute_bytes.len() + 2; // trailing NUL
        let paths_bytes = substitute_padded_bytes + print_bytes.len() + 2;
        let mut buffer = Vec::with_capacity(8 + paths_bytes);
        buffer.extend_from_slice(&IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
        buffer.extend_from_slice(&((8 + paths_bytes) as u16).to_le_bytes()); // ReparseDataLength
        buffer.extend_from_slice(&0u16.to_le_bytes()); // Reserved
        buffer.extend_from_slice(&0u16.to_le_bytes()); // SubstituteNameOffset
        buffer.extend_from_slice(&(substitute_bytes.len() as u16).to_le_bytes());
        buffer.extend_from_slice(&(substitute_padded_bytes as u16).to_le_bytes()); // PrintNameOffset
        buffer.extend_from_slice(&(print_bytes.len() as u16).to_le_bytes());
        buffer.extend_from_slice(&substitute_bytes);
        buffer.extend_from_slice(&[0, 0]);
        buffer.extend_from_slice(&print_bytes);
        buffer.extend_from_slice(&[0, 0]);
        buffer
    }

    pub fn create_junction(link: &Path, target: &Path) -> Result<(), String> {
        if !target.is_dir() {
            return Err(format!(
                "junction target {} does not name an existing directory",
                target.display()
            ));
        }
        let substitute = substitute_name_wide(target)?;

        if link.exists() {
            // symlink_metadata does not follow a reparse point: an existing
            // junction here must be refused, never silently repointed (the
            // retired shell form also failed when the name was taken). Only a
            // genuinely empty plain directory is convertible.
            let existing = std::fs::symlink_metadata(link)
                .map_err(|error| format!("inspect {}: {error}", link.display()))?;
            if existing.file_type().is_symlink() {
                return Err(format!(
                    "refusing an occupied link path {} (already a reparse point)",
                    link.display()
                ));
            }
            let empty_directory =
                std::fs::read_dir(link).map_err(|error| format!("inspect {}: {error}", link.display()))?
                    .next()
                    .is_none();
            if !empty_directory {
                return Err(format!(
                    "refusing an occupied link path {}",
                    link.display()
                ));
            }
        } else {
            std::fs::create_dir(link)
                .map_err(|error| format!("create link directory {}: {error}", link.display()))?;
        }

        let link_wide = wide(link);
        let raw = unsafe {
            create_file_w(
                link_wide.as_ptr(),
                GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            return Err(format!(
                "open link directory {}: {}",
                link.display(),
                std::io::Error::last_os_error()
            ));
        }
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };

        let print_name: Vec<u16> = target.as_os_str().encode_wide().collect();
        let buffer = mount_point_reparse_buffer(&substitute, &print_name);
        let mut returned: u32 = 0;
        let ok = unsafe {
            device_io_control(
                handle.as_raw_handle(),
                FSCTL_SET_REPARSE_POINT,
                buffer.as_ptr().cast(),
                buffer.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null(),
            )
        };
        if ok == 0 {
            return Err(format!(
                "set the mount-point reparse point on {}: {}",
                link.display(),
                std::io::Error::last_os_error()
            ));
        }

        match reparse_tag_of(link) {
            Ok(tag) if tag == IO_REPARSE_TAG_MOUNT_POINT => Ok(()),
            other => Err(format!(
                "the junction at {} did not read back as a mount point: {:?}",
                link.display(),
                other
            )),
        }
    }

    pub fn reparse_tag_of(path: &Path) -> Result<u32, String> {
        let wide_path = wide(path);
        let raw = unsafe {
            create_file_w(
                wide_path.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            return Err(format!(
                "open {} for its reparse tag: {}",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut info = FILE_ATTRIBUTE_TAG_INFO {
            FileAttributes: 0,
            ReparseTag: 0,
        };
        let ok = unsafe {
            get_file_information_by_handle_ex(
                handle.as_raw_handle(),
                FILE_ATTRIBUTE_TAG_INFO_CLASS,
                &mut info as *mut FILE_ATTRIBUTE_TAG_INFO as *mut c_void,
                std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        };
        if ok == 0 {
            return Err(format!(
                "query the tag of {}: {}",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
        Ok(info.ReparseTag)
    }
}

#[cfg(windows)]
pub use imp::{create_junction, reparse_tag_of};

#[cfg(not(windows))]
mod stub {
    use std::path::Path;

    pub fn create_junction(_link: &Path, _target: &Path) -> Result<(), String> {
        Err("junction fixtures are Windows-only".to_string())
    }

    pub fn reparse_tag_of(_path: &Path) -> Result<u32, String> {
        Err("junction fixtures are Windows-only".to_string())
    }
}

#[cfg(not(windows))]
pub use stub::{create_junction, reparse_tag_of};
