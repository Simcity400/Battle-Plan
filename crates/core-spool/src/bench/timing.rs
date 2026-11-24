use std::{
    io,
    sync::OnceLock,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub struct Calibration {
    pub hz: f64,
    pub observed_step: u64,
}
static CALIBRATION: OnceLock<Calibration> = OnceLock::new();
pub fn calibrate() -> io::Result<Calibration> {
    if let Some(value) = CALIBRATION.get() {
        return Ok(*value);
    }
    let start = Instant::now();
    let stamp = stamp()?;
    while start.elapsed() < Duration::from_millis(250) {
        std::hint::spin_loop();
    }
    let hz = (stamp_now()? - stamp) as f64 / start.elapsed().as_secs_f64();
    let mut observed_step = u64::MAX;
    let mut previous = cycles()?;
    for _ in 0..1024 {
        let next = cycles()?;
        let delta = next.saturating_sub(previous);
        if delta != 0 {
            observed_step = observed_step.min(delta);
        }
        previous = next;
    }
    if observed_step == u64::MAX {
        return Err(io::Error::other("thread cycle counter did not advance"));
    }
    let result = Calibration { hz, observed_step };
    let _ = CALIBRATION.set(result);
    Ok(result)
}
fn stamp() -> io::Result<u64> {
    stamp_now()
}
#[cfg(target_arch = "x86_64")]
fn stamp_now() -> io::Result<u64> {
    Ok(unsafe { std::arch::x86_64::_rdtsc() })
}
#[cfg(not(target_arch = "x86_64"))]
fn stamp_now() -> io::Result<u64> {
    Err(io::Error::other("cycle calibration requires x86-64"))
}

pub struct Clock {
    wall: Instant,
    cpu: u64,
    hz: f64,
}
#[derive(Default, Clone, Copy)]
pub struct Time {
    pub wall_ns: u64,
    pub cpu_ns: u64,
    pub cycles: u64,
}
impl Time {
    pub fn add(&mut self, other: Self) {
        self.wall_ns += other.wall_ns;
        self.cpu_ns += other.cpu_ns;
        self.cycles += other.cycles;
    }
}
impl Clock {
    pub fn start() -> io::Result<Self> {
        let hz = calibrate()?.hz;
        Ok(Self {
            wall: Instant::now(),
            cpu: cycles()?,
            hz,
        })
    }
    pub fn stop(self) -> io::Result<Time> {
        let cycles = cycles()?.saturating_sub(self.cpu);
        Ok(Time {
            wall_ns: u64::try_from(self.wall.elapsed().as_nanos()).map_err(io::Error::other)?,
            cpu_ns: (cycles as f64 * 1e9 / self.hz) as u64,
            cycles,
        })
    }
}
#[cfg(windows)]
fn cycles() -> io::Result<u64> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> *mut std::ffi::c_void;
        fn QueryThreadCycleTime(thread: *mut std::ffi::c_void, cycles: *mut u64) -> i32;
    }
    let mut value = 0;
    if unsafe { QueryThreadCycleTime(GetCurrentThread(), &raw mut value) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(value)
}
#[cfg(not(windows))]
fn cycles() -> io::Result<u64> {
    Err(io::Error::other(
        "core-spool-bench requires QueryThreadCycleTime on Windows",
    ))
}
