//! Callback-bounded native acquisition with three precreated, owned-job workers.
//!
//! Dispatch is unsafe because ordinary thread ownership cannot establish the
//! foreign world's lifetime or consistency boundary. The sole hourly callback
//! must finish every accepted request before its live-read lease ends. Workers
//! receive neither the enclosing captured frame nor Shared, and construct their
//! own !Send live reader. ETA owns only its isolated table buffers and sources.

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    thread::JoinHandle,
};

use engine::memory::LiveMemory;
use serde::Serialize;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;

use crate::extended::{
    characters::{self, CharactersCopyContext, CharactersCopyReport, CharactersJob},
    division_eta,
    intel::{self, RelationsCopyContext, RelationsCopyReport, RelationsJob},
};

pub(crate) struct CharactersRequest {
    pub context: CharactersCopyContext,
    pub first: CharactersJob,
    pub second: Option<CharactersJob>,
}

pub(crate) struct RelationsRequest {
    pub context: RelationsCopyContext,
    pub first: RelationsJob,
    pub second: Option<RelationsJob>,
}

pub(crate) struct EtaRequest {
    pub base: usize,
    pub state: usize,
    pub checked: bool,
    pub captured: division_eta::Captured,
    pub sources: division_eta::OwnedSources,
}
pub(crate) type EtaCompletion = Completion<EtaRequest, division_eta::AcquisitionReport>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// Jobs are recovered but may contain a partial observation stream. No
    /// report or consistency credit is valid; the executor does not retry.
    Panicked,
    /// Shutdown canceled an accepted request before its first live read.
    Stopped,
}

pub(crate) struct Completion<T, R> {
    pub request: T,
    pub report: Result<R, Failure>,
}

