//! A persistent, flat pool for hour-bounded native acquisition.
//!
//! Jobs own their outputs and shared source leases. Dispatch cannot prove the
//! foreign world's lifetime: the sole hourly callback (the game thread inside
//! the hook) must collect every accepted ticket before the trampoline. While
//! collecting, that callback can execute its awaited job if queued and admitted
//! for caller execution. Worker-only jobs retain the same completion barrier.
//! Runners must not submit, collect, stop, or drop this pool. Dependency barriers
//! belong to the callback; a worker never waits for a job.

use std::{
    collections::VecDeque,
    mem::{size_of, size_of_val},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    thread::JoinHandle,
};

use serde::Serialize;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;

const MAX_WORKERS: usize = 15;
static NEXT_POOL_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// The owned request may contain partial observations. No report is valid,
    /// and this request is never retried.
    Panicked,
    /// The accepted request was canceled before its runner began.
    Stopped,
}

pub(crate) struct Completion<J, R> {
    pub request: J,
    pub report: Result<R, Failure>,
    pub table_reads: crate::read_accounting::TableCounts,
    #[cfg(feature = "callback_profile")]
    pub profile: Profile,
}

/// Diagnostic executor stages in raw QPC ticks. Zero timestamps mean that the
/// stage was not reached, such as an unclaimed request canceled during shutdown.
/// Publication, notification endpoints and collection are stamped under the
/// existing queue mutex. Wake-end to collection also includes mutex acquisition.
#[cfg(feature = "callback_profile")]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Profile {
    pub queued_qpc: i64,
    pub claimed_qpc: i64,
    pub runner_begin_qpc: i64,
    pub runner_end_qpc: i64,
    pub runner_thread_id: u32,
    /// The QPC runner interval also includes both diagnostic cycle queries.
    pub runner_thread_cycles: Option<u64>,
    pub submit_lock_wait_ticks: u64,
    pub completion_lock_wait_ticks: u64,
    /// Only the initial finish lock acquisition, excluding condition-variable waits.
    pub finish_lock_wait_ticks: u64,
    pub published_qpc: i64,
    pub wake_begin_qpc: i64,
    pub wake_end_qpc: i64,
    pub collected_qpc: i64,
}

#[cfg(feature = "callback_profile")]
fn profile_thread_cycles() -> Option<u64> {
    unsafe extern "system" {
        fn QueryThreadCycleTime(
            thread: windows_sys::Win32::Foundation::HANDLE,
            cycles: *mut u64,
        ) -> i32;
    }
    let mut cycles = 0;
    let ok = unsafe {
        QueryThreadCycleTime(
            windows_sys::Win32::System::Threading::GetCurrentThread(),
            &mut cycles,
        )
    };
    (ok != 0).then_some(cycles)
}

/// Pool and admission identity prevent stale tickets from consuming another
/// request's output. Collection is single-use.
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

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct Calibration {
    pub thread_ids: Vec<u32>,
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
}

#[derive(Default)]
struct Counters {
    accepted: u64,
    completed: u64,
    panicked: u64,
    stopped: u64,
    rejected: u64,
    caller_executed: u64,
    worker_executed: u64,
}

// Slots are preallocated. Keep completion counters inline to avoid a per-job
// allocation; their storage is included in the pool's measured slot size.
#[allow(clippy::large_enum_variant)]
enum Payload<J, R> {
    Vacant,
    Queued(J),
    Running,
    Complete(Completion<J, R>),
}

struct Slot<J, R> {
    generation: u64,
    /// Claim order among queued jobs; lower first, equal keeps arrival order.
    priority: u8,
    worker_only: bool,
    payload: Payload<J, R>,
    #[cfg(feature = "callback_profile")]
    profile: Profile,
}

struct State<J, R> {
    slots: Vec<Slot<J, R>>,
    free: Vec<usize>,
    ready: VecDeque<usize>,
    generation: u64,
    started: usize,
    active: usize,
    stop: bool,
    reserved_workers: usize,
    counters: Counters,
}

