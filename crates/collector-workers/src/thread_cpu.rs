use collector_status::status::{sequence, status};
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

#[derive(Clone, Copy)]
pub enum Role {
    Runtime,
    Spool,
    Status,
    SpoolSizing,
    FrameBuilder,
    FramePriority,
    Capture,
    SpoolWriter,
    JournalWriter,
    #[cfg(feature = "worker_sampler")]
    Sampler,
}
const REGISTERED_ROLES: usize = NAMED - 1;
static REGISTERED: [AtomicU32; REGISTERED_ROLES] = [const { AtomicU32::new(0) }; REGISTERED_ROLES];
#[cfg(feature = "worker_sampler")]
static REGISTERED_CREATION: [AtomicU64; REGISTERED_ROLES] =
    [const { AtomicU64::new(0) }; REGISTERED_ROLES];
#[cfg(feature = "worker_sampler")]
static DECODE_CREATION: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "worker_sampler")]
pub struct StopOnThreadExit(std::marker::PhantomData<*mut ()>);

#[cfg(feature = "worker_sampler")]
impl StopOnThreadExit {
    /// # Safety
    /// Only an owned sampled worker may create this guard, on its own thread. Keep it until
    /// that worker exits and keep its sampler control alive through the exit handshake.
    pub unsafe fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}
#[cfg(feature = "worker_sampler")]
impl Drop for StopOnThreadExit {
    fn drop(&mut self) {
        // SAFETY: This owned sampled worker completes the exit handshake before leaving.
        unsafe { crate::worker_sampler_state::stop_before_thread_exit() };
    }
}

static HELPERS: [AtomicU32; MAX_HELPERS] = [const { AtomicU32::new(0) }; MAX_HELPERS];
#[cfg(feature = "worker_sampler")]
static HELPER_CREATION: [AtomicU64; MAX_HELPERS] = [const { AtomicU64::new(0) }; MAX_HELPERS];
use crate::spool_helper::MAX_HELPERS;

/// # Safety
/// Only attach may register the live helper thread for this index. Keep the helper in the
/// worker census and join it through the sampler handshake before releasing the slot.
pub unsafe fn register_helper(index: usize, id: u32) {
    let Some(slot) = HELPERS.get(index) else {
        return;
    };
    #[cfg(feature = "worker_sampler")]
    HELPER_CREATION[index].store(registered_creation_time(id).unwrap_or(0), Ordering::Release);
    slot.store(id, Ordering::Release);
}

#[cfg(feature = "worker_sampler")]
pub fn helpers() -> impl Iterator<Item = u32> {
    HELPERS.iter().map(|id| id.load(Ordering::Acquire))
}

#[cfg(feature = "worker_sampler")]
pub(crate) fn helper_creation(index: usize, id: u32) -> Option<u64> {
    let registered = HELPERS.get(index)?;
    if id == 0 || registered.load(Ordering::Acquire) != id {
        return None;
    }
    let created = HELPER_CREATION[index].load(Ordering::Acquire);
    (created != 0 && registered.load(Ordering::Acquire) == id).then_some(created)
}

/// # Safety
/// Register only the designated worker for this live session and role. If the sampler can target
/// it, the thread must run the sampler exit handshake before exiting.
pub unsafe fn register(role: Role) {
    // SAFETY: This updates the designated role of this runtime's owned worker.
    unsafe { register_id(role, GetCurrentThreadId()) };
}

/// # Safety
/// Register only the designated worker for this live session and role. If the sampler can target
/// it, the thread must run the sampler exit handshake before exiting.
pub unsafe fn register_id(role: Role, id: u32) {
    #[cfg(feature = "worker_sampler")]
    REGISTERED_CREATION[role as usize]
        .store(registered_creation_time(id).unwrap_or(0), Ordering::Release);
    REGISTERED[role as usize].store(id, Ordering::Release);
}

#[cfg(feature = "worker_sampler")]
fn registered_creation_time(id: u32) -> Option<u64> {
    if id == 0 {
        return None;
    }
    let handle = unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, 0, id) };
    if handle.is_null() {
        return None;
    }
    let [mut created, mut exited, mut kernel, mut user] = [ZERO; 4];
    let read =
        unsafe { GetThreadTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0 };
    unsafe { CloseHandle(handle) };
    read.then(|| filetime(created))
        .filter(|created| *created != 0)
}

#[cfg(feature = "worker_sampler")]
/// # Safety
/// Only the active session's decode thread may register its own identity. It must perform the
/// sampler exit handshake before exiting.
pub unsafe fn register_decode() {
    DECODE_CREATION.store(
        registered_creation_time(unsafe { GetCurrentThreadId() }).unwrap_or(0),
        Ordering::Release,
    );
}

#[cfg(feature = "worker_sampler")]
pub(crate) fn registered_creation(role: &str, id: u32) -> Option<u64> {
    if id == 0 {
        return None;
    }
    if role == "decode" {
        if crate::thread_cpu::decode_worker() != id {
            return None;
        }
        let created = DECODE_CREATION.load(Ordering::Acquire);
        return (created != 0 && crate::thread_cpu::decode_worker() == id).then_some(created);
    }
    let slot = match role {
        "runtime" => Role::Runtime,
        "spool" => Role::Spool,
        "status" => Role::Status,
        "spool_sizing" => Role::SpoolSizing,
        "frame_builder" => Role::FrameBuilder,
        "frame_priority" => Role::FramePriority,
        "capture" => Role::Capture,
        "spool_writer" => Role::SpoolWriter,
        "journal_writer" => Role::JournalWriter,
        "sampler" => Role::Sampler,
        _ => return None,
    } as usize;
    let registered = &REGISTERED[slot];
    if registered.load(Ordering::Acquire) != id {
        return None;
    }
    let created = REGISTERED_CREATION[slot].load(Ordering::Acquire);
    (created != 0 && registered.load(Ordering::Acquire) == id).then_some(created)
}

