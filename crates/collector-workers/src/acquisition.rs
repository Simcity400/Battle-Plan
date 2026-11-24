use std::sync::Arc;
use std::{
    cell::UnsafeCell,
    collections::VecDeque,
    mem::{MaybeUninit, size_of, size_of_val},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Condvar, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
};
use std::{
    os::windows::io::{AsHandle, AsRawHandle},
    thread::JoinHandle,
};

use serde::Serialize;
use windows_sys::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, SetThreadPriorityBoost,
    THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_NORMAL, WaitOnAddress, WakeByAddressAll,
    WakeByAddressSingle,
};

pub const MAX_WORKERS: usize = 15;
const WORKER_STACK_BYTES: usize = 4 * 1024 * 1024;
const VACANT: u64 = 0;
const PENDING: u64 = 1;
const PUBLISHED: u64 = 2;
const COLLECTING: u64 = 3;
const RUNNING: u64 = 4;
const PHASE_MASK: u64 = 7;
const MAX_GENERATION: u64 = u64::MAX >> 3;
pub fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}
const fn word(generation: u64, phase: u64) -> u64 {
    generation << 3 | phase
}
static NEXT_POOL_ID: AtomicU64 = AtomicU64::new(1);
static BOOST_REFUSALS: AtomicU64 = AtomicU64::new(0);
static PRIORITY_REFUSALS: AtomicU64 = AtomicU64::new(0);
static JOINS: AtomicU64 = AtomicU64::new(0);
static JOIN_WALL_NS: AtomicU64 = AtomicU64::new(0);
static JOIN_WALL_MAX_NS: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
#[path = "acquisition/readiness_tests.rs"]
mod readiness_tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Panicked,
    Stopped,
}

pub struct Completion<J, R> {
    pub request: J,
    pub report: Result<R, Failure>,
    #[cfg(feature = "acquisition_profile")]
    pub table_reads: collector_memory::read_accounting::TableCounts,
    #[cfg(feature = "callback_profile")]
    pub profile: Profile,
}

impl<J, R> Completion<J, R> {
    pub fn inline(request: J, report: Result<R, Failure>) -> Self {
        Self {
            request,
            report,
            #[cfg(feature = "acquisition_profile")]
            table_reads: collector_memory::read_accounting::ZERO,
            #[cfg(feature = "callback_profile")]
            profile: Profile::default(),
        }
    }
}

pub trait Publication: Send + Sync {
    fn published(&self);
    fn failed(&self) {}
}
type OnCompleted<J, R> = fn(&mut J, &Result<R, Failure>) -> Option<Arc<dyn Publication>>;

#[cfg(feature = "callback_profile")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Profile {
    pub queued_qpc: i64,
    pub(crate) claimed_qpc: i64,
    pub(crate) runner_begin_qpc: i64,
    pub(crate) runner_end_qpc: i64,
    pub(crate) runner_thread_id: u32,
    pub(crate) runner_thread_cycles: Option<u64>,
    pub(crate) submit_lock_wait_ticks: u64,
    pub(crate) finish_lock_wait_ticks: u64,
    pub(crate) published_qpc: i64,
    pub(crate) collected_qpc: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Ticket {
    pool: u64,
    slot: usize,
    generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitError {
    Empty,
    Full,
    Stopped,
    Exhausted,
}

#[derive(Clone, Debug, Default)]
pub struct ThreadIds {
    values: [u32; MAX_WORKERS],
    len: usize,
}
impl ThreadIds {
    pub fn ids(&self) -> &[u32] {
        &self.values[..self.len]
    }
}
impl Serialize for ThreadIds {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.values[..self.len].serialize(serializer)
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Calibration {
    pub thread_ids: ThreadIds,
    pub capacity: usize,
    pub queued: usize,
    pub active: usize,
    pub uncollected: usize,
    pub accepted: u64,
    pub completed: u64,
    pub panicked: u64,
    pub stopped: u64,
    pub rejected: u64,
    pub caller_executed: u64,
    pub worker_executed: u64,
    pub(crate) join_sleeps: u64,
    pub(crate) join_help: bool,
    pub(crate) help_executed: u64,
    pub(crate) help_ns: u64,
    pub callback_help: bool,
    pub(crate) callback_help_executed: u64,
    pub(crate) callback_help_ns: u64,
    pub(crate) wake_boost_refusals: u64,
    pub(crate) priority_refusals: u64,
    pub joins: u64,
    pub(crate) join_wall_ns: u64,
    pub(crate) join_wall_max_ns: u64,
}

#[derive(Default)]
struct SubmitCounters {
    accepted: u64,
    rejected: u64,
}

#[derive(Default)]
struct Counters {
    completed: AtomicU64,
    panicked: AtomicU64,
    stopped: AtomicU64,
    caller_executed: AtomicU64,
    worker_executed: AtomicU64,
    join_sleeps: AtomicU64,
    callback_help_executed: AtomicU64,
    callback_help_ns: AtomicU64,
    #[cfg(feature = "executor_join_help")]
    help_executed: AtomicU64,
    #[cfg(feature = "executor_join_help")]
    help_ns: AtomicU64,
}

#[cfg(feature = "executor_join_help")]
const JOIN_HELP_ATTEMPTS: usize = 4;

#[cfg(feature = "executor_join_help")]
struct HelpBudget {
    attempts: usize,
    ticks: Option<u64>,
}
#[cfg(feature = "executor_join_help")]
impl HelpBudget {
    fn new() -> Self {
        Self {
            attempts: JOIN_HELP_ATTEMPTS,
            ticks: None,
        }
    }
    fn attempt(&mut self) -> bool {
        if self.attempts == 0 || self.ticks == Some(0) {
            return false;
        }
        self.attempts -= 1;
        true
    }
}

#[cfg(all(test, feature = "executor_join_help"))]
#[derive(Default)]
struct HelpAttemptGate {
    armed: AtomicBool,
    entered: AtomicBool,
    released: AtomicBool,
}

fn add(counter: &AtomicU64, amount: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(amount))
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Vacant,
    Queued,
    Running,
    Parked,
}

struct Slot {
    generation: u64,
    priority: u8,
    worker_only: bool,
    callback_help: bool,
    phase: Phase,
    #[cfg(feature = "executor_join_help")]
    class: Option<usize>,
}

struct Claim {
    index: usize,
    generation: u64,
    #[cfg(feature = "executor_join_help")]
    class: Option<usize>,
}

struct Cell<J, R>(UnsafeCell<MaybeUninit<Completion<J, R>>>);
impl<J, R> Cell<J, R> {
    fn pointer(&self) -> *mut Completion<J, R> {
        self.0.get().cast()
    }
}
// SAFETY: the module ownership protocol admits one cell writer/reader at a
// time. The mutex hands Queued ownership to a runner; release publication and
// acquire collection hand it to one collector; release VACANT and locked
// acquire admission hand it to the next submitter. No shared J/R references.
unsafe impl<J: Send, R: Send> Sync for Cell<J, R> {}

struct State {
    slots: Vec<Slot>,
    ready: VecDeque<usize>,
    generation: u64,
    started: usize,
    stop: bool,
    reserved_workers: usize,
    parked: usize,
    counters: SubmitCounters,
}

impl State {
    fn new(capacity: usize) -> Result<Self, &'static str> {
        let mut slots = Vec::new();
        let mut ready = VecDeque::new();
        slots
            .try_reserve_exact(capacity)
            .map_err(|_| "acquisition metadata allocation failed")?;
        ready
            .try_reserve_exact(capacity)
            .map_err(|_| "acquisition ready queue allocation failed")?;
        for _ in 0..capacity {
            slots.push(Slot {
                generation: 0,
                priority: 0,
                worker_only: false,
                callback_help: false,
                phase: Phase::Vacant,
                #[cfg(feature = "executor_join_help")]
                class: None,
            });
        }
        Ok(Self {
            slots,
            ready,
            generation: 0,
            started: 0,
            stop: false,
            reserved_workers: 0,
            parked: 0,
            counters: SubmitCounters::default(),
        })
    }

    fn free_slot<J, R, O: Observer>(&self, shared: &Shared<J, R, O>, worker_only: bool) -> Option<usize> {
        shared
            .words
            .iter()
            .enumerate()
            .rfind(|(index, value)| {
                (*index < self.reserved_workers) == worker_only
                    && value.load(Ordering::Acquire) & PHASE_MASK == VACANT
            })
            .map(|(index, _)| index)
    }

    fn claim<J, R, O: Observer>(&mut self, shared: &Shared<J, R, O>, position: usize) -> Claim {
        let index = self
            .ready
            .remove(position)
            .expect("checked acquisition ready position");
        shared.ready_count.fetch_sub(1, Ordering::Release);
        let slot = &mut self.slots[index];
        debug_assert!(slot.phase == Phase::Queued);
        slot.phase = Phase::Running;
        shared.active.fetch_add(1, Ordering::SeqCst);
        #[cfg(feature = "callback_profile")]
        // SAFETY: the queue mutex and removed ready entry give this claimant
        // exclusive ownership of the Queued cell, before its runner begins.
        unsafe {
            (*shared.cells[index].pointer()).profile.claimed_qpc =
                collector_primitives::clock::qpc();
        }
        shared.words[index].store(word(slot.generation, RUNNING), Ordering::SeqCst);
        Claim {
            index,
            generation: slot.generation,
            #[cfg(feature = "executor_join_help")]
            class: slot.class,
        }
    }

    fn take_ready<J, R, O: Observer>(
        &mut self,
        shared: &Shared<J, R, O>,
        awaited: Option<Ticket>,
    ) -> Option<Claim> {
        let position = if let Some(ticket) = awaited {
            let slot = self.slots.get(ticket.slot)?;
            if slot.generation != ticket.generation
                || slot.worker_only
                || slot.phase != Phase::Queued
                || shared.words[ticket.slot].load(Ordering::Acquire)
                    != word(ticket.generation, PENDING)
            {
                return None;
            }
            self.ready
                .iter()
                .position(|&index| index == ticket.slot)
                .expect("queued acquisition slot has a ready entry")
        } else {
            self.ready
                .iter()
                .enumerate()
                .min_by_key(|(_, index)| self.slots[**index].priority)
                .map(|(position, _)| position)?
        };
        Some(self.claim(shared, position))
    }

    fn take_child<J, R, O: Observer>(&mut self, shared: &Shared<J, R, O>, ticket: Ticket) -> Option<Claim> {
        let slot = self.slots.get(ticket.slot)?;
        if slot.generation != ticket.generation
            || slot.phase != Phase::Queued
            || shared.words[ticket.slot].load(Ordering::Acquire) != word(ticket.generation, PENDING)
        {
            return None;
        }
        let position = self.ready.iter().position(|&index| index == ticket.slot)?;
        Some(self.claim(shared, position))
    }

    fn stop<J, R, O: Observer>(&mut self, shared: &Shared<J, R, O>) {
        self.stop = true;
        shared.stopping.store(true, Ordering::Release);
        self.cancel_queued(shared);
    }

    fn cancel_queued<J, R, O: Observer>(&mut self, shared: &Shared<J, R, O>) {
        shared.ready_count.store(0, Ordering::Release);
        while let Some(index) = self.ready.pop_front() {
            let slot = &self.slots[index];
            debug_assert!(slot.phase == Phase::Queued);
            let completion = shared.cells[index].pointer();
            // SAFETY: cancellation holds the mutex and has removed the only
            // ready entry. No claimant can access this Queued cell. The job
            // and optional profile were initialized at submission.
            unsafe {
                std::ptr::addr_of_mut!((*completion).report).write(Err(Failure::Stopped));
                #[cfg(feature = "acquisition_profile")]
                std::ptr::addr_of_mut!((*completion).table_reads)
                    .write(collector_memory::read_accounting::ZERO);
            }
            add(&shared.counters.stopped, 1);
            add(&shared.counters.completed, 1);
            shared.publish(index, slot.generation);
        }
    }
}

type Runner<J, R> = dyn Fn(&mut J) -> R + Send + Sync;
type Classes<J> = (usize, fn(&J) -> usize);
struct BatchContext(UnsafeCell<collector_memory::terminal::Context>);
// SAFETY: writes are locked and require uncollected == 0 and active == 0.
// Both the native body and its detached continuation end before active retires.
unsafe impl Sync for BatchContext {}

struct Shared<J, R, O: Observer> {
    id: u64,
    state: Mutex<State>,
    started_ready: Condvar,
    work_ready: Condvar,
    reserve_ready: Condvar,
    regulars: AtomicUsize,
    called_up: AtomicBool,
    priority: AtomicI32,
    away: AtomicU32,
    hurried: AtomicU64,
    burst_peers: AtomicUsize,
    ready_count: AtomicUsize,
    stopping: AtomicBool,
    spin_ticks: u64,
    run: Box<Runner<J, R>>,
    callback_help: OnceLock<fn(&J) -> bool>,
    on_completed: OnceLock<OnCompleted<J, R>>,
    thread_ids: Vec<AtomicU32>,
    cells: Box<[Cell<J, R>]>,
    words: Box<[AtomicU64]>,
    active: AtomicUsize,
    uncollected: AtomicUsize,
    awaited: AtomicU64,
    worker_waiters: AtomicUsize,
    ready_waiting: AtomicBool,
    ready_wake: AtomicU64,
    settling: AtomicUsize,
    counters: Counters,
    observer: O,
    #[cfg(feature = "executor_join_help")]
    classify: Option<fn(&J) -> usize>,
    #[cfg(feature = "executor_join_help")]
    means: Box<[AtomicU64]>,
    #[cfg(feature = "executor_join_help")]
    maxima: Box<[AtomicU64]>,
    #[cfg(feature = "executor_join_help")]
    body_begin: Box<[AtomicU64]>,
    #[cfg(feature = "executor_join_help")]
    frequency: u64,
    join_failed: AtomicBool,
    context: BatchContext,
}

pub trait Observer: Send + Sync + 'static {
    fn waking_worker(&self) {}
    fn relaying_work(&self) {}
    fn waiting_for_job(&self) {}
    fn waiting_for_join(&self, _ticket: Ticket) {}
    #[cfg(feature = "executor_join_help")]
    fn attempting_join_help(&self) {}
    #[cfg(feature = "executor_join_help")]
    fn join_help_contended(&self) {}
}

pub struct Unobserved;

impl Observer for Unobserved {}

#[cfg(test)]
#[derive(Default)]
struct FixtureObserver {
    notified_one: AtomicU64,
    relayed_one: AtomicU64,
    before_join_wait: OnceLock<Box<dyn Fn(Ticket) + Send + Sync>>,
    before_work_wait: OnceLock<Box<dyn Fn() + Send + Sync>>,
    #[cfg(feature = "executor_join_help")]
    help_attempt_gate: HelpAttemptGate,
    #[cfg(feature = "executor_join_help")]
    help_lock_contentions: AtomicU64,
}

#[cfg(test)]
impl Observer for FixtureObserver {
    fn waking_worker(&self) {
        add(&self.notified_one, 1);
    }

    fn relaying_work(&self) {
        add(&self.relayed_one, 1);
    }

    fn waiting_for_job(&self) {
        if let Some(hook) = self.before_work_wait.get() {
            hook();
        }
    }

    fn waiting_for_join(&self, ticket: Ticket) {
        if let Some(hook) = self.before_join_wait.get() {
            hook(ticket);
        }
    }

    #[cfg(feature = "executor_join_help")]
    fn attempting_join_help(&self) {
        let gate = &self.help_attempt_gate;
        if gate.armed.swap(false, Ordering::SeqCst) {
            gate.entered.store(true, Ordering::Release);
            let until = std::time::Instant::now() + collector_test_support::TEST_HANG_GUARD;
            while !gate.released.load(Ordering::Acquire) && std::time::Instant::now() < until {
                std::thread::yield_now();
            }
        }
    }

    #[cfg(feature = "executor_join_help")]
    fn join_help_contended(&self) {
        add(&self.help_lock_contentions, 1);
    }
}

