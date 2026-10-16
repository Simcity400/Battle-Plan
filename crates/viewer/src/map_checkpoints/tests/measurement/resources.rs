//! Process affinity and per-row private-commit sampling for the ignored harness.
use std::{
    io,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};

#[cfg(windows)]
pub(super) fn affinity() -> io::Result<String> {
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetProcessAffinityMask, SetProcessAffinityMask,
    };
    if let Some(value) = std::env::var_os("BATTLE_PLAN_MAP_BENCH_PROCESSOR_AFFINITY").or_else(|| std::env::var_os("HOI4_MAP_BENCH_PROCESSOR_AFFINITY")) {
        let value = value.to_string_lossy();
        let mask = if let Some(hex) = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
        {
            usize::from_str_radix(hex, 16)
        } else {
            value.parse()
        }
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        if mask == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "processor affinity must be nonzero",
            ));
        }
        // SAFETY: the current-process pseudo handle and validated mask are valid.
        if unsafe { SetProcessAffinityMask(GetCurrentProcess(), mask) } == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    let (mut process, mut system) = (0, 0);
    // SAFETY: both outputs are live usize values; the pseudo handle is valid.
    if unsafe { GetProcessAffinityMask(GetCurrentProcess(), &mut process, &mut system) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(format!("0x{process:x}"))
}

#[cfg(not(windows))]
pub(super) fn affinity() -> io::Result<String> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "map memory/affinity measurement requires Windows",
    ))
}

#[cfg(windows)]
fn private_bytes() -> io::Result<(usize, usize)> {
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        cb: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        pools: [usize; 4],
        pagefile: usize,
        peak_pagefile: usize,
        private: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn K32GetProcessMemoryInfo(
            process: windows_sys::Win32::Foundation::HANDLE,
            counters: *mut Counters,
            size: u32,
        ) -> i32;
    }
    let mut counters = Counters {
        cb: std::mem::size_of::<Counters>() as u32,
        ..Default::default()
    };
    // SAFETY: Counters has the Windows PROCESS_MEMORY_COUNTERS_EX layout and
    // cb names its complete size; the current-process pseudo handle is valid.
    if unsafe {
        K32GetProcessMemoryInfo(
            windows_sys::Win32::System::Threading::GetCurrentProcess(),
            &mut counters,
            counters.cb,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok((counters.private, counters.peak_pagefile))
}

#[cfg(not(windows))]
fn private_bytes() -> io::Result<(usize, usize)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "private bytes measurement requires Windows",
    ))
}

pub(super) struct Peak {
    stop: Arc<(Mutex<bool>, Condvar)>,
    worker: Option<JoinHandle<io::Result<(usize, usize, u64)>>>,
    baseline: usize,
}

impl Peak {
    pub(super) fn start() -> Self {
        let (baseline, lifetime) = private_bytes().unwrap();
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let thread_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let (mut peak, mut lifetime, mut samples) = (baseline, lifetime, 1);
            loop {
                let (lock, wake) = &*thread_stop;
                let stopped = lock.lock().unwrap();
                let (stopped, _) = wake
                    .wait_timeout_while(stopped, Duration::from_millis(10), |stopped| !*stopped)
                    .unwrap();
                let finish = *stopped;
                drop(stopped);
                let (private, process_peak) = private_bytes()?;
                peak = peak.max(private);
                lifetime = lifetime.max(process_peak);
                samples += 1;
                if finish {
                    return Ok((peak, lifetime, samples));
                }
            }
        });
        Self {
            stop,
            worker: Some(worker),
            baseline,
        }
    }

    pub(super) fn finish(mut self) -> serde_json::Value {
        self.signal();
        let (peak, lifetime, samples) = self.worker.take().unwrap().join().unwrap().unwrap();
        serde_json::json!({"peakPrivateBytes": peak, "baselinePrivateBytes": self.baseline,
            "processLifetimePeakCommitBytes": lifetime, "samples": samples, "sampleIntervalMs": 10,
            "scope": "current-process PrivateUsage sampled across this prefix; includes resident baseline, attribution and oracle/source alternatives; lifetime peak commit is cumulative across rows; excludes GPU and native hour builders"})
    }

    fn signal(&self) {
        let (lock, wake) = &*self.stop;
        *lock.lock().unwrap() = true;
        wake.notify_one();
    }
}

impl Drop for Peak {
    fn drop(&mut self) {
        self.signal();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
