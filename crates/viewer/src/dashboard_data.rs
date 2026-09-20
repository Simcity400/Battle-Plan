//! One source owner and a bounded response pump shared by native maps and charts.
//!
//! File opening and query completion never depend on the native window being
//! visible. Only compact projected results cross the desktop control pipe.

use analytics::{
    AnalyticsRuntime, QueryResultHandle, QuerySubscription, RuntimeLimits, SourceHandle, SourceKey,
};
use desktop_protocol::{
    dashboard_data::{
        CountryFlagReply, CountryFlagRequest, DataCommand, DataEvent, LandOwnersReply,
        MAX_REPLY_BYTES, ReplyChunks, SourceReply,
    },
    replay_revision,
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, VecDeque},
    future::Future,
    io::{self, Write},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    task::{Context, Poll, Wake, Waker},
    thread,
};

const MAX_PENDING: usize = 64;
const MAX_SOURCE_LEASES: usize = 64;
const MAX_SERIALIZED_BYTES: usize = MAX_REPLY_BYTES * 2;
const MAX_CANCELLED: usize = MAX_PENDING * 2;

/// The same token stays with a request through loading, serialization and every
/// fragment. Cancellation cannot be forgotten when a reply leaves a queue.
struct Request {
    id: String,
    cancelled: AtomicBool,
}

impl Request {
    fn new(id: String) -> Arc<Self> {
        Arc::new(Self {
            id,
            cancelled: AtomicBool::new(false),
        })
    }

    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

pub struct SharedSource {
    flags: Mutex<Option<crate::country_flags::FlagContext>>,
    land_owners: Mutex<Option<analytics::LandOwners>>,
    handle: SourceHandle,
    key: SourceKey,
    path: PathBuf,
    runtime: Arc<AnalyticsRuntime>,
}

impl SharedSource {
    pub fn replay(&self) -> Arc<replay::container::Replay> {
        self.handle.replay()
    }

    pub fn metadata(&self) -> &analytics::ReplayMetadata {
        self.handle.metadata()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn key(&self) -> &SourceKey {
        &self.key
    }
}

impl Drop for SharedSource {
    fn drop(&mut self) {
        self.runtime.close_source(&self.key);
    }
}

#[derive(Default)]
struct Sources {
    next: u64,
    entries: BTreeMap<(PathBuf, String), Weak<SharedSource>>,
}

struct Lease {
    request: Arc<Request>,
    source: Arc<SharedSource>,
}

struct PendingQuery {
    request: Arc<Request>,
    subscription: Pin<Box<QuerySubscription>>,
    // Hold the source registration while its projected result is requested.
    _source: Arc<SharedSource>,
}

enum Payload {
    Source(Box<SourceReply>),
    Query(QueryResultHandle),
    Flags(CountryFlagReply),
    LandOwners(LandOwnersReply),
    MapSettings(crate::map_settings::Reply),
}

impl Payload {
    fn serialize(&self, request: &Request) -> Result<String, String> {
        match self {
            Self::Source(source) => bounded_json(source, &request.cancelled),
            Self::Query(result) => bounded_json(&**result, &request.cancelled),
            Self::Flags(result) => bounded_json(result, &request.cancelled),
            Self::MapSettings(result) => bounded_json(result, &request.cancelled),
            Self::LandOwners(result) => bounded_json(result, &request.cancelled),
        }
    }
}

#[derive(Default)]
struct Pending {
    // Admission includes completed replies until their final fragment is sent.
    requests: BTreeMap<String, Arc<Request>>,
    queries: BTreeMap<String, PendingQuery>,
    leases: BTreeMap<String, Lease>,
    source_requests: BTreeMap<String, String>,
    ready: VecDeque<(Arc<Request>, Payload)>,
    errors: VecDeque<(Arc<Request>, String)>,
    // Recent cancel-before-admission messages are bounded separately. Live work
    // keeps its own token even after an unrelated tombstone evicts an old id.
    cancelled: VecDeque<String>,
    next_lease: u64,
}

impl Pending {
    fn begin(&mut self, id: String) -> Option<Arc<Request>> {
        if self.cancelled.contains(&id)
            || self.requests.contains_key(&id)
            || self.source_requests.contains_key(&id)
            || self.requests.len() >= MAX_PENDING * 2
        {
            return None;
        }
        let at_capacity = self.requests.len() >= MAX_PENDING;
        let request = Request::new(id);
        self.requests
            .insert(request.id.clone(), Arc::clone(&request));
        if at_capacity {
            // Reserve at most another MAX_PENDING small rejection replies. A
            // conforming host already bounds all outstanding requests to 64.
            self.errors
                .push_back((request, "Dashboard request capacity reached".into()));
            None
        } else {
            Some(request)
        }
    }

    fn current(&self, request: &Arc<Request>) -> bool {
        !request.cancelled()
            && self
                .requests
                .get(&request.id)
                .is_some_and(|current| Arc::ptr_eq(current, request))
    }

    fn rollback(&mut self, request: &Arc<Request>) -> Option<Lease> {
        let lease_id = self.source_requests.get(&request.id)?;
        if !self
            .leases
            .get(lease_id)
            .is_some_and(|lease| Arc::ptr_eq(&lease.request, request))
        {
            return None;
        }
        let lease_id = self.source_requests.remove(&request.id)?;
        self.leases.remove(&lease_id)
    }

    fn finish(&mut self, request: &Arc<Request>, delivered: bool) -> Option<Lease> {
        if self
            .requests
            .get(&request.id)
            .is_some_and(|current| Arc::ptr_eq(current, request))
        {
            self.requests.remove(&request.id);
        }
        if delivered && !request.cancelled() {
            None
        } else {
            self.rollback(request)
        }
    }

