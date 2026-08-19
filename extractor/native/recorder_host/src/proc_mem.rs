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

extern "system" {
    fn GetCurrentProcess() -> Handle;
    fn K32GetProcessMemoryInfo(
        process: Handle,
        counters: *mut ProcessMemoryCounters,
        cb: u32,
    ) -> i32;
}

#[derive(Debug, Clone, Copy)]
pub struct PeakMemory {
    pub peak_private_bytes: u64,
    pub peak_working_set_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct CurrentMemory {
    pub private_bytes: u64,
    pub working_set_bytes: u64,
}

pub fn current_memory() -> Option<CurrentMemory> {
    let counters = query()?;
    Some(CurrentMemory {
        private_bytes: counters.pagefile_usage as u64,
        working_set_bytes: counters.working_set_size as u64,
    })
}

pub fn current_private_bytes() -> Option<u64> {
    current_memory().map(|memory| memory.private_bytes)
}

pub fn peak_memory() -> Option<PeakMemory> {
    let counters = query()?;
    Some(PeakMemory {
        peak_private_bytes: counters.peak_pagefile_usage as u64,
        peak_working_set_bytes: counters.peak_working_set_size as u64,
    })
}

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
