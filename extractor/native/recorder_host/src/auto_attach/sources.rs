pub mod wmi;

use std::path::{Path, PathBuf};
use std::time::Instant;

use super::identity::{ProcessIdentity, ProcessKey};

pub trait Clock: Send {
    fn now_ms(&self) -> u64;
}

pub trait ProcessProbe: Send {
    fn snapshot(&self) -> Result<ProcessSnapshot, String>;
    fn identify(&self, pid: u32) -> Result<ProcessIdentity, String>;
    fn collector_module_residency(
        &self,
        _pid: u32,
        _dll_path: &Path,
    ) -> Result<CollectorModuleResidency, String> {
        Err("collector module residency probe is unavailable".to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectorModuleResidency {
    Loaded,
    Absent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessSnapshot {
    pub entries: Vec<SnapshotEntry>,
    pub complete: bool,
    pub incomplete_reason: Option<String>,
}

impl ProcessSnapshot {
    pub fn complete(entries: Vec<SnapshotEntry>) -> Self {
        Self {
            entries,
            complete: true,
            incomplete_reason: None,
        }
    }

    pub fn incomplete(entries: Vec<SnapshotEntry>, reason: impl Into<String>) -> Self {
        Self {
            entries,
            complete: false,
            incomplete_reason: Some(reason.into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotEntry {
    pub pid: u32,
    pub name: String,
}

pub trait ProcessEventSource: Send {
    fn drain_events(&mut self) -> Vec<RawProcessEvent>;
    fn health(&self) -> super::policy::WatcherHealth;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawProcessEvent {
    Start { pid: u32, name: String },
    Stop { pid: u32, name: String },
}

pub trait AttachEnvironment: Send {
    fn has_visible_window(&self, key: ProcessKey) -> bool;
    fn resolve_modset(&self, identity: &ProcessIdentity) -> ModsetResolution;
    fn game_build_admission(&self, identity: &ProcessIdentity) -> GameBuildAdmission;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameBuildAdmission {
    Supported,
    Unsupported(Box<crate::game_build::UnsupportedGameBuild>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModsetResolution {
    pub resolved: bool,
    pub reasons: Vec<String>,
}

pub trait PayloadStager: Send {
    fn rearm_after_proven_unload(
        &self,
        key: ProcessKey,
        prior_dll_path: &Path,
    ) -> Result<(), String>;

    fn stage(
        &self,
        key: ProcessKey,
        attempt_id: &str,
        pipe_token: &str,
        pipe_token_hash: &str,
    ) -> Result<StagedPayload, String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedPayload {
    pub staging_dir: PathBuf,
    pub dll_path: PathBuf,
    pub control_path: PathBuf,
    pub dll_sha256: String,
    pub source_sha256: String,
    pub pipe_token: String,
    pub pipe_token_hash: String,
    pub rearmed_after_proven_unload: bool,
}

pub trait WorkerSpawner: Send {
    fn spawn(&self, spec: &WorkerSpec) -> Result<Box<dyn WorkerHandle>, String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerSpec {
    pub key: ProcessKey,
    pub attempt_id: String,
    pub payload: StagedPayload,
    pub out_dir: PathBuf,
    pub session_id: String,
    pub control_challenge: [u8; 16],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerStatus {
    Handshaking,
    /// Heartbeat matched (pid, pipe token, session, ABI, DLL hash). => WaitingForWorld.
    Attached,
    Recording {
        last_confirmed_hour: Option<i64>,
    },
    Finalizing,
    Complete {
        replay_path: String,
    },
    Failed {
        reason: String,
    },
    UnsupportedGameBuild {
        message: String,
    },
}

pub trait WorkerHandle: Send {
    fn poll(&mut self) -> WorkerStatus;
    fn request_stop(&mut self);
}

pub struct SystemClock {
    origin: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl SystemClock {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        self.origin.elapsed().as_millis() as u64
    }
}

#[derive(Default)]
pub struct SnapshotOnlyEventSource;

impl ProcessEventSource for SnapshotOnlyEventSource {
    fn drain_events(&mut self) -> Vec<RawProcessEvent> {
        Vec::new()
    }
    fn health(&self) -> super::policy::WatcherHealth {
        super::policy::WatcherHealth::Degraded
    }
}

#[cfg(windows)]
mod ffi {
    use std::ffi::c_void;

    pub type Handle = *mut c_void;
    pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    pub const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    pub const TH32CS_SNAPMODULE: u32 = 0x0000_0008;
    pub const TH32CS_SNAPMODULE32: u32 = 0x0000_0010;
    pub const ERROR_NO_MORE_FILES: u32 = 18;
    pub const TOKEN_QUERY: u32 = 0x0008;
    pub const TOKEN_USER_CLASS: i32 = 1;
    pub const MAX_PATH: usize = 260;

    #[repr(C)]
    pub struct ProcessEntry32W {
        pub dw_size: u32,
        pub cnt_usage: u32,
        pub th32_process_id: u32,
        pub th32_default_heap_id: usize,
        pub th32_module_id: u32,
        pub cnt_threads: u32,
        pub th32_parent_process_id: u32,
        pub pc_pri_class_base: i32,
        pub dw_flags: u32,
        pub sz_exe_file: [u16; MAX_PATH],
    }

    #[repr(C)]
    pub struct ModuleEntry32W {
        pub dw_size: u32,
        pub th32_module_id: u32,
        pub th32_process_id: u32,
        pub glblcnt_usage: u32,
        pub proccnt_usage: u32,
        pub mod_base_addr: *mut u8,
        pub mod_base_size: u32,
        pub h_module: Handle,
        pub sz_module: [u16; 256],
        pub sz_exe_path: [u16; MAX_PATH],
    }

    #[repr(C)]
    pub struct SidAndAttributes {
        pub sid: *mut c_void,
        pub attributes: u32,
    }

    #[repr(C)]
    pub struct TokenUser {
        pub user: SidAndAttributes,
    }

    #[allow(clashing_extern_declarations)]
    extern "system" {
        pub fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> Handle;
        pub fn CloseHandle(handle: Handle) -> i32;
        pub fn QueryFullProcessImageNameW(
            process: Handle,
            flags: u32,
            exe_name: *mut u16,
            size: *mut u32,
        ) -> i32;
        pub fn GetProcessTimes(
            process: Handle,
            creation: *mut u64,
            exit: *mut u64,
            kernel: *mut u64,
            user: *mut u64,
        ) -> i32;
        pub fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> Handle;
        pub fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
        pub fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
        pub fn Module32FirstW(snapshot: Handle, entry: *mut ModuleEntry32W) -> i32;
        pub fn Module32NextW(snapshot: Handle, entry: *mut ModuleEntry32W) -> i32;
        pub fn GetLastError() -> u32;
        pub fn SetLastError(error: u32);
        pub fn ProcessIdToSessionId(process_id: u32, session_id: *mut u32) -> i32;
        pub fn GetCurrentProcessId() -> u32;
    }

    #[link(name = "advapi32")]
    extern "system" {
        pub fn OpenProcessToken(process: Handle, desired_access: u32, token: *mut Handle) -> i32;
        pub fn GetTokenInformation(
            token: Handle,
            info_class: i32,
            info: *mut c_void,
            info_len: u32,
            return_len: *mut u32,
        ) -> i32;
        pub fn ConvertSidToStringSidW(sid: *mut c_void, string_sid: *mut *mut u16) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub fn LocalFree(mem: *mut c_void) -> *mut c_void;
    }

    pub fn invalid_handle() -> Handle {
        usize::MAX as Handle
    }
}

#[cfg(windows)]
#[derive(Default)]
pub struct ToolhelpProbe;

#[cfg(windows)]
impl ToolhelpProbe {
    pub fn new() -> Self {
        Self
    }

    pub fn current_session_and_sid_hash() -> Result<(u32, String), String> {
        unsafe {
            let pid = ffi::GetCurrentProcessId();
            let session = process_session_id(pid)?;
            let handle = ffi::OpenProcess(ffi::PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return Err("OpenProcess(self) failed".to_string());
            }
            let sid_hash = process_sid_hash(handle);
            ffi::CloseHandle(handle);
            Ok((session, sid_hash?))
        }
    }
}

#[cfg(windows)]
fn process_session_id(pid: u32) -> Result<u32, String> {
    unsafe {
        let mut sid = 0u32;
        if ffi::ProcessIdToSessionId(pid, &mut sid) == 0 {
            return Err(format!("ProcessIdToSessionId({pid}) failed"));
        }
        Ok(sid)
    }
}

#[cfg(windows)]
unsafe fn process_sid_hash(handle: ffi::Handle) -> Result<String, String> {
    use snapshot::sha256::{to_hex, Sha256};
    let mut token: ffi::Handle = std::ptr::null_mut();
    if ffi::OpenProcessToken(handle, ffi::TOKEN_QUERY, &mut token) == 0 {
        return Err("OpenProcessToken failed".to_string());
    }
    let result = (|| {
        let mut len = 0u32;
        ffi::GetTokenInformation(
            token,
            ffi::TOKEN_USER_CLASS,
            std::ptr::null_mut(),
            0,
            &mut len,
        );
        if len == 0 {
            return Err("GetTokenInformation size query failed".to_string());
        }
        let mut buf = vec![0u8; len as usize];
        if ffi::GetTokenInformation(
            token,
            ffi::TOKEN_USER_CLASS,
            buf.as_mut_ptr() as *mut _,
            len,
            &mut len,
        ) == 0
        {
            return Err("GetTokenInformation failed".to_string());
        }
        let token_user = &*(buf.as_ptr() as *const ffi::TokenUser);
        let mut str_sid: *mut u16 = std::ptr::null_mut();
        if ffi::ConvertSidToStringSidW(token_user.user.sid, &mut str_sid) == 0 || str_sid.is_null()
        {
            return Err("ConvertSidToStringSidW failed".to_string());
        }
        let mut n = 0usize;
        while *str_sid.add(n) != 0 {
            n += 1;
        }
        let slice = std::slice::from_raw_parts(str_sid, n);
        let sid_string = String::from_utf16_lossy(slice);
        ffi::LocalFree(str_sid as *mut _);
        let mut h = Sha256::new();
        h.update(sid_string.as_bytes());
        Ok(to_hex(&h.finalize()))
    })();
    ffi::CloseHandle(token);
    result
}

#[cfg(windows)]
impl ProcessProbe for ToolhelpProbe {
    fn snapshot(&self) -> Result<ProcessSnapshot, String> {
        unsafe {
            let snap = ffi::CreateToolhelp32Snapshot(ffi::TH32CS_SNAPPROCESS, 0);
            if snap == ffi::invalid_handle() || snap.is_null() {
                return Err("CreateToolhelp32Snapshot failed".to_string());
            }
            let mut out = Vec::new();
            let mut entry: ffi::ProcessEntry32W = std::mem::zeroed();
            entry.dw_size = std::mem::size_of::<ffi::ProcessEntry32W>() as u32;
            ffi::SetLastError(0);
            if ffi::Process32FirstW(snap, &mut entry) != 0 {
                loop {
                    let mut n = 0usize;
                    while n < entry.sz_exe_file.len() && entry.sz_exe_file[n] != 0 {
                        n += 1;
                    }
                    let name = String::from_utf16_lossy(&entry.sz_exe_file[..n]);
                    out.push(SnapshotEntry {
                        pid: entry.th32_process_id,
                        name,
                    });
                    ffi::SetLastError(0);
                    if ffi::Process32NextW(snap, &mut entry) == 0 {
                        let err = ffi::GetLastError();
                        ffi::CloseHandle(snap);
                        if err == ffi::ERROR_NO_MORE_FILES {
                            return Ok(ProcessSnapshot::complete(out));
                        }
                        return Ok(ProcessSnapshot::incomplete(
                            out,
                            format!("Process32NextW failed with OS error {err}"),
                        ));
                    }
                }
            } else {
                let err = ffi::GetLastError();
                ffi::CloseHandle(snap);
                if err == ffi::ERROR_NO_MORE_FILES {
                    return Ok(ProcessSnapshot::complete(Vec::new()));
                }
                return Err(format!("Process32FirstW failed with OS error {err}"));
            }
        }
    }

    fn identify(&self, pid: u32) -> Result<ProcessIdentity, String> {
        use super::identity::ProcessFiletime;
        unsafe {
            let handle = ffi::OpenProcess(ffi::PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return Err(format!("OpenProcess({pid}) failed"));
            }
            let result = (|| {
                let (mut creation, mut exit, mut kernel, mut user) = (0u64, 0u64, 0u64, 0u64);
                if ffi::GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user)
                    == 0
                {
                    return Err(format!("GetProcessTimes({pid}) failed"));
                }
                let mut buf = vec![0u16; 32 * 1024];
                let mut len = buf.len() as u32;
                if ffi::QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len) == 0 {
                    return Err(format!("QueryFullProcessImageNameW({pid}) failed"));
                }
                let image_path = String::from_utf16_lossy(&buf[..len as usize]);
                let session_id = process_session_id(pid)?;
                let user_sid_hash = process_sid_hash(handle)?;
                Ok(ProcessIdentity {
                    key: ProcessKey {
                        pid,
                        creation_filetime: ProcessFiletime(creation),
                    },
                    image_path,
                    session_id,
                    user_sid_hash,
                })
            })();
            ffi::CloseHandle(handle);
            result
        }
    }

    fn collector_module_residency(
        &self,
        pid: u32,
        dll_path: &Path,
    ) -> Result<CollectorModuleResidency, String> {
        let expected = normalized_windows_path(dll_path);
        if expected.is_empty() {
            return Err("collector module residency path is empty".to_string());
        }
        unsafe {
            let snap = ffi::CreateToolhelp32Snapshot(
                ffi::TH32CS_SNAPMODULE | ffi::TH32CS_SNAPMODULE32,
                pid,
            );
            if snap == ffi::invalid_handle() || snap.is_null() {
                return Err(format!(
                    "CreateToolhelp32Snapshot(modules, {pid}) failed with OS error {}",
                    ffi::GetLastError()
                ));
            }
            let result = (|| {
                let mut entry: ffi::ModuleEntry32W = std::mem::zeroed();
                entry.dw_size = std::mem::size_of::<ffi::ModuleEntry32W>() as u32;
                ffi::SetLastError(0);
                if ffi::Module32FirstW(snap, &mut entry) == 0 {
                    let error = ffi::GetLastError();
                    if error == ffi::ERROR_NO_MORE_FILES {
                        return Ok(CollectorModuleResidency::Absent);
                    }
                    return Err(format!(
                        "Module32FirstW({pid}) failed with OS error {error}"
                    ));
                }
                loop {
                    let mut n = 0usize;
                    while n < entry.sz_exe_path.len() && entry.sz_exe_path[n] != 0 {
                        n += 1;
                    }
                    let observed_text = String::from_utf16_lossy(&entry.sz_exe_path[..n]);
                    let observed = normalized_windows_path(Path::new(&observed_text));
                    if observed == expected {
                        return Ok(CollectorModuleResidency::Loaded);
                    }
                    ffi::SetLastError(0);
                    if ffi::Module32NextW(snap, &mut entry) == 0 {
                        let error = ffi::GetLastError();
                        if error == ffi::ERROR_NO_MORE_FILES {
                            return Ok(CollectorModuleResidency::Absent);
                        }
                        return Err(format!(
                            "Module32NextW({pid}) stopped with OS error {error}"
                        ));
                    }
                }
            })();
            ffi::CloseHandle(snap);
            result
        }
    }
}

#[cfg(windows)]
fn normalized_windows_path(path: &Path) -> String {
    path.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('/', r"\")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
}
