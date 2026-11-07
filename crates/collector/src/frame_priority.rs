//! Bounded pressure notifications; one owned worker reconciles native priorities.
use crate::status::{sequence, status};
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows_sys::Win32::Foundation::WAIT_TIMEOUT;
use windows_sys::Win32::System::Threading::{
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateEventW, CreateWaitableTimerExW,
    GetCurrentThreadId, OpenThread, SetEvent, SetThreadPriority, SetThreadSelectedCpuSets,
    SetWaitableTimer, THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_HIGHEST, THREAD_PRIORITY_IDLE,
    THREAD_PRIORITY_NORMAL, THREAD_QUERY_LIMITED_INFORMATION, THREAD_SET_INFORMATION,
    THREAD_SET_LIMITED_INFORMATION, TIMER_ALL_ACCESS, WaitForMultipleObjects,
};

#[derive(Clone, Copy)]
pub(crate) enum Role {
    Decode,
    Spool,
    FirstHelper,
    SecondHelper,
    Sizing,
}
const ROLES: [&str; 5] = ["decode", "spool", "first_helper", "second_helper", "sizing"];
#[derive(Clone, Copy)]
pub(crate) enum Caller {
    Callback,
    Store,
    Worker,
}
const CALLERS: [&str; 3] = ["callback", "store", "worker"];
const UNKNOWN: u32 = 3;
/// A copy-only thread's value while the game ticks: idle priority, off the main thread's core.
const TICK: u32 = 4;
/// The same while at most one frame is free: normal priority, still off the main thread's core.
const TICK_PRESSED: u32 = 5;
/// The pressure word while the game ticks, with [`PRESSED`] when at most one frame is free.
const TICKING: u32 = 1 << 8;
const PRESSED: u32 = 1 << 9;
/// The ticking word when the main game thread runs on another processor group.
const NO_PROCESSOR: u32 = u32::MAX;
const BOOST_NS: u64 = 4_000_000;
const REST_NS: u64 = 1_000_000;
const RETRY: Duration = Duration::from_millis(1);

