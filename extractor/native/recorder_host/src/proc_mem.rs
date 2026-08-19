//! Process peak-memory telemetry (Windows) for the G5 bounded-memory gate.
//!
//! The recorder records its own peak private (pagefile-committed) bytes and peak
//! working set into `native_session_manifest.json` so G5 — "peak memory is
//! O(current world state), independent of session length" — is measurable from
//! the manifest of every future run, not just from an ad-hoc profiler. Private
//! (commit) is the headline number: unlike working set it excludes evictable
//! file-backed pages, so it is the honest "did we allocate proportional to the
//! stream?" signal.
//!
//! A process peak answers G5 with one number and can never say WHICH owner it
//! belongs to, which is why [`crate::mem_account`] now publishes the component
//! table beside it. Nor is G5 settled by these counters: the artifact build was
//! measured world-size-dominated from 129 to 602 game hours in the retained
//! `t30_*_report.json` and `t31_final_report.json` receipts, but the session
//! manifest's own rewrite — 33.5 KB of JSON per game hour — happens after this
//! reading is taken and is not covered by it.

use std::ffi::c_void;

type Handle = *mut c_void;

#[repr(C)]
struct ProcessMemoryCounters {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
}

// Both exported by kernel32.dll (the K32-prefixed PSAPI shim; Windows 7+), so no
// extra link directive beyond the default kernel32 is needed.
extern "system" {
    fn GetCurrentProcess() -> Handle;
    fn K32GetProcessMemoryInfo(
        process: Handle,
        counters: *mut ProcessMemoryCounters,
        cb: u32,
    ) -> i32;
}

/// A peak-memory snapshot in bytes.
#[derive(Debug, Clone, Copy)]
pub struct PeakMemory {
    /// Peak private commit (pagefile) usage — the bounded-allocation signal.
    pub peak_private_bytes: u64,
    /// Peak working set (includes evictable shared/file-backed resident pages).
    pub peak_working_set_bytes: u64,
}

/// An instantaneous memory reading in bytes.
///
/// The peak counters answer "how bad did it ever get"; these answer "how bad is
/// it right now", which is what a phase-by-phase attribution needs — a peak
/// cannot be differenced between two points in time, because it never falls.
#[derive(Debug, Clone, Copy)]
pub struct CurrentMemory {
    /// Current private (commit) bytes.
    pub private_bytes: u64,
    /// Current working set bytes.
    pub working_set_bytes: u64,
}

/// Query the current process's instantaneous memory counters.
pub fn current_memory() -> Option<CurrentMemory> {
    let counters = query()?;
    Some(CurrentMemory {
        private_bytes: counters.pagefile_usage as u64,
        working_set_bytes: counters.working_set_size as u64,
    })
}

/// Current private (commit) bytes, or `None` if the OS call fails.
pub fn current_private_bytes() -> Option<u64> {
    current_memory().map(|memory| memory.private_bytes)
}

/// Query the current process's peak memory counters. Returns `None` if the OS
/// call fails (telemetry is best-effort and never gates a capture).
pub fn peak_memory() -> Option<PeakMemory> {
    let counters = query()?;
    Some(PeakMemory {
        peak_private_bytes: counters.peak_pagefile_usage as u64,
        peak_working_set_bytes: counters.peak_working_set_size as u64,
    })
}

/// One `GetProcessMemoryInfo` call. Both the peak and the current readings come
/// from the same struct, so there is exactly one FFI site to get right.
fn query() -> Option<ProcessMemoryCounters> {
    let mut counters = ProcessMemoryCounters {
        cb: std::mem::size_of::<ProcessMemoryCounters>() as u32,
        page_fault_count: 0,
        peak_working_set_size: 0,
        working_set_size: 0,
        quota_peak_paged_pool_usage: 0,
        quota_paged_pool_usage: 0,
        quota_peak_non_paged_pool_usage: 0,
        quota_non_paged_pool_usage: 0,
        pagefile_usage: 0,
        peak_pagefile_usage: 0,
    };
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<ProcessMemoryCounters>() as u32,
        )
    };
    if ok == 0 {
        return None;
    }
    Some(counters)
}
