//! One controlled query worker with scoped subscribers and shared source/result
//! ownership. Hosts supply transport; no Tauri, graphics, or game assets enter.

use std::{
    collections::BTreeMap,
    fs::File,
    future::Future,
    io::{self, Read},
    num::NonZeroUsize,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Waker},
    thread::{self, JoinHandle},
};

use replay::container::Replay;

use crate::{
    CountryScan, QueryError, QueryErrorCode, QueryKind, QueryLimits, QueryPriority, QueryRequest,
    QueryResultHandle, QueryTicket, ScanStep, SourceHandle, SourceKey,
    cache::{ByteBudget, Reservation},
    job::validate_range,
    projection::ProjectionCache,
    query::QueryKey,
    source::{self, source_revision},
};

type ReplyValue = Result<QueryResultHandle, QueryError>;
type Notification = (Arc<Reply>, ReplyValue);

#[derive(Clone, Debug)]
pub struct RuntimeLimits {
    /// Compressed backing and exposed metadata, including closed but held sources.
    pub source_bytes: usize,
    /// Reserved during each open for the exposed metadata and its catalogue.
    pub source_metadata_bytes: usize,
    /// Result payloads, outstanding replies, and admitted output reservations.
    pub output_bytes: usize,
    /// Country-table projections shared across countries, ranges and resolutions.
    pub projection_bytes: usize,
    pub max_sources: usize,
    pub max_scopes: usize,
    pub max_cache_entries: usize,
    pub frame_quantum: NonZeroUsize,
    /// A waiting history receives a turn after this many foreground quanta.
    pub foreground_quanta: NonZeroUsize,
    pub query: QueryLimits,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            source_bytes: 1024 * 1024 * 1024,
            source_metadata_bytes: 16 * 1024 * 1024,
            output_bytes: 64 * 1024 * 1024,
            projection_bytes: 64 * 1024 * 1024,
            max_sources: 16,
            max_scopes: 128,
            max_cache_entries: 256,
            frame_quantum: NonZeroUsize::new(8).unwrap(),
            foreground_quanta: NonZeroUsize::new(4).unwrap(),
            query: QueryLimits::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RuntimeStats {
    pub source_bytes: usize,
    pub opening_bytes: usize,
    pub output_bytes: usize,
    pub projection_bytes: usize,
    pub projection_hits: u64,
    pub block_reads: u64,
    pub sources: usize,
    pub retired_sources: usize,
    pub subscriptions: usize,
    pub queued_jobs: usize,
    pub active_jobs: usize,
    pub cache_entries: usize,
    pub cache_hits: u64,
    pub scans_started: u64,
    pub completed_scans: u64,
}

#[derive(Default)]
struct ReplyState {
    value: Option<ReplyValue>,
    completed: bool,
    waker: Option<Waker>,
}

#[derive(Default)]
struct Reply {
    state: Mutex<ReplyState>,
    ready: Condvar,
}

impl Reply {
    fn complete(&self, value: ReplyValue) {
        let wake = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.completed {
                return;
            }
            state.value = Some(value);
            state.completed = true;
            state.waker.take()
        };
        self.ready.notify_all();
        if let Some(waker) = wake {
            waker.wake();
        }
    }
}

/// Await this directly on the host executor. There is no OS waiter per widget.
/// Dropping it removes only this scope/generation from shared work.
pub struct QuerySubscription {
    ticket: QueryTicket,
    reply: Arc<Reply>,
    runtime: Weak<Shared>,
}

impl QuerySubscription {
    pub fn ticket(&self) -> &QueryTicket {
        &self.ticket
    }

    pub fn try_recv(&mut self) -> Option<ReplyValue> {
        self.reply
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .value
            .take()
    }

