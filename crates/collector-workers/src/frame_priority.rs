use crate::cpu_sets::{prepare, select};
use std::{
    os::windows::io::{AsHandle, AsRawHandle, FromRawHandle, OwnedHandle},
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows_sys::Win32::Foundation::WAIT_TIMEOUT;
use windows_sys::Win32::System::Threading::{
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateEventW, CreateWaitableTimerExW,
    GetCurrentThreadId, OpenThread, SetEvent, SetThreadPriority, SetWaitableTimer,
    THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_HIGHEST, THREAD_PRIORITY_IDLE,
    THREAD_PRIORITY_LOWEST, THREAD_PRIORITY_NORMAL, THREAD_QUERY_LIMITED_INFORMATION,
    THREAD_SET_INFORMATION, THREAD_SET_LIMITED_INFORMATION, TIMER_ALL_ACCESS,
    WaitForMultipleObjects,
};

#[derive(Clone, Copy)]
pub enum Role {
    Decode,
    Spool,
    Sizing,
    Builder,
    SpoolWriter,
    JournalWriter,
    Helper(usize),
}
const FIXED_ROLES: usize = 6;
const SLOTS: usize = FIXED_ROLES + crate::spool_helper::MAX_HELPERS;
impl Role {
    const fn slot(self) -> usize {
        match self {
            Role::Decode => 0,
            Role::Spool => 1,
            Role::Sizing => 2,
            Role::Builder => 3,
            Role::SpoolWriter => 4,
            Role::JournalWriter => 5,
            Role::Helper(helper) => FIXED_ROLES + helper,
        }
    }
}
const UNKNOWN: u32 = 3;
const TICK: u32 = 4;
const TICK_PRESSED: u32 = 5;
const TICK_DECODE: u32 = 6;
const TICKING: u32 = 1 << 8;
const PRESSED: u32 = 1 << 9;
const NO_PROCESSOR: u32 = u32::MAX;
const BOOST_NS: u64 = 4_000_000;
const REST_NS: u64 = 1_000_000;
const RETRY: Duration = Duration::from_millis(1);

fn level(free: usize) -> u32 {
    if free <= 1 {
        2
    } else if free == 2 {
        1
    } else {
        0
    }
}
fn priority(level: u32) -> i32 {
    match level {
        TICK => THREAD_PRIORITY_IDLE,
        TICK_DECODE => THREAD_PRIORITY_LOWEST,
        TICK_PRESSED => THREAD_PRIORITY_NORMAL,
        2 => THREAD_PRIORITY_HIGHEST,
        1 => THREAD_PRIORITY_ABOVE_NORMAL,
        _ => THREAD_PRIORITY_LOWEST,
    }
}
fn for_role(pressure: u32, role: usize) -> u32 {
    if pressure & TICKING != 0 {
        return if pressure & PRESSED != 0 {
            TICK_PRESSED
        } else if role == Role::Decode.slot() {
            TICK_DECODE
        } else {
            TICK
        };
    }
    if role == Role::Decode.slot() {
        pressure >> 2
    } else {
        pressure & 3
    }
}

#[derive(Default)]
enum Budget {
    #[default]
    Idle,
    Boost(u64),
    Rest(Option<u64>),
}
impl Budget {
    fn wanted(&mut self, pressure: u32, now: u64) -> u32 {
        match *self {
            Self::Boost(end) if now >= end || pressure == 0 => *self = Self::Rest(None),
            Self::Rest(Some(end)) if now >= end => *self = Self::Idle,
            _ => {}
        }
        if matches!(self, Self::Idle) && pressure != 0 {
            *self = Self::Boost(now.saturating_add(BOOST_NS));
        }
        if matches!(self, Self::Boost(_)) {
            pressure
        } else {
            0
        }
    }
    fn normal_applied(&mut self, now: u64) {
        if matches!(self, Self::Rest(None)) {
            *self = Self::Rest(Some(now.saturating_add(REST_NS)));
        }
    }
    fn delay(&self, now: u64) -> Option<Duration> {
        match *self {
            Self::Idle => None,
            Self::Boost(end) | Self::Rest(Some(end)) => {
                Some(Duration::from_nanos(end.saturating_sub(now)))
            }
            Self::Rest(None) => Some(RETRY),
        }
    }
}

struct Target {
    handle: OwnedHandle,
    applied: AtomicU32,
    confined: AtomicBool,
    selected: AtomicU32,
}
impl Target {
    fn new(handle: OwnedHandle) -> Self {
        Self {
            handle,
            applied: AtomicU32::new(0),
            confined: AtomicBool::new(false),
            selected: AtomicU32::new(0),
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Boundary {
    WriterRegistered,
    DesiredRead,
    BeforeSyscall,
    AfterSyscall,
    Published,
    FinalDesiredRead,
}

pub struct Pipeline {
    available: AtomicUsize,
    callbacks: AtomicUsize,
    ticking: AtomicU32,
    applying: AtomicU32,
    last_game: AtomicU32,
    targets: [OnceLock<Target>; SLOTS],
    event: OnceLock<OwnedHandle>,
    stopping: AtomicBool,
    failed: AtomicBool,
    writing: AtomicBool,
    pending: AtomicBool,
    announced: AtomicU32,
    #[cfg(test)]
    hook: OnceLock<Box<dyn Fn(Boundary) + Send + Sync>>,
}
impl Pipeline {
    pub fn new() -> Self {
        Self {
            available: AtomicUsize::new(0),
            callbacks: AtomicUsize::new(0),
            ticking: AtomicU32::new(0),
            applying: AtomicU32::new(0),
            last_game: AtomicU32::new(0),
            targets: [const { OnceLock::new() }; SLOTS],
            event: OnceLock::new(),
            stopping: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            writing: AtomicBool::new(false),
            pending: AtomicBool::new(false),
            announced: AtomicU32::new(0),
            #[cfg(test)]
            hook: OnceLock::new(),
        }
    }
    fn free_count(&self) -> usize {
        self.available.load(Ordering::SeqCst)
    }
    pub fn publish(&self) {
        self.available.fetch_add(1, Ordering::SeqCst);
    }
    pub fn claim(&self) {
        let _ = self
            .available
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |free| {
                free.checked_sub(1)
            });
        self.refresh();
    }
    fn pressure(&self) -> u32 {
        if self.stopping.load(Ordering::SeqCst) {
            return 0;
        }
        if self.ticking.load(Ordering::SeqCst) != 0 {
            return if self.free_count() == 0 {
                TICKING | PRESSED
            } else {
                TICKING
            };
        }
        if self.callbacks.load(Ordering::SeqCst) == 0 {
            return 0;
        }
        let level = level(self.free_count());
        level | level << 2
    }
    fn notify(&self) {
        if !self.pending.swap(true, Ordering::SeqCst) {
            if let Some(event) = self.event.get() {
                // SAFETY: the pipeline retains this auto-reset event through every notifier.
                unsafe { SetEvent(event.as_raw_handle()) };
            }
        }
    }
    pub fn tick(&self, processor: Option<Option<u32>>) {
        let word = match processor {
            None => 0,
            Some(processor) => {
                if let Some(processor) = processor {
                    self.last_game
                        .store(processor.saturating_add(1), Ordering::Relaxed);
                }
                processor.map_or(NO_PROCESSOR, |processor| processor.saturating_add(1))
            }
        };
        self.ticking.store(word, Ordering::SeqCst);
        self.refresh();
    }
    fn desired_away(&self, value: u32) -> u32 {
        if self.stopping.load(Ordering::Acquire) || value == 2 {
            return 0;
        }
        let word = self.applying.load(Ordering::SeqCst);
        let word = if word != 0 && word != NO_PROCESSOR {
            word
        } else {
            self.last_game.load(Ordering::Relaxed)
        };
        if word == 0 || word == NO_PROCESSOR {
            return 0;
        }
        let narrow = value == 0 || value == TICK || value == TICK_DECODE;
        (word << 1) | u32::from(narrow)
    }
    pub fn callback(&self) -> Callback<'_> {
        self.callbacks.fetch_add(1, Ordering::SeqCst);
        self.refresh();
        Callback(self)
    }
    /// # Safety
    /// Register only this session's designated collector worker on its own thread, and keep it in
    /// the worker census until it exits.
    pub unsafe fn register(&self, role: Role) {
        let Some(slot) = self.targets.get(role.slot()) else {
            return;
        };
        if slot.get().is_some() {
            return;
        }
        // SAFETY: this id names the registering collector worker, never a game thread.
        let handle = unsafe {
            OpenThread(
                THREAD_SET_INFORMATION
                    | THREAD_SET_LIMITED_INFORMATION
                    | THREAD_QUERY_LIMITED_INFORMATION,
                0,
                GetCurrentThreadId(),
            )
        };
        if handle.is_null() {
            return;
        }
        let target = Target::new(unsafe { OwnedHandle::from_raw_handle(handle) });
        if slot.set(target).is_ok() {
            self.notify();
        }
    }
    pub fn registered(&self, role: Role) -> bool {
        self.targets
            .get(role.slot())
            .is_some_and(|slot| slot.get().is_some())
    }
    pub fn refresh(&self) {
        let pressure = self.pressure();
        if pressure != self.announced.load(Ordering::SeqCst)
            && self.announced.swap(pressure, Ordering::SeqCst) != pressure
        {
            self.notify();
        }
    }
    #[cfg(test)]
    fn boundary(&self, boundary: Boundary) {
        if let Some(hook) = self.hook.get() {
            hook(boundary);
        }
    }
    fn reconcile(
        &self,
        budget: &mut Budget,
        now: impl Fn() -> u64,
        mut set: impl FnMut(usize, &Target, u32) -> bool,
    ) {
        if self
            .writing
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            self.pending.store(true, Ordering::SeqCst);
            return;
        }
        struct Writing<'a>(&'a AtomicBool);
        impl Drop for Writing<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _writing = Writing(&self.writing);
        self.pending.swap(false, Ordering::SeqCst);
        #[cfg(test)]
        self.boundary(Boundary::WriterRegistered);
        let word = self.ticking.load(Ordering::SeqCst);
        self.applying.store(word, Ordering::SeqCst);
        let pressure = self.pressure();
        let wanted = if pressure & TICKING != 0 {
            *budget = Budget::default();
            pressure
        } else {
            budget.wanted(pressure, now())
        };
        #[cfg(test)]
        self.boundary(Boundary::DesiredRead);
        let mut success = true;
        for (role, slot) in self.targets.iter().enumerate() {
            let Some(target) = slot.get() else {
                continue;
            };
            let value = for_role(wanted, role);
            let away = self.desired_away(value);
            if target.applied.load(Ordering::SeqCst) == value
                && target.selected.load(Ordering::SeqCst) == away
            {
                continue;
            }
            target.applied.store(UNKNOWN, Ordering::SeqCst);
            #[cfg(test)]
            self.boundary(Boundary::BeforeSyscall);
            let applied = set(role, target, value);
            #[cfg(test)]
            self.boundary(Boundary::AfterSyscall);
            if applied {
                target.applied.store(value, Ordering::SeqCst);
            } else {
                success = false;
            }
        }
        if success && wanted == 0 {
            budget.normal_applied(now());
        }
        self.announced.store(pressure, Ordering::SeqCst);
        #[cfg(test)]
        self.boundary(Boundary::Published);
        let final_pressure = self.pressure();
        let final_wanted = if final_pressure & TICKING != 0 {
            *budget = Budget::default();
            final_pressure
        } else {
            budget.wanted(final_pressure, now())
        };
        #[cfg(test)]
        self.boundary(Boundary::FinalDesiredRead);
        if !success
            || final_pressure != pressure
            || final_wanted != wanted
            || self.ticking.load(Ordering::SeqCst) != word
        {
            self.pending.store(true, Ordering::SeqCst);
        }
    }
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }
}