impl<J, R, O: Observer> Shared<J, R, O> {
    fn lock(&self) -> MutexGuard<'_, State> {
        lock_unpoisoned(&self.state)
    }
    fn wait_for_work<'a>(&self, state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.work_ready
            .wait(state)
            .unwrap_or_else(|error| error.into_inner())
    }
    fn stop(&self) {
        self.lock().stop(self);
        self.work_ready.notify_all();
        self.reserve_ready.notify_all();
    }
    fn working(&self) -> usize {
        self.regulars
            .load(Ordering::Acquire)
            .min(self.thread_ids.len())
    }

    fn publish(&self, index: usize, generation: u64) {
        #[cfg(feature = "callback_profile")]
        // SAFETY: called only by the unique Running claimant, or by locked
        // cancellation of Queued work. Every completion field is initialized
        // and exclusively owned until the SC release store below.
        unsafe {
            (*self.cells[index].pointer()).profile.published_qpc =
                collector_primitives::clock::qpc();
        }
        self.words[index].store(word(generation, PUBLISHED), Ordering::SeqCst);
        if self.ready_waiting.load(Ordering::SeqCst) {
            self.ready_wake.fetch_add(1, Ordering::SeqCst);
            unsafe { WakeByAddressSingle(self.ready_wake.as_ptr().cast()) };
        }
        if self.awaited.load(Ordering::SeqCst) == generation {
            unsafe { WakeByAddressSingle(self.words[index].as_ptr().cast()) };
        }
        if self.worker_waiters.load(Ordering::SeqCst) != 0 {
            unsafe { WakeByAddressAll(self.words[index].as_ptr().cast()) };
        }
    }

    fn wait_active(&self) {
        self.settling.fetch_add(1, Ordering::SeqCst);
        loop {
            let active = self.active.load(Ordering::SeqCst);
            if active == 0 {
                break;
            }
            unsafe {
                WaitOnAddress(
                    self.active.as_ptr().cast(),
                    (&active as *const usize).cast(),
                    size_of::<usize>(),
                    u32::MAX,
                )
            };
        }
        self.settling.fetch_sub(1, Ordering::SeqCst);
    }

    fn execute(&self, claim: Claim, caller: bool, help: bool) {
        let index = claim.index;
        let completion = self.cells[index].pointer();
        let _phase =
            collector_memory::terminal::enter(collector_memory::terminal::Phase::Acquisition);
        // SAFETY: claim was removed under the mutex, incrementing active and
        // retaining one uncollected admission. Context cannot reset until all
        // admissions are collected and every active publisher has retired.
        let borrowed_context =
            unsafe { collector_memory::terminal::borrow_context(self.context.0.get()) };
        #[cfg(feature = "acquisition_profile")]
        let reads = collector_memory::read_accounting::Capture::new();
        #[cfg(not(feature = "acquisition_profile"))]
        let reads = collector_memory::read_accounting::Scope::new();
        #[cfg(feature = "callback_profile")]
        let runner_thread_id = unsafe { GetCurrentThreadId() };
        #[cfg(any(feature = "callback_profile", feature = "executor_join_help"))]
        let runner_begin_qpc = collector_primitives::clock::qpc();
        #[cfg(feature = "executor_join_help")]
        self.body_begin[index].store(runner_begin_qpc.max(0) as u64, Ordering::Release);
        #[cfg(feature = "callback_profile")]
        let runner_cycles_begin = crate::callback_profile_state::thread_cycles();
        let mut report = if collector_memory::terminal::failed() {
            Err(Failure::Stopped)
        } else {
            // SAFETY: only this claimant owns the Running cell. The job was
            // initialized at admission; no move or other cell reference occurs
            // while its body runs. The borrow ends inside the unwind boundary.
            match catch_unwind(AssertUnwindSafe(|| unsafe {
                (self.run)(&mut (*completion).request)
            })) {
                Ok(report) if !collector_memory::terminal::failed() => Ok(report),
                Ok(_) => Err(Failure::Stopped),
                Err(payload) => {
                    std::mem::forget(payload);
                    Err(Failure::Panicked)
                }
            }
        };
        let publication = match catch_unwind(AssertUnwindSafe(|| {
            self.on_completed.get().and_then(|completed| {
                // SAFETY: the claimant still exclusively owns this Running cell.
                unsafe { completed(&mut (*completion).request, &report) }
            })
        })) {
            Ok(publication) => publication,
            Err(payload) => {
                std::mem::forget(payload);
                report = Err(Failure::Panicked);
                None
            }
        };
        #[cfg(feature = "callback_profile")]
        let runner_cycles_end = crate::callback_profile_state::thread_cycles();
        #[cfg(any(feature = "callback_profile", feature = "executor_join_help"))]
        let runner_end_qpc = collector_primitives::clock::qpc();
        drop(borrowed_context);
        if let Err(failure) = report {
            add(
                if failure == Failure::Panicked {
                    &self.counters.panicked
                } else {
                    &self.counters.stopped
                },
                1,
            );
            self.lock().stop(self);
            self.work_ready.notify_all();
        }
        // SAFETY: still the sole Running claimant, with the job borrow ended.
        // Initialize report/counts and update only already-initialized stages.
        unsafe {
            std::ptr::addr_of_mut!((*completion).report).write(report);
            #[cfg(feature = "acquisition_profile")]
            {
                std::ptr::addr_of_mut!((*completion).table_reads)
                    .write(collector_memory::read_accounting::ZERO);
                reads.finish_into(&mut (*completion).table_reads);
            }
            #[cfg(not(feature = "acquisition_profile"))]
            drop(reads);
            #[cfg(feature = "callback_profile")]
            {
                (*completion).profile.runner_begin_qpc = runner_begin_qpc;
                (*completion).profile.runner_end_qpc = runner_end_qpc;
                (*completion).profile.runner_thread_id = runner_thread_id;
                (*completion).profile.runner_thread_cycles = runner_cycles_begin
                    .zip(runner_cycles_end)
                    .and_then(|(begin, end)| end.checked_sub(begin));
            }
        }
        #[cfg(feature = "executor_join_help")]
        {
            let ticks = runner_end_qpc.saturating_sub(runner_begin_qpc).max(1) as u64;
            if let Some(class) = claim.class {
                let _ =
                    self.means[class].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
                        Some(if old == 0 {
                            ticks
                        } else if ticks >= old {
                            old + (ticks - old) / 8
                        } else {
                            old - (old - ticks) / 8
                        })
                    });
                let _ =
                    self.maxima[class].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
                        Some(ticks.max(old - old / 16))
                    });
            }
            if help {
                add(&self.counters.help_executed, 1);
                add(
                    &self.counters.help_ns,
                    (u128::from(ticks) * 1_000_000_000 / u128::from(self.frequency))
                        .min(u128::from(u64::MAX)) as u64,
                );
            }
        }
        #[cfg(not(feature = "executor_join_help"))]
        let _ = help;
        add(&self.counters.completed, 1);
        add(
            if caller {
                &self.counters.caller_executed
            } else {
                &self.counters.worker_executed
            },
            1,
        );
        self.publish(index, claim.generation);
        if let Some(publication) = publication {
            // SAFETY: active still retains this batch's immutable context.
            let _context =
                unsafe { collector_memory::terminal::borrow_context(self.context.0.get()) };
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| publication.published())) {
                std::mem::forget(payload);
                if let Err(payload) = catch_unwind(AssertUnwindSafe(|| publication.failed())) {
                    std::mem::forget(payload);
                }
                add(&self.counters.panicked, 1);
                self.stop();
            }
        }
        let active = self.active.fetch_sub(1, Ordering::SeqCst) - 1;
        if active == 0 && self.settling.load(Ordering::SeqCst) != 0 {
            // SAFETY: active lives until every runner/waiter releases Shared.
            unsafe { WakeByAddressAll(self.active.as_ptr().cast()) };
        }
    }

    #[cfg(feature = "executor_join_help")]
    fn try_help(&self, awaited: Ticket, budget: &mut HelpBudget) -> Option<Claim> {
        if !budget.attempt() {
            return None;
        }
        self.classify?;
        self.observer.attempting_join_help();
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                self.observer.join_help_contended();
                return None;
            }
        };
        if state.stop
            || collector_memory::terminal::failed()
            || self.words[awaited.slot].load(Ordering::Acquire) != word(awaited.generation, RUNNING)
        {
            return None;
        }
        let slot = &state.slots[awaited.slot];
        if slot.generation != awaited.generation || slot.phase != Phase::Running {
            return None;
        }
        let class = slot.class?;
        let mean = self.means[class].load(Ordering::Relaxed);
        let begin = self.body_begin[awaited.slot].load(Ordering::Acquire);
        if mean == 0 || begin == 0 {
            return None;
        }
        let elapsed = (collector_primitives::clock::qpc().max(0) as u64).saturating_sub(begin);
        let remaining = mean.saturating_sub(elapsed);
        let available = *budget.ticks.get_or_insert(remaining / 2);
        let position = state
            .ready
            .iter()
            .enumerate()
            .filter(|(_, index)| {
                let candidate = &state.slots[**index];
                if candidate.worker_only {
                    return false;
                }
                let Some(class) = candidate.class else {
                    return false;
                };
                let candidate_maximum = self.maxima[class].load(Ordering::Relaxed);
                candidate_maximum != 0
                    && candidate_maximum <= remaining / 2
                    && candidate_maximum <= available
            })
            .min_by_key(|(_, index)| state.slots[**index].priority)
            .map(|(position, _)| position)?;
        let candidate = &state.slots[state.ready[position]];
        let candidate_maximum = self.maxima[candidate.class?].load(Ordering::Relaxed);
        let elapsed = (collector_primitives::clock::qpc().max(0) as u64).saturating_sub(begin);
        let remaining = self.means[class]
            .load(Ordering::Relaxed)
            .saturating_sub(elapsed);
        if candidate_maximum == 0
            || candidate_maximum > remaining / 2
            || candidate_maximum > available
            || self.words[awaited.slot].load(Ordering::Acquire) != word(awaited.generation, RUNNING)
        {
            return None;
        }
        budget.ticks = Some(available - candidate_maximum);
        Some(state.claim(self, position))
    }
}

impl<J, R, O: Observer> Drop for Shared<J, R, O> {
    fn drop(&mut self) {
        for (cell, state) in self.cells.iter().zip(&self.words) {
            let dropped = catch_unwind(AssertUnwindSafe(|| {
                match state.load(Ordering::Relaxed) & PHASE_MASK {
                    VACANT => {}
                    PENDING | RUNNING => {
                        // SAFETY: last Shared owner: no runner, submitter, unpark
                        // or collector remains. Every pending/running cell has a job;
                        // other fields may be uninitialized after an executor
                        // failure, so only drop the owned job.
                        unsafe {
                            std::ptr::addr_of_mut!((*cell.pointer()).request).drop_in_place()
                        };
                    }
                    PUBLISHED => {
                        // SAFETY: last owner, fully initialized at publication;
                        // this completion was never moved out by collection.
                        unsafe { cell.pointer().drop_in_place() };
                    }
                    _ => unreachable!("last acquisition owner cannot still be collecting"),
                }
            }));
            if let Err(payload) = dropped {
                std::mem::forget(payload);
            }
        }
    }
}

pub struct Pool<J: Send + 'static, R: Copy + Send + 'static, O: Observer = Unobserved> {
    shared: Arc<Shared<J, R, O>>,
    threads: Mutex<Vec<JoinHandle<()>>>,
    allocated_bytes: usize,
}

impl<J: Send + 'static, R: Copy + Send + 'static> Pool<J, R> {
    #[cfg(any(test, feature = "test-support"))]
    /// # Safety
    /// Spawned workers register with `collector_timing::capture_gate::reader()`, exempting them
    /// from blocking on the process-wide capture gate. Caller must hold active collector session
    /// ownership (or exclusive fixture control) and stop or drop the pool before teardown releases
    /// shared capture-gate or native resources.
    #[cfg(any(test, feature = "test-support"))]
    pub unsafe fn start(
        worker_count: usize,
        capacity: usize,
        run: impl Fn(&mut J) -> R + Send + Sync + 'static,
    ) -> Result<Self, &'static str> {
        // SAFETY: Caller upholds the pool startup contract.
        unsafe { Self::start_boosted(worker_count, capacity, false, run) }
    }
    /// # Safety
    /// Spawned workers register with `collector_timing::capture_gate::reader()`, exempting them
    /// from blocking on the process-wide capture gate. Caller must hold active collector session
    /// ownership (or exclusive fixture control) and stop or drop the pool before teardown releases
    /// shared capture-gate or native resources.
    pub unsafe fn start_with_class(
        worker_count: usize,
        capacity: usize,
        classes: Option<Classes<J>>,
        run: impl Fn(&mut J) -> R + Send + Sync + 'static,
    ) -> Result<Self, &'static str> {
        // SAFETY: Caller upholds the pool startup contract.
        unsafe { Self::start_classified_boosted(worker_count, capacity, false, classes, Unobserved, run) }
    }
    /// # Safety
    /// Same requirements as [`Pool::start_with_class`].
    #[cfg(any(test, feature = "test-support"))]
    unsafe fn start_boosted(
        worker_count: usize,
        capacity: usize,
        wake_boost: bool,
        run: impl Fn(&mut J) -> R + Send + Sync + 'static,
    ) -> Result<Self, &'static str> {
        // SAFETY: Caller upholds the pool startup contract.
        unsafe { Self::start_classified_boosted(worker_count, capacity, wake_boost, None, Unobserved, run) }
    }
}

#[cfg(test)]
impl<J: Send + 'static, R: Copy + Send + 'static> Pool<J, R, FixtureObserver> {
    /// # Safety
    /// The fixture has exclusive control of its workers through shutdown.
    unsafe fn start_fixture(
        worker_count: usize,
        capacity: usize,
        classes: Option<Classes<J>>,
        run: impl Fn(&mut J) -> R + Send + Sync + 'static,
    ) -> Result<Self, &'static str> {
        // SAFETY: The fixture owns these workers through shutdown.
        unsafe {
            Self::start_classified_boosted(
                worker_count, capacity, false, classes, FixtureObserver::default(), run,
            )
        }
    }
}

