
use std::ffi::{c_void, OsString};
use std::fs::{self, File};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Component, Path, PathBuf};
use std::ptr;

type Handle = *mut c_void;
const INVALID_HANDLE_VALUE: Handle = usize::MAX as Handle;
const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_USER_CLASS: u32 = 1;
const OWNER_SECURITY_INFORMATION: u32 = 0x0000_0001;
const DACL_SECURITY_INFORMATION: u32 = 0x0000_0004;
const PROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x8000_0000;
const SE_DACL_PROTECTED: u16 = 0x1000;
const SE_FILE_OBJECT: u32 = 1;
const SDDL_REVISION_1: u32 = 1;

#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

const FOLDERID_LOCAL_APP_DATA: Guid = Guid {
    data1: 0xF1B3_2785,
    data2: 0x6FBA,
    data3: 0x4FCF,
    data4: [0x9D, 0x55, 0x7B, 0x8E, 0x7F, 0x15, 0x70, 0x91],
};

#[repr(C)]
struct FileTime {
    low: u32,
    high: u32,
}

#[repr(C)]
struct ByHandleFileInformation {
    file_attributes: u32,
    creation_time: FileTime,
    last_access_time: FileTime,
    last_write_time: FileTime,
    volume_serial_number: u32,
    file_size_high: u32,
    file_size_low: u32,
    number_of_links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

#[repr(C)]
struct SidAndAttributes {
    sid: *mut c_void,
    attributes: u32,
}

#[repr(C)]
struct TokenUser {
    user: SidAndAttributes,
}

#[link(name = "shell32")]
extern "system" {
    fn SHGetKnownFolderPath(
        folder_id: *const Guid,
        flags: u32,
        token: Handle,
        path: *mut *mut u16,
    ) -> i32;
}

#[link(name = "ole32")]
extern "system" {
    fn CoTaskMemFree(memory: *mut c_void);
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(
        file_name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *mut c_void,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: Handle,
    ) -> Handle;
    fn GetFileInformationByHandle(file: Handle, information: *mut ByHandleFileInformation) -> i32;
    fn GetCurrentProcess() -> Handle;
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(process: Handle, desired_access: u32, token: *mut Handle) -> i32;
    fn GetTokenInformation(
        token: Handle,
        information_class: u32,
        information: *mut c_void,
        information_length: u32,
        return_length: *mut u32,
    ) -> i32;
    fn GetLengthSid(sid: *mut c_void) -> u32;
    fn CopySid(destination_length: u32, destination: *mut c_void, source: *mut c_void) -> i32;
    fn EqualSid(first: *mut c_void, second: *mut c_void) -> i32;
    fn GetSecurityInfo(
        handle: Handle,
        object_type: u32,
        security_information: u32,
        owner: *mut *mut c_void,
        group: *mut *mut c_void,
        dacl: *mut *mut c_void,
        sacl: *mut *mut c_void,
        security_descriptor: *mut *mut c_void,
    ) -> u32;
    fn SetSecurityInfo(
        handle: Handle,
        object_type: u32,
        security_information: u32,
        owner: *mut c_void,
        group: *mut c_void,
        dacl: *mut c_void,
        sacl: *mut c_void,
    ) -> u32;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        descriptor: *const u16,
        revision: u32,
        security_descriptor: *mut *mut c_void,
        size: *mut u32,
    ) -> i32;
    fn GetSecurityDescriptorDacl(
        security_descriptor: *mut c_void,
        present: *mut i32,
        dacl: *mut *mut c_void,
        defaulted: *mut i32,
    ) -> i32;
    fn GetSecurityDescriptorControl(
        security_descriptor: *mut c_void,
        control: *mut u16,
        revision: *mut u32,
    ) -> i32;
}

struct SecurityDescriptor(*mut c_void);

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        unsafe { LocalFree(self.0) };
    }
}

struct PrivateAcl {
    _descriptor: SecurityDescriptor,
    dacl: *mut c_void,
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn local_app_data() -> Result<PathBuf, String> {
    unsafe {
        let mut raw = ptr::null_mut();
        let result = SHGetKnownFolderPath(&FOLDERID_LOCAL_APP_DATA, 0, ptr::null_mut(), &mut raw);
        if result < 0 || raw.is_null() {
            return Err(format!(
                "resolve LocalAppData Known Folder failed: HRESULT 0x{:08x}",
                result as u32
            ));
        }
        let mut len = 0usize;
        while *raw.add(len) != 0 {
            len = len
                .checked_add(1)
                .ok_or("LocalAppData path length overflow")?;
            if len > 32_767 {
                CoTaskMemFree(raw.cast());
                return Err("LocalAppData Known Folder path exceeds Windows limit".into());
            }
        }
        let path = PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(raw, len)));
        CoTaskMemFree(raw.cast());
        if path.as_os_str().is_empty() || !path.is_absolute() {
            return Err("LocalAppData Known Folder returned a non-absolute path".into());
        }
        Ok(path)
    }
}

