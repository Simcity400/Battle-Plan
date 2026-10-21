//! A flat, preallocated pool for hour-bounded native acquisition.
//!
//! The queue mutex owns metadata and claim order, never a running payload.
//! Each slot has one cell and one word: `generation << 3 | phase`.
//! Submission owns a VACANT cell under the queue mutex, initializes its job,
//! marks metadata Queued and releases PENDING before enqueueing. Claiming
//! removes that entry and releases RUNNING under the mutex; only that claimant
//! then borrows the job, in place, outside the mutex. It ends every live reader
//! and context borrow, fills the completion and counters, and releases PUBLISHED.
//! After publication it never accesses the cell again. Cancellation owns only
//! still-Queued cells under the mutex and publishes their Stopped completions.
//! An acquire CAS from PUBLISHED to COLLECTING gives exactly one collector the
//! complete cell. Collection moves that value once into `out`, then releases
//! VACANT; report-only inspection copies R and restores PUBLISHED without
//! moving the request or transferring its ticket. A new submitter acquires VACANT under the mutex before reusing the storage.
//! Parked cells are PENDING but metadata Parked: only locked unpark or
//! rejection completion accesses them. Rejection initializes its completion
//! directly in the existing heap cell and never admits or runs the job.
//!
//! Jobs own outputs and source leases. The sole hourly callback must collect
//! every accepted ticket before the trampoline, including on unwind. A join
//! may run its exact awaited queued job when caller execution is allowed;
//! optional join help is bounded by learned body times. Worker-only jobs
//! always retain their completion barrier. Runners never submit, collect,
//! stop or drop this pool; dependency barriers belong to the callback.

use std::sync::Arc;
use std::thread::JoinHandle;
use std::{
    cell::UnsafeCell,
    collections::VecDeque,
    mem::{MaybeUninit, size_of, size_of_val},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
};

use serde::Serialize;
use windows_sys::Win32::System::Threading::{
    GetCurrentThreadId, WaitOnAddress, WakeByAddressAll, WakeByAddressSingle,
};

pub(crate) const MAX_WORKERS: usize = 15;
// Ordinary naval tree censuses fall back to borrowed ancestor frames when
// pack depth scratch is short. Keep the worker reserve explicit, independent
// of RUST_MIN_STACK, matching the supported executable's stack reserve (4 MiB).
// Native retention/copy depth is documented beside the engine reader.
const WORKER_STACK_BYTES: usize = 4 * 1024 * 1024;
/// Joins begin spinning only while a worker is idle; readiness polls complete
/// producer barriers before sleeping. Both use this fixed per-wait deadline.
/// The first spin sets one deadline for the join. Default eligibility is
/// reconsidered on claim and worker retirement, without extending that deadline.
/// The pool-busy feature permits one episode, ending when all workers are active.
/// The zero-budget feature exposes an SMT/wake-latency A/B without affinity
/// or priority changes. A spare worker does not prove a spare physical core.
const JOIN_SPIN: std::time::Duration = if cfg!(feature = "executor_no_join_spin") {
    std::time::Duration::ZERO
} else {
    std::time::Duration::from_millis(2)
};
const VACANT: u64 = 0;
const PENDING: u64 = 1;
const PUBLISHED: u64 = 2;
const COLLECTING: u64 = 3;
const RUNNING: u64 = 4;
const PHASE_MASK: u64 = 7;
const MAX_GENERATION: u64 = u64::MAX >> 3;
// A cell array cannot have enough slots to use this bit: its byte length is
// bounded by isize::MAX. The lower bits retain the sleeping collector's slot + 1.
const AWAITED_SPIN: u64 = 1 << 63;
const AWAITED_SLOT_MASK: u64 = !AWAITED_SPIN;
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
#[path = "acquisition/collection_bench.rs"]
mod collection_bench;

#[cfg(test)]
#[path = "acquisition/readiness_tests.rs"]
mod readiness_tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// Partial observations are retained but no report is valid; never retry.
    Panicked,
    /// Cancellation or terminal allocation failure; never retry.
    Stopped,
}

pub(crate) struct Completion<J, R> {
    pub request: J,
    pub report: Result<R, Failure>,
    pub table_reads: crate::read_accounting::TableCounts,
    pub timeline: Timeline,
    #[cfg(feature = "callback_profile")]
    pub profile: Profile,
}

/// Lifecycle in raw QPC ticks; zero denotes a stage never reached.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Timeline {
    pub queued_qpc: i64,
    pub body_begin_qpc: i64,
    pub body_end_qpc: i64,
    /// Immediately before the publisher releases the slot word.
    pub published_qpc: i64,
    /// After the collector's acquire CAS, before moving the completion.
    pub collected_qpc: i64,
    /// Only the first queue-lock acquisition in a pending join; zero for a
    /// completion collected by the initial lock-free CAS. Address waits and
    /// optional help claims are excluded.
    pub finish_lock_ticks: u64,
    /// Checked heap growth attempts admitted inside this job's body.
    pub growths: u64,
}

/// Diagnostic stages in raw QPC ticks. The publisher cannot stamp a wake's
/// end in the cell: the collector owns it immediately after publication.
#[cfg(feature = "callback_profile")]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Profile {
    pub queued_qpc: i64,
    pub claimed_qpc: i64,
    pub runner_begin_qpc: i64,
    pub runner_end_qpc: i64,
    pub runner_thread_id: u32,
    pub runner_thread_cycles: Option<u64>,
    pub submit_lock_wait_ticks: u64,
    pub finish_lock_wait_ticks: u64,
    pub published_qpc: i64,
    pub collected_qpc: i64,
}

