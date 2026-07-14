//! Abstract seams (design §9 "Abstract ProcessEventSource, ProcessProbe, Clock,
//! PayloadStager, WorkerSpawner for deterministic race, PID-reuse, and WMI-loss
//! tests") plus the real Windows adapters.
//!
//! Everything the coordinator depends on that touches the OS or another process
//! is a trait here, so the coordinator's logic is driven by fakes in tests and
//! by these adapters in production. The real adapters use the same raw
//! `extern "system"` FFI style already established in `modset.rs` — this crate
//! deliberately carries no `windows`/`windows-sys` dependency (that Cargo change
//! is design §9, outside this lane's fence; see the dossier).

/// Real WMI `Win32_ProcessTrace` push source (design §2). Drop-in replacement
/// for [`SnapshotOnlyEventSource`] in production wiring; see the dossier
/// `W12C_HEARTBEAT_WMI.md` for the one-line `run_supervisor` swap.
pub mod wmi;

use std::path::PathBuf;
use std::time::Instant;

use super::identity::{ProcessIdentity, ProcessKey};

/// Monotonic millisecond clock. Abstracted so tests advance time deterministically.
pub trait Clock: Send {
    fn now_ms(&self) -> u64;
}

/// A live process enumeration + per-pid identity probe. Snapshot is the
/// authoritative liveness source (design §2); identify resolves the full
/// authority-grade identity from the live handle (design §3).
pub trait ProcessProbe: Send {
    /// All currently-live processes as `(pid, image-name-or-path)`. Used to find
    /// `hoi4.exe` candidates and to prove liveness during reconciliation.
    fn snapshot(&self) -> Result<ProcessSnapshot, String>;
    /// Resolve the full identity of `pid` from its live OS handle. Fails closed
    /// (Err) if any authority field (creation time, path, session, SID) cannot
    /// be read — the coordinator then holds the candidate in `WaitingForIdentity`.
    fn identify(&self, pid: u32) -> Result<ProcessIdentity, String>;
}

/// One Toolhelp enumeration together with the witness needed before absence may
/// be interpreted as process exit. Positive entries from an incomplete snapshot
/// remain useful for discovery, but an incomplete snapshot can never prove that
/// an unlisted process exited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessSnapshot {
    pub entries: Vec<SnapshotEntry>,
    pub complete: bool,
    /// Present only when `complete == false`; retained for diagnostics instead
    /// of silently presenting a truncated list as authoritative.
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
    /// Executable basename as reported by the enumeration (Toolhelp gives just
    /// the basename; that is enough for the cheap `hoi4.exe` pre-filter).
    pub name: String,
}

/// Low-latency push source (WMI in production). Health drives the reconciliation
/// cadence (design §2). `drain_events` is non-blocking.
pub trait ProcessEventSource: Send {
    fn drain_events(&mut self) -> Vec<RawProcessEvent>;
    fn health(&self) -> super::policy::WatcherHealth;
}

/// A raw push event before identity resolution. Start events carry only the pid
/// and name; the coordinator probes to get the FILETIME/identity (WMI SID/
/// session are hints only, design §3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawProcessEvent {
    Start { pid: u32, name: String },
    Stop { pid: u32, name: String },
}

/// Window-readiness + modset resolution for a resolved identity (design §3, §6).
pub trait AttachEnvironment: Send {
    /// The injector requires a visible PID-owned window (design §6); until then
    /// the candidate stays in `WaitingForWindow`.
    fn has_visible_window(&self, pid: u32) -> bool;
    /// Resolve the modset for this identity. `resolved == false` blocks
    /// injection (design §3: "auto-attach blocks injection while preserving
    /// errors").
    fn resolve_modset(&self, identity: &ProcessIdentity) -> ModsetResolution;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModsetResolution {
    pub resolved: bool,
    pub reasons: Vec<String>,
}

/// Stages a fresh, never-before-used collector DLL directory per attempt
/// (design §5 unique DLL path — never reuse a path hooked into a live game).
pub trait PayloadStager: Send {
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
    /// SHA-256 of the copied DLL; a mismatch vs the source hash fails closed.
    pub dll_sha256: String,
    pub source_sha256: String,
    /// Coordinator-minted raw token actually written to the worker control file
    /// and used in the data/control pipe names. It is never journalled.
    pub pipe_token: String,
    /// Hash of that exact raw token. Only this derivative is journalled.
    pub pipe_token_hash: String,
}

/// Spawns and supervises one `collector_record` worker per game process
/// (design §5, §7). The worker performs the actual single hook injection; the
/// coordinator never injects directly.
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
    /// Fresh challenge echoed by the worker's H4CT proof messages. Like the pipe
    /// token, this is process-local authority and is never journalled raw.
    pub control_challenge: [u8; 16],
}