fn level(free: usize, store_waiters: usize) -> u32 {
    if store_waiters != 0 || free <= 1 {
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
        TICK_PRESSED => THREAD_PRIORITY_NORMAL,
        2 => THREAD_PRIORITY_HIGHEST,
        1 => THREAD_PRIORITY_ABOVE_NORMAL,
        _ => THREAD_PRIORITY_NORMAL,
    }
}
fn for_role(pressure: u32, role: usize) -> u32 {
    if pressure & TICKING != 0 {
        // Sizing reads live game memory and waits the tick out at its own priority.
        return if role == Role::Sizing as usize {
            0
        } else if pressure & PRESSED != 0 {
            TICK_PRESSED
        } else {
            TICK
        };
    }
    if role == Role::Decode as usize {
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
    calls: [AtomicU64; 6],
    failures: AtomicU64,
    /// A CPU-set selection is applied and must be cleared when the tick ends.
    confined: AtomicBool,
    /// The ticking word whose CPU sets are applied.
    selected: AtomicU32,
}
impl Target {
    fn new(handle: OwnedHandle) -> Self {
        Self {
            handle,
            applied: AtomicU32::new(0),
            calls: [const { AtomicU64::new(0) }; 6],
            failures: AtomicU64::new(0),
            confined: AtomicBool::new(false),
            selected: AtomicU32::new(0),
        }
    }
}

/// Logical processors one processor group holds.
const GROUP_PROCESSORS: usize = 64;

/// Every group-0 logical processor's CPU-set id with its physical core, read once, in fixed
/// storage: the reconciler allocates nothing.
struct Cores {
    /// `(logical processor, core, id)` for the first `len` entries.
    sets: [(u8, u8, u32); GROUP_PROCESSORS],
    len: usize,
    /// Per core index: the ids of every other core's processors, `others_len[core]` of them.
    others: [[u32; GROUP_PROCESSORS]; GROUP_PROCESSORS],
    others_len: [usize; GROUP_PROCESSORS],
}
impl Cores {
    fn read() -> Option<Self> {
        use windows_sys::Win32::System::SystemInformation::{
            CpuSetInformation, GetSystemCpuSetInformation, SYSTEM_CPU_SET_INFORMATION,
        };
        let mut buffer = [0u64; 2048];
        let mut length = 0u32;
        // SAFETY: the buffer holds its full byte length, aligned, and outlives the call.
        if unsafe {
            GetSystemCpuSetInformation(
                buffer.as_mut_ptr().cast(),
                size_of_val(&buffer) as u32,
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        } == 0
        {
            return None;
        }
        let mut cores = Self {
            sets: [(0, 0, 0); GROUP_PROCESSORS],
            len: 0,
            others: [[0; GROUP_PROCESSORS]; GROUP_PROCESSORS],
            others_len: [0; GROUP_PROCESSORS],
        };
        let entry_size = size_of::<SYSTEM_CPU_SET_INFORMATION>();
        let bytes = buffer.as_ptr().cast::<u8>();
        let mut offset = 0usize;
        while offset + entry_size <= length as usize && cores.len < GROUP_PROCESSORS {
            // SAFETY: entries are packed back to back, each starting with its own size.
            let entry = unsafe { &*bytes.add(offset).cast::<SYSTEM_CPU_SET_INFORMATION>() };
            if (entry.Size as usize) < entry_size {
                break;
            }
            if entry.Type == CpuSetInformation {
                // SAFETY: the CpuSet arm is the active one for CpuSetInformation entries.
                let set = unsafe { entry.Anonymous.CpuSet };
                if set.Group == 0 && usize::from(set.CoreIndex) < GROUP_PROCESSORS {
                    cores.sets[cores.len] = (set.LogicalProcessorIndex, set.CoreIndex, set.Id);
                    cores.len += 1;
                }
            }
            offset += entry.Size as usize;
        }
        for core in 0..GROUP_PROCESSORS {
            for &(_, other, id) in &cores.sets[..cores.len] {
                if usize::from(other) != core {
                    cores.others[core][cores.others_len[core]] = id;
                    cores.others_len[core] += 1;
                }
            }
        }
        (cores.len != 0).then_some(cores)
    }

    /// The ids of every core but the one running `processor`; none when that leaves nothing.
    fn away_from(&self, processor: u32) -> Option<&[u32]> {
        let &(_, core, _) = self.sets[..self.len]
            .iter()
            .find(|&&(logical, _, _)| u32::from(logical) == processor)?;
        let core = usize::from(core);
        Some(&self.others[core][..self.others_len[core]]).filter(|ids| !ids.is_empty())
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

pub(crate) struct Pipeline {
    available: AtomicUsize,
    callbacks: AtomicUsize,
    store_waiters: AtomicUsize,
    store_expected: AtomicBool,
    /// While the game ticks: one plus the group-0 processor running the main game thread
    /// ([`NO_PROCESSOR`] when unknown); zero otherwise.
    ticking: AtomicU32,
    /// The ticking word this reconciliation pass applies CPU sets for.
    applying: AtomicU32,
    ticks: AtomicU64,
    cores: OnceLock<Option<Cores>>,
    targets: [OnceLock<Target>; 5],
    event: OnceLock<OwnedHandle>,
    stopping: AtomicBool,
    failed: AtomicBool,
    writing: AtomicBool,
    pending: AtomicBool,
    announced: AtomicU32,
    effective: AtomicU32,
    budget_state: AtomicU32,
    passes: AtomicU64,
    wakes: AtomicU64,
    wake_failures: AtomicU64,
    registration_failures: AtomicU64,
    cost: [[AtomicU64; 4]; 3],
    #[cfg(test)]
    hook: OnceLock<Box<dyn Fn(Boundary) + Send + Sync>>,
}
impl Pipeline {
    pub(crate) fn new() -> Self {
        Self {
            available: AtomicUsize::new(0),
            callbacks: AtomicUsize::new(0),
            store_waiters: AtomicUsize::new(0),
            store_expected: AtomicBool::new(false),
            ticking: AtomicU32::new(0),
            applying: AtomicU32::new(0),
            ticks: AtomicU64::new(0),
            cores: OnceLock::new(),
            targets: [const { OnceLock::new() }; 5],
            event: OnceLock::new(),
            stopping: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            writing: AtomicBool::new(false),
            pending: AtomicBool::new(false),
            announced: AtomicU32::new(0),
            effective: AtomicU32::new(0),
            budget_state: AtomicU32::new(0),
            passes: AtomicU64::new(0),
            wakes: AtomicU64::new(0),
            wake_failures: AtomicU64::new(0),
            registration_failures: AtomicU64::new(0),
            cost: [const { [const { AtomicU64::new(0) }; 4] }; 3],
            #[cfg(test)]
            hook: OnceLock::new(),
        }
    }
    fn free_count(&self) -> usize {
        self.available.load(Ordering::SeqCst).count_ones() as usize
    }
    // Mark before queue publication so a racing pop cannot be undone by a late mark.
    pub(crate) fn publish(&self, index: usize) {
        self.available.fetch_or(1 << index, Ordering::SeqCst);
    }
    pub(crate) fn claim(&self, index: usize, caller: Caller) {
        self.available.fetch_and(!(1 << index), Ordering::SeqCst);
        self.refresh(caller);
    }
    fn pressure(&self) -> u32 {
        if self.stopping.load(Ordering::SeqCst) {
            return 0;
        }
        if self.ticking.load(Ordering::SeqCst) != 0 {
            return if self.free_count() <= 1 {
                TICKING | PRESSED
            } else {
                TICKING
            };
        }
        if self.callbacks.load(Ordering::SeqCst) == 0 {
            return 0;
        }
        let free = self.free_count();
        level(free, 0)
            | (level(
                free,
                self.store_waiters.load(Ordering::SeqCst)
                    + usize::from(self.store_expected.load(Ordering::SeqCst)),
            ) << 2)
    }
    fn notify(&self) {
        if !self.pending.swap(true, Ordering::SeqCst) {
            if let Some(event) = self.event.get() {
                self.wakes.fetch_add(1, Ordering::Relaxed);
                // SAFETY: the pipeline retains this auto-reset event through every notifier.
                if unsafe { SetEvent(event.as_raw_handle()) } == 0 {
                    self.wake_failures.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }
    /// The game thread enters its hourly tick on group-0 `processor` (`Some(None)` when it
    /// runs on another group) or leaves it (`None`). Copy-only threads then work at idle
    /// priority, off that processor's core, until the tick returns.
    pub(crate) fn tick(&self, processor: Option<Option<u32>>) {
        let word = match processor {
            None => 0,
            Some(processor) => {
                self.ticks.fetch_add(1, Ordering::Relaxed);
                processor.map_or(NO_PROCESSOR, |processor| processor.saturating_add(1))
            }
        };
        self.ticking.store(word, Ordering::SeqCst);
        self.refresh(Caller::Callback);
    }
    /// The CPU sets of every core but the main game thread's, for this pass's ticking word.
    fn tick_sets(&self) -> Option<&[u32]> {
        let word = self.applying.load(Ordering::SeqCst);
        if word == 0 || word == NO_PROCESSOR {
            return None;
        }
        self.cores.get()?.as_ref()?.away_from(word - 1)
    }
    pub(crate) fn callback(&self) -> Callback<'_> {
        self.callbacks.fetch_add(1, Ordering::SeqCst);
        self.refresh(Caller::Callback);
        Callback(self)
    }
    // Only the named collector worker registers, after its NORMAL initialization succeeds.
    pub(crate) fn register(&self, role: Role) {
        let slot = &self.targets[role as usize];
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
            self.registration_failures.fetch_add(1, Ordering::Relaxed);
            return;
        }
        // SAFETY: OpenThread returned a fresh owned handle.
        let target = Target::new(unsafe { OwnedHandle::from_raw_handle(handle) });
        if slot.set(target).is_ok() {
            self.notify();
        }
    }
    /// Whether `role`'s worker registered a handle the reconciler writes.
    pub(crate) fn registered(&self, role: Role) -> bool {
        self.targets[role as usize].get().is_some()
    }
    pub(crate) fn refresh(&self, caller: Caller) {
        #[cfg(feature = "callback_profile")]
        let begin = (crate::sampling::qpc(), crate::spool_phases::thread_cycles());
        let pressure = self.pressure();
        if pressure != self.announced.load(Ordering::SeqCst)
            && self.announced.swap(pressure, Ordering::SeqCst) != pressure
        {
            self.notify();
        }
        let cost = &self.cost[caller as usize];
        cost[0].fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "callback_profile")]
        {
            let cycles = crate::spool_phases::thread_cycles();
            cost[1].fetch_add(
                (crate::sampling::qpc() - begin.0).max(0) as u64,
                Ordering::Relaxed,
            );
            if begin.1 != 0 && cycles != 0 {
                cost[2].fetch_add(cycles.saturating_sub(begin.1), Ordering::Relaxed);
                cost[3].fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    #[cfg(test)]
    fn boundary(&self, boundary: Boundary) {
        if let Some(hook) = self.hook.get() {
            hook(boundary);
        }
    }
    // Only the reconciler writes; every pass has one gate attempt and at most five syscalls.
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
        // Clear before work, never after final validation: racing notifications must survive.
        self.pending.swap(false, Ordering::SeqCst);
        #[cfg(test)]
        self.boundary(Boundary::WriterRegistered);
        let word = self.ticking.load(Ordering::SeqCst);
        self.applying.store(word, Ordering::SeqCst);
        let pressure = self.pressure();
        let wanted = if pressure & TICKING != 0 {
            // A tick ends any boost: no expired deadline may keep rearming the timer.
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
            let ticking = value == TICK || value == TICK_PRESSED;
            if target.applied.load(Ordering::SeqCst) == value
                && (!ticking || target.selected.load(Ordering::SeqCst) == word)
            {
                continue;
            }
            target.applied.store(UNKNOWN, Ordering::SeqCst);
            #[cfg(test)]
            self.boundary(Boundary::BeforeSyscall);
            let applied = set(role, target, value);
            #[cfg(test)]
            self.boundary(Boundary::AfterSyscall);
            target.calls[value as usize].fetch_add(1, Ordering::Relaxed);
            if applied {
                target.applied.store(value, Ordering::SeqCst);
            } else {
                target.failures.fetch_add(1, Ordering::Relaxed);
                success = false;
            }
        }
        if success && wanted == 0 {
            budget.normal_applied(now());
        }
        // Publish before the final read: later mutations either fail validation or notify again.
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
        self.effective.store(final_wanted, Ordering::SeqCst);
        self.budget_state.store(
            match budget {
                Budget::Idle => 0,
                Budget::Boost(_) => 1,
                Budget::Rest(None) => 2,
                Budget::Rest(Some(_)) => 3,
            },
            Ordering::SeqCst,
        );
        self.passes.fetch_add(1, Ordering::Relaxed);
    }
    pub(crate) fn expect_store(&self, loaned: &AtomicBool) {
        self.store_expected.store(true, Ordering::SeqCst);
        // A loan returned after the caller saw it outstanding must not leave the hint set.
        if !loaned.load(Ordering::SeqCst) {
            self.store_expected.store(false, Ordering::SeqCst);
        }
        self.refresh(Caller::Callback);
    }
    pub(crate) fn store_returned(&self) {
        self.clear_store_expected();
        self.refresh(Caller::Worker);
    }
    pub(crate) fn clear_store_expected(&self) {
        self.store_expected.store(false, Ordering::SeqCst);
    }
    pub(crate) fn wait_for_store(&self) -> StoreWait<'_> {
        self.store_waiters.fetch_add(1, Ordering::SeqCst);
        self.refresh(Caller::Store);
        StoreWait(self)
    }
    pub(crate) fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }
    pub(crate) fn status(&self, frequency: u64) -> impl serde::Serialize + '_ {
        status!({
            "free": self.free_count(),
            "controller_failed": self.failed(),
            "callbacks": self.callbacks.load(Ordering::SeqCst),
            "effective_priority": priority(for_role(self.effective.load(Ordering::SeqCst), 0)),
            "budget": match self.budget_state.load(Ordering::SeqCst) { 1 => "boost", 2 => "normal_pending", 3 => "normal_cooldown", _ => "idle" },
            "boost_budget_ns": BOOST_NS,
            "normal_cooldown_ns": REST_NS,
            "store_waiters": self.store_waiters.load(Ordering::SeqCst),
            "ticks": self.ticks.load(Ordering::Relaxed),
            "tick_cores_known": self.cores.get().is_some_and(Option::is_some),
            "store_expected": self.store_expected.load(Ordering::SeqCst),
            "requested_priority": priority(for_role(self.pressure(), 0)),
            "pending_reconciliation": self.pending.load(Ordering::SeqCst),
            "writer_active": self.writing.load(Ordering::SeqCst),
            "reconciliation_passes": self.passes.load(Ordering::Relaxed),
            "wake_calls": self.wakes.load(Ordering::Relaxed),
            "wake_failures": self.wake_failures.load(Ordering::Relaxed),
            "registration_failures": self.registration_failures.load(Ordering::Relaxed),
            "threads": sequence(self.targets.iter().zip(ROLES).enumerate().map(|(index, (target, role))| {
                let target = target.get();
                status!({
                    "role": role,
                    "requested_priority": priority(for_role(self.pressure(), index)),
                    "effective_priority": priority(for_role(self.effective.load(Ordering::SeqCst), index)),
                    "registered": target.is_some(),
                    "applied_priority": target.map(|target| target.applied.load(Ordering::SeqCst)).filter(|value| *value != UNKNOWN).map(priority),
                    "normal_calls": target.map(|target| target.calls[0].load(Ordering::Relaxed)),
                    "above_normal_calls": target.map(|target| target.calls[1].load(Ordering::Relaxed)),
                    "highest_calls": target.map(|target| target.calls[2].load(Ordering::Relaxed)),
                    "tick_idle_calls": target.map(|target| target.calls[TICK as usize].load(Ordering::Relaxed)),
                    "tick_pressed_calls": target.map(|target| target.calls[TICK_PRESSED as usize].load(Ordering::Relaxed)),
                    "failures": target.map(|target| target.failures.load(Ordering::Relaxed)),
                })
            })),
            "refresh_cost": sequence(self.cost.iter().zip(CALLERS).map(move |(cost, caller)| status!({
                "caller": caller,
                "calls": cost[0].load(Ordering::Relaxed),
                "profiled": cfg!(feature = "callback_profile"),
                "wall_ns": cfg!(feature = "callback_profile").then(|| (u128::from(cost[1].load(Ordering::Relaxed)) * 1_000_000_000 / u128::from(frequency.max(1))) as u64),
                "cycle_samples": cost[3].load(Ordering::Relaxed),
                "cycles": (cost[3].load(Ordering::Relaxed) != 0).then(|| cost[2].load(Ordering::Relaxed)),
            }))),
            "scope": "status-only collector policy: while the game ticks, decode, spool and both helpers (copy-only threads) run at idle priority (normal while at most one frame is free) with CPU sets on every core but the one running the main game thread (none when unknown), and sizing stays normal and waits the tick out; outside ticks, pool pressure promotes at two free frames, highest at at most one; expected/pending store loans promote decode alone; no promotion without an active capture callback. Refresh reads a fixed atomic availability bitmask, not ArrayQueue::len; each bit is published before its queue push and cleared after its pop, so the pressure hint may briefly include one not-yet-enqueued recycle but cannot lose a racing claim; refresh has no priority setters, CAS loops, allocation or blocking; plain release makes no clock/cycle queries, unchanged announced pressure makes no wake syscall, changed pressure coalesces at most one SetEvent. A single owned HIGHEST, boost-disabled reconciler writes registered collector handles only, never game threads; each pass has one gate attempt, five target visits, at most five setters, and final desired-state validation. Pending is cleared only before work; validation failures, registration, API failures and racing notifications schedule another off-callback pass (one-ms retry). Applied values are actual successful writes, not proof of convergence. Elevation is limited to four-ms high-resolution-timer bursts followed by at least one ms after successful NORMAL application on every registered target, even if pressure persists; deadline and callback-exit revocation happen mid-item without waiting for sizing. The high-resolution timer avoids coarse timer rounding; durations still exclude scheduling and API delays, which Windows does not bound; idle event failures are retried by a 20-ms wait timeout; timer/wait failure demotes and exits, and the loader detaches; API failures remain unknown and retry. Reconciler is joined before final release quiescence and demotes on stop, unwind or timer/wait failure. Refresh clocks/cycles are callback_profile-only; controller CPU is in collector_thread_cpu.",
        })
    }
}

pub(crate) struct Callback<'a>(&'a Pipeline);
impl Drop for Callback<'_> {
    fn drop(&mut self) {
        self.0.callbacks.fetch_sub(1, Ordering::SeqCst);
        self.0.refresh(Caller::Callback);
    }
}
pub(crate) struct StoreWait<'a>(&'a Pipeline);
impl StoreWait<'_> {
    pub(crate) fn retry(&self) {
        self.0.refresh(Caller::Store);
    }
}
impl Drop for StoreWait<'_> {
    fn drop(&mut self) {
        self.0.store_waiters.fetch_sub(1, Ordering::SeqCst);
        self.0.refresh(Caller::Store);
    }
}

