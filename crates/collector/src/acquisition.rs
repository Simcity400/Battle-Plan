//! Callback-bounded native acquisition with three precreated, owned-job workers.
//!
//! Dispatch is unsafe because ordinary thread ownership cannot establish the
//! foreign world's lifetime or consistency boundary. The sole hourly callback
//! must finish every accepted request before its live-read lease ends. Workers
//! receive neither the enclosing captured frame nor Shared, and construct their
//! own !Send live reader. Immutable country authentication is acquired once by
//! the callback, owned through Arc leases, and verified after all worker joins.

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
    construction, division_eta, economy, governance,
    intel::{self, RelationsCopyContext, RelationsCopyReport, RelationsJob},
    production,
};

pub(crate) struct CharactersRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub context: CharactersCopyContext,
    pub first: CharactersJob,
    pub second: Option<CharactersJob>,
}

pub(crate) struct WorldPair<J, C> {
    pub context: C,
    pub first: J,
    pub second: Option<J>,
}

/// Thirty hourly tables, using only their existing owned raw-copy buffers.
pub(crate) struct WorldRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub economy: WorldPair<economy::EconomyRestJob, economy::EconomyRestCopyContext>,
    pub construction:
        WorldPair<construction::ConstructionRestJob, construction::ConstructionRestCopyContext>,
    pub production: WorldPair<production::ProductionRestJob, production::ProductionRestCopyContext>,
    pub focus: WorldPair<governance::FocusJob, governance::FocusCopyContext>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WorldReport {
    pub economy: economy::EconomyRestCopyReport,
    pub construction: construction::ConstructionRestCopyReport,
    pub production: production::ProductionRestCopyReport,
    pub focus: governance::FocusCopyReport,
    pub ticks: u64,
}

// Exactly one field is populated. Fixed storage avoids boxing a large enum
// in the callback; the same existing thread runs either kind of owned job.
struct CharactersTask {
    characters: Option<CharactersRequest>,
    world: Option<WorldRequest>,
}
#[derive(Clone, Copy)]
struct CharactersTaskReport {
    characters: Option<CharactersCopyReport>,
    world: Option<WorldReport>,
}
pub(crate) type WorldCompletion = Completion<WorldRequest, WorldReport>;

pub(crate) struct RelationsRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub context: RelationsCopyContext,
    pub first: RelationsJob,
    pub second: Option<RelationsJob>,
}

