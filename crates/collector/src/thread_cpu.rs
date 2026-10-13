//! CPU time of the collector's own threads, read when status is built. The
//! cumulative counters let a status diff measure contention with the game.
use crate::status::{sequence, status};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use windows_sys::Win32::{
    Foundation::{CloseHandle, FILETIME, HANDLE},
    System::Threading::{
        GetCurrentProcess, GetCurrentThreadId, GetProcessTimes, GetThreadTimes, OpenThread,
        THREAD_QUERY_LIMITED_INFORMATION,
    },
};

unsafe extern "system" {
    fn QueryThreadCycleTime(thread: HANDLE, cycles: *mut u64) -> i32;
    fn QueryProcessCycleTime(process: HANDLE, cycles: *mut u64) -> i32;
}

/// Collector threads that register themselves; decode and acquisition
/// workers already publish their ids.
#[derive(Clone, Copy)]
pub(crate) enum Role {
    Runtime,
    Spool,
    Status,
    SpoolHelper,
    SecondSpoolHelper,
    SpoolSizing,
}
static REGISTERED: [AtomicU32; 6] = [const { AtomicU32::new(0) }; 6];

/// Call first on the registering thread.
pub(crate) fn register(role: Role) {
    register_id(role, unsafe { GetCurrentThreadId() });
}

/// Register a thread that published its id to its owner.
pub(crate) fn register_id(role: Role, id: u32) {
    REGISTERED[role as usize].store(id, Ordering::Release);
}

const NAMED: usize = 7;
const THREADS: usize = NAMED + crate::acquisition::MAX_WORKERS;

/// The creation time first seen for an id; a later thread reusing that id
/// after ours exited is not reported as ours.
struct Identity {
    id: AtomicU32,
    created: AtomicU64,
}
static IDENTITIES: [Identity; THREADS] = [const {
    Identity {
        id: AtomicU32::new(0),
        created: AtomicU64::new(0),
    }
}; THREADS];

impl Identity {
    fn admit(&self, id: u32, created: u64) -> bool {
        if self.id.load(Ordering::Acquire) != id {
            self.created.store(created, Ordering::Release);
            self.id.store(id, Ordering::Release);
            return true;
        }
        self.created.load(Ordering::Acquire) == created
    }
}

/// Cycles from QueryThreadCycleTime; user and kernel in 100 ns units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Times {
    cycles: u64,
    user: u64,
    kernel: u64,
}
impl Times {
    fn add(self, other: Self) -> Self {
        Self {
            cycles: self.cycles.saturating_add(other.cycles),
            user: self.user.saturating_add(other.user),
            kernel: self.kernel.saturating_add(other.kernel),
        }
    }
}

#[derive(Clone, Copy)]
struct Sample {
    role: &'static str,
    index: Option<usize>,
    id: u32,
    times: Option<Times>,
}

fn filetime(value: FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}
const ZERO: FILETIME = FILETIME {
    dwLowDateTime: 0,
    dwHighDateTime: 0,
};

/// Creation time and counters of a live or retained thread object.
fn read_thread(id: u32) -> Option<(u64, Times)> {
    if id == 0 {
        return None;
    }
    let handle = unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, 0, id) };
    if handle.is_null() {
        return None;
    }
    let [mut created, mut exited, mut kernel, mut user] = [ZERO; 4];
    let mut cycles = 0;
    let read = unsafe {
        GetThreadTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0
            && QueryThreadCycleTime(handle, &mut cycles) != 0
    };
    unsafe { CloseHandle(handle) };
    read.then(|| {
        (
            filetime(created),
            Times {
                cycles,
                user: filetime(user),
                kernel: filetime(kernel),
            },
        )
    })
}

fn read_process() -> Option<Times> {
    let process = unsafe { GetCurrentProcess() };
    let [mut created, mut exited, mut kernel, mut user] = [ZERO; 4];
    let mut cycles = 0;
    let read = unsafe {
        GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) != 0
            && QueryProcessCycleTime(process, &mut cycles) != 0
    };
    read.then(|| Times {
        cycles,
        user: filetime(user),
        kernel: filetime(kernel),
    })
}