pub(crate) type CharactersCompletion = Completion<CharactersRequest, CharactersCopyReport>;
pub(crate) type RelationsCompletion = Completion<RelationsRequest, RelationsCopyReport>;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(crate) struct WorkerCalibration {
    pub accepted: u64,
    pub completed: u64,
    pub panicked: u64,
    pub stopped: u64,
    pub rejected: u64,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct Calibration {
    pub thread_ids: [u32; 3],
    pub characters: WorkerCalibration,
    pub relations: WorkerCalibration,
    pub eta: WorkerCalibration,
    pub eta_country_cache: EtaCountryCacheCalibration,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum EtaCountryCacheSeed {
    Ready { entries: usize },
    Capacity { required_cells: usize },
    Unavailable { reason: &'static str },
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(crate) struct EtaCountryCacheCalibration {
    pub ready: u64,
    pub capacity_fallbacks: u64,
    pub unavailable_fallbacks: u64,
    pub latest: Option<EtaCountryCacheSeed>,
}
impl EtaCountryCacheCalibration {
    fn observe(&mut self, seed: engine::country_memory::SeedStatus, entries: usize) {
        self.latest = Some(match seed {
            engine::country_memory::SeedStatus::Ready => {
                self.ready = self.ready.saturating_add(1);
                EtaCountryCacheSeed::Ready { entries }
            }
            engine::country_memory::SeedStatus::Capacity { required_cells } => {
                self.capacity_fallbacks = self.capacity_fallbacks.saturating_add(1);
                EtaCountryCacheSeed::Capacity { required_cells }
            }
            engine::country_memory::SeedStatus::Unavailable { reason } => {
                self.unavailable_fallbacks = self.unavailable_fallbacks.saturating_add(1);
                EtaCountryCacheSeed::Unavailable { reason }
            }
        });
    }
}

#[derive(Default)]
struct Counters {
    accepted: AtomicU64,
    completed: AtomicU64,
    panicked: AtomicU64,
    stopped: AtomicU64,
    rejected: AtomicU64,
}

impl Counters {
    fn snapshot(&self) -> WorkerCalibration {
        WorkerCalibration {
            accepted: self.accepted.load(Ordering::Relaxed),
            completed: self.completed.load(Ordering::Relaxed),
            panicked: self.panicked.load(Ordering::Relaxed),
            stopped: self.stopped.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
        }
    }
}

struct State<T, R> {
    pending: Option<T>,
    completed: Option<Completion<T, R>>,
    busy: bool,
    stop: bool,
}

struct Slot<T, R> {
    state: Mutex<State<T, R>>,
    changed: Condvar,
    complete: AtomicBool,
    thread_id: AtomicU32,
    counters: Counters,
}

impl<T, R> Slot<T, R> {
    fn new() -> Self {
        Self {
            state: Mutex::new(State {
                pending: None,
                completed: None,
                busy: false,
                stop: false,
            }),
            changed: Condvar::new(),
            complete: AtomicBool::new(false),
            thread_id: AtomicU32::new(0),
            counters: Counters::default(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State<T, R>> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn wait<'a>(&self, state: MutexGuard<'a, State<T, R>>) -> MutexGuard<'a, State<T, R>> {
        self.changed
            .wait(state)
            .unwrap_or_else(|error| error.into_inner())
    }
}

struct Worker<T: Send + 'static, R: Copy + Send + 'static> {
    slot: Arc<Slot<T, R>>,
    thread: Mutex<Option<JoinHandle<()>>>,
    join_failed: AtomicBool,
}

impl<T: Send + 'static, R: Copy + Send + 'static> Worker<T, R> {
    fn start(name: &str, run: impl FnMut(&mut T) -> R + Send + 'static) -> Result<Self, String> {
        let slot = Arc::new(Slot::new());
        let worker_slot = Arc::clone(&slot);
        let handle = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || worker_loop(worker_slot, run))
            .map_err(|error| format!("cannot start {name}: {error}"))?;
        let worker = Self {
            slot,
            thread: Mutex::new(Some(handle)),
            join_failed: AtomicBool::new(false),
        };
        // Registration completes before runtime can install the hook. The
        // producer changes this predicate under the same mutex as the wait.
        let mut state = worker.slot.lock();
        while worker.slot.thread_id.load(Ordering::Acquire) == 0 {
            state = worker.slot.wait(state);
        }
        drop(state);
        Ok(worker)
    }

    fn begin(&self, request: &mut Option<T>) -> bool {
        let mut state = self.slot.lock();
        if state.stop || state.busy || request.is_none() {
            self.slot.counters.rejected.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        self.slot.complete.store(false, Ordering::Relaxed);
        state.pending = request.take();
        state.busy = true;
        self.slot.counters.accepted.fetch_add(1, Ordering::Relaxed);
        self.slot.changed.notify_all();
        true
    }

    fn finish(&self) -> Option<Completion<T, R>> {
        let mut state = self.slot.lock();
        if !state.busy {
            return None;
        }
        while !self.slot.complete.load(Ordering::Acquire) {
            state = self.slot.wait(state);
        }
        let result = state.completed.take();
        state.busy = false;
        self.slot.complete.store(false, Ordering::Relaxed);
        result
    }

    fn stop(&self) {
        let mut state = self.slot.lock();
        state.stop = true;
        self.slot.changed.notify_all();
    }

    fn join(&self) -> bool {
        // Serialize joins, including repeated shutdown calls. No worker ever
        // accesses this mutex, and the slot mutex is not held while joining.
        let mut thread = self
            .thread
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(handle) = thread.take()
            && let Err(payload) = handle.join()
        {
            // The thread is joined, but even its panic payload's Drop is
            // arbitrary code. Report the failure without another unwind.
            std::mem::forget(payload);
            self.join_failed.store(true, Ordering::Release);
        }
        !self.join_failed.load(Ordering::Acquire)
    }
}

impl<T: Send + 'static, R: Copy + Send + 'static> Drop for Worker<T, R> {
    fn drop(&mut self) {
        self.stop();
        self.join();
    }
}

fn worker_loop<T: Send + 'static, R: Copy + Send + 'static>(
    slot: Arc<Slot<T, R>>,
    mut run: impl FnMut(&mut T) -> R,
) {
    {
        let _state = slot.lock();
        slot.thread_id
            .store(unsafe { GetCurrentThreadId() }, Ordering::Release);
        slot.changed.notify_all();
    }
    loop {
        let (mut request, canceled) = {
            let mut state = slot.lock();
            while state.pending.is_none() && !state.stop {
                state = slot.wait(state);
            }
            let Some(request) = state.pending.take() else {
                return;
            };
            (request, state.stop)
        };
        let report = if canceled {
            slot.counters.stopped.fetch_add(1, Ordering::Relaxed);
            Err(Failure::Stopped)
        } else {
            // The request stays outside the unwind closure. Even a partial
            // acquisition returns its owned buffers; no read is retried here.
            match catch_unwind(AssertUnwindSafe(|| run(&mut request))) {
                Ok(report) => Ok(report),
                Err(payload) => {
                    // A panic payload may have an arbitrary panicking Drop.
                    // This worker permanently stops after this one failure;
                    // intentionally retain at most one payload instead of
                    // risking a second unwind before job ownership is returned.
                    std::mem::forget(payload);
                    slot.counters.panicked.fetch_add(1, Ordering::Relaxed);
                    Err(Failure::Panicked)
                }
            }
        };
        let failed = report.is_err();
        let mut state = slot.lock();
        state.completed = Some(Completion { request, report });
        state.stop |= failed;
        slot.counters.completed.fetch_add(1, Ordering::Relaxed);
        // `run` has returned or fully unwound: its local LiveMemory is gone,
        // no native call is outstanding, and the output buffers are complete.
        slot.complete.store(true, Ordering::Release);
        slot.changed.notify_all();
        if state.stop {
            return;
        }
    }
}

fn run_characters(request: &mut CharactersRequest) -> CharactersCopyReport {
    // SAFETY: accepted dispatch retains the caller's foreign-memory lease
    // until finish. This reader never crosses threads or escapes this call.
    let memory = unsafe { LiveMemory::new() };
    characters::copy_characters_refresh(
        &memory,
        request.context,
        &mut request.first,
        request.second.as_mut(),
    )
}

fn run_relations(request: &mut RelationsRequest) -> RelationsCopyReport {
    // SAFETY: same callback-bounded contract as run_characters.
    let memory = unsafe { LiveMemory::new() };
    intel::copy_relations_refresh(
        &memory,
        request.context,
        &mut request.first,
        request.second.as_mut(),
    )
}

fn run_eta_with_memory(
    request: &mut EtaRequest,
    memory: &impl engine::Memory,
    country_cache: &mut engine::country_memory::Cache,
    seed_calibration: &Mutex<EtaCountryCacheCalibration>,
) -> division_eta::AcquisitionReport {
    let started = crate::sampling::qpc();
    let cached = country_cache.prepare(memory, request.base, request.state);
    seed_calibration
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .observe(cached.seed_status(), cached.cached_entries());
    let (matched, _) = request.captured.capture(
        &cached,
        request.base,
        request.state,
        &[request.checked],
        request.sources.view(),
    );
    let mut matched = matched[0];
    if let Err(reason) = cached.verify() {
        request.captured.fail(reason, request.checked);
        matched = false;
    }
    request.sources.clear();
    division_eta::AcquisitionReport {
        matched,
        ticks: crate::sampling::qpc().saturating_sub(started).max(0) as u64,
    }
}

pub(crate) struct Pool {
    characters: Worker<CharactersRequest, CharactersCopyReport>,
    relations: Worker<RelationsRequest, RelationsCopyReport>,
    eta: Worker<EtaRequest, division_eta::AcquisitionReport>,
    eta_country_cache_bytes: usize,
    eta_country_cache: Arc<Mutex<EtaCountryCacheCalibration>>,
}

impl Pool {
    /// Startup only: allocates fixed slots and registered threads. Partial
    /// startup failure drops and joins every worker already created.
    pub(crate) fn start() -> Result<Self, String> {
        let mut eta_cache = engine::country_memory::Cache::with_capacity(8192);
        let eta_country_cache_bytes = eta_cache.allocated_bytes();
        let eta_country_cache = Arc::new(Mutex::new(EtaCountryCacheCalibration::default()));
        let eta_worker_calibration = Arc::clone(&eta_country_cache);
        Ok(Self {
            characters: Worker::start("hoi4-acquire-characters", run_characters)?,
            relations: Worker::start("hoi4-acquire-relations", run_relations)?,
            eta: Worker::start("hoi4-acquire-eta", move |request| {
                // SAFETY: dispatch retains the hourly lease until finish_eta;
                // neither this reader nor the worker-local country cache escapes.
                let memory = unsafe { LiveMemory::new() };
                run_eta_with_memory(request, &memory, &mut eta_cache, &eta_worker_calibration)
            })?,
            eta_country_cache_bytes,
            eta_country_cache,
        })
    }

    pub(crate) fn thread_ids(&self) -> [u32; 3] {
        [
            self.characters.slot.thread_id.load(Ordering::Acquire),
            self.relations.slot.thread_id.load(Ordering::Acquire),
            self.eta.slot.thread_id.load(Ordering::Acquire),
        ]
    }

    /// Success consumes the request without allocating. Failure leaves it
    /// intact for the caller. Only the sole callback dispatches and finishes.
    ///
    /// # Safety
    /// The current hourly callback must keep the admitted foreign world valid
    /// and stable for this whole acquisition. Every accepted request MUST be
    /// finished before final verification, publication, callback exit or any
    /// action permitting the next game tick. Do not hold a lock needed by the
    /// worker while finishing. No timeout may abandon an outstanding reader.
    pub(crate) unsafe fn begin_characters(&self, request: &mut Option<CharactersRequest>) -> bool {
        self.characters.begin(request)
    }

    /// # Safety
    /// The same callback/lifetime/barrier contract as begin_characters applies.
    pub(crate) unsafe fn begin_relations(&self, request: &mut Option<RelationsRequest>) -> bool {
        self.relations.begin(request)
    }

    pub(crate) fn finish_characters(&self) -> Option<CharactersCompletion> {
        self.characters.finish()
    }

    pub(crate) fn finish_relations(&self) -> Option<RelationsCompletion> {
        self.relations.finish()
    }
    /// # Safety
    /// Same callback/lifetime/barrier contract as begin_characters. Sources and
    /// all row/cache buffers are owned; no Captured frame or Shared is borrowed.
    pub(crate) unsafe fn begin_eta(&self, request: &mut Option<EtaRequest>) -> bool {
        self.eta.begin(request)
    }
    pub(crate) fn finish_eta(&self) -> Option<EtaCompletion> {
        self.eta.finish()
    }
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.eta_country_cache_bytes
    }

    pub(crate) fn calibration(&self) -> Calibration {
        Calibration {
            thread_ids: self.thread_ids(),
            characters: self.characters.slot.counters.snapshot(),
            relations: self.relations.slot.counters.snapshot(),
            eta: self.eta.slot.counters.snapshot(),
            eta_country_cache: *self
                .eta_country_cache
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        }
    }

    /// Runtime only, after dispatch has been excluded. Stops all workers
    /// before joining either; accepted work is completed or returned canceled.
    /// Idempotent. A join failure remains an error on subsequent calls.
    pub(crate) fn shutdown(&self) -> Result<(), String> {
        self.characters.stop();
        self.relations.stop();
        self.eta.stop();
        let characters_ok = self.characters.join();
        let relations_ok = self.relations.join();
        let eta_ok = self.eta.join();
        if characters_ok && relations_ok && eta_ok {
            Ok(())
        } else {
            Err("native acquisition worker terminated outside its acquisition guard".into())
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_eta_request(checked: bool) -> EtaRequest {
        let mut sources = division_eta::OwnedSources::new(0, checked);
        sources.copy_from(
            crate::extended::production::Sources {
                first: &[],
                first_outcome: engine::walk::Outcome::Rows { len: 0 },
                second: &[],
                second_outcome: engine::walk::Outcome::Rows { len: 0 },
            },
            checked,
        );
        EtaRequest {
            base: 0,
            state: 0x1000,
            checked,
            captured: division_eta::Captured::empty(),
            sources,
        }
    }
    fn empty_eta_memory() -> Vec<u8> {
        let mut bytes = vec![0; 0x3000];
        for (offset, pointer) in [
            (engine::cgame_state::COUNTRIES, 0x2000_u64),
            (engine::cgame_state::COUNTRY_LINKS, 0x2100),
            (engine::cgame_state::PROVINCES, 0x2200),
        ] {
            let at = 0x1000 + offset;
            bytes[at..at + 8].copy_from_slice(&pointer.to_le_bytes());
            bytes[at + 8..at + 12].copy_from_slice(&2_u32.to_le_bytes());
            bytes[at + 12..at + 16].copy_from_slice(&2_u32.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn eta_owned_worker_returns_verified_both_passes_before_completion_and_can_be_reused() {
        let bytes = empty_eta_memory();
        let mut countries = engine::country_memory::Cache::with_capacity(64);
        let calibration = Arc::new(Mutex::new(EtaCountryCacheCalibration::default()));
        let worker_calibration = Arc::clone(&calibration);
        let worker = Worker::start("eta-owned-test", move |request| {
            let memory = engine::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            run_eta_with_memory(request, &memory, &mut countries, &worker_calibration)
        })
        .unwrap();
        for checked in [false, true] {
            let mut request = Some(empty_eta_request(checked));
            assert!(worker.begin(&mut request));
            assert!(request.is_none());
            let completed = worker.finish().unwrap();
            assert_eq!(completed.report.unwrap().matched, checked);
            assert_eq!(
                completed.request.captured.outcomes(),
                [engine::walk::Outcome::Rows { len: 0 }]
            );
            assert!(matches!(
                completed.request.sources.view().first_outcome,
                engine::walk::Outcome::Unavailable { .. }
            ));
        }
        worker.stop();
        assert!(worker.join());
        assert_eq!(calibration.lock().unwrap().ready, 2);
        let mut rejected = Some(empty_eta_request(true));
        assert!(!worker.begin(&mut rejected));
        let mut request = rejected.unwrap();
        let bytes = empty_eta_memory();
        // Rejection preserves complete ownership for the ordinary synchronous
        // acquisition path; no stopped worker can touch these buffers again.
        let report = run_eta_with_memory(
            &mut request,
            &engine::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &mut engine::country_memory::Cache::with_capacity(64),
            &calibration,
        );
        assert!(report.matched);
        assert_eq!(calibration.lock().unwrap().ready, 3);
    }

    #[test]
    fn eta_worker_country_authentication_verification_failure_invalidates_rows() {
        use std::cell::Cell;
        struct Changed {
            bytes: Vec<u8>,
            root_reads: Cell<usize>,
        }
        impl engine::Memory for Changed {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if !engine::Memory::copy(
                    &engine::memory::SliceMemory {
                        base: 0,
                        bytes: &self.bytes,
                    },
                    address,
                    destination,
                ) {
                    return false;
                }
                if address == 0x1000 + engine::cgame_state::COUNTRIES {
                    self.root_reads.set(self.root_reads.get() + 1);
                    if self.root_reads.get() > 1 {
                        destination[0] ^= 1;
                    }
                }
                true
            }
        }
        let memory = Changed {
            bytes: empty_eta_memory(),
            root_reads: Cell::new(0),
        };
        let mut request = empty_eta_request(true);
        let report = run_eta_with_memory(
            &mut request,
            &memory,
            &mut engine::country_memory::Cache::with_capacity(64),
            &Mutex::new(EtaCountryCacheCalibration::default()),
        );
        assert!(!report.matched);
        assert!(matches!(
            request.captured.outcomes()[0],
            engine::walk::Outcome::Unavailable { .. }
        ));
        assert!(matches!(
            request.sources.view().first_outcome,
            engine::walk::Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn eta_country_cache_calibration_distinguishes_each_seed_and_retains_fallback_counts() {
        use engine::country_memory::SeedStatus;
        let mut calibration = EtaCountryCacheCalibration::default();
        assert!(calibration.latest.is_none());
        calibration.observe(SeedStatus::Ready, 10);
        assert_eq!(
            serde_json::to_value(calibration.latest).unwrap(),
            serde_json::json!({"state": "ready", "entries": 10})
        );
        calibration.observe(
            SeedStatus::Capacity {
                required_cells: 16384,
            },
            0,
        );
        assert_eq!(
            serde_json::to_value(calibration.latest).unwrap(),
            serde_json::json!({"state": "capacity", "required_cells": 16384})
        );
        calibration.observe(
            SeedStatus::Unavailable {
                reason: "synthetic unreadable census",
            },
            0,
        );
        assert_eq!(
            serde_json::to_value(calibration.latest).unwrap(),
            serde_json::json!({"state": "unavailable", "reason": "synthetic unreadable census"})
        );
        calibration.observe(SeedStatus::Ready, 12);
        assert_eq!(calibration.ready, 2);
        assert_eq!(calibration.capacity_fallbacks, 1);
        assert_eq!(calibration.unavailable_fallbacks, 1);
    }

    #[test]
    fn eta_panic_and_pre_run_cancellation_recover_owned_buffers_without_a_success_report() {
        let worker = Worker::start(
            "eta-panic-test",
            |request: &mut EtaRequest| -> division_eta::AcquisitionReport {
                request
                    .captured
                    .fail("partial worker result", request.checked);
                panic!("synthetic ETA panic");
            },
        )
        .unwrap();
        let mut request = Some(empty_eta_request(true));
        assert!(worker.begin(&mut request));
        let completion = worker.finish().unwrap();
        assert_eq!(completion.report, Err(Failure::Panicked));
        assert!(worker.join());
        let slot = Arc::new(Slot::new());
        {
            let mut state = slot.lock();
            state.pending = Some(completion.request);
            state.busy = true;
            state.stop = true;
        }
        worker_loop(
            Arc::clone(&slot),
            |_: &mut EtaRequest| -> division_eta::AcquisitionReport {
                panic!("canceled request must not run")
            },
        );
        let completion = slot.lock().completed.take().unwrap();
        assert_eq!(completion.report, Err(Failure::Stopped));
        assert!(completion.request.checked);
    }

    struct Owned {
        first: Vec<u8>,
        second: Option<Vec<u8>>,
    }

    fn change_owned(request: &mut Owned) -> usize {
        request.first[0] += 1;
        if let Some(second) = request.second.as_mut() {
            second[0] += 2;
        }
        request.first.len()
    }

    fn panic_owned(request: &mut Owned) -> usize {
        request.first[0] = 42;
        panic!("synthetic acquisition failure");
    }

    #[test]
    fn acquisition_owned_slots_return_both_buffers_and_reuse_without_replacement() {
        let worker = Worker::start("owned-acquisition-test", change_owned).unwrap();
        assert_ne!(worker.slot.thread_id.load(Ordering::Acquire), 0);
        let mut request = Some(Owned {
            first: vec![1, 2, 3],
            second: Some(vec![4, 5]),
        });
        let first_pointer = request.as_ref().unwrap().first.as_ptr();
        let second_pointer = request.as_ref().unwrap().second.as_ref().unwrap().as_ptr();
        for iteration in 0..3 {
            assert!(worker.begin(&mut request));
            assert!(request.is_none());
            let done = worker.finish().unwrap();
            assert_eq!(done.report, Ok(3));
            assert_eq!(done.request.first, [2 + iteration, 2, 3]);
            assert_eq!(done.request.first.as_ptr(), first_pointer);
            let second = done.request.second.as_ref().unwrap();
            assert_eq!(second.as_slice(), [6 + 2 * iteration, 5]);
            assert_eq!(second.as_ptr(), second_pointer);
            request = Some(done.request);
        }
        assert!(worker.finish().is_none());
        assert_eq!(worker.slot.counters.snapshot().accepted, 3);
        assert_eq!(worker.slot.counters.snapshot().completed, 3);
    }

    #[test]
    fn acquisition_busy_and_stopped_slots_leave_the_request_owned_by_caller() {
        let worker = Worker::start("owned-acquisition-test", change_owned).unwrap();
        let mut first = Some(Owned {
            first: vec![1],
            second: None,
        });
        let mut rejected = Some(Owned {
            first: vec![9],
            second: None,
        });
        assert!(worker.begin(&mut first));
        // Even an already-complete result retains exclusive slot ownership
        // until finish consumes it; the second request cannot replace it.
        assert!(!worker.begin(&mut rejected));
        assert_eq!(rejected.as_ref().unwrap().first, [9]);
        assert_eq!(worker.finish().unwrap().report, Ok(1));
        worker.stop();
        assert!(worker.join());
        assert!(!worker.begin(&mut rejected));
        assert_eq!(rejected.as_ref().unwrap().first, [9]);
        assert!(worker.join());
    }

    #[test]
    fn acquisition_panic_recovers_partial_owned_jobs_before_completion_and_stops() {
        let worker = Worker::start("owned-acquisition-test", panic_owned).unwrap();
        let mut request = Some(Owned {
            first: vec![7],
            second: Some(vec![8]),
        });
        assert!(worker.begin(&mut request));
        let done = worker.finish().unwrap();
        assert_eq!(done.report, Err(Failure::Panicked));
        assert_eq!(done.request.first, [42]);
        assert_eq!(done.request.second.as_deref(), Some(&[8][..]));
        request = Some(done.request);
        assert!(!worker.begin(&mut request));
        assert!(worker.join());
        assert_eq!(worker.slot.counters.snapshot().panicked, 1);
        assert_eq!(worker.slot.counters.snapshot().completed, 1);
    }

    #[test]
    fn acquisition_pending_stop_returns_unread_request() {
        // Exercise deterministic pre-run cancellation with owned bytes only.
        let slot = Arc::new(Slot::new());
        {
            let mut state = slot.lock();
            state.pending = Some(Owned {
                first: vec![7],
                second: Some(vec![8]),
            });
            state.busy = true;
            state.stop = true;
        }
        worker_loop(Arc::clone(&slot), change_owned);
        assert!(slot.complete.load(Ordering::Acquire));
        let done = slot.lock().completed.take().unwrap();
        assert_eq!(done.report, Err(Failure::Stopped));
        assert_eq!(done.request.first, [7]);
        assert_eq!(done.request.second.as_deref(), Some(&[8][..]));
        assert_eq!(slot.counters.snapshot().stopped, 1);
    }

    struct LeaseRequest {
        live: Arc<AtomicBool>,
        panic: bool,
    }

    fn local_lease(request: &mut LeaseRequest) -> bool {
        struct Lease<'a>(&'a AtomicBool);
        impl Drop for Lease<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        request.live.store(true, Ordering::Release);
        let _lease = Lease(&request.live);
        assert!(
            !request.panic,
            "synthetic failure while a local lease exists"
        );
        true
    }

    #[test]
    fn acquisition_completion_follows_local_lease_drop_on_return_and_unwind() {
        for panic in [false, true] {
            let worker = Worker::start("owned-acquisition-test", local_lease).unwrap();
            let live = Arc::new(AtomicBool::new(false));
            let mut request = Some(LeaseRequest {
                live: Arc::clone(&live),
                panic,
            });
            assert!(worker.begin(&mut request));
            let done = worker.finish().unwrap();
            assert!(!live.load(Ordering::Acquire));
            assert_eq!(
                done.report,
                if panic {
                    Err(Failure::Panicked)
                } else {
                    Ok(true)
                }
            );
            worker.stop();
            assert!(worker.join());
        }
    }

    struct IndependentRequest {
        meet: Arc<std::sync::Barrier>,
        bytes: Vec<u8>,
    }

    fn independent(request: &mut IndependentRequest) -> u8 {
        request.meet.wait();
        request.bytes[0] += 1;
        request.bytes[0]
    }

    #[test]
    fn acquisition_independent_slots_can_run_together_with_disjoint_owned_outputs() {
        let first = Worker::start("owned-acquisition-test-first", independent).unwrap();
        let second = Worker::start("owned-acquisition-test-second", independent).unwrap();
        assert_ne!(
            first.slot.thread_id.load(Ordering::Acquire),
            second.slot.thread_id.load(Ordering::Acquire)
        );
        let meet = Arc::new(std::sync::Barrier::new(2));
        let mut first_request = Some(IndependentRequest {
            meet: Arc::clone(&meet),
            bytes: vec![1],
        });
        let mut second_request = Some(IndependentRequest {
            meet,
            bytes: vec![9],
        });
        assert!(first.begin(&mut first_request));
        assert!(second.begin(&mut second_request));
        let first_done = first.finish().unwrap();
        let second_done = second.finish().unwrap();
        assert_eq!(first_done.report, Ok(2));
        assert_eq!(second_done.report, Ok(10));
        assert_eq!(first_done.request.bytes, [2]);
        assert_eq!(second_done.request.bytes, [10]);
    }
}