impl<J: Send + 'static, R: Copy + Send + 'static, O: Observer> Pool<J, R, O> {
    pub fn set_on_completed(&self, completed: OnCompleted<J, R>) {
        assert!(self.shared.on_completed.set(completed).is_ok());
    }
    /// # Safety
    /// Same requirements as [`Pool::start_with_class`].
    unsafe fn start_classified_boosted(
        worker_count: usize,
        capacity: usize,
        wake_boost: bool,
        classes: Option<Classes<J>>,
        observer: O,
        run: impl Fn(&mut J) -> R + Send + Sync + 'static,
    ) -> Result<Self, &'static str> {
        if worker_count > MAX_WORKERS || capacity == 0 {
            return Err("acquisition pool requires an admitted worker count and nonzero capacity");
        }
        let id = NEXT_POOL_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| "acquisition pool identity exhausted")?;
        let state = State::new(capacity)?;
        let mut cells = Vec::new();
        cells
            .try_reserve_exact(capacity)
            .map_err(|_| "acquisition cell allocation failed")?;
        for _ in 0..capacity {
            cells.push(Cell(UnsafeCell::new(MaybeUninit::uninit())));
        }
        let mut words = Vec::new();
        words
            .try_reserve_exact(capacity)
            .map_err(|_| "acquisition word allocation failed")?;
        for _ in 0..capacity {
            words.push(AtomicU64::new(VACANT));
        }
        let mut threads = Vec::new();
        threads
            .try_reserve_exact(worker_count)
            .map_err(|_| "acquisition thread handle allocation failed")?;
        let mut thread_ids = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            thread_ids.push(AtomicU32::new(0));
        }
        #[cfg(feature = "executor_join_help")]
        let (classify, means, maxima, body_begin, frequency) = {
            let mut means = Vec::new();
            means
                .try_reserve_exact(classes.map_or(0, |(count, _)| count))
                .map_err(|_| "acquisition class history allocation failed")?;
            for _ in 0..classes.map_or(0, |(count, _)| count) {
                means.push(AtomicU64::new(0));
            }
            let mut maxima = Vec::new();
            maxima
                .try_reserve_exact(means.len())
                .map_err(|_| "acquisition class maximum allocation failed")?;
            for _ in 0..means.len() {
                maxima.push(AtomicU64::new(0));
            }
            let mut body_begin = Vec::new();
            body_begin
                .try_reserve_exact(capacity)
                .map_err(|_| "acquisition body clock allocation failed")?;
            for _ in 0..capacity {
                body_begin.push(AtomicU64::new(0));
            }
            let mut frequency = 0_i64;
            if unsafe {
                windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency)
            } == 0
                || frequency <= 0
            {
                return Err("acquisition body clock frequency unavailable");
            }
            (
                classes.map(|(_, classify)| classify),
                means.into_boxed_slice(),
                maxima.into_boxed_slice(),
                body_begin.into_boxed_slice(),
                frequency as u64,
            )
        };
        #[cfg(not(feature = "executor_join_help"))]
        let _ = classes;
        let spin_ticks = {
            let mut frequency = 0_i64;
            if unsafe {
                windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency)
            } != 0
                && frequency > 0
            {
                (frequency as u64 / 25_000).max(1)
            } else {
                0
            }
        };
        let shared = Arc::new(Shared {
            id,
            state: Mutex::new(state),
            started_ready: Condvar::new(),
            work_ready: Condvar::new(),
            reserve_ready: Condvar::new(),
            regulars: AtomicUsize::new(usize::MAX),
            called_up: AtomicBool::new(false),
            priority: AtomicI32::new(THREAD_PRIORITY_ABOVE_NORMAL),
            away: AtomicU32::new(0),
            hurried: AtomicU64::new(0),
            burst_peers: AtomicUsize::new(0),
            ready_count: AtomicUsize::new(0),
            stopping: AtomicBool::new(false),
            spin_ticks,
            run: Box::new(run),
            callback_help: OnceLock::new(),
            on_completed: OnceLock::new(),
            thread_ids,
            cells: cells.into_boxed_slice(),
            words: words.into_boxed_slice(),
            active: AtomicUsize::new(0),
            uncollected: AtomicUsize::new(0),
            awaited: AtomicU64::new(0),
            worker_waiters: AtomicUsize::new(0),
            ready_waiting: AtomicBool::new(false),
            ready_wake: AtomicU64::new(0),
            settling: AtomicUsize::new(0),
            counters: Counters::default(),
            observer,
            #[cfg(feature = "executor_join_help")]
            classify,
            #[cfg(feature = "executor_join_help")]
            means,
            #[cfg(feature = "executor_join_help")]
            maxima,
            #[cfg(feature = "executor_join_help")]
            body_begin,
            #[cfg(feature = "executor_join_help")]
            frequency,
            join_failed: AtomicBool::new(false),
            context: BatchContext(UnsafeCell::new(
                collector_memory::terminal::Context::default(),
            )),
        });
        let allocated_bytes = {
            let state = shared.lock();
            let arc_layout = std::alloc::Layout::new::<[AtomicUsize; 2]>()
                .extend(std::alloc::Layout::new::<Shared<J, R, O>>())
                .expect("admitted acquisition shared layout")
                .0
                .pad_to_align();
            let bytes = arc_layout.size()
                + size_of_val(&*shared.cells)
                + size_of_val(&*shared.words)
                + state.slots.capacity() * size_of::<Slot>()
                + state.ready.capacity() * size_of::<usize>()
                + shared.thread_ids.capacity() * size_of::<AtomicU32>()
                + threads.capacity() * size_of::<JoinHandle<()>>()
                + size_of_val(&*shared.run);
            #[cfg(feature = "executor_join_help")]
            let bytes = bytes
                + size_of_val(&*shared.means)
                + size_of_val(&*shared.maxima)
                + size_of_val(&*shared.body_begin);
            bytes
        };
        let pool = Self {
            shared,
            threads: Mutex::new(threads),
            allocated_bytes,
        };
        for index in 0..worker_count {
            let shared = Arc::clone(&pool.shared);
            match std::thread::Builder::new()
                .name(format!("native-acquisition-{index}"))
                .stack_size(WORKER_STACK_BYTES)
                .spawn(move || worker_loop(shared, index, wake_boost))
            {
                Ok(thread) => pool
                    .threads
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(thread),
                Err(_) => {
                    let _ =
                    // SAFETY: This is the pool's owner. No submitted work is still borrowing its
                    // storage at shutdown.
                    unsafe { pool.shutdown() };
                    return Err("start native acquisition worker failed");
                }
            }
        }
        let mut state = pool.shared.lock();
        while state.started != worker_count {
            state = pool
                .shared
                .started_ready
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
        drop(state);
        Ok(pool)
    }

    /// # Safety
    /// As [`Self::submit_with_priority`].
    #[cfg(any(test, feature = "test-support"))]
    pub unsafe fn submit(&self, request: &mut Option<J>) -> Result<Ticket, SubmitError> {
        unsafe { self.submit_with_priority(request, 0, false, true) }
    }
    pub fn reserve_worker_slots(&self, count: usize) -> Result<(), &'static str> {
        let mut state = self.shared.lock();
        if self.shared.thread_ids.is_empty() || state.generation != 0 {
            return Err("worker-only acquisition requires a worker before admission");
        }
        if count == 0 || count > state.slots.len() {
            return Err("worker-only reservation exceeds available slots");
        }
        state.reserved_workers = count;
        Ok(())
    }

    /// # Safety
    /// Keep the foreign live-read lease until EVERY accepted ticket is collected,
    /// active publishers have settled and every successor they admitted has
    /// drained, including on failure and unwinding. Mutable outputs must be
    /// disjoint; shared seeds live through all consumers. Each ticket has one
    /// collecting owner. Nested workers collect only their children through
    /// finish_worker_into.
    pub unsafe fn submit_with_priority(
        &self,
        request: &mut Option<J>,
        priority: u8,
        worker_only: bool,
        wake: bool,
    ) -> Result<Ticket, SubmitError> {
        #[cfg(feature = "callback_profile")]
        let submit_lock_begin = collector_primitives::clock::qpc();
        let mut state = self.shared.lock();
        #[cfg(feature = "callback_profile")]
        let submit_lock_wait_ticks = collector_primitives::clock::qpc()
            .saturating_sub(submit_lock_begin)
            .max(0) as u64;
        let free = state.free_slot(&self.shared, worker_only);
        let rejection = if request.is_none() {
            Some(SubmitError::Empty)
        } else if state.stop || collector_memory::terminal::failed() {
            Some(SubmitError::Stopped)
        } else if free.is_none() {
            Some(SubmitError::Full)
        } else if state.generation == MAX_GENERATION {
            Some(SubmitError::Exhausted)
        } else {
            None
        };
        if let Some(rejection) = rejection {
            state.counters.rejected = state.counters.rejected.saturating_add(1);
            return Err(rejection);
        }
        if self.shared.uncollected.load(Ordering::Acquire) == 0
            && self.shared.active.load(Ordering::Acquire) == 0
        {
            // SAFETY: no runner or detached continuation can still borrow
            // this context. Parked jobs never run. The mutex excludes any
            // competing claim, admission or reset.
            unsafe { *self.shared.context.0.get() = collector_memory::terminal::context() };
        }
        let index = free.expect("checked free acquisition slot");
        state.generation += 1;
        let generation = state.generation;
        let slot = &mut state.slots[index];
        slot.generation = generation;
        slot.priority = priority;
        slot.worker_only = worker_only;
        slot.callback_help = self
            .shared
            .callback_help
            .get()
            .is_some_and(|eligible| eligible(request.as_ref().expect("checked request")));
        slot.phase = Phase::Queued;
        #[cfg(feature = "executor_join_help")]
        {
            slot.class = self
                .shared
                .classify
                .map(|classify| classify(request.as_ref().expect("checked request")))
                .filter(|&class| class < self.shared.means.len());
            self.shared.body_begin[index].store(0, Ordering::Relaxed);
        }
        let completion = self.shared.cells[index].pointer();
        // SAFETY: locked admission acquired this word's VACANT release; the
        // preceding collector/unpark finished its read and owns the old job.
        // The cell holds no initialized owned value. Initialize each field
        // separately, without constructing or moving a whole completion.
        unsafe {
            std::ptr::addr_of_mut!((*completion).request)
                .write(request.take().expect("checked owned acquisition request"));
            #[cfg(feature = "callback_profile")]
            std::ptr::addr_of_mut!((*completion).profile).write(Profile {
                queued_qpc: collector_primitives::clock::qpc(),
                submit_lock_wait_ticks,
                ..Profile::default()
            });
        }
        self.shared.uncollected.fetch_add(1, Ordering::Relaxed);
        self.shared.words[index].store(word(generation, PENDING), Ordering::Release);
        state.ready.push_back(index);
        self.shared.ready_count.fetch_add(1, Ordering::Release);
        state.counters.accepted = state.counters.accepted.saturating_add(1);
        drop(state);
        if wake {
            self.shared.work_ready.notify_one();
        }
        Ok(Ticket {
            pool: self.shared.id,
            slot: index,
            generation,
        })
    }

    pub fn settle(&self) {
        self.shared.lock().cancel_queued(&self.shared);
        self.shared.wait_active();
    }
    pub fn wake_workers(&self, jobs: usize) {
        if jobs == 0 {
            return;
        }
        if jobs >= 6 {
            self.shared.burst_peers.store(0, Ordering::Release);
            self.shared.observer.waking_worker();
            self.shared.work_ready.notify_all();
            return;
        }
        let limit = self.shared.working().saturating_sub(1);
        let peers = jobs.saturating_sub(1).min(limit);
        if peers != 0 {
            let _ =
                self.shared
                    .burst_peers
                    .fetch_update(Ordering::Release, Ordering::Relaxed, |old| {
                        Some(old.saturating_add(peers).min(limit))
                    });
        }
        self.shared.observer.waking_worker();
        self.shared.work_ready.notify_one();
    }
    pub fn hold_pace(&self) {
        self.hold_pace_with(MAX_WORKERS);
    }

    pub fn hold_pace_with(&self, target: usize) {
        let target = target.max(1);
        if self.shared.called_up.load(Ordering::Acquire)
            && self.shared.regulars.load(Ordering::Acquire) == target
            && self.shared.priority.load(Ordering::Acquire) == THREAD_PRIORITY_ABOVE_NORMAL
            && self.shared.away.load(Ordering::Acquire) == 0
        {
            return;
        }
        let (prev_regulars, clear_away) = {
            let state = self.shared.lock();
            let clear_away = self.shared.away.load(Ordering::Acquire) != 0;
            let prev_regulars = self.shared.regulars.load(Ordering::Acquire);
            self.pace(
                state,
                target,
                THREAD_PRIORITY_ABOVE_NORMAL,
                true,
                None,
            );
            (prev_regulars, clear_away)
        };
        if let Ok(threads) = self.threads.try_lock() {
            let limit = prev_regulars.min(target).min(threads.len());
            for thread in threads.iter().take(limit) {
                if !self.shared.called_up.load(Ordering::Acquire) {
                    break;
                }
                let handle = thread.as_raw_handle();
                // SAFETY: a live worker handle owned by `threads`, held through these calls.
                if unsafe { SetThreadPriority(handle as _, THREAD_PRIORITY_ABOVE_NORMAL) } == 0 {
                    PRIORITY_REFUSALS.fetch_add(1, Ordering::Relaxed);
                }
                if clear_away {
                    // SAFETY: `thread` is a live collector worker handle owned by `threads`.
                    unsafe { crate::cpu_sets::select_away(thread.as_handle(), None) };
                }
            }
            self.shared.hurried.fetch_add(1, Ordering::Release);
        }
    }
    pub fn beside_pace(&self, regulars: usize, game: Option<u32>) {
        let regulars = regulars.max(1);
        let away = game.map_or(0, |processor| processor.saturating_add(1));
        if !self.shared.called_up.load(Ordering::Acquire)
            && self.shared.regulars.load(Ordering::Acquire) == regulars
            && self.shared.priority.load(Ordering::Acquire) == THREAD_PRIORITY_NORMAL
            && self.shared.away.load(Ordering::Acquire) == away
        {
            return;
        }
        let state = self.shared.lock();
        self.pace(state, regulars, THREAD_PRIORITY_NORMAL, false, game);
    }
    fn pace(
        &self,
        state: MutexGuard<'_, State>,
        regulars: usize,
        priority: i32,
        called_up: bool,
        away: Option<u32>,
    ) {
        let previous_regulars = self.shared.regulars.swap(regulars, Ordering::AcqRel);
        self.shared.priority.store(priority, Ordering::Release);
        self.shared.away.store(
            away.map_or(0, |processor| processor.saturating_add(1)),
            Ordering::Release,
        );
        let rested = !self.shared.called_up.swap(called_up, Ordering::AcqRel);
        drop(state);
        if regulars > previous_regulars || (called_up && rested) {
            self.shared.reserve_ready.notify_all();
        }
        if regulars < previous_regulars || (!called_up && !rested) {
            self.shared.work_ready.notify_all();
        }
    }
    pub fn park(&self, request: &mut Option<J>, worker_only: bool) -> Option<Ticket> {
        let mut state = self.shared.lock();
        if request.is_none() || state.generation == MAX_GENERATION {
            return None;
        }
        let index = state.free_slot(&self.shared, worker_only).or_else(|| {
            self.shared
                .words
                .iter()
                .rposition(|value| value.load(Ordering::Acquire) & PHASE_MASK == VACANT)
        })?;
        state.generation += 1;
        let generation = state.generation;
        let slot = &mut state.slots[index];
        slot.generation = generation;
        slot.priority = 0;
        slot.worker_only = worker_only;
        slot.phase = Phase::Parked;
        // SAFETY: locked park acquired VACANT and exclusively owns the empty
        // cell. Parked has only an initialized job; no completion is exposed.
        unsafe {
            std::ptr::addr_of_mut!((*self.shared.cells[index].pointer()).request)
                .write(request.take().expect("checked parked request"))
        };
        self.shared.words[index].store(word(generation, PENDING), Ordering::Release);
        state.parked += 1;
        Some(Ticket {
            pool: self.shared.id,
            slot: index,
            generation,
        })
    }
    pub fn unpark(&self, ticket: Ticket) -> Option<J> {
        if ticket.pool != self.shared.id {
            return None;
        }
        let mut state = self.shared.lock();
        let slot = state.slots.get_mut(ticket.slot)?;
        if slot.generation != ticket.generation
            || slot.phase != Phase::Parked
            || self.shared.words[ticket.slot].load(Ordering::Acquire)
                != word(ticket.generation, PENDING)
        {
            return None;
        }
        // SAFETY: locked, matching Parked cell owns exactly this job and no
        // runner/collector can access it. Read once; release VACANT afterwards.
        let request = unsafe {
            std::ptr::addr_of!((*self.shared.cells[ticket.slot].pointer()).request).read()
        };
        slot.phase = Phase::Vacant;
        self.shared.words[ticket.slot].store(word(ticket.generation, VACANT), Ordering::Release);
        state.parked -= 1;
        Some(request)
    }

    #[inline(never)]
    pub fn finish_parked_into(
        &self,
        ticket: Ticket,
        out: &mut Option<Completion<J, R>>,
        complete: impl FnOnce(&mut J) -> Option<Result<R, Failure>>,
    ) -> bool {
        assert!(out.is_none(), "parked completion output must be empty");
        if ticket.pool != self.shared.id {
            return false;
        }
        let mut state = self.shared.lock();
        let Some(slot) = state.slots.get_mut(ticket.slot) else {
            return false;
        };
        if slot.generation != ticket.generation
            || slot.phase != Phase::Parked
            || self.shared.words[ticket.slot].load(Ordering::Acquire)
                != word(ticket.generation, PENDING)
        {
            return false;
        }
        let cell = self.shared.cells[ticket.slot].pointer();
        // SAFETY: the matching parked cell has one initialized request, owned
        // exclusively under the mutex. A panicking handler leaves it parked,
        // so the batch's drain still owns and drops it exactly once.
        let Some(report) = complete(unsafe { &mut (*cell).request }) else {
            return false;
        };
        // SAFETY: initialize the other fields in the heap cell, then move its
        // sole completion into empty output before releasing the cell. The
        // promoted zero array is copied directly without a stack temporary.
        unsafe {
            std::ptr::addr_of_mut!((*cell).report).write(report);
            #[cfg(feature = "acquisition_profile")]
            std::ptr::copy_nonoverlapping(
                &collector_memory::read_accounting::ZERO,
                std::ptr::addr_of_mut!((*cell).table_reads),
                1,
            );
            #[cfg(feature = "callback_profile")]
            std::ptr::addr_of_mut!((*cell).profile).write(Profile::default());
            std::ptr::write(out, Some(std::ptr::read(cell)));
        }
        slot.phase = Phase::Vacant;
        self.shared.words[ticket.slot].store(word(ticket.generation, VACANT), Ordering::Release);
        state.parked -= 1;
        true
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn finish(&self, ticket: Ticket) -> Option<Completion<J, R>> {
        let mut out = None;
        self.finish_into(ticket, &mut out);
        out
    }
    pub fn finish_into(&self, ticket: Ticket, out: &mut Option<Completion<J, R>>) -> bool {
        let begin = std::time::Instant::now();
        let collected = self.finish_into_timed(ticket, out);
        if collected {
            let wall = begin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
            JOINS.fetch_add(1, Ordering::Relaxed);
            JOIN_WALL_NS.fetch_add(wall, Ordering::Relaxed);
            JOIN_WALL_MAX_NS.fetch_max(wall, Ordering::Relaxed);
        }
        collected
    }
    pub fn run_unclaimed_child(&self, ticket: Ticket) -> bool {
        if ticket.pool != self.shared.id
            || self
                .shared
                .words
                .get(ticket.slot)
                .map(|value| value.load(Ordering::Acquire))
                != Some(word(ticket.generation, PENDING))
        {
            return false;
        }
        let claim = self.shared.lock().take_child(&self.shared, ticket);
        let Some(claim) = claim else {
            return false;
        };
        self.shared.execute(claim, true, false);
        true
    }
    pub fn finish_worker_into(&self, ticket: Ticket, out: &mut Option<Completion<J, R>>) -> bool {
        assert!(out.is_none());
        if ticket.pool != self.shared.id || ticket.slot >= self.shared.words.len() {
            return false;
        }
        struct Waiting<'a>(&'a AtomicUsize);
        impl Drop for Waiting<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        self.shared.worker_waiters.fetch_add(1, Ordering::SeqCst);
        let _waiting = Waiting(&self.shared.worker_waiters);
        let value = &self.shared.words[ticket.slot];
        loop {
            if self.collect(ticket, out, 0) {
                return true;
            }
            let observed = value.load(Ordering::SeqCst);
            if observed == word(ticket.generation, PUBLISHED) {
                continue;
            }
            if observed != word(ticket.generation, PENDING)
                && observed != word(ticket.generation, RUNNING)
            {
                return false;
            }
            unsafe {
                WaitOnAddress(
                    value.as_ptr().cast(),
                    (&observed as *const u64).cast(),
                    size_of::<u64>(),
                    u32::MAX,
                )
            };
        }
    }

    fn collect(&self, ticket: Ticket, out: &mut Option<Completion<J, R>>, lock_ticks: u64) -> bool {
        let Some(value) = self.shared.words.get(ticket.slot) else {
            return false;
        };
        if value
            .compare_exchange(
                word(ticket.generation, PUBLISHED),
                word(ticket.generation, COLLECTING),
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_err()
        {
            return false;
        }
        let completion = self.shared.cells[ticket.slot].pointer();
        // SAFETY: acquire CAS synchronizes with fully initialized publication
        // and gives this collector exclusive ownership. Racing/stale tickets
        // cannot read the cell. Move straight into empty out, without Drop;
        // no cell references survive the following VACANT release.
        unsafe {
            #[cfg(feature = "callback_profile")]
            {
                (*completion).profile.collected_qpc = collector_primitives::clock::qpc();
                (*completion).profile.finish_lock_wait_ticks = lock_ticks;
            }
            std::ptr::write(out, Some(std::ptr::read(completion)));
        }
        #[cfg(not(feature = "callback_profile"))]
        let _ = lock_ticks;
        value.store(word(ticket.generation, VACANT), Ordering::Release);
        self.shared.uncollected.fetch_sub(1, Ordering::Release);
        true
    }
    fn finish_into_timed(&self, ticket: Ticket, out: &mut Option<Completion<J, R>>) -> bool {
        debug_assert!(
            out.is_none(),
            "collector storage holds an uncollected completion"
        );
        if ticket.pool != self.shared.id || ticket.slot >= self.shared.words.len() {
            return false;
        }
        if self.collect(ticket, out, 0) {
            return true;
        }
        #[cfg(feature = "callback_profile")]
        let finish_lock_begin = collector_primitives::clock::qpc();
        let mut state = self.shared.lock();
        #[cfg(feature = "callback_profile")]
        let finish_lock_ticks = collector_primitives::clock::qpc()
            .saturating_sub(finish_lock_begin)
            .max(0) as u64;
        #[cfg(not(feature = "callback_profile"))]
        let finish_lock_ticks = 0;
        let slot = &state.slots[ticket.slot];
        let value = self.shared.words[ticket.slot].load(Ordering::Acquire);
        if slot.generation != ticket.generation
            || slot.phase == Phase::Parked
            || (value != word(ticket.generation, PENDING)
                && value != word(ticket.generation, RUNNING)
                && value != word(ticket.generation, PUBLISHED))
        {
            return false;
        }
        if value == word(ticket.generation, PENDING)
            && let Some(claim) = state.take_ready(&self.shared, Some(ticket))
        {
            drop(state);
            self.shared.execute(claim, true, false);
            return self.collect(ticket, out, finish_lock_ticks);
        }
        drop(state);
        let value = &self.shared.words[ticket.slot];
        #[cfg(feature = "executor_join_help")]
        let mut help_budget = HelpBudget::new();
        loop {
            if self.collect(ticket, out, finish_lock_ticks) {
                return true;
            }
            let observed = value.load(Ordering::Acquire);
            if observed == word(ticket.generation, PUBLISHED) {
                continue;
            }
            if observed != word(ticket.generation, PENDING)
                && observed != word(ticket.generation, RUNNING)
            {
                return false;
            }
            #[cfg(feature = "executor_join_help")]
            if let Some(claim) = self.shared.try_help(ticket, &mut help_budget) {
                self.shared.execute(claim, true, true);
                continue;
            }
            if self.help_callback() {
                continue;
            }
            self.shared
                .awaited
                .store(ticket.generation, Ordering::SeqCst);
            let observed = value.load(Ordering::SeqCst);
            if observed == word(ticket.generation, PENDING)
                || observed == word(ticket.generation, RUNNING)
            {
                add(&self.shared.counters.join_sleeps, 1);
                self.shared.observer.waiting_for_join(ticket);
                unsafe {
                    WaitOnAddress(
                        value.as_ptr().cast(),
                        (&observed as *const u64).cast(),
                        size_of::<u64>(),
                        u32::MAX,
                    )
                };
            }
            self.shared.awaited.store(0, Ordering::SeqCst);
        }
    }
    pub fn allocated_bytes(&self) -> usize {
        self.allocated_bytes
    }
    pub fn thread_ids(&self) -> Vec<u32> {
        self.shared
            .thread_ids
            .iter()
            .map(|id| id.load(Ordering::Acquire))
            .collect()
    }
    pub fn calibration(&self) -> Calibration {
        let mut thread_ids = ThreadIds::default();
        thread_ids.len = self.shared.thread_ids.len();
        for (out, id) in thread_ids.values.iter_mut().zip(&self.shared.thread_ids) {
            *out = id.load(Ordering::Acquire);
        }
        let state = self.shared.lock();
        let counters = &self.shared.counters;
        Calibration {
            thread_ids,
            capacity: state.slots.len(),
            queued: state.ready.len(),
            active: self.shared.active.load(Ordering::Acquire),
            uncollected: self.shared.uncollected.load(Ordering::Acquire),
            accepted: state.counters.accepted,
            rejected: state.counters.rejected,
            completed: counters.completed.load(Ordering::Relaxed),
            panicked: counters.panicked.load(Ordering::Relaxed),
            stopped: counters.stopped.load(Ordering::Relaxed),
            caller_executed: counters.caller_executed.load(Ordering::Relaxed),
            worker_executed: counters.worker_executed.load(Ordering::Relaxed),
            join_sleeps: counters.join_sleeps.load(Ordering::Relaxed),
            join_help: cfg!(feature = "executor_join_help"),
            #[cfg(feature = "executor_join_help")]
            help_executed: counters.help_executed.load(Ordering::Relaxed),
            #[cfg(not(feature = "executor_join_help"))]
            help_executed: 0,
            #[cfg(feature = "executor_join_help")]
            help_ns: counters.help_ns.load(Ordering::Relaxed),
            #[cfg(not(feature = "executor_join_help"))]
            help_ns: 0,
            callback_help: self.shared.callback_help.get().is_some(),
            callback_help_executed: counters.callback_help_executed.load(Ordering::Relaxed),
            callback_help_ns: counters.callback_help_ns.load(Ordering::Relaxed),
            wake_boost_refusals: BOOST_REFUSALS.load(Ordering::Relaxed),
            priority_refusals: PRIORITY_REFUSALS.load(Ordering::Relaxed),
            joins: JOINS.load(Ordering::Relaxed),
            join_wall_ns: JOIN_WALL_NS.load(Ordering::Relaxed),
            join_wall_max_ns: JOIN_WALL_MAX_NS.load(Ordering::Relaxed),
        }
    }
    pub fn wait_ready<T>(&self, ready: impl FnMut() -> Option<T>) -> T {
        self.wait_ready_observing(ready, || {})
    }

    pub fn wait_ready_observing<T>(
        &self,
        mut ready: impl FnMut() -> Option<T>,
        before_wait: impl FnMut(),
    ) -> T {
        #[cfg(feature = "callback_profile")]
        if let Some(value) = ready() {
            return value;
        }
        #[cfg(feature = "callback_profile")]
        let begin = collector_primitives::clock::qpc();
        let value = self.park_until_ready_with(&mut ready, before_wait);
        #[cfg(feature = "callback_profile")]
        crate::callback_profile_state::record_readiness(collector_primitives::clock::qpc() - begin);
        value
    }

    #[cfg(test)]
    fn park_until_ready<T>(&self, ready: impl FnMut() -> Option<T>) -> T {
        self.park_until_ready_with(ready, || {})
    }

    fn park_until_ready_with<T>(
        &self,
        mut ready: impl FnMut() -> Option<T>,
        mut before_wait: impl FnMut(),
    ) -> T {
        if let Some(value) = ready() {
            return value;
        }
        while self.help_callback() {
            if let Some(value) = ready() {
                return value;
            }
        }
        struct Registration<'a>(&'a AtomicBool);
        impl Drop for Registration<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        assert!(!self.shared.ready_waiting.swap(true, Ordering::SeqCst));
        let _registration = Registration(&self.shared.ready_waiting);
        loop {
            let wake = self.shared.ready_wake.load(Ordering::SeqCst);
            if let Some(value) = ready() {
                return value;
            }
            if self.help_callback() {
                continue;
            }
            add(&self.shared.counters.join_sleeps, 1);
            before_wait();
            unsafe {
                WaitOnAddress(
                    self.shared.ready_wake.as_ptr().cast(),
                    (&wake as *const u64).cast(),
                    size_of::<u64>(),
                    u32::MAX,
                )
            };
        }
    }

    pub fn set_callback_help(&self, eligible: fn(&J) -> bool) {
        let state = self.shared.lock();
        assert_eq!(state.generation, 0, "help policy must precede admission");
        assert!(self.shared.callback_help.set(eligible).is_ok());
    }

    fn help_callback(&self) -> bool {
        if self.shared.callback_help.get().is_none() || collector_memory::terminal::failed() {
            return false;
        }
        let claim = {
            let mut state = match self.shared.state.try_lock() {
                Ok(state) => state,
                Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => return false,
            };
            if state.stop {
                return false;
            }
            let Some(position) = state
                .ready
                .iter()
                .enumerate()
                .filter(|(_, index)| state.slots[**index].callback_help)
                .min_by_key(|(_, index)| state.slots[**index].priority)
                .map(|(position, _)| position)
            else {
                return false;
            };
            state.claim(&self.shared, position)
        };
        let begin = std::time::Instant::now();
        self.shared.execute(claim, true, false);
        add(&self.shared.counters.callback_help_executed, 1);
        add(
            &self.shared.counters.callback_help_ns,
            begin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64,
        );
        true
    }

    pub fn notify_ready(&self) {
        if self.shared.ready_waiting.load(Ordering::SeqCst) {
            self.shared.ready_wake.fetch_add(1, Ordering::SeqCst);
            unsafe { WakeByAddressSingle(self.shared.ready_wake.as_ptr().cast()) };
        }
    }
    pub fn completed_report(&self, ticket: Ticket) -> Option<Result<R, Failure>> {
        if ticket.pool != self.shared.id {
            return None;
        }
        let value = self.shared.words.get(ticket.slot)?;
        value
            .compare_exchange(
                word(ticket.generation, PUBLISHED),
                word(ticket.generation, COLLECTING),
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .ok()?;
        // SAFETY: the CAS exclusively owns the fully initialized completion;
        // copying R neither moves nor drops its request or other fields.
        let report = unsafe { (*self.shared.cells[ticket.slot].pointer()).report };
        value.store(word(ticket.generation, PUBLISHED), Ordering::SeqCst);
        Some(report)
    }

    pub fn is_complete(&self, ticket: Ticket) -> bool {
        ticket.pool == self.shared.id
            && self.shared.words.get(ticket.slot).is_some_and(|value| {
                value.load(Ordering::SeqCst) == word(ticket.generation, PUBLISHED)
            })
    }
    pub fn stopped(&self) -> bool {
        self.shared.lock().stop
    }
    /// # Safety
    /// Only the owner settling a contained panic may stop this pool before joining its requests and
    /// workers.
    pub unsafe fn stop_for_contained_panic(&self) {
        add(&self.shared.counters.panicked, 1);
        self.shared.stop();
    }
    /// # Safety
    /// Only the pool's owner may shut it down after admission closes and all requests settle, while
    /// its sampler and payload storage are still live.
    pub unsafe fn shutdown(&self) -> Result<(), &'static str> {
        #[cfg(feature = "worker_sampler")]
        // SAFETY: This owned sampled worker completes the exit handshake before leaving.
        unsafe {
            crate::worker_sampler_state::stop_before_thread_exit()
        };
        self.shared.stop();
        let mut threads = self
            .threads
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for thread in threads.drain(..) {
            if thread.join().is_err() {
                self.shared.join_failed.store(true, Ordering::Release);
            }
        }
        drop(threads);
        if self.shared.join_failed.load(Ordering::Acquire) {
            return Err("native acquisition worker terminated outside its acquisition guard");
        }
        self.shared.wait_active();
        Ok(())
    }
}
impl<J: Send + 'static, R: Copy + Send + 'static, O: Observer> Drop for Pool<J, R, O> {
    fn drop(&mut self) {
        let _ =
        // SAFETY: This is the pool's owner. No submitted work is still borrowing its storage at
        // shutdown.
        unsafe { self.shutdown() };
    }
}