    /// Blocking convenience for console clients. Async hosts should await self.
    pub fn recv(self) -> ReplyValue {
        let mut state = self
            .reply
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        loop {
            if let Some(value) = state.value.take() {
                return value;
            }
            if state.completed {
                return Err(QueryError::invalid("Query reply was already consumed"));
            }
            state = self
                .reply
                .ready
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

impl Future for QuerySubscription {
    type Output = ReplyValue;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self
            .reply
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(value) = state.value.take() {
            return Poll::Ready(value);
        }
        if state.completed {
            return Poll::Ready(Err(QueryError::invalid("Query reply was already consumed")));
        }
        if state
            .waker
            .as_ref()
            .is_none_or(|waker| !waker.will_wake(cx.waker()))
        {
            state.waker = Some(cx.waker().clone());
        }
        Poll::Pending
    }
}

impl Drop for QuerySubscription {
    fn drop(&mut self) {
        if let Some(shared) = self.runtime.upgrade() {
            cancel(&shared, &self.ticket);
        }
    }
}

struct Scope {
    ticket: QueryTicket,
    generation: u64,
    key: QueryKey,
    priority: QueryPriority,
    job: Option<u64>,
    reply: Arc<Reply>,
}

struct Job {
    key: QueryKey,
    source: SourceHandle,
    priority: QueryPriority,
    cancelled: Arc<AtomicBool>,
    running: bool,
}

struct Cached {
    result: QueryResultHandle,
    touched: u64,
}

struct Opening {
    bytes: usize,
    cancelled: Arc<AtomicBool>,
}

#[derive(Default)]
struct State {
    closed: bool,
    sources: BTreeMap<SourceKey, SourceHandle>,
    retired: Vec<SourceHandle>,
    opening: BTreeMap<SourceKey, Opening>,
    source_bytes: usize,
    scopes: BTreeMap<String, Scope>,
    jobs: BTreeMap<u64, Job>,
    cache: BTreeMap<QueryKey, Cached>,
    next_job: u64,
    tick: u64,
    cache_hits: u64,
    scans_started: u64,
    completed_scans: u64,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    stopped: AtomicBool,
    limits: RuntimeLimits,
    output: Arc<ByteBudget>,
    projections: ProjectionCache,
}

pub struct AnalyticsRuntime {
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl AnalyticsRuntime {
    pub fn new(limits: RuntimeLimits) -> io::Result<Self> {
        if limits.source_bytes == 0
            || limits.max_sources == 0
            || limits.max_scopes == 0
            || limits.source_metadata_bytes == 0
            || limits.source_metadata_bytes > limits.source_bytes
            || limits.max_cache_entries == 0
            || limits.query.max_result_bytes == 0
            || limits.query.max_countries == 0
            || limits.query.max_block_raw_bytes == 0
            || limits.output_bytes < limits.query.max_result_bytes
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid analytics runtime limits",
            ));
        }
        let output = ByteBudget::new(limits.output_bytes);
        let projections = ProjectionCache::new(limits.projection_bytes);
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            stopped: AtomicBool::new(false),
            limits,
            output,
            projections,
        });
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("replay-analytics".into())
            .spawn(move || run(worker_shared))?;
        Ok(Self {
            shared,
            worker: Mutex::new(Some(worker)),
        })
    }

    /// Register an already opened immutable snapshot. The injecting host owns
    /// the path/revision witness; reusing a key for a different Arc is refused.
    /// Initial catalogue decoding is blocking and belongs off the UI thread.
    pub fn register_source(
        &self,
        key: SourceKey,
        path: PathBuf,
        replay: Arc<Replay>,
    ) -> Result<SourceHandle, QueryError> {
        key.validate()?;
        {
            let state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if state.closed {
                return Err(QueryError::closed());
            }
            if let Some(existing) = state.sources.get(&key) {
                if existing.path() == path && Arc::ptr_eq(&existing.0.replay, &replay) {
                    return Ok(existing.clone());
                }
                return Err(QueryError::new(
                    QueryErrorCode::SourceChanged,
                    "Source key already names another snapshot",
                ));
            }
        }
        let opening = self.begin_open(&key, replay.bytes().len())?;
        let prepared = source::prepare(
            key.clone(),
            path,
            replay,
            &self.shared.limits.query,
            self.shared.limits.source_metadata_bytes,
            || self.check_open(&opening, &AtomicBool::new(false)),
        );
        self.finish_open(key, prepared)
    }

    /// Read a source once, checking the expected length/mtime witness before and
    /// after loading. Limits are reserved before I/O; concurrent duplicate opens
    /// report a resource status instead of allocating another backing buffer.
    pub fn open_source(&self, key: SourceKey, path: PathBuf) -> Result<SourceHandle, QueryError> {
        self.open_source_cancellable(key, path, &AtomicBool::new(false))
    }

    /// Cancellation is checked between bounded reads and catalogue frames. A
    /// single replay parse or block decompression remains indivisible.
    pub fn open_source_cancellable(
        &self,
        key: SourceKey,
        path: PathBuf,
        cancelled: &AtomicBool,
    ) -> Result<SourceHandle, QueryError> {
        self.check_open(&AtomicBool::new(false), cancelled)?;
        key.validate()?;
        let path = path.canonicalize()?;
        if source_revision(&path)? != key.revision {
            return Err(QueryError::new(
                QueryErrorCode::SourceChanged,
                "Recording revision changed before opening",
            ));
        }
        if let Some(existing) = self.source(&key) {
            if existing.path() == path {
                return Ok(existing);
            }
            return Err(QueryError::new(
                QueryErrorCode::SourceChanged,
                "Source identifier belongs to a different path",
            ));
        }
        let length = usize::try_from(path.metadata()?.len())
            .map_err(|_| QueryError::limit("Recording is too large for this address space"))?;
        let opening = self.begin_open(&key, length)?;
        let check = || self.check_open(&opening, cancelled);
        let prepared = (|| {
            check()?;
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(length)
                .map_err(|_| QueryError::limit("Cannot allocate the admitted replay backing"))?;
            let mut file = File::open(&path)?;
            let mut chunk = [0u8; 64 * 1024];
            while bytes.len() < length {
                check()?;
                let wanted = chunk.len().min(length - bytes.len());
                let count = file.read(&mut chunk[..wanted])?;
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..count]);
            }
            check()?;
            let mut extra = [0u8; 1];
            if file.read(&mut extra)? != 0 {
                return Err(QueryError::new(
                    QueryErrorCode::SourceChanged,
                    "Recording grew while opening",
                ));
            }
            if bytes.len() != length || source_revision(&path)? != key.revision {
                return Err(QueryError::new(
                    QueryErrorCode::SourceChanged,
                    "Recording changed while opening",
                ));
            }
            check()?;
            let replay = Arc::new(Replay::open(bytes)?);
            let source = source::prepare(
                key.clone(),
                path.clone(),
                replay,
                &self.shared.limits.query,
                self.shared.limits.source_metadata_bytes,
                check,
            )?;
            if source_revision(&path)? != key.revision {
                return Err(QueryError::new(
                    QueryErrorCode::SourceChanged,
                    "Recording changed during catalogue discovery",
                ));
            }
            check()?;
            Ok(source)
        })();
        self.finish_open(key, prepared)
    }

    fn check_open(&self, opening: &AtomicBool, external: &AtomicBool) -> Result<(), QueryError> {
        if self.shared.stopped.load(Ordering::Relaxed) {
            Err(QueryError::closed())
        } else if opening.load(Ordering::Relaxed) || external.load(Ordering::Relaxed) {
            Err(QueryError::cancelled())
        } else {
            Ok(())
        }
    }

    fn begin_open(&self, key: &SourceKey, bytes: usize) -> Result<Arc<AtomicBool>, QueryError> {
        let bytes = bytes
            .checked_add(self.shared.limits.source_metadata_bytes)
            .ok_or_else(|| QueryError::limit("Source admission size overflow"))?;
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reap_sources(&mut state);
        if state.closed {
            return Err(QueryError::closed());
        }
        if state.opening.contains_key(key) || state.sources.contains_key(key) {
            return Err(QueryError::limit(
                "This source is already opening or registered",
            ));
        }
        if state.sources.len() + state.retired.len() + state.opening.len()
            >= self.shared.limits.max_sources
        {
            return Err(QueryError::limit("Source count budget is occupied"));
        }
        let opening = state
            .opening
            .values()
            .map(|opening| opening.bytes)
            .sum::<usize>();
        if state
            .source_bytes
            .saturating_add(opening)
            .saturating_add(bytes)
            > self.shared.limits.source_bytes
        {
            return Err(QueryError::limit("Source backing budget is occupied"));
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        state.opening.insert(
            key.clone(),
            Opening {
                bytes,
                cancelled: Arc::clone(&cancelled),
            },
        );
        Ok(cancelled)
    }

    fn finish_open(
        &self,
        key: SourceKey,
        prepared: Result<SourceHandle, QueryError>,
    ) -> Result<SourceHandle, QueryError> {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let opening = state.opening.remove(&key);
        if state.closed {
            return Err(QueryError::closed());
        }
        if opening.is_none_or(|opening| opening.cancelled.load(Ordering::Relaxed)) {
            return Err(QueryError::cancelled());
        }
        let source = prepared?;
        let opening = state
            .opening
            .values()
            .map(|opening| opening.bytes)
            .sum::<usize>();
        if state
            .source_bytes
            .saturating_add(opening)
            .saturating_add(source.accounted_bytes())
            > self.shared.limits.source_bytes
        {
            return Err(QueryError::limit(
                "Source metadata and backing exceed the admitted source budget",
            ));
        }
        state.source_bytes += source.accounted_bytes();
        state.sources.insert(key, source.clone());
        Ok(source)
    }

    pub fn source(&self, key: &SourceKey) -> Option<SourceHandle> {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reap_sources(&mut state);
        state.sources.get(key).cloned()
    }

    pub fn request(&self, request: QueryRequest) -> Result<QuerySubscription, QueryError> {
        let key = QueryKey::parse(&request, &self.shared.limits.query)?;
        let ticket = request.ticket();
        let generation = ticket.generation_number()?;
        let reply = Arc::new(Reply::default());
        let mut notifications = Vec::new();
        {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if state.closed {
                return Err(QueryError::closed());
            }
            reap_sources(&mut state);
            let source = state.sources.get(&key.source).cloned().ok_or_else(|| {
                QueryError::new(
                    QueryErrorCode::UnknownSource,
                    "Source revision is not registered",
                )
            })?;
            validate_range(&source.0.replay, &key)?;
            if let Some(previous) = state.scopes.get(&ticket.scope_id) {
                if generation <= previous.generation {
                    return Err(QueryError::invalid(
                        "A replacement generation must increase within its scope",
                    ));
                }
            } else if state.scopes.len() >= self.shared.limits.max_scopes {
                return Err(QueryError::limit("Subscription budget is occupied"));
            }
            let next_job = state
                .next_job
                .checked_add(1)
                .ok_or_else(|| QueryError::limit("Query identifier space exhausted"))?;
            let existing_job = state
                .jobs
                .iter()
                .find(|(_, job)| job.key == key && !job.cancelled.load(Ordering::Relaxed))
                .map(|(id, _)| *id);
            if let Some(previous) = detach_scope(&mut state, &ticket.scope_id, None, existing_job) {
                notifications.push(previous);
            }
            state.tick = state.tick.saturating_add(1);
            let tick = state.tick;
            let cached = state.cache.get_mut(&key).map(|cached| {
                cached.touched = tick;
                cached.result.clone()
            });
            let job_id = if let Some(result) = cached {
                state.cache_hits += 1;
                notifications.push((Arc::clone(&reply), Ok(result)));
                None
            } else {
                let id = existing_job.unwrap_or(next_job);
                if existing_job.is_none() {
                    state.next_job = next_job;
                    state.jobs.insert(
                        id,
                        Job {
                            key: key.clone(),
                            source,
                            priority: request.priority,
                            cancelled: Arc::new(AtomicBool::new(false)),
                            running: false,
                        },
                    );
                }
                Some(id)
            };
            state.scopes.insert(
                ticket.scope_id.clone(),
                Scope {
                    ticket: ticket.clone(),
                    generation,
                    key,
                    priority: request.priority,
                    job: job_id,
                    reply: Arc::clone(&reply),
                },
            );
            if let Some(id) = job_id {
                refresh_priority(&mut state, id);
            }
        }
        deliver(notifications);
        self.shared.wake.notify_one();
        Ok(QuerySubscription {
            ticket,
            reply,
            runtime: Arc::downgrade(&self.shared),
        })
    }

    pub fn cancel(&self, ticket: &QueryTicket) {
        cancel(&self.shared, ticket);
    }

    pub fn close_source(&self, key: &SourceKey) {
        let notifications = {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(opening) = state.opening.get(key) {
                opening.cancelled.store(true, Ordering::Relaxed);
            }
            if let Some(source) = state.sources.remove(key) {
                state.retired.push(source);
            }
            state.cache.retain(|query, _| query.source != *key);
            let scopes: Vec<_> = state
                .scopes
                .iter()
                .filter(|(_, scope)| scope.key.source == *key)
                .map(|(id, _)| id.clone())
                .collect();
            let notifications = scopes
                .into_iter()
                .filter_map(|id| detach_scope(&mut state, &id, None, None))
                .collect();
            reap_sources(&mut state);
            notifications
        };
        deliver(notifications);
        self.shared
            .projections
            .retain_sources(|source| source != key);
        self.shared.wake.notify_one();
    }

    pub fn stats(&self) -> RuntimeStats {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        reap_sources(&mut state);
        let opening_bytes = state
            .opening
            .values()
            .map(|opening| opening.bytes)
            .sum::<usize>();
        let projections = &self.shared.projections;
        RuntimeStats {
            source_bytes: state.source_bytes + opening_bytes,
            opening_bytes,
            output_bytes: self.shared.output.used(),
            projection_bytes: projections.bytes(),
            projection_hits: projections.hits.load(Ordering::Relaxed),
            block_reads: projections.block_reads.load(Ordering::Relaxed),
            sources: state.sources.len(),
            retired_sources: state.retired.len(),
            subscriptions: state.scopes.len(),
            queued_jobs: state.jobs.values().filter(|job| !job.running).count(),
            active_jobs: state.jobs.values().filter(|job| job.running).count(),
            cache_entries: state.cache.len(),
            cache_hits: state.cache_hits,
            scans_started: state.scans_started,
            completed_scans: state.completed_scans,
        }
    }

    /// Cancel, wake and join the one owned query worker. Source/result handles
    /// already held by other native consumers retain their ordinary ownership.
    pub fn shutdown(&self) {
        self.stop();
        let mut worker = self
            .worker
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(worker) = worker.take() {
            let _ = worker.join();
        }
    }

    /// Nonjoining cancellation for transport loss. A later shutdown still joins
    /// the worker, including when stop has already been called.
    pub fn stop(&self) {
        stop(&self.shared);
    }
}