fn set_native(pipeline: &Pipeline, target: &Target, value: u32) -> bool {
    let handle = target.handle.as_raw_handle();
    // SAFETY: the reconciler alone writes these retained collector-owned handles.
    let mut applied = unsafe { SetThreadPriority(handle, priority(value)) != 0 };
    let sets = if value == TICK || value == TICK_PRESSED {
        pipeline.tick_sets()
    } else {
        None
    };
    if let Some(sets) = sets {
        // SAFETY: as above; the id slice lives in the pipeline through the call.
        let selected =
            unsafe { SetThreadSelectedCpuSets(handle, sets.as_ptr(), sets.len() as u32) != 0 };
        // A failed reselection may leave the previous one installed: keep it to clear.
        if selected {
            target.confined.store(true, Ordering::SeqCst);
            target
                .selected
                .store(pipeline.applying.load(Ordering::SeqCst), Ordering::SeqCst);
        }
        applied &= selected;
    } else if target.confined.load(Ordering::SeqCst) {
        // SAFETY: as above; a null, empty list clears the selection.
        let cleared = unsafe { SetThreadSelectedCpuSets(handle, std::ptr::null(), 0) != 0 };
        if cleared {
            target.confined.store(false, Ordering::SeqCst);
        }
        applied &= cleared;
    }
    if applied && value != TICK && value != TICK_PRESSED {
        target.selected.store(0, Ordering::SeqCst);
    } else if applied && sets.is_none() {
        // Idle priority without a selection: the main thread's core is unknown.
        target
            .selected
            .store(pipeline.applying.load(Ordering::SeqCst), Ordering::SeqCst);
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
            let success = set_native(self.0, target, 0);
            target.calls[0].fetch_add(1, Ordering::Relaxed);
            if success {
                target.applied.store(0, Ordering::SeqCst);
            } else {
                target.failures.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn wait(event: &OwnedHandle, timer: &OwnedHandle, delay: Option<Duration>) -> bool {
    let handles = [event.as_raw_handle(), timer.as_raw_handle()];
    let (count, timeout) = if let Some(delay) = delay {
        let due = -(delay.as_nanos().div_ceil(100).clamp(1, i64::MAX as u128) as i64);
        // SAFETY: this worker owns the timer and both handles outlive the synchronous wait.
        if unsafe { SetWaitableTimer(handles[1], &due, 0, None, std::ptr::null(), 0) } == 0 {
            return false;
        }
        (2, u32::MAX)
    } else {
        // An idle timeout also recovers a failed SetEvent; no worker priority is elevated then.
        (1, 20)
    };
    // SAFETY: the fixed handle array is valid for count entries; wait-all is disabled.
    let result = unsafe { WaitForMultipleObjects(count, handles.as_ptr(), 0, timeout) };
    result < count || (count == 1 && result == WAIT_TIMEOUT)
}

pub(crate) struct Reconciler {
    pipeline: Arc<Pipeline>,
    thread: Option<JoinHandle<()>>,
}
impl Reconciler {
    pub(crate) fn start(pipeline: Arc<Pipeline>) -> Result<Self, &'static str> {
        // SAFETY: unnamed, auto-reset handles; no game objects or callbacks are involved.
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
        // SAFETY: each non-null handle is freshly owned, including on partial initialization failure.
        let event = (!event.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(event) });
        let timer = (!timer.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(timer) });
        let (Some(event), Some(timer)) = (event, timer) else {
            return Err("frame priority high-resolution timer or event unavailable");
        };
        pipeline
            .event
            .set(event)
            .map_err(|_| "frame priority reconciler already started")?;
        let worker = Arc::clone(&pipeline);
        let (send, ready) = std::sync::mpsc::sync_channel(0);
        let thread = std::thread::Builder::new()
            .name("hoi4-frame-priority".into())
            .spawn(move || {
                #[cfg(feature = "worker_sampler")]
                let _sampler_exit = crate::thread_cpu::StopOnThreadExit;
                crate::thread_cpu::register(crate::thread_cpu::Role::FramePriority);
                let mut own = crate::runtime::OwnPriority::frame_pipeline();
                if !own.reconciler() {
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
                let _ = worker.cores.set(Cores::read());
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
            crate::thread_cpu::register_id(crate::thread_cpu::Role::FramePriority, 0);
            return Err("frame priority reconciler policy failed");
        }
        Ok(Self {
            pipeline,
            thread: Some(thread),
        })
    }
    pub(crate) fn thread_id(&self) -> u32 {
        // SAFETY: the owned join handle remains live through this query.
        unsafe {
            windows_sys::Win32::System::Threading::GetThreadId(
                self.thread.as_ref().unwrap().as_raw_handle() as _,
            )
        }
    }
    pub(crate) fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
    pub(crate) fn finish(&mut self) {
        self.pipeline.stopping.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            // SAFETY: the pipeline owns this event until after the joined worker exits.
            unsafe { SetEvent(self.pipeline.event.get().unwrap().as_raw_handle()) };
            let _ = thread.join();
            crate::thread_cpu::register_id(crate::thread_cpu::Role::FramePriority, 0);
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