fn worker_loop<J: Send + 'static, R: Copy + Send + 'static, O: Observer>(
    shared: Arc<Shared<J, R, O>>,
    index: usize,
    wake_boost: bool,
) {
    // SAFETY: This thread is a collector-owned acquisition worker reading under the capture gate.
    unsafe { collector_timing::capture_gate::reader() };
    #[cfg(feature = "worker_sampler")]
    let _sampler_registration = crate::worker_sampler_state::register_worker();
    {
        if !wake_boost && unsafe { SetThreadPriorityBoost(GetCurrentThread(), 1) } == 0 {
            BOOST_REFUSALS.fetch_add(1, Ordering::Relaxed);
        }
    }
    let mut priority = i32::MIN;
    let mut away = crate::cpu_sets::AwayFromGame::default();
    let mut hurried = 0;
    shared.thread_ids[index].store(unsafe { GetCurrentThreadId() }, Ordering::Release);
    crate::hour_stages::enter_worker(index);
    let mut state = shared.lock();
    state.started += 1;
    shared.started_ready.notify_all();
    loop {
        if state.stop {
            return;
        }
        if index >= shared.working() {
            state = shared
                .reserve_ready
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
            continue;
        }
        let hurry = shared.hurried.load(Ordering::Acquire);
        if hurry != hurried {
            hurried = hurry;
            priority = THREAD_PRIORITY_ABOVE_NORMAL;
            away = crate::cpu_sets::AwayFromGame::default();
        }
        let wanted = shared.priority.load(Ordering::Acquire);
        if wanted != priority {
            priority = wanted;
            if unsafe { SetThreadPriority(GetCurrentThread(), wanted) } == 0 {
                PRIORITY_REFUSALS.fetch_add(1, Ordering::Relaxed);
            }
        }
        // SAFETY: This thread is a collector-owned acquisition worker applying the pool's affinity policy.
        unsafe { away.follow(shared.away.load(Ordering::Acquire).checked_sub(1)) };
        if let Some(claim) = state.take_ready(&shared, None) {
            let relay = if state.ready.is_empty() {
                shared.burst_peers.store(0, Ordering::Release);
                false
            } else {
                shared
                    .burst_peers
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |peers| {
                        peers.checked_sub(1)
                    })
                    .is_ok()
            };
            drop(state);
            if relay {
                shared.observer.relaying_work();
                shared.work_ready.notify_one();
            }
            shared.execute(claim, false, false);
            state = shared.lock();
        } else {
            if shared.spin_ticks != 0 && shared.active.load(Ordering::Acquire) != 0 {
                drop(state);
                let deadline = collector_primitives::clock::qpc()
                    .saturating_add(shared.spin_ticks as i64);
                let mut woken = false;
                loop {
                    if shared.ready_count.load(Ordering::Acquire) != 0
                        || shared.stopping.load(Ordering::Acquire)
                        || index >= shared.working()
                    {
                        woken = true;
                        break;
                    }
                    if shared.active.load(Ordering::Acquire) == 0
                        || collector_primitives::clock::qpc() >= deadline
                    {
                        break;
                    }
                    for _ in 0..32 {
                        std::hint::spin_loop();
                    }
                }
                state = shared.lock();
                if woken || !state.ready.is_empty() || state.stop || index >= shared.working() {
                    continue;
                }
            }
            shared.burst_peers.store(0, Ordering::Release);
            if shared.active.load(Ordering::Acquire) == 0 {
                crate::private_heap::PRIVATE_HEAP.maybe_flush_thread_cache();
            }
            shared.observer.waiting_for_job();
            state = shared.wait_for_work(state);
        }
    }
}

pub fn recommended_worker_count() -> usize {
    let machine = std::thread::available_parallelism().map_or(1, usize::from);
    worker_count_for_cpus(machine, allowed_cpus())
}

pub fn usable_cpus() -> usize {
    let machine = std::thread::available_parallelism().map_or(1, usize::from);
    cpus(machine, allowed_cpus())
}

fn worker_count_for_cpus(machine: usize, allowed: Option<usize>) -> usize {
    worker_count_for_topology(cpus(machine, allowed))
}

fn cpus(machine: usize, allowed: Option<usize>) -> usize {
    allowed.map_or(machine, |allowed| allowed.min(machine))
}

fn allowed_cpus() -> Option<usize> {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessAffinityMask};
    let (mut process, mut system) = (0_usize, 0_usize);
    let ok = unsafe { GetProcessAffinityMask(GetCurrentProcess(), &mut process, &mut system) };
    (ok != 0 && process != 0).then(|| process.count_ones() as usize)
}