    fn cancel(&mut self, id: &str) -> (Option<PendingQuery>, Option<Lease>) {
        if !self.cancelled.iter().any(|cancelled| cancelled == id) {
            if self.cancelled.len() == MAX_CANCELLED {
                self.cancelled.pop_front();
            }
            self.cancelled.push_back(id.to_owned());
        }
        if let Some(request) = self.requests.remove(id) {
            request.cancel();
        }
        let query = self.queries.remove(id);
        self.ready.retain(|(request, _)| request.id != id);
        self.errors.retain(|(request, _)| request.id != id);
        let lease = self
            .source_requests
            .remove(id)
            .and_then(|lease_id| self.leases.remove(&lease_id));
        if let Some(lease) = &lease {
            lease.request.cancel();
        }
        (query, lease)
    }

    fn stop(&mut self) -> Self {
        for request in self.requests.values() {
            request.cancel();
        }
        for lease in self.leases.values() {
            lease.request.cancel();
        }
        std::mem::take(self)
    }
}

#[derive(Default)]
struct Signal {
    version: AtomicU64,
    mutex: Mutex<()>,
    changed: Condvar,
}

impl Signal {
    fn notify(&self) {
        let _guard = self.mutex.lock().unwrap_or_else(|error| error.into_inner());
        self.version.fetch_add(1, Ordering::Release);
        self.changed.notify_all();
    }
}

impl Wake for Signal {
    fn wake(self: Arc<Self>) {
        self.notify();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.notify();
    }
}

struct Inner {
    lease_namespace: uuid::Uuid,
    map_settings: Mutex<crate::map_settings::Store>,
    flag_options: Mutex<Option<(installed_content::ResolveOptions, bool)>>,
    runtime: Arc<AnalyticsRuntime>,
    sources: Mutex<Sources>,
    pending: Mutex<Pending>,
    loader: Mutex<Option<SyncSender<LoadJob>>>,
    signal: Arc<Signal>,
    closed: AtomicBool,
}

impl Inner {
    fn lease_id(&self, sequence: u64) -> String {
        format!("lease-{}-{sequence}", self.lease_namespace)
    }

    /// The source lock serializes cold opens, including duplicate map requests.
    /// Call only on the dedicated loader or a native map's loading thread.
    fn open(
        &self,
        path: &Path,
        expected: Option<&str>,
        cancelled: &AtomicBool,
    ) -> Result<Arc<SharedSource>, String> {
        self.check_open(cancelled)?;
        let path = path.canonicalize().map_err(|error| error.to_string())?;
        let revision = replay_revision(&path).map_err(|error| error.to_string())?;
        if expected.is_some_and(|expected| expected != revision) {
            return Err("The recording changed. Reopen it before requesting another view.".into());
        }
        let mut sources = self.sources.lock().map_err(|error| error.to_string())?;
        // A cancelled open may have waited behind another source's cold load.
        self.check_open(cancelled)?;
        sources
            .entries
            .retain(|_, source| source.strong_count() > 0);
        let cache_key = (path.clone(), revision.clone());
        if let Some(source) = sources.entries.get(&cache_key).and_then(Weak::upgrade) {
            return Ok(source);
        }
        sources.next = sources
            .next
            .checked_add(1)
            .ok_or("Source identity exhausted")?;
        let key = SourceKey {
            source_id: format!("source-{}", sources.next),
            revision,
        };
        let handle = self
            .runtime
            .open_source_cancellable(key.clone(), path.clone(), cancelled)
            .map_err(|error| error.to_string())?;
        let source = Arc::new(SharedSource {
            flags: Mutex::new(None),
            land_owners: Mutex::new(None),
            handle,
            key,
            path,
            runtime: Arc::clone(&self.runtime),
        });
        // Construct the owner before checking again so cancellation after a
        // successful registration still closes that registration on drop.
        self.check_open(cancelled)?;
        sources.entries.insert(cache_key, Arc::downgrade(&source));
        Ok(source)
    }

    fn check_open(&self, cancelled: &AtomicBool) -> Result<(), String> {
        if self.closed.load(Ordering::Acquire) {
            Err("Dashboard data service is closing".into())
        } else if cancelled.load(Ordering::Acquire) {
            Err("Dashboard source request was cancelled".into())
        } else {
            Ok(())
        }
    }

    fn begin(&self, id: String) -> Option<Arc<Request>> {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.closed.load(Ordering::Acquire) {
            return None;
        }
        let request = pending.begin(id);
        drop(pending);
        // begin may have queued a bounded capacity error instead of work.
        self.signal.notify();
        request
    }

    fn error(&self, request: Arc<Request>, message: String) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !self.closed.load(Ordering::Acquire) && pending.current(&request) {
            pending
                .errors
                .push_back((request, bounded_text(&message, 2048)));
        }
        drop(pending);
        self.signal.notify();
    }

    fn finish(&self, request: &Arc<Request>, delivered: bool) {
        let lease = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .finish(request, delivered);
        drop(lease);
    }