impl<J, R> State<J, R> {
    fn new(capacity: usize) -> Result<Self, String> {
        let mut slots = Vec::new();
        let mut free = Vec::new();
        let mut ready = VecDeque::new();
        slots
            .try_reserve_exact(capacity)
            .map_err(|e| e.to_string())?;
        free.try_reserve_exact(capacity)
            .map_err(|e| e.to_string())?;
        ready
            .try_reserve_exact(capacity)
            .map_err(|e| e.to_string())?;
        for index in 0..capacity {
            slots.push(Slot {
                generation: 0,
                priority: 0,
                worker_only: false,
                payload: Payload::Vacant,
                #[cfg(feature = "callback_profile")]
                profile: Profile::default(),
            });
            free.push(capacity - index - 1);
        }
        Ok(Self {
            slots,
            free,
            ready,
            generation: 0,
            started: 0,
            active: 0,
            stop: false,
            reserved_workers: 0,
            counters: Counters::default(),
        })
    }

    fn take_ready(&mut self, awaited: Option<Ticket>) -> Option<(usize, J)> {
        let index = if let Some(ticket) = awaited {
            let slot = self.slots.get(ticket.slot)?;
            if slot.generation != ticket.generation
                || slot.worker_only
                || !matches!(slot.payload, Payload::Queued(_))
            {
                return None;
            }
            // finish validates the pool identity before taking this lock.
            // Removing only this entry preserves every other worker's FIFO order.
            let position = self
                .ready
                .iter()
                .position(|&index| index == ticket.slot)
                .expect("queued acquisition slot has a ready entry");
            self.ready
                .remove(position)
                .expect("checked acquisition ready position")
        } else {
            let position = self
                .ready
                .iter()
                .enumerate()
                .min_by_key(|(_, index)| self.slots[**index].priority)
                .map(|(position, _)| position)?;
            self.ready
                .remove(position)
                .expect("checked acquisition ready position")
        };
        #[cfg(feature = "callback_profile")]
        {
            self.slots[index].profile.claimed_qpc = crate::sampling::qpc();
        }
        let Payload::Queued(request) =
            std::mem::replace(&mut self.slots[index].payload, Payload::Running)
        else {
            unreachable!("ready acquisition slot contains an owned queued job")
        };
        self.active += 1;
        Some((index, request))
    }

    fn stop(&mut self) {
        self.stop = true;
        while let Some(index) = self.ready.pop_front() {
            let Payload::Queued(request) =
                std::mem::replace(&mut self.slots[index].payload, Payload::Vacant)
            else {
                unreachable!("canceled acquisition slot contains an owned queued job")
            };
            self.slots[index].payload = Payload::Complete(Completion {
                request,
                report: Err(Failure::Stopped),
                table_reads: crate::read_accounting::ZERO,
                #[cfg(feature = "callback_profile")]
                profile: self.slots[index].profile,
            });
            self.counters.stopped = self.counters.stopped.saturating_add(1);
            self.counters.completed = self.counters.completed.saturating_add(1);
            #[cfg(feature = "callback_profile")]
            if let Payload::Complete(completion) = &mut self.slots[index].payload {
                completion.profile.published_qpc = crate::sampling::qpc();
            }
        }
    }

    fn allocated_bytes(&self) -> usize {
        self.slots.capacity() * size_of::<Slot<J, R>>()
            + self.free.capacity() * size_of::<usize>()
            + self.ready.capacity() * size_of::<usize>()
    }
}

type Runner<J, R> = dyn Fn(&mut J) -> R + Send + Sync;

struct Shared<J, R> {
    id: u64,
    state: Mutex<State<J, R>>,
    changed: Condvar,
    work_ready: Condvar,
    run: Box<Runner<J, R>>,
    thread_ids: Vec<AtomicU32>,
    join_failed: AtomicBool,
}