fn worker_count_for_topology(available: usize) -> usize {
    available.saturating_sub(1).clamp(1, MAX_WORKERS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Barrier, mpsc};

    fn progressed(ready: impl FnMut() -> bool) -> bool {
        progressed_within(collector_test_support::TEST_HANG_GUARD, ready)
    }
    fn progressed_within(window: std::time::Duration, mut ready: impl FnMut() -> bool) -> bool {
        let until = std::time::Instant::now() + window;
        while !ready() {
            if std::time::Instant::now() >= until {
                return false;
            }
            std::thread::yield_now();
        }
        true
    }
    fn wait_until(ready: impl FnMut() -> bool) {
        assert!(
            progressed(ready),
            "acquisition did not progress under the hang guard"
        );
    }
    fn wake_join<J: Send + 'static, R: Copy + Send + 'static, O: Observer>(pool: &Pool<J, R, O>, slot: usize) {
        unsafe { WakeByAddressAll(pool.shared.words[slot].as_ptr().cast()) };
    }
    fn rescue_join<J: Send + 'static, R: Copy + Send + 'static, O: Observer>(
        pool: &Pool<J, R, O>,
        slot: usize,
        mut returned: impl FnMut() -> bool,
    ) -> bool {
        progressed(|| {
            if returned() {
                return true;
            }
            wake_join(pool, slot);
            false
        })
    }
    struct Gate(Arc<AtomicBool>);
    impl Gate {
        fn open(&self) {
            self.0.store(true, Ordering::Release);
        }
    }
    impl Drop for Gate {
        fn drop(&mut self) {
            self.open();
        }
    }

    #[test]
    fn a_published_join_does_not_take_the_queue_lock() {
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe { Pool::start(1, 1, |job: &mut u8| *job) }.unwrap();
        let ticket = submit(&pool, &mut Some(7));
        wait_until(|| pool.is_complete(ticket));
        let released = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let (held_tx, held_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let pool_ref = &pool;
            let released_ref = &released;
            let helper = scope.spawn(move || {
                let lock = pool_ref.shared.lock();
                held_tx.send(()).unwrap();
                let requested = release_rx
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .is_ok();
                released_ref.store(true, Ordering::Release);
                drop(lock);
                requested
            });
            held_rx
                .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                .unwrap();
            let completion = pool.finish(ticket).unwrap();
            let before_release = !released.load(Ordering::Acquire);
            let _ = release_tx.send(());
            let requested = helper.join().unwrap();
            assert!(
                before_release && requested,
                "published collection waited for the queue lock"
            );
            assert_eq!(completion.report, Ok(7));
            #[cfg(feature = "callback_profile")]
            assert_eq!(completion.profile.finish_lock_wait_ticks, 0);
        });
        assert!(!pool.is_complete(ticket));
    }

    #[test]
    fn reserves_take_no_work_until_the_pool_holds_the_game_and_its_call_reaches_a_running_job() {
        use windows_sys::Win32::System::Threading::GetThreadPriority;
        let release = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = mpsc::channel();
        let pool = {
            let release = Arc::clone(&release);
            let run = move |gated: &mut bool| {
                let worker = unsafe { GetCurrentThreadId() };
                if *gated {
                    entered_tx.send(worker).unwrap();
                    wait_until(|| release.load(Ordering::Acquire));
                }
                (worker, unsafe { GetThreadPriority(GetCurrentThread()) })
            };
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            unsafe { Pool::start(2, 2, run) }.unwrap()
        };
        let release = Gate(release);
        let ids = pool.thread_ids();
        let (regular, reserve) = (ids[0], ids[1]);
        let run = |pool: &Pool<bool, (u32, i32)>| {
            let ticket = submit(pool, &mut Some(false));
            wait_until(|| pool.is_complete(ticket));
            pool.finish(ticket).unwrap().report.unwrap()
        };
        for _ in 0..2 {
            pool.beside_pace(1, None);
            for _ in 0..64 {
                assert_eq!(
                    run(&pool),
                    (regular, THREAD_PRIORITY_NORMAL),
                    "a resting reserve took work"
                );
            }
            let held = submit(&pool, &mut Some(true));
            assert_eq!(
                entered_rx.recv_timeout(collector_test_support::TEST_HANG_GUARD),
                Ok(regular)
            );
            let queued = submit(&pool, &mut Some(false));
            assert!(!pool.is_complete(queued));
            pool.hold_pace();
            wait_until(|| pool.is_complete(queued));
            assert_eq!(pool.finish(queued).unwrap().report.unwrap().0, reserve);
            release.open();
            wait_until(|| pool.is_complete(held));
            assert_eq!(
                pool.finish(held).unwrap().report.unwrap(),
                (regular, THREAD_PRIORITY_ABOVE_NORMAL)
            );
            release.0.store(false, Ordering::Release);
        }
    }

    #[test]
    fn a_worker_publishes_while_the_queue_lock_is_held() {
        let release = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = mpsc::channel();
        let pool = {
            let release = Arc::clone(&release);
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            unsafe {
                Pool::start(1, 1, move |job: &mut u8| {
                    entered_tx.send(()).unwrap();
                    wait_until(|| release.load(Ordering::Acquire));
                    *job += 1;
                    *job
                })
            }
            .unwrap()
        };
        let release = Gate(release);
        let ticket = submit(&pool, &mut Some(7));
        entered_rx
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap();
        let lock = pool.shared.lock();
        release.open();
        let published = progressed(|| pool.is_complete(ticket));
        let completed = pool.shared.counters.completed.load(Ordering::Relaxed);
        let worker_executed = pool.shared.counters.worker_executed.load(Ordering::Relaxed);
        drop(lock);
        assert!(published, "worker publication waited for the queue lock");
        assert_eq!((completed, worker_executed), (1, 1));
        assert_eq!(pool.finish(ticket).unwrap().report, Ok(8));
    }

    #[test]
    fn worker_and_caller_execute_the_job_in_its_cell() {
        struct Request {
            bytes: [u8; 4096],
        }
        for workers in [0, 1] {
            let (entered_tx, entered_rx) = mpsc::channel();
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            let pool = unsafe {
                Pool::start(workers, 2, move |job: &mut Request| {
                    let address = job as *mut Request as usize;
                    job.bytes[0] = 42;
                    entered_tx.send(address).unwrap();
                    address
                })
            }
            .unwrap();
            let ticket = submit(&pool, &mut Some(Request { bytes: [0; 4096] }));
            let cell_begin = pool.shared.cells[ticket.slot].pointer() as usize;
            let cell_end = cell_begin + size_of::<Cell<Request, usize>>();
            if workers != 0 {
                let address = entered_rx
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap();
                assert!(
                    (cell_begin..cell_end).contains(&address),
                    "worker moved the request to its stack"
                );
            }
            let done = pool.finish(ticket).unwrap();
            let address = done.report.unwrap();
            assert!(
                address >= cell_begin && address + size_of::<Request>() <= cell_end,
                "claimant moved the request to its stack"
            );
            assert_eq!(done.request.bytes[0], 42);
            let calibration = pool.calibration();
            assert_eq!(calibration.caller_executed, u64::from(workers == 0));
            assert_eq!(calibration.worker_executed, workers as u64);
        }
    }

    #[test]
    fn published_collection_is_single_use_even_with_racing_collectors() {
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe { Pool::start(1, 1, |job: &mut u8| *job) }.unwrap();
        let ticket = submit(&pool, &mut Some(42));
        wait_until(|| pool.is_complete(ticket));
        let gate = Barrier::new(3);
        std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                gate.wait();
                pool.finish(ticket)
            });
            let second = scope.spawn(|| {
                gate.wait();
                pool.finish(ticket)
            });
            gate.wait();
            let (first, second) = (first.join().unwrap(), second.join().unwrap());
            assert_ne!(first.is_some(), second.is_some());
            assert_eq!(first.or(second).unwrap().report, Ok(42));
        });
        assert_eq!(pool.calibration().uncollected, 0);
    }

    fn busy_pool() -> (Pool<u64, u64>, Gate, mpsc::Receiver<()>) {
        let release = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = mpsc::channel();
        let pool = {
            let release = Arc::clone(&release);
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            unsafe {
                Pool::start(1, 1, move |job: &mut u64| {
                    entered_tx.send(()).unwrap();
                    wait_until(|| release.load(Ordering::Acquire));
                    *job
                })
            }
            .unwrap()
        };
        pool.reserve_worker_slots(1).unwrap();
        (pool, Gate(release), entered_rx)
    }

    fn busy_pool_round(
        pool: &Pool<u64, u64>,
        release: &Gate,
        entered: &mpsc::Receiver<()>,
        round: u64,
        window: std::time::Duration,
    ) -> (Result<u64, Failure>, bool, bool) {
        release.0.store(false, Ordering::Release);
        let ticket = unsafe { pool.submit_with_priority(&mut Some(round), 0, true, true) }.unwrap();
        entered
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap();
        let joined = AtomicBool::new(false);
        let sleeps_before = pool.shared.counters.join_sleeps.load(Ordering::Relaxed);
        std::thread::scope(|scope| {
            let helper = scope.spawn(|| {
                let registered = progressed(|| {
                    pool.shared.awaited.load(Ordering::SeqCst) == ticket.generation
                        && pool.shared.counters.join_sleeps.load(Ordering::Acquire) > sleeps_before
                });
                release.open();
                let woke = progressed_within(window, || joined.load(Ordering::Acquire));
                if !woke {
                    rescue_join(pool, ticket.slot, || joined.load(Ordering::Acquire));
                }
                (registered, woke)
            });
            let report = pool.finish(ticket).unwrap().report;
            joined.store(true, Ordering::Release);
            let (registered, woke) = helper.join().unwrap();
            (report, registered, woke)
        })
    }

    #[test]
    fn busy_pool_address_joins_do_not_lose_publication_wakes() {
        let (pool, release, entered) = busy_pool();
        for round in 0..512 {
            let (report, registered, woke) = busy_pool_round(
                &pool,
                &release,
                &entered,
                round,
                collector_test_support::TEST_HANG_GUARD,
            );
            assert_eq!(report, Ok(round));
            assert!(registered && woke, "lost address wake in round {round}");
        }
        assert!(pool.calibration().join_sleeps >= 512);
    }

    #[test]
    fn cancellation_wakes_a_join_sleeping_on_a_worker_only_queued_ticket() {
        let trace = Arc::new(collector_memory::read_accounting::fixtures::ReadTrace::default());
        let release = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = mpsc::channel();
        let pool = {
            let release = Arc::clone(&release);
            let runner_trace = Arc::clone(&trace);
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            unsafe {
                Pool::start(1, 2, move |job: &mut u8| {
                    runner_trace.record(*job, || {
                        collector_memory::read_accounting::fixtures::read(1);
                        assert_eq!(*job, 1, "cancelled queued job ran");
                        entered_tx.send(()).unwrap();
                        wait_until(|| release.load(Ordering::Acquire));
                        *job
                    })
                })
            }
            .unwrap()
        };
        let release = Gate(release);
        pool.reserve_worker_slots(1).unwrap();
        let running = submit(&pool, &mut Some(1));
        entered_rx
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap();
        let queued = unsafe { pool.submit_with_priority(&mut Some(2), 0, true, true) }.unwrap();
        let joined = AtomicBool::new(false);
        let sleeps_before = pool.shared.counters.join_sleeps.load(Ordering::Relaxed);
        std::thread::scope(|scope| {
            let helper = scope.spawn(|| {
                let registered = progressed(|| {
                    pool.shared.awaited.load(Ordering::SeqCst) == queued.generation
                        && pool.shared.counters.join_sleeps.load(Ordering::Acquire) > sleeps_before
                });
                // SAFETY: The admitted capture owns failure settlement for this acquisition pool.
                unsafe { pool.stop_for_contained_panic() };
                let woke = progressed(|| joined.load(Ordering::Acquire));
                if !woke {
                    rescue_join(&pool, queued.slot, || joined.load(Ordering::Acquire));
                }
                registered && woke
            });
            let done = pool.finish(queued).unwrap();
            joined.store(true, Ordering::Release);
            assert!(helper.join().unwrap());
            assert_eq!(done.report, Err(Failure::Stopped));
            assert_eq!(
                trace.take(done.request),
                collector_memory::read_accounting::ZERO
            );
            #[cfg(feature = "acquisition_profile")]
            assert_eq!(done.table_reads, collector_memory::read_accounting::ZERO);
        });
        release.open();
        assert_eq!(pool.finish(running).unwrap().report, Ok(1));
        let calibration = pool.calibration();
        assert_eq!(
            (
                calibration.completed,
                calibration.stopped,
                calibration.panicked
            ),
            (2, 1, 1)
        );
        assert_eq!(calibration.caller_executed, 0);
    }

    #[test]
    fn settle_waits_on_active_without_a_queue_relock_and_cancels_unstarted_work() {
        let release = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = mpsc::channel();
        let pool = {
            let release = Arc::clone(&release);
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            unsafe {
                Pool::start(1, 2, move |job: &mut u8| {
                    assert_eq!(*job, 1, "settle ran a queued job");
                    entered_tx.send(()).unwrap();
                    wait_until(|| release.load(Ordering::Acquire));
                    *job
                })
            }
            .unwrap()
        };
        let release = Gate(release);
        let running = submit(&pool, &mut Some(1));
        entered_rx
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap();
        let queued = submit(&pool, &mut Some(2));
        let settled = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let helper = scope.spawn(|| {
                pool.settle();
                settled.store(true, Ordering::Release);
            });
            wait_until(|| pool.shared.settling.load(Ordering::SeqCst) != 0);
            assert!(!settled.load(Ordering::Acquire));
            let lock = pool.shared.lock();
            release.open();
            let returned = progressed(|| settled.load(Ordering::Acquire));
            drop(lock);
            if !returned {
                unsafe { WakeByAddressAll(pool.shared.active.as_ptr().cast()) };
            }
            helper.join().unwrap();
            assert!(
                returned,
                "settle needed the queue lock after its active wait"
            );
        });
        assert_eq!(pool.finish(running).unwrap().report, Ok(1));
        assert_eq!(pool.finish(queued).unwrap().report, Err(Failure::Stopped));
        assert_eq!(pool.shared.active.load(Ordering::Acquire), 0);
        pool.settle();
        assert_eq!(pool.shared.settling.load(Ordering::SeqCst), 0);
        assert!(!pool.stopped());
    }

    #[test]
    fn the_generation_word_bound_also_refuses_parking_without_taking_the_job() {
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe { Pool::start(0, 1, |job: &mut u8| *job) }.unwrap();
        pool.shared.lock().generation = MAX_GENERATION - 1;
        let last = submit(&pool, &mut Some(1));
        assert_eq!(last.generation, MAX_GENERATION);
        assert_eq!(pool.finish(last).unwrap().report, Ok(1));
        let mut job = Some(2);
        assert_eq!(
            unsafe { pool.submit(&mut job) },
            Err(SubmitError::Exhausted)
        );
        assert!(pool.park(&mut job, false).is_none());
        assert_eq!(job, Some(2));
    }

    #[test]
    fn concurrent_shutdown_waiters_all_return_after_caller_publication() {
        for round in 0..64 {
            let release = Arc::new(AtomicBool::new(false));
            let (entered_tx, entered_rx) = mpsc::channel();
            let pool = {
                let release = Arc::clone(&release);
                // SAFETY: Test owns this local acquisition pool and drops it before test exit.
                unsafe {
                    Pool::start(0, 1, move |job: &mut u8| {
                        entered_tx.send(()).unwrap();
                        wait_until(|| release.load(Ordering::Acquire));
                        *job
                    })
                }
                .unwrap()
            };
            let release = Gate(release);
            let ticket = submit(&pool, &mut Some(42));
            let stopped = AtomicUsize::new(0);
            std::thread::scope(|scope| {
                let caller = scope.spawn(|| pool.finish(ticket).unwrap());
                entered_rx
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap();
                let stop = || {
                    // SAFETY: This is the pool's owner. No submitted work is still borrowing its
                    // storage at shutdown.
                    unsafe { pool.shutdown() }.unwrap();
                    stopped.fetch_add(1, Ordering::Release);
                };
                let first = scope.spawn(stop);
                let second = scope.spawn(stop);
                let registered = progressed(|| pool.shared.settling.load(Ordering::SeqCst) == 2);
                release.open();
                let returned = progressed(|| stopped.load(Ordering::Acquire) == 2);
                if !returned {
                    unsafe { WakeByAddressAll(pool.shared.active.as_ptr().cast()) };
                }
                assert_eq!(caller.join().unwrap().report, Ok(42));
                first.join().unwrap();
                second.join().unwrap();
                assert!(
                    registered && returned,
                    "concurrent shutdown lost a wake in round {round}"
                );
            });
            assert_eq!(pool.shared.settling.load(Ordering::SeqCst), 0);
        }
    }

    #[cfg(feature = "executor_join_help")]
    fn help_case(
        awaited_history: bool,
        candidate_history: bool,
        worker_only: bool,
        above_bound: bool,
        classified: bool,
    ) {
        let release = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (ran_tx, ran_rx) = mpsc::channel();
        let pool = {
            let release = Arc::clone(&release);
            let classes: Option<Classes<(u8, u8)>> =
                classified.then_some((2, |job: &(u8, u8)| usize::from(job.1)));
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            unsafe {
                Pool::start_with_class(1, 3, classes, move |job: &mut (u8, u8)| {
                    if job.0 == 0 {
                        entered_tx.send(()).unwrap();
                        wait_until(|| release.load(Ordering::Acquire));
                    } else {
                        ran_tx.send(std::thread::current().id()).unwrap();
                    }
                    std::thread::current().id()
                })
            }
            .unwrap()
        };
        let release = Gate(release);
        pool.reserve_worker_slots(1).unwrap();
        if classified {
            if awaited_history {
                pool.shared.means[0].store(pool.shared.frequency * 60, Ordering::Relaxed);
            }
            if candidate_history {
                let short = (pool.shared.frequency / 1000).max(1);
                pool.shared.means[1].store(short, Ordering::Relaxed);
                pool.shared.maxima[1].store(
                    if above_bound {
                        pool.shared.frequency * 60
                    } else {
                        short
                    },
                    Ordering::Relaxed,
                );
            }
        }
        let awaited = submit(&pool, &mut Some((0, 0)));
        entered_rx
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap();
        let candidate =
            unsafe { pool.submit_with_priority(&mut Some((1, 1)), 0, worker_only, true) }.unwrap();
        let expected_help =
            awaited_history && candidate_history && !worker_only && !above_bound && classified;
        let caller = std::thread::current().id();
        std::thread::scope(|scope| {
            let pool_ref = &pool;
            let release_ref = &release;
            let helper = scope.spawn(move || {
                let observed = if expected_help {
                    ran_rx
                        .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                        .is_ok_and(|thread| thread == caller)
                } else {
                    progressed(|| {
                        pool_ref.shared.awaited.load(Ordering::SeqCst) == awaited.generation
                    })
                };
                release_ref.open();
                let worker_ran = expected_help
                    || ran_rx
                        .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                        .is_ok_and(|thread| thread != caller);
                observed && worker_ran
            });
            assert_ne!(pool.finish(awaited).unwrap().report.unwrap(), caller);
            assert!(
                helper.join().unwrap(),
                "join help did not follow its history/bound/worker-only rule"
            );
        });
        wait_until(|| pool.is_complete(candidate));
        let done = pool.finish(candidate).unwrap();
        assert_eq!(done.report.unwrap() == caller, expected_help);
        let calibration = pool.calibration();
        assert!(calibration.join_help);
        assert_eq!(calibration.help_executed, u64::from(expected_help));
        assert_eq!(calibration.caller_executed, u64::from(expected_help));
        assert_eq!(calibration.help_ns > 0, expected_help);
        if classified {
            assert!(
                pool.shared
                    .means
                    .iter()
                    .all(|mean| mean.load(Ordering::Relaxed) > 0)
            );
        }
    }

    #[cfg(feature = "executor_join_help")]
    #[test]
    fn join_help_runs_a_short_eligible_job_while_the_awaited_body_runs() {
        help_case(true, true, false, false, true);
    }

    #[cfg(feature = "executor_join_help")]
    #[test]
    fn join_help_never_runs_worker_only_or_above_bound_or_unknown_classes() {
        help_case(true, true, true, false, true);
        help_case(true, true, false, true, true);
        help_case(false, true, false, false, true);
        help_case(true, false, false, false, true);
        help_case(true, true, false, false, false);
    }

    #[cfg(feature = "executor_join_help")]
    #[test]
    fn join_help_rejects_a_candidate_whose_mean_fits_but_decaying_maximum_does_not() {
        help_case(true, true, false, true, true);
    }

    #[cfg(feature = "executor_join_help")]
    #[test]
    fn join_help_subtracts_elapsed_body_time_from_its_prediction() {
        let release = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = mpsc::channel();
        let pool = {
            let release = Arc::clone(&release);
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            unsafe {
                Pool::start_with_class(
                    1,
                    2,
                    Some((2, |job: &u8| usize::from(*job != 0))),
                    move |job: &mut u8| {
                        if *job == 0 {
                            entered_tx.send(()).unwrap();
                            wait_until(|| release.load(Ordering::Acquire));
                        }
                        std::thread::current().id()
                    },
                )
            }
            .unwrap()
        };
        let release = Gate(release);
        pool.shared.means[0].store(4, Ordering::Relaxed);
        pool.shared.means[1].store(1, Ordering::Relaxed);
        pool.shared.maxima[1].store(1, Ordering::Relaxed);
        let awaited = submit(&pool, &mut Some(0));
        entered_rx
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap();
        let candidate = submit(&pool, &mut Some(1));
        let begin = pool.shared.body_begin[awaited.slot].load(Ordering::Acquire);
        wait_until(|| {
            (collector_primitives::clock::qpc().max(0) as u64).saturating_sub(begin) >= 4
        });
        let helped = pool.shared.try_help(awaited, &mut HelpBudget::new());
        if let Some(claim) = helped {
            pool.shared.execute(claim, true, true);
        }
        release.open();
        assert_ne!(
            pool.finish(awaited).unwrap().report.unwrap(),
            std::thread::current().id()
        );
        wait_until(|| pool.is_complete(candidate));
        let done = pool.finish(candidate).unwrap();
        assert_ne!(
            done.report.unwrap(),
            std::thread::current().id(),
            "help ignored the awaited body's elapsed time"
        );
        assert_eq!(pool.calibration().help_executed, 0);
    }

    #[cfg(feature = "executor_join_help")]
    #[test]
    fn join_help_claims_in_priority_order_and_rechecks_its_awaited_word() {
        let release = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (ran_tx, ran_rx) = mpsc::channel();
        let pool = {
            let release = Arc::clone(&release);
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            unsafe {
                Pool::start_with_class(
                    1,
                    4,
                    Some((2, |job: &u8| usize::from(*job != 0))),
                    move |job: &mut u8| {
                        if *job == 0 {
                            entered_tx.send(()).unwrap();
                            wait_until(|| release.load(Ordering::Acquire));
                        } else {
                            ran_tx.send((*job, std::thread::current().id())).unwrap();
                        }
                        *job
                    },
                )
            }
            .unwrap()
        };
        let release = Gate(release);
        pool.shared.means[0].store(pool.shared.frequency * 60, Ordering::Relaxed);
        pool.shared.means[1].store((pool.shared.frequency / 1000).max(1), Ordering::Relaxed);
        pool.shared.maxima[1].store((pool.shared.frequency / 1000).max(1), Ordering::Relaxed);
        let awaited = submit(&pool, &mut Some(0));
        entered_rx
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap();
        let low = unsafe { pool.submit_with_priority(&mut Some(1), 9, false, true) }.unwrap();
        let high = unsafe { pool.submit_with_priority(&mut Some(2), 1, false, true) }.unwrap();
        std::thread::scope(|scope| {
            let helper = scope.spawn(move || {
                let first = ran_rx
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap();
                let second = ran_rx
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap();
                release.open();
                [first, second]
            });
            assert_eq!(pool.finish(awaited).unwrap().report, Ok(0));
            let ran = helper.join().unwrap();
            assert_eq!(ran.map(|(job, _)| job), [2, 1]);
            assert!(
                ran.iter()
                    .all(|(_, thread)| *thread == std::thread::current().id())
            );
        });
        assert_eq!(pool.finish(high).unwrap().report, Ok(2));
        assert_eq!(pool.finish(low).unwrap().report, Ok(1));
        assert_eq!(pool.calibration().help_executed, 2);
    }

    #[cfg(feature = "executor_join_help")]
    fn help_budget_case(time_budget: bool) {
        let release = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = mpsc::channel();
        let pool = {
            let release = Arc::clone(&release);
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            unsafe {
                Pool::start_with_class(
                    1,
                    JOIN_HELP_ATTEMPTS + 3,
                    Some((2, |job: &usize| usize::from(*job != 0))),
                    move |job: &mut usize| {
                        if *job == 0 {
                            entered_tx.send(()).unwrap();
                            wait_until(|| release.load(Ordering::Acquire));
                        }
                        *job
                    },
                )
            }
            .unwrap()
        };
        let release = Gate(release);
        pool.shared.means[0].store(pool.shared.frequency * 60, Ordering::Relaxed);
        pool.shared.means[1].store(1, Ordering::Relaxed);
        pool.shared.maxima[1].store(
            if time_budget {
                pool.shared.frequency * 20
            } else {
                1
            },
            Ordering::Relaxed,
        );
        let awaited = submit(&pool, &mut Some(0));
        entered_rx
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap();
        let candidates: Vec<_> = (1..=JOIN_HELP_ATTEMPTS + 2)
            .map(|job| submit(&pool, &mut Some(job)))
            .collect();
        let mut budget = HelpBudget::new();
        for _ in 0..JOIN_HELP_ATTEMPTS + 2 {
            if let Some(claim) = pool.shared.try_help(awaited, &mut budget) {
                pool.shared.execute(claim, true, true);
            }
        }
        let helped = pool.shared.counters.help_executed.load(Ordering::Relaxed);
        release.open();
        assert_eq!(pool.finish(awaited).unwrap().report, Ok(0));
        for (index, candidate) in candidates.into_iter().enumerate() {
            wait_until(|| pool.is_complete(candidate));
            assert_eq!(pool.finish(candidate).unwrap().report, Ok(index + 1));
        }
        assert_eq!(
            helped,
            if time_budget {
                1
            } else {
                JOIN_HELP_ATTEMPTS as u64
            }
        );
        assert_eq!(budget.attempts, 0, "help attempts were replenished");
    }

    #[cfg(feature = "executor_join_help")]
    #[test]
    fn join_help_stops_after_its_attempt_budget() {
        help_budget_case(false);
    }

    #[cfg(feature = "executor_join_help")]
    #[test]
    fn join_help_stops_after_its_cumulative_predicted_maximum_budget() {
        help_budget_case(true);
    }

    #[cfg(feature = "executor_join_help")]
    #[test]
    fn join_help_collects_publication_while_a_contended_queue_lock_stays_held() {
        let release = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = mpsc::channel();
        let pool = {
            let release = Arc::clone(&release);
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            unsafe {
                Pool::start_fixture(
                    1,
                    2,
                    Some((2, |job: &u8| usize::from(*job != 0))),
                    move |job: &mut u8| {
                        if *job == 0 {
                            entered_tx.send(()).unwrap();
                            wait_until(|| release.load(Ordering::Acquire));
                        }
                        *job
                    },
                )
            }
            .unwrap()
        };
        let release = Gate(release);
        pool.shared.means[0].store(pool.shared.frequency * 60, Ordering::Relaxed);
        let awaited = submit(&pool, &mut Some(0));
        entered_rx
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap();
        let candidate = submit(&pool, &mut Some(1));
        pool.shared
            .observer.help_attempt_gate
            .armed
            .store(true, Ordering::SeqCst);
        std::thread::scope(|scope| {
            let (done_tx, done_rx) = mpsc::channel();
            let pool_ref = &pool;
            let collector = scope.spawn(move || {
                let done = pool_ref.finish(awaited).unwrap();
                done_tx.send(done.report).unwrap();
            });
            let armed = progressed(|| {
                pool.shared
                    .observer.help_attempt_gate
                    .entered
                    .load(Ordering::Acquire)
            });
            let lock = armed.then(|| pool.shared.lock());
            pool.shared.means[1].store(1, Ordering::Relaxed);
            pool.shared.maxima[1].store(1, Ordering::Relaxed);
            pool.shared
                .observer.help_attempt_gate
                .released
                .store(true, Ordering::Release);
            let contended = armed
                && progressed(|| {
                    pool.shared
                        .observer
                        .help_lock_contentions
                        .load(Ordering::Acquire)
                        > 0
                });
            release.open();
            let published = progressed(|| {
                pool.is_complete(awaited)
                    || pool.shared.words[awaited.slot].load(Ordering::Acquire)
                        == word(awaited.generation, VACANT)
            });
            let collected_while_held =
                done_rx.recv_timeout(collector_test_support::TEST_HANG_GUARD);
            drop(lock);
            if collected_while_held.is_err() {
                rescue_join(&pool, awaited.slot, || collector.is_finished());
            }
            collector.join().unwrap();
            assert!(armed, "help never entered its armed attempt");
            assert!(contended, "help never met the held queue lock");
            assert!(published, "awaited publication needed the queue lock");
            assert_eq!(
                collected_while_held.unwrap(),
                Ok(0),
                "help blocked collection on the queue lock"
            );
        });
        wait_until(|| pool.is_complete(candidate));
        assert_eq!(pool.finish(candidate).unwrap().report, Ok(1));
        assert_eq!(pool.calibration().help_executed, 0);
    }

    #[test]
    fn vacant_and_parked_and_uncollected_cells_drop_each_owned_job_once() {
        struct OwnedDrop(Arc<AtomicU64>);
        impl Drop for OwnedDrop {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let dropped = Arc::new(AtomicU64::new(0));
        {
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            let pool = unsafe { Pool::start(0, 3, |_: &mut OwnedDrop| ()) }.unwrap();
            let first = submit(&pool, &mut Some(OwnedDrop(Arc::clone(&dropped))));
            let _second = submit(&pool, &mut Some(OwnedDrop(Arc::clone(&dropped))));
            let _parked = pool
                .park(&mut Some(OwnedDrop(Arc::clone(&dropped))), false)
                .unwrap();
            drop(pool.finish(first).unwrap());
            assert_eq!(dropped.load(Ordering::Relaxed), 1);
            // SAFETY: This is the pool's owner. No submitted work is still borrowing its storage
            // at shutdown.
            unsafe { pool.shutdown() }.unwrap();
        }
        assert_eq!(dropped.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn cell_teardown_contains_each_panicking_destructor_and_drains_later_cells() {
        struct PanicPayload;
        impl Drop for PanicPayload {
            fn drop(&mut self) {
                panic!("synthetic panic payload destructor must be forgotten");
            }
        }
        struct Request {
            drops: Arc<AtomicUsize>,
            panics: bool,
        }
        impl Drop for Request {
            fn drop(&mut self) {
                self.drops.fetch_add(1, Ordering::Relaxed);
                if self.panics {
                    std::panic::panic_any(PanicPayload);
                }
            }
        }
        let dropped: [Arc<AtomicUsize>; 3] = std::array::from_fn(|_| Arc::new(AtomicUsize::new(0)));
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe { Pool::start(0, 3, |_: &mut Request| ()) }.unwrap();
        let parked = pool
            .park(
                &mut Some(Request {
                    drops: Arc::clone(&dropped[2]),
                    panics: false,
                }),
                false,
            )
            .unwrap();
        let healthy = submit(
            &pool,
            &mut Some(Request {
                drops: Arc::clone(&dropped[1]),
                panics: false,
            }),
        );
        let panicking = submit(
            &pool,
            &mut Some(Request {
                drops: Arc::clone(&dropped[0]),
                panics: true,
            }),
        );
        assert_eq!((panicking.slot, healthy.slot, parked.slot), (0, 1, 2));
        for ticket in [panicking, healthy] {
            let claim = pool
                .shared
                .lock()
                .take_ready(&pool.shared, Some(ticket))
                .unwrap();
            pool.shared.execute(claim, true, false);
            assert!(pool.is_complete(ticket));
        }
        assert!(catch_unwind(AssertUnwindSafe(|| drop(pool))).is_ok());
        assert_eq!(
            dropped
                .each_ref()
                .map(|count| count.load(Ordering::Relaxed)),
            [1, 1, 1]
        );
    }

    #[cfg(feature = "callback_profile")]
    #[test]
    fn executor_profile_orders_reached_stages_and_marks_canceled_work() {
        for workers in [0, 1] {
            let run = |_: &mut u8| unsafe { GetCurrentThreadId() };
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            let pool = unsafe { Pool::start(workers, 1, run) }.unwrap();
            let mut job = Some(7);
            let ticket = unsafe { pool.submit(&mut job) }.unwrap();
            let completion = pool.finish(ticket).unwrap();
            let profile = completion.profile;
            assert_eq!(completion.report.unwrap(), profile.runner_thread_id);
            assert_ne!(profile.runner_thread_id, 0);
            let stages = [
                profile.queued_qpc,
                profile.claimed_qpc,
                profile.runner_begin_qpc,
                profile.runner_end_qpc,
                profile.published_qpc,
                profile.collected_qpc,
            ];
            assert!(stages[0] > 0);
            assert!(stages.windows(2).all(|pair| pair[0] <= pair[1]));
        }
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe { Pool::start(0, 1, |_: &mut u8| ()) }.unwrap();
        let mut job = Some(7);
        let ticket = unsafe { pool.submit(&mut job) }.unwrap();
        // SAFETY: This is the pool's owner. No submitted work is still borrowing its storage at
        // shutdown.
        unsafe { pool.shutdown() }.unwrap();
        let completion = pool.finish(ticket).unwrap();
        assert_eq!(completion.report, Err(Failure::Stopped));
        let profile = completion.profile;
        assert!(profile.queued_qpc > 0);
        assert_eq!(profile.claimed_qpc, 0);
        assert_eq!(profile.runner_begin_qpc, 0);
        assert_eq!(profile.runner_end_qpc, 0);
        assert_eq!(profile.runner_thread_id, 0);
        assert_eq!(profile.runner_thread_cycles, None);
        assert!(profile.published_qpc >= profile.queued_qpc);
        assert!(profile.collected_qpc >= profile.published_qpc);
    }

    struct Owned {
        payload: Vec<u8>,
    }

    fn change_owned(request: &mut Owned) -> usize {
        request.payload[0] += 1;
        request.payload.len()
    }

    #[test]
    fn reserved_worker_job_cannot_be_claimed_by_its_collector() {
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe { Pool::start(1, 3, |_: &mut u8| std::thread::current().id()) }.unwrap();
        assert!(pool.reserve_worker_slots(4).is_err());
        pool.reserve_worker_slots(2).unwrap();
        let ordinary = submit(&pool, &mut Some(1));
        let mut rejected = Some(3);
        assert_eq!(
            unsafe { pool.submit(&mut rejected) },
            Err(SubmitError::Full)
        );
        let mut request = Some(2);
        let ticket = unsafe { pool.submit_with_priority(&mut request, 0, true, true) }.unwrap();
        assert!(request.is_none());
        let second = unsafe { pool.submit_with_priority(&mut Some(4), 0, true, true) }.unwrap();
        assert!(
            pool.shared
                .lock()
                .take_ready(&pool.shared, Some(ticket))
                .is_none()
        );
        assert!(
            pool.shared
                .lock()
                .take_ready(&pool.shared, Some(second))
                .is_none()
        );
        assert_ne!(
            pool.finish(ticket).unwrap().report.unwrap(),
            std::thread::current().id()
        );
        assert!(pool.finish(ticket).is_none());
        assert_ne!(
            pool.finish(second).unwrap().report.unwrap(),
            std::thread::current().id()
        );
        assert!(pool.finish(ordinary).is_some());
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let stopped = unsafe { Pool::start(0, 1, |_: &mut u8| ()) }.unwrap();
        assert!(stopped.reserve_worker_slots(1).is_err());
    }

    #[test]
    fn a_parked_request_never_runs_and_returns_once_to_its_submitter() {
        for workers in [1, 2] {
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            let pool = unsafe { Pool::start(workers, 3, |request: &mut u8| *request) }.unwrap();
            pool.reserve_worker_slots(2).unwrap();
            // SAFETY: This is the pool's owner. No submitted work is still borrowing its storage
            // at shutdown.
            unsafe { pool.shutdown() }.unwrap();
            let mut request = Some(7);
            assert_eq!(
                unsafe { pool.submit_with_priority(&mut request, 0, true, true) },
                Err(SubmitError::Stopped)
            );
            let parked = pool.park(&mut request, true).unwrap();
            assert!(request.is_none());
            assert!(parked.slot < 2);
            assert!(pool.finish(parked).is_none());
            let calibration = pool.calibration();
            assert_eq!(calibration.accepted, 0);
            assert_eq!(calibration.rejected, 1);
            assert_eq!(calibration.queued, 0);
            assert_eq!(calibration.uncollected, 0);
            assert_eq!(calibration.caller_executed + calibration.worker_executed, 0);
            let second = pool.park(&mut Some(8), true).unwrap();
            assert!(second.slot < 2);
            let other = pool.park(&mut Some(9), true).unwrap();
            assert_eq!(other.slot, 2);
            let mut overflow = Some(10);
            assert!(pool.park(&mut overflow, true).is_none());
            assert_eq!(overflow, Some(10));
            assert_eq!(pool.unpark(parked), Some(7));
            assert_eq!(pool.unpark(parked), None);
            assert_eq!(pool.unpark(other), Some(9));
            assert_eq!(pool.unpark(second), Some(8));
            assert_eq!(
                pool.shared
                    .words
                    .iter()
                    .filter(|value| value.load(Ordering::Acquire) & PHASE_MASK == VACANT)
                    .count(),
                3
            );
            assert_eq!(pool.shared.lock().parked, 0);
        }
        let pool =
        // SAFETY: The attaching runtime or isolated fixture owns this worker through shutdown.
        unsafe { Pool::start_fixture(1, 1, None, |request: &mut u8| *request + 1) }.unwrap();
        let (idle_tx, idle) = mpsc::channel();
        assert!(pool.shared.observer.before_work_wait.set(Box::new(move || {
            let _ = idle_tx.send(());
        })).is_ok());
        let parked = pool.park(&mut Some(3), false).unwrap();
        assert_eq!(unsafe { pool.submit(&mut Some(4)) }, Err(SubmitError::Full));
        while idle.try_recv().is_ok() {}
        {
            let _state = pool.shared.lock();
            pool.shared.work_ready.notify_all();
        }
        idle.recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap();
        assert_eq!(pool.calibration().worker_executed, 0);
        assert_eq!(pool.unpark(parked), Some(3));
        let ticket = submit(&pool, &mut Some(4));
        assert_eq!(ticket.slot, parked.slot);
        assert_eq!(pool.unpark(ticket), None);
        assert_eq!(pool.finish(ticket).unwrap().report, Ok(5));
        assert_eq!(pool.unpark(parked), None);
    }

    #[test]
    fn parked_rejection_completes_in_its_heap_cell_without_allocating_or_running() {
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe {
            Pool::start(1, 1, |_: &mut Box<[u8; 4096]>| -> u8 {
                panic!("rejected worker must never run")
            })
        }
        .unwrap();
        let owner = Box::new([7; 4096]);
        let allocation = owner.as_ptr();
        let ticket = pool.park(&mut Some(owner), true).unwrap();
        let cell = pool.shared.cells[ticket.slot].pointer();
        let mut out = None;
        assert!(!pool.finish_parked_into(ticket, &mut out, |_| None));
        assert!(out.is_none());
        let reads = collector_memory::read_accounting::Capture::new();
        collector_test_support::with_allocations_denied(|| {
            assert!(pool.finish_parked_into(ticket, &mut out, |request| {
                assert_eq!(request as *mut _, unsafe {
                    std::ptr::addr_of_mut!((*cell).request)
                });
                request[0] = 9;
                Some(Ok(3))
            }));
        });
        assert_eq!(pool.shared.lock().parked, 0);
        assert_eq!(pool.calibration().worker_executed, 0);
        assert_eq!(pool.calibration().uncollected, 0);
        let completed = out.take().unwrap();
        assert_eq!(completed.request.as_ptr(), allocation);
        assert_eq!(completed.request[0], 9);
        assert_eq!(completed.report, Ok(3));
        assert_eq!(reads.finish(), collector_memory::read_accounting::ZERO);
        #[cfg(feature = "acquisition_profile")]
        assert_eq!(
            completed.table_reads,
            collector_memory::read_accounting::ZERO
        );
        assert!(!pool.finish_parked_into(ticket, &mut out, |_| panic!("stale ticket")));
        assert!(pool.unpark(ticket).is_none());
        let next = pool.park(&mut Some(Box::new([0; 4096])), true).unwrap();
        assert_eq!(next.slot, ticket.slot);
        assert!(!pool.finish_parked_into(ticket, &mut out, |_| panic!("reused ticket")));
        assert!(pool.unpark(next).is_some());
    }

    #[test]
    fn panicking_parked_rejection_keeps_the_job_owned_for_drain() {
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe { Pool::start(1, 1, |request: &mut u8| *request) }.unwrap();
        let ticket = pool.park(&mut Some(7), true).unwrap();
        let mut out = None;
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                pool.finish_parked_into(ticket, &mut out, |request| {
                    *request = 9;
                    panic!("contained refusal panic")
                })
            }))
            .is_err()
        );
        assert!(out.is_none());
        assert_eq!(pool.unpark(ticket), Some(9));
        assert_eq!(pool.unpark(ticket), None);
    }

    fn submit<J: Send + 'static, R: Copy + Send + 'static, O: Observer>(
        pool: &Pool<J, R, O>,
        request: &mut Option<J>,
    ) -> Ticket {
        unsafe { pool.submit(request) }.unwrap()
    }

    #[test]
    fn owned_buffers_return_and_fixed_slots_reuse_without_replacement() {
        for workers in [0, 2, MAX_WORKERS] {
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            let pool = unsafe { Pool::start(workers, 1, change_owned) }.unwrap();
            let mut ids = pool.thread_ids();
            assert_eq!(ids.len(), workers);
            assert!(ids.iter().all(|&id| id != 0));
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), workers);
            let bytes = pool.allocated_bytes();
            let mut request = Some(Owned {
                payload: vec![1, 2, 3],
            });
            let first_pointer = request.as_ref().unwrap().payload.as_ptr();
            for iteration in 0..3 {
                let ticket = submit(&pool, &mut request);
                assert!(request.is_none());
                let done = pool.finish(ticket).unwrap();
                assert_eq!(done.report, Ok(3));
                assert_eq!(done.request.payload, [2 + iteration, 2, 3]);
                assert_eq!(done.request.payload.as_ptr(), first_pointer);
                request = Some(done.request);
                assert!(pool.finish(ticket).is_none());
                assert_eq!(pool.allocated_bytes(), bytes);
            }
            let calibration = pool.calibration();
            assert_eq!(calibration.accepted, 3);
            assert_eq!(calibration.completed, 3);
            assert_eq!(calibration.uncollected, 0);
            // SAFETY: This is the pool's owner. No submitted work is still borrowing its storage
            // at shutdown.
            unsafe { pool.shutdown() }.unwrap();
            // SAFETY: This is the pool's owner. No submitted work is still borrowing its storage
            // at shutdown.
            unsafe { pool.shutdown() }.unwrap();
        }
    }

    #[test]
    fn caller_claims_awaited_ticket_and_keeps_other_slots_owned() {
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe { Pool::start(0, 2, |request: &mut u8| *request + 1) }.unwrap();
        let first = submit(&pool, &mut Some(1));
        let second = submit(&pool, &mut Some(2));
        assert_eq!(pool.finish(second).unwrap().report, Ok(3));
        let calibration = pool.calibration();
        assert_eq!(calibration.caller_executed, 1);
        assert_eq!(calibration.worker_executed, 0);
        assert_eq!(calibration.completed, 1);
        assert_eq!(calibration.queued, 1);
        assert_eq!(calibration.uncollected, 1);
        let third = submit(&pool, &mut Some(3));
        let mut rejected = Some(9);
        assert_eq!(
            unsafe { pool.submit(&mut rejected) },
            Err(SubmitError::Full)
        );
        assert_eq!(rejected, Some(9));
        assert_eq!(pool.finish(first).unwrap().report, Ok(2));
        assert_eq!(pool.finish(third).unwrap().report, Ok(4));
        // SAFETY: This is the pool's owner. No submitted work is still borrowing its storage at
        // shutdown.
        unsafe { pool.shutdown() }.unwrap();
        assert_eq!(
            unsafe { pool.submit(&mut rejected) },
            Err(SubmitError::Stopped)
        );
        assert_eq!(rejected, Some(9));
    }

    #[test]
    fn stale_foreign_and_exhausted_tickets_never_consume_another_request() {
        // SAFETY: Test owns these local acquisition pools and drops them before test exit.
        let first_pool = unsafe { Pool::start(0, 1, |request: &mut u8| *request) }.unwrap();
        let other_pool = unsafe { Pool::start(0, 1, |request: &mut u8| *request) }.unwrap();
        let old = submit(&first_pool, &mut Some(1));
        let other = submit(&other_pool, &mut Some(2));
        assert!(other_pool.finish(old).is_none());
        assert_eq!(first_pool.finish(old).unwrap().request, 1);
        let fresh = submit(&first_pool, &mut Some(3));
        assert!(first_pool.finish(old).is_none());
        assert_eq!(first_pool.calibration().caller_executed, 1);
        assert_eq!(first_pool.finish(fresh).unwrap().request, 3);
        assert_eq!(other_pool.finish(other).unwrap().request, 2);
        first_pool.shared.lock().generation = MAX_GENERATION;
        let mut rejected = Some(4);
        assert_eq!(
            unsafe { first_pool.submit(&mut rejected) },
            Err(SubmitError::Exhausted)
        );
        assert_eq!(rejected, Some(4));
    }

    #[test]
    fn panic_returns_partial_request_cancels_queued_jobs_and_does_not_retry() {
        use collector_memory::read_accounting::fixtures::ReadTrace;
        use engine_native_memory::{
            Memory,
            memory::{CountedMemory, ReadCounts, SliceMemory},
        };
        use replay_data::rows::TableId;

        let trace = Arc::new(ReadTrace::default());
        let runner_trace = Arc::clone(&trace);
        let calls = Arc::new(AtomicU64::new(0));
        let count = Arc::clone(&calls);
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe {
            Pool::start(0, 2, move |request: &mut Owned| -> usize {
                let key = request.payload.as_ptr() as usize;
                runner_trace.record(key, || {
                    count.fetch_add(1, Ordering::Relaxed);
                    let _reads = collector_memory::read_accounting::table(TableId::Divisions);
                    let memory = CountedMemory::new(SliceMemory {
                        base: 0x1000,
                        bytes: &request.payload,
                    });
                    assert!(memory.copy(0x1000, &mut [0]));
                    request.payload[0] = 42;
                    panic!("synthetic acquisition failure");
                })
            })
        }
        .unwrap();
        let make = || Owned { payload: vec![7] };
        let first = submit(&pool, &mut Some(make()));
        let second = submit(&pool, &mut Some(make()));
        let failed = pool.finish(second).unwrap();
        let canceled = pool.finish(first).unwrap();
        assert_eq!(canceled.report, Err(Failure::Stopped));
        assert_eq!(
            trace.take(canceled.request.payload.as_ptr() as usize),
            collector_memory::read_accounting::ZERO
        );
        #[cfg(feature = "acquisition_profile")]
        assert_eq!(
            canceled.table_reads,
            collector_memory::read_accounting::ZERO
        );
        assert_eq!(canceled.request.payload, [7]);
        assert_eq!(failed.report, Err(Failure::Panicked));
        let mut expected = collector_memory::read_accounting::ZERO;
        expected[TableId::Divisions.index()] = ReadCounts {
            copy_calls: 1,
            spans: 1,
            bytes: 1,
            ..ReadCounts::ZERO
        };
        assert_eq!(
            trace.take(failed.request.payload.as_ptr() as usize),
            expected
        );
        #[cfg(feature = "acquisition_profile")]
        assert_eq!(failed.table_reads, expected);
        assert_eq!(failed.request.payload, [42]);
        let mut rejected = Some(failed.request);
        assert_eq!(
            unsafe { pool.submit(&mut rejected) },
            Err(SubmitError::Stopped)
        );
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let calibration = pool.calibration();
        assert_eq!(calibration.panicked, 1);
        assert_eq!(calibration.stopped, 1);
        assert_eq!(calibration.completed, 2);
    }

    #[test]
    fn shutdown_cancels_pending_without_running_and_returns_arc_leases() {
        let trace = Arc::new(collector_memory::read_accounting::fixtures::ReadTrace::default());
        let runner_trace = Arc::clone(&trace);
        let mut owner = Arc::new(vec![7]);
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool: Pool<Arc<Vec<u8>>, ()> = unsafe {
            Pool::start(0, 1, move |_: &mut Arc<Vec<u8>>| {
                runner_trace.record((), || {
                    collector_memory::read_accounting::fixtures::read(1);
                    panic!("canceled job ran");
                });
            })
        }
        .unwrap();
        let ticket = submit(&pool, &mut Some(Arc::clone(&owner)));
        // SAFETY: This is the pool's owner. No submitted work is still borrowing its storage at
        // shutdown.
        unsafe { pool.shutdown() }.unwrap();
        assert!(Arc::get_mut(&mut owner).is_none());
        let completion = pool.finish(ticket).unwrap();
        assert_eq!(completion.report, Err(Failure::Stopped));
        assert_eq!(trace.take(()), collector_memory::read_accounting::ZERO);
        #[cfg(feature = "acquisition_profile")]
        assert_eq!(
            completion.table_reads,
            collector_memory::read_accounting::ZERO
        );
        assert!(Arc::get_mut(&mut owner).is_none());
        drop(completion);
        assert!(Arc::get_mut(&mut owner).is_some());
        assert_eq!(pool.calibration().caller_executed, 0);
    }

    #[test]
    fn completion_follows_local_reader_drop_on_success_and_unwind() {
        struct Lease<'a>(&'a AtomicBool);
        impl Drop for Lease<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        for panics in [false, true] {
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            let pool = unsafe {
                Pool::start(1, 1, move |live: &mut Arc<AtomicBool>| {
                    live.store(true, Ordering::Release);
                    let _lease = Lease(live);
                    assert!(!panics, "synthetic reader failure");
                    true
                })
            }
            .unwrap();
            let live = Arc::new(AtomicBool::new(false));
            let ticket = submit(&pool, &mut Some(Arc::clone(&live)));
            let done = pool.finish(ticket).unwrap();
            assert!(!live.load(Ordering::Acquire));
            assert_eq!(
                done.report,
                if panics {
                    Err(Failure::Panicked)
                } else {
                    Ok(true)
                }
            );
        }
    }

    #[test]
    fn callback_panic_does_not_abandon_an_already_running_worker() {
        let gate = Arc::new(Barrier::new(2));
        let worker_gate = Arc::clone(&gate);
        let (entered_tx, entered_rx) = mpsc::channel();
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe {
            Pool::start(1, 3, move |request: &mut u8| {
                if *request == 1 {
                    entered_tx.send(()).unwrap();
                    worker_gate.wait();
                    *request = 10;
                    10
                } else {
                    assert_ne!(*request, 3, "canceled job must never run");
                    *request = 20;
                    panic!("synthetic caller failure");
                }
            })
        }
        .unwrap();
        let running = submit(&pool, &mut Some(1));
        entered_rx.recv().unwrap();
        let panicking = submit(&pool, &mut Some(2));
        let pending = submit(&pool, &mut Some(3));
        let failed = pool.finish(panicking).unwrap();
        let canceled = pool.finish(pending).unwrap();
        let active_after_panic = pool.calibration().active;
        gate.wait();
        let completed = pool.finish(running).unwrap();
        // SAFETY: This is the pool's owner. No submitted work is still borrowing its storage at
        // shutdown.
        unsafe { pool.shutdown() }.unwrap();
        assert_eq!(failed.request, 20);
        assert_eq!(failed.report, Err(Failure::Panicked));
        assert_eq!(canceled.request, 3);
        assert_eq!(canceled.report, Err(Failure::Stopped));
        assert_eq!(active_after_panic, 1);
        assert_eq!(completed.request, 10);
        assert_eq!(completed.report, Ok(10));
        let calibration = pool.calibration();
        assert_eq!(calibration.completed, 3);
        assert_eq!(calibration.uncollected, 0);
        assert_eq!(calibration.worker_executed, 1);
        assert_eq!(calibration.caller_executed, 1);
    }

    #[test]
    fn worker_panic_cancels_queued_jobs_while_another_worker_drains() {
        let successful_gate = Arc::new(Barrier::new(2));
        let successful_release = Arc::clone(&successful_gate);
        let panic_gate = Arc::new(Barrier::new(2));
        let panic_release = Arc::clone(&panic_gate);
        let (entered_tx, entered_rx) = mpsc::channel();
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe {
            Pool::start(2, 3, move |request: &mut u8| {
                entered_tx.send(*request).unwrap();
                match *request {
                    1 => {
                        successful_gate.wait();
                        10
                    }
                    2 => {
                        panic_gate.wait();
                        panic!("synthetic worker failure")
                    }
                    _ => panic!("canceled queued job ran"),
                }
            })
        }
        .unwrap();
        let first = submit(&pool, &mut Some(1));
        let first_started = entered_rx.recv().unwrap();
        let second = submit(&pool, &mut Some(2));
        let second_started = entered_rx.recv().unwrap();
        let queued = submit(&pool, &mut Some(3));
        panic_release.wait();
        wait_until(|| pool.shared.counters.panicked.load(Ordering::Acquire) != 0);
        let failed = pool.finish(second).unwrap();
        let canceled = pool.finish(queued).unwrap();
        wait_until(|| pool.shared.active.load(Ordering::Acquire) == 1);
        let active_after_panic = pool.calibration().active;
        successful_release.wait();
        let successful = pool.finish(first).unwrap();
        // SAFETY: This is the pool's owner. No submitted work is still borrowing its storage at
        // shutdown.
        unsafe { pool.shutdown() }.unwrap();
        assert_eq!((first_started, second_started), (1, 2));
        assert_eq!(failed.request, 2);
        assert_eq!(failed.report, Err(Failure::Panicked));
        assert_eq!(canceled.request, 3);
        assert_eq!(canceled.report, Err(Failure::Stopped));
        assert_eq!(active_after_panic, 1);
        assert_eq!(successful.report, Ok(10));
        let calibration = pool.calibration();
        assert_eq!(calibration.worker_executed, 2);
        assert_eq!(calibration.caller_executed, 0);
        assert_eq!(calibration.uncollected, 0);
    }

    #[test]
    fn split_notifications_wake_work_and_release_idle_peer_on_panic_stop() {
        let (entered_tx, entered_rx) = mpsc::channel();
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe {
            Pool::start(2, 1, move |request: &mut u8| -> u8 {
                entered_tx.send(()).unwrap();
                *request = 9;
                panic!("synthetic split-notification failure");
            })
        }
        .unwrap();
        let ticket = submit(&pool, &mut Some(1));
        entered_rx.recv().unwrap();

        wait_until(|| pool.shared.counters.panicked.load(Ordering::Acquire) != 0);
        let failed = pool.finish(ticket).unwrap();
        assert_eq!(failed.request, 9);
        assert_eq!(failed.report, Err(Failure::Panicked));

        let mut threads = pool
            .threads
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for thread in threads.drain(..) {
            thread.join().unwrap();
        }
        drop(threads);
        // SAFETY: This is the pool's owner. No submitted work is still borrowing its storage at
        // shutdown.
        unsafe { pool.shutdown() }.unwrap();
        let calibration = pool.calibration();
        assert_eq!(calibration.panicked, 1);
        assert_eq!(calibration.worker_executed, 1);
        assert_eq!(calibration.caller_executed, 0);
        assert_eq!(calibration.uncollected, 0);
    }

    #[test]
    fn workers_and_callback_execute_disjoint_owned_jobs_together() {
        let meet = Arc::new(Barrier::new(3));
        let (entered_tx, entered_rx) = mpsc::channel();
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe {
            Pool::start(2, 3, move |request: &mut u8| {
                entered_tx.send(*request).unwrap();
                meet.wait();
                *request += 1;
                *request
            })
        }
        .unwrap();
        let first = submit(&pool, &mut Some(1));
        assert_eq!(entered_rx.recv().unwrap(), 1);
        let second = submit(&pool, &mut Some(2));
        assert_eq!(entered_rx.recv().unwrap(), 2);
        let third = submit(&pool, &mut Some(3));
        assert_eq!(pool.finish(third).unwrap().report, Ok(4));
        for (index, ticket) in [first, second].into_iter().enumerate() {
            assert_eq!(pool.finish(ticket).unwrap().report, Ok(index as u8 + 2));
        }
        let calibration = pool.calibration();
        assert_eq!(calibration.worker_executed, 2);
        assert_eq!(calibration.caller_executed, 1);
    }

    #[test]
    fn shutdown_waits_for_caller_work_and_preserves_its_completion() {
        let gate = Arc::new(Barrier::new(2));
        let release = Arc::clone(&gate);
        let (entered_tx, entered_rx) = mpsc::channel();
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = Arc::new(
            unsafe {
                Pool::start(0, 1, move |request: &mut u8| {
                    entered_tx.send(()).unwrap();
                    gate.wait();
                    *request += 1;
                    *request
                })
            }
            .unwrap(),
        );
        let ticket = submit(&pool, &mut Some(7));
        let helper_pool = Arc::clone(&pool);
        let helper = std::thread::spawn(move || helper_pool.finish(ticket).unwrap());
        entered_rx.recv().unwrap();
        let shutdown_pool = Arc::clone(&pool);
        let (stopped_tx, stopped_rx) = mpsc::channel();
        let stop = std::thread::spawn(move || {
            shutdown_pool.shared.stop();
            stopped_tx.send(()).unwrap();
            // SAFETY: This is the pool's owner. No submitted work is still borrowing its storage
            // at shutdown.
            unsafe { shutdown_pool.shutdown() }.unwrap();
        });
        stopped_rx.recv().unwrap();
        let active_before_release = pool.calibration().active;
        let stopped_before_release = stop.is_finished();
        release.wait();
        assert_eq!(helper.join().unwrap().report, Ok(8));
        stop.join().unwrap();
        assert_eq!(active_before_release, 1);
        assert!(!stopped_before_release);
        assert_eq!(pool.calibration().active, 0);
    }

    fn wait_until_running<J: Send + 'static, R: Copy + Send + 'static>(
        pool: &Pool<J, R>,
        active: usize,
    ) {
        let limit = std::time::Instant::now() + collector_test_support::TEST_HANG_GUARD;
        while pool.calibration().active < active {
            assert!(std::time::Instant::now() < limit, "job never started");
            std::thread::yield_now();
        }
    }

    #[test]
    fn collector_parks_when_every_worker_is_busy() {
        let release = Gate(Arc::new(AtomicBool::new(false)));
        let (claimed_tx, claimed) = mpsc::channel();
        let pool = {
            let release = Arc::clone(&release.0);
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            unsafe {
                Pool::start_fixture(1, 4, None, move |job: &mut u64| {
                    claimed_tx.send(*job).unwrap();
                    wait_until(|| release.load(Ordering::Acquire));
                    *job
                })
            }
            .unwrap()
        };
        pool.reserve_worker_slots(4).unwrap();
        let running = unsafe { pool.submit_with_priority(&mut Some(20), 0, true, true) }.unwrap();
        assert_eq!(
            claimed
                .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                .unwrap(),
            20
        );
        let queued = unsafe { pool.submit_with_priority(&mut Some(2), 0, true, true) }.unwrap();
        let (slept_tx, slept) = mpsc::channel();
        {
            let release = Arc::clone(&release.0);
            assert!(pool.shared.observer.before_join_wait.set(Box::new(move |ticket| {
                let _ = slept_tx.send(ticket);
                release.store(true, Ordering::Release);
            })).is_ok());
        }
        let joined = AtomicBool::new(false);
        let (report, woke) = std::thread::scope(|scope| {
            let guard = scope.spawn(|| {
                let woke = progressed(|| joined.load(Ordering::Acquire));
                if !woke {
                    pool.shared.stop();
                    rescue_join(&pool, queued.slot, || joined.load(Ordering::Acquire));
                }
                woke
            });
            let report = pool.finish(queued).unwrap().report;
            joined.store(true, Ordering::Release);
            (report, guard.join().unwrap())
        });
        assert!(woke, "the join was rescued: its publication wake was lost");
        assert_eq!(report.unwrap(), 2);
        assert_eq!(slept.try_recv(), Ok(queued));
        assert_eq!(claimed.try_recv(), Ok(2));
        assert_eq!(pool.finish(running).unwrap().report.unwrap(), 20);
    }

    #[test]
    fn a_panic_during_a_join_is_collected_and_cancels_queued_work() {
        let panic_gate = Arc::new(AtomicBool::new(false));
        let slow_gate = Arc::new(AtomicBool::new(false));
        let (panic_now, slow_done) = (Arc::clone(&panic_gate), Arc::clone(&slow_gate));
        // SAFETY: Test owns this local acquisition pool and drops it before test exit.
        let pool = unsafe {
            Pool::start(2, 4, move |value: &mut u64| {
                let gate = if *value == 99 { &panic_now } else { &slow_done };
                if *value == 99 || *value == 50 {
                    while !gate.load(Ordering::Acquire) {
                        std::thread::yield_now();
                    }
                }
                if *value == 99 {
                    panic!("synthetic acquisition panic");
                }
                *value
            })
        }
        .unwrap();
        pool.reserve_worker_slots(4).unwrap();
        let slow = unsafe { pool.submit_with_priority(&mut Some(50), 0, true, true) }.unwrap();
        wait_until_running(&pool, 1);
        let failing = unsafe { pool.submit_with_priority(&mut Some(99), 0, true, true) }.unwrap();
        wait_until_running(&pool, 2);
        let queued = unsafe { pool.submit_with_priority(&mut Some(7), 0, true, true) }.unwrap();
        panic_gate.store(true, Ordering::Release);
        assert!(matches!(
            pool.finish(failing).unwrap().report,
            Err(Failure::Panicked)
        ));
        assert!(matches!(
            pool.finish(queued).unwrap().report,
            Err(Failure::Stopped)
        ));
        assert_eq!(
            pool.shared.words[queued.slot].load(Ordering::Acquire),
            word(queued.generation, VACANT)
        );
        slow_gate.store(true, Ordering::Release);
        assert_eq!(pool.finish(slow).unwrap().report.unwrap(), 50);
    }

    #[test]
    fn process_affinity_limits_the_pool_below_the_machine_count() {
        assert_eq!(worker_count_for_cpus(16, Some(4)), 3);
        assert_eq!(worker_count_for_cpus(16, Some(16)), 15);
        assert_eq!(worker_count_for_cpus(16, None), 15);
        assert_eq!(worker_count_for_cpus(8, Some(64)), 7);
        assert_eq!(worker_count_for_cpus(16, Some(1)), 1);
        assert_eq!(worker_count_for_cpus(64, Some(40)), MAX_WORKERS);
    }

    #[test]
    fn the_pool_never_exceeds_the_cpus_this_process_may_use() {
        let allowed = allowed_cpus().expect("process affinity readable");
        assert!(allowed >= 1);
        assert!(recommended_worker_count() <= allowed.saturating_sub(1).max(1));
    }

    #[test]
    fn hardware_threads_size_the_pool_and_reserve_one_for_the_callback() {
        assert_eq!(worker_count_for_topology(16), 15);
        assert_eq!(worker_count_for_topology(8), 7);
        assert_eq!(worker_count_for_topology(4), 3);
        assert_eq!(worker_count_for_topology(2), 1);
        assert_eq!(worker_count_for_topology(1), 1);
        assert_eq!(worker_count_for_topology(0), 1);
        assert_eq!(worker_count_for_topology(32), MAX_WORKERS);
        // SAFETY: Test validates rejection of an excessive worker count before spawning threads.
        assert!(unsafe { Pool::start(MAX_WORKERS + 1, 1, |request: &mut u8| *request) }.is_err());
    }

    #[test]
    fn a_release_wakes_one_worker_per_job_and_the_workers_run_them_all() {
        for workers in 1..=4 {
            for jobs in 1..=workers + 1 {
                let gate = Arc::new(AtomicBool::new(false));
                let ran = Arc::new(Mutex::new(Vec::new()));
                let (entered, started) = mpsc::channel();
                let pool = {
                    let (gate, ran) = (Arc::clone(&gate), Arc::clone(&ran));
                    // SAFETY: Test owns this local acquisition pool and drops it before test exit.
                    unsafe {
                        Pool::start_fixture(workers, 8, None, move |job: &mut usize| {
                            ran.lock().unwrap().push(std::thread::current().id());
                            if *job < workers {
                                entered.send(()).unwrap();
                                wait_until(|| gate.load(Ordering::Acquire));
                            }
                        })
                    }
                    .unwrap()
                };
                let release = Gate(gate);
                pool.reserve_worker_slots(8).unwrap();
                let tickets: Vec<_> = (0..jobs)
                    .map(|job| unsafe {
                        pool.submit_with_priority(&mut Some(job), 0, true, false)
                            .unwrap()
                    })
                    .collect();
                collector_test_support::with_allocations_denied(|| pool.wake_workers(jobs));
                for _ in 0..jobs.min(workers) {
                    started
                        .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                        .unwrap();
                }
                {
                    assert_eq!(pool.shared.observer.notified_one.load(Ordering::Relaxed), 1);
                    assert!(pool.shared.observer.relayed_one.load(Ordering::Relaxed) <= (jobs - 1).min(workers - 1) as u64);
                }
                release.open();
                for ticket in tickets {
                    assert_eq!(pool.finish(ticket).unwrap().report, Ok(()), "{jobs} jobs");
                }
                let ran = ran.lock().unwrap();
                assert_eq!(ran.len(), jobs);
                let distinct: std::collections::HashSet<_> = ran.iter().copied().collect();
                assert_eq!(distinct.len(), jobs.min(workers), "{jobs} jobs");
                assert!(!distinct.contains(&std::thread::current().id()));
                let calibration = pool.calibration();
                assert_eq!(
                    (calibration.caller_executed, calibration.worker_executed),
                    (0, jobs as u64)
                );
                // SAFETY: This is the pool's owner. No submitted work is still borrowing its
                // storage at shutdown.
                unsafe { pool.shutdown() }.unwrap();
            }
        }
    }

    #[test]
    fn busy_workers_retain_and_merge_burst_relays_without_collector_allocation() {
        const WORKERS: usize = 4;
        let gate = Arc::new(AtomicBool::new(false));
        let (entered, started) = mpsc::channel();
        let pool = {
            let gate = Arc::clone(&gate);
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            unsafe {
                Pool::start_fixture(WORKERS, 8, None, move |job: &mut usize| {
                    if *job < WORKERS {
                        entered.send(()).unwrap();
                        wait_until(|| gate.load(Ordering::Acquire));
                    }
                    *job
                })
            }
            .unwrap()
        };
        let release = Gate(gate);
        pool.reserve_worker_slots(8).unwrap();
        let running: Vec<_> = (0..WORKERS)
            .map(|job| unsafe {
                pool.submit_with_priority(&mut Some(job), 0, true, true)
                    .unwrap()
            })
            .collect();
        for _ in 0..WORKERS {
            started
                .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                .unwrap();
        }
        let queued: Vec<_> = (WORKERS..2 * WORKERS)
            .map(|job| unsafe {
                pool.submit_with_priority(&mut Some(job), 0, true, false)
                    .unwrap()
            })
            .collect();
        collector_test_support::with_allocations_denied(|| {
            pool.wake_workers(2);
            pool.wake_workers(2);
            assert_eq!(pool.shared.burst_peers.load(Ordering::Acquire), 2);
            assert_eq!(pool.shared.observer.notified_one.load(Ordering::Relaxed), 2);
            assert_eq!(pool.shared.observer.relayed_one.load(Ordering::Relaxed), 0);
        });
        release.open();
        for (job, ticket) in running.into_iter().chain(queued).enumerate() {
            assert_eq!(pool.finish(ticket).unwrap().report, Ok(job));
        }
        assert_eq!(pool.shared.burst_peers.load(Ordering::Acquire), 0);
        assert_eq!(pool.calibration().caller_executed, 0);
        assert_eq!(pool.calibration().uncollected, 0);
    }
}

