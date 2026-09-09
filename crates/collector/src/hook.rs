//! Retour owns instruction relocation. The five-byte patch is published while
//! existing process threads are suspended, without heap use in that interval.
//! The candidate sites belong to the engine main thread; no thread is created
//! to enter them. Live thread-id witnesses must confirm that static finding.

use retour::RawDetour;
use std::{ffi::c_void, io, mem::size_of, ops::Range, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_NO_MORE_FILES, GetLastError, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::{
            Debug::{CONTEXT, CONTEXT_CONTROL_AMD64, FlushInstructionCache, GetThreadContext},
            ToolHelp::*,
        },
        Memory::*,
        Threading::*,
    },
};

pub struct Hook {
    relocation: RawDetour,
    relay: *mut c_void,
    site: usize,
    original: [u8; 16],
    patch: [u8; 5],
    enabled: bool,
    page_protection: u32,
}

impl Hook {
    /// Caller verified the executable and all 16 sentinel bytes at this site.
    pub unsafe fn new(site: usize, callback: usize, original: [u8; 16]) -> Result<Self, String> {
        // A relay in rel32 range lets retour relocate exactly the same five-byte
        // prologue that we patch. The known candidates have no hot-patch prefix.
        let aligned = site & !0xffff;
        let mut relay = ptr::null_mut();
        for distance in (0x10000..0x7000_0000_usize).step_by(0x10000) {
            for address in [aligned.checked_sub(distance), aligned.checked_add(distance)]
                .into_iter()
                .flatten()
            {
                relay = unsafe {
                    VirtualAlloc(
                        address as *const c_void,
                        4096,
                        MEM_RESERVE | MEM_COMMIT,
                        PAGE_READWRITE,
                    )
                };
                if !relay.is_null() {
                    break;
                }
            }
            if !relay.is_null() {
                break;
            }
        }
        if relay.is_null() {
            return Err("cannot allocate a nearby detour relay".into());
        }
        let result = (|| {
            let mut code = [0_u8; 14];
            code[..6].copy_from_slice(&[0xff, 0x25, 0, 0, 0, 0]);
            code[6..].copy_from_slice(&(callback as u64).to_le_bytes());
            unsafe {
                ptr::copy_nonoverlapping(code.as_ptr(), relay.cast(), code.len());
            }
            let mut previous = 0;
            if unsafe { VirtualProtect(relay, 4096, PAGE_EXECUTE_READ, &mut previous) } == 0
                || unsafe { FlushInstructionCache(GetCurrentProcess(), relay, code.len()) } == 0
            {
                return Err(io::Error::last_os_error().to_string());
            }
            let relocation = unsafe { RawDetour::new(site as *const (), relay.cast()) }
                .map_err(|e| e.to_string())?;
            // Retour emits its trampoline after the relay; publish that code too.
            if unsafe { FlushInstructionCache(GetCurrentProcess(), ptr::null(), 0) } == 0 {
                return Err(io::Error::last_os_error().to_string());
            }
            let mut region: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
            if unsafe {
                VirtualQuery(
                    site as *const c_void,
                    &mut region,
                    size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error().to_string());
            }
            let relative = i32::try_from(relay as i64 - (site + 5) as i64)
                .map_err(|_| "relay outside rel32 range")?;
            let mut patch = [0xe9, 0, 0, 0, 0];
            patch[1..].copy_from_slice(&relative.to_le_bytes());
            Ok(Self {
                relocation,
                relay,
                site,
                original,
                patch,
                enabled: false,
                page_protection: region.Protect,
            })
        })();
        if result.is_err() {
            unsafe {
                VirtualFree(relay, 0, MEM_RELEASE);
            }
        }
        result
    }

    pub fn trampoline(&self) -> usize {
        self.relocation.trampoline() as *const () as usize
    }

