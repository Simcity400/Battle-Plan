mod format;
#[cfg(test)]
mod tests;

use crate::worker_sampler_state::{
    self, Control, EARLY_EXIT, EMERGENCY_EXIT, JobGuard, SAMPLER_CONTROL, SampledJob, creation,
};
use collector_status::status::status;
use serde::{Serialize, Serializer};
use std::{
    io,
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, STATUS_WAIT_0, WAIT_TIMEOUT},
    System::{
        Diagnostics::Debug::{CONTEXT, CONTEXT_CONTROL_AMD64, GetThreadContext},
        Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
        Threading::{
            CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, GetCurrentThread,
            GetCurrentThreadId, OpenThread, ResumeThread, SetThreadPriority, SetWaitableTimer,
            Sleep, SuspendThread, SwitchToThread, THREAD_GET_CONTEXT, THREAD_PRIORITY_HIGHEST,
            THREAD_QUERY_INFORMATION, THREAD_SUSPEND_RESUME, THREAD_SYNCHRONIZE, TIMER_ALL_ACCESS,
            WaitForSingleObject,
        },
    },
};

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtDelayExecution(alertable: u8, interval: *const i64) -> i32;
    fn NtWaitForSingleObject(handle: HANDLE, alertable: u8, timeout: *const i64) -> i32;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn QueryThreadCycleTime(thread: HANDLE, cycles: *mut u64) -> i32;
}

const DEFAULT_INTERVAL_US: u32 = 250;
const CHUNK_RECORDS: usize = 64 * 1024;
const EXIT_SPIN_US: u64 = 25;
const ROLE_NAMES: [&str; 10] = [
    "runtime",
    "decode",
    "spool",
    "status",
    "spool_helper",
    "second_spool_helper",
    "spool_sizing",
    "acquisition",
    "game",
    "sampler",
];
const GAME_ROLE: u8 = 8;
const HELPER_ROLE: u8 = 4;
static SAMPLER_ID: AtomicU32 = AtomicU32::new(0);
static GAME_ID: AtomicU32 = AtomicU32::new(0);
static GAME_CREATION: AtomicU64 = AtomicU64::new(0);
static GAME_JOB: AtomicU16 = AtomicU16::new(0);
static GAME_GATE: GameGate = GameGate::new();
/// # Safety
/// Only the live admitted callback or worker for this session may publish sampler observation
/// state. Drop the guard before leaving the observation scope on the originating thread.
pub unsafe fn job(role: crate::roles::Role) -> JobGuard {
    // SAFETY: This is the live admitted worker recording the role for its own job.
    unsafe {
        worker_sampler_state::job(role as u16 + 1, || {
            (GetCurrentThreadId() == GAME_ID.load(Ordering::Acquire)).then_some(&GAME_JOB)
        })
    }
}