pub(crate) struct EtaRequest {
    pub countries: Arc<engine::country_memory::Cache>,
    pub base: usize,
    pub state: usize,
    pub checked: bool,
    pub captured: division_eta::Captured,
    pub first: Option<Arc<crate::division_census::UnitInputs>>,
    pub second: Option<Arc<crate::division_census::UnitInputs>>,
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
    pub world: WorkerCalibration,
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
    // Shares the existing ETA worker calibration lock; World leaves this empty.
    // Published separately from country authentication to keep scopes distinct.
    #[serde(skip)]
    pub read_witness: EtaReadCalibration,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(crate) struct EtaReadCalibration {
    pub captures: u64,
    pub stages: division_eta::StageDiagnostics,
    pub hits: u64,
    pub retained_misses: u64,
    pub direct_reads: u64,
    pub witness_skips: u64,
    pub witness_fallbacks: u64,
    pub witness_entries_peak: usize,
    pub witness_bytes_peak: usize,
    pub witness_entries_required: usize,
    pub witness_bytes_required: usize,
}
impl EtaReadCalibration {
    fn observe(&mut self, next: division_eta::CacheDiagnostics) {
        self.captures = self.captures.saturating_add(1);
        self.stages.absorb(next.stages);
        self.hits = self.hits.saturating_add(next.hits);
        self.retained_misses = self.retained_misses.saturating_add(next.retained_misses);
        self.direct_reads = self.direct_reads.saturating_add(next.direct_reads);
        self.witness_skips = self.witness_skips.saturating_add(next.witness_skips);
        self.witness_fallbacks = self
            .witness_fallbacks
            .saturating_add(next.witness_fallbacks);
        self.witness_entries_peak = self.witness_entries_peak.max(next.witness_entries_peak);
        self.witness_bytes_peak = self.witness_bytes_peak.max(next.witness_bytes_peak);
        self.witness_entries_required = self
            .witness_entries_required
            .max(next.witness_entries_required);
        self.witness_bytes_required = self.witness_bytes_required.max(next.witness_bytes_required);
    }
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

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WorldCalibration {
    pub country_cache: EtaCountryCacheCalibration,
    pub acquisition_ticks: u64,
    pub last_error: Option<&'static str>,
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

fn note_dispatch(counters: &Counters, accepted: bool) {
    if accepted {
        &counters.accepted
    } else {
        &counters.rejected
    }
    .fetch_add(1, Ordering::Relaxed);
}

fn note_completion<R>(counters: &Counters, report: &Result<R, Failure>) {
    counters.completed.fetch_add(1, Ordering::Relaxed);
    match report {
        Err(Failure::Panicked) => {
            counters.panicked.fetch_add(1, Ordering::Relaxed);
        }
        Err(Failure::Stopped) => {
            counters.stopped.fetch_add(1, Ordering::Relaxed);
        }
        Ok(_) => {}
    }
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
    let cached = request.countries.view(&memory);
    characters::copy_characters_refresh(
        &cached,
        request.context,
        &mut request.first,
        request.second.as_mut(),
    )
}

fn run_relations(request: &mut RelationsRequest) -> RelationsCopyReport {
    // SAFETY: same callback-bounded contract as run_characters.
    let memory = unsafe { LiveMemory::new() };
    let cached = request.countries.view(&memory);
    intel::copy_relations_refresh(
        &cached,
        request.context,
        &mut request.first,
        request.second.as_mut(),
    )
}

/// Genuine synchronous acquisition for H0/full refresh and rejected dispatch.
/// Extraction leaves every family's reads, fault masks and overflow unchanged.
pub(crate) fn run_world(request: &mut WorldRequest, memory: &impl engine::Memory) -> WorldReport {
    let started = crate::sampling::qpc();
    let production = production::copy_production_rest_refresh(
        memory,
        request.production.context,
        &mut request.production.first,
        request.production.second.as_mut(),
    );
    let construction = construction::copy_construction_rest_refresh(
        memory,
        request.construction.context,
        &mut request.construction.first,
        request.construction.second.as_mut(),
    );
    let economy = economy::copy_economy_rest_refresh(
        memory,
        request.economy.context,
        &mut request.economy.first,
        request.economy.second.as_mut(),
    );
    let focus = governance::copy_focus_refresh(
        memory,
        request.focus.context,
        &mut request.focus.first,
        request.focus.second.as_mut(),
    );
    WorldReport {
        economy,
        construction,
        production,
        focus,
        ticks: crate::sampling::qpc().saturating_sub(started).max(0) as u64,
    }
}

fn run_world_shared(
    request: &mut WorldRequest,
    memory: &impl engine::Memory,
    calibration: &Mutex<WorldCalibration>,
) -> WorldReport {
    let started = crate::sampling::qpc();
    let snapshot = Arc::clone(&request.countries);
    let cached = snapshot.view(memory);
    let seed = cached.seed_status();
    let entries = cached.cached_entries();
    let mut report = run_world(request, &cached);
    // The callback's Countries anchor owns preparation and terminal checking.
    // No worker may publish or credit checks before that shared verdict.
    report.ticks = crate::sampling::qpc().saturating_sub(started).max(0) as u64;
    let mut counters = calibration
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    counters.country_cache.observe(seed, entries);
    counters.acquisition_ticks = counters.acquisition_ticks.saturating_add(report.ticks);
    report
}

fn run_eta_with_memory(
    request: &mut EtaRequest,
    memory: &impl engine::Memory,
    seed_calibration: &Mutex<EtaCountryCacheCalibration>,
) -> division_eta::AcquisitionReport {
    let started = crate::sampling::qpc();
    let cached = request.countries.view(memory);
    seed_calibration
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .observe(cached.seed_status(), cached.cached_entries());
    let (matched, _) = request.captured.capture(
        &cached,
        request.base,
        request.state,
        &[request.checked],
        production::Sources {
            first: request.first.as_ref().map_or(&[], |inputs| inputs.seeds()),
            first_outcome: request.first.as_ref().map_or(
                engine::walk::Outcome::Unavailable {
                    reason: "division inputs unavailable",
                },
                |inputs| inputs.outcome(),
            ),
            second: request.second.as_ref().map_or(&[], |inputs| inputs.seeds()),
            second_outcome: request.second.as_ref().map_or(
                engine::walk::Outcome::Unavailable {
                    reason: "division check inputs unavailable",
                },
                |inputs| inputs.outcome(),
            ),
        },
    );
    seed_calibration
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .read_witness
        .observe(request.captured.cache_diagnostics());
    // The outer callback joins every reader and then verifies these shared
    // country observations. Its existing dependent-table verdict applies to
    // ETA too; this worker's result cannot be published independently.
    request.first = None;
    request.second = None;
    division_eta::AcquisitionReport {
        matched: matched[0],
        ticks: crate::sampling::qpc().saturating_sub(started).max(0) as u64,
    }
}

pub(crate) struct Pool {
    characters: Worker<CharactersTask, CharactersTaskReport>,
    characters_requests: Counters,
    world_requests: Counters,
    relations: Worker<RelationsRequest, RelationsCopyReport>,
    eta: Worker<EtaRequest, division_eta::AcquisitionReport>,
    eta_country_cache: Arc<Mutex<EtaCountryCacheCalibration>>,
    world_calibration: Arc<Mutex<WorldCalibration>>,
}

impl Pool {
    /// Startup only: allocates fixed slots and registered threads. Partial
    /// startup failure drops and joins every worker already created.
    pub(crate) fn start() -> Result<Self, String> {
        let eta_country_cache = Arc::new(Mutex::new(EtaCountryCacheCalibration::default()));
        let eta_worker_calibration = Arc::clone(&eta_country_cache);
        let world_calibration = Arc::new(Mutex::new(WorldCalibration::default()));
        let world_worker_calibration = Arc::clone(&world_calibration);
        Ok(Self {
            characters: Worker::start(
                "hoi4-acquire-characters",
                move |request: &mut CharactersTask| {
                    match (&mut request.characters, &mut request.world) {
                        (Some(characters), None) => CharactersTaskReport {
                            characters: Some(run_characters(characters)),
                            world: None,
                        },
                        (None, Some(world)) => {
                            // SAFETY: same callback-bounded lease as the Characters job.
                            let memory = unsafe { LiveMemory::new() };
                            CharactersTaskReport {
                                characters: None,
                                world: Some(run_world_shared(
                                    world,
                                    &memory,
                                    &world_worker_calibration,
                                )),
                            }
                        }
                        _ => panic!("invalid Characters/World acquisition request"),
                    }
                },
            )?,
            characters_requests: Counters::default(),
            world_requests: Counters::default(),
            relations: Worker::start("hoi4-acquire-relations", run_relations)?,
            eta: Worker::start("hoi4-acquire-eta", move |request| {
                // SAFETY: dispatch retains the hourly lease until finish_eta;
                // neither this reader nor its immutable country view escapes.
                let memory = unsafe { LiveMemory::new() };
                run_eta_with_memory(request, &memory, &eta_worker_calibration)
            })?,
            eta_country_cache,
            world_calibration,
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
        let mut task = request.take().map(|characters| CharactersTask {
            characters: Some(characters),
            world: None,
        });
        let accepted = self.characters.begin(&mut task);
        if let Some(task) = task {
            *request = task.characters;
        }
        note_dispatch(&self.characters_requests, accepted);
        accepted
    }

    /// # Safety
    /// Same callback lease as begin_characters; uses that same thread only
    /// when no Characters refresh was scheduled for the current hour.
    pub(crate) unsafe fn begin_world(&self, request: &mut Option<WorldRequest>) -> bool {
        let mut task = request.take().map(|world| CharactersTask {
            characters: None,
            world: Some(world),
        });
        let accepted = self.characters.begin(&mut task);
        if let Some(task) = task {
            *request = task.world;
        }
        note_dispatch(&self.world_requests, accepted);
        accepted
    }

    /// # Safety
    /// The same callback/lifetime/barrier contract as begin_characters applies.
    pub(crate) unsafe fn begin_relations(&self, request: &mut Option<RelationsRequest>) -> bool {
        self.relations.begin(request)
    }

    pub(crate) fn finish_characters(&self) -> Option<CharactersCompletion> {
        self.characters.finish().map(|completion| {
            note_completion(&self.characters_requests, &completion.report);
            Completion {
                request: completion
                    .request
                    .characters
                    .expect("submitted Characters request"),
                report: completion
                    .report
                    .map(|report| report.characters.expect("Characters report")),
            }
        })
    }

    pub(crate) fn finish_world(&self) -> Option<WorldCompletion> {
        self.characters.finish().map(|completion| {
            note_completion(&self.world_requests, &completion.report);
            if let Err(failure) = completion.report {
                self.world_calibration
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .last_error = Some(match failure {
                    Failure::Panicked => "World acquisition worker panicked",
                    Failure::Stopped => "World acquisition worker stopped before reading",
                });
            }
            Completion {
                request: completion.request.world.expect("submitted World request"),
                report: completion
                    .report
                    .map(|report| report.world.expect("World report")),
            }
        })
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
        // Snapshot payload belongs to Shared, counted once there. These
        // workers retain no separate country-cache payload allocation.
        0
    }
    pub(crate) fn world_calibration(&self) -> WorldCalibration {
        *self
            .world_calibration
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn calibration(&self) -> Calibration {
        Calibration {
            thread_ids: self.thread_ids(),
            characters: self.characters_requests.snapshot(),
            world: self.world_requests.snapshot(),
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

    fn country_snapshot(
        memory: &impl engine::Memory,
        cells: usize,
    ) -> Arc<engine::country_memory::Cache> {
        let mut storage = Arc::new(engine::country_memory::Cache::with_capacity(cells));
        engine::country_memory::Cache::prepare_shared(&mut storage, memory, 0, 0x1000).unwrap()
    }

    fn empty_country_snapshot() -> Arc<engine::country_memory::Cache> {
        country_snapshot(
            &engine::memory::SliceMemory {
                base: 0,
                bytes: &empty_eta_memory(),
            },
            64,
        )
    }

    #[test]
    fn country_snapshot_leases_return_after_success_panic_and_rejected_dispatch() {
        for panics in [false, true] {
            let bytes = empty_eta_memory();
            let memory = engine::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let mut owner = Arc::new(engine::country_memory::Cache::with_capacity(64));
            let snapshot =
                engine::country_memory::Cache::prepare_shared(&mut owner, &memory, 0, 0x1000)
                    .unwrap();
            let mut request = empty_eta_request(false);
            request.countries = snapshot;
            let worker = Worker::start("shared-country-lease-test", move |_: &mut EtaRequest| {
                assert!(!panics, "synthetic acquisition failure");
            })
            .unwrap();
            let mut pending = Some(request);
            assert!(worker.begin(&mut pending));
            let completed = worker.finish().unwrap();
            assert_eq!(completed.report.is_err(), panics);
            assert!(
                engine::country_memory::Cache::prepare_shared(&mut owner, &memory, 0, 0x1000,)
                    .is_err(),
                "completion retains the snapshot until its request is restored"
            );
            worker.stop();
            assert!(worker.join());
            let mut rejected = Some(completed.request);
            assert!(!worker.begin(&mut rejected));
            assert!(Arc::get_mut(&mut owner).is_none());
            assert!(
                rejected
                    .as_ref()
                    .unwrap()
                    .countries
                    .view(&memory)
                    .verify()
                    .is_ok()
            );
            drop(rejected);
            assert!(
                engine::country_memory::Cache::prepare_shared(&mut owner, &memory, 0, 0x1000,)
                    .is_ok(),
                "all completed/rejected request leases are gone"
            );
        }
    }

    fn empty_eta_request(checked: bool) -> EtaRequest {
        let first = Arc::new(crate::division_census::UnitInputs::empty_rows());
        EtaRequest {
            countries: empty_country_snapshot(),
            base: 0,
            state: 0x1000,
            checked,
            captured: division_eta::Captured::empty(),
            second: checked.then(|| Arc::clone(&first)),
            first: Some(first),
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

    fn world_request(checked: bool) -> WorldRequest {
        let mut request = WorldRequest {
            countries: empty_country_snapshot(),
            economy: WorldPair {
                context: economy::EconomyRestCopyContext {
                    base: 0,
                    game_state: 0x1000,
                    needed: [checked; 10],
                    resource_rows: [0; 3],
                    resource_second: [0; 3],
                    resource_scratch: 0,
                },
                first: Default::default(),
                second: checked.then(Default::default),
            },
            construction: WorldPair {
                context: construction::ConstructionRestCopyContext {
                    base: 0,
                    game_state: 0x1000,
                    needed: [checked; 10],
                },
                first: Default::default(),
                second: checked.then(Default::default),
            },
            production: WorldPair {
                context: production::ProductionRestCopyContext {
                    base: 0,
                    game_state: 0x1000,
                    needed: [checked; 8],
                },
                first: Default::default(),
                second: checked.then(Default::default),
            },
            focus: WorldPair {
                context: governance::Captured::with_capacities(
                    &Default::default(),
                    &[0; governance::N],
                )
                .focus_copy_context(0, 0x1000, [checked; 2]),
                first: Default::default(),
                second: checked.then(Default::default),
            },
        };
        macro_rules! provision {
            ($pair:expr) => {{
                $pair.first.log = crate::raw::RawLog::new(512, 65536);
                if let Some(second) = &mut $pair.second {
                    second.log = crate::raw::RawLog::new(512, 65536);
                }
            }};
        }
        provision!(request.economy);
        provision!(request.construction);
        provision!(request.production);
        request.focus.first = governance::FocusJob::new();
        request.focus.second = checked.then(governance::FocusJob::new);
        request
    }

    #[test]
    fn world_owned_worker_matches_direct_copy_partitions_and_returns_both_passes() {
        for checked in [false, true] {
            let bytes = empty_eta_memory();
            let mut direct = world_request(checked);
            let direct_report = run_world(
                &mut direct,
                &engine::memory::SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
            );
            let calibration = Arc::new(Mutex::new(WorldCalibration::default()));
            let worker_calibration = Arc::clone(&calibration);
            let worker = Worker::start("world-owned-test", move |request| {
                run_world_shared(
                    request,
                    &engine::memory::SliceMemory {
                        base: 0,
                        bytes: &bytes,
                    },
                    &worker_calibration,
                )
            })
            .unwrap();
            let mut request = Some(world_request(checked));
            let capacity = request.as_ref().unwrap().economy.first.log.spans.capacity();
            assert!(worker.begin(&mut request));
            assert!(request.is_none());
            let completed = worker.finish().unwrap();
            let report = completed.report.unwrap();
            assert_eq!(calibration.lock().unwrap().country_cache.ready, 1);
            assert_eq!(
                completed.request.economy.first.log.spans.capacity(),
                capacity
            );
            macro_rules! compare {
                ($family:ident, $n:expr) => {{
                    assert_eq!(report.$family.raw_ok, direct_report.$family.raw_ok);
                    assert_eq!(report.$family.matched, direct_report.$family.matched);
                    assert_eq!(report.$family.raw_bytes, direct_report.$family.raw_bytes);
                    assert_eq!(completed.request.$family.second.is_some(), checked);
                    for slot in 0..$n {
                        assert!(
                            completed
                                .request
                                .$family
                                .first
                                .log
                                .partition_equal(&direct.$family.first.log, slot)
                        );
                    }
                }};
            }
            compare!(economy, 10);
            compare!(construction, 10);
            compare!(production, 8);
            assert_eq!(report.focus.raw_ok, direct_report.focus.raw_ok);
            assert_eq!(report.focus.matched, direct_report.focus.matched);
            assert_eq!(report.focus.mismatches, direct_report.focus.mismatches);
            assert_eq!(report.focus.raw_bytes, direct_report.focus.raw_bytes);
            assert_eq!(completed.request.focus.second.is_some(), checked);
            let decoded_focus = |job: &governance::FocusJob| {
                assert!(job.is_provisioned());
                let mut captured =
                    governance::Captured::with_capacities(&Default::default(), &[0; governance::N]);
                captured.decode_focus_payload(0, job, &mut [0; 2]);
                captured.tables()[..2].to_vec()
            };
            assert_eq!(
                decoded_focus(&completed.request.focus.first),
                decoded_focus(&direct.focus.first),
            );
            if let Some(second) = &completed.request.focus.second {
                assert_eq!(
                    decoded_focus(second),
                    decoded_focus(direct.focus.second.as_ref().unwrap())
                );
            }
            worker.stop();
            assert!(worker.join());
            let mut rejected = Some(completed.request);
            assert!(!worker.begin(&mut rejected));
            let bytes = empty_eta_memory();
            let fallback = run_world(
                rejected.as_mut().unwrap(),
                &engine::memory::SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
            );
            assert_eq!(fallback.economy.raw_bytes, direct_report.economy.raw_bytes);
            assert_eq!(fallback.economy.matched, direct_report.economy.matched);
        }
    }

    #[test]
    fn world_shared_country_fault_is_detected_at_outer_barrier_and_capacity_is_observable() {
        struct Changed {
            bytes: Vec<u8>,
            reads: std::cell::Cell<usize>,
        }
        impl engine::Memory for Changed {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if !engine::Memory::copy(
                    &engine::memory::SliceMemory {
                        base: 0,
                        bytes: &self.bytes,
                    },
                    address,
                    output,
                ) {
                    return false;
                }
                if address == 0x1000 + engine::cgame_state::COUNTRIES {
                    self.reads.set(self.reads.get() + 1);
                    if self.reads.get() > 1 {
                        output[0] ^= 1;
                    }
                }
                true
            }
        }
        let changed = Changed {
            bytes: empty_eta_memory(),
            reads: std::cell::Cell::new(0),
        };
        let calibration = Mutex::new(WorldCalibration::default());
        let mut request = world_request(true);
        request.countries = country_snapshot(&changed, 64);
        run_world_shared(&mut request, &changed, &calibration);
        assert_eq!(
            changed.reads.get(),
            1,
            "worker must use the shared authentication"
        );
        assert!(
            request.countries.view(&changed).verify().is_err(),
            "the outer barrier must reject shared source mutation before publishing rows"
        );
        assert_eq!(calibration.lock().unwrap().last_error, None);
        let bytes = empty_eta_memory();
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut request = world_request(false);
        request.countries = country_snapshot(&memory, 0);
        run_world_shared(&mut request, &memory, &calibration);
        assert_eq!(
            calibration.lock().unwrap().country_cache.capacity_fallbacks,
            1
        );
        let mut overflow = world_request(false);
        overflow.economy.first.log = Default::default();
        let report = run_world(&mut overflow, &memory);
        assert!(!report.economy.raw_ok);
        assert!(report.focus.raw_ok);
    }

    #[test]
    fn characters_and_world_requests_reuse_one_thread_without_mixing_reports() {
        let worker = Worker::start("characters-world-test", |request: &mut CharactersTask| {
            assert_ne!(request.characters.is_some(), request.world.is_some());
            CharactersTaskReport {
                characters: request
                    .characters
                    .as_ref()
                    .map(|_| CharactersCopyReport::default()),
                world: request.world.as_ref().map(|_| WorldReport::default()),
            }
        })
        .unwrap();
        let thread = worker.slot.thread_id.load(Ordering::Acquire);
        let mut world = Some(CharactersTask {
            characters: None,
            world: Some(world_request(true)),
        });
        assert!(worker.begin(&mut world));
        let completed = worker.finish().unwrap();
        assert!(completed.request.characters.is_none());
        assert!(completed.request.world.unwrap().economy.second.is_some());
        assert!(completed.report.unwrap().world.is_some());
        assert!(completed.report.unwrap().characters.is_none());
        let mut characters = Some(CharactersTask {
            characters: Some(CharactersRequest {
                countries: empty_country_snapshot(),
                context: CharactersCopyContext {
                    base: 0,
                    game_state: 0x1000,
                },
                first: Default::default(),
                second: None,
            }),
            world: None,
        });
        assert!(worker.begin(&mut characters));
        let completed = worker.finish().unwrap();
        assert!(completed.request.world.is_none());
        assert!(completed.report.unwrap().characters.is_some());
        assert!(completed.report.unwrap().world.is_none());
        assert_eq!(worker.slot.thread_id.load(Ordering::Acquire), thread);
        worker.stop();
        assert!(worker.join());
    }

    #[test]
    fn world_panic_recovers_owned_bundle_without_retry_and_separate_counters_do_not_mix() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let worker_calls = Arc::clone(&calls);
        let worker = Worker::start(
            "world-panic-test",
            move |_request: &mut WorldRequest| -> WorldReport {
                worker_calls.fetch_add(1, Ordering::Relaxed);
                panic!("synthetic World acquisition panic");
            },
        )
        .unwrap();
        let mut request = Some(world_request(true));
        let capacity = request.as_ref().unwrap().focus.first.capacities();
        assert!(worker.begin(&mut request));
        let completed = worker.finish().unwrap();
        assert!(matches!(completed.report, Err(Failure::Panicked)));
        assert_eq!(completed.request.focus.first.capacities(), capacity);
        assert!(completed.request.focus.second.is_some());
        assert!(worker.join());
        let mut rejected = Some(completed.request);
        assert!(!worker.begin(&mut rejected));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let world = Counters::default();
        let characters = Counters::default();
        note_dispatch(&world, true);
        note_completion::<WorldReport>(&world, &Err(Failure::Panicked));
        note_dispatch(&world, false);
        assert_eq!(world.snapshot().accepted, 1);
        assert_eq!(world.snapshot().completed, 1);
        assert_eq!(world.snapshot().panicked, 1);
        assert_eq!(world.snapshot().rejected, 1);
        assert_eq!(characters.snapshot().accepted, 0);
    }

    #[test]
    fn eta_owned_worker_returns_both_passes_for_outer_verification_and_can_be_reused() {
        let bytes = empty_eta_memory();
        let calibration = Arc::new(Mutex::new(EtaCountryCacheCalibration::default()));
        let worker_calibration = Arc::clone(&calibration);
        let worker = Worker::start("eta-owned-test", move |request| {
            let memory = engine::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            run_eta_with_memory(request, &memory, &worker_calibration)
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
            assert!(completed.request.first.is_none());
            assert!(completed.request.second.is_none());
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
            &calibration,
        );
        assert!(report.matched);
        assert_eq!(calibration.lock().unwrap().ready, 3);
    }

    #[test]
    fn eta_shared_country_mutation_is_rejected_by_outer_verification() {
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
        request.countries = country_snapshot(&memory, 64);
        let report = run_eta_with_memory(
            &mut request,
            &memory,
            &Mutex::new(EtaCountryCacheCalibration::default()),
        );
        assert!(
            report.matched,
            "table checks await the shared outer verdict"
        );
        assert_eq!(memory.root_reads.get(), 1, "ETA must not reseed countries");
        assert!(
            request.countries.view(&memory).verify().is_err(),
            "outer country verdict must reject these otherwise consistent rows"
        );
        assert!(request.first.is_none());
        assert!(request.second.is_none());
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
    fn eta_read_calibration_sums_work_and_retains_peak_demand_separately() {
        let mut calibration = EtaCountryCacheCalibration::default();
        calibration
            .read_witness
            .observe(division_eta::CacheDiagnostics {
                hits: 5,
                retained_misses: 2,
                direct_reads: 3,
                witness_skips: 4,
                witness_fallbacks: 1,
                witness_entries_peak: 8,
                witness_bytes_peak: 100,
                witness_entries_required: 10,
                witness_bytes_required: 140,
                stages: division_eta::StageDiagnostics {
                    walk: division_eta::StageCost {
                        calls: 1,
                        ticks: 11,
                    },
                    supply_builds: 2,
                    ..Default::default()
                },
            });
        calibration
            .read_witness
            .observe(division_eta::CacheDiagnostics {
                hits: 7,
                retained_misses: 1,
                direct_reads: 2,
                witness_skips: 3,
                witness_fallbacks: 2,
                witness_entries_peak: 6,
                witness_bytes_peak: 120,
                witness_entries_required: 12,
                witness_bytes_required: 130,
                stages: division_eta::StageDiagnostics {
                    walk: division_eta::StageCost {
                        calls: 2,
                        ticks: 17,
                    },
                    supply_builds: 3,
                    ..Default::default()
                },
            });
        assert_eq!(
            serde_json::to_value(calibration.read_witness).unwrap(),
            serde_json::json!({
                "captures": 2, "hits": 12, "retained_misses": 3,
                "direct_reads": 5, "witness_skips": 7, "witness_fallbacks": 3,
                "witness_entries_peak": 8, "witness_bytes_peak": 120,
                "witness_entries_required": 12, "witness_bytes_required": 140,
                "stages": division_eta::StageDiagnostics {
                    walk: division_eta::StageCost { calls: 3, ticks: 28 },
                    supply_builds: 5,
                    ..Default::default()
                },
            })
        );
        // Worker read diagnostics are not mislabeled as country authentication.
        assert!(
            serde_json::to_value(calibration)
                .unwrap()
                .get("read_witness")
                .is_none()
        );
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