fn open_no_follow(path: &Path, manage_security: bool) -> Result<(File, bool), String> {
    let access = FILE_READ_ATTRIBUTES
        | if manage_security {
            READ_CONTROL | WRITE_DAC
        } else {
            0
        };
    let handle = unsafe {
        CreateFileW(
            wide(path).as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(format!(
            "open replay storage component {} no-follow: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    let file = unsafe { File::from_raw_handle(handle) };
    let mut information: ByHandleFileInformation = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(handle, &mut information) } == 0 {
        return Err(format!(
            "inspect replay storage component {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    if information.file_attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(format!(
            "replay storage rejects reparse point {}",
            path.display()
        ));
    }
    Ok((
        file,
        information.file_attributes & FILE_ATTRIBUTE_DIRECTORY != 0,
    ))
}

fn current_user_sid() -> Result<Vec<u8>, String> {
    let mut token = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(format!(
            "open current-user token: {}",
            std::io::Error::last_os_error()
        ));
    }
    let token = unsafe { File::from_raw_handle(token) };
    let mut needed = 0u32;
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TOKEN_USER_CLASS,
            ptr::null_mut(),
            0,
            &mut needed,
        );
    }
    if needed == 0 {
        return Err("current-user token returned no SID length".into());
    }
    let mut token_user = vec![0u8; needed as usize];
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TOKEN_USER_CLASS,
            token_user.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(format!(
            "read current-user SID: {}",
            std::io::Error::last_os_error()
        ));
    }
    let source = unsafe { (*(token_user.as_ptr() as *const TokenUser)).user.sid };
    let sid_len = unsafe { GetLengthSid(source) };
    if sid_len == 0 {
        return Err("current-user SID has zero length".into());
    }
    let mut sid = vec![0u8; sid_len as usize];
    if unsafe { CopySid(sid_len, sid.as_mut_ptr().cast(), source) } == 0 {
        return Err(format!(
            "copy current-user SID: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(sid)
}

impl PrivateAcl {
    fn new() -> Result<Self, String> {
        let mut sddl: Vec<u16> = "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut descriptor = ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_mut_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(format!(
                "create replay private ACL: {}",
                std::io::Error::last_os_error()
            ));
        }
        let descriptor_owner = SecurityDescriptor(descriptor);
        let mut present = 0;
        let mut defaulted = 0;
        let mut dacl = ptr::null_mut();
        if unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) }
            == 0
            || present == 0
            || dacl.is_null()
        {
            return Err("replay private descriptor has no DACL".into());
        }
        Ok(Self {
            _descriptor: descriptor_owner,
            dacl,
        })
    }

    fn apply_and_audit(&self, path: &Path, file: &File, current_sid: &[u8]) -> Result<(), String> {
        let handle = file.as_raw_handle();
        assert_current_owner_and_control(path, handle, current_sid, false)?;
        let result = unsafe {
            SetSecurityInfo(
                handle,
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                self.dacl,
                ptr::null_mut(),
            )
        };
        if result != 0 {
            return Err(format!(
                "apply protected replay ACL to {}: Windows error {result}",
                path.display()
            ));
        }
        assert_current_owner_and_control(path, handle, current_sid, true)
    }
}

fn assert_current_owner_and_control(
    path: &Path,
    handle: Handle,
    current_sid: &[u8],
    require_protected: bool,
) -> Result<(), String> {
    let mut owner = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let result = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if result != 0 || descriptor.is_null() || owner.is_null() {
        if !descriptor.is_null() {
            unsafe { LocalFree(descriptor) };
        }
        return Err(format!(
            "read replay storage security for {}: Windows error {result}",
            path.display()
        ));
    }
    let descriptor_guard = SecurityDescriptor(descriptor);
    if unsafe { EqualSid(owner, current_sid.as_ptr() as *mut c_void) } == 0 {
        return Err(format!(
            "replay storage refuses foreign-owned entry {}",
            path.display()
        ));
    }
    if require_protected {
        let mut control = 0u16;
        let mut revision = 0u32;
        if unsafe { GetSecurityDescriptorControl(descriptor_guard.0, &mut control, &mut revision) }
            == 0
            || control & SE_DACL_PROTECTED == 0
        {
            return Err(format!(
                "replay storage DACL is not protected: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn audit_tree(
    path: &Path,
    sid: &[u8],
    acl: &PrivateAcl,
    require_directory: bool,
) -> Result<(), String> {
    let (file, is_directory) = open_no_follow(path, true)?;
    if require_directory && !is_directory {
        return Err(format!(
            "replay storage root is not a directory: {}",
            path.display()
        ));
    }
    acl.apply_and_audit(path, &file, sid)?;
    if is_directory {
        for entry in fs::read_dir(path)
            .map_err(|error| format!("enumerate replay storage {}: {error}", path.display()))?
        {
            let child = entry
                .map_err(|error| format!("enumerate replay storage {}: {error}", path.display()))?
                .path();
            audit_tree(&child, sid, acl, false)?;
        }
    }
    Ok(())
}

fn create_and_retain(path: &Path) -> Result<Vec<File>, String> {
    if !path.is_absolute() {
        return Err("replay storage root must be absolute".into());
    }
    let mut guards = Vec::new();
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(component, Component::Prefix(_) | Component::RootDir) {
            continue;
        }
        match fs::create_dir(&current) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(format!(
                    "create replay storage {}: {error}",
                    current.display()
                ))
            }
        }
        let (guard, is_directory) = open_no_follow(&current, false)?;
        if !is_directory {
            return Err(format!(
                "replay storage ancestor is not a directory: {}",
                current.display()
            ));
        }
        guards.push(guard);
    }
    Ok(guards)
}

pub(super) fn prepare_product_spool() -> Result<(PathBuf, Vec<File>), String> {
    let spool = local_app_data()?.join("battle-plan").join("replay_spool");
    let guards = create_and_retain(&spool)?;
    let sid = current_user_sid()?;
    let acl = PrivateAcl::new()?;
    audit_tree(&spool, &sid, &acl, true)?;
    Ok((spool, guards))
}
