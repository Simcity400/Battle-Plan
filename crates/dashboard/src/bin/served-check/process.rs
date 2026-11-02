use windows_sys::Win32::{
    Foundation::FILETIME,
    System::Threading::{GetCurrentProcess, GetProcessTimes},
};

pub(super) struct Metrics {
    pub cpu_seconds: Option<f64>,
    pub peak_commit_bytes: Option<usize>,
}

#[repr(C)]
#[derive(Default)]
struct Counters {
    size: u32,
    page_faults: u32,
    peak_working_set: usize,
    working_set: usize,
    quota_peak_paged_pool: usize,
    quota_paged_pool: usize,
    quota_peak_non_paged_pool: usize,
    quota_non_paged_pool: usize,
    committed: usize,
    peak_committed: usize,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn K32GetProcessMemoryInfo(
        process: windows_sys::Win32::Foundation::HANDLE,
        counters: *mut Counters,
        size: u32,
    ) -> i32;
}

pub(super) fn metrics() -> Metrics {
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    let ticks =
        |time: FILETIME| u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime);
    // SAFETY: current-process pseudo-handle and four owned FILETIMEs.
    let cpu = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    let mut counters = Counters {
        size: size_of::<Counters>() as u32,
        ..Counters::default()
    };
    // SAFETY: current-process pseudo-handle and an owned block of its stated size.
    let memory =
        unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.size) };
    Metrics {
        cpu_seconds: (cpu != 0).then(|| (ticks(kernel) + ticks(user)) as f64 / 10_000_000.0),
        peak_commit_bytes: (memory != 0).then_some(counters.peak_committed),
    }
}