/// Pool identity, 61-bit generation and a single-use collection CAS prevent
/// foreign, stale or racing tickets from consuming another output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ticket {
    pool: u64,
    slot: usize,
    generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SubmitError {
    Empty,
    Full,
    Stopped,
    Exhausted,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ThreadIds {
    values: [u32; MAX_WORKERS],
    len: usize,
}
impl ThreadIds {
    pub(crate) fn ids(&self) -> &[u32] {
        &self.values[..self.len]
    }
}
impl Serialize for ThreadIds {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.values[..self.len].serialize(serializer)
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct Calibration {
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
    pub spins: u64,
    /// Summed wall spent spinning, including any descheduling in that span.
    pub spin_wall_ns: u64,
    /// WaitOnAddress calls by joins, including spurious returns.
    pub join_sleeps: u64,
    pub join_spin_budget_ns: u64,
    pub join_spin_policy: &'static str,
    pub join_help: bool,
    pub help_executed: u64,
    /// Body wall of jobs executed by join help, also in caller_executed.
    pub help_ns: u64,
    pub wake_boost_refusals: u64,
    /// Workers left at normal priority because raising it failed.
    pub priority_refusals: u64,
    pub joins: u64,
    pub join_wall_ns: u64,
    pub join_wall_max_ns: u64,
}

#[derive(Default)]
struct SubmitCounters {
    accepted: u64,
    rejected: u64,
    #[cfg(test)]
    notified_one: u64,
    #[cfg(test)]
    relayed_one: u64,
}

#[derive(Default)]
struct Counters {
    completed: AtomicU64,
    panicked: AtomicU64,
    stopped: AtomicU64,
    caller_executed: AtomicU64,
    worker_executed: AtomicU64,
    spins: AtomicU64,
    spin_wall_ns: AtomicU64,
    join_sleeps: AtomicU64,
    #[cfg(feature = "executor_join_help")]
    help_executed: AtomicU64,
    #[cfg(feature = "executor_join_help")]
    help_ns: AtomicU64,
    #[cfg(all(test, feature = "executor_join_help"))]
    help_lock_contentions: AtomicU64,
}

#[cfg(feature = "executor_join_help")]
const JOIN_HELP_ATTEMPTS: usize = 4;

/// A join never replenishes either budget after a spurious wake. The time
/// budget is half its first valid predicted remainder, charged by each
/// admitted candidate's decaying maximum, rather than its eventual body time.
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

/// Hold an armed help attempt before try_lock, with its budget still available.
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
    /// Lower priority first, with FIFO for equal priorities.
    priority: u8,
    worker_only: bool,
    queued_qpc: i64,
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

/// The job is initialized once on admission. Other completion fields become
/// initialized before PUBLISHED. No Drop here: Shared drops only owned fields
/// according to the slot word, after every participant has gone away.
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
                queued_qpc: 0,
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

    fn free_slot<J, R>(&self, shared: &Shared<J, R>, worker_only: bool) -> Option<usize> {
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

    fn claim<J, R>(&mut self, shared: &Shared<J, R>, position: usize) -> Claim {
        let index = self
            .ready
            .remove(position)
            .expect("checked acquisition ready position");
        let slot = &mut self.slots[index];
        debug_assert!(slot.phase == Phase::Queued);
        slot.phase = Phase::Running;
        shared.active.fetch_add(1, Ordering::SeqCst);
        #[cfg(feature = "callback_profile")]
        // SAFETY: the queue mutex and removed ready entry give this claimant
        // exclusive ownership of the Queued cell, before its runner begins.
        unsafe {
            (*shared.cells[index].pointer()).profile.claimed_qpc = crate::sampling::qpc();
        }
        shared.words[index].store(word(slot.generation, RUNNING), Ordering::SeqCst);
        // Claim's SC store/load pairs with registration then the SC word reload:
        // the collector either sees RUNNING or the claimant sees its spin flag.
        // The wake sequence changes even if the wake precedes WaitOnAddress.
        let awaited = shared.awaited.load(Ordering::SeqCst);
        if awaited == ((index as u64 + 1) | AWAITED_SPIN) {
            shared.wake_spin_waiter();
        }
        Claim {
            index,
            generation: slot.generation,
            #[cfg(feature = "executor_join_help")]
            class: slot.class,
        }
    }

    fn take_ready<J, R>(
        &mut self,
        shared: &Shared<J, R>,
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

    fn stop<J, R>(&mut self, shared: &Shared<J, R>) {
        self.stop = true;
        shared.stopped.store(true, Ordering::SeqCst);
        self.cancel_queued(shared);
    }

    fn cancel_queued<J, R>(&mut self, shared: &Shared<J, R>) {
        while let Some(index) = self.ready.pop_front() {
            let slot = &self.slots[index];
            debug_assert!(slot.phase == Phase::Queued);
            let completion = shared.cells[index].pointer();
            // SAFETY: cancellation holds the mutex and has removed the only
            // ready entry. No claimant can access this Queued cell. The job,
            // timeline and optional profile were initialized at submission.
            unsafe {
                std::ptr::addr_of_mut!((*completion).report).write(Err(Failure::Stopped));
                std::ptr::addr_of_mut!((*completion).table_reads)
                    .write(crate::read_accounting::ZERO);
                (*completion).timeline.queued_qpc = slot.queued_qpc;
            }
            add(&shared.counters.stopped, 1);
            add(&shared.counters.completed, 1);
            shared.publish(index, slot.generation);
        }
    }
}

type Runner<J, R> = dyn Fn(&mut J) -> R + Send + Sync;
type Classes<J> = (usize, fn(&J) -> usize);
struct BatchContext(UnsafeCell<crate::allocation::Context>);
// SAFETY: writes are locked and only at uncollected == 0. Every runner drops
// its immutable borrow before PUBLISHED, which precedes the final decrement.
unsafe impl Sync for BatchContext {}

struct Shared<J, R> {
    id: u64,
    state: Mutex<State>,
    started_ready: Condvar,
    work_ready: Condvar,
    /// Remaining held-burst notifications, relayed one per worker claim.
    burst_peers: AtomicUsize,
    run: Box<Runner<J, R>>,
    thread_ids: Vec<AtomicU32>,
    cells: Box<[Cell<J, R>]>,
    words: Box<[AtomicU64]>,
    active: AtomicUsize,
    /// Mirrors the locked, terminal stop state for lock-free spin eligibility.
    stopped: AtomicBool,
    uncollected: AtomicUsize,
    /// One sleeping collector: slot + 1, optionally AWAITED_SPIN, or zero.
    awaited: AtomicU64,
    /// Eligibility waits use this address because an idle transition does not
    /// change the awaited slot word. Publication advances and wakes it too.
    spin_wake: AtomicU64,
    /// The callback's readiness scheduler waits for any producer publication,
    /// without collecting or running an unrelated job in a fixed join order.
    ready_waiting: AtomicBool,
    ready_wake: AtomicU64,
    /// Registered active-count waiters. A count preserves concurrent shutdown:
    /// an early zero observer cannot unregister another still-sleeping waiter.
    settling: AtomicUsize,
    counters: Counters,
    /// Drops every join wake (publication, claim and idle), so fixtures prove
    /// that a lost wake fails their assertions instead of hanging the test.
    #[cfg(test)]
    suppress_join_wakes: AtomicBool,
    #[cfg(all(test, feature = "executor_join_help"))]
    help_attempt_gate: HelpAttemptGate,
    #[cfg(feature = "executor_join_help")]
    classify: Option<fn(&J) -> usize>,
    #[cfg(feature = "executor_join_help")]
    means: Box<[AtomicU64]>,
    #[cfg(feature = "executor_join_help")]
    maxima: Box<[AtomicU64]>,
    /// The helper reads this atomic, never a Running cell's timeline.
    #[cfg(feature = "executor_join_help")]
    body_begin: Box<[AtomicU64]>,
    #[cfg(feature = "executor_join_help")]
    frequency: u64,
    join_failed: AtomicBool,
    context: BatchContext,
}

impl<J, R> Shared<J, R> {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
    fn wait_for_work<'a>(&self, state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.work_ready
            .wait(state)
            .unwrap_or_else(|error| error.into_inner())
    }
    fn stop(&self) {
        self.lock().stop(self);
        self.work_ready.notify_all();
    }

    fn publish(&self, index: usize, generation: u64) {
        let completion = self.cells[index].pointer();
        // SAFETY: called only by the unique Running claimant, or by locked
        // cancellation of Queued work. Every completion field is initialized
        // and exclusively owned until the SC release store below.
        unsafe {
            let published_qpc = crate::sampling::qpc();
            (*completion).timeline.published_qpc = published_qpc;
            #[cfg(feature = "callback_profile")]
            {
                (*completion).profile.published_qpc = published_qpc;
            }
        }
        self.words[index].store(word(generation, PUBLISHED), Ordering::SeqCst);
        // No cell access after this store, even if the slot is already reused.
        // Dekker in the SC order: either the sleeper's word reload sees this
        // store, or this load sees its awaited registration. WaitOnAddress
        // also returns immediately if publication precedes its actual wait:
        // either the slot word differs, or the eligibility wake sequence does.
        #[cfg(test)]
        if self.suppress_join_wakes.load(Ordering::SeqCst) {
            return;
        }
        if self.ready_waiting.load(Ordering::SeqCst) {
            self.ready_wake.fetch_add(1, Ordering::SeqCst);
            // SAFETY: stable aligned storage lives through the callback wait.
            unsafe { WakeByAddressSingle(self.ready_wake.as_ptr().cast()) };
        }
        let awaited = self.awaited.load(Ordering::SeqCst);
        if awaited & AWAITED_SLOT_MASK == index as u64 + 1 {
            if awaited & AWAITED_SPIN != 0 {
                self.wake_spin_waiter();
            } else {
                // SAFETY: stable, aligned atomic storage lives through every wait.
                unsafe { WakeByAddressSingle(self.words[index].as_ptr().cast()) };
            }
        }
    }

    fn wake_spin_waiter(&self) {
        #[cfg(test)]
        if self.suppress_join_wakes.load(Ordering::SeqCst) {
            return;
        }
        // Sampled after SC registration and before the word/active reloads.
        // Changing the comparison value closes the wake-before-wait gap for
        // idle transitions, which leave the slot word unchanged. Between that
        // sample and its wait, only this preallocated batch can run: its finite
        // cell count cannot produce enough transitions to wrap this sequence.
        self.spin_wake.fetch_add(1, Ordering::SeqCst);
        // SAFETY: this aligned atomic shares the lifetime of every join waiter.
        unsafe { WakeByAddressSingle(self.spin_wake.as_ptr().cast()) };
    }

    fn wait_active(&self) {
        self.settling.fetch_add(1, Ordering::SeqCst);
        loop {
            let active = self.active.load(Ordering::SeqCst);
            if active == 0 {
                break;
            }
            // The same SC Dekker as publication: the zero decrement either
            // precedes this reload or observes settling and wakes all waiters.
            // SAFETY: stable aligned usize and a same-size local comparison.
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
        let _phase = crate::allocation::enter(crate::allocation::Phase::Acquisition);
        // SAFETY: claim was removed under the mutex, incrementing active and
        // retaining one uncollected admission. Context cannot reset until all
        // admissions are collected, after every such borrow has ended.
        let borrowed_context = unsafe { crate::allocation::borrow_context(self.context.0.get()) };
        let reads = crate::read_accounting::Capture::new();
        #[cfg(feature = "callback_profile")]
        let runner_thread_id = unsafe { GetCurrentThreadId() };
        let runner_begin_qpc = crate::sampling::qpc();
        #[cfg(feature = "executor_join_help")]
        self.body_begin[index].store(runner_begin_qpc.max(0) as u64, Ordering::Release);
        #[cfg(feature = "callback_profile")]
        let runner_cycles_begin = crate::callback_profile::thread_cycles();
        let growths_before = engine_allocation::growths();
        let report = if crate::allocation::failed() {
            Err(Failure::Stopped)
        } else {
            // SAFETY: only this claimant owns the Running cell. The job was
            // initialized at admission; no move or other cell reference occurs
            // while its body runs. The borrow ends inside the unwind boundary.
            match catch_unwind(AssertUnwindSafe(|| unsafe {
                (self.run)(&mut (*completion).request)
            })) {
                Ok(report) if !crate::allocation::failed() => Ok(report),
                Ok(_) => Err(Failure::Stopped),
                Err(payload) => {
                    // Arbitrary payload Drop may panic; a stopped pool bounds
                    // these leaks by its active runners.
                    std::mem::forget(payload);
                    Err(Failure::Panicked)
                }
            }
        };
        let growths = engine_allocation::growths().saturating_sub(growths_before);
        if caller {
            crate::sampling::record_caller_growths(growths);
        }
        #[cfg(feature = "callback_profile")]
        let runner_cycles_end = crate::callback_profile::thread_cycles();
        let runner_end_qpc = crate::sampling::qpc();
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
            // Failure closes admission and cancels queued work before this
            // runner publishes. Running peers keep their owned completions.
            self.lock().stop(self);
            self.work_ready.notify_all();
        }
        // SAFETY: still the sole Running claimant, with the job borrow ended.
        // Initialize report/counts and update only already-initialized stages.
        unsafe {
            std::ptr::addr_of_mut!((*completion).report).write(report);
            std::ptr::addr_of_mut!((*completion).table_reads).write(crate::read_accounting::ZERO);
            reads.finish_into(&mut (*completion).table_reads);
            (*completion).timeline.body_begin_qpc = runner_begin_qpc;
            (*completion).timeline.body_end_qpc = runner_end_qpc;
            (*completion).timeline.growths = growths;
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
        // Counters precede publication: an acquire collector observes them
        // when it immediately calls calibration, even before active retires.
        self.publish(index, claim.generation);
        let active = self.active.fetch_sub(1, Ordering::SeqCst) - 1;
        if active < self.thread_ids.len() {
            // Idle's SC decrement/load pairs with registration then the SC
            // active reload. Either the collector sees the idle worker, or we
            // see its flag and change/wake spin_wake. A wake before its OS wait
            // still returns immediately because the comparison value differs.
            if self.awaited.load(Ordering::SeqCst) & AWAITED_SPIN != 0 {
                self.wake_spin_waiter();
            }
        }
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
        #[cfg(test)]
        if self.help_attempt_gate.armed.swap(false, Ordering::SeqCst) {
            self.help_attempt_gate
                .entered
                .store(true, Ordering::Release);
            let until = std::time::Instant::now() + crate::TEST_HANG_GUARD;
            while !self.help_attempt_gate.released.load(Ordering::Acquire)
                && std::time::Instant::now() < until
            {
                std::thread::yield_now();
            }
        }
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                #[cfg(test)]
                add(&self.counters.help_lock_contentions, 1);
                return None;
            }
        };
        if state.stop
            || crate::allocation::failed()
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
        let elapsed = (crate::sampling::qpc().max(0) as u64).saturating_sub(begin);
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
                // Divide rather than multiply, so a saturated maximum cannot
                // overflow the two-times bound. No history means no help.
                candidate_maximum != 0
                    && candidate_maximum <= remaining / 2
                    && candidate_maximum <= available
            })
            .min_by_key(|(_, index)| state.slots[**index].priority)
            .map(|(position, _)| position)?;
        // Refresh the prediction after searching: bodies and their class
        // histories can advance without the queue mutex while we choose a job.
        let candidate = &state.slots[state.ready[position]];
        let candidate_maximum = self.maxima[candidate.class?].load(Ordering::Relaxed);
        let elapsed = (crate::sampling::qpc().max(0) as u64).saturating_sub(begin);
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

impl<J, R> Drop for Shared<J, R> {
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
                // Match the batch and runner teardown convention: an arbitrary
                // panic payload's destructor must not interrupt later cells.
                std::mem::forget(payload);
            }
        }
    }
}

pub(crate) struct Pool<J: Send + 'static, R: Copy + Send + 'static> {
    shared: Arc<Shared<J, R>>,
    threads: Mutex<Vec<JoinHandle<()>>>,
    allocated_bytes: usize,
}