impl Drop for AnalyticsRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn reap_sources(state: &mut State) {
    state.retired.retain(|source| {
        if source.externally_held() {
            true
        } else {
            state.source_bytes -= source.accounted_bytes();
            false
        }
    });
}

fn rank(priority: QueryPriority) -> u8 {
    match priority {
        QueryPriority::Current => 0,
        QueryPriority::Prefetch => 1,
        QueryPriority::History => 2,
    }
}

fn refresh_priority(state: &mut State, id: u64) {
    let priority = state
        .scopes
        .values()
        .filter(|scope| scope.job == Some(id))
        .map(|scope| scope.priority)
        .min_by_key(|priority| rank(*priority));
    if let Some(priority) = priority
        && let Some(job) = state.jobs.get_mut(&id)
    {
        job.priority = priority;
    }
}

fn detach_scope(
    state: &mut State,
    scope_id: &str,
    generation: Option<&str>,
    preserve_job: Option<u64>,
) -> Option<Notification> {
    if state.scopes.get(scope_id).is_none_or(|scope| {
        generation.is_some_and(|generation| scope.ticket.generation != generation)
    }) {
        return None;
    }
    let scope = state.scopes.remove(scope_id)?;
    if let Some(id) = scope.job {
        if !state
            .scopes
            .values()
            .any(|remaining| remaining.job == Some(id))
            && preserve_job != Some(id)
        {
            if let Some(job) = state.jobs.get(&id) {
                job.cancelled.store(true, Ordering::Relaxed);
                if !job.running {
                    state.jobs.remove(&id);
                }
            }
        } else {
            refresh_priority(state, id);
        }
    }
    Some((scope.reply, Err(QueryError::cancelled())))
}

