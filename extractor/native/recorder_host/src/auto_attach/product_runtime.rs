//! stop request reaches the coordinator's stop ownership instead of bypassing

use std::fs;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
#[cfg(windows)]
use std::sync::{Arc, Mutex};

pub const PACKAGED_COLLECTOR_DLL_NAME: &str = "collector.dll";
pub const PRODUCT_DATA_DIRECTORY_NAME: &str = "Battle Plan Recorder";

pub fn resolve_packaged_collector_dll(
    explicit: Option<PathBuf>,
    current_exe: &Path,
) -> Result<PathBuf, String> {
    if let Some(explicit) = explicit {
        return validate_payload_file(&explicit, None, "explicit collector DLL override");
    }

    let executable = fs::canonicalize(current_exe)
        .map_err(|error| format!("canonicalize installed recorder executable failed: {error}"))?;
    let install_dir = executable
        .parent()
        .ok_or("installed recorder executable has no parent directory")?;
    let candidate = install_dir.join(PACKAGED_COLLECTOR_DLL_NAME);
    validate_payload_file(&candidate, Some(install_dir), "packaged collector DLL")
}

fn validate_payload_file(
    candidate: &Path,
    required_parent: Option<&Path>,
    label: &str,
) -> Result<PathBuf, String> {
    let link_metadata = fs::symlink_metadata(candidate)
        .map_err(|error| format!("stat {label} {} failed: {error}", candidate.display()))?;
    if link_metadata.file_type().is_symlink() || !link_metadata.is_file() {
        return Err(format!(
            "{label} {} is not a regular non-symlink file",
            candidate.display()
        ));
    }
    if link_metadata.len() == 0 {
        return Err(format!("{label} {} is empty", candidate.display()));
    }
    let canonical = fs::canonicalize(candidate).map_err(|error| {
        format!(
            "canonicalize {label} {} failed: {error}",
            candidate.display()
        )
    })?;
    if let Some(required_parent) = required_parent {
        if canonical.parent() != Some(required_parent) {
            return Err(format!(
                "{label} resolved outside the installed recorder directory"
            ));
        }
    }
    Ok(canonical)
}

#[cfg(windows)]
pub fn default_product_data_root() -> Result<PathBuf, String> {
    use std::ffi::{c_void, OsString};
    use std::os::windows::ffi::OsStringExt;
    use std::ptr;

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

    #[link(name = "shell32")]
    extern "system" {
        fn SHGetKnownFolderPath(
            folder_id: *const Guid,
            flags: u32,
            token: *mut c_void,
            path: *mut *mut u16,
        ) -> i32;
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoTaskMemFree(memory: *mut c_void);
    }

    unsafe {
        let mut raw: *mut u16 = ptr::null_mut();
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
                .ok_or("LocalAppData Known Folder path length overflow")?;
            if len > 32_767 {
                CoTaskMemFree(raw.cast());
                return Err("LocalAppData Known Folder path exceeds Windows limit".into());
            }
        }
        let root = PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(raw, len)));
        CoTaskMemFree(raw.cast());
        if root.as_os_str().is_empty() || !root.is_absolute() {
            return Err("LocalAppData Known Folder returned a non-absolute path".into());
        }
        let product_root = root.join(PRODUCT_DATA_DIRECTORY_NAME);
        ensure_private_product_data_root(&product_root)?;
        Ok(product_root)
    }
}

#[cfg(windows)]
fn ensure_private_product_data_root(path: &Path) -> Result<(), String> {
    private_storage::prepare(path)
}

#[cfg(windows)]
mod private_storage {
    use super::*;
    use std::ffi::c_void;
    use std::fs::File;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::path::Component;
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
    const TOKEN_USER_CLASS: i32 = 1;
    const OWNER_SECURITY_INFORMATION: u32 = 0x0000_0001;
    const DACL_SECURITY_INFORMATION: u32 = 0x0000_0004;
    const PROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x8000_0000;
    const SE_FILE_OBJECT: u32 = 1;
    const SDDL_REVISION_1: u32 = 1;

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
        fn GetFileInformationByHandle(
            file: Handle,
            information: *mut ByHandleFileInformation,
        ) -> i32;
        fn GetCurrentProcess() -> Handle;
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn OpenProcessToken(process: Handle, desired_access: u32, token: *mut Handle) -> i32;
        fn GetTokenInformation(
            token: Handle,
            information_class: i32,
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
    }

    struct SecurityDescriptor(*mut c_void);