pub struct Callback<'a>(&'a Pipeline);
impl Drop for Callback<'_> {
    fn drop(&mut self) {
        self.0.callbacks.fetch_sub(1, Ordering::SeqCst);
        self.0.refresh();
    }
}
fn set_native(pipeline: &Pipeline, target: &Target, value: u32) -> bool {
    let handle = target.handle.as_raw_handle();
    // SAFETY: the reconciler alone writes these retained collector-owned handles.
    let mut applied = unsafe { SetThreadPriority(handle, priority(value)) != 0 };
    let away = pipeline.desired_away(value);
    if away != target.selected.load(Ordering::SeqCst) {
        let processor = away >> 1;
        let narrow = away & 1 != 0;
        let sets = (processor != 0 && processor != NO_PROCESSOR)
            .then(|| {
                if narrow {
                    crate::cpu_sets::background_away_from_game(Some(processor - 1))
                } else {
                    crate::cpu_sets::away_from_game(Some(processor - 1))
                }
            })
            .flatten();
        if sets.is_some() {
            // SAFETY: The reconciler owns these retained collector worker handles and applies session policy.
            let selected = unsafe { select(target.handle.as_handle(), sets) };
            if selected {
                target.confined.store(true, Ordering::SeqCst);
                target.selected.store(away, Ordering::SeqCst);
            }
            applied &= selected;
        } else if target.confined.load(Ordering::SeqCst) {
            // SAFETY: The reconciler owns these retained collector worker handles and clears session affinity.
            let cleared = unsafe { select(target.handle.as_handle(), None) };
            if cleared {
                target.confined.store(false, Ordering::SeqCst);
                target.selected.store(0, Ordering::SeqCst);
            }
            applied &= cleared;
        } else {
            target.selected.store(away, Ordering::SeqCst);
        }
    }
    applied
}