#[cfg(test)]
mod worker_join_tests {
    use super::*;
    use std::sync::Weak;

    #[test]
    fn parent_and_child_worker_joins_keep_the_callback_lease_and_its_own_waiter() {
        for panic_child in [false, true] {
            let binding = Arc::new(OnceLock::<Weak<Pool<u8, u32>>>::new());
            let worker_binding = Arc::clone(&binding);
            let caller = unsafe { GetCurrentThreadId() };
            let run = move |request: &mut u8| {
                let thread = unsafe { GetCurrentThreadId() };
                assert_ne!(thread, caller);
                if *request == 1 {
                    assert!(!panic_child, "synthetic child acquisition failure");
                    return thread;
                }
                let pool = worker_binding.get().unwrap().upgrade().unwrap();
                let mut child = Some(1);
                let ticket =
                    unsafe { pool.submit_with_priority(&mut child, 0, true, true) }.unwrap();
                let mut completed = None;
                assert!(pool.finish_worker_into(ticket, &mut completed));
                match completed.unwrap().report {
                    Ok(child) => assert_ne!(thread, child),
                    Err(failure) => assert_eq!(failure, Failure::Panicked),
                }
                thread
            };
            // SAFETY: Test owns this local acquisition pool and drops it before test exit.
            let pool = Arc::new(unsafe { Pool::start(2, 3, run) }.unwrap());
            pool.reserve_worker_slots(2).unwrap();
            binding.set(Arc::downgrade(&pool)).unwrap();
            let mut parent = Some(0);
            let ticket = unsafe { pool.submit_with_priority(&mut parent, 0, true, true) }.unwrap();
            let mut completed = None;
            assert!(pool.finish_into(ticket, &mut completed));
            assert_ne!(completed.unwrap().report.unwrap(), caller);
            assert_eq!(pool.shared.worker_waiters.load(Ordering::SeqCst), 0);
            assert_eq!(pool.calibration().uncollected, 0);
            assert_eq!(pool.calibration().panicked, u64::from(panic_child));
        }
    }
}