fn sample(role: &'static str, index: Option<usize>, id: u32, identity: &Identity) -> Sample {
    let times = read_thread(id)
        .filter(|(created, _)| identity.admit(id, *created))
        .map(|(_, times)| times);
    Sample {
        role,
        index,
        id,
        times,
    }
}

fn times_status(times: Times, hz: Option<u64>) -> impl serde::Serialize {
    status!({
        "cycles": times.cycles,
        "cycle_ns": crate::hook_timing::ticks_to_ns(times.cycles, hz),
        "user_ns": times.user.saturating_mul(100),
        "kernel_ns": times.kernel.saturating_mul(100),
    })
}

fn sample_status(sample: Sample, hz: Option<u64>) -> impl serde::Serialize {
    status!({
        "role": sample.role,
        "index": sample.index,
        "thread_id": sample.id,
        "available": sample.times.is_some(),
        "cycles": sample.times.map(|times| times.cycles),
        "cycle_ns": sample.times.and_then(|times| crate::hook_timing::ticks_to_ns(times.cycles, hz)),
        "user_ns": sample.times.map(|times| times.user.saturating_mul(100)),
        "kernel_ns": sample.times.map(|times| times.kernel.saturating_mul(100)),
    })
}

const SCOPE: &str = "cumulative CPU of each collector thread (runtime, decode, spool, status, the spool's two conversion helpers and its sizing thread, and acquisition workers) since it started, read when this status was built: cycles from QueryThreadCycleTime (cycle_ns converts them with hook_timing's tsc_hz), user/kernel from GetThreadTimes at scheduler-tick resolution. collector_total sums the available threads; process is the whole game process. Hooked game-thread time inside collector callbacks is in hook_timing, not here. A thread that exited or whose id now names another thread is unavailable";

fn status_from(
    samples: [Option<Sample>; THREADS],
    process: Option<Times>,
    hz: Option<u64>,
) -> impl serde::Serialize {
    let total = samples
        .iter()
        .flatten()
        .filter_map(|sample| sample.times)
        .fold(Times::default(), Times::add);
    status!({
        "tsc_hz": hz,
        "threads": sequence(samples.into_iter().flatten().map(move |sample| sample_status(sample, hz))),
        "collector_total": times_status(total, hz),
        "process": process.map(|process| times_status(process, hz)),
        "scope": SCOPE,
    })
}