struct Reset<'a>(&'a Pipeline);
impl Drop for Reset<'_> {
    fn drop(&mut self) {
        if !self.0.stopping.swap(true, Ordering::SeqCst) {
            self.0.failed.store(true, Ordering::SeqCst);
        }
        for slot in &self.0.targets {
            let Some(target) = slot.get() else {
                continue;
            };
            if target.applied.load(Ordering::SeqCst) == 0 {
                continue;
            }
            target.applied.store(UNKNOWN, Ordering::SeqCst);
            if set_native(self.0, target, 0) {
                target.applied.store(0, Ordering::SeqCst);
            }
        }
    }
}

fn wait(event: &OwnedHandle, timer: &OwnedHandle, delay: Option<Duration>) -> bool {
    let handles = [event.as_raw_handle(), timer.as_raw_handle()];
    let (count, timeout) = if let Some(delay) = delay {
        let due = -(delay.as_nanos().div_ceil(100).clamp(1, i64::MAX as u128) as i64);
        if unsafe { SetWaitableTimer(handles[1], &due, 0, None, std::ptr::null(), 0) } == 0 {
            return false;
        }
        (2, u32::MAX)
    } else {
        (1, 20)
    };
    let result = unsafe { WaitForMultipleObjects(count, handles.as_ptr(), 0, timeout) };
    result < count || (count == 1 && result == WAIT_TIMEOUT)
}

