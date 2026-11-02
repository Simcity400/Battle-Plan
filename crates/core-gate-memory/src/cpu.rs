use std::{io, time::Instant};

#[cfg(windows)]
mod native {
    use super::*;
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        size: u32,
        faults: u32,
        peak_working: usize,
        working: usize,
        peak_paged: usize,
        paged: usize,
        peak_nonpaged: usize,
        nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> *mut c_void;
        fn GetCurrentProcess() -> *mut c_void;
        fn QueryThreadCycleTime(thread: *mut c_void, cycles: *mut u64) -> i32;
        fn GetProcessTimes(
            process: *mut c_void,
            created: *mut FileTime,
            exited: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        fn K32GetProcessMemoryInfo(process: *mut c_void, counters: *mut Counters, size: u32)
        -> i32;
    }

    pub fn cycles() -> io::Result<u64> {
        let mut value = 0;
        // The pseudo-handle and writable counter belong to the calling thread.
        if unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut value) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(value)
    }

    pub fn process_ns() -> io::Result<u64> {
        let (mut created, mut exited, mut kernel, mut user) = (
            FileTime::default(),
            FileTime::default(),
            FileTime::default(),
            FileTime::default(),
        );
        // Four owned C-layout FILETIME buffers match the Windows ABI.
        if unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let ticks = |t: FileTime| u64::from(t.high) << 32 | u64::from(t.low);
        Ok((ticks(kernel) + ticks(user)) * 100)
    }

    pub fn peaks() -> io::Result<(usize, usize)> {
        let mut counters = Counters {
            size: size_of::<Counters>() as u32,
            ..Counters::default()
        };
        // The current process and the complete writable C-layout counters are valid.
        if unsafe {
            K32GetProcessMemoryInfo(
                GetCurrentProcess(),
                &mut counters,
                size_of::<Counters>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok((counters.peak_working, counters.peak_pagefile))
    }
}

#[cfg(windows)]
pub use native::*;

#[cfg(not(windows))]
pub fn cycles() -> io::Result<u64> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "QueryThreadCycleTime requires Windows",
    ))
}
#[cfg(not(windows))]
pub fn process_ns() -> io::Result<u64> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "GetProcessTimes requires Windows",
    ))
}
#[cfg(not(windows))]
pub fn peaks() -> io::Result<(usize, usize)> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Process commit counters require Windows",
    ))
}

pub struct Clock {
    wall: Instant,
    stamp: u64,
}

impl Clock {
    pub fn start() -> io::Result<Self> {
        let _ = cycles()?;
        Ok(Self {
            wall: Instant::now(),
            stamp: stamp()?,
        })
    }

    pub fn elapsed(&self) -> io::Result<(f64, f64)> {
        let seconds = self.wall.elapsed().as_secs_f64();
        Ok((seconds, stamp()?.wrapping_sub(self.stamp) as f64 / seconds))
    }
}

fn stamp() -> io::Result<u64> {
    #[cfg(target_arch = "x86_64")]
    {
        // The invariant counter supplies the same cycle calibration used by dashboard.
        Ok(unsafe { std::arch::x86_64::_rdtsc() })
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Cycle calibration requires x86-64",
        ))
    }
}