struct GameGate {
    in_callback: AtomicBool,
    touching: AtomicBool,
    spins: AtomicU64,
    yields: AtomicU64,
}
impl GameGate {
    const fn new() -> Self {
        Self {
            in_callback: AtomicBool::new(false),
            touching: AtomicBool::new(false),
            spins: AtomicU64::new(0),
            yields: AtomicU64::new(0),
        }
    }
    fn leave(&self) {
        let spin_ticks = (load(&COUNTERS.qpc_hz) / (1_000_000 / EXIT_SPIN_US)).max(1);
        self.leave_with(spin_ticks, qpc, || unsafe {
            SwitchToThread();
        });
    }
    fn leave_with(
        &self,
        spin_ticks: u64,
        mut now: impl FnMut() -> u64,
        mut yield_thread: impl FnMut(),
    ) {
        self.in_callback.store(false, Ordering::SeqCst);
        let mut spins = 0_u64;
        let mut yields = 0_u64;
        if self.touching.load(Ordering::SeqCst) {
            let begin = now();
            while self.touching.load(Ordering::SeqCst) && now().saturating_sub(begin) < spin_ticks {
                std::hint::spin_loop();
                spins = spins.saturating_add(1);
            }
            while self.touching.load(Ordering::SeqCst) {
                yield_thread();
                yields = yields.saturating_add(1);
            }
        }
        self.spins.fetch_add(spins, Ordering::Relaxed);
        self.yields.fetch_add(yields, Ordering::Relaxed);
    }
    fn claim(&self) -> bool {
        self.touching.store(true, Ordering::SeqCst);
        if self.in_callback.load(Ordering::SeqCst) {
            true
        } else {
            self.touching.store(false, Ordering::SeqCst);
            false
        }
    }
}
pub struct CallbackGuard {
    entered: bool,
    _thread: std::marker::PhantomData<*mut ()>,
}
/// # Safety
/// Only the live admitted callback or worker for this session may publish sampler observation
/// state. Drop the guard before leaving the observation scope on the originating thread.
pub unsafe fn callback() -> CallbackGuard {
    let id = unsafe { GetCurrentThreadId() };
    if GAME_ID
        .compare_exchange(0, id, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        GAME_CREATION.store(
            creation(unsafe { GetCurrentThread() }).unwrap_or(0),
            Ordering::Release,
        );
    }
    let entered = GAME_ID.load(Ordering::Acquire) == id;
    if entered {
        GAME_GATE.in_callback.store(true, Ordering::SeqCst);
    }
    CallbackGuard {
        entered,
        _thread: std::marker::PhantomData,
    }
}
impl Drop for CallbackGuard {
    fn drop(&mut self) {
        if self.entered {
            GAME_GATE.leave();
        }
    }
}

#[derive(Debug)]
struct Settings {
    enabled: bool,
    interval_us: u32,
    output: PathBuf,
}
impl Settings {
    fn parse(value: Option<&str>, output_dir: &Path) -> Result<Self, String> {
        let mut settings = Self {
            enabled: true,
            interval_us: DEFAULT_INTERVAL_US,
            output: output_dir.join("worker-samples.bin"),
        };
        for part in value.unwrap_or("").split(',').filter(|p| !p.is_empty()) {
            if part == "off" {
                settings.enabled = false;
            } else if let Some(v) = part.strip_prefix("interval_us=") {
                settings.interval_us =
                    v.parse::<u32>().ok().filter(|v| *v != 0).ok_or_else(|| {
                        "worker_sampler interval_us must be a nonzero u32".to_owned()
                    })?;
            } else {
                return Err(format!("worker_sampler unsupported option: {part}"));
            }
        }
        Ok(settings)
    }
}