impl<J, R> Shared<J, R> {
    fn lock(&self) -> MutexGuard<'_, State<J, R>> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn wait<'a>(&self, state: MutexGuard<'a, State<J, R>>) -> MutexGuard<'a, State<J, R>> {
        self.changed
            .wait(state)
            .unwrap_or_else(|error| error.into_inner())
    }

    fn wait_for_work<'a>(&self, state: MutexGuard<'a, State<J, R>>) -> MutexGuard<'a, State<J, R>> {
        self.work_ready
            .wait(state)
            .unwrap_or_else(|error| error.into_inner())
    }

    fn stop(&self) {
        self.lock().stop();
        self.changed.notify_all();
        self.work_ready.notify_all();
    }

    fn execute(&self, index: usize, mut request: J, caller: bool) {
        // Ownership stays outside the unwind guard. Locally constructed live
        // readers have dropped before either outcome is made collectable.
        let reads = crate::read_accounting::Capture::new();
        #[cfg(feature = "callback_profile")]
        let runner_thread_id = unsafe { GetCurrentThreadId() };
        #[cfg(feature = "callback_profile")]
        let runner_begin_qpc = crate::sampling::qpc();
        #[cfg(feature = "callback_profile")]
        let runner_cycles_begin = profile_thread_cycles();
        let report = match catch_unwind(AssertUnwindSafe(|| (self.run)(&mut request))) {
            Ok(report) => Ok(report),
            Err(payload) => {
                // Arbitrary panic payload Drop can itself panic. A stopped
                // pool bounds these leaks by the number of active runners.
                std::mem::forget(payload);
                Err(Failure::Panicked)
            }
        };
        #[cfg(feature = "callback_profile")]
        let runner_cycles_end = profile_thread_cycles();
        #[cfg(feature = "callback_profile")]
        let runner_end_qpc = crate::sampling::qpc();
        let table_reads = reads.finish();
        #[cfg(feature = "callback_profile")]
        let completion_lock_begin = crate::sampling::qpc();
        let mut state = self.lock();
        #[cfg(feature = "callback_profile")]
        let completion_lock_wait_ticks = crate::sampling::qpc()
            .saturating_sub(completion_lock_begin)
            .max(0) as u64;
        let stopped = report.is_err();
        if stopped {
            state.counters.panicked = state.counters.panicked.saturating_add(1);
            state.stop();
        }
        state.slots[index].payload = Payload::Complete(Completion {
            request,
            report,
            table_reads,
            #[cfg(feature = "callback_profile")]
            profile: Profile {
                runner_begin_qpc,
                runner_end_qpc,
                runner_thread_id,
                runner_thread_cycles: runner_cycles_begin
                    .zip(runner_cycles_end)
                    .and_then(|(begin, end)| end.checked_sub(begin)),
                completion_lock_wait_ticks,
                ..state.slots[index].profile
            },
        });
        state.active -= 1;
        state.counters.completed = state.counters.completed.saturating_add(1);
        if caller {
            state.counters.caller_executed = state.counters.caller_executed.saturating_add(1);
        } else {
            state.counters.worker_executed = state.counters.worker_executed.saturating_add(1);
        }
        #[cfg(feature = "callback_profile")]
        if let Payload::Complete(completion) = &mut state.slots[index].payload {
            completion.profile.published_qpc = crate::sampling::qpc();
            completion.profile.wake_begin_qpc = crate::sampling::qpc();
        }
        // Publish the wake before unlocking: releasing a contended mutex may
        // schedule another thread before this worker can notify the collector.
        // Waiters still recheck their ticket and payload under this same mutex.
        self.changed.notify_all();
        #[cfg(feature = "callback_profile")]
        if let Payload::Complete(completion) = &mut state.slots[index].payload {
            completion.profile.wake_end_qpc = crate::sampling::qpc();
        }
        drop(state);
        if stopped {
            self.work_ready.notify_all();
        }
    }
}

pub(crate) struct Pool<J: Send + 'static, R: Copy + Send + 'static> {
    shared: Arc<Shared<J, R>>,
    // Joining never holds the state lock. Concurrent shutdown calls serialize
    // on this separate lock and retain the first join failure.
    threads: Mutex<Vec<JoinHandle<()>>>,
    allocated_bytes: usize,
}

