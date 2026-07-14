//! Per-user/session supervisor singleton retained for the supervisor lifetime.

use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;

type Handle = *mut c_void;

#[link(name = "kernel32")]
extern "system" {
    fn CreateMutexW(attributes: *mut c_void, initial_owner: i32, name: *const u16) -> Handle;
    fn ReleaseMutex(handle: Handle) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetLastError() -> u32;
}

const ERROR_ALREADY_EXISTS: u32 = 183;

/// Owned named-mutex guard. Dropping the sole value releases the supervisor
/// lease; it is deliberately neither Clone nor Copy.
pub struct SupervisorMutex {
    handle: Handle,
}

unsafe impl Send for SupervisorMutex {}

impl SupervisorMutex {
    pub fn acquire(session_id: u32, sid_hash: &str) -> Result<Self, String> {
        if sid_hash.is_empty()
            || !sid_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || byte == b'-' || byte == b'_')
        {
            return Err("cannot scope auto-attach singleton to an invalid SID hash".to_string());
        }
        let name = format!(r"Local\HOI4ReplayAutoAttach_{session_id}_{sid_hash}");
        let mut wide: Vec<u16> = OsStr::new(&name).encode_wide().collect();
        wide.push(0);
        let handle = unsafe { CreateMutexW(std::ptr::null_mut(), 1, wide.as_ptr()) };
        if handle.is_null() {
            return Err(format!("CreateMutexW failed: {}", unsafe {
                GetLastError()
            }));
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(handle) };
            return Err(format!(
                "auto-attach supervisor already running for session {session_id}"
            ));
        }
        Ok(Self { handle })
    }
}

impl Drop for SupervisorMutex {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.handle);
            CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_supervisor_is_rejected_until_owner_drops() {
        let unique = format!(
            "{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let first = SupervisorMutex::acquire(std::process::id(), &unique).unwrap();
        let error = SupervisorMutex::acquire(std::process::id(), &unique)
            .err()
            .expect("second supervisor must be rejected");
        assert!(error.contains("already running"), "{error}");
        drop(first);
        SupervisorMutex::acquire(std::process::id(), &unique)
            .expect("mutex becomes available after owner exits");
    }
}