    /// Terminal cleanup is safe on either service thread. It never joins a
    /// thread; the owning Service performs all joins during retirement.
    fn stop(&self) {
        let pending = {
            let mut pending = self
                .pending
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            self.closed.store(true, Ordering::Release);
            pending.stop()
        };
        self.loader
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        drop(pending);
        self.runtime.stop();
        self.signal.notify();
    }
}

struct OpenJob {
    request: Arc<Request>,
    path: String,
    revision: String,
}

enum LoadJob {
    MapSettings {
        target: desktop_protocol::dashboard_data::MapSettingsTarget,
        request: Arc<Request>,
        source_id: String,
        source: Option<Arc<SharedSource>>,
        settings: Option<desktop_protocol::CampaignMapSettings>,
        options: installed_content::ResolveOptions,
    },
    Open(OpenJob),
    LandOwners {
        request: Arc<Request>,
        source_id: String,
        source: Arc<SharedSource>,
    },
    Flags {
        request: Arc<Request>,
        query: CountryFlagRequest,
        source: Arc<SharedSource>,
        options: installed_content::ResolveOptions,
        allow_unverified: bool,
    },
}

pub struct Service {
    inner: Arc<Inner>,
    thread_ids: [thread::ThreadId; 2],
    threads: Mutex<Vec<thread::JoinHandle<()>>>,
}

struct ThreadExit(Arc<Inner>);

impl Drop for ThreadExit {
    fn drop(&mut self) {
        self.0.stop();
    }
}

impl Service {
    pub fn new(emit: Arc<dyn Fn(DataEvent) -> bool + Send + Sync>) -> io::Result<Arc<Self>> {
        let (loader, jobs) = mpsc::sync_channel::<LoadJob>(4);
        let inner = Arc::new(Inner {
            lease_namespace: uuid::Uuid::new_v4(),
            map_settings: Mutex::new(crate::map_settings::Store::open()),
            flag_options: Mutex::new(None),
            runtime: Arc::new(AnalyticsRuntime::new(RuntimeLimits::default())?),
            sources: Mutex::new(Sources::default()),
            pending: Mutex::new(Pending::default()),
            loader: Mutex::new(Some(loader)),
            signal: Arc::new(Signal::default()),
            closed: AtomicBool::new(false),
        });
        let load_inner = Arc::clone(&inner);
        let load = thread::Builder::new()
            .name("dashboard source loader".into())
            .spawn(move || {
                let _exit = ThreadExit(Arc::clone(&load_inner));
                let mut visual_catalogue: Option<(
                    PathBuf,
                    PathBuf,
                    Vec<installed_content::content::visual::VisualMod>,
                )> = None;
                while let Ok(job) = jobs.recv() {
                    if load_inner.closed.load(Ordering::Acquire) {
                        break;
                    }
                    let job = match job {
                        LoadJob::MapSettings {
                            target,
                            request,
                            source_id,
                            source,
                            settings,
                            options,
                        } => {
                            let result = (|| -> Result<crate::map_settings::Reply, String> {
                                if request.cancelled() {
                                    return Err("Cancelled".into());
                                }
                                let replay = source.as_ref().map(|source| source.replay());
                                let campaign = replay.as_ref().map(|replay| crate::map_settings::campaign_key(replay)).unwrap_or_default();
                                let key = target.key(&campaign).map_err(str::to_owned)?;
                                let mut settings = settings;
                                {
                                    let mut store = load_inner.map_settings.lock().map_err(|e| e.to_string())?;
                                    store.configure_game(&options.user_dir);
                                    if let Some(settings) = &mut settings && settings.use_game_graphics {
                                        if matches!(target, desktop_protocol::dashboard_data::MapSettingsTarget::Global) && store.game_graphics().is_none() {
                                            return Err(store.game_graphics_error().unwrap_or_else(|| "the game graphics settings unavailable".into()));
                                        }
                                        settings.graphics = store.inherited(&key).graphics;
                                    }
                                }
                                if settings.is_none()
                                    || visual_catalogue.as_ref().is_none_or(|(game, user, _)| {
                                        game != &options.game_dir || user != &options.user_dir
                                    })
                                {
                                    let mods = crate::map_settings::catalogue(
                                        &options,
                                        &request.cancelled,
                                    )
                                    .map_err(|e| e.to_string())?;
                                    visual_catalogue = Some((
                                        options.game_dir.clone(),
                                        options.user_dir.clone(),
                                        mods,
                                    ));
                                }
                                let mods = visual_catalogue
                                    .as_ref()
                                    .expect("catalogue loaded")
                                    .2
                                    .clone();
                                if let Some(settings) = &settings {
                                    settings.validate().map_err(str::to_owned)?;
                                    if !crate::map_settings::sample_counts().contains(&settings.graphics.samples) {
                                        return Err("This anti-aliasing setting is unavailable on the current renderer".into());
                                    }
                                    if let Some(selected) = &settings.visual_mods {
                                        for id in selected {
                                            let item = mods
                                                .iter()
                                                .find(|item| &item.id == id)
                                                .ok_or("Selected mod is no longer installed")?;
                                            if let Some(reason) = &item.reason {
                                                return Err(format!("{}: {reason}", item.name));
                                            }
                                        }
                                    }
                                }
                                if request.cancelled() {
                                    return Err("Cancelled".into());
                                }
                                let mut store =
                                    load_inner.map_settings.lock().map_err(|e| e.to_string())?;
                                store.check()?;
                                if let Some(settings) = settings {
                                    store
                                        .save(key.clone(), settings)
                                        .map_err(|e| e.to_string())?;
                                }
                                let recorded_mods = replay.as_ref().into_iter().flat_map(|replay| replay.identity.mods.iter())
                                    .filter_map(|recorded| {
                                        mods.iter()
                                            .find(|item| {
                                                item.recorded_id == recorded.id
                                                    && item.reason.is_none()
                                            })
                                            .map(|item| item.id.clone())
                                    })
                                    .collect();
                                Ok(crate::map_settings::Reply {
                                    source_id,
                                    revision: source.as_ref().map(|source| source.key().revision.clone()).unwrap_or_default(),
                                    settings: store.preferences(&key),
                                    inherited_settings: store.inherited(&key),
                                    mods,
                                    recorded_mods,
                                    sample_counts: crate::map_settings::sample_counts(),
                                    game_graphics: store.game_graphics(),
                                    game_graphics_error: store.game_graphics_error(),
                                })
                            })();
                            let mut pending =
                                load_inner.pending.lock().unwrap_or_else(|e| e.into_inner());
                            if pending.current(&request)
                                && !load_inner.closed.load(Ordering::Acquire)
                            {
                                match result {
                                    Ok(reply) => pending
                                        .ready
                                        .push_back((request, Payload::MapSettings(reply))),
                                    Err(error) => pending
                                        .errors
                                        .push_back((request, bounded_text(&error, 2048))),
                                }
                            }
                            drop(pending);
                            load_inner.signal.notify();
                            continue;
                        }
                        LoadJob::Open(job) => {
                            let user = load_inner.flag_options.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|(options, _)| options.user_dir.clone());
                            if let Some(user) = user {
                                load_inner.map_settings.lock().unwrap_or_else(|e| e.into_inner()).configure_game(&user);
                            }
                            job
                        },
                        LoadJob::LandOwners {
                            request,
                            source_id,
                            source,
                        } => {
                            if request.cancelled() {
                                continue;
                            }
                            let result = (|| -> Result<LandOwnersReply, String> {
                                let mut cache = source
                                    .land_owners
                                    .lock()
                                    .map_err(|error| error.to_string())?;
                                if cache.is_none() {
                                    *cache = Some(
                                        analytics::land_owners(
                                            &source.replay(),
                                            &request.cancelled,
                                        )
                                        .map_err(|error| error.to_string())?,
                                    );
                                }
                                let result =
                                    cache.as_ref().ok_or("Country discovery unavailable")?;
                                Ok(LandOwnersReply {
                                    source_id,
                                    revision: source.key().revision.clone(),
                                    countries: result.countries.clone(),
                                    warnings: result.warnings.clone(),
                                })
                            })();
                            let mut pending = load_inner
                                .pending
                                .lock()
                                .unwrap_or_else(|error| error.into_inner());
                            if pending.current(&request)
                                && !load_inner.closed.load(Ordering::Acquire)
                            {
                                match result {
                                    Ok(reply) => pending
                                        .ready
                                        .push_back((request, Payload::LandOwners(reply))),
                                    Err(error) => pending
                                        .errors
                                        .push_back((request, bounded_text(&error, 2048))),
                                }
                            }
                            drop(pending);
                            load_inner.signal.notify();
                            continue;
                        }
                        LoadJob::Flags {
                            request,
                            query,
                            source,
                            options,
                            allow_unverified,
                        } => {
                            if request.cancelled() {
                                continue;
                            }
                            let result = (|| -> Result<CountryFlagReply, String> {
                                let mut cache =
                                    source.flags.lock().map_err(|error| error.to_string())?;
                                if !cache
                                    .as_ref()
                                    .is_some_and(|cached| cached.matches(&options, &query))
                                {
                                    *cache = Some(
                                        crate::country_flags::FlagContext::load(
                                            source.replay(),
                                            options,
                                            allow_unverified,
                                            &query,
                                            &request.cancelled,
                                        )
                                        .map_err(|error| error.to_string())?,
                                    );
                                }
                                cache
                                    .as_ref()
                                    .ok_or("Country flag context unavailable")?
                                    .reply(&query, &request.cancelled)
                                    .map_err(|error| error.to_string())
                            })();
                            let mut pending = load_inner
                                .pending
                                .lock()
                                .unwrap_or_else(|error| error.into_inner());
                            if pending.current(&request)
                                && !load_inner.closed.load(Ordering::Acquire)
                            {
                                match result {
                                    Ok(reply) => {
                                        pending.ready.push_back((request, Payload::Flags(reply)))
                                    }
                                    Err(error) => pending
                                        .errors
                                        .push_back((request, bounded_text(&error, 2048))),
                                }
                            }
                            drop(pending);
                            load_inner.signal.notify();
                            continue;
                        }
                    };
                    if job.request.cancelled() {
                        continue;
                    }
                    let result = load_inner.open(
                        Path::new(&job.path),
                        Some(&job.revision),
                        &job.request.cancelled,
                    );
                    let mut pending = load_inner
                        .pending
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    if !pending.current(&job.request) || load_inner.closed.load(Ordering::Acquire) {
                        drop(pending);
                        drop(result);
                        load_inner.finish(&job.request, false);
                        continue;
                    }
                    match result {
                        Ok(source) if pending.leases.len() < MAX_SOURCE_LEASES => {
                            if let Some(next) = pending.next_lease.checked_add(1) {
                                pending.next_lease = next;
                                let id = load_inner.lease_id(next);
                                let reply = SourceReply {
                                    id: id.clone(),
                                    path: source.path().to_string_lossy().into_owned(),
                                    revision: source.key().revision.clone(),
                                    metadata: source.metadata().clone(),
                                };
                                pending.leases.insert(
                                    id.clone(),
                                    Lease {
                                        request: Arc::clone(&job.request),
                                        source,
                                    },
                                );
                                pending.source_requests.insert(job.request.id.clone(), id);
                                pending
                                    .ready
                                    .push_back((job.request, Payload::Source(Box::new(reply))));
                            } else {
                                pending.errors.push_back((
                                    job.request,
                                    "Source lease identity exhausted".into(),
                                ));
                            }
                        }
                        Ok(_) => pending.errors.push_back((
                            job.request,
                            "Too many open dashboard source bindings".into(),
                        )),
                        Err(error) => pending
                            .errors
                            .push_back((job.request, bounded_text(&error, 2048))),
                    }
                    drop(pending);
                    load_inner.signal.notify();
                }
            })?;
        let pump_inner = Arc::clone(&inner);
        let pump = match thread::Builder::new()
            .name("dashboard response pump".into())
            .spawn(move || run_pump(pump_inner, emit))
        {
            Ok(pump) => pump,
            Err(error) => {
                inner.stop();
                let _ = load.join();
                inner.runtime.shutdown();
                return Err(error);
            }
        };
        let thread_ids = [load.thread().id(), pump.thread().id()];
        Ok(Arc::new(Self {
            inner,
            thread_ids,
            threads: Mutex::new(vec![load, pump]),
        }))
    }