fn cancel(shared: &Shared, ticket: &QueryTicket) {
    let notification = {
        let mut state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        detach_scope(&mut state, &ticket.scope_id, Some(&ticket.generation), None)
    };
    if let Some((reply, value)) = notification {
        reply.complete(value);
    }
    shared.wake.notify_one();
}

fn deliver(notifications: Vec<Notification>) {
    for (reply, value) in notifications {
        reply.complete(value);
    }
}

fn publish(state: &mut State, id: u64, value: ReplyValue) -> Vec<Notification> {
    state.jobs.remove(&id);
    state
        .scopes
        .values_mut()
        .filter_map(|scope| {
            if scope.job == Some(id) {
                scope.job = None;
                Some((Arc::clone(&scope.reply), value.clone()))
            } else {
                None
            }
        })
        .collect()
}

fn evict_oldest(state: &mut State) -> bool {
    let key = state
        .cache
        .iter()
        .min_by_key(|(_, cached)| cached.touched)
        .map(|(key, _)| key.clone());
    if let Some(key) = key {
        state.cache.remove(&key);
        true
    } else {
        false
    }
}

struct Active {
    id: u64,
    key: QueryKey,
    source: SourceHandle,
    cancelled: Arc<AtomicBool>,
    scan: Option<CountryScan>,
    output: Reservation,
}