    pub fn executable_ranges(&self) -> [Range<usize>; 2] {
        let trampoline = self.trampoline();
        [
            self.relay as usize..self.relay as usize + 4096,
            trampoline & !0xfff..(trampoline & !0xfff) + 4096,
        ]
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        let mut threads = Threads::open()?;
        let mut previous = 0;
        if unsafe {
            VirtualProtect(
                self.site as *const c_void,
                16,
                PAGE_EXECUTE_READWRITE,
                &mut previous,
            )
        } == 0
        {
            return Err(io::Error::last_os_error().to_string());
        }
        // No formatting, allocation, locks, or destruction while threads are held.
        let outcome = (|| {
            threads.suspend()?;
            if threads.contains_ip(std::slice::from_ref(&(self.site..self.site + 16)))? {
                return Ok(false);
            }
            let target = self.site as *mut u8;
            let mut expected = self.original;
            if self.enabled {
                expected[..5].copy_from_slice(&self.patch);
            }
            if unsafe { std::slice::from_raw_parts(target, 16) } != expected {
                return Err(0);
            }
            let bytes = if enabled {
                &self.patch[..]
            } else {
                &self.original[..5]
            };
            unsafe {
                ptr::copy_nonoverlapping(bytes.as_ptr(), target, 5);
            }
            self.enabled = enabled;
            if unsafe { FlushInstructionCache(GetCurrentProcess(), target.cast(), 16) } == 0 {
                return Err(unsafe { GetLastError() });
            }
            if !enabled && unsafe { std::slice::from_raw_parts(target, 16) } != self.original {
                return Err(0);
            }
            Ok(true)
        })();
        let mut ignored = 0;
        let protection_ok = unsafe {
            VirtualProtect(
                self.site as *const c_void,
                16,
                self.page_protection,
                &mut ignored,
            )
        } != 0;
        let resume = threads.resume();
        if !protection_ok {
            return Err("failed to restore hook-page protection".into());
        }
        resume.map_err(|code| format!("ResumeThread failed: {code}"))?;
        outcome
            .map_err(|code| format!("patch refused or verification failed (Windows error {code})"))
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        // The worker never drops an enabled hook or an in-use trampoline.
        assert!(!self.enabled);
        unsafe {
            VirtualFree(self.relay, 0, MEM_RELEASE);
        }
    }
}

struct Thread {
    handle: HANDLE,
    suspended: bool,
}

// The native x64 CONTEXT contract requires 16 bytes even though windows-sys
// 0.61's Rust fields only give the generated struct eight-byte alignment.
#[repr(C, align(16))]
struct AlignedContext(CONTEXT);

pub struct Threads {
    threads: Vec<Thread>,
}

impl Threads {
    pub fn open() -> Result<Self, String> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error().to_string());
        }
        let result = (|| {
            let mut result = Self {
                threads: Vec::new(),
            };
            let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
            entry.dwSize = size_of::<THREADENTRY32>() as u32;
            let mut found = unsafe { Thread32First(snapshot, &mut entry) };
            while found != 0 {
                if entry.th32OwnerProcessID == unsafe { GetCurrentProcessId() }
                    && entry.th32ThreadID != unsafe { GetCurrentThreadId() }
                {
                    let handle = unsafe {
                        OpenThread(
                            THREAD_SUSPEND_RESUME
                                | THREAD_GET_CONTEXT
                                | THREAD_QUERY_LIMITED_INFORMATION
                                | THREAD_SYNCHRONIZE,
                            0,
                            entry.th32ThreadID,
                        )
                    };
                    if handle.is_null() {
                        return Err(io::Error::last_os_error().to_string());
                    }
                    result.threads.push(Thread {
                        handle,
                        suspended: false,
                    });
                }
                found = unsafe { Thread32Next(snapshot, &mut entry) };
            }
            if unsafe { GetLastError() } != ERROR_NO_MORE_FILES {
                return Err(io::Error::last_os_error().to_string());
            }
            Ok(result)
        })();
        unsafe {
            CloseHandle(snapshot);
        }
        result
    }

    pub fn suspend(&mut self) -> Result<(), u32> {
        for thread in &mut self.threads {
            if unsafe { WaitForSingleObject(thread.handle, 0) } == 0 {
                continue;
            }
            if unsafe { SuspendThread(thread.handle) } == u32::MAX {
                return Err(unsafe { GetLastError() });
            }
            thread.suspended = true;
        }
        Ok(())
    }

    pub fn contains_ip(&self, ranges: &[Range<usize>]) -> Result<bool, u32> {
        for thread in self.threads.iter().filter(|t| t.suspended) {
            let mut context = AlignedContext(unsafe { std::mem::zeroed() });
            context.0.ContextFlags = CONTEXT_CONTROL_AMD64;
            if unsafe { GetThreadContext(thread.handle, &mut context.0) } == 0 {
                return Err(unsafe { GetLastError() });
            }
            if ranges
                .iter()
                .any(|range| range.contains(&(context.0.Rip as usize)))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn resume(&mut self) -> Result<(), u32> {
        let mut error = None;
        for thread in self.threads.iter_mut().rev().filter(|t| t.suspended) {
            if unsafe { ResumeThread(thread.handle) } == u32::MAX {
                error = Some(unsafe { GetLastError() });
            } else {
                thread.suspended = false;
            }
        }
        error.map_or(Ok(()), Err)
    }
}

impl Drop for Threads {
    fn drop(&mut self) {
        let _ = self.resume();
        for thread in &self.threads {
            unsafe {
                CloseHandle(thread.handle);
            }
        }
    }
}