impl<J: Send + 'static, R: Copy + Send + 'static> Pool<J, R> {
    /// Allocates bounded queues and starts workers before hook install. Zero
    /// workers is valid: the callback then executes all accepted jobs.
    pub(crate) fn start(
        worker_count: usize,
        capacity: usize,
        run: impl Fn(&mut J) -> R + Send + Sync + 'static,
    ) -> Result<Self, String> {
        if worker_count > MAX_WORKERS || capacity == 0 {
            return Err(format!(
                "acquisition pool requires 0..={MAX_WORKERS} workers and nonzero capacity"
            ));
        }
        let id = NEXT_POOL_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| "acquisition pool identity exhausted")?;
        let state = State::new(capacity)?;
        let mut threads = Vec::new();
        threads
            .try_reserve_exact(worker_count)
            .map_err(|e| e.to_string())?;
        let thread_ids: Vec<_> = (0..worker_count).map(|_| AtomicU32::new(0)).collect();
        // Jobs' heap payloads are counted by their frame/workspace owners;
        // this counts fixed executor storage, not OS thread stacks.
        let allocated_bytes = size_of::<Shared<J, R>>()
            + state.allocated_bytes()
            + thread_ids.capacity() * size_of::<AtomicU32>()
            + threads.capacity() * size_of::<JoinHandle<()>>()
            + size_of_val(&run);
        let shared = Arc::new(Shared {
            id,
            state: Mutex::new(state),
            changed: Condvar::new(),
            work_ready: Condvar::new(),
            run: Box::new(run),
            thread_ids,
            join_failed: AtomicBool::new(false),
        });
        let pool = Self {
            shared,
            threads: Mutex::new(threads),
            allocated_bytes,
        };
        for index in 0..worker_count {
            let shared = Arc::clone(&pool.shared);
            let thread = std::thread::Builder::new()
                .name(format!("native-acquisition-{index}"))
                .spawn(move || worker_loop(shared, index));
            match thread {
                Ok(thread) => pool
                    .threads
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(thread),
                Err(error) => {
                    let _ = pool.shutdown();
                    return Err(format!("start native acquisition worker: {error}"));
                }
            }
        }
        let mut state = pool.shared.lock();
        while state.started != worker_count {
            state = pool.shared.wait(state);
        }
        drop(state);
        Ok(pool)
    }

    /// Rejection leaves the request with the caller, including when an
    /// uncollected completion occupies the last slot.
    ///
    /// # Safety
    /// The callback must keep the foreign live-read lease valid until EVERY
    /// accepted ticket has been collected, even after another job fails.
    /// All jobs must own disjoint mutable outputs; immutable native seeds must
    /// remain inside the hourly lease until every consuming job finishes. The
    /// runner may execute on any worker or on the collecting acquisition
    /// thread, which cannot enter this pool recursively. One collector only.
    #[cfg(test)]
    pub(crate) unsafe fn submit(&self, request: &mut Option<J>) -> Result<Ticket, SubmitError> {
        // SAFETY: the caller's contract is unchanged.
        unsafe { self.submit_with_priority(request, 0, false) }
    }

    /// Reserve existing slots for work that cannot run on the callback.
    /// Called before admission begins; its worker is started before hook install.
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

    /// `submit` with a claim priority: queued jobs with a lower number run
    /// first once a worker frees.
    ///
    /// # Safety
    /// As `submit`.
    pub(crate) unsafe fn submit_with_priority(
        &self,
        request: &mut Option<J>,
        priority: u8,
        worker_only: bool,
    ) -> Result<Ticket, SubmitError> {
        #[cfg(feature = "callback_profile")]
        let submit_lock_begin = crate::sampling::qpc();
        let mut state = self.shared.lock();
        #[cfg(feature = "callback_profile")]
        let submit_lock_wait_ticks = crate::sampling::qpc()
            .saturating_sub(submit_lock_begin)
            .max(0) as u64;
        let free = state
            .free
            .iter()
            .rposition(|index| (*index < state.reserved_workers) == worker_only);
        let rejection = if request.is_none() {
            Some(SubmitError::Empty)
        } else if state.stop {
            Some(SubmitError::Stopped)
        } else if free.is_none() {
            Some(SubmitError::Full)
        } else if state.generation == u64::MAX {
            Some(SubmitError::Exhausted)
        } else {
            None
        };
        if let Some(rejection) = rejection {
            state.counters.rejected = state.counters.rejected.saturating_add(1);
            return Err(rejection);
        }
        let index = state
            .free
            .swap_remove(free.expect("checked free acquisition slot"));
        state.generation += 1;
        let generation = state.generation;
        state.slots[index] = Slot {
            generation,
            priority,
            worker_only,
            payload: Payload::Queued(request.take().expect("checked owned acquisition request")),
            #[cfg(feature = "callback_profile")]
            profile: Profile {
                queued_qpc: crate::sampling::qpc(),
                submit_lock_wait_ticks,
                ..Profile::default()
            },
        };
        state.ready.push_back(index);
        state.counters.accepted = state.counters.accepted.saturating_add(1);
        drop(state);
        self.shared.work_ready.notify_one();
        Ok(Ticket {
            pool: self.shared.id,
            slot: index,
            generation,
        })
    }

    /// Collects once, executing this ticket if queued and caller execution was
    /// admitted. Otherwise wait for its worker, including during unwind drain.
    /// Invalid, foreign, or consumed tickets return None without executing work.
    /// The submit safety contract continues through this method.
    pub(crate) fn finish(&self, ticket: Ticket) -> Option<Completion<J, R>> {
        if ticket.pool != self.shared.id {
            return None;
        }
        #[cfg(feature = "callback_profile")]
        let finish_lock_begin = crate::sampling::qpc();
        let mut state = self.shared.lock();
        #[cfg(feature = "callback_profile")]
        let finish_lock_wait_ticks = crate::sampling::qpc()
            .saturating_sub(finish_lock_begin)
            .max(0) as u64;
        loop {
            let slot = state.slots.get_mut(ticket.slot)?;
            if slot.generation != ticket.generation || matches!(slot.payload, Payload::Vacant) {
                return None;
            }
            if matches!(slot.payload, Payload::Complete(_)) {
                let Payload::Complete(completion) =
                    std::mem::replace(&mut slot.payload, Payload::Vacant)
                else {
                    unreachable!("checked complete acquisition slot")
                };
                state.free.push(ticket.slot);
                #[cfg(feature = "callback_profile")]
                let completion = {
                    let mut completion = completion;
                    completion.profile.finish_lock_wait_ticks = finish_lock_wait_ticks;
                    completion.profile.collected_qpc = crate::sampling::qpc();
                    completion
                };
                return Some(completion);
            }
            // Run the awaited job here only while it is still queued. Once a
            // worker has it, wait: taking another queued job would keep the
            // game thread busy past the awaited completion and delay every
            // later dispatch.
            let queued = !matches!(slot.payload, Payload::Running);
            if queued && let Some((index, request)) = state.take_ready(Some(ticket)) {
                drop(state);
                self.shared.execute(index, request, true);
                state = self.shared.lock();
            } else {
                state = self.shared.wait(state);
            }
        }
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
        let thread_ids = self.thread_ids();
        let state = self.shared.lock();
        Calibration {
            thread_ids,
            capacity: state.slots.len(),
            queued: state.ready.len(),
            active: state.active,
            uncollected: state.slots.len() - state.free.len(),
            accepted: state.counters.accepted,
            completed: state.counters.completed,
            panicked: state.counters.panicked,
            stopped: state.counters.stopped,
            rejected: state.counters.rejected,
            caller_executed: state.counters.caller_executed,
            worker_executed: state.counters.worker_executed,
        }
    }

    /// Runtime calls this after the hook is quiescent. Stops
    /// admission and cancels queued jobs before joining workers, then also
    /// waits for any caller-executed job. Completed payloads remain collectable.
    /// Idempotent; a join failure remains an error on subsequent calls.
    pub(crate) fn shutdown(&self) -> Result<(), String> {
        self.shared.stop();
        let mut threads = self
            .threads
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for thread in threads.drain(..) {
            if let Err(payload) = thread.join() {
                std::mem::forget(payload);
                self.shared.join_failed.store(true, Ordering::Release);
            }
        }
        drop(threads);
        // An unexpected executor panic may leave a Running marker. Runtime
        // must refuse unload in that case, rather than waiting forever here.
        if self.shared.join_failed.load(Ordering::Acquire) {
            return Err(
                "native acquisition worker terminated outside its acquisition guard".into(),
            );
        }
        let mut state = self.shared.lock();
        while state.active != 0 {
            state = self.shared.wait(state);
        }
        Ok(())
    }
}