struct Counters {
    enabled: AtomicBool,
    interval_us: AtomicU32,
    timer: AtomicU32,
    qpc_hz: AtomicU64,
    rounds: AtomicU64,
    interval_sum: AtomicU64,
    interval_max: AtomicU64,
    written: AtomicU64,
    dropped: AtomicU64,
    idle: AtomicU64,
    open_failed: AtomicU64,
    capture_failed: AtomicU64,
    write_failed: AtomicU64,
    backpressure: AtomicU64,
    buffer_full: AtomicU64,
    game_exit_races: AtomicU64,
    suspension_count: AtomicU64,
    suspension_sum: AtomicU64,
    suspension_max: AtomicU64,
    resume_retries: AtomicU64,
}
static COUNTERS: Counters = Counters {
    enabled: AtomicBool::new(false),
    interval_us: AtomicU32::new(DEFAULT_INTERVAL_US),
    timer: AtomicU32::new(0),
    qpc_hz: AtomicU64::new(0),
    rounds: AtomicU64::new(0),
    interval_sum: AtomicU64::new(0),
    interval_max: AtomicU64::new(0),
    written: AtomicU64::new(0),
    dropped: AtomicU64::new(0),
    idle: AtomicU64::new(0),
    open_failed: AtomicU64::new(0),
    capture_failed: AtomicU64::new(0),
    write_failed: AtomicU64::new(0),
    backpressure: AtomicU64::new(0),
    buffer_full: AtomicU64::new(0),
    game_exit_races: AtomicU64::new(0),
    suspension_count: AtomicU64::new(0),
    suspension_sum: AtomicU64::new(0),
    suspension_max: AtomicU64::new(0),
    resume_retries: AtomicU64::new(0),
};
#[derive(Default)]
struct Metadata {
    path: String,
    stop_error: Option<String>,
}
static META: Mutex<Metadata> = Mutex::new(Metadata {
    path: String::new(),
    stop_error: None,
});
fn load(v: &AtomicU64) -> u64 {
    v.load(Ordering::Relaxed)
}
fn ticks_ns(ticks: u64) -> u64 {
    let hz = load(&COUNTERS.qpc_hz);
    if hz == 0 {
        0
    } else {
        ((u128::from(ticks) * 1_000_000_000) / u128::from(hz)).min(u128::from(u64::MAX)) as u64
    }
}
fn reset(settings: &Settings) {
    SAMPLER_ID.store(0, Ordering::Release);
    EMERGENCY_EXIT.store(false, Ordering::Relaxed);
    COUNTERS.enabled.store(settings.enabled, Ordering::Relaxed);
    COUNTERS
        .interval_us
        .store(settings.interval_us, Ordering::Relaxed);
    COUNTERS.timer.store(0, Ordering::Relaxed);
    for counter in [
        &COUNTERS.rounds,
        &COUNTERS.interval_sum,
        &COUNTERS.interval_max,
        &COUNTERS.written,
        &COUNTERS.dropped,
        &COUNTERS.idle,
        &COUNTERS.open_failed,
        &COUNTERS.capture_failed,
        &COUNTERS.write_failed,
        &COUNTERS.backpressure,
        &COUNTERS.buffer_full,
        &COUNTERS.game_exit_races,
        &COUNTERS.suspension_count,
        &COUNTERS.suspension_sum,
        &COUNTERS.suspension_max,
        &COUNTERS.resume_retries,
    ] {
        counter.store(0, Ordering::Relaxed);
    }
    GAME_GATE.spins.store(0, Ordering::Relaxed);
    GAME_GATE.yields.store(0, Ordering::Relaxed);
    *META.lock().unwrap_or_else(|e| e.into_inner()) = Metadata {
        path: settings.output.to_string_lossy().into_owned(),
        stop_error: None,
    };
}
pub struct Status;
pub fn status() -> Status {
    Status
}
impl Serialize for Status {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let metadata = META.lock().unwrap_or_else(|e| e.into_inner());
        let rounds = load(&COUNTERS.rounds);
        let windows = load(&COUNTERS.suspension_count);
        status!({
            "enabled": COUNTERS.enabled.load(Ordering::Relaxed),
            "interval_us": COUNTERS.interval_us.load(Ordering::Relaxed),
            "timer_kind": match COUNTERS.timer.load(Ordering::Relaxed) { 1 => "high_resolution_waitable_timer", 2 => "sleep_1ms", _ => "not_started" },
            "rounds": rounds,
            "mean_round_interval_ns": ticks_ns(load(&COUNTERS.interval_sum) / rounds.saturating_sub(1).max(1)),
            "max_round_interval_ns": ticks_ns(load(&COUNTERS.interval_max)),
            "samples_written": load(&COUNTERS.written),
            "dropped_samples": load(&COUNTERS.dropped),
            "reason_counts": { "open_thread": load(&COUNTERS.open_failed), "capture": load(&COUNTERS.capture_failed), "write": load(&COUNTERS.write_failed), "writer_backpressure": load(&COUNTERS.backpressure), "buffer_full":load(&COUNTERS.buffer_full), "game_exit_race": load(&COUNTERS.game_exit_races), },
            "idle_skips": load(&COUNTERS.idle),
            "handshake_spins": load(&GAME_GATE.spins),
            "handshake_yields": load(&GAME_GATE.yields),
            "suspension": { "count":windows, "mean_ns":ticks_ns(load(&COUNTERS.suspension_sum) / windows.max(1)), "max_ns":ticks_ns(load(&COUNTERS.suspension_max)),
                "resume_retries":load(&COUNTERS.resume_retries), },
            "output_path": &metadata.path, "stop_error":&metadata.stop_error,
            "scope":"Diagnostic CPU-gated point samples since attach; cycle weights cover each thread's previous check, not only the sampled function. Writer backpressure counts missed thread-sampling opportunities. Game exit races remain inside the callback exit wait and are discarded. Samples_written counts flushed records.",
        }).serialize(serializer)
    }
}