const BASE_NAMED: usize = 10;
#[cfg(not(feature = "worker_sampler"))]
const NAMED: usize = BASE_NAMED;
#[cfg(feature = "worker_sampler")]
const NAMED: usize = BASE_NAMED + 1;
const THREADS: usize = NAMED + MAX_HELPERS + crate::acquisition::MAX_WORKERS;

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
        "cycle_ns": collector_timing::hook_timing::ticks_to_ns(times.cycles, hz),
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
        "cycle_ns": sample.times.and_then(|times| collector_timing::hook_timing::ticks_to_ns(times.cycles, hz)),
        "user_ns": sample.times.map(|times| times.user.saturating_mul(100)),
        "kernel_ns": sample.times.map(|times| times.kernel.saturating_mul(100)),
    })
}

#[cfg(not(feature = "worker_sampler"))]
const SCOPE: &str = "cumulative CPU of each collector thread (runtime, decode, spool, status, the spool's conversion helpers and its sizing thread, the frame-priority reconciler, the capture thread that reads hours beside the game, and acquisition workers) since it started, read when this status was built: cycles from QueryThreadCycleTime (cycle_ns converts them with hook_timing's tsc_hz), user/kernel from GetThreadTimes at scheduler-tick resolution. collector_total sums the available threads; process is the whole game process. Hooked game-thread time inside collector callbacks is in hook_timing, not here. A thread that exited or whose id now names another thread is unavailable";
#[cfg(feature = "worker_sampler")]
const SCOPE: &str = "cumulative CPU of each collector thread (runtime, decode, spool, status, the spool's conversion helpers and its sizing thread, the frame-priority reconciler, the capture thread that reads hours beside the game, acquisition workers, and sampler) since it started, read when this status was built: cycles from QueryThreadCycleTime (cycle_ns converts them with hook_timing's tsc_hz), user/kernel from GetThreadTimes at scheduler-tick resolution. collector_total sums the available threads; process is the whole game process. Hooked game-thread time inside collector callbacks is in hook_timing, not here. A thread that exited or whose id now names another thread is unavailable";

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

macro_rules! named_threads {
    () => {
        [
            (
                "runtime",
                REGISTERED[Role::Runtime as usize].load(Ordering::Acquire),
            ),
            ("decode", crate::thread_cpu::decode_worker()),
            (
                "spool",
                REGISTERED[Role::Spool as usize].load(Ordering::Acquire),
            ),
            (
                "status",
                REGISTERED[Role::Status as usize].load(Ordering::Acquire),
            ),
            (
                "spool_sizing",
                REGISTERED[Role::SpoolSizing as usize].load(Ordering::Acquire),
            ),
            (
                "frame_builder",
                REGISTERED[Role::FrameBuilder as usize].load(Ordering::Acquire),
            ),
            (
                "frame_priority",
                REGISTERED[Role::FramePriority as usize].load(Ordering::Acquire),
            ),
            (
                "capture",
                REGISTERED[Role::Capture as usize].load(Ordering::Acquire),
            ),
            (
                "spool_writer",
                REGISTERED[Role::SpoolWriter as usize].load(Ordering::Acquire),
            ),
            (
                "journal_writer",
                REGISTERED[Role::JournalWriter as usize].load(Ordering::Acquire),
            ),
            #[cfg(feature = "worker_sampler")]
            (
                "sampler",
                REGISTERED[Role::Sampler as usize].load(Ordering::Acquire),
            ),
        ]
    };
}

#[cfg(feature = "worker_sampler")]
pub fn registered() -> impl Iterator<Item = (&'static str, u32)> {
    named_threads!()
        .into_iter()
        .take(BASE_NAMED)
        .filter(|(role, _)| !matches!(*role, "frame_priority" | "frame_builder" | "capture"))
}

pub fn status(acquisition: &[u32]) -> impl serde::Serialize + use<> {
    let named = named_threads!();
    let workers = acquisition
        .iter()
        .take(crate::acquisition::MAX_WORKERS)
        .enumerate()
        .map(|(index, id)| ("acquisition", Some(index), *id));
    let helpers = HELPERS
        .iter()
        .enumerate()
        .map(|(index, id)| ("spool_helper", Some(index), id.load(Ordering::Acquire)));
    let threads = named
        .into_iter()
        .map(|(role, id)| (role, None, id))
        .chain(helpers)
        .chain(workers);
    let mut samples = [None; THREADS];
    for (slot, ((role, index, id), identity)) in samples.iter_mut().zip(threads.zip(&IDENTITIES)) {
        if id != 0 {
            *slot = Some(sample(role, index, id, identity));
        }
    }
    status_from(
        samples,
        read_process(),
        collector_timing::hook_timing::tsc_hz(),
    )
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
        // SAFETY: Only the current session's designated thread registers this role.
        unsafe { register(Role::Runtime) };
        let me = unsafe { GetCurrentThreadId() };
        // SAFETY: This updates the designated role of this runtime's owned worker.
        unsafe { register_id(Role::Capture, me) };
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
        let capture = threads
            .iter()
            .find(|thread| thread["role"] == "capture")
            .unwrap();
        assert_eq!(capture["thread_id"], me);
    }
}

static WORKER_ID: AtomicU32 = AtomicU32::new(0);

pub fn decode_worker() -> u32 {
    WORKER_ID.load(Ordering::Acquire)
}

/// # Safety
/// Only the decode owner's startup and joined detach sequences may set or clear this session's
/// decode thread id.
pub unsafe fn set_decode_worker(id: u32) {
    WORKER_ID.store(id, Ordering::Release);
}