impl<J: Send + 'static, R: Copy + Send + 'static> Pool<J, R> {
    /// Unclassified pools disable join help, including tests and portable
    /// benchmarks. Zero workers executes accepted jobs on their collector.
    #[cfg(test)]
    pub(crate) fn start(
        worker_count: usize,
        capacity: usize,
        run: impl Fn(&mut J) -> R + Send + Sync + 'static,
    ) -> Result<Self, &'static str> {
        Self::start_boosted(worker_count, capacity, false, run)
    }
    pub(crate) fn start_with_class(
        worker_count: usize,
        capacity: usize,
        classes: Option<Classes<J>>,
        run: impl Fn(&mut J) -> R + Send + Sync + 'static,
    ) -> Result<Self, &'static str> {
        Self::start_classified_boosted(worker_count, capacity, false, classes, run)
    }
    #[cfg(test)]
    fn start_boosted(
        worker_count: usize,
        capacity: usize,
        wake_boost: bool,
        run: impl Fn(&mut J) -> R + Send + Sync + 'static,
    ) -> Result<Self, &'static str> {
        Self::start_classified_boosted(worker_count, capacity, wake_boost, None, run)
    }
    fn start_classified_boosted(
        worker_count: usize,
        capacity: usize,
        wake_boost: bool,
        classes: Option<Classes<J>>,
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
            // SAFETY: valid local output, queried once before workers start.
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
        let shared = Arc::new(Shared {
            id,
            state: Mutex::new(state),
            started_ready: Condvar::new(),
            work_ready: Condvar::new(),
            burst_peers: AtomicUsize::new(0),
            run: Box::new(run),
            thread_ids,
            cells: cells.into_boxed_slice(),
            words: words.into_boxed_slice(),
            active: AtomicUsize::new(0),
            stopped: AtomicBool::new(false),
            uncollected: AtomicUsize::new(0),
            awaited: AtomicU64::new(0),
            spin_wake: AtomicU64::new(0),
            ready_waiting: AtomicBool::new(false),
            ready_wake: AtomicU64::new(0),
            settling: AtomicUsize::new(0),
            counters: Counters::default(),
            #[cfg(test)]
            suppress_join_wakes: AtomicBool::new(false),
            #[cfg(all(test, feature = "executor_join_help"))]
            help_attempt_gate: HelpAttemptGate::default(),
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
            context: BatchContext(UnsafeCell::new(crate::allocation::Context::default())),
        });
        // Arc allocates strong/weak atomic counts followed by its aligned
        // value. Count the real storage; vector capacities and boxed slices
        // include reserved space, and a zero-sized run closure costs no bytes.
        let allocated_bytes = {
            let state = shared.lock();
            let arc_layout = std::alloc::Layout::new::<[AtomicUsize; 2]>()
                .extend(std::alloc::Layout::new::<Shared<J, R>>())
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
                    let _ = pool.shutdown();
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
    /// Keep the foreign live-read lease until EVERY accepted ticket is
    /// collected, even on failure. Mutable outputs must be disjoint; shared
    /// seeds live through all consumers. One collector of pending tickets,
    /// never recursive; the acquire CAS also contains racing ready collectors.
    #[cfg(test)]
    pub(crate) unsafe fn submit(&self, request: &mut Option<J>) -> Result<Ticket, SubmitError> {
        unsafe { self.submit_with_priority(request, 0, false, true) }
    }
    pub(crate) fn reserve_worker_slots(&self, count: usize) -> Result<(), &'static str> {
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

    /// Lower claim priority first, FIFO for ties. Held bursts use wake_workers.
    /// Rejection retains the caller's request, including a full pending pool.
    /// # Safety
    /// The same lease and unique-collector contract as submit.
    pub(crate) unsafe fn submit_with_priority(
        &self,
        request: &mut Option<J>,
        priority: u8,
        worker_only: bool,
        wake: bool,
    ) -> Result<Ticket, SubmitError> {
        #[cfg(feature = "callback_profile")]
        let submit_lock_begin = crate::sampling::qpc();
        let mut state = self.shared.lock();
        #[cfg(feature = "callback_profile")]
        let submit_lock_wait_ticks = crate::sampling::qpc()
            .saturating_sub(submit_lock_begin)
            .max(0) as u64;
        let free = state.free_slot(&self.shared, worker_only);
        let rejection = if request.is_none() {
            Some(SubmitError::Empty)
        } else if state.stop || crate::allocation::failed() {
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
        if self.shared.uncollected.load(Ordering::Acquire) == 0 {
            // SAFETY: no accepted job owns a context borrow; every borrow ends
            // before its completion is published and collected. Parked jobs
            // never run. The mutex excludes any competing admission/reset.
            unsafe { *self.shared.context.0.get() = crate::allocation::context() };
        }
        let index = free.expect("checked free acquisition slot");
        state.generation += 1;
        let generation = state.generation;
        let queued_qpc = crate::sampling::qpc();
        let slot = &mut state.slots[index];
        slot.generation = generation;
        slot.priority = priority;
        slot.worker_only = worker_only;
        slot.queued_qpc = queued_qpc;
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
            std::ptr::addr_of_mut!((*completion).timeline).write(Timeline {
                queued_qpc,
                ..Timeline::default()
            });
            #[cfg(feature = "callback_profile")]
            std::ptr::addr_of_mut!((*completion).profile).write(Profile {
                queued_qpc,
                submit_lock_wait_ticks,
                ..Profile::default()
            });
        }
        self.shared.uncollected.fetch_add(1, Ordering::Relaxed);
        self.shared.words[index].store(word(generation, PENDING), Ordering::Release);
        state.ready.push_back(index);
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

    /// Cancel unstarted work, then wait on active rather than the queue mutex
    /// or a condvar. Completed/failed payloads remain collectable. The Batch
    /// unwind barrier calls this even if one retained collection panicked.
    pub(crate) fn settle(&self) {
        self.shared.lock().cancel_queued(&self.shared);
        self.shared.wait_active();
    }
    pub(crate) fn wake_workers(&self, jobs: usize) {
        if jobs == 0 {
            return;
        }
        let limit = self.shared.thread_ids.len().saturating_sub(1);
        let peers = jobs.saturating_sub(1).min(limit);
        if peers != 0 {
            let _ =
                self.shared
                    .burst_peers
                    .fetch_update(Ordering::Release, Ordering::Relaxed, |old| {
                        Some(old.saturating_add(peers).min(limit))
                    });
        }
        #[cfg(test)]
        {
            self.shared.lock().counters.notified_one += 1;
        }
        // The collector notifies one worker, not every peer in a held burst.
        // Claims relay the rest before their bodies, one notification each.
        self.shared.work_ready.notify_one();
    }
    /// Park never admits, queues or counts a request. Prefer its partition,
    /// then any VACANT cell; exhaustion/fullness retains it with the caller.
    pub(crate) fn park(&self, request: &mut Option<J>, worker_only: bool) -> Option<Ticket> {
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
        slot.queued_qpc = 0;
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
    pub(crate) fn unpark(&self, ticket: Ticket) -> Option<J> {
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

    /// Complete a rejected request in its already allocated cell. `complete`
    /// must not call back into this pool; None retains the job for another path.
    /// In particular, no job-sized request or table-count array is constructed
    /// on the rejection handler's stack.
    #[inline(never)]
    pub(crate) fn finish_parked_into(
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
            std::ptr::copy_nonoverlapping(
                &crate::read_accounting::ZERO,
                std::ptr::addr_of_mut!((*cell).table_reads),
                1,
            );
            std::ptr::addr_of_mut!((*cell).timeline).write(Timeline::default());
            #[cfg(feature = "callback_profile")]
            std::ptr::addr_of_mut!((*cell).profile).write(Profile::default());
            std::ptr::write(out, Some(std::ptr::read(cell)));
        }
        slot.phase = Phase::Vacant;
        self.shared.words[ticket.slot].store(word(ticket.generation, VACANT), Ordering::Release);
        state.parked -= 1;
        true
    }

    #[cfg(test)]
    pub(crate) fn finish(&self, ticket: Ticket) -> Option<Completion<J, R>> {
        let mut out = None;
        self.finish_into(ticket, &mut out);
        out
    }
    pub(crate) fn finish_into(&self, ticket: Ticket, out: &mut Option<Completion<J, R>>) -> bool {
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
            let collected_qpc = crate::sampling::qpc();
            (*completion).timeline.collected_qpc = collected_qpc;
            (*completion).timeline.finish_lock_ticks = lock_ticks;
            #[cfg(feature = "callback_profile")]
            {
                (*completion).profile.collected_qpc = collected_qpc;
                (*completion).profile.finish_lock_wait_ticks = lock_ticks;
            }
            std::ptr::write(out, Some(std::ptr::read(completion)));
        }
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
        let finish_lock_begin = crate::sampling::qpc();
        let mut state = self.shared.lock();
        let finish_lock_ticks = crate::sampling::qpc()
            .saturating_sub(finish_lock_begin)
            .max(0) as u64;
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
        let mut spin_until = None;
        #[cfg(feature = "executor_join_help")]
        let mut help_budget = HelpBudget::new();
        loop {
            if self.collect(ticket, out, finish_lock_ticks) {
                return true;
            }
            let observed = value.load(Ordering::Acquire);
            if observed == word(ticket.generation, PUBLISHED) {
                // Publication may race the failed CAS above. Retry collection
                // instead of treating this newly completed ticket as invalid.
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
            let spin_available = !JOIN_SPIN.is_zero()
                && (!cfg!(feature = "executor_spin_until_pool_busy") || spin_until.is_none())
                && spin_until.is_none_or(|deadline| deadline > std::time::Instant::now())
                && !self.shared.stopped.load(Ordering::SeqCst);
            let registration =
                (ticket.slot as u64 + 1) | if spin_available { AWAITED_SPIN } else { 0 };
            self.shared.awaited.store(registration, Ordering::SeqCst);
            // Snapshot the wake sequence BEFORE reloading eligibility. Claim,
            // idle retirement and publication pair their SC transition/load
            // with our registration/reload. If we missed the transition, its
            // wake changes this comparison value even before the OS wait starts.
            let wake = self.shared.spin_wake.load(Ordering::SeqCst);
            let observed = value.load(Ordering::SeqCst);
            let active = self.shared.active.load(Ordering::SeqCst);
            if spin_available
                && observed == word(ticket.generation, RUNNING)
                && !self.shared.stopped.load(Ordering::SeqCst)
                && active < self.shared.thread_ids.len()
                && spin_until.is_none_or(|deadline| deadline > std::time::Instant::now())
            {
                self.shared.awaited.store(0, Ordering::SeqCst);
                // Like the locked policy, the first eligible pass starts the
                // deadline here and counts a spin even if then descheduled.
                let deadline =
                    *spin_until.get_or_insert_with(|| std::time::Instant::now() + JOIN_SPIN);
                self.spin(ticket, deadline);
                continue;
            }
            if observed == word(ticket.generation, PENDING)
                || observed == word(ticket.generation, RUNNING)
            {
                add(&self.shared.counters.join_sleeps, 1);
                // Eligibility sleeps use the shared sequence; a publisher of
                // this registered slot advances/wakes it as well. Other sleeps
                // use the slot word, with publication's original SC pairing.
                // SAFETY: stable aligned u64 and a same-size local comparison.
                // Spurious returns recheck every predicate without a queue lock.
                let (address, comparison) = if spin_available {
                    (&self.shared.spin_wake, wake)
                } else {
                    (value, observed)
                };
                unsafe {
                    WaitOnAddress(
                        address.as_ptr().cast(),
                        (&comparison as *const u64).cast(),
                        size_of::<u64>(),
                        u32::MAX,
                    )
                };
                // With wakes suppressed, only a fixture's rescue advances the
                // sequence: absorb legal spurious returns until it does, so
                // the suppression test cannot collect early.
                #[cfg(test)]
                while spin_available
                    && self.shared.suppress_join_wakes.load(Ordering::SeqCst)
                    && self.shared.spin_wake.load(Ordering::SeqCst) == wake
                {
                    // SAFETY: as above, on the eligibility sequence.
                    unsafe {
                        WaitOnAddress(
                            self.shared.spin_wake.as_ptr().cast(),
                            (&wake as *const u64).cast(),
                            size_of::<u64>(),
                            u32::MAX,
                        )
                    };
                }
            }
            self.shared.awaited.store(0, Ordering::SeqCst);
        }
    }
    #[inline]
    fn spin(&self, ticket: Ticket, deadline: std::time::Instant) {
        let value = &self.shared.words[ticket.slot];
        let begin = std::time::Instant::now();
        add(&self.shared.counters.spins, 1);
        while value.load(Ordering::Acquire) == word(ticket.generation, RUNNING)
            && (!cfg!(feature = "executor_spin_until_pool_busy")
                || self.shared.active.load(Ordering::Acquire) < self.shared.thread_ids.len())
        {
            for _ in 0..64 {
                std::hint::spin_loop();
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
        }
        add(
            &self.shared.counters.spin_wall_ns,
            begin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64,
        );
    }
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.allocated_bytes
    }
    pub(crate) fn thread_ids(&self) -> Vec<u32> {
        self.shared
            .thread_ids
            .iter()
            .map(|id| id.load(Ordering::Acquire))
            .collect()
    }
    pub(crate) fn calibration(&self) -> Calibration {
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
            spins: counters.spins.load(Ordering::Relaxed),
            spin_wall_ns: counters.spin_wall_ns.load(Ordering::Relaxed),
            join_sleeps: counters.join_sleeps.load(Ordering::Relaxed),
            join_spin_budget_ns: JOIN_SPIN.as_nanos() as u64,
            join_spin_policy: if JOIN_SPIN.is_zero() {
                "disabled"
            } else if cfg!(feature = "executor_spin_until_pool_busy") {
                "until_pool_busy"
            } else {
                "until_deadline"
            },
            join_help: cfg!(feature = "executor_join_help"),
            #[cfg(feature = "executor_join_help")]
            help_executed: counters.help_executed.load(Ordering::Relaxed),
            #[cfg(not(feature = "executor_join_help"))]
            help_executed: 0,
            #[cfg(feature = "executor_join_help")]
            help_ns: counters.help_ns.load(Ordering::Relaxed),
            #[cfg(not(feature = "executor_join_help"))]
            help_ns: 0,
            wake_boost_refusals: BOOST_REFUSALS.load(Ordering::Relaxed),
            priority_refusals: PRIORITY_REFUSALS.load(Ordering::Relaxed),
            joins: JOINS.load(Ordering::Relaxed),
            join_wall_ns: JOIN_WALL_NS.load(Ordering::Relaxed),
            join_wall_max_ns: JOIN_WALL_MAX_NS.load(Ordering::Relaxed),
        }
    }
    /// Wait until the sole callback collector can advance a dependency edge.
    /// `ready` only inspects tickets; it never collects, submits or waits. The
    /// workers must be able to complete the outstanding producers themselves.
    /// Registration precedes both the sequence snapshot and the readiness
    /// recheck: a publication either appears in that recheck or changes the
    /// sequence before the address wait. No queue mutex or payload is borrowed.
    pub(crate) fn wait_ready<T>(&self, mut ready: impl FnMut() -> Option<T>) -> T {
        self.wait_ready_with_spin(JOIN_SPIN, &mut ready)
    }

    fn wait_ready_with_spin<T>(
        &self,
        budget: std::time::Duration,
        mut ready: impl FnMut() -> Option<T>,
    ) -> T {
        if let Some(value) = ready() {
            return value;
        }
        // Keep short producer edges on the callback instead of sleeping
        // between publications. The fixed budget never extends after a wake.
        // This wait inspects tickets only; it never claims a queued job.
        if !budget.is_zero() && !self.shared.stopped.load(Ordering::Acquire) {
            let begin = std::time::Instant::now();
            add(&self.shared.counters.spins, 1);
            let result = loop {
                if let Some(value) = ready() {
                    break Some(value);
                }
                if begin.elapsed() >= budget
                    || self.shared.stopped.load(Ordering::Acquire)
                    || (cfg!(feature = "executor_spin_until_pool_busy")
                        && self.shared.active.load(Ordering::Acquire)
                            >= self.shared.thread_ids.len())
                {
                    break None;
                }
                for _ in 0..64 {
                    std::hint::spin_loop();
                }
            };
            add(
                &self.shared.counters.spin_wall_ns,
                begin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64,
            );
            if let Some(value) = result {
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
            add(&self.shared.counters.join_sleeps, 1);
            // SAFETY: stable aligned u64 and a same-size local comparison.
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

    /// Inspect only a published, copyable report, without taking the request.
    /// The collection CAS excludes a racing collector while it is copied; no
    /// reference to the cell escapes. The sole pending collector may defer a
    /// contained failure's settlement while advancing a successful edge.
    pub(crate) fn completed_report(&self, ticket: Ticket) -> Option<Result<R, Failure>> {
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

    pub(crate) fn is_complete(&self, ticket: Ticket) -> bool {
        ticket.pool == self.shared.id
            && self.shared.words.get(ticket.slot).is_some_and(|value| {
                value.load(Ordering::SeqCst) == word(ticket.generation, PUBLISHED)
            })
    }
    pub(crate) fn stopped(&self) -> bool {
        self.shared.lock().stop
    }
    pub(crate) fn stop_for_contained_panic(&self) {
        add(&self.shared.counters.panicked, 1);
        self.shared.stop();
    }
    pub(crate) fn shutdown(&self) -> Result<(), &'static str> {
        #[cfg(feature = "worker_sampler")]
        crate::worker_sampler::stop_before_thread_exit();
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
impl<J: Send + 'static, R: Copy + Send + 'static> Drop for Pool<J, R> {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

// Above-normal priority and the existing dynamic-wake-boost policy; no affinity
// change. Workers run only while the hourly callback holds the game's
// simulation, so other processes' normal-priority threads cannot lengthen that
// pause; between hours they wait for work and take no CPU.
fn worker_loop<J: Send + 'static, R: Copy + Send + 'static>(
    shared: Arc<Shared<J, R>>,
    index: usize,
    wake_boost: bool,
) {
    #[cfg(feature = "worker_sampler")]
    let _sampler_registration = crate::worker_sampler::register_worker();
    {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, SetThreadPriorityBoost,
            THREAD_PRIORITY_ABOVE_NORMAL,
        };
        // SAFETY: this worker's pseudo-handle needs no close.
        if !wake_boost && unsafe { SetThreadPriorityBoost(GetCurrentThread(), 1) } == 0 {
            BOOST_REFUSALS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: as above.
        if unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL) } == 0 {
            PRIORITY_REFUSALS.fetch_add(1, Ordering::Relaxed);
        }
    }
    shared.thread_ids[index].store(unsafe { GetCurrentThreadId() }, Ordering::Release);
    let mut state = shared.lock();
    state.started += 1;
    shared.started_ready.notify_all();
    loop {
        if state.stop {
            return;
        }
        if let Some(claim) = state.take_ready(&shared, None) {
            let relay = if state.ready.is_empty() {
                // Under the queue mutex: no unclaimed job needs another wake.
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
            #[cfg(test)]
            {
                state.counters.relayed_one += u64::from(relay);
            }
            drop(state);
            if relay {
                shared.work_ready.notify_one();
            }
            shared.execute(claim, false, false);
            state = shared.lock();
        } else {
            shared.burst_peers.store(0, Ordering::Release);
            state = shared.wait_for_work(state);
        }
    }
}

/// One acquisition worker per logical CPU the game may run on, beyond the
/// collecting callback. These reads are memory-latency bound, so SMT siblings
/// add real throughput, and the game's own threads are parked for the whole
/// block. The game's process affinity counts, not the machine's CPUs: with more
/// workers than allowed CPUs, a join spinning "while a worker is idle" would
/// take a CPU from queued work. Query once before hook install; this neither
/// pins threads nor changes game affinity.
pub(crate) fn recommended_worker_count() -> usize {
    let machine = std::thread::available_parallelism().map_or(1, usize::from);
    worker_count_for_cpus(machine, allowed_cpus())
}

/// Workers for `machine` logical CPUs of which the process may use `allowed`
/// (None: affinity unreadable, use the machine count).
fn worker_count_for_cpus(machine: usize, allowed: Option<usize>) -> usize {
    worker_count_for_topology(allowed.map_or(machine, |allowed| allowed.min(machine)))
}

/// Logical CPUs in the game process's affinity mask (current processor group).
fn allowed_cpus() -> Option<usize> {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessAffinityMask};
    let (mut process, mut system) = (0_usize, 0_usize);
    // SAFETY: the pseudo-handle needs no close; both outputs are valid locals.
    let ok = unsafe { GetProcessAffinityMask(GetCurrentProcess(), &mut process, &mut system) };
    (ok != 0 && process != 0).then(|| process.count_ones() as usize)
}

/// Hardware threads minus the callback, within the pool's admitted range.
fn worker_count_for_topology(available: usize) -> usize {
    available.saturating_sub(1).clamp(1, MAX_WORKERS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Barrier, mpsc};

    fn progressed(ready: impl FnMut() -> bool) -> bool {
        progressed_within(crate::TEST_HANG_GUARD, ready)
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
    /// Wake a join on either address it can sleep on: its slot word, or the
    /// eligibility sequence, advanced so a join not yet in its OS wait returns.
    fn wake_join<J: Send + 'static, R: Copy + Send + 'static>(pool: &Pool<J, R>, slot: usize) {
        pool.shared.spin_wake.fetch_add(1, Ordering::SeqCst);
        // SAFETY: stable aligned words of this live pool.
        unsafe {
            WakeByAddressAll(pool.shared.spin_wake.as_ptr().cast());
            WakeByAddressAll(pool.shared.words[slot].as_ptr().cast());
        }
    }
    /// After a fixture's observation window, wake its join repeatedly until
    /// `returned`, within the hang guard, so a lost wake fails an assertion
    /// instead of blocking the scoped join. Returns whether it returned.
    fn rescue_join<J: Send + 'static, R: Copy + Send + 'static>(
        pool: &Pool<J, R>,
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
    /// Always releases a gated body on test unwind before its pool drops.
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
        let pool = Pool::start(1, 1, |job: &mut u8| *job).unwrap();
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
                // Old collection blocks here until the guard releases; mark
                // that release so the test fails an assertion, never hangs.
                let requested = release_rx.recv_timeout(crate::TEST_HANG_GUARD).is_ok();
                released_ref.store(true, Ordering::Release);
                drop(lock);
                requested
            });
            held_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
            let completion = pool.finish(ticket).unwrap();
            let before_release = !released.load(Ordering::Acquire);
            let _ = release_tx.send(());
            let requested = helper.join().unwrap();
            assert!(
                before_release && requested,
                "published collection waited for the queue lock"
            );
            assert_eq!(completion.report, Ok(7));
            assert_eq!(completion.timeline.finish_lock_ticks, 0);
            #[cfg(feature = "callback_profile")]
            assert_eq!(completion.profile.finish_lock_wait_ticks, 0);
        });
        assert!(!pool.is_complete(ticket));
    }

    #[test]
    fn a_worker_publishes_while_the_queue_lock_is_held() {
        let release = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = mpsc::channel();
        let pool = {
            let release = Arc::clone(&release);
            Pool::start(1, 1, move |job: &mut u8| {
                entered_tx.send(()).unwrap();
                wait_until(|| release.load(Ordering::Acquire));
                *job += 1;
                *job
            })
            .unwrap()
        };
        let release = Gate(release);
        let ticket = submit(&pool, &mut Some(7));
        entered_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
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
            let pool = Pool::start(workers, 2, move |job: &mut Request| {
                let address = job as *mut Request as usize;
                job.bytes[0] = 42;
                entered_tx.send(address).unwrap();
                address
            })
            .unwrap();
            let ticket = submit(&pool, &mut Some(Request { bytes: [0; 4096] }));
            let cell_begin = pool.shared.cells[ticket.slot].pointer() as usize;
            let cell_end = cell_begin + size_of::<Cell<Request, usize>>();
            if workers != 0 {
                let address = entered_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
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
        let pool = Pool::start(1, 1, |job: &mut u8| *job).unwrap();
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

    /// A one-worker pool whose worker-only jobs hold until released, so
    /// each join finds its job running with no idle worker and sleeps.
    fn busy_pool() -> (Pool<u64, u64>, Gate, mpsc::Receiver<()>) {
        let release = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = mpsc::channel();
        let pool = {
            let release = Arc::clone(&release);
            Pool::start(1, 1, move |job: &mut u64| {
                entered_tx.send(()).unwrap();
                wait_until(|| release.load(Ordering::Acquire));
                *job
            })
            .unwrap()
        };
        pool.reserve_worker_slots(1).unwrap();
        (pool, Gate(release), entered_rx)
    }

    /// One round: the join sleeps on its running job, then the job publishes.
    /// Returns its report, whether the join was seen asleep, and whether it
    /// returned within `window` without a rescue.
    fn busy_pool_round(
        pool: &Pool<u64, u64>,
        release: &Gate,
        entered: &mpsc::Receiver<()>,
        round: u64,
        window: std::time::Duration,
    ) -> (Result<u64, Failure>, bool, bool) {
        release.0.store(false, Ordering::Release);
        let ticket = unsafe { pool.submit_with_priority(&mut Some(round), 0, true, true) }.unwrap();
        entered.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
        let joined = AtomicBool::new(false);
        let sleeps_before = pool.shared.counters.join_sleeps.load(Ordering::Relaxed);
        std::thread::scope(|scope| {
            let helper = scope.spawn(|| {
                let registered = progressed(|| {
                    pool.shared.awaited.load(Ordering::SeqCst) & AWAITED_SLOT_MASK
                        == ticket.slot as u64 + 1
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
            let (report, registered, woke) =
                busy_pool_round(&pool, &release, &entered, round, crate::TEST_HANG_GUARD);
            assert_eq!(report, Ok(round));
            assert!(registered && woke, "lost address wake in round {round}");
        }
        let calibration = pool.calibration();
        assert_eq!(calibration.spins, 0);
        assert_eq!(calibration.spin_wall_ns, 0);
        assert!(calibration.join_sleeps >= 512);
    }

    /// The fixtures' rescue reaches a join on either wait address: with every
    /// join wake suppressed, the round reports the lost wake and still
    /// collects instead of hanging. An eligible join sleeps on the unchanged
    /// eligibility sequence, so the loss is deterministic.
    #[cfg(not(feature = "executor_no_join_spin"))]
    #[test]
    fn a_suppressed_join_wake_fails_the_fixture_instead_of_hanging() {
        let (pool, release, entered) = busy_pool();
        pool.shared
            .suppress_join_wakes
            .store(true, Ordering::SeqCst);
        let (report, registered, woke) = busy_pool_round(
            &pool,
            &release,
            &entered,
            7,
            std::time::Duration::from_millis(200),
        );
        assert_eq!(report, Ok(7));
        assert!(registered, "the join never slept");
        assert!(!woke, "a suppressed join wake still woke the join");
    }

    #[test]
    fn cancellation_wakes_a_join_sleeping_on_a_worker_only_queued_ticket() {
        let release = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = mpsc::channel();
        let pool = {
            let release = Arc::clone(&release);
            Pool::start(1, 2, move |job: &mut u8| {
                assert_eq!(*job, 1, "cancelled queued job ran");
                entered_tx.send(()).unwrap();
                wait_until(|| release.load(Ordering::Acquire));
                *job
            })
            .unwrap()
        };
        let release = Gate(release);
        pool.reserve_worker_slots(1).unwrap();
        let running = submit(&pool, &mut Some(1));
        entered_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
        let queued = unsafe { pool.submit_with_priority(&mut Some(2), 0, true, true) }.unwrap();
        let joined = AtomicBool::new(false);
        let sleeps_before = pool.shared.counters.join_sleeps.load(Ordering::Relaxed);
        std::thread::scope(|scope| {
            let helper = scope.spawn(|| {
                let registered = progressed(|| {
                    pool.shared.awaited.load(Ordering::SeqCst) & AWAITED_SLOT_MASK
                        == queued.slot as u64 + 1
                        && pool.shared.counters.join_sleeps.load(Ordering::Acquire) > sleeps_before
                });
                pool.stop_for_contained_panic();
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
            assert_eq!(done.table_reads, crate::read_accounting::ZERO);
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
            Pool::start(1, 2, move |job: &mut u8| {
                assert_eq!(*job, 1, "settle ran a queued job");
                entered_tx.send(()).unwrap();
                wait_until(|| release.load(Ordering::Acquire));
                *job
            })
            .unwrap()
        };
        let release = Gate(release);
        let running = submit(&pool, &mut Some(1));
        entered_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
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
                // SAFETY: stable active word in the borrowed live pool;
                // rescue a missing settle wake before asserting.
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
        // Nothing running: the barrier returns with no address wait necessary.
        pool.settle();
        assert_eq!(pool.shared.settling.load(Ordering::SeqCst), 0);
        assert!(!pool.stopped());
    }

    #[test]
    fn the_generation_word_bound_also_refuses_parking_without_taking_the_job() {
        let pool = Pool::start(0, 1, |job: &mut u8| *job).unwrap();
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
                Pool::start(0, 1, move |job: &mut u8| {
                    entered_tx.send(()).unwrap();
                    wait_until(|| release.load(Ordering::Acquire));
                    *job
                })
                .unwrap()
            };
            let release = Gate(release);
            let ticket = submit(&pool, &mut Some(42));
            let stopped = AtomicUsize::new(0);
            std::thread::scope(|scope| {
                let caller = scope.spawn(|| pool.finish(ticket).unwrap());
                entered_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
                let stop = || {
                    pool.shutdown().unwrap();
                    stopped.fetch_add(1, Ordering::Release);
                };
                let first = scope.spawn(stop);
                let second = scope.spawn(stop);
                let registered = progressed(|| pool.shared.settling.load(Ordering::SeqCst) == 2);
                release.open();
                let returned = progressed(|| stopped.load(Ordering::Acquire) == 2);
                if !returned {
                    // SAFETY: stable active word in the borrowed live pool;
                    // rescue a lost wake so the regression fails an assertion.
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

    #[cfg(feature = "executor_no_join_spin")]
    #[test]
    fn zero_spin_feature_sleeps_even_while_a_worker_is_idle() {
        let release = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = mpsc::channel();
        let pool = {
            let release = Arc::clone(&release);
            Pool::start(2, 1, move |job: &mut u8| {
                entered_tx.send(()).unwrap();
                wait_until(|| release.load(Ordering::Acquire));
                *job
            })
            .unwrap()
        };
        let release = Gate(release);
        let ticket = submit(&pool, &mut Some(42));
        entered_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
        std::thread::scope(|scope| {
            let helper = scope.spawn(|| {
                let registered = progressed(|| {
                    pool.shared.awaited.load(Ordering::SeqCst) != 0
                        && pool.shared.counters.join_sleeps.load(Ordering::Acquire) > 0
                });
                release.open();
                registered
            });
            assert_eq!(pool.finish(ticket).unwrap().report, Ok(42));
            assert!(helper.join().unwrap());
        });
        let calibration = pool.calibration();
        assert_eq!(
            (
                calibration.spins,
                calibration.spin_wall_ns,
                calibration.join_spin_budget_ns
            ),
            (0, 0, 0)
        );
        assert!(calibration.join_sleeps > 0);
        assert_eq!(calibration.join_spin_policy, "disabled");
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
            Pool::start_with_class(1, 3, classes, move |job: &mut (u8, u8)| {
                if job.0 == 0 {
                    entered_tx.send(()).unwrap();
                    wait_until(|| release.load(Ordering::Acquire));
                } else {
                    ran_tx.send(std::thread::current().id()).unwrap();
                }
                std::thread::current().id()
            })
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
        entered_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
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
                        .recv_timeout(crate::TEST_HANG_GUARD)
                        .is_ok_and(|thread| thread == caller)
                } else {
                    progressed(|| {
                        pool_ref.shared.awaited.load(Ordering::SeqCst) & AWAITED_SLOT_MASK
                            == awaited.slot as u64 + 1
                    })
                };
                release_ref.open();
                // Retain this receiver until the worker has run a candidate
                // excluded from help; dropping it sooner would panic its body.
                let worker_ran = expected_help
                    || ran_rx
                        .recv_timeout(crate::TEST_HANG_GUARD)
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
            .unwrap()
        };
        let release = Gate(release);
        // Four ticks would admit a one-tick candidate before subtracting
        // elapsed time. Hold the actual body until all four have elapsed.
        pool.shared.means[0].store(4, Ordering::Relaxed);
        pool.shared.means[1].store(1, Ordering::Relaxed);
        pool.shared.maxima[1].store(1, Ordering::Relaxed);
        let awaited = submit(&pool, &mut Some(0));
        entered_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
        let candidate = submit(&pool, &mut Some(1));
        let begin = pool.shared.body_begin[awaited.slot].load(Ordering::Acquire);
        wait_until(|| (crate::sampling::qpc().max(0) as u64).saturating_sub(begin) >= 4);
        let helped = pool.shared.try_help(awaited, &mut HelpBudget::new());
        if let Some(claim) = helped {
            // Complete even an incorrect claim before asserting, so this test
            // cannot strand active on the failure it is meant to catch.
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
            .unwrap()
        };
        let release = Gate(release);
        pool.shared.means[0].store(pool.shared.frequency * 60, Ordering::Relaxed);
        pool.shared.means[1].store((pool.shared.frequency / 1000).max(1), Ordering::Relaxed);
        pool.shared.maxima[1].store((pool.shared.frequency / 1000).max(1), Ordering::Relaxed);
        let awaited = submit(&pool, &mut Some(0));
        entered_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
        let low = unsafe { pool.submit_with_priority(&mut Some(1), 9, false, true) }.unwrap();
        let high = unsafe { pool.submit_with_priority(&mut Some(2), 1, false, true) }.unwrap();
        std::thread::scope(|scope| {
            let helper = scope.spawn(move || {
                let first = ran_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
                let second = ran_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
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
    #[test]
    fn join_help_body_history_uses_a_decaying_maximum() {
        let pool = Pool::start_with_class(0, 1, Some((2, |_: &u8| 0)), |_: &mut u8| ()).unwrap();
        for previous in [u64::MAX, 1] {
            pool.shared.maxima[0].store(previous, Ordering::Relaxed);
            let ticket = submit(&pool, &mut Some(0));
            let done = pool.finish(ticket).unwrap();
            let sample = done
                .timeline
                .body_end_qpc
                .saturating_sub(done.timeline.body_begin_qpc)
                .max(1) as u64;
            assert_eq!(
                pool.shared.maxima[0].load(Ordering::Relaxed),
                sample.max(previous - previous / 16)
            );
            assert_eq!(pool.shared.maxima[1].load(Ordering::Relaxed), 0);
        }
    }

    #[cfg(feature = "executor_join_help")]
    fn help_budget_case(time_budget: bool) {
        let release = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = mpsc::channel();
        let pool = {
            let release = Arc::clone(&release);
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
        entered_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
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
            Pool::start_with_class(
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
            .unwrap()
        };
        let release = Gate(release);
        pool.shared.means[0].store(pool.shared.frequency * 60, Ordering::Relaxed);
        let awaited = submit(&pool, &mut Some(0));
        entered_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
        let candidate = submit(&pool, &mut Some(1));
        pool.shared
            .help_attempt_gate
            .armed
            .store(true, Ordering::SeqCst);
        std::thread::scope(|scope| {
            let (done_tx, done_rx) = mpsc::channel();
            let pool_ref = &pool;
            let collector = scope.spawn(move || {
                let done = pool_ref.finish(awaited).unwrap();
                done_tx.send(done.report).unwrap();
            });
            // Hold the first budgeted attempt after validation unlocks, before
            // its try_lock. Spurious address returns cannot spend its budget
            // before we own the mutex and install the candidate's history.
            let armed = progressed(|| {
                pool.shared
                    .help_attempt_gate
                    .entered
                    .load(Ordering::Acquire)
            });
            let lock = armed.then(|| pool.shared.lock());
            pool.shared.means[1].store(1, Ordering::Relaxed);
            pool.shared.maxima[1].store(1, Ordering::Relaxed);
            pool.shared
                .help_attempt_gate
                .released
                .store(true, Ordering::Release);
            let contended = armed
                && progressed(|| {
                    pool.shared
                        .counters
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
            let collected_while_held = done_rx.recv_timeout(crate::TEST_HANG_GUARD);
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
            let pool = Pool::start(0, 3, |_: &mut OwnedDrop| ()).unwrap();
            let first = submit(&pool, &mut Some(OwnedDrop(Arc::clone(&dropped))));
            let _second = submit(&pool, &mut Some(OwnedDrop(Arc::clone(&dropped))));
            let _parked = pool
                .park(&mut Some(OwnedDrop(Arc::clone(&dropped))), false)
                .unwrap();
            drop(pool.finish(first).unwrap());
            assert_eq!(dropped.load(Ordering::Relaxed), 1);
            pool.shutdown().unwrap();
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
        let pool = Pool::start(0, 3, |_: &mut Request| ()).unwrap();
        // Admission uses highest slot first. Arrange ascending drain order:
        // slot 0 panics, slot 1 is healthy Published, slot 2 is healthy Parked.
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

    #[test]
    fn executor_allocated_bytes_include_every_real_allocation() {
        fn verify<J: Send + 'static, R: Copy + Send + 'static>(
            workers: usize,
            capacity: usize,
            classes: Classes<J>,
            run: impl Fn(&mut J) -> R + Send + Sync + 'static,
        ) {
            let runner_bytes = size_of_val(&run);
            assert!(runner_bytes > 0, "fixture must cover the boxed runner");
            let pool = Pool::start_classified_boosted(workers, capacity, false, Some(classes), run)
                .unwrap();
            assert_eq!(pool.shared.cells.len(), capacity);
            assert_eq!(pool.shared.words.len(), capacity);
            assert_eq!(size_of::<Cell<J, R>>(), size_of::<Completion<J, R>>());
            let state = pool.shared.lock();
            let threads = pool.threads.lock().unwrap();
            // Compute the Arc header's padding independently of Layout::extend
            // and account for each live allocation, including reserved space.
            let shared_alignment = std::mem::align_of::<Shared<J, R>>();
            let shared_offset = (2 * size_of::<AtomicUsize>()).next_multiple_of(shared_alignment);
            let arc_alignment = shared_alignment.max(std::mem::align_of::<AtomicUsize>());
            let allocations = [
                (shared_offset + size_of::<Shared<J, R>>()).next_multiple_of(arc_alignment),
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
            pool.shutdown().unwrap();
        }
        for workers in [0, 2] {
            for capacity in [1, 3, crate::acquisition_jobs::MAX_JOBS] {
                let bias = [7u8; 129];
                verify(workers, capacity, (3, |_| 0), move |request: &mut u8| {
                    u64::from(*request) + u64::from(std::hint::black_box(&bias)[0])
                });
                verify(
                    workers,
                    capacity,
                    (3, |_| 0),
                    move |_: &mut crate::acquisition_jobs::Job| {
                        crate::acquisition_jobs::Report::World(
                            crate::acquisition_jobs::WorldReport {
                                ticks: u64::from(std::hint::black_box(&bias)[0]),
                                ..Default::default()
                            },
                        )
                    },
                );
            }
        }
    }

    /// Every completion carries when it was queued, when its body ran (on a
    /// worker or on the joining caller), when it was published and when it
    /// was collected; a canceled job its queueing, publication and collection.
    #[test]
    fn completions_carry_their_queue_and_body_timeline() {
        for workers in [0, 1] {
            let pool = Pool::start(workers, 1, |job: &mut u8| *job).unwrap();
            let mut job = Some(7);
            let ticket = unsafe { pool.submit(&mut job) }.unwrap();
            let completion = pool.finish(ticket).unwrap();
            assert_eq!(completion.report, Ok(7));
            let timeline = completion.timeline;
            assert!(timeline.queued_qpc > 0);
            assert!(timeline.queued_qpc <= timeline.body_begin_qpc);
            assert!(timeline.body_begin_qpc <= timeline.body_end_qpc);
            assert!(timeline.body_end_qpc <= timeline.published_qpc);
            assert!(timeline.published_qpc <= timeline.collected_qpc);
        }
        let pool = Pool::start(0, 1, |_: &mut u8| ()).unwrap();
        let mut job = Some(7);
        let ticket = unsafe { pool.submit(&mut job) }.unwrap();
        pool.shutdown().unwrap();
        let timeline = pool.finish(ticket).unwrap().timeline;
        assert!(timeline.queued_qpc > 0);
        assert_eq!((timeline.body_begin_qpc, timeline.body_end_qpc), (0, 0));
        assert!(timeline.queued_qpc <= timeline.published_qpc);
        assert!(timeline.published_qpc <= timeline.collected_qpc);
    }

    #[test]
    fn completions_stamp_only_their_job_body_growths() {
        for workers in [0, 1] {
            let pool = Pool::start(workers, 1, |job: &mut (bool, Vec<u8>)| {
                if job.0 {
                    engine_allocation::reserve(&mut job.1, 1).unwrap();
                    job.1.push(7);
                }
            })
            .unwrap();
            for grows in [true, false] {
                // Unrelated caller allocations must not enter the job's stamp.
                let _unrelated = engine_allocation::vec::<u64>(8).unwrap();
                let mut job = Some((grows, Vec::new()));
                let ticket = unsafe { pool.submit(&mut job) }.unwrap();
                let completion = pool.finish(ticket).unwrap();
                assert_eq!(completion.report, Ok(()));
                if grows {
                    assert!(completion.timeline.growths >= 1);
                } else {
                    assert_eq!(completion.timeline.growths, 0);
                }
            }
            pool.shutdown().unwrap();
        }
    }

    #[cfg(feature = "callback_profile")]
    #[test]
    fn executor_profile_orders_reached_stages_and_marks_canceled_work() {
        for workers in [0, 1] {
            let pool =
                Pool::start(workers, 1, |_: &mut u8| unsafe { GetCurrentThreadId() }).unwrap();
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
        let pool = Pool::start(0, 1, |_: &mut u8| ()).unwrap();
        let mut job = Some(7);
        let ticket = unsafe { pool.submit(&mut job) }.unwrap();
        pool.shutdown().unwrap();
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
        let pool = Pool::start(1, 3, |_: &mut u8| std::thread::current().id()).unwrap();
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
        let stopped = Pool::start(0, 1, |_: &mut u8| ()).unwrap();
        assert!(stopped.reserve_worker_slots(1).is_err());
    }

    #[test]
    fn a_parked_request_never_runs_and_returns_once_to_its_submitter() {
        // Reserving worker-only slots requires a worker.
        for workers in [1, 2] {
            let pool = Pool::start(workers, 3, |request: &mut u8| *request).unwrap();
            pool.reserve_worker_slots(2).unwrap();
            pool.shutdown().unwrap();
            // A stopped pool rejects; the request parks in its own partition.
            let mut request = Some(7);
            assert_eq!(
                unsafe { pool.submit_with_priority(&mut request, 0, true, true) },
                Err(SubmitError::Stopped)
            );
            let parked = pool.park(&mut request, true).unwrap();
            assert!(request.is_none());
            assert!(parked.slot < 2);
            // Collection neither runs nor returns it, and waits for nothing.
            assert!(pool.finish(parked).is_none());
            let calibration = pool.calibration();
            assert_eq!(calibration.accepted, 0);
            assert_eq!(calibration.rejected, 1);
            assert_eq!(calibration.queued, 0);
            assert_eq!(calibration.uncollected, 0);
            assert_eq!(calibration.caller_executed + calibration.worker_executed, 0);
            // A full partition parks in the other one; a full pool leaves the
            // request with its caller.
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
        // A parked request beside idle runners stays parked; its slot is
        // reused once returned, and a stale ticket takes nothing.
        let pool = Pool::start(1, 1, |request: &mut u8| *request + 1).unwrap();
        let parked = pool.park(&mut Some(3), false).unwrap();
        assert_eq!(unsafe { pool.submit(&mut Some(4)) }, Err(SubmitError::Full));
        std::thread::sleep(std::time::Duration::from_millis(20));
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
        let pool = Pool::start(1, 1, |_: &mut Box<[u8; 4096]>| -> u8 {
            panic!("rejected worker must never run")
        })
        .unwrap();
        let owner = Box::new([7; 4096]);
        let allocation = owner.as_ptr();
        let ticket = pool.park(&mut Some(owner), true).unwrap();
        let cell = pool.shared.cells[ticket.slot].pointer();
        let mut out = None;
        assert!(!pool.finish_parked_into(ticket, &mut out, |_| None));
        assert!(out.is_none());
        crate::test_support::with_allocations_denied(|| {
            assert!(pool.finish_parked_into(ticket, &mut out, |request| {
                // SAFETY: the handler exclusively borrows this parked cell.
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
        assert_eq!(completed.table_reads, crate::read_accounting::ZERO);
        assert!(!pool.finish_parked_into(ticket, &mut out, |_| panic!("stale ticket")));
        assert!(pool.unpark(ticket).is_none());
        let next = pool.park(&mut Some(Box::new([0; 4096])), true).unwrap();
        assert_eq!(next.slot, ticket.slot);
        assert!(!pool.finish_parked_into(ticket, &mut out, |_| panic!("reused ticket")));
        assert!(pool.unpark(next).is_some());
    }

    #[test]
    fn panicking_parked_rejection_keeps_the_job_owned_for_drain() {
        let pool = Pool::start(1, 1, |request: &mut u8| *request).unwrap();
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

    fn submit<J: Send + 'static, R: Copy + Send + 'static>(
        pool: &Pool<J, R>,
        request: &mut Option<J>,
    ) -> Ticket {
        // Tests own ordinary Rust data and keep all accepted leases alive.
        unsafe { pool.submit(request) }.unwrap()
    }

    #[test]
    fn owned_buffers_return_and_fixed_slots_reuse_without_replacement() {
        for workers in [0, 2, MAX_WORKERS] {
            let pool = Pool::start(workers, 1, change_owned).unwrap();
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
            pool.shutdown().unwrap();
            pool.shutdown().unwrap();
        }
    }

    #[test]
    fn caller_claims_awaited_ticket_and_keeps_other_slots_owned() {
        let pool = Pool::start(0, 2, |request: &mut u8| *request + 1).unwrap();
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
        pool.shutdown().unwrap();
        assert_eq!(
            unsafe { pool.submit(&mut rejected) },
            Err(SubmitError::Stopped)
        );
        assert_eq!(rejected, Some(9));
    }

    #[test]
    fn stale_foreign_and_exhausted_tickets_never_consume_another_request() {
        let first_pool = Pool::start(0, 1, |request: &mut u8| *request).unwrap();
        let other_pool = Pool::start(0, 1, |request: &mut u8| *request).unwrap();
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
        use engine_native::{
            Memory, memory::CountedMemory, memory::ReadCounts, memory::SliceMemory,
        };
        use replay_data::rows::TableId;

        let calls = Arc::new(AtomicU64::new(0));
        let count = Arc::clone(&calls);
        let pool = Pool::start(0, 2, move |request: &mut Owned| -> usize {
            count.fetch_add(1, Ordering::Relaxed);
            let _reads = crate::read_accounting::table(TableId::Divisions);
            let memory = CountedMemory::new(SliceMemory {
                base: 0x1000,
                bytes: &request.payload,
            });
            assert!(memory.copy(0x1000, &mut [0]));
            request.payload[0] = 42;
            panic!("synthetic acquisition failure");
        })
        .unwrap();
        let make = || Owned { payload: vec![7] };
        let first = submit(&pool, &mut Some(make()));
        let second = submit(&pool, &mut Some(make()));
        let failed = pool.finish(second).unwrap();
        let canceled = pool.finish(first).unwrap();
        assert_eq!(canceled.report, Err(Failure::Stopped));
        assert_eq!(canceled.table_reads, crate::read_accounting::ZERO);
        assert_eq!(canceled.request.payload, [7]);
        assert_eq!(failed.report, Err(Failure::Panicked));
        let mut expected = crate::read_accounting::ZERO;
        expected[TableId::Divisions.index()] = ReadCounts {
            copy_calls: 1,
            spans: 1,
            bytes: 1,
            ..ReadCounts::ZERO
        };
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
        let mut owner = Arc::new(vec![7]);
        let pool: Pool<Arc<Vec<u8>>, ()> =
            Pool::start(0, 1, |_: &mut Arc<Vec<u8>>| panic!("canceled job ran")).unwrap();
        let ticket = submit(&pool, &mut Some(Arc::clone(&owner)));
        pool.shutdown().unwrap();
        assert!(Arc::get_mut(&mut owner).is_none());
        let completion = pool.finish(ticket).unwrap();
        assert_eq!(completion.report, Err(Failure::Stopped));
        assert_eq!(completion.table_reads, crate::read_accounting::ZERO);
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
            let pool = Pool::start(1, 1, move |live: &mut Arc<AtomicBool>| {
                live.store(true, Ordering::Release);
                let _lease = Lease(live);
                assert!(!panics, "synthetic reader failure");
                true
            })
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
        let pool = Pool::start(1, 3, move |request: &mut u8| {
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
        pool.shutdown().unwrap();
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
        let pool = Pool::start(2, 3, move |request: &mut u8| {
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
        .unwrap();
        let first = submit(&pool, &mut Some(1));
        let first_started = entered_rx.recv().unwrap();
        let second = submit(&pool, &mut Some(2));
        let second_started = entered_rx.recv().unwrap();
        let queued = submit(&pool, &mut Some(3));
        panic_release.wait();
        // Observe completion without callback assistance, so the panic must
        // originate on the worker and cancel the still-queued third request.
        wait_until(|| pool.shared.counters.panicked.load(Ordering::Acquire) != 0);
        let failed = pool.finish(second).unwrap();
        let canceled = pool.finish(queued).unwrap();
        wait_until(|| pool.shared.active.load(Ordering::Acquire) == 1);
        let active_after_panic = pool.calibration().active;
        successful_release.wait();
        let successful = pool.finish(first).unwrap();
        pool.shutdown().unwrap();
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
        let pool = Pool::start(2, 1, move |request: &mut u8| -> u8 {
            entered_tx.send(()).unwrap();
            *request = 9;
            panic!("synthetic split-notification failure");
        })
        .unwrap();
        let ticket = submit(&pool, &mut Some(1));
        // Submit must wake one idle worker through work_ready, not changed.
        entered_rx.recv().unwrap();

        // Atomic publication wakes a collector. The same panic-induced stop
        // also wakes the idle peer from work_ready so both workers can exit
        // without a second stop notification.
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
        pool.shutdown().unwrap();
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
        let pool = Pool::start(2, 3, move |request: &mut u8| {
            entered_tx.send(*request).unwrap();
            meet.wait();
            *request += 1;
            *request
        })
        .unwrap();
        let first = submit(&pool, &mut Some(1));
        assert_eq!(entered_rx.recv().unwrap(), 1);
        let second = submit(&pool, &mut Some(2));
        assert_eq!(entered_rx.recv().unwrap(), 2);
        // Both workers are occupied. Collect the queued third participant;
        // collecting a running participant cannot supply its unrelated peer.
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
        let pool = Arc::new(
            Pool::start(0, 1, move |request: &mut u8| {
                entered_tx.send(()).unwrap();
                gate.wait();
                *request += 1;
                *request
            })
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
            shutdown_pool.shutdown().unwrap();
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
        let limit = std::time::Instant::now() + crate::TEST_HANG_GUARD;
        while pool.calibration().active < active {
            assert!(std::time::Instant::now() < limit, "job never started");
            std::thread::yield_now();
        }
    }

    /// A runner that counts starts, then sleeps its request in milliseconds.
    fn sleeping_pool(workers: usize) -> (Pool<u64, u64>, Arc<AtomicU64>) {
        let started = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&started);
        let pool = Pool::start(workers, 4, move |sleep_ms: &mut u64| {
            counter.fetch_add(1, Ordering::AcqRel);
            std::thread::sleep(std::time::Duration::from_millis(*sleep_ms));
            *sleep_ms
        })
        .unwrap();
        pool.reserve_worker_slots(4).unwrap();
        (pool, started)
    }

    fn wait_for_starts(started: &AtomicU64, count: u64) {
        let limit = std::time::Instant::now() + crate::TEST_HANG_GUARD;
        while started.load(Ordering::Acquire) < count {
            assert!(std::time::Instant::now() < limit, "job never started");
            std::thread::yield_now();
        }
    }

    #[cfg(not(any(
        feature = "executor_no_join_spin",
        feature = "executor_spin_until_pool_busy"
    )))]
    fn queued_join_spin_case(stop_before_idle: bool) {
        let gates: [Arc<AtomicBool>; 3] = std::array::from_fn(|_| Arc::new(AtomicBool::new(false)));
        let (entered_tx, entered_rx) = mpsc::channel();
        let pool = {
            let gates = gates.clone();
            Pool::start(2, 3, move |job: &mut usize| {
                entered_tx.send(*job).unwrap();
                while !gates[*job].load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                *job
            })
            .unwrap()
        };
        // Setup failures release every body before the pool's shutdown runs.
        let gates = gates.map(Gate);
        pool.reserve_worker_slots(3).unwrap();
        let first = unsafe { pool.submit_with_priority(&mut Some(0), 0, true, true) }.unwrap();
        assert_eq!(entered_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap(), 0);
        let second = unsafe { pool.submit_with_priority(&mut Some(1), 0, true, true) }.unwrap();
        assert_eq!(entered_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap(), 1);
        let awaited = unsafe { pool.submit_with_priority(&mut Some(2), 0, true, true) }.unwrap();
        if stop_before_idle {
            // Start this variant's join on Running, so a stale initial stop
            // snapshot would spin after the pool stops and B becomes idle.
            gates[0].open();
            assert_eq!(entered_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap(), 2);
        }
        let spins_before = pool.shared.counters.spins.load(Ordering::Acquire);
        std::thread::scope(|scope| {
            let (done_tx, done_rx) = mpsc::channel();
            let pool_ref = &pool;
            let collector = scope.spawn(move || {
                done_tx
                    .send(pool_ref.finish(awaited).unwrap().report)
                    .unwrap();
            });
            let slept_queued = progressed(|| {
                pool.shared.awaited.load(Ordering::SeqCst) != 0
                    && pool.shared.counters.join_sleeps.load(Ordering::Acquire) > 0
            });
            // A retires, then its worker claims C, which remains gated. B is
            // still running, so claim alone must not start a busy-pool spin.
            gates[0].open();
            let claimed =
                stop_before_idle || entered_rx.recv_timeout(crate::TEST_HANG_GUARD) == Ok(2);
            let slept_running =
                claimed && progressed(|| pool.shared.awaited.load(Ordering::SeqCst) != 0);
            let spins_while_busy = pool.shared.counters.spins.load(Ordering::Acquire);
            if stop_before_idle {
                pool.stop_for_contained_panic();
            }
            // B's retirement leaves C running with an idle worker. The join
            // must reconsider its current RUNNING generation before C publishes.
            gates[1].open();
            let reconsidered_stop = if stop_before_idle {
                let idle = progressed(|| pool.shared.active.load(Ordering::SeqCst) == 1);
                // Force a legal spurious return to test the current stop check.
                let sleeps = pool.shared.counters.join_sleeps.load(Ordering::Acquire);
                let reconsidered = progressed(|| {
                    wake_join(&pool, awaited.slot);
                    pool.shared.counters.join_sleeps.load(Ordering::Acquire) > sleeps
                });
                idle && reconsidered
            } else {
                true
            };
            let spun_before_publication = if stop_before_idle {
                pool.shared.counters.spins.load(Ordering::Acquire) > spins_before
            } else {
                progressed(|| pool.shared.counters.spins.load(Ordering::Acquire) > spins_before)
            };
            let unpublished = !pool.is_complete(awaited);
            // No assertion or fallible unwrap above can strand scoped threads:
            // always release all gates, even when the old join stays asleep.
            for gate in &gates {
                gate.open();
            }
            let report = done_rx.recv_timeout(crate::TEST_HANG_GUARD);
            if report.is_err() {
                rescue_join(&pool, awaited.slot, || collector.is_finished());
            }
            collector.join().unwrap();
            assert!(
                slept_queued,
                "worker-only join did not sleep before the transition"
            );
            assert!(
                claimed && slept_running,
                "awaited queued ticket was not claimed"
            );
            assert!(
                reconsidered_stop,
                "stopped running join did not reconsider after an idle transition"
            );
            assert_eq!(
                spins_while_busy, spins_before,
                "joined a busy pool by spinning"
            );
            assert!(
                unpublished,
                "awaited body published before the spin observation"
            );
            assert_eq!(
                spun_before_publication, !stop_before_idle,
                "join did not follow current spin eligibility"
            );
            assert_eq!(report.unwrap(), Ok(2));
        });
        assert_eq!(pool.finish(first).unwrap().report, Ok(0));
        assert_eq!(pool.finish(second).unwrap().report, Ok(1));
        assert_eq!(pool.calibration().caller_executed, 0);
    }

    #[cfg(not(any(
        feature = "executor_no_join_spin",
        feature = "executor_spin_until_pool_busy"
    )))]
    #[test]
    fn queued_worker_only_join_spins_after_claim_and_another_worker_retires() {
        queued_join_spin_case(false);
    }

    #[cfg(not(any(
        feature = "executor_no_join_spin",
        feature = "executor_spin_until_pool_busy"
    )))]
    #[test]
    fn a_stopped_pool_does_not_start_a_spin_after_another_worker_retires() {
        queued_join_spin_case(true);
    }

    #[cfg(not(feature = "executor_no_join_spin"))]
    #[test]
    fn collector_starts_one_spin_on_its_running_ticket_when_a_worker_is_idle() {
        // Each job holds its worker until the test releases it, so the join
        // always finds its ticket running however the threads are scheduled.
        let released = Arc::new(AtomicU64::new(0));
        let started = Arc::new(AtomicU64::new(0));
        let (gate, counter) = (Arc::clone(&released), Arc::clone(&started));
        let pool = Pool::start(2, 4, move |job: &mut u64| {
            let order = counter.fetch_add(1, Ordering::AcqRel) + 1;
            while gate.load(Ordering::Acquire) < order {
                std::thread::yield_now();
            }
            *job
        })
        .unwrap();
        pool.reserve_worker_slots(4).unwrap();
        // Collects the `order`th job while a helper waits for the join to
        // spin, holds the job for `hold` more, then releases it. The helper
        // always releases, so a join that never spins fails instead of hanging.
        let collect = |job: u64, order: u64, hold: std::time::Duration| {
            let spins_before = pool.calibration().spins;
            let ticket =
                unsafe { pool.submit_with_priority(&mut Some(job), 0, true, true) }.unwrap();
            wait_for_starts(&started, order);
            std::thread::scope(|scope| {
                let helper = scope.spawn(|| {
                    let limit = std::time::Instant::now() + crate::TEST_HANG_GUARD;
                    while pool.calibration().spins == spins_before
                        && std::time::Instant::now() < limit
                    {
                        std::thread::yield_now();
                    }
                    let spun = pool.calibration().spins > spins_before;
                    std::thread::sleep(hold);
                    released.store(order, Ordering::Release);
                    spun
                });
                let report = pool.finish(ticket).unwrap().report.unwrap();
                (ticket, report, helper.join().unwrap())
            })
        };
        // One running job, one idle worker: the join spins and collects it.
        let (short, report, spun) = collect(20, 1, std::time::Duration::ZERO);
        assert_eq!((report, spun), (20, true));
        let after_short = pool.calibration().spins;
        pool.settle();
        // A job held past the cap is still collected after the spin parks.
        // The cap's deadline is set before the spin is counted, so holding
        // for longer than the cap after seeing the spin always outlasts it.
        let (_, report, spun) = collect(50, 2, JOIN_SPIN * 25);
        assert_eq!((report, spun), (50, true));
        let calibration = pool.calibration();
        assert_eq!(calibration.spins, after_short + 1);
        assert!(
            calibration.join_sleeps > 0,
            "spent spin budget did not sleep"
        );
        // Worker-only tickets are never executed by the collector.
        assert_eq!(calibration.caller_executed, 0);
        assert_eq!(calibration.worker_executed, 2);
        assert!(pool.finish(short).is_none());
        // A slot reused by a later generation is not mistaken for the old one.
        released.store(u64::MAX, Ordering::Release);
        let again = unsafe { pool.submit_with_priority(&mut Some(2), 0, true, true) }.unwrap();
        assert_eq!(pool.finish(again).unwrap().report.unwrap(), 2);
    }

    #[cfg(not(feature = "executor_no_join_spin"))]
    fn spin_policy_case(stop_when_busy: bool) {
        let gates: [Arc<AtomicBool>; 2] = std::array::from_fn(|_| Arc::new(AtomicBool::new(false)));
        let (entered_tx, entered_rx) = mpsc::channel();
        let pool = {
            let gates = gates.clone();
            Pool::start(2, 2, move |job: &mut usize| {
                entered_tx.send(*job).unwrap();
                wait_until(|| gates[*job].load(Ordering::Acquire));
                *job
            })
            .unwrap()
        };
        let gates = gates.map(Gate);
        pool.reserve_worker_slots(2).unwrap();
        let awaited = unsafe { pool.submit_with_priority(&mut Some(0), 0, true, true) }.unwrap();
        assert_eq!(entered_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap(), 0);
        std::thread::scope(|scope| {
            let (spun_tx, spun_rx) = mpsc::channel();
            let pool_ref = &pool;
            let spinner = scope.spawn(move || {
                // Exercise the same spin loop with enough time to gate the
                // pool-busy transition independent of the OS's 2 ms scheduling.
                pool_ref.spin(awaited, std::time::Instant::now() + crate::TEST_HANG_GUARD);
                spun_tx.send(()).unwrap();
            });
            wait_until(|| pool.shared.counters.spins.load(Ordering::Acquire) == 1);
            let busy = unsafe { pool.submit_with_priority(&mut Some(1), 0, true, true) }.unwrap();
            assert_eq!(entered_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap(), 1);
            assert_eq!(pool.shared.active.load(Ordering::Acquire), 2);
            let ended_while_busy = spun_rx
                .recv_timeout(if stop_when_busy {
                    crate::TEST_HANG_GUARD
                } else {
                    // A negative observation while both body gates stay closed;
                    // no sleep is used to establish either worker's state.
                    std::time::Duration::from_millis(50)
                })
                .is_ok();
            gates[0].open();
            gates[1].open();
            if !ended_while_busy {
                spun_rx.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
            }
            spinner.join().unwrap();
            assert_eq!(ended_while_busy, stop_when_busy);
            wait_until(|| pool.is_complete(awaited) && pool.is_complete(busy));
            assert_eq!(pool.finish(awaited).unwrap().report, Ok(0));
            assert_eq!(pool.finish(busy).unwrap().report, Ok(1));
        });
        let calibration = pool.calibration();
        assert_eq!(calibration.spins, 1);
        assert_eq!(
            calibration.join_spin_policy,
            if stop_when_busy {
                "until_pool_busy"
            } else {
                "until_deadline"
            }
        );
    }

    #[cfg(not(any(
        feature = "executor_no_join_spin",
        feature = "executor_spin_until_pool_busy"
    )))]
    #[test]
    fn default_spin_continues_after_the_pool_becomes_busy() {
        spin_policy_case(false);
    }

    #[cfg(all(
        feature = "executor_spin_until_pool_busy",
        not(feature = "executor_no_join_spin")
    ))]
    #[test]
    fn pool_busy_spin_feature_ends_the_spin_episode_when_all_workers_are_active() {
        spin_policy_case(true);
    }

    #[test]
    fn collector_parks_without_spinning_when_every_worker_is_busy() {
        let (pool, started) = sleeping_pool(1);
        let running = unsafe { pool.submit_with_priority(&mut Some(20), 0, true, true) }.unwrap();
        wait_for_starts(&started, 1);
        // Queued behind the only worker, then running on it: never a spare CPU.
        let queued = unsafe { pool.submit_with_priority(&mut Some(2), 0, true, true) }.unwrap();
        assert_eq!(pool.finish(queued).unwrap().report.unwrap(), 2);
        assert_eq!(pool.finish(running).unwrap().report.unwrap(), 20);
        assert_eq!(pool.calibration().spins, 0);
    }

    #[test]
    fn a_panic_during_a_spin_is_collected_and_cancels_queued_work() {
        // Gates hold both jobs running until the queue is arranged, so the
        // scenario does not depend on scheduling speed.
        let panic_gate = Arc::new(AtomicBool::new(false));
        let slow_gate = Arc::new(AtomicBool::new(false));
        let (panic_now, slow_done) = (Arc::clone(&panic_gate), Arc::clone(&slow_gate));
        let pool = Pool::start(2, 4, move |value: &mut u64| {
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
        .unwrap();
        pool.reserve_worker_slots(4).unwrap();
        let slow = unsafe { pool.submit_with_priority(&mut Some(50), 0, true, true) }.unwrap();
        wait_until_running(&pool, 1);
        let failing = unsafe { pool.submit_with_priority(&mut Some(99), 0, true, true) }.unwrap();
        wait_until_running(&pool, 2);
        // Both workers are busy, so this ticket waits in the queue.
        let queued = unsafe { pool.submit_with_priority(&mut Some(7), 0, true, true) }.unwrap();
        panic_gate.store(true, Ordering::Release);
        assert!(matches!(
            pool.finish(failing).unwrap().report,
            Err(Failure::Panicked)
        ));
        // The panic cancelled the queued ticket and published it for waiters.
        assert!(matches!(
            pool.finish(queued).unwrap().report,
            Err(Failure::Stopped)
        ));
        assert_eq!(
            pool.shared.words[queued.slot].load(Ordering::Acquire),
            word(queued.generation, VACANT)
        );
        // The slow job was already running and completes normally.
        slow_gate.store(true, Ordering::Release);
        assert_eq!(pool.finish(slow).unwrap().report.unwrap(), 50);
    }

    #[test]
    fn process_affinity_limits_the_pool_below_the_machine_count() {
        // A 16-thread machine with the game held to 4 CPUs: 3 workers.
        assert_eq!(worker_count_for_cpus(16, Some(4)), 3);
        assert_eq!(worker_count_for_cpus(16, Some(16)), 15);
        // Unreadable affinity falls back to the machine count.
        assert_eq!(worker_count_for_cpus(16, None), 15);
        // A mask wider than the reported machine (another processor group)
        // never raises the count; one allowed CPU still keeps one worker.
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
        // A single thread, and an unavailable count, still leave one worker.
        assert_eq!(worker_count_for_topology(1), 1);
        assert_eq!(worker_count_for_topology(0), 1);
        // Wider boxes stop at the pool's cap.
        assert_eq!(worker_count_for_topology(32), MAX_WORKERS);
        assert!(Pool::start(MAX_WORKERS + 1, 1, |request: &mut u8| *request).is_err());
    }

    /// One collector wake releases the burst through worker relays. Every
    /// gated job must enter on a distinct worker before any can return.
    #[test]
    fn a_release_wakes_one_worker_per_job_and_the_workers_run_them_all() {
        for workers in 1..=4 {
            for jobs in 1..=workers + 1 {
                let gate = Arc::new(AtomicBool::new(false));
                let ran = Arc::new(Mutex::new(Vec::new()));
                let (entered, started) = mpsc::channel();
                let pool = {
                    let (gate, ran) = (Arc::clone(&gate), Arc::clone(&ran));
                    Pool::start(workers, 8, move |job: &mut usize| {
                        ran.lock().unwrap().push(std::thread::current().id());
                        if *job < workers {
                            entered.send(()).unwrap();
                            wait_until(|| gate.load(Ordering::Acquire));
                        }
                    })
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
                crate::test_support::with_allocations_denied(|| pool.wake_workers(jobs));
                for _ in 0..jobs.min(workers) {
                    started.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
                }
                {
                    let state = pool.shared.lock();
                    assert_eq!(state.counters.notified_one, 1);
                    assert!(state.counters.relayed_one <= (jobs - 1).min(workers - 1) as u64);
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
                pool.shutdown().unwrap();
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
            Pool::start(WORKERS, 8, move |job: &mut usize| {
                if *job < WORKERS {
                    entered.send(()).unwrap();
                    wait_until(|| gate.load(Ordering::Acquire));
                }
                *job
            })
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
            started.recv_timeout(crate::TEST_HANG_GUARD).unwrap();
        }
        let queued: Vec<_> = (WORKERS..2 * WORKERS)
            .map(|job| unsafe {
                pool.submit_with_priority(&mut Some(job), 0, true, false)
                    .unwrap()
            })
            .collect();
        crate::test_support::with_allocations_denied(|| {
            pool.wake_workers(2);
            pool.wake_workers(2);
            assert_eq!(pool.shared.burst_peers.load(Ordering::Acquire), 2);
            let state = pool.shared.lock();
            assert_eq!(state.counters.notified_one, 2);
            assert_eq!(state.counters.relayed_one, 0);
        });
        release.open();
        for (job, ticket) in running.into_iter().chain(queued).enumerate() {
            assert_eq!(pool.finish(ticket).unwrap().report, Ok(job));
        }
        assert_eq!(pool.shared.burst_peers.load(Ordering::Acquire), 0);
        assert_eq!(pool.calibration().caller_executed, 0);
        assert_eq!(pool.calibration().uncollected, 0);
    }

    /// Times job-sized submission and the burst wakes against a pool of
    /// `MAX_WORKERS` workers running jobs, with wake boost and burst wake-all
    /// independently enabled, without and with the CPUs the pool leaves busy.
    #[test]
    #[ignore = "release micro-bench: cargo test -p collector --release --lib -- --ignored --nocapture --test-threads=1 dispatch_and_release_against_fifteen_workers"]
    fn dispatch_and_release_against_fifteen_workers() {
        // Job-sized requests and completions: a stack the size of a worker's
        // several times over.
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(dispatch_and_release_bench)
            .unwrap()
            .join()
            .unwrap();
    }

    fn dispatch_and_release_bench() {
        use crate::acquisition_jobs::{Job, Report};
        use std::time::{Duration, Instant};
        const JOB: usize = size_of::<Job>();
        struct Request {
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
            let mean =
                samples.iter().map(|&ns| u128::from(ns)).sum::<u128>() / samples.len() as u128;
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
            size_of::<Cell<Job, Report>>() + size_of::<Slot>() + size_of::<AtomicU64>(),
            size_of::<Completion<Job, Report>>()
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
            for (wake_boost, wake_all) in
                [(true, true), (false, true), (true, false), (false, false)]
            {
                let pool =
                    Pool::start_boosted(MAX_WORKERS, 111, wake_boost, |request: &mut Request| {
                        spin(request.micros);
                    })
                    .unwrap();
                let wake = |jobs: usize| {
                    if wake_all {
                        pool.shared.work_ready.notify_all();
                    } else {
                        pool.wake_workers(jobs);
                    }
                };
                // Submission wall per job, and wall per release, by kind.
                const ROUNDS: usize = 200;
                let mut held = Vec::with_capacity(52 * ROUNDS);
                let mut waking = Vec::with_capacity(20 * ROUNDS);
                let mut releases: [Vec<u64>; 3] =
                    std::array::from_fn(|_| Vec::with_capacity(ROUNDS));
                for _ in 0..ROUNDS {
                    let mut tickets = Vec::new();
                    let mut submit = |micros: u64, wake: bool, samples: &mut Vec<u64>| {
                        let mut request = Some(Request {
                            micros,
                            _bytes: [0; JOB],
                        });
                        let begin = Instant::now();
                        let ticket =
                            unsafe { pool.submit_with_priority(&mut request, 0, false, wake) }
                                .unwrap();
                        samples.push(qpc_ns(begin));
                        tickets.push(ticket);
                    };
                    // The first burst, held and released at once.
                    for _ in 0..45 {
                        submit(300, false, &mut held);
                    }
                    let begin = Instant::now();
                    wake(45);
                    releases[0].push(qpc_ns(begin));
                    // A small burst while the pool is busy.
                    for _ in 0..6 {
                        submit(200, false, &mut held);
                    }
                    let begin = Instant::now();
                    wake(6);
                    releases[1].push(qpc_ns(begin));
                    // Waking submissions, one worker each.
                    for _ in 0..20 {
                        submit(100, true, &mut waking);
                    }
                    // A one-job burst.
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
                pool.shutdown().unwrap();
            }
            stop.store(true, Ordering::Relaxed);
            for spinner in spinners {
                spinner.join().unwrap();
            }
        }
    }
}