    pub fn configure_flags(&self, config: &crate::Config, allow_unverified: bool) {
        self.inner
            .map_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .configure_game(&config.user_dir);
        *self
            .inner
            .flag_options
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = config
            .resolve_options()
            .ok()
            .map(|options| (options, allow_unverified));
    }

    pub fn map_settings(
        &self,
        replay: &replay::container::Replay,
    ) -> desktop_protocol::CampaignMapSettings {
        self.inner
            .map_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&crate::map_settings::campaign_key(replay))
    }

    pub fn widget_map_settings(
        &self,
        replay: &replay::container::Replay,
        id: &str,
    ) -> desktop_protocol::CampaignMapSettings {
        self.inner
            .map_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&format!(
                "{}/widget/{id}",
                crate::map_settings::campaign_key(replay)
            ))
    }

    pub fn source(&self, id: &str) -> Result<Arc<SharedSource>, String> {
        self.inner
            .pending
            .lock()
            .map_err(|error| error.to_string())?
            .leases
            .get(id)
            .map(|lease| Arc::clone(&lease.source))
            .ok_or_else(|| "The map's recording binding has been released".into())
    }

    pub fn open_map(
        &self,
        path: &Path,
        revision: Option<&str>,
    ) -> Result<Arc<SharedSource>, String> {
        self.inner.open(path, revision, &self.inner.closed)
    }

    pub fn dispatch(&self, command: DataCommand) {
        if self.inner.closed.load(Ordering::Acquire) {
            return;
        }
        if command
            .request_id()
            .is_some_and(|id| id.is_empty() || id.len() > 128 || id.contains('\0'))
        {
            return;
        }
        match command {
            DataCommand::MapSettings {
                id,
                target,
                source_id,
                revision,
                settings,
            } => {
                let Some(request) = self.inner.begin(id) else {
                    return;
                };
                if let Err(error) = target.key("") {
                    self.inner.error(request, error.into());
                    return;
                }
                let source = if matches!(
                    target,
                    desktop_protocol::dashboard_data::MapSettingsTarget::Global
                ) {
                    if !source_id.is_empty() || !revision.is_empty() {
                        self.inner
                            .error(request, "Global settings do not take a campaign".into());
                        return;
                    }
                    None
                } else {
                    match self.source(&source_id) {
                        Ok(source) if source.key().revision == revision => Some(source),
                        _ => {
                            self.inner
                                .error(request, "Campaign source changed or closed".into());
                            return;
                        }
                    }
                };
                let Some((options, _)) = self
                    .inner
                    .flag_options
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
                else {
                    self.inner.error(
                        request,
                        "Choose the game and user folders in Settings first".into(),
                    );
                    return;
                };
                let job = LoadJob::MapSettings {
                    target,
                    request: Arc::clone(&request),
                    source_id,
                    source,
                    settings,
                    options,
                };
                if self
                    .inner
                    .loader
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .is_none_or(|loader| loader.try_send(job).is_err())
                {
                    self.inner.error(
                        request,
                        "The asset loader is busy. Try map settings again.".into(),
                    );
                }
            }
            DataCommand::Open { id, path, revision } => {
                let Some(request) = self.inner.begin(id) else {
                    return;
                };
                if path.is_empty()
                    || path.contains('\0')
                    || path.len() > 32_000
                    || revision.is_empty()
                    || revision.len() > 128
                    || revision.contains('\0')
                {
                    self.inner
                        .error(request, "Invalid dashboard source request".into());
                    return;
                }
                let job = OpenJob {
                    request: Arc::clone(&request),
                    path,
                    revision,
                };
                let accepted = self
                    .inner
                    .loader
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .as_ref()
                    .is_some_and(|loader| loader.try_send(LoadJob::Open(job)).is_ok());
                if !accepted {
                    self.inner.error(
                        request,
                        "The source loader is busy. Retry this recording.".into(),
                    );
                }
            }
            DataCommand::LandOwners {
                id,
                source_id,
                revision,
            } => {
                let Some(request) = self.inner.begin(id) else {
                    return;
                };
                let source = match self.source(&source_id) {
                    Ok(source) if source.key().revision == revision => source,
                    _ => {
                        self.inner
                            .error(request, "Country source is unavailable or changed".into());
                        return;
                    }
                };
                let job = LoadJob::LandOwners {
                    request: Arc::clone(&request),
                    source_id,
                    source,
                };
                let accepted = self
                    .inner
                    .loader
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .as_ref()
                    .is_some_and(|loader| loader.try_send(job).is_ok());
                if !accepted {
                    self.inner.error(
                        request,
                        "Country discovery is busy. Retry the country picker.".into(),
                    );
                }
            }
            DataCommand::CountryFlags { id, request: query } => {
                let Some(request) = self.inner.begin(id) else {
                    return;
                };
                if let Err(error) = query.validate() {
                    self.inner.error(request, error.into());
                    return;
                }
                let source = match self.source(&query.source_id) {
                    Ok(source) if source.key().revision == query.revision => source,
                    _ => {
                        self.inner.error(
                            request,
                            "Country flag source is unavailable or changed".into(),
                        );
                        return;
                    }
                };
                let Some((options, allow_unverified)) = self
                    .inner
                    .flag_options
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone()
                else {
                    self.inner.error(
                        request,
                        "Choose the game and user folders in the native viewer to load flags"
                            .into(),
                    );
                    return;
                };
                let job = LoadJob::Flags {
                    request: Arc::clone(&request),
                    query,
                    source,
                    options,
                    allow_unverified,
                };
                let accepted = self
                    .inner
                    .loader
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .as_ref()
                    .is_some_and(|loader| loader.try_send(job).is_ok());
                if !accepted {
                    self.inner.error(
                        request,
                        "The asset loader is busy. Retry the country picker.".into(),
                    );
                }
            }
            DataCommand::Query { id, mut request } => {
                let Some(token) = self.inner.begin(id) else {
                    return;
                };
                let source = match self.source(&request.source_id) {
                    Ok(source) => source,
                    Err(error) => {
                        self.inner.error(token, error);
                        return;
                    }
                };
                if request.revision != source.key().revision {
                    self.inner.error(
                        token,
                        "The query belongs to a different recording revision".into(),
                    );
                    return;
                }
                // The UI lease is not the shared source/cache identity.
                request.source_id = source.key().source_id.clone();
                let mut pending = self
                    .inner
                    .pending
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if self.inner.closed.load(Ordering::Acquire) || !pending.current(&token) {
                    return;
                }
                match self.inner.runtime.request(request) {
                    Ok(subscription) => {
                        pending.queries.insert(
                            token.id.clone(),
                            PendingQuery {
                                request: token,
                                subscription: Box::pin(subscription),
                                _source: source,
                            },
                        );
                    }
                    Err(error) => pending.errors.push_back((token, error.to_string())),
                }
                drop(pending);
                self.inner.signal.notify();
            }
            DataCommand::Cancel { id } => {
                let discarded = self
                    .inner
                    .pending
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .cancel(&id);
                drop(discarded);
                self.inner.signal.notify();
            }
            DataCommand::Release { source_id } => {
                let mut pending = self
                    .inner
                    .pending
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let request = pending
                    .leases
                    .get(&source_id)
                    .map(|lease| Arc::clone(&lease.request));
                let discarded = request.map(|request| pending.cancel(&request.id));
                drop(pending);
                drop(discarded);
                self.inner.signal.notify();
            }
        }
    }

    pub fn shutdown(&self) {
        self.inner.stop();
        // A service thread only requests stop. Keep its handle for the owner's
        // subsequent retirement. Check before locking: another caller may hold
        // the join lock while it waits for this very callback to return.
        if self.thread_ids.contains(&thread::current().id()) {
            return;
        }
        let mut threads = self
            .threads
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for owned in threads.drain(..) {
            let _ = owned.join();
        }
        // An earlier pump failure set closed, but does not replace these joins.
        self.inner.runtime.shutdown();
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_pump(inner: Arc<Inner>, emit: Arc<dyn Fn(DataEvent) -> bool + Send + Sync>) {
    // A rejected callback or an unwind must close admission and cancel the
    // loader as well as queries. Joining belongs to Service::shutdown.
    let _exit = ThreadExit(Arc::clone(&inner));
    let waker = Waker::from(Arc::clone(&inner.signal));
    let mut context = Context::from_waker(&waker);
    let mut streams = VecDeque::<(Arc<Request>, ReplyChunks)>::new();
    while !inner.closed.load(Ordering::Acquire) {
        let observed = inner.signal.version.load(Ordering::Acquire);
        let mut pending = inner
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let ids: Vec<_> = pending.queries.keys().cloned().collect();
        for id in ids {
            let result = pending.queries.get_mut(&id).and_then(|query| {
                match query.subscription.as_mut().poll(&mut context) {
                    Poll::Ready(result) => Some(result),
                    Poll::Pending => None,
                }
            });
            if let Some(result) = result {
                let query = pending
                    .queries
                    .remove(&id)
                    .expect("polled query remains pending");
                let request = Arc::clone(&query.request);
                match result {
                    Ok(result) => pending.ready.push_back((request, Payload::Query(result))),
                    Err(error) => pending.errors.push_back((request, error.to_string())),
                }
            }
        }
        streams.retain(|(request, _)| !request.cancelled());
        let error = pending.errors.pop_front();
        let serialized_bytes: usize = streams
            .iter()
            .map(|(_, chunks)| chunks.retained_bytes())
            .sum();
        let ready = if serialized_bytes <= MAX_SERIALIZED_BYTES - MAX_REPLY_BYTES - 128 {
            pending.ready.pop_front()
        } else {
            None
        };
        drop(pending);
        let mut progressed = false;
        if let Some((request, message)) = error {
            if !emit_error(&inner, &request, message, &*emit) {
                return;
            }
            progressed = true;
        }
        if let Some((request, payload)) = ready {
            let response = payload
                .serialize(&request)
                .and_then(|json| ReplyChunks::new(request.id.clone(), json));
            drop(payload);
            // Cancellation can arrive while the serializer is outside the
            // pending lock. Never publish its output on returning to the pump.
            if request.cancelled() || inner.closed.load(Ordering::Acquire) {
                inner.finish(&request, false);
            } else {
                match response {
                    Ok(chunks)
                        if serialized_bytes.saturating_add(chunks.retained_bytes())
                            <= MAX_SERIALIZED_BYTES =>
                    {
                        streams.push_back((request, chunks));
                    }
                    Ok(_) => {
                        if !emit_error(
                            &inner,
                            &request,
                            "Projected replies exceed the transport byte budget".into(),
                            &*emit,
                        ) {
                            return;
                        }
                    }
                    Err(message) => {
                        if !emit_error(&inner, &request, message, &*emit) {
                            return;
                        }
                    }
                }
            }
            progressed = true;
        }
        if let Some((request, mut chunks)) = streams.pop_front() {
            if request.cancelled() || inner.closed.load(Ordering::Acquire) {
                inner.finish(&request, false);
            } else if let Some(event) = chunks.next() {
                let last = matches!(&event, DataEvent::Chunk { last: true, .. });
                match deliver(&request, &inner.closed, event, &*emit) {
                    Delivery::Closed => return,
                    Delivery::Cancelled => inner.finish(&request, false),
                    Delivery::Sent if last => inner.finish(&request, true),
                    Delivery::Sent => streams.push_back((request, chunks)),
                }
            } else {
                // A stream must finish through its explicit last fragment.
                inner.finish(&request, false);
            }
            progressed = true;
        }
        if !progressed {
            let guard = inner
                .signal
                .mutex
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if !inner.closed.load(Ordering::Acquire)
                && inner.signal.version.load(Ordering::Acquire) == observed
            {
                drop(
                    inner
                        .signal
                        .changed
                        .wait(guard)
                        .unwrap_or_else(|error| error.into_inner()),
                );
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Delivery {
    Sent,
    Cancelled,
    Closed,
}

fn deliver(
    request: &Request,
    closed: &AtomicBool,
    event: DataEvent,
    emit: &(dyn Fn(DataEvent) -> bool + Send + Sync),
) -> Delivery {
    if request.cancelled() || closed.load(Ordering::Acquire) {
        return Delivery::Cancelled;
    }
    // No service lock crosses this interruptible, bounded handoff. A fragment
    // already handed to the callback is discarded by the host on cancellation.
    if !emit(event) {
        Delivery::Closed
    } else if request.cancelled() || closed.load(Ordering::Acquire) {
        Delivery::Cancelled
    } else {
        Delivery::Sent
    }
}

fn emit_error(
    inner: &Inner,
    request: &Arc<Request>,
    message: String,
    emit: &(dyn Fn(DataEvent) -> bool + Send + Sync),
) -> bool {
    // A source reply that cannot be serialized/delivered must not retain an
    // unreachable lease, even while its error waits for output capacity.
    let lease = inner
        .pending
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .rollback(request);
    drop(lease);
    let delivery = deliver(
        request,
        &inner.closed,
        DataEvent::Error {
            id: request.id.clone(),
            message: bounded_text(&message, 2048),
        },
        emit,
    );
    inner.finish(request, false);
    delivery != Delivery::Closed
}

struct JsonBuffer<'a> {
    bytes: Vec<u8>,
    cancelled: &'a AtomicBool,
}

impl Write for JsonBuffer<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.cancelled.load(Ordering::Acquire) {
            // Write::write_all retries Interrupted; cancellation must terminate
            // serialization instead of entering that retry loop.
            return Err(io::Error::other("Dashboard request was cancelled"));
        }
        let length = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|length| *length <= MAX_REPLY_BYTES)
            .ok_or_else(|| {
                io::Error::other("Projected response exceeds the transport byte budget")
            })?;
        if length > self.bytes.capacity() {
            let capacity = length
                .max(self.bytes.capacity().saturating_mul(2))
                .min(MAX_REPLY_BYTES);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(|_| io::Error::other("Not enough memory for projected response"))?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn bounded_json(value: &impl Serialize, cancelled: &AtomicBool) -> Result<String, String> {
    let mut output = JsonBuffer {
        bytes: Vec::new(),
        cancelled,
    };
    serde_json::to_writer(&mut output, value).map_err(|error| error.to_string())?;
    if cancelled.load(Ordering::Acquire) {
        return Err("Dashboard request was cancelled".into());
    }
    String::from_utf8(output.bytes).map_err(|error| error.to_string())
}

fn bounded_text(value: &str, maximum: usize) -> String {
    let mut end = value.len().min(maximum);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use replay::{
        container::Replay,
        identity::{Identity, Roster, SampleBoundary, SessionStart, Verification},
        spool::Record,
    };
    use std::time::Duration;

    /// A valid in-memory source needs a session identity, but no map assets,
    /// recorded tables or file I/O for these ownership tests.
    pub(crate) fn source(runtime: &Arc<AnalyticsRuntime>) -> Arc<SharedSource> {
        let session = SessionStart {
            id: 1,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 10_000_000,
            sample_boundary: SampleBoundary::EndOfHour,
            identity: Identity {
                game_version: "synthetic".into(),
                exe_hash: [1; 32],
                exe_size: 1,
                vanilla_hash: [2; 32],
                dlc: vec![],
                mods: vec![],
                coverage_policy: 1,
                language: "l_english".into(),
                recorder_version: "test".into(),
                collector_hash: [3; 32],
                verification: Verification::Verified {
                    launcher: Roster::default(),
                    memory: Roster::default(),
                },
            },
        };
        let replay = Arc::new(
            Replay::from_records(
                b"dashboard-lease-test".to_vec(),
                &[vec![Record::SessionStart(session)]],
            )
            .unwrap(),
        );
        let key = SourceKey {
            source_id: "source-test".into(),
            revision: "revision-test".into(),
        };
        let path = PathBuf::from("memory.battleplan");
        let handle = runtime
            .register_source(key.clone(), path.clone(), replay)
            .unwrap();
        Arc::new(SharedSource {
            flags: Mutex::new(None),
            land_owners: Mutex::new(None),
            handle,
            key,
            path,
            runtime: Arc::clone(runtime),
        })
    }

    #[test]
    fn cancellation_survives_serialized_output_tombstone_eviction_and_id_reuse() {
        let mut pending = Pending::default();
        let request = pending.begin("open-1".into()).unwrap();
        let job = OpenJob {
            request: Arc::clone(&request),
            path: "unused".into(),
            revision: "revision".into(),
        };
        let mut chunks = ReplyChunks::new(request.id.clone(), "{\"ready\":true}".into()).unwrap();
        drop(pending.cancel(&request.id));
        assert!(pending.begin(request.id.clone()).is_none());
        for index in 0..MAX_CANCELLED + 1 {
            drop(pending.cancel(&format!("unrelated-{index}")));
        }
        assert_eq!(pending.cancelled.len(), MAX_CANCELLED);
        let replacement = pending.begin(request.id.clone()).unwrap();
        assert!(job.request.cancelled());
        assert!(!pending.current(&job.request));
        drop(pending.finish(&job.request, false));
        assert!(pending.current(&replacement));
        assert_eq!(
            deliver(
                &request,
                &AtomicBool::new(false),
                chunks.next().unwrap(),
                &|_| { panic!("A cancelled serialized reply reached the callback") }
            ),
            Delivery::Cancelled
        );
    }

    #[test]
    fn completed_replies_still_occupy_admission_until_delivery() {
        let mut pending = Pending::default();
        let requests: Vec<_> = (0..MAX_PENDING)
            .map(|index| pending.begin(format!("reply-{index}")).unwrap())
            .collect();
        // These tokens may all be with a serializer/stream rather than a query.
        assert!(pending.queries.is_empty());
        assert!(pending.begin("over-capacity".into()).is_none());
        let (rejection, _) = pending.errors.pop_front().unwrap();
        drop(pending.finish(&rejection, false));
        drop(pending.finish(&requests[0], true));
        assert!(pending.begin("replacement".into()).is_some());
        for index in 0..MAX_PENDING * 3 {
            let _ = pending.begin(format!("overflow-{index}"));
        }
        assert_eq!(pending.requests.len(), MAX_PENDING * 2);
        assert_eq!(pending.errors.len(), MAX_PENDING);
    }

    #[test]
    fn cancellation_interrupts_serialization_without_retrying_the_writer() {
        struct CancelDuringSerialization<'a>(&'a AtomicBool);
        impl Serialize for CancelDuringSerialization<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                use serde::ser::SerializeSeq;
                let mut sequence = serializer.serialize_seq(Some(2))?;
                sequence.serialize_element("first")?;
                self.0.store(true, Ordering::Release);
                sequence.serialize_element("cancelled")?;
                sequence.end()
            }
        }
        let cancelled = AtomicBool::new(false);
        let error = bounded_json(&CancelDuringSerialization(&cancelled), &cancelled).unwrap_err();
        assert!(error.contains("cancelled"));
        let mut buffer = JsonBuffer {
            bytes: Vec::new(),
            cancelled: &cancelled,
        };
        assert_ne!(
            buffer.write(b"x").unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert!(buffer.bytes.is_empty());
    }

    #[test]
    fn cancellation_during_handoff_stops_following_fragments() {
        let request = Request::new("reply-1".into());
        let json = bounded_json(&"x".repeat(20_000), &request.cancelled).unwrap();
        let mut chunks = ReplyChunks::new(request.id.clone(), json).unwrap();
        let closed = AtomicBool::new(false);
        assert_eq!(
            deliver(&request, &closed, chunks.next().unwrap(), &|_| {
                request.cancel();
                true
            }),
            Delivery::Cancelled
        );
        assert_eq!(
            deliver(&request, &closed, chunks.next().unwrap(), &|_| {
                panic!("A later fragment followed cancellation during handoff")
            }),
            Delivery::Cancelled
        );
    }

    #[test]
    fn stale_release_cannot_remove_a_lease_from_a_restarted_service() {
        let old = Service::new(Arc::new(|_| true)).unwrap();
        let stale_id = old.inner.lease_id(1);
        old.shutdown();
        let restarted = Service::new(Arc::new(|_| true)).unwrap();
        let current_id = restarted.inner.lease_id(1);
        assert_ne!(stale_id, current_id);
        let source = source(&restarted.inner.runtime);
        let request = restarted.inner.begin("new-open".into()).unwrap();
        restarted
            .inner
            .pending
            .lock()
            .unwrap()
            .source_requests
            .insert(request.id.clone(), current_id.clone());
        restarted
            .inner
            .pending
            .lock()
            .unwrap()
            .leases
            .insert(current_id.clone(), Lease { request, source });
        restarted.dispatch(DataCommand::Release {
            source_id: stale_id,
        });
        assert!(restarted.source(&current_id).is_ok());
        restarted.dispatch(DataCommand::Release {
            source_id: current_id.clone(),
        });
        assert!(restarted.source(&current_id).is_err());
        restarted.shutdown();
    }

    #[test]
    fn failed_source_reply_rolls_back_before_callback_and_preserves_sibling_lease() {
        let service = Service::new(Arc::new(|_| true)).unwrap();
        let source = source(&service.inner.runtime);
        let source_witness = Arc::downgrade(&source);
        let request = service.inner.begin("open-1".into()).unwrap();
        let sibling = service.inner.begin("open-2".into()).unwrap();
        {
            let mut pending = service.inner.pending.lock().unwrap();
            pending
                .source_requests
                .insert(request.id.clone(), "lease-1".into());
            pending.leases.insert(
                "lease-1".into(),
                Lease {
                    request: Arc::clone(&request),
                    source: Arc::clone(&source),
                },
            );
            pending
                .source_requests
                .insert(sibling.id.clone(), "lease-2".into());
            pending.leases.insert(
                "lease-2".into(),
                Lease {
                    request: Arc::clone(&sibling),
                    source,
                },
            );
        }
        assert!(emit_error(
            &service.inner,
            &request,
            "Serialization failed".into(),
            &|event| {
                assert!(matches!(event, DataEvent::Error { .. }));
                assert_eq!(
                    source_witness.strong_count(),
                    1,
                    "only the sibling lease may remain at handoff"
                );
                true
            }
        ));
        assert!(
            !service
                .inner
                .pending
                .lock()
                .unwrap()
                .source_requests
                .contains_key(&request.id)
        );
        assert!(service.source("lease-1").is_err());
        assert!(service.source("lease-2").is_ok());
        assert_eq!(service.inner.runtime.stats().sources, 1);
        // Explicitly releasing a not-yet-delivered source also cancels its reply.
        service.dispatch(DataCommand::Release {
            source_id: "lease-2".into(),
        });
        assert!(sibling.cancelled());
        assert!(service.source("lease-2").is_err());
        assert_eq!(service.inner.runtime.stats().sources, 0);
        service.shutdown();
    }

    #[test]
    fn failed_delivery_stops_admission_and_shutdown_still_joins() {
        let (called, receive) = mpsc::sync_channel(1);
        let service = Service::new(Arc::new(move |_| {
            called.send(()).unwrap();
            false
        }))
        .unwrap();
        let source = source(&service.inner.runtime);
        let request = service.inner.begin("open-1".into()).unwrap();
        let reply = SourceReply {
            id: "lease-1".into(),
            path: "memory.battleplan".into(),
            revision: source.key().revision.clone(),
            metadata: source.metadata().clone(),
        };
        {
            let mut pending = service.inner.pending.lock().unwrap();
            pending
                .source_requests
                .insert(request.id.clone(), reply.id.clone());
            pending.leases.insert(
                reply.id.clone(),
                Lease {
                    request: Arc::clone(&request),
                    source,
                },
            );
            pending
                .ready
                .push_back((Arc::clone(&request), Payload::Source(Box::new(reply))));
        }
        service.inner.signal.notify();
        receive.recv_timeout(Duration::from_secs(5)).unwrap();
        {
            let guard = service.inner.signal.mutex.lock().unwrap();
            let (guard, _) = service
                .inner
                .signal
                .changed
                .wait_timeout_while(guard, Duration::from_secs(5), |_| {
                    !service.inner.closed.load(Ordering::Acquire)
                })
                .unwrap();
            assert!(service.inner.closed.load(Ordering::Acquire));
            drop(guard);
        }
        assert!(service.inner.begin("after-close".into()).is_none());
        service.shutdown();
        assert!(request.cancelled());
        assert!(service.threads.lock().unwrap().is_empty());
        assert!(service.inner.loader.lock().unwrap().is_none());
        let pending = service.inner.pending.lock().unwrap();
        assert!(pending.requests.is_empty());
        assert!(pending.queries.is_empty());
        assert!(pending.leases.is_empty());
        assert!(pending.source_requests.is_empty());
        assert!(pending.ready.is_empty());
        assert!(pending.errors.is_empty());
        drop(pending);
        assert_eq!(service.inner.runtime.stats().sources, 0);
    }
}