impl<J: Send + 'static, R: Copy + Send + 'static> Drop for Pool<J, R> {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn worker_loop<J: Send + 'static, R: Copy + Send + 'static>(
    shared: Arc<Shared<J, R>>,
    index: usize,
) {
    shared.thread_ids[index].store(unsafe { GetCurrentThreadId() }, Ordering::Release);
    let mut state = shared.lock();
    state.started += 1;
    shared.changed.notify_all();
    loop {
        if state.stop {
            return;
        }
        if let Some((index, request)) = state.take_ready(None) {
            drop(state);
            shared.execute(index, request, false);
            state = shared.lock();
        } else {
            state = shared.wait_for_work(state);
        }
    }
}

/// One acquisition worker per hardware thread beyond the collecting callback.
/// These reads are memory-latency bound, so SMT siblings add real throughput,
/// and the game's own threads are parked for the whole block. Query once before
/// hook install; this neither pins threads nor changes game affinity.
pub(crate) fn recommended_worker_count() -> usize {
    worker_count_for_topology(std::thread::available_parallelism().map_or(1, usize::from))
}

/// Hardware threads minus the callback, within the pool's admitted range.
fn worker_count_for_topology(available: usize) -> usize {
    available.saturating_sub(1).clamp(1, MAX_WORKERS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Barrier, mpsc};

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
                profile.wake_begin_qpc,
                profile.wake_end_qpc,
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
        assert_eq!(profile.wake_begin_qpc, 0);
        assert_eq!(profile.wake_end_qpc, 0);
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
        let ticket = unsafe { pool.submit_with_priority(&mut request, 0, true) }.unwrap();
        assert!(request.is_none());
        let second = unsafe { pool.submit_with_priority(&mut Some(4), 0, true) }.unwrap();
        assert!(pool.shared.lock().take_ready(Some(ticket)).is_none());
        assert!(pool.shared.lock().take_ready(Some(second)).is_none());
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
        first_pool.shared.lock().generation = u64::MAX;
        let mut rejected = Some(4);
        assert_eq!(
            unsafe { first_pool.submit(&mut rejected) },
            Err(SubmitError::Exhausted)
        );
        assert_eq!(rejected, Some(4));
    }

    #[test]
    fn panic_returns_partial_request_cancels_queued_jobs_and_does_not_retry() {
        use engine::{
            Memory,
            memory::{CountedMemory, ReadCounts, SliceMemory},
        };
        use replay::rows::TableId;

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
        let mut state = pool.shared.lock();
        while state.counters.panicked == 0 {
            state = pool.shared.wait(state);
        }
        drop(state);
        let failed = pool.finish(second).unwrap();
        let canceled = pool.finish(queued).unwrap();
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

        // Completion/panic state remains on changed. The same panic-induced
        // stop must also wake the other idle worker from work_ready so both
        // executor threads can exit without a second stop notification.
        let mut state = pool.shared.lock();
        while state.counters.panicked == 0 {
            state = pool.shared.wait(state);
        }
        drop(state);
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
}