/// # Safety
/// Spawns acquisition workers registered with `collector_timing::capture_gate::reader()`. Caller
/// must hold exclusive fixture control while the temporary pool runs.
#[cfg(any(test, feature = "test-support"))]
pub unsafe fn verify_allocated_bytes<J: Send + 'static, R: Copy + Send + 'static>(
    workers: usize,
    capacity: usize,
    classes: Classes<J>,
    run: impl Fn(&mut J) -> R + Send + Sync + 'static,
) {
    let runner_bytes = size_of_val(&run);
    assert!(runner_bytes > 0, "fixture must cover the boxed runner");
    // SAFETY: Caller holds exclusive fixture control; pool is shut down before returning.
    let pool = unsafe {
        Pool::start_classified_boosted(workers, capacity, false, Some(classes), Unobserved, run)
    }
    .unwrap();
    assert_eq!(pool.shared.cells.len(), capacity);
    assert_eq!(pool.shared.words.len(), capacity);
    assert_eq!(size_of::<Cell<J, R>>(), size_of::<Completion<J, R>>());
    let state = pool.shared.lock();
    let threads = pool.threads.lock().unwrap();
    let shared_alignment = std::mem::align_of::<Shared<J, R, Unobserved>>();
    let shared_offset = (2 * size_of::<AtomicUsize>()).next_multiple_of(shared_alignment);
    let arc_alignment = shared_alignment.max(std::mem::align_of::<AtomicUsize>());
    let allocations = [
        (shared_offset + size_of::<Shared<J, R, Unobserved>>()).next_multiple_of(arc_alignment),
        pool.shared.cells.len() * size_of::<Cell<J, R>>(),
        pool.shared.words.len() * size_of::<AtomicU64>(),
        state.slots.capacity() * size_of::<Slot>(),
        state.ready.capacity() * size_of::<usize>(),
        pool.shared.thread_ids.capacity() * size_of::<AtomicU32>(),
        threads.capacity() * size_of::<JoinHandle<()>>(),
        runner_bytes,
    ];
    assert_eq!(size_of_val(&*pool.shared.run), runner_bytes);
    let expected: usize = allocations.into_iter().sum();
    #[cfg(feature = "executor_join_help")]
    let expected = {
        assert_eq!(pool.shared.means.len(), classes.0);
        assert_eq!(pool.shared.maxima.len(), classes.0);
        assert_eq!(pool.shared.body_begin.len(), capacity);
        expected
            + pool.shared.means.len() * size_of::<AtomicU64>()
            + pool.shared.maxima.len() * size_of::<AtomicU64>()
            + pool.shared.body_begin.len() * size_of::<AtomicU64>()
    };
    assert_eq!(
        pool.allocated_bytes(),
        expected,
        "executor bytes omitted a live allocation"
    );
    drop(threads);
    drop(state);
    // SAFETY: This is the pool's owner. No submitted work is still borrowing its storage at
    // shutdown.
    unsafe { pool.shutdown() }.unwrap();
}