    impl Drop for SecurityDescriptor {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.0);
            }
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
                "open product storage component {} without reparse traversal failed: {}",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
        let file = unsafe { File::from_raw_handle(handle) };
        let mut information: ByHandleFileInformation = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(handle, &mut information) } == 0 {
            return Err(format!(
                "inspect product storage component {} failed: {}",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
        if information.file_attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(format!(
                "product storage rejects reparse point {}",
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
                "open current user token for product storage failed: {}",
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
            return Err("current user token returned no SID length".into());
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
                "read current user SID failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        let source = unsafe { (*(token_user.as_ptr() as *const TokenUser)).user.sid };
        let sid_len = unsafe { GetLengthSid(source) };
        if sid_len == 0 {
            return Err("current user SID has zero length".into());
        }
        let mut sid = vec![0u8; sid_len as usize];
        if unsafe { CopySid(sid_len, sid.as_mut_ptr().cast(), source) } == 0 {
            return Err(format!(
                "copy current user SID failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(sid)
    }

    impl PrivateAcl {
        fn new() -> Result<Self, String> {
            let mut sddl: Vec<u16> = "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)"
                .encode_utf16()
                .collect();
            sddl.push(0);
            let mut descriptor = ptr::null_mut();
            if unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    ptr::null_mut(),
                )
            } == 0
            {
                return Err(format!(
                    "create private product data ACL failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let descriptor_owner = SecurityDescriptor(descriptor);
            let mut present = 0;
            let mut defaulted = 0;
            let mut dacl = ptr::null_mut();
            if unsafe {
                GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted)
            } == 0
                || present == 0
                || dacl.is_null()
            {
                return Err("private product data descriptor has no DACL".into());
            }
            Ok(Self {
                _descriptor: descriptor_owner,
                dacl,
            })
        }

        fn apply_owned(&self, path: &Path, file: &File, current_sid: &[u8]) -> Result<(), String> {
            let handle = file.as_raw_handle();
            let mut owner = ptr::null_mut();
            let mut descriptor = ptr::null_mut();
            let result = unsafe {
                GetSecurityInfo(
                    handle,
                    SE_FILE_OBJECT,
                    OWNER_SECURITY_INFORMATION,
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
                    "read product storage owner for {} failed: Windows error {result}",
                    path.display()
                ));
            }
            let same_owner = unsafe { EqualSid(owner, current_sid.as_ptr() as *mut c_void) } != 0;
            unsafe { LocalFree(descriptor) };
            if !same_owner {
                return Err(format!(
                    "product storage refuses foreign-owned entry {}",
                    path.display()
                ));
            }
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
                    "apply private ACL to {} failed: Windows error {result}",
                    path.display()
                ));
            }
            Ok(())
        }
    }

    fn audit_tree(
        path: &Path,
        current_sid: &[u8],
        acl: &PrivateAcl,
        require_directory: bool,
    ) -> Result<(), String> {
        let (file, is_directory) = open_no_follow(path, true)?;
        if require_directory && !is_directory {
            return Err(format!(
                "product storage entry has unexpected type {}",
                path.display()
            ));
        }
        acl.apply_owned(path, &file, current_sid)?;
        if !is_directory {
            return Ok(());
        }
        let entries = fs::read_dir(path).map_err(|error| {
            format!(
                "enumerate product storage {} failed: {error}",
                path.display()
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!(
                    "enumerate product storage {} failed: {error}",
                    path.display()
                )
            })?;
            let child = entry.path();
            audit_tree(&child, current_sid, acl, false)?;
        }
        Ok(())
    }

    pub fn prepare(path: &Path) -> Result<(), String> {
        if !path.is_absolute() {
            return Err("product data root must be absolute".into());
        }
        let mut guards = Vec::new();
        let mut current = PathBuf::new();
        for component in path.components() {
            current.push(component.as_os_str());
            if matches!(component, Component::Prefix(_)) {
                continue;
            }
            let exists = match fs::symlink_metadata(&current) {
                Ok(_) => true,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => {
                    return Err(format!(
                        "inspect product storage ancestor {} failed: {error}",
                        current.display()
                    ))
                }
            };
            if !exists {
                fs::create_dir(&current).map_err(|error| {
                    format!(
                        "create product storage component {} failed: {error}",
                        current.display()
                    )
                })?;
            }
            let (guard, is_directory) = open_no_follow(&current, false)?;
            if !is_directory {
                return Err(format!(
                    "product storage ancestor is not a directory: {}",
                    current.display()
                ));
            }
            guards.push(guard);
        }
        let sid = current_user_sid()?;
        let acl = PrivateAcl::new()?;
        audit_tree(path, &sid, &acl, true)?;
        drop(guards);
        Ok(())
    }
}

/// pressing the explicit stop button transfers ownership to the coordinator,
#[cfg(windows)]
pub fn run_product_window<F>(out_root: PathBuf, supervisor: F) -> Result<(), String>
where
    F: FnOnce(
            Arc<AtomicBool>,
            Arc<dyn Fn(super::SupervisorEvent) + Send + Sync>,
        ) -> Result<(), String>
        + Send
        + 'static,
{
    desktop::run(out_root, supervisor)
}

#[cfg(windows)]
pub fn show_product_error(message: &str) {
    desktop::show_error(message);
}

#[cfg(windows)]
mod desktop {
    use super::*;
    use std::ffi::c_void;
    use std::mem;
    use std::ptr;

    type Handle = *mut c_void;
    type Hwnd = Handle;
    type Hinstance = Handle;
    type Hicon = Handle;
    type Hcursor = Handle;
    type Hbrush = Handle;
    type Wparam = usize;
    type Lparam = isize;
    type Lresult = isize;

    const WM_CLOSE: u32 = 0x0010;
    const WM_DESTROY: u32 = 0x0002;
    const WM_COMMAND: u32 = 0x0111;
    const WM_TIMER: u32 = 0x0113;
    const WM_QUERYENDSESSION: u32 = 0x0011;
    const WM_ENDSESSION: u32 = 0x0016;
    const WS_OVERLAPPED: u32 = 0x0000_0000;
    const WS_CAPTION: u32 = 0x00C0_0000;
    const WS_SYSMENU: u32 = 0x0008_0000;
    const WS_MINIMIZEBOX: u32 = 0x0002_0000;
    const WS_VISIBLE: u32 = 0x1000_0000;
    const WS_CHILD: u32 = 0x4000_0000;
    const WS_VSCROLL: u32 = 0x0020_0000;
    const WS_EX_CLIENTEDGE: u32 = 0x0000_0200;
    const BS_PUSHBUTTON: u32 = 0;
    const ES_MULTILINE: u32 = 0x0004;
    const ES_AUTOVSCROLL: u32 = 0x0040;
    const ES_READONLY: u32 = 0x0800;
    const EM_SETSEL: u32 = 0x00B1;
    const EM_SCROLLCARET: u32 = 0x00B7;
    const CW_USEDEFAULT: i32 = i32::MIN;
    const SW_SHOW: i32 = 5;
    const SW_HIDE: i32 = 0;
    const IDC_ARROW: usize = 32_512;
    const COLOR_WINDOW: usize = 5;
    const BUTTON_STOP: usize = 100;
    const BUTTON_OPEN_FOLDER: usize = 101;
    const STATUS_TIMER: usize = 1;

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub(super) struct ProductPresentation {
        pub(super) status_text: String,
        pub(super) open_target: PathBuf,
        recordings_root: PathBuf,
        terminal_result_seen: bool,
        entries: Vec<String>,
        unsupported_game_build: bool,
    }

    impl ProductPresentation {
        pub(super) fn watching(out_root: PathBuf) -> Self {
            let initial = format!(
                "Watching for the game. Recording starts automatically when a supported game is ready.\r\nRecordings folder: {}",
                out_root.display()
            );
            Self {
                status_text: initial.clone(),
                open_target: out_root.clone(),
                recordings_root: out_root,
                terminal_result_seen: false,
                entries: vec![initial],
                unsupported_game_build: false,
            }
        }

        fn append(&mut self, entry: String) {
            const MAX_ENTRIES: usize = 64;
            const MAX_ENTRY_CHARS: usize = 2_000;
            const MAX_STATUS_UTF16_UNITS: usize = 24_000;

            let mut bounded: String = entry.chars().take(MAX_ENTRY_CHARS).collect();
            if entry.chars().count() > MAX_ENTRY_CHARS {
                bounded.push_str("...");
            }
            self.entries.push(bounded);
            while self.entries.len() > MAX_ENTRIES
                || (self.entries.len() > 1
                    && self
                        .entries
                        .iter()
                        .map(|item| item.encode_utf16().count())
                        .sum::<usize>()
                        > MAX_STATUS_UTF16_UNITS)
            {
                self.entries.remove(0);
            }
            self.status_text = self.entries.join("\r\n\r\n");
        }

        pub(super) fn apply(&mut self, event: super::super::SupervisorEvent) {
            match event {
                super::super::SupervisorEvent::RecordingStarted { pid, run_dir } => {
                    self.append(format!(
                        "Recording started for the game (PID {pid}).\r\nOutput folder: {}",
                        run_dir.display()
                    ));
                    self.open_target = run_dir;
                    self.terminal_result_seen = false;
                }
                super::super::SupervisorEvent::RecordingSucceeded { pid, replay_path } => {
                    self.append(format!(
                        "Recording finished and verified for the game (PID {pid}).\r\nReplay: {}\r\nThe recorder is still watching for another game.",
                        replay_path.display()
                    ));
                    self.open_target = replay_path
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or(replay_path);
                    self.terminal_result_seen = true;
                }
                super::super::SupervisorEvent::RecordingFailed {
                    pid,
                    reason,
                    detail,
                    run_dir,
                } => {
                    if reason == crate::auto_attach::state::ReasonCode::UnsupportedGameBuild {
                        let message = detail
                            .unwrap_or_else(|| reason.as_str().to_string())
                            .replace("\r\n", "\n")
                            .replace('\n', "\r\n");
                        self.entries = vec![message.clone()];
                        self.status_text = message;
                        self.unsupported_game_build = true;
                        crate::game_build::mark_refusal_presented();
                        self.open_target = self.recordings_root.clone();
                        self.terminal_result_seen = true;
                        return;
                    }
                    let detail = detail.unwrap_or_else(|| reason.as_str().to_string());
                    let run_dir_is_directory = run_dir.as_ref().is_some_and(|path| path.is_dir());
                    let (open_target, visible_run_dir) =
                        select_failure_folder(&self.recordings_root, run_dir, run_dir_is_directory);
                    let location = visible_run_dir
                        .as_ref()
                        .map(|path| format!("\r\nAttempt folder: {}", path.display()))
                        .unwrap_or_else(|| {
                            format!("\r\nRecordings folder: {}", self.recordings_root.display())
                        });
                    self.append(format!(
                        "Recording failed for the game (PID {pid}): {detail}{location}\r\nThe recorder is still watching for another game."
                    ));
                    self.open_target = open_target;
                    self.terminal_result_seen = true;
                }
            }
        }

        pub(super) fn stopping(&mut self) {
            self.append("Stopping safely; finishing and verifying recordings...".to_string());
        }

        pub(super) fn finished(&mut self, error: Option<&str>) {
            if self.unsupported_game_build {
                self.entries
                    .push("The recorder has stopped. Select Close when ready.".to_string());
                self.status_text = self.entries.join("\r\n\r\n");
                self.terminal_result_seen = true;
                return;
            }
            if let Some(error) = error {
                self.append(format!(
                    "Recorder stopped because of an error: {error}\r\nReview the recordings folder, then select Close."
                ));
                self.terminal_result_seen = true;
            } else if self.terminal_result_seen {
                self.append("Recorder stopped safely. Select Close when ready.".to_string());
            } else {
                self.append(format!(
                    "Recorder stopped safely. No active recording needed finalization.\r\nRecordings folder: {}\r\nSelect Close when ready.",
                    self.recordings_root.display()
                ));
                self.open_target = self.recordings_root.clone();
            }
        }
    }

    pub(super) fn select_failure_folder(
        recordings_root: &Path,
        candidate: Option<PathBuf>,
        candidate_is_directory: bool,
    ) -> (PathBuf, Option<PathBuf>) {
        match candidate {
            Some(candidate) if candidate_is_directory => (candidate.clone(), Some(candidate)),
            _ => (recordings_root.to_path_buf(), None),
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) enum StopButtonAction {
        RequestStop,
        Close,
    }

    pub(super) fn stop_button_action(supervisor_done: bool) -> StopButtonAction {
        if supervisor_done {
            StopButtonAction::Close
        } else {
            StopButtonAction::RequestStop
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) enum CompletionDisposition {
        WaitForSupervisor,
        PresentCloseAction,
        AutoClose,
    }

    pub(super) fn completion_disposition(
        supervisor_done: bool,
        close_when_done: bool,
    ) -> CompletionDisposition {
        match (supervisor_done, close_when_done) {
            (false, _) => CompletionDisposition::WaitForSupervisor,
            (true, false) => CompletionDisposition::PresentCloseAction,
            (true, true) => CompletionDisposition::AutoClose,
        }
    }

    #[repr(C)]
    struct WndClassW {
        style: u32,
        wnd_proc: Option<unsafe extern "system" fn(Hwnd, u32, Wparam, Lparam) -> Lresult>,
        cls_extra: i32,
        wnd_extra: i32,
        instance: Hinstance,
        icon: Hicon,
        cursor: Hcursor,
        background: Hbrush,
        menu_name: *const u16,
        class_name: *const u16,
    }

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct Msg {
        hwnd: Hwnd,
        message: u32,
        wparam: Wparam,
        lparam: Lparam,
        time: u32,
        point: Point,
        private: u32,
    }

    struct UiState {
        stop: Arc<AtomicBool>,
        done: AtomicBool,
        error: Mutex<Option<String>>,
        presentation: Mutex<ProductPresentation>,
        rendered_status: Mutex<String>,
        status: AtomicPtr<c_void>,
        stop_button: AtomicPtr<c_void>,
        open_button: AtomicPtr<c_void>,
        completion_ui_applied: AtomicBool,
        close_when_done: AtomicBool,
    }

    static UI_STATE: AtomicPtr<UiState> = AtomicPtr::new(ptr::null_mut());

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(module_name: *const u16) -> Hinstance;
        fn GetConsoleWindow() -> Hwnd;
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassW(class: *const WndClassW) -> u16;
        fn CreateWindowExW(
            ex_style: u32,
            class_name: *const u16,
            window_name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            parent: Hwnd,
            menu: Handle,
            instance: Hinstance,
            param: *mut c_void,
        ) -> Hwnd;
        fn DefWindowProcW(hwnd: Hwnd, message: u32, wparam: Wparam, lparam: Lparam) -> Lresult;
        fn DestroyWindow(hwnd: Hwnd) -> i32;
        fn DispatchMessageW(message: *const Msg) -> Lresult;
        fn EnableWindow(hwnd: Hwnd, enable: i32) -> i32;
        fn GetMessageW(message: *mut Msg, hwnd: Hwnd, min: u32, max: u32) -> i32;
        fn LoadCursorW(instance: Hinstance, cursor_name: *const u16) -> Hcursor;
        fn MessageBoxW(hwnd: Hwnd, text: *const u16, caption: *const u16, kind: u32) -> i32;
        fn PostQuitMessage(exit_code: i32);
        fn SendMessageW(hwnd: Hwnd, message: u32, wparam: Wparam, lparam: Lparam) -> Lresult;
        fn SetTimer(hwnd: Hwnd, id: usize, interval_ms: u32, callback: *mut c_void) -> usize;
        fn SetWindowTextW(hwnd: Hwnd, text: *const u16) -> i32;
        fn ShowWindow(hwnd: Hwnd, command: i32) -> i32;
        fn TranslateMessage(message: *const Msg) -> i32;
        fn UpdateWindow(hwnd: Hwnd) -> i32;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            hwnd: Hwnd,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show_command: i32,
        ) -> Handle;
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe fn state() -> Option<&'static UiState> {
        UI_STATE.load(Ordering::Acquire).as_ref()
    }

    unsafe fn stop_or_close(hwnd: Hwnd, close_when_done: bool) {
        if let Some(state) = state() {
            if close_when_done {
                state.close_when_done.store(true, Ordering::Release);
            }
            match stop_button_action(state.done.load(Ordering::Acquire)) {
                StopButtonAction::Close => {
                    DestroyWindow(hwnd);
                    return;
                }
                StopButtonAction::RequestStop => {}
            }
            if state.stop.swap(true, Ordering::AcqRel) {
                return;
            }
            if let Ok(mut presentation) = state.presentation.lock() {
                presentation.stopping();
            }
            let status = state.status.load(Ordering::Acquire);
            if !status.is_null() {
                SetWindowTextW(
                    status,
                    wide("Stopping safely; finishing and verifying recordings...").as_ptr(),
                );
            }
            let button = state.stop_button.load(Ordering::Acquire);
            if !button.is_null() {
                EnableWindow(button, 0);
            }
        }
    }

    unsafe fn open_recordings_folder(hwnd: Hwnd) {
        let Some(state) = state() else {
            return;
        };
        let target = match state.presentation.lock() {
            Ok(presentation) => presentation.open_target.clone(),
            Err(_) => {
                show_error("The recordings folder could not be resolved.");
                return;
            }
        };
        let opened = ShellExecuteW(
            hwnd,
            wide("open").as_ptr(),
            wide(&target.display().to_string()).as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOW,
        ) as isize;
        if opened <= 32 {
            show_error(&format!(
                "Could not open the recordings folder:\n{}",
                target.display()
            ));
        }
    }

    unsafe extern "system" fn window_proc(
        hwnd: Hwnd,
        message: u32,
        wparam: Wparam,
        lparam: Lparam,
    ) -> Lresult {
        match message {
            WM_COMMAND if wparam & 0xffff == BUTTON_STOP => {
                stop_or_close(hwnd, false);
                0
            }
            WM_COMMAND if wparam & 0xffff == BUTTON_OPEN_FOLDER => {
                open_recordings_folder(hwnd);
                0
            }
            WM_CLOSE => {
                stop_or_close(hwnd, false);
                0
            }
            WM_QUERYENDSESSION => {
                stop_or_close(hwnd, true);
                1
            }
            WM_ENDSESSION => {
                if wparam != 0 {
                    stop_or_close(hwnd, true);
                }
                0
            }
            WM_TIMER if wparam == STATUS_TIMER => {
                if let Some(state) = state() {
                    let status = state.status.load(Ordering::Acquire);
                    if !status.is_null() {
                        if let Ok(presentation) = state.presentation.lock() {
                            if let Ok(mut rendered) = state.rendered_status.lock() {
                                if rendered.as_str() != presentation.status_text.as_str() {
                                    SetWindowTextW(
                                        status,
                                        wide(&presentation.status_text).as_ptr(),
                                    );
                                    SendMessageW(status, EM_SETSEL, usize::MAX, -1);
                                    SendMessageW(status, EM_SCROLLCARET, 0, 0);
                                    rendered.clone_from(&presentation.status_text);
                                }
                            }
                        }
                    }
                    match completion_disposition(
                        state.done.load(Ordering::Acquire),
                        state.close_when_done.load(Ordering::Acquire),
                    ) {
                        CompletionDisposition::AutoClose => {
                            DestroyWindow(hwnd);
                        }
                        CompletionDisposition::PresentCloseAction
                            if !state.completion_ui_applied.swap(true, Ordering::AcqRel) =>
                        {
                            let button = state.stop_button.load(Ordering::Acquire);
                            if !button.is_null() {
                                SetWindowTextW(button, wide("Close").as_ptr());
                                EnableWindow(button, 1);
                            }
                        }
                        _ => {}
                    }
                }
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    pub fn run<F>(out_root: PathBuf, supervisor: F) -> Result<(), String>
    where
        F: FnOnce(
                Arc<AtomicBool>,
                Arc<dyn Fn(super::super::SupervisorEvent) + Send + Sync>,
            ) -> Result<(), String>
            + Send
            + 'static,
    {
        let presentation = ProductPresentation::watching(out_root);
        let rendered_status = presentation.status_text.clone();
        let state = Box::new(UiState {
            stop: Arc::new(AtomicBool::new(false)),
            done: AtomicBool::new(false),
            error: Mutex::new(None),
            presentation: Mutex::new(presentation),
            rendered_status: Mutex::new(rendered_status),
            status: AtomicPtr::new(ptr::null_mut()),
            stop_button: AtomicPtr::new(ptr::null_mut()),
            open_button: AtomicPtr::new(ptr::null_mut()),
            completion_ui_applied: AtomicBool::new(false),
            close_when_done: AtomicBool::new(false),
        });
        let state_ptr = Box::into_raw(state);
        if UI_STATE
            .compare_exchange(
                ptr::null_mut(),
                state_ptr,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            unsafe { drop(Box::from_raw(state_ptr)) };
            return Err("product recorder window is already running".into());
        }
        let state_ref = unsafe { &*state_ptr };
        let stop = Arc::clone(&state_ref.stop);
        let state_address = state_ptr as usize;
        let event_state_address = state_address;
        let report_event: Arc<dyn Fn(super::super::SupervisorEvent) + Send + Sync> =
            Arc::new(move |event| {
                let state = unsafe { &*(event_state_address as *mut UiState) };
                if let Ok(mut presentation) = state.presentation.lock() {
                    presentation.apply(event);
                }
            });
        let thread = match std::thread::Builder::new()
            .name("auto-attach-supervisor".to_string())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    supervisor(stop, report_event)
                }));
                let state_ptr = state_address as *mut UiState;
                let state = unsafe { &*state_ptr };
                let error = match result {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(error),
                    Err(payload) => Some(match payload.downcast_ref::<&str>() {
                        Some(message) => format!("auto-attach supervisor panicked: {message}"),
                        None => match payload.downcast_ref::<String>() {
                            Some(message) => format!("auto-attach supervisor panicked: {message}"),
                            None => "auto-attach supervisor panicked with a non-text payload"
                                .to_string(),
                        },
                    }),
                };
                if let Ok(mut presentation) = state.presentation.lock() {
                    presentation.finished(error.as_deref());
                }
                if let Some(error) = error {
                    if let Ok(mut stored) = state.error.lock() {
                        *stored = Some(error);
                    }
                }
                state.done.store(true, Ordering::Release);
            }) {
            Ok(thread) => thread,
            Err(error) => {
                UI_STATE.store(ptr::null_mut(), Ordering::Release);
                unsafe { drop(Box::from_raw(state_ptr)) };
                return Err(format!("start auto-attach supervisor failed: {error}"));
            }
        };

        let result = unsafe { run_message_loop(state_ref) };
        state_ref.stop.store(true, Ordering::Release);
        let join_result = thread
            .join()
            .map_err(|_| "auto-attach supervisor thread panicked".to_string());
        let supervisor_error = state_ref.error.lock().ok().and_then(|error| error.clone());
        UI_STATE.store(ptr::null_mut(), Ordering::Release);
        unsafe { drop(Box::from_raw(state_ptr)) };
        result.and(join_result)?;
        match supervisor_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    unsafe fn run_message_loop(state: &UiState) -> Result<(), String> {
        let console = GetConsoleWindow();
        if !console.is_null() {
            ShowWindow(console, SW_HIDE);
        }
        let instance = GetModuleHandleW(ptr::null());
        let class_name = wide("Hoi4ReplayRecorderWindow");
        let class = WndClassW {
            style: 0,
            wnd_proc: Some(window_proc),
            cls_extra: 0,
            wnd_extra: 0,
            instance,
            icon: ptr::null_mut(),
            cursor: LoadCursorW(ptr::null_mut(), IDC_ARROW as *const u16),
            background: (COLOR_WINDOW + 1) as Hbrush,
            menu_name: ptr::null(),
            class_name: class_name.as_ptr(),
        };
        if RegisterClassW(&class) == 0 {
            return Err(format!(
                "register recorder window failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide("Battle Plan Recorder").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            700,
            300,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null_mut(),
        );
        if hwnd.is_null() {
            return Err(format!(
                "create recorder window failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        let status = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            wide("EDIT").as_ptr(),
            wide(
                &state
                    .presentation
                    .lock()
                    .map_err(|_| "recorder presentation lock was poisoned".to_string())?
                    .status_text,
            )
            .as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_VSCROLL | ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY,
            24,
            28,
            640,
            125,
            hwnd,
            ptr::null_mut(),
            instance,
            ptr::null_mut(),
        );
        let open_button = CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide("Open recordings folder").as_ptr(),
            WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
            60,
            180,
            250,
            34,
            hwnd,
            BUTTON_OPEN_FOLDER as Handle,
            instance,
            ptr::null_mut(),
        );
        let stop_button = CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide("Stop and finish recordings").as_ptr(),
            WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
            370,
            180,
            260,
            34,
            hwnd,
            BUTTON_STOP as Handle,
            instance,
            ptr::null_mut(),
        );
        if status.is_null()
            || open_button.is_null()
            || stop_button.is_null()
            || SetTimer(hwnd, STATUS_TIMER, 100, ptr::null_mut()) == 0
        {
            DestroyWindow(hwnd);
            return Err(format!(
                "initialize recorder window controls failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        state.status.store(status, Ordering::Release);
        state.stop_button.store(stop_button, Ordering::Release);
        state.open_button.store(open_button, Ordering::Release);
        SendMessageW(status, EM_SETSEL, usize::MAX, -1);
        SendMessageW(status, EM_SCROLLCARET, 0, 0);
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);

        let mut message: Msg = mem::zeroed();
        loop {
            let read = GetMessageW(&mut message, ptr::null_mut(), 0, 0);
            if read == -1 {
                return Err(format!(
                    "recorder window message loop failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            if read == 0 {
                break;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        Ok(())
    }

    pub fn show_error(message: &str) {
        unsafe {
            MessageBoxW(
                ptr::null_mut(),
                wide(message).as_ptr(),
                wide("Battle Plan Recorder").as_ptr(),
                0x0000_0010,
            );
        }
    }
}

#[cfg(windows)]
mod stop {
    use super::*;

    type HandlerRoutine = Option<unsafe extern "system" fn(u32) -> i32>;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetConsoleCtrlHandler(handler: HandlerRoutine, add: i32) -> i32;
    }

    const CTRL_C_EVENT: u32 = 0;
    const CTRL_BREAK_EVENT: u32 = 1;
    const CTRL_CLOSE_EVENT: u32 = 2;
    const CTRL_LOGOFF_EVENT: u32 = 5;
    const CTRL_SHUTDOWN_EVENT: u32 = 6;

    static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
    static HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);

    unsafe extern "system" fn handler(event: u32) -> i32 {
        if matches!(
            event,
            CTRL_C_EVENT
                | CTRL_BREAK_EVENT
                | CTRL_CLOSE_EVENT
                | CTRL_LOGOFF_EVENT
                | CTRL_SHUTDOWN_EVENT
        ) {
            // finalization. It only transfers ownership to the ordinary
            STOP_REQUESTED.store(true, Ordering::Release);
            1
        } else {
            0
        }
    }

    pub struct GracefulStopRegistration {
        _private: (),
    }

    impl GracefulStopRegistration {
        pub fn install() -> Result<Self, String> {
            if HANDLER_INSTALLED.swap(true, Ordering::AcqRel) {
                return Err("auto-attach graceful-stop handler is already installed".into());
            }
            STOP_REQUESTED.store(false, Ordering::Release);
            if unsafe { SetConsoleCtrlHandler(Some(handler), 1) } == 0 {
                HANDLER_INSTALLED.store(false, Ordering::Release);
                return Err(format!(
                    "install auto-attach graceful-stop handler failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(Self { _private: () })
        }

        pub fn is_requested(&self) -> bool {
            STOP_REQUESTED.load(Ordering::Acquire)
        }
    }

    impl Drop for GracefulStopRegistration {
        fn drop(&mut self) {
            unsafe {
                SetConsoleCtrlHandler(Some(handler), 0);
            }
            HANDLER_INSTALLED.store(false, Ordering::Release);
        }
    }
}

#[cfg(windows)]
pub use stop::GracefulStopRegistration;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "hoi4_auto_attach_package_{tag}_{}_{}",
                std::process::id(),
                nonce
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_nonempty(path: &Path) {
        let mut file = fs::File::create(path).unwrap();
        file.write_all(b"fixture").unwrap();
        file.sync_all().unwrap();
    }

    #[test]
    fn ordinary_product_resolves_only_the_sibling_packaged_payload() {
        let dir = TempDir::new("sibling");
        let exe = dir.0.join("collector_record.exe");
        let dll = dir.0.join(PACKAGED_COLLECTOR_DLL_NAME);
        write_nonempty(&exe);
        write_nonempty(&dll);
        assert_eq!(
            resolve_packaged_collector_dll(None, &exe).unwrap(),
            fs::canonicalize(dll).unwrap()
        );
    }

    #[test]
    fn missing_or_empty_packaged_payload_fails_closed() {
        let dir = TempDir::new("missing_empty");
        let exe = dir.0.join("collector_record.exe");
        write_nonempty(&exe);
        assert!(resolve_packaged_collector_dll(None, &exe).is_err());
        fs::File::create(dir.0.join(PACKAGED_COLLECTOR_DLL_NAME)).unwrap();
        assert!(resolve_packaged_collector_dll(None, &exe).is_err());
    }

    #[test]
    fn explicit_support_override_is_still_validated_as_a_nonempty_file() {
        let dir = TempDir::new("override");
        let exe = dir.0.join("collector_record.exe");
        let override_dll = dir.0.join("support-build.dll");
        write_nonempty(&exe);
        write_nonempty(&override_dll);
        assert_eq!(
            resolve_packaged_collector_dll(Some(override_dll.clone()), &exe).unwrap(),
            fs::canonicalize(override_dll).unwrap()
        );
        assert!(resolve_packaged_collector_dll(Some(dir.0.clone()), &exe).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn product_presentation_surfaces_start_failure_and_verified_path() {
        use super::desktop::ProductPresentation;
        use crate::auto_attach::{state::ReasonCode, SupervisorEvent};

        let root = PathBuf::from(r"C:\Users\Example\AppData\Local\Battle Plan Recorder");
        let run_dir = root.join("run_attempt-1");
        let replay = run_dir.join("campaign.battleplan");
        let mut presentation = ProductPresentation::watching(root.clone());
        assert!(presentation
            .status_text
            .contains(&root.display().to_string()));
        assert_eq!(presentation.open_target, root);

        presentation.apply(SupervisorEvent::RecordingStarted {
            pid: 42,
            run_dir: run_dir.clone(),
        });
        assert!(presentation.status_text.contains("Recording started"));
        assert!(presentation.status_text.contains("PID 42"));
        assert_eq!(presentation.open_target, run_dir);

        presentation.apply(SupervisorEvent::RecordingFailed {
            pid: 42,
            reason: ReasonCode::FinalizeVerifyFailed,
            detail: Some("verification rejected the artifact".into()),
            run_dir: None,
        });
        assert!(presentation.status_text.contains("Recording failed"));
        assert!(presentation.status_text.contains("still watching"));
        assert!(presentation
            .status_text
            .contains("verification rejected the artifact"));
        assert!(!presentation.status_text.contains("Attempt folder:"));
        assert!(presentation.status_text.contains("Recordings folder:"));
        assert_eq!(presentation.open_target, root);

        presentation.apply(SupervisorEvent::RecordingSucceeded {
            pid: 42,
            replay_path: replay.clone(),
        });
        assert!(presentation.status_text.contains("finished and verified"));
        assert!(presentation.status_text.contains("Recording started"));
        assert!(presentation.status_text.contains("Recording failed"));
        assert!(presentation
            .status_text
            .contains("verification rejected the artifact"));
        assert!(presentation
            .status_text
            .contains(&replay.display().to_string()));
        assert_eq!(presentation.open_target, replay.parent().unwrap());

        presentation.stopping();
        assert!(presentation.status_text.contains("Stopping safely"));
        presentation.finished(None);
        assert!(presentation.status_text.contains("stopped safely"));
        assert!(presentation.status_text.contains("Select Close"));
        assert!(presentation
            .status_text
            .contains(&replay.display().to_string()));
    }

    #[cfg(windows)]
    #[test]
    fn product_presentation_shows_only_the_unsupported_build_sentence() {
        use super::desktop::ProductPresentation;
        use crate::auto_attach::{state::ReasonCode, SupervisorEvent};
        use crate::game_build::{ObservedGameBuild, UnsupportedGameBuild, VersionSource};

        let root = PathBuf::from(r"C:\Users\Example\AppData\Local\Battle Plan Recorder");
        let refusal = UnsupportedGameBuild {
            observed: ObservedGameBuild {
                display_version: "Operation Bombers v1.20.0.b100 (d300)".to_string(),
                version_source: VersionSource::LauncherSettingsVersion,
                exe_sha256: Some([0x5a; 32]),
                exe_read_error: None,
                exe_path: PathBuf::from(r"D:\Steam\common\the game\hoi4.exe"),
            },
        };
        let mut presentation = ProductPresentation::watching(root.clone());
        presentation.apply(SupervisorEvent::RecordingFailed {
            pid: 42,
            reason: ReasonCode::UnsupportedGameBuild,
            detail: Some(refusal.message()),
            run_dir: Some(root.join("run_attempt-1")),
        });

        let rendered = refusal.message().replace('\n', "\r\n");
        assert_eq!(presentation.status_text, rendered);
        for absent in [
            "Recording failed",
            "PID 42",
            "still watching",
            "Watching for the game",
            "Attempt folder:",
            ReasonCode::UnsupportedGameBuild.as_str(),
        ] {
            assert!(
                !presentation.status_text.contains(absent),
                "window must not show {absent:?}: {}",
                presentation.status_text
            );
        }
        assert_eq!(presentation.open_target, root);

        presentation.finished(Some(refusal.message().as_str()));
        assert!(presentation.status_text.starts_with(&rendered));
        assert!(presentation
            .status_text
            .ends_with("The recorder has stopped. Select Close when ready."));
        assert!(!presentation
            .status_text
            .contains("Recorder stopped because of an error"));
    }

    #[cfg(windows)]
    #[test]
    fn an_earlier_recording_does_not_survive_the_unsupported_build_sentence() {
        use super::desktop::ProductPresentation;
        use crate::auto_attach::{state::ReasonCode, SupervisorEvent};
        use crate::game_build::{ObservedGameBuild, UnsupportedGameBuild, VersionSource};

        let root = PathBuf::from(r"C:\recordings");
        let mut presentation = ProductPresentation::watching(root.clone());
        presentation.apply(SupervisorEvent::RecordingSucceeded {
            pid: 7,
            replay_path: root.join("earlier.battleplan"),
        });
        assert!(presentation.status_text.contains("finished and verified"));

        let refusal = UnsupportedGameBuild {
            observed: ObservedGameBuild {
                display_version: "Operation Bombers v1.20.0.b100 (d300)".to_string(),
                version_source: VersionSource::LauncherSettingsVersion,
                exe_sha256: Some([0x5a; 32]),
                exe_read_error: None,
                exe_path: PathBuf::from("hoi4.exe"),
            },
        };
        presentation.apply(SupervisorEvent::RecordingFailed {
            pid: 8,
            reason: ReasonCode::UnsupportedGameBuild,
            detail: Some(refusal.message()),
            run_dir: None,
        });
        assert_eq!(
            presentation.status_text,
            refusal.message().replace('\n', "\r\n")
        );
    }

    #[cfg(windows)]
    #[test]
    fn product_presentation_reports_no_active_stop_and_fatal_error() {
        use super::desktop::ProductPresentation;

        let root = PathBuf::from(r"C:\recordings");
        let mut presentation = ProductPresentation::watching(root.clone());
        presentation.stopping();
        presentation.finished(None);
        assert!(presentation
            .status_text
            .contains("No active recording needed finalization"));
        assert!(presentation.status_text.contains("Select Close"));
        assert_eq!(presentation.open_target, root);

        presentation.finished(Some("journal write failed"));
        assert!(presentation
            .status_text
            .contains("stopped because of an error"));
        assert!(presentation.status_text.contains("journal write failed"));
        assert!(presentation.status_text.contains("select Close"));
    }

    #[cfg(windows)]
    #[test]
    fn stop_button_requests_stop_until_supervisor_finishes_then_closes() {
        use super::desktop::{
            completion_disposition, stop_button_action, CompletionDisposition, StopButtonAction,
        };

        assert_eq!(stop_button_action(false), StopButtonAction::RequestStop);
        assert_eq!(stop_button_action(true), StopButtonAction::Close);
        assert_eq!(
            completion_disposition(false, false),
            CompletionDisposition::WaitForSupervisor
        );
        assert_eq!(
            completion_disposition(false, true),
            CompletionDisposition::WaitForSupervisor
        );
        assert_eq!(
            completion_disposition(true, false),
            CompletionDisposition::PresentCloseAction
        );
        assert_eq!(
            completion_disposition(true, true),
            CompletionDisposition::AutoClose
        );
    }

    #[cfg(windows)]
    #[test]
    fn failure_folder_selection_never_surfaces_a_non_directory_attempt_path() {
        use super::desktop::select_failure_folder;

        let root = PathBuf::from(r"C:\recordings");
        let candidate = root.join("run_attempt-1");
        assert_eq!(
            select_failure_folder(&root, Some(candidate.clone()), false),
            (root.clone(), None)
        );
        assert_eq!(
            select_failure_folder(&root, Some(candidate.clone()), true),
            (candidate.clone(), Some(candidate))
        );
        assert_eq!(select_failure_folder(&root, None, true), (root, None));
    }

    #[cfg(windows)]
    #[test]
    fn product_event_log_is_chronological_and_bounded() {
        use super::desktop::ProductPresentation;
        use crate::auto_attach::SupervisorEvent;

        let root = PathBuf::from(r"C:\recordings");
        let mut presentation = ProductPresentation::watching(root.clone());
        for pid in 0..80 {
            presentation.apply(SupervisorEvent::RecordingStarted {
                pid,
                run_dir: root.join(format!("run_{pid}")),
            });
        }
        assert!(!presentation.status_text.contains("(PID 0)."));
        assert!(presentation.status_text.contains("(PID 79)."));
        let pid_78 = presentation.status_text.find("(PID 78).").unwrap();
        let pid_79 = presentation.status_text.find("(PID 79).").unwrap();
        assert!(pid_78 < pid_79);
        assert!(presentation.status_text.encode_utf16().count() <= 24_256);
    }
}