fn record_stop_panic() {
    META.lock().unwrap_or_else(|e| e.into_inner()).stop_error =
        Some("worker_sampler thread panicked".into());
}
pub struct Sampler {
    control: Arc<Control>,
}
/// # Safety
/// Only the runtime attach sequence may prepare the sampler, before any callback or worker can
/// publish sampler state.
pub unsafe fn prepare() {
    EARLY_EXIT.store(false, Ordering::Release);
    *SAMPLER_CONTROL.lock().unwrap_or_else(|e| e.into_inner()) = Weak::new();
}
/// # Safety
/// Only the detaching runtime may release sampler state, after stopping the sampler and joining all
/// its worker targets.
pub unsafe fn release() {
    *SAMPLER_CONTROL.lock().unwrap_or_else(|e| e.into_inner()) = Weak::new();
    worker_sampler_state::clear_jobs();
    *META.lock().unwrap_or_else(|e| e.into_inner()) = Metadata::default();
}
pub fn thread_id() -> u32 {
    SAMPLER_ID.load(Ordering::Acquire)
}
/// # Safety
/// Only the runtime may start the sampler, once after preparation, and must stop and join it before
/// releasing its targets or callback storage.
pub unsafe fn start(
    config: Option<&str>,
    output_dir: &Path,
    acquisition_ids: Vec<u32>,
) -> Result<Sampler, String> {
    let settings = Settings::parse(config, output_dir)?;
    reset(&settings);
    if settings.enabled && EARLY_EXIT.load(Ordering::Acquire) {
        return Err("collector thread exited before worker_sampler startup".into());
    }
    let stop = Arc::new(AtomicBool::new(false));
    let active = Arc::new(AtomicBool::new(false));
    let control = Arc::new(Control::new(
        Arc::clone(&stop),
        Arc::clone(&active),
        settings.enabled,
        record_stop_panic,
    ));
    if !settings.enabled {
        return Ok(Sampler { control });
    }
    let scratch = Scratch::default();
    let chunk = format::Chunk::new(CHUNK_RECORDS)
        .map_err(|e| format!("worker_sampler record buffer: {e}"))?;
    let mut frequency = 0;
    if unsafe { QueryPerformanceFrequency(&raw mut frequency) } == 0 || frequency <= 0 {
        return Err("worker_sampler QueryPerformanceFrequency failed".into());
    }
    COUNTERS.qpc_hz.store(frequency as u64, Ordering::Relaxed);
    let header = format::Header::current(settings.interval_us, frequency as u64)?;
    let writer = format::Writer::create(&settings.output, &header)
        .map_err(|e| format!("worker_sampler output: {e}"))?;
    let (thread_stop, thread_active) = (Arc::clone(&stop), Arc::clone(&active));
    *SAMPLER_CONTROL.lock().unwrap_or_else(|e| e.into_inner()) = Arc::downgrade(&control);
    let handle = thread::Builder::new()
        .name("worker-sampler".into())
        .spawn(move || {
            SAMPLER_ID.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
            // SAFETY: Only the current session's designated thread registers this role.
            unsafe { crate::thread_cpu::register(crate::thread_cpu::Role::Sampler) };
            unsafe {
                SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
            }
            run(
                settings,
                acquisition_ids,
                thread_stop,
                thread_active,
                scratch,
                chunk,
                writer,
            );
        });
    let handle = match handle {
        Ok(handle) => handle,
        Err(error) => {
            control.join_complete.store(true, Ordering::Release);
            control.ready.store(true, Ordering::Release);
            return Err(format!("worker_sampler thread: {error}"));
        }
    };
    *control.thread.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
    control.ready.store(true, Ordering::Release);
    while thread_id() == 0 && !control.join_complete.load(Ordering::Acquire) {
        thread::yield_now();
    }
    Ok(Sampler { control })
}
impl Sampler {
    /// # Safety
    /// Only the runtime may activate this sampler, after the session's workers and callback state
    /// are ready and before detach.
    pub unsafe fn activate(&self) -> Result<(), &'static str> {
        if COUNTERS.enabled.load(Ordering::Relaxed) && EARLY_EXIT.load(Ordering::Acquire) {
            EMERGENCY_EXIT.store(true, Ordering::Release);
            self.control.stop_join();
            return Err("collector thread exited before worker_sampler activation");
        }
        self.control.active.store(true, Ordering::Release);
        Ok(())
    }
    /// # Safety
    /// Only the owning runtime may stop this sampler. Complete the join before releasing callback
    /// storage or letting sampled workers exit.
    pub unsafe fn stop(&mut self) -> u64 {
        self.control.stop_join();
        load(&COUNTERS.written)
    }
}
impl Drop for Sampler {
    fn drop(&mut self) {
        // SAFETY: This is the owning runtime's shutdown, while the worker's shared state is still
        // live.
        unsafe { self.stop() };
    }
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Target {
    handle: Handle,
    info: format::ThreadInfo,
    role_id: u8,
    last_cycles: u64,
    job: Option<SampledJob>,
}
impl Target {
    fn open(id: u32, role: u8, worker: u8, slot: u16) -> Option<Self> {
        if id == 0 || id == unsafe { GetCurrentThreadId() } {
            return None;
        }
        let handle = Handle(unsafe {
            OpenThread(
                THREAD_SUSPEND_RESUME
                    | THREAD_GET_CONTEXT
                    | THREAD_QUERY_INFORMATION
                    | THREAD_SYNCHRONIZE,
                0,
                id,
            )
        });
        if handle.0.is_null() {
            return None;
        }
        let mut cycles = 0;
        if unsafe { QueryThreadCycleTime(handle.0, &raw mut cycles) } == 0 {
            return None;
        }
        let registered = worker_sampler_state::registered_job(id);
        let expected = if role == GAME_ROLE {
            GAME_CREATION.load(Ordering::Acquire)
        } else if role == 7 {
            registered.as_ref().map_or(0, |(created, _)| *created)
        } else if role == HELPER_ROLE {
            crate::thread_cpu::helper_creation(usize::from(worker), id).unwrap_or(0)
        } else {
            crate::thread_cpu::registered_creation(ROLE_NAMES[role as usize], id).unwrap_or(0)
        };
        #[cfg(not(test))]
        if expected == 0 || creation(handle.0) != Some(expected) {
            return None;
        }
        #[cfg(test)]
        if expected != 0 && creation(handle.0) != Some(expected) {
            return None;
        }
        let job = registered.map(|(_, job)| job);
        Some(Self {
            handle,
            info: format::ThreadInfo {
                slot,
                thread_id: id,
                role: ROLE_NAMES[role as usize].into(),
                worker_index: (worker != 255).then_some(worker),
            },
            role_id: role,
            last_cycles: cycles,
            job,
        })
    }
    fn role(&self) -> u8 {
        self.role_id
    }
}
#[repr(align(16))]
#[derive(Default)]
struct AlignedContext(CONTEXT);
#[derive(Default)]
struct Scratch {
    context: AlignedContext,
    stamp: i64,
    end: i64,
    job: u16,
    in_callback: bool,
    suspended: bool,
}

#[inline(never)]
fn capture_window(target: &Target, scratch: &mut Scratch) -> bool {
    scratch.suspended = false;
    let handle = target.handle.0;
    let retries = &COUNTERS.resume_retries;
    let game = target.role() == GAME_ROLE;
    scratch.context.0.ContextFlags = CONTEXT_CONTROL_AMD64;
    unsafe {
        QueryPerformanceCounter(&raw mut scratch.stamp);
    }
    if unsafe { SuspendThread(handle) } == u32::MAX {
        return false;
    }
    scratch.suspended = true;
    let mut ok = unsafe { GetThreadContext(handle, &raw mut scratch.context.0) } != 0;
    if ok {
        scratch.job = if game {
            GAME_JOB.load(Ordering::Acquire)
        } else {
            target.job.as_ref().map_or(0, SampledJob::id)
        };
        scratch.in_callback = GAME_GATE.in_callback.load(Ordering::SeqCst);
    }
    if !resume_held(handle, retries) {
        ok = false;
    }
    unsafe {
        QueryPerformanceCounter(&raw mut scratch.end);
    }
    ok && scratch.context.0.Rip != 0
}

fn resume_held(handle: HANDLE, retries: &AtomicU64) -> bool {
    let timeout = 0_i64;
    let retry_delay = -10_000_i64;
    resume_until_released(
        retries,
        || unsafe { ResumeThread(handle) },
        || unsafe { NtWaitForSingleObject(handle, 0, &raw const timeout) },
        || unsafe {
            NtDelayExecution(0, &raw const retry_delay);
        },
    )
}

fn resume_until_released(
    retries: &AtomicU64,
    mut resume: impl FnMut() -> u32,
    mut wait: impl FnMut() -> i32,
    mut delay: impl FnMut(),
) -> bool {
    while resume() == u32::MAX {
        retries.fetch_add(1, Ordering::Relaxed);
        if wait() == STATUS_WAIT_0 {
            return false;
        }
        delay();
    }
    true
}

fn qpc() -> u64 {
    let mut value = 0;
    unsafe {
        QueryPerformanceCounter(&raw mut value);
    }
    value.max(0) as u64
}
struct Timer {
    handle: Option<Handle>,
}
impl Timer {
    fn new() -> Self {
        let handle = unsafe {
            CreateWaitableTimerExW(
                ptr::null(),
                ptr::null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS,
            )
        };
        let handle = (!handle.is_null()).then_some(Handle(handle));
        COUNTERS
            .timer
            .store(if handle.is_some() { 1 } else { 2 }, Ordering::Relaxed);
        Self { handle }
    }
    fn wait(&mut self, stop: &AtomicBool, round_start: u64, interval_ticks: u64) {
        let remaining = round_start
            .saturating_add(interval_ticks)
            .saturating_sub(qpc());
        if stop.load(Ordering::Acquire) {
            return;
        }
        if remaining == 0 {
            if self.handle.is_none() {
                unsafe {
                    Sleep(1);
                }
            }
            return;
        }
        let hz = load(&COUNTERS.qpc_hz).max(1);
        if let Some(handle) = &self.handle {
            let units = (u128::from(remaining) * 10_000_000).div_ceil(u128::from(hz));
            let due = -(units.clamp(1, i64::MAX as u128) as i64);
            if unsafe { SetWaitableTimer(handle.0, &raw const due, 0, None, ptr::null(), 0) } != 0 {
                loop {
                    let result = unsafe { WaitForSingleObject(handle.0, 50) };
                    if result == 0 || stop.load(Ordering::Acquire) {
                        return;
                    }
                    if result != WAIT_TIMEOUT {
                        break;
                    }
                }
            }
            self.handle = None;
            COUNTERS.timer.store(2, Ordering::Relaxed);
        }
        let mut remaining = (u128::from(remaining) * 1000)
            .div_ceil(u128::from(hz))
            .max(1) as u64;
        while remaining != 0 && !stop.load(Ordering::Acquire) {
            let slice = remaining.min(50);
            unsafe {
                Sleep(slice as u32);
            }
            remaining -= slice;
        }
    }
}
fn discover(targets: &mut Vec<Target>, ids: &[u32]) {
    let registered = crate::thread_cpu::registered().filter_map(|(name, id)| {
        let role = ROLE_NAMES.iter().position(|role| *role == name)?;
        Some((id, role as u8, 255))
    });
    let helpers = crate::thread_cpu::helpers()
        .enumerate()
        .map(|(index, id)| (id, HELPER_ROLE, index as u8));
    let workers = ids
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, 7, index as u8));
    let game = std::iter::once((GAME_ID.load(Ordering::Acquire), GAME_ROLE, 255));
    for (id, role, worker) in registered.chain(helpers).chain(workers).chain(game) {
        if id == 0 || id == thread_id() || targets.iter().any(|t| t.info.thread_id == id) {
            continue;
        }
        let Ok(slot) = u16::try_from(targets.len()) else {
            COUNTERS.open_failed.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        if let Some(target) = Target::open(id, role, worker, slot) {
            targets.push(target);
        } else {
            COUNTERS.open_failed.fetch_add(1, Ordering::Relaxed);
        }
    }
}
fn sample(target: &mut Target, scratch: &mut Scratch) -> Option<format::Record> {
    let mut cycles = 0;
    if unsafe { QueryThreadCycleTime(target.handle.0, &raw mut cycles) } == 0 {
        COUNTERS.capture_failed.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let weight = cycles.saturating_sub(target.last_cycles);
    target.last_cycles = cycles;
    if weight == 0 {
        COUNTERS.idle.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let game = target.role() == GAME_ROLE;
    if game && !GAME_GATE.claim() {
        return None;
    }
    let captured = capture_window(target, scratch);
    if game {
        GAME_GATE.touching.store(false, Ordering::SeqCst);
    }
    if scratch.suspended {
        let elapsed = scratch.end.saturating_sub(scratch.stamp) as u64;
        COUNTERS.suspension_count.fetch_add(1, Ordering::Relaxed);
        COUNTERS
            .suspension_sum
            .fetch_add(elapsed, Ordering::Relaxed);
        COUNTERS
            .suspension_max
            .fetch_max(elapsed, Ordering::Relaxed);
    }
    if !captured {
        COUNTERS.capture_failed.fetch_add(1, Ordering::Relaxed);
        COUNTERS.dropped.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    if game && !scratch.in_callback {
        COUNTERS.game_exit_races.fetch_add(1, Ordering::Relaxed);
        COUNTERS.dropped.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    Some(format::Record {
        qpc: scratch.stamp.max(0) as u64,
        cycles: weight,
        slot: target.info.slot,
        role: target.role(),
        worker: target.info.worker_index.unwrap_or(255),
        job: scratch.job,
        pc: scratch.context.0.Rip,
    })
}
fn flush(
    chunk: &mut format::Chunk,
    writer: &mut format::Writer,
    targets: usize,
    interval_ticks: u64,
) -> io::Result<()> {
    let begin = qpc();
    let count = chunk.len();
    if let Err(error) = writer.records(chunk.bytes()) {
        COUNTERS
            .write_failed
            .fetch_add(count as u64, Ordering::Relaxed);
        COUNTERS.dropped.fetch_add(count as u64, Ordering::Relaxed);
        chunk.clear();
        return Err(error);
    }
    COUNTERS.written.fetch_add(count as u64, Ordering::Relaxed);
    chunk.clear();
    let missed = qpc().saturating_sub(begin) / interval_ticks.max(1);
    let skipped = missed.saturating_mul(targets as u64);
    COUNTERS.backpressure.fetch_add(skipped, Ordering::Relaxed);
    COUNTERS.dropped.fetch_add(skipped, Ordering::Relaxed);
    Ok(())
}
fn run(
    settings: Settings,
    ids: Vec<u32>,
    stop: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    mut scratch: Scratch,
    mut chunk: format::Chunk,
    mut writer: format::Writer,
) {
    let mut timer = Timer::new();
    let mut targets = Vec::with_capacity(ids.len() + 8);
    let mut refresh = Instant::now();
    let mut previous = 0_u64;
    let interval_ticks = ((u128::from(load(&COUNTERS.qpc_hz)) * u128::from(settings.interval_us))
        / 1_000_000)
        .max(1) as u64;
    let mut error = None;
    while !stop.load(Ordering::Acquire) {
        if !active.load(Ordering::Acquire) {
            unsafe {
                Sleep(1);
            }
            continue;
        }
        if targets.is_empty() || refresh.elapsed() >= Duration::from_millis(50) {
            discover(&mut targets, &ids);
            refresh = Instant::now();
        }
        let now = qpc();
        if previous != 0 {
            let interval = now.saturating_sub(previous);
            COUNTERS.interval_sum.fetch_add(interval, Ordering::Relaxed);
            COUNTERS.interval_max.fetch_max(interval, Ordering::Relaxed);
        }
        previous = now;
        COUNTERS.rounds.fetch_add(1, Ordering::Relaxed);
        for target in &mut targets {
            if stop.load(Ordering::Acquire) {
                break;
            }
            if let Some(record) = sample(target, &mut scratch)
                && !chunk.append(&record)
            {
                COUNTERS.dropped.fetch_add(1, Ordering::Relaxed);
                COUNTERS.buffer_full.fetch_add(1, Ordering::Relaxed);
            }
        }
        if chunk.full()
            && let Err(e) = flush(&mut chunk, &mut writer, targets.len(), interval_ticks)
        {
            error = Some(e.to_string());
            break;
        }
        timer.wait(&stop, previous, interval_ticks);
    }
    if error.is_none()
        && let Err(e) = flush(&mut chunk, &mut writer, 0, interval_ticks)
    {
        error = Some(e.to_string());
    }
    if error.is_none() {
        let job_roles = crate::roles::ROLES
            .iter()
            .map(|role| {
                serde_json::to_value(role).and_then(|v| {
                    v.as_str().map(str::to_owned).ok_or_else(|| {
                        serde_json::Error::io(io::Error::other("job role is not a string"))
                    })
                })
            })
            .collect::<Result<Vec<_>, _>>();
        match job_roles {
            Ok(job_roles) => {
                let trailer = format::Trailer {
                    record_count: load(&COUNTERS.written),
                    dropped_counts: format::Drops {
                        total: load(&COUNTERS.dropped),
                        capture: load(&COUNTERS.capture_failed),
                        write: load(&COUNTERS.write_failed),
                        writer_backpressure: load(&COUNTERS.backpressure),
                        buffer_full: load(&COUNTERS.buffer_full),
                        game_exit_race: load(&COUNTERS.game_exit_races),
                    },
                    threads: targets.iter().map(|t| t.info.clone()).collect(),
                    job_roles,
                    modules: format::modules(),
                };
                if let Err(e) = writer.finish(&trailer) {
                    error = Some(e.to_string());
                }
            }
            Err(e) => error = Some(e.to_string()),
        }
    }
    if error.is_none() && EMERGENCY_EXIT.load(Ordering::Acquire) {
        error = Some("sampled collector thread stopped sampler before exit".into());
    }
    META.lock().unwrap_or_else(|e| e.into_inner()).stop_error = error;
}