/// A running worker's reported status, polled by the coordinator. This is the
/// collector-proof ladder: hook success alone is NOT "attached" (design
/// self-refutation) — the coordinator only advances on these worker-proven
/// milestones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerStatus {
    /// Hook installed; waiting for a session-matching heartbeat.
    Handshaking,
    /// Heartbeat matched (pid, pipe token, session, ABI, DLL hash). => WaitingForWorld.
    Attached,
    /// A complete known-type H4CR record with its type's minimum payload arrived;
    /// capture is live. Unknown/header-only envelopes never prove this state.
    Recording { last_confirmed_hour: Option<i64> },
    /// Folding/packing/verifying.
    Finalizing,
    /// Verified replay produced.
    Complete { replay_path: String },
    /// Hard failure; carries a reason for the red status card.
    Failed { reason: String },
}

pub trait WorkerHandle: Send {
    /// Non-blocking status poll.
    fn poll(&mut self) -> WorkerStatus;
    /// Signal a graceful stop (explicit app quit / process-exit finalize).
    fn request_stop(&mut self);
}

// =====================================================================
// Real adapters
// =====================================================================

/// Monotonic clock from `Instant`.
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

/// A snapshot-only event source: reports `Degraded` health so the coordinator
/// relies on reconciliation polling alone (the design's WMI-loss fallback,
/// which is fully functional for both app-first and game-first, just at the 2 s
/// discovery latency). The real WMI `Win32_ProcessStartTrace` source needs the
/// COM binding from design §9 (a Cargo change outside this lane) — documented in
/// the dossier. Until it lands, this keeps the product correct and shippable.
#[derive(Default)]
pub struct SnapshotOnlyEventSource;

impl ProcessEventSource for SnapshotOnlyEventSource {
    fn drain_events(&mut self) -> Vec<RawProcessEvent> {
        Vec::new()
    }
    fn health(&self) -> super::policy::WatcherHealth {
        // No push channel => permanently in the bounded-poll fallback.
        super::policy::WatcherHealth::Degraded
    }
}

// ---------------------------------------------------------------------
// Windows Toolhelp process probe (raw FFI, mirrors modset.rs style).
// ---------------------------------------------------------------------

#[cfg(windows)]
mod ffi {
    use std::ffi::c_void;

    pub type Handle = *mut c_void;
    pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    pub const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    pub const ERROR_NO_MORE_FILES: u32 = 18;
    pub const TOKEN_QUERY: u32 = 0x0008;
    pub const TOKEN_USER_CLASS: i32 = 1; // TOKEN_INFORMATION_CLASS::TokenUser
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
    pub struct SidAndAttributes {
        pub sid: *mut c_void,
        pub attributes: u32,
    }

    #[repr(C)]
    pub struct TokenUser {
        pub user: SidAndAttributes,
    }

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

    /// INVALID_HANDLE_VALUE == (HANDLE)-1.
    pub fn invalid_handle() -> Handle {
        usize::MAX as Handle
    }
}

/// Real Windows process probe (Toolhelp snapshot + handle-authority identify).
#[cfg(windows)]
#[derive(Default)]
pub struct ToolhelpProbe;

#[cfg(windows)]
impl ToolhelpProbe {
    pub fn new() -> Self {
        Self
    }

    /// The coordinator's OWN (session_id, sid_hash), used to prove a candidate
    /// is same-user/same-session (design §3). Read once at startup.
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

/// SHA-256 hex of the process token's user SID string. Raw SID is never
/// returned (design §8).
#[cfg(windows)]
unsafe fn process_sid_hash(handle: ffi::Handle) -> Result<String, String> {
    use snapshot::sha256::{to_hex, Sha256};
    let mut token: ffi::Handle = std::ptr::null_mut();
    if ffi::OpenProcessToken(handle, ffi::TOKEN_QUERY, &mut token) == 0 {
        return Err("OpenProcessToken failed".to_string());
    }
    let result = (|| {
        let mut len = 0u32;
        // First call sizes the buffer.
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
        // Read the wide string.
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_is_monotonic_nondecreasing() {
        let c = SystemClock::new();
        let a = c.now_ms();
        let b = c.now_ms();
        assert!(b >= a);
    }

    #[test]
    fn snapshot_only_source_reports_degraded_and_no_events() {
        let mut s = SnapshotOnlyEventSource;
        assert!(s.drain_events().is_empty());
        assert_eq!(s.health(), super::super::policy::WatcherHealth::Degraded);
    }
}
