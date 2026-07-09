//! Process peak-memory telemetry (Windows) for the G5 bounded-memory gate.
//!
//! The recorder records its own peak private (pagefile-committed) bytes and peak
//! working set into `native_session_manifest.json` so G5 — "peak memory is
//! O(current world state), independent of session length" — is measurable from
//! the manifest of every future run, not just from an ad-hoc profiler. Private
//! (commit) is the headline number: unlike working set it excludes evictable
//! file-backed pages, so it is the honest "did we allocate proportional to the
//! stream?" signal.

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

/// Query the current process's peak memory counters. Returns `None` if the OS
/// call fails (telemetry is best-effort and never gates a capture).
pub fn peak_memory() -> Option<PeakMemory> {
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
    Some(PeakMemory {
        peak_private_bytes: counters.peak_pagefile_usage as u64,
        peak_working_set_bytes: counters.peak_working_set_size as u64,
    })
}