fn background(job: &Job) -> bool {
    // A whole-campaign history cannot monopolize the foreground by declaring
    // itself Current. Its hint still orders it against other waiting histories.
    job.key.kind == QueryKind::History || job.priority == QueryPriority::History
}

fn acquire(
    shared: &Shared,
    state: &mut State,
    history: bool,
    notifications: &mut Vec<Notification>,
) -> Option<Active> {
    let id = state
        .jobs
        .iter()
        .filter(|(_, job)| !job.running && background(job) == history)
        .min_by_key(|(id, job)| (rank(job.priority), **id))
        .map(|(id, _)| *id)?;
    let reservation = loop {
        if let Some(reservation) = shared.output.reserve(shared.limits.query.max_result_bytes) {
            break reservation;
        }
        if !evict_oldest(state) {
            notifications.extend(publish(
                state,
                id,
                Err(QueryError::limit(
                    "Output budget is held by active queries or result consumers",
                )),
            ));
            return None;
        }
    };
    let job = state.jobs.get_mut(&id).expect("selected query exists");
    job.running = true;
    state.scans_started += 1;
    Some(Active {
        id,
        key: job.key.clone(),
        source: job.source.clone(),
        cancelled: Arc::clone(&job.cancelled),
        scan: None,
        output: reservation,
    })
}