pub struct Reconciler {
    pipeline: engine_allocation::SharedBox<Pipeline>,
    thread: Option<JoinHandle<()>>,
}
impl Reconciler {
    /// # Safety
    /// Only the runtime may start the reconciler for this session's pipeline. Keep it in the worker
    /// census and join it before tearing down the pipeline and scheduler state.
    pub unsafe fn start(
        pipeline: engine_allocation::SharedBox<Pipeline>,
    ) -> Result<Self, &'static str> {
        let (event, timer) = unsafe {
            (
                CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()),
                CreateWaitableTimerExW(
                    std::ptr::null(),
                    std::ptr::null(),
                    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                    TIMER_ALL_ACCESS,
                ),
            )
        };
        let event = (!event.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(event) });
        let timer = (!timer.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(timer) });
        let (Some(event), Some(timer)) = (event, timer) else {
            return Err("frame priority high-resolution timer or event unavailable");
        };
        pipeline
            .event
            .set(event)
            .map_err(|_| "frame priority reconciler already started")?;
        let worker = pipeline.clone();
        let (send, ready) = std::sync::mpsc::sync_channel(0);
        let thread = std::thread::Builder::new()
            .name("hoi4-frame-priority".into())
            .spawn(move || {
                crate::private_heap::off_hold_path();
                #[cfg(feature = "worker_sampler")]
                // SAFETY: This runtime-owned worker keeps the guard until it exits.
                let _sampler_exit = unsafe { crate::thread_cpu::StopOnThreadExit::new() };
                // SAFETY: Only the current session's designated thread registers this role.
                unsafe { crate::thread_cpu::register(crate::thread_cpu::Role::FramePriority) };
                let mut own =
                // SAFETY: This is the dedicated worker applying its own session's priority policy.
                unsafe { crate::priority::OwnPriority::frame_pipeline() };
                if !
                // SAFETY: This runtime-owned reconciler adjusts only its current thread.
                unsafe { own.reconciler() }
                {
                    let _ = send.send(false);
                    return;
                }
                let _reset = Reset(&worker);
                if send.send(true).is_err() {
                    return;
                }
                let started = Instant::now();
                let now = || started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
                let mut budget = Budget::default();
                prepare();
                loop {
                    let stopping = worker.stopping.load(Ordering::SeqCst);
                    worker.reconcile(&mut budget, now, |_, target, value| {
                        set_native(&worker, target, value)
                    });
                    if stopping {
                        break;
                    }
                    let delay = budget.delay(now());
                    let delay = if worker.pending.load(Ordering::SeqCst) {
                        Some(delay.map_or(RETRY, |delay| delay.min(RETRY)))
                    } else {
                        delay
                    };
                    if !wait(worker.event.get().unwrap(), &timer, delay) {
                        break;
                    }
                }
            })
            .map_err(|_| "frame priority reconciler could not start")?;
        if ready.recv() != Ok(true) {
            let _ = thread.join();
            // SAFETY: This updates the designated role of this runtime's owned worker.
            unsafe { crate::thread_cpu::register_id(crate::thread_cpu::Role::FramePriority, 0) };
            return Err("frame priority reconciler policy failed");
        }
        Ok(Self {
            pipeline,
            thread: Some(thread),
        })
    }
    pub fn thread_id(&self) -> u32 {
        unsafe {
            windows_sys::Win32::System::Threading::GetThreadId(
                self.thread.as_ref().unwrap().as_raw_handle() as _,
            )
        }
    }
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
    pub fn finish(&mut self) {
        self.pipeline.stopping.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            // SAFETY: the pipeline owns this event until after the joined worker exits.
            unsafe { SetEvent(self.pipeline.event.get().unwrap().as_raw_handle()) };
            let _ = thread.join();
            // SAFETY: This updates the designated role of this runtime's owned worker.
            unsafe { crate::thread_cpu::register_id(crate::thread_cpu::Role::FramePriority, 0) };
        }
    }
}
impl Drop for Reconciler {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests;
