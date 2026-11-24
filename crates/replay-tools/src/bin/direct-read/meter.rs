use serde_json::{Value, json};
use std::time::Instant;

#[derive(Clone, Copy, Default)]
struct Counters {
    cpu_ns: u64,
    read_bytes: u64,
    read_calls: u64,
}

#[cfg(windows)]
fn counters() -> Counters {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentProcess, GetProcessIoCounters, GetProcessTimes, IO_COUNTERS},
    };
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    let mut io = IO_COUNTERS {
        ReadOperationCount: 0,
        WriteOperationCount: 0,
        OtherOperationCount: 0,
        ReadTransferCount: 0,
        WriteTransferCount: 0,
        OtherTransferCount: 0,
    };
    unsafe {
        GetProcessTimes(GetCurrentProcess(), &raw mut created, &raw mut exited, &raw mut kernel, &raw mut user);
        GetProcessIoCounters(GetCurrentProcess(), &raw mut io);
    }
    let ticks = |time: FILETIME| u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime);
    Counters {
        cpu_ns: (ticks(kernel) + ticks(user)) * 100,
        read_bytes: io.ReadTransferCount,
        read_calls: io.ReadOperationCount,
    }
}

#[cfg(not(windows))]
fn counters() -> Counters {
    Counters::default()
}

pub struct Meter {
    wall: Instant,
    before: Counters,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Spent {
    pub wall_ms: f64,
    pub cpu_ms: f64,
    pub read_bytes: u64,
    pub read_calls: u64,
}

impl Meter {
    pub fn start() -> Self {
        Self {
            before: counters(),
            wall: Instant::now(),
        }
    }

    pub fn stop(&self) -> Spent {
        let wall_ms = self.wall.elapsed().as_secs_f64() * 1e3;
        let after = counters();
        Spent {
            wall_ms,
            cpu_ms: (after.cpu_ns - self.before.cpu_ns) as f64 / 1e6,
            read_bytes: after.read_bytes - self.before.read_bytes,
            read_calls: after.read_calls - self.before.read_calls,
        }
    }
}

impl Spent {
    pub fn json(&self) -> Value {
        json!({
            "wall_ms": round(self.wall_ms),
            "cpu_ms": round(self.cpu_ms),
            "read_bytes": self.read_bytes,
            "read_calls": self.read_calls,
        })
    }
}

pub fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

pub fn spread(values: &mut [f64]) -> Value {
    values.sort_by(f64::total_cmp);
    let at = |quantile: f64| {
        let index = ((values.len() as f64 * quantile).ceil() as usize).clamp(1, values.len()) - 1;
        round(values[index])
    };
    if values.is_empty() {
        return Value::Null;
    }
    json!({ "p50": at(0.5), "p95": at(0.95), "max": at(1.0), "min": round(values[0]) })
}
