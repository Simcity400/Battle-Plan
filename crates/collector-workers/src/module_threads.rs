use std::{ffi::c_void, ops::Range};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_INVALID_PARAMETER, ERROR_NO_MORE_FILES, GetLastError, HANDLE, HMODULE,
        INVALID_HANDLE_VALUE, WAIT_EVENT, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    System::{
        Diagnostics::{
            Debug::IMAGE_NT_HEADERS64,
            ToolHelp::{
                CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First,
                Thread32Next,
            },
        },
        SystemServices::IMAGE_DOS_HEADER,
        Threading::{
            GetCurrentProcessId, GetCurrentThreadId, OpenThread, THREAD_QUERY_INFORMATION,
            THREAD_SYNCHRONIZE, WaitForSingleObject,
        },
    },
};

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQueryInformationThread(
        thread: HANDLE,
        class: u32,
        information: *mut c_void,
        length: u32,
        returned: *mut u32,
    ) -> i32;
}
const THREAD_START_ADDRESS: u32 = 9;

pub fn started_in(module: HMODULE) -> Result<Vec<u32>, String> {
    let image = image(module);
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(format!(
            "its threads could not be listed (error {})",
            unsafe { GetLastError() }
        ));
    }
    let mut found = Vec::new();
    let listed = (|| {
        let process = unsafe { GetCurrentProcessId() };
        let current = unsafe { GetCurrentThreadId() };
        let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        let mut more = unsafe { Thread32First(snapshot, &raw mut entry) };
        while more != 0 {
            let id = entry.th32ThreadID;
            if entry.th32OwnerProcessID == process && id != current && runs_from(id, &image)? {
                found.push(id);
            }
            more = unsafe { Thread32Next(snapshot, &raw mut entry) };
        }
        match unsafe { GetLastError() } {
            ERROR_NO_MORE_FILES => Ok(()),
            error => Err(format!("its threads could not be listed (error {error})")),
        }
    })();
    unsafe { CloseHandle(snapshot) };
    listed.map(|()| found)
}

fn image(module: HMODULE) -> Range<usize> {
    let base = module as usize;
    // SAFETY: a loaded image's headers stay mapped while it is loaded, which the caller's being
    // in it or holding it guarantees.
    let size = unsafe {
        let dos = std::ptr::read_unaligned(base as *const IMAGE_DOS_HEADER);
        let nt =
            std::ptr::read_unaligned((base + dos.e_lfanew as usize) as *const IMAGE_NT_HEADERS64);
        nt.OptionalHeader.SizeOfImage as usize
    };
    base..base + size
}

fn runs_from(id: u32, image: &Range<usize>) -> Result<bool, String> {
    let thread = unsafe { OpenThread(THREAD_QUERY_INFORMATION | THREAD_SYNCHRONIZE, 0, id) };
    if thread.is_null() {
        return match unsafe { GetLastError() } {
            ERROR_INVALID_PARAMETER => Ok(false),
            error => Err(format!("thread {id} could not be opened (error {error})")),
        };
    }
    let told = (|| {
        let wait = unsafe { WaitForSingleObject(thread, 0) };
        if !alive(id, wait, unsafe { GetLastError() })? {
            return Ok(false);
        }
        let mut start = 0_usize;
        let status = unsafe {
            NtQueryInformationThread(
                thread,
                THREAD_START_ADDRESS,
                (&raw mut start).cast(),
                size_of::<usize>() as u32,
                std::ptr::null_mut(),
            )
        };
        if status < 0 {
            return Err(format!(
                "thread {id}'s start could not be read (status {status:#x})"
            ));
        }
        Ok(image.contains(&start))
    })();
    unsafe { CloseHandle(thread) };
    told
}

fn alive(id: u32, wait: WAIT_EVENT, error: u32) -> Result<bool, String> {
    match wait {
        WAIT_OBJECT_0 => Ok(false),
        WAIT_TIMEOUT => Ok(true),
        WAIT_FAILED => Err(format!(
            "thread {id}'s liveness could not be read (error {error})"
        )),
        other => Err(format!(
            "thread {id}'s liveness could not be read (wait result {other:#x})"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Foundation::WAIT_ABANDONED;

    #[test]
    fn only_a_signalled_wait_is_a_thread_gone() {
        assert_eq!(alive(7, WAIT_OBJECT_0, 0), Ok(false));
        assert_eq!(alive(7, WAIT_TIMEOUT, 0), Ok(true));
        let failed = alive(7, WAIT_FAILED, 6).unwrap_err();
        assert!(
            failed.contains("thread 7") && failed.contains("error 6"),
            "{failed}"
        );
        assert!(alive(7, WAIT_ABANDONED, 0).is_err());
    }
}