#[derive(Default)]
pub(crate) struct FairTurns {
    foreground: usize,
}

impl FairTurns {
    pub(crate) fn history(&mut self, foreground: bool, history: bool, burst: usize) -> bool {
        if history && (!foreground || self.foreground >= burst) {
            self.foreground = 0;
            true
        } else {
            self.foreground = self.foreground.saturating_add(1).min(burst);
            false
        }
    }
}

struct WorkerExit(Arc<Shared>);

impl Drop for WorkerExit {
    fn drop(&mut self) {
        // Also resolves waiting futures if the worker unwinds unexpectedly.
        stop(&self.0);
    }
}

fn run(shared: Arc<Shared>) {
    let _exit = WorkerExit(Arc::clone(&shared));
    let mut foreground = None;
    let mut history = None;
    let mut turns = FairTurns::default();
    loop {
        let mut notifications = Vec::new();
        {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if state.closed {
                return;
            }
            shared
                .projections
                .retain_sources(|source| state.sources.contains_key(source));
            if foreground.is_none() {
                foreground = acquire(&shared, &mut state, false, &mut notifications);
            }
            if history.is_none() {
                history = acquire(&shared, &mut state, true, &mut notifications);
            }
            if let Some(active) = &foreground {
                let speculative = state
                    .jobs
                    .get(&active.id)
                    .is_some_and(|job| job.priority == QueryPriority::Prefetch);
                if speculative
                    && state.jobs.values().any(|job| {
                        !job.running && job.priority == QueryPriority::Current && !background(job)
                    })
                {
                    active.cancelled.store(true, Ordering::Relaxed);
                }
            }
        }
        deliver(notifications);
        if foreground.is_none() && history.is_none() {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            while !state.closed && !state.jobs.values().any(|job| !job.running) {
                state = shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
            if state.closed {
                return;
            }
            continue;
        }
        let background_turn = turns.history(
            foreground.is_some(),
            history.is_some(),
            shared.limits.foreground_quanta.get(),
        );
        let slot = if background_turn {
            &mut history
        } else {
            &mut foreground
        };
        let mut active = slot.take().expect("scheduler selected a ready query");
        let step = (|| {
            if active.cancelled.load(Ordering::Relaxed) {
                return Err(QueryError::cancelled());
            }
            if active.scan.is_none() {
                active.scan = Some(CountryScan::from_key(
                    active.source.replay(),
                    active.key.clone(),
                    shared.limits.query.clone(),
                )?);
            }
            active
                .scan
                .as_mut()
                .expect("query scan initialized")
                .advance_cached(
                    shared.limits.frame_quantum,
                    &active.cancelled,
                    &shared.projections,
                )
        })();
        match step {
            Ok(ScanStep::Yielded) => *slot = Some(active),
            completed => {
                let notifications = {
                    let mut state = shared
                        .state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    let value = if state.closed || active.cancelled.load(Ordering::Relaxed) {
                        Err(QueryError::cancelled())
                    } else {
                        match completed {
                            Ok(ScanStep::Ready(result)) => {
                                let result = active.output.into_result(result);
                                state.tick = state.tick.saturating_add(1);
                                let touched = state.tick;
                                state.cache.insert(
                                    active.key,
                                    Cached {
                                        result: result.clone(),
                                        touched,
                                    },
                                );
                                while state.cache.len() > shared.limits.max_cache_entries {
                                    evict_oldest(&mut state);
                                }
                                state.completed_scans += 1;
                                Ok(result)
                            }
                            Err(error) => Err(error),
                            Ok(ScanStep::Yielded) => unreachable!(),
                        }
                    };
                    publish(&mut state, active.id, value)
                };
                deliver(notifications);
            }
        }
    }
}

fn stop(shared: &Shared) {
    shared.stopped.store(true, Ordering::Relaxed);
    let notifications = {
        let mut state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.closed = true;
        for opening in state.opening.values() {
            opening.cancelled.store(true, Ordering::Relaxed);
        }
        for job in state.jobs.values() {
            job.cancelled.store(true, Ordering::Relaxed);
        }
        state.jobs.clear();
        state.cache.clear();
        shared.projections.retain_sources(|_| false);
        let sources = std::mem::take(&mut state.sources);
        state.retired.extend(sources.into_values());
        reap_sources(&mut state);
        std::mem::take(&mut state.scopes)
            .into_values()
            .map(|scope| (scope.reply, Err(QueryError::closed())))
            .collect()
    };
    deliver(notifications);
    shared.wake.notify_all();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::atomic::AtomicUsize, task::Wake};

    struct WakeCount(AtomicUsize);

    impl Wake for WakeCount {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn subscription_wakes_an_executor_without_a_waiter_thread() {
        let reply = Arc::new(Reply::default());
        let mut subscription = QuerySubscription {
            ticket: QueryTicket {
                scope_id: "widget".into(),
                generation: "1".into(),
            },
            reply: Arc::clone(&reply),
            runtime: Weak::new(),
        };
        let count = Arc::new(WakeCount(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&count));
        let mut cx = Context::from_waker(&waker);
        assert!(Pin::new(&mut subscription).poll(&mut cx).is_pending());
        reply.complete(Err(QueryError::closed()));
        assert_eq!(count.0.load(Ordering::Relaxed), 1);
        assert!(matches!(
            Pin::new(&mut subscription).poll(&mut cx),
            Poll::Ready(Err(QueryError {
                code: QueryErrorCode::Closed,
                ..
            }))
        ));
    }

    #[test]
    fn foreground_load_gives_history_a_bounded_share_of_quanta() {
        let mut turns = FairTurns::default();
        let sequence: Vec<_> = (0..15).map(|_| turns.history(true, true, 4)).collect();
        assert_eq!(
            sequence,
            vec![
                false, false, false, false, true, false, false, false, false, true, false, false,
                false, false, true
            ]
        );
        assert!(turns.history(false, true, 4));
        assert!(!turns.history(true, false, 4));
    }

    #[test]
    fn closing_an_opening_source_cancels_its_admission_and_late_publication() {
        let runtime = AnalyticsRuntime::new(RuntimeLimits::default()).unwrap();
        let key = SourceKey {
            source_id: "opening".into(),
            revision: "snapshot".into(),
        };
        let cancellation = runtime.begin_open(&key, 1024).unwrap();
        assert_eq!(
            runtime.stats().opening_bytes,
            1024 + RuntimeLimits::default().source_metadata_bytes
        );
        runtime.close_source(&key);
        assert!(cancellation.load(Ordering::Relaxed));
        let error = runtime
            .finish_open(
                key.clone(),
                Err(QueryError::invalid("unused prepared result")),
            )
            .err()
            .unwrap();
        assert_eq!(error.code, QueryErrorCode::Cancelled);
        assert_eq!(runtime.stats().opening_bytes, 0);
        assert!(runtime.source(&key).is_none());
        runtime.stop();
        runtime.shutdown();
        assert!(runtime.worker.lock().unwrap().is_none());
    }
}
