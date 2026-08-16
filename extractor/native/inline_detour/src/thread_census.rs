//! A READ-ONLY census of this process's other threads: which are hot and which
//! logical processor each prefers (lane LO, 2026-08-16).
//!
//! # Why it lives in this crate
//!
//! Thread enumeration is a choke point here by design — the source ban
//! `no_private_thread_suspender_outside_this_crate` refuses any other crate a
//! `Thread32Next` loop, because the private walkers this crate replaced read a
//! broken walk as the end of the list. This census suspends nothing and patches
//! nothing; it is a plain sample. But it enumerates, so it lives beside the
//! freeze's own snapshot walk (`fill_with_snapshotted_thread_ids`) rather than
//! being the fifth private copy of it.
//!
//! # What it is for
//!
//! The collector's worker-affinity policy: sample cycles per thread over a short
//! window, take the ideal processor of the busiest few, and let the caller keep
//! its own workers off those processors. Handles are opened with
//! `THREAD_QUERY_LIMITED_INFORMATION` only; nothing about any thread is set.

use core::time::Duration;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_NO_MORE_FILES, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::Kernel::PROCESSOR_NUMBER;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, GetProcessAffinityMask,
    GetThreadIdealProcessorEx, OpenThread, THREAD_QUERY_LIMITED_INFORMATION,
};

// `QueryThreadCycleTime` sits behind a `windows-sys` feature this crate does not
// otherwise carry (Win32_System_Threading has it in some versions, Realtime in
// others); a hand declaration is the one-liner that does not widen the feature
// set.
#[link(name = "kernel32")]
extern "system" {
    fn QueryThreadCycleTime(thread: HANDLE, cycles: *mut u64) -> i32;
}

/// One sampled thread: cycles consumed over the window and its ideal
/// processor (group-relative number in group 0; `None` when unreadable or in
/// another processor group).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotThread {
    pub thread_id: u32,
    pub cycles: u64,
    pub ideal_processor: Option<u8>,
}

/// The census: the process affinity mask, how many threads were sampled, and
/// the busiest threads in descending cycle order (at most `top`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadCensus {
    pub process_affinity_mask: usize,
    pub threads_sampled: usize,
    pub hottest: Vec<HotThread>,
}

/// Why no census could be taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadCensusRefusal {
    ProcessAffinityUnreadable { last_error: u32 },
    SnapshotUnavailable { last_error: u32 },
    /// The walk stopped for a reason other than "no more threads": the list
    /// is TRUNCATED and nothing may be decided from it (the defect the crate's
    /// choke-point policy names — a broken walk read as the end of the list).
    WalkBroken { last_error: u32 },
}

/// Sample every OTHER thread of this process over `window` and return the
/// `top` busiest with their ideal processors. Blocking for `window` on the
/// calling thread; opens no handle it does not close.
pub fn sample_other_threads(window: Duration, top: usize) -> Result<ThreadCensus, ThreadCensusRefusal> {
    // SAFETY: plain Win32 calls with documented arguments; every handle
    // opened is closed before returning; no thread is suspended, patched or
    // otherwise touched.
    unsafe {
        let mut process_mask = 0usize;
        let mut system_mask = 0usize;
        if GetProcessAffinityMask(GetCurrentProcess(), &mut process_mask, &mut system_mask) == 0
            || process_mask == 0
        {
            return Err(ThreadCensusRefusal::ProcessAffinityUnreadable {
                last_error: GetLastError(),
            });
        }
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
            return Err(ThreadCensusRefusal::SnapshotUnavailable {
                last_error: GetLastError(),
            });
        }
        let own_pid = GetCurrentProcessId();
        let own_tid = GetCurrentThreadId();
        let mut entry: THREADENTRY32 = core::mem::zeroed();
        entry.dwSize = core::mem::size_of::<THREADENTRY32>() as u32;
        let mut handles: Vec<(u32, HANDLE)> = Vec::new();
        // FAIL CLOSED ON A BROKEN WALK: only `ERROR_NO_MORE_FILES` is the end
        // of the list; any other stop leaves the census truncated and is
        // refused, exactly as the freeze's own walk refuses it.
        let mut stepped = Thread32First(snapshot, &mut entry);
        let walk = loop {
            if stepped == 0 {
                let last_error = GetLastError();
                break if last_error == ERROR_NO_MORE_FILES {
                    Ok(())
                } else {
                    Err(ThreadCensusRefusal::WalkBroken { last_error })
                };
            }
            if entry.th32OwnerProcessID == own_pid && entry.th32ThreadID != own_tid {
                let handle = OpenThread(THREAD_QUERY_LIMITED_INFORMATION, 0, entry.th32ThreadID);
                if !handle.is_null() {
                    handles.push((entry.th32ThreadID, handle));
                }
            }
            entry.dwSize = core::mem::size_of::<THREADENTRY32>() as u32;
            stepped = Thread32Next(snapshot, &mut entry);
        };
        CloseHandle(snapshot);
        if let Err(refusal) = walk {
            for &(_, handle) in &handles {
                CloseHandle(handle);
            }
            return Err(refusal);
        }

        let cycles_of = |handle: HANDLE| -> u64 {
            let mut value = 0u64;
            if QueryThreadCycleTime(handle, &mut value) != 0 {
                value
            } else {
                0
            }
        };
        let before: Vec<u64> = handles.iter().map(|&(_, handle)| cycles_of(handle)).collect();
        std::thread::sleep(window);
        let mut sampled: Vec<HotThread> = handles
            .iter()
            .enumerate()
            .map(|(index, &(thread_id, handle))| {
                let mut ideal = PROCESSOR_NUMBER {
                    Group: 0,
                    Number: 0,
                    Reserved: 0,
                };
                let ideal_processor =
                    if GetThreadIdealProcessorEx(handle, &mut ideal) != 0 && ideal.Group == 0 {
                        Some(ideal.Number)
                    } else {
                        None
                    };
                HotThread {
                    thread_id,
                    cycles: cycles_of(handle).saturating_sub(before[index]),
                    ideal_processor,
                }
            })
            .collect();
        for &(_, handle) in &handles {
            CloseHandle(handle);
        }
        let threads_sampled = sampled.len();
        sampled.sort_unstable_by(|a, b| b.cycles.cmp(&a.cycles).then(a.thread_id.cmp(&b.thread_id)));
        sampled.truncate(top);
        Ok(ThreadCensus {
            process_affinity_mask: process_mask,
            threads_sampled,
            hottest: sampled,
        })
    }
}