/// # Safety
/// Spawns acquisition workers registered with `collector_timing::capture_gate::reader()`. Caller
/// must hold exclusive fixture control while the benchmark pools run.
#[cfg(feature = "test-support")]
pub unsafe fn dispatch_and_release_bench<J, R, const JOB: usize>() {
    use std::time::{Duration, Instant};
    struct Request<const JOB: usize> {
        micros: u64,
        _bytes: [u8; JOB],
    }
    fn spin(micros: u64) {
        let end = Instant::now() + Duration::from_micros(micros);
        while Instant::now() < end {
            std::hint::spin_loop();
        }
    }
    fn qpc_ns(begin: Instant) -> u64 {
        begin.elapsed().as_nanos() as u64
    }
    fn summarize(label: &str, samples: &mut [u64]) {
        samples.sort_unstable();
        let mean = samples.iter().map(|&ns| u128::from(ns)).sum::<u128>() / samples.len() as u128;
        println!(
            "{label}: mean {mean} ns, p50 {} ns, p95 {} ns, p99 {} ns, max {} ns ({} samples)",
            samples[samples.len() / 2],
            samples[(samples.len() - 1) * 95 / 100],
            samples[(samples.len() - 1) * 99 / 100],
            samples[samples.len() - 1],
            samples.len(),
        );
    }
    println!(
        "Job {} bytes; slot {} bytes; completion {} bytes",
        JOB,
        size_of::<Cell<J, R>>() + size_of::<Slot>() + size_of::<AtomicU64>(),
        size_of::<Completion<J, R>>()
    );
    let cpus = std::thread::available_parallelism().map_or(1, usize::from);
    for load in [0, cpus.saturating_sub(MAX_WORKERS)] {
        let stop = Arc::new(AtomicBool::new(false));
        let spinners: Vec<_> = (0..load)
            .map(|_| {
                let stop = Arc::clone(&stop);
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        std::hint::spin_loop();
                    }
                })
            })
            .collect();
        for (wake_boost, wake_all) in [(true, true), (false, true), (true, false), (false, false)] {
            // SAFETY: Caller holds exclusive fixture control; pool is shut down each iteration.
            let pool = unsafe {
                Pool::start_boosted(
                    MAX_WORKERS,
                    111,
                    wake_boost,
                    |request: &mut Request<JOB>| {
                        spin(request.micros);
                    },
                )
            }
            .unwrap();
            let wake = |jobs: usize| {
                if wake_all {
                    pool.shared.work_ready.notify_all();
                } else {
                    pool.wake_workers(jobs);
                }
            };
            const ROUNDS: usize = 200;
            let mut held = Vec::with_capacity(52 * ROUNDS);
            let mut waking = Vec::with_capacity(20 * ROUNDS);
            let mut releases: [Vec<u64>; 3] = std::array::from_fn(|_| Vec::with_capacity(ROUNDS));
            for _ in 0..ROUNDS {
                let mut tickets = Vec::new();
                let mut submit = |micros: u64, wake: bool, samples: &mut Vec<u64>| {
                    let mut request = Some(Request {
                        micros,
                        _bytes: [0; JOB],
                    });
                    let begin = Instant::now();
                    let ticket =
                        unsafe { pool.submit_with_priority(&mut request, 0, false, wake) }.unwrap();
                    samples.push(qpc_ns(begin));
                    tickets.push(ticket);
                };
                for _ in 0..45 {
                    submit(300, false, &mut held);
                }
                let begin = Instant::now();
                wake(45);
                releases[0].push(qpc_ns(begin));
                for _ in 0..6 {
                    submit(200, false, &mut held);
                }
                let begin = Instant::now();
                wake(6);
                releases[1].push(qpc_ns(begin));
                for _ in 0..20 {
                    submit(100, true, &mut waking);
                }
                submit(100, false, &mut held);
                let begin = Instant::now();
                wake(1);
                releases[2].push(qpc_ns(begin));
                for ticket in tickets {
                    assert!(pool.finish(ticket).unwrap().report.is_ok());
                }
            }
            println!("load {load} spinners, boost {wake_boost}, wake all {wake_all}");
            summarize("held submit per job", &mut held);
            summarize("waking submit per job", &mut waking);
            for (label, samples) in ["release 45 jobs", "release 6 jobs", "release 1 job"]
                .into_iter()
                .zip(&mut releases)
            {
                summarize(label, samples);
            }
            // SAFETY: This is the pool's owner. No submitted work is still borrowing its storage
            // at shutdown.
            unsafe { pool.shutdown() }.unwrap();
        }
        stop.store(true, Ordering::Relaxed);
        for spinner in spinners {
            spinner.join().unwrap();
        }
    }
}