/// status.json `collector_thread_cpu`. `acquisition` lists the worker ids.
pub(crate) fn status(acquisition: &[u32]) -> impl serde::Serialize + use<> {
    let named = [
        (
            "runtime",
            REGISTERED[Role::Runtime as usize].load(Ordering::Acquire),
        ),
        ("decode", crate::decode::worker_id()),
        (
            "spool",
            REGISTERED[Role::Spool as usize].load(Ordering::Acquire),
        ),
        (
            "status",
            REGISTERED[Role::Status as usize].load(Ordering::Acquire),
        ),
        (
            "spool_helper",
            REGISTERED[Role::SpoolHelper as usize].load(Ordering::Acquire),
        ),
        (
            "second_spool_helper",
            REGISTERED[Role::SecondSpoolHelper as usize].load(Ordering::Acquire),
        ),
        (
            "spool_sizing",
            REGISTERED[Role::SpoolSizing as usize].load(Ordering::Acquire),
        ),
    ];
    let workers = acquisition
        .iter()
        .take(crate::acquisition::MAX_WORKERS)
        .enumerate()
        .map(|(index, id)| ("acquisition", Some(index), *id));
    let threads = named
        .into_iter()
        .map(|(role, id)| (role, None, id))
        .chain(workers);
    let mut samples = [None; THREADS];
    for (slot, ((role, index, id), identity)) in samples.iter_mut().zip(threads.zip(&IDENTITIES)) {
        if id != 0 {
            *slot = Some(sample(role, index, id, identity));
        }
    }
    status_from(samples, read_process(), crate::hook_timing::tsc_hz())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn a_reused_thread_id_is_not_reported_as_the_original_thread() {
        let identity = Identity {
            id: AtomicU32::new(0),
            created: AtomicU64::new(0),
        };
        assert!(identity.admit(41, 1000));
        assert!(identity.admit(41, 1000));
        assert!(!identity.admit(41, 2000));
        // A newly registered id replaces the old identity.
        assert!(identity.admit(42, 3000));
        assert!(identity.admit(42, 3000));
    }

    #[test]
    fn the_current_thread_reports_cycles_and_a_missing_thread_is_unavailable() {
        let begin = read_thread(unsafe { GetCurrentThreadId() }).unwrap();
        let mut value = 0_u64;
        for index in 0..5_000_000_u64 {
            value = std::hint::black_box(value.wrapping_mul(31).wrapping_add(index));
        }
        let end = read_thread(unsafe { GetCurrentThreadId() }).unwrap();
        assert_eq!(begin.0, end.0);
        assert!(end.1.cycles > begin.1.cycles);
        assert_eq!(read_thread(0), None);
        assert!(read_process().unwrap().cycles >= end.1.cycles);
    }

    #[test]
    fn status_lists_each_thread_and_sums_the_available_ones() {
        let mut samples = [None; THREADS];
        samples[0] = Some(Sample {
            role: "decode",
            index: None,
            id: 7,
            times: Some(Times {
                cycles: 3_000,
                user: 20,
                kernel: 5,
            }),
        });
        samples[1] = Some(Sample {
            role: "acquisition",
            index: Some(3),
            id: 9,
            times: Some(Times {
                cycles: 6_000,
                user: 10,
                kernel: 0,
            }),
        });
        samples[2] = Some(Sample {
            role: "spool",
            index: None,
            id: 11,
            times: None,
        });
        let process = Times {
            cycles: 90_000,
            user: 1_000,
            kernel: 100,
        };
        let status: Value =
            serde_json::to_value(status_from(samples, Some(process), Some(3_000_000_000))).unwrap();
        assert_eq!(
            status["threads"],
            json!([
                {"role": "decode", "index": null, "thread_id": 7, "available": true, "cycles": 3000,
                 "cycle_ns": 1000, "user_ns": 2000, "kernel_ns": 500},
                {"role": "acquisition", "index": 3, "thread_id": 9, "available": true, "cycles": 6000,
                 "cycle_ns": 2000, "user_ns": 1000, "kernel_ns": 0},
                {"role": "spool", "index": null, "thread_id": 11, "available": false, "cycles": null,
                 "cycle_ns": null, "user_ns": null, "kernel_ns": null},
            ])
        );
        assert_eq!(
            status["collector_total"],
            json!({"cycles": 9000, "cycle_ns": 3000, "user_ns": 3000, "kernel_ns": 500})
        );
        assert_eq!(status["process"]["cycles"], 90_000);
        assert_eq!(status["process"]["user_ns"], 100_000);
        assert_eq!(status["tsc_hz"], 3_000_000_000_u64);
    }

    #[test]
    fn registered_threads_appear_in_live_status() {
        register(Role::Runtime);
        let me = unsafe { GetCurrentThreadId() };
        let status: Value = serde_json::to_value(status(&[me])).unwrap();
        let threads = status["threads"].as_array().unwrap();
        let worker = threads
            .iter()
            .find(|thread| thread["role"] == "acquisition")
            .unwrap();
        assert_eq!(worker["thread_id"], me);
        assert_eq!(worker["index"], 0);
        assert_eq!(worker["available"], true);
        assert!(worker["cycles"].as_u64().unwrap() > 0);
        assert!(threads.iter().any(|thread| thread["role"] == "runtime"));
    }
}
