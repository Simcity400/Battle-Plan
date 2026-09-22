//! The native data service: one owner of opened recordings, a small loader
//! pool that answers product requests, and a bounded response pump.
//!
//! Opening files and answering products never depends on a visible window.
//! Only compact JSON replies cross the desktop control pipe.

use crate::{
    Config, Source, SourceKey, map_settings,
    products::{self, ContentOptions, LoadContext},
    protocol::{CampaignMapSettings, replay_revision},
    transport::{
        DataCommand, DataEvent, MAX_REPLY_BYTES, MapSettingsTarget, ProductRequest, ReplyChunks,
        SourceReply, bounded_json, bounded_text,
    },
};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

const MAX_PENDING: usize = 64;
const MAX_SOURCE_LEASES: usize = 64;
const MAX_SERIALIZED_BYTES: usize = MAX_REPLY_BYTES * 2;
const MAX_CANCELLED: usize = MAX_PENDING * 2;
const LOADER_THREADS: usize = 3;

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

#[derive(Default)]
struct Sources {
    next: u64,
    entries: BTreeMap<(PathBuf, String), Weak<Source>>,
}

struct Lease {
    request: Arc<Request>,
    source: Arc<Source>,
}

#[derive(Default)]
struct Pending {
    // Admission includes completed replies until their final fragment is sent.
    requests: BTreeMap<String, Arc<Request>>,
    leases: BTreeMap<String, Lease>,
    source_requests: BTreeMap<String, String>,
    ready: VecDeque<(Arc<Request>, String)>,
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

    fn cancel(&mut self, id: &str) -> Option<Lease> {
        if !self.cancelled.iter().any(|cancelled| cancelled == id) {
            if self.cancelled.len() == MAX_CANCELLED {
                self.cancelled.pop_front();
            }
            self.cancelled.push_back(id.to_owned());
        }
        if let Some(request) = self.requests.remove(id) {
            request.cancel();
        }
        self.ready.retain(|(request, _)| request.id != id);
        self.errors.retain(|(request, _)| request.id != id);
        let lease = self
            .source_requests
            .remove(id)
            .and_then(|lease_id| self.leases.remove(&lease_id));
        if let Some(lease) = &lease {
            lease.request.cancel();
        }
        lease
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

    /// Queue a finished job's result if its request is still current.
    fn complete(&mut self, request: Arc<Request>, result: Result<String, String>) {
        if !self.current(&request) {
            return;
        }
        match result {
            Ok(json) => self.ready.push_back((request, json)),
            Err(error) => self.errors.push_back((request, bounded_text(&error, 2048))),
        }
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

type VisualCatalogue = (PathBuf, PathBuf, Vec<installed_content::content::visual::VisualMod>);

struct Inner {
    lease_namespace: uuid::Uuid,
    map_settings: Mutex<map_settings::Store>,
    content: Mutex<Option<ContentOptions>>,
    visual_catalogue: Mutex<Option<VisualCatalogue>>,
    sources: Mutex<Sources>,
    pending: Mutex<Pending>,
    loader: Mutex<Option<Sender<LoadJob>>>,
    signal: Arc<Signal>,
    closed: AtomicBool,
}

impl Inner {
    fn lease_id(&self, sequence: u64) -> String {
        format!("lease-{}-{sequence}", self.lease_namespace)
    }

    fn content(&self) -> Option<ContentOptions> {
        self.content
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// The source lock serializes cold opens, including duplicate map requests.
    fn open(
        &self,
        path: &Path,
        expected: Option<&str>,
        cancelled: &AtomicBool,
    ) -> Result<Arc<Source>, String> {
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
        let source = Arc::new(
            Source::open(key, path.clone(), cancelled).map_err(|error| error.to_string())?,
        );
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
        self.complete(request, Err(message));
    }

    fn complete(&self, request: Arc<Request>, result: Result<String, String>) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !self.closed.load(Ordering::Acquire) {
            pending.complete(request, result);
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

    fn submit(&self, request: Arc<Request>, job: LoadJob) {
        let accepted = self
            .loader
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .is_some_and(|loader| loader.send(job).is_ok());
        if !accepted {
            self.error(request, "Dashboard data service is closing".into());
        }
    }

    /// Terminal cleanup is safe on any service thread. It never joins a
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
        self.signal.notify();
    }

    fn map_settings_job(
        &self,
        target: MapSettingsTarget,
        request: &Request,
        source_id: String,
        source: Option<Arc<Source>>,
        settings: Option<CampaignMapSettings>,
        options: installed_content::ResolveOptions,
    ) -> Result<String, String> {
        if request.cancelled() {
            return Err("Cancelled".into());
        }
        let replay = source.as_ref().map(|source| source.replay());
        let campaign = replay
            .as_ref()
            .map(|replay| map_settings::campaign_key(replay))
            .unwrap_or_default();
        let key = target.key(&campaign).map_err(str::to_owned)?;
        let mut settings = settings;
        {
            let mut store = self.map_settings.lock().map_err(|e| e.to_string())?;
            store.configure_game(&options.user_dir);
            if let Some(settings) = &mut settings
                && settings.use_game_graphics
            {
                if matches!(target, MapSettingsTarget::Global) && store.game_graphics().is_none() {
                    return Err(store
                        .game_graphics_error()
                        .unwrap_or_else(|| "the game graphics settings unavailable".into()));
                }
                settings.graphics = store.inherited(&key).graphics;
            }
        }
        let mods = {
            let mut catalogue = self
                .visual_catalogue
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if settings.is_none()
                || catalogue.as_ref().is_none_or(|(game, user, _)| {
                    game != &options.game_dir || user != &options.user_dir
                })
            {
                let mods = map_settings::catalogue(&options, &request.cancelled)
                    .map_err(|e| e.to_string())?;
                *catalogue = Some((options.game_dir.clone(), options.user_dir.clone(), mods));
            }
            catalogue.as_ref().expect("catalogue loaded").2.clone()
        };
        if let Some(settings) = &settings {
            settings.validate().map_err(str::to_owned)?;
            if !map_settings::sample_counts().contains(&settings.graphics.samples) {
                return Err(
                    "This anti-aliasing setting is unavailable on the current renderer".into(),
                );
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
        let mut store = self.map_settings.lock().map_err(|e| e.to_string())?;
        store.check()?;
        if let Some(settings) = settings {
            store
                .save(key.clone(), settings)
                .map_err(|e| e.to_string())?;
        }
        let recorded_mods = replay
            .as_ref()
            .into_iter()
            .flat_map(|replay| replay.identity.mods.iter())
            .filter_map(|recorded| {
                mods.iter()
                    .find(|item| item.recorded_id == recorded.id && item.reason.is_none())
                    .map(|item| item.id.clone())
            })
            .collect();
        let reply = map_settings::Reply {
            source_id,
            revision: source
                .as_ref()
                .map(|source| source.key().revision.clone())
                .unwrap_or_default(),
            settings: store.preferences(&key),
            inherited_settings: store.inherited(&key),
            mods,
            recorded_mods,
            sample_counts: map_settings::sample_counts(),
            game_graphics: store.game_graphics(),
            game_graphics_error: store.game_graphics_error(),
        };
        bounded_json(&reply, &request.cancelled)
    }

    fn run_loader(self: &Arc<Self>, jobs: &Mutex<Receiver<LoadJob>>) {
        loop {
            let job = {
                let receiver = jobs.lock().unwrap_or_else(|error| error.into_inner());
                receiver.recv()
            };
            let Ok(job) = job else {
                return;
            };
            if self.closed.load(Ordering::Acquire) {
                return;
            }
            match job {
                LoadJob::MapSettings {
                    target,
                    request,
                    source_id,
                    source,
                    settings,
                    options,
                } => {
                    let result = self
                        .map_settings_job(target, &request, source_id, source, settings, options);
                    self.complete(request, result);
                }
                LoadJob::Product {
                    request,
                    lease_id,
                    source,
                    body,
                } => {
                    if request.cancelled() {
                        continue;
                    }
                    let Some(product) = products::find(&body.product) else {
                        self.error(request, format!("Unknown data product: {}", body.product));
                        continue;
                    };
                    let context = LoadContext {
                        content: self.content(),
                    };
                    let result = (product.load)(
                        &source,
                        &lease_id,
                        body.params,
                        &context,
                        &request.cancelled,
                    );
                    self.complete(request, result);
                }
                LoadJob::Open(job) => {
                    if let Some(content) = self.content() {
                        self.map_settings
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .configure_game(&content.options.user_dir);
                    }
                    self.open_job(job);
                }
            }
        }
    }

    fn open_job(&self, job: OpenJob) {
        if job.request.cancelled() {
            return;
        }
        let result = self.open(
            Path::new(&job.path),
            Some(&job.revision),
            &job.request.cancelled,
        );
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !pending.current(&job.request) || self.closed.load(Ordering::Acquire) {
            drop(pending);
            drop(result);
            self.finish(&job.request, false);
            return;
        }
        match result {
            Ok(source) if pending.leases.len() < MAX_SOURCE_LEASES => {
                if let Some(next) = pending.next_lease.checked_add(1) {
                    pending.next_lease = next;
                    let id = self.lease_id(next);
                    let reply = SourceReply {
                        id: id.clone(),
                        path: source.path().to_string_lossy().into_owned(),
                        revision: source.key().revision.clone(),
                        metadata: source.metadata().clone(),
                    };
                    let json = bounded_json(&reply, &job.request.cancelled);
                    match json {
                        Ok(json) => {
                            pending.leases.insert(
                                id.clone(),
                                Lease {
                                    request: Arc::clone(&job.request),
                                    source,
                                },
                            );
                            pending.source_requests.insert(job.request.id.clone(), id);
                            pending.ready.push_back((job.request, json));
                        }
                        Err(error) => pending
                            .errors
                            .push_back((job.request, bounded_text(&error, 2048))),
                    }
                } else {
                    pending
                        .errors
                        .push_back((job.request, "Source lease identity exhausted".into()));
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
        target: MapSettingsTarget,
        request: Arc<Request>,
        source_id: String,
        source: Option<Arc<Source>>,
        settings: Option<CampaignMapSettings>,
        options: installed_content::ResolveOptions,
    },
    Open(OpenJob),
    Product {
        request: Arc<Request>,
        lease_id: String,
        source: Arc<Source>,
        body: ProductRequest,
    },
}

pub struct Service {
    inner: Arc<Inner>,
    thread_ids: Vec<thread::ThreadId>,
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
        let (loader, jobs) = mpsc::channel::<LoadJob>();
        let jobs = Arc::new(Mutex::new(jobs));
        let inner = Arc::new(Inner {
            lease_namespace: uuid::Uuid::new_v4(),
            map_settings: Mutex::new(map_settings::Store::open()),
            content: Mutex::new(None),
            visual_catalogue: Mutex::new(None),
            sources: Mutex::new(Sources::default()),
            pending: Mutex::new(Pending::default()),
            loader: Mutex::new(Some(loader)),
            signal: Arc::new(Signal::default()),
            closed: AtomicBool::new(false),
        });
        let mut threads = Vec::new();
        let spawn_result = (|| -> io::Result<()> {
            for index in 0..LOADER_THREADS {
                let load_inner = Arc::clone(&inner);
                let jobs = Arc::clone(&jobs);
                threads.push(
                    thread::Builder::new()
                        .name(format!("dashboard loader {index}"))
                        .spawn(move || {
                            let _exit = ThreadExit(Arc::clone(&load_inner));
                            load_inner.run_loader(&jobs);
                        })?,
                );
            }
            let pump_inner = Arc::clone(&inner);
            threads.push(
                thread::Builder::new()
                    .name("dashboard response pump".into())
                    .spawn(move || run_pump(pump_inner, emit))?,
            );
            Ok(())
        })();
        if let Err(error) = spawn_result {
            inner.stop();
            for thread in threads {
                let _ = thread.join();
            }
            return Err(error);
        }
        let thread_ids = threads.iter().map(|thread| thread.thread().id()).collect();
        Ok(Arc::new(Self {
            inner,
            thread_ids,
            threads: Mutex::new(threads),
        }))
    }

    /// Game and user folders for products that read artwork or localisation.
    pub fn configure_content(&self, config: &Config, allow_unverified: bool) {
        self.inner
            .map_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .configure_game(&config.user_dir);
        *self
            .inner
            .content
            .lock()
            .unwrap_or_else(|error| error.into_inner()) =
            config.resolve_options().ok().map(|options| ContentOptions {
                options,
                allow_unverified,
            });
    }

    pub fn map_settings(&self, replay: &replay::container::Replay) -> CampaignMapSettings {
        self.inner
            .map_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&map_settings::campaign_key(replay))
    }

    pub fn widget_map_settings(
        &self,
        replay: &replay::container::Replay,
        id: &str,
    ) -> CampaignMapSettings {
        self.inner
            .map_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&format!(
                "{}/widget/{id}",
                map_settings::campaign_key(replay)
            ))
    }

    pub fn source(&self, id: &str) -> Result<Arc<Source>, String> {
        self.inner
            .pending
            .lock()
            .map_err(|error| error.to_string())?
            .leases
            .get(id)
            .map(|lease| Arc::clone(&lease.source))
            .ok_or_else(|| "The map's recording binding has been released".into())
    }

    /// A native map opens or shares a source directly, without a lease.
    pub fn open_map(&self, path: &Path, revision: Option<&str>) -> Result<Arc<Source>, String> {
        self.inner.open(path, revision, &self.inner.closed)
    }

    fn leased_source(
        &self,
        request: &Arc<Request>,
        source_id: &str,
        revision: &str,
        what: &str,
    ) -> Option<Arc<Source>> {
        match self.source(source_id) {
            Ok(source) if source.key().revision == revision => Some(source),
            _ => {
                self.inner.error(
                    Arc::clone(request),
                    format!("{what} source is unavailable or changed"),
                );
                None
            }
        }
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
                let source = if matches!(target, MapSettingsTarget::Global) {
                    if !source_id.is_empty() || !revision.is_empty() {
                        self.inner
                            .error(request, "Global settings do not take a campaign".into());
                        return;
                    }
                    None
                } else {
                    let Some(source) =
                        self.leased_source(&request, &source_id, &revision, "Campaign")
                    else {
                        return;
                    };
                    Some(source)
                };
                let Some(content) = self.inner.content() else {
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
                    options: content.options,
                };
                self.inner.submit(request, job);
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
                self.inner.submit(request, LoadJob::Open(job));
            }
            DataCommand::Product { id, request: body } => {
                let Some(request) = self.inner.begin(id) else {
                    return;
                };
                if let Err(error) = body.validate() {
                    self.inner.error(request, error.into());
                    return;
                }
                if products::find(&body.product).is_none() {
                    self.inner
                        .error(request, format!("Unknown data product: {}", body.product));
                    return;
                }
                let Some(source) =
                    self.leased_source(&request, &body.source_id, &body.revision, "Product")
                else {
                    return;
                };
                let job = LoadJob::Product {
                    request: Arc::clone(&request),
                    lease_id: body.source_id.clone(),
                    source,
                    body,
                };
                self.inner.submit(request, job);
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
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_pump(inner: Arc<Inner>, emit: Arc<dyn Fn(DataEvent) -> bool + Send + Sync>) {
    // A rejected callback or an unwind must close admission and stop the
    // loaders. Joining belongs to Service::shutdown.
    let _exit = ThreadExit(Arc::clone(&inner));
    let mut streams = VecDeque::<(Arc<Request>, ReplyChunks)>::new();
    while !inner.closed.load(Ordering::Acquire) {
        let observed = inner.signal.version.load(Ordering::Acquire);
        let mut pending = inner
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
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
        if let Some((request, json)) = ready {
            if request.cancelled() || inner.closed.load(Ordering::Acquire) {
                inner.finish(&request, false);
            } else {
                match ReplyChunks::new(request.id.clone(), json) {
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
    // A source reply that cannot be delivered must not retain an unreachable
    // lease, even while its error waits for output capacity.
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

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{
        container::Replay,
        identity::{Identity, Roster, SampleBoundary, SessionStart, Verification},
        spool::Record,
    };
    use std::time::Duration;

    /// A valid in-memory source needs a session identity, but no map assets,
    /// recorded tables or file I/O for these ownership tests.
    fn source() -> Arc<Source> {
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
        Arc::new(
            Source::from_replay(
                key,
                PathBuf::from("memory.battleplan"),
                replay,
                &AtomicBool::new(false),
            )
            .unwrap(),
        )
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
    fn late_results_for_cancelled_requests_are_dropped() {
        let mut pending = Pending::default();
        let request = pending.begin("product-1".into()).unwrap();
        drop(pending.cancel(&request.id));
        pending.complete(Arc::clone(&request), Ok("{}".into()));
        assert!(pending.ready.is_empty());
        let current = pending.begin("product-2".into()).unwrap();
        pending.complete(Arc::clone(&current), Err("failed".into()));
        assert_eq!(pending.errors.len(), 1);
        assert!(pending.ready.is_empty());
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
        let source = source();
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
    fn unknown_products_and_unleased_sources_fail_before_loading() {
        let (sent, receive) = mpsc::channel();
        let service = Service::new(Arc::new(move |event| sent.send(event).is_ok())).unwrap();
        service.dispatch(DataCommand::Product {
            id: "p-1".into(),
            request: ProductRequest {
                source_id: "lease-missing".into(),
                revision: "r".into(),
                product: "national-focus".into(),
                params: serde_json::json!({}),
            },
        });
        service.dispatch(DataCommand::Product {
            id: "p-2".into(),
            request: ProductRequest {
                source_id: "lease-missing".into(),
                revision: "r".into(),
                product: "no-such-product".into(),
                params: serde_json::json!({}),
            },
        });
        let mut errors = Vec::new();
        for _ in 0..2 {
            match receive.recv_timeout(Duration::from_secs(5)).unwrap() {
                DataEvent::Error { id, message } => errors.push((id, message)),
                other => panic!("unexpected event {other:?}"),
            }
        }
        errors.sort();
        assert!(errors[0].1.contains("unavailable or changed"));
        assert!(errors[1].1.contains("Unknown data product"));
        service.shutdown();
    }

    #[test]
    fn failed_source_reply_rolls_back_before_callback_and_preserves_sibling_lease() {
        let service = Service::new(Arc::new(|_| true)).unwrap();
        let source = source();
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
        // Explicitly releasing a not-yet-delivered source also cancels its reply.
        service.dispatch(DataCommand::Release {
            source_id: "lease-2".into(),
        });
        assert!(sibling.cancelled());
        assert!(service.source("lease-2").is_err());
        assert_eq!(source_witness.strong_count(), 0);
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
        let source = source();
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
                .push_back((Arc::clone(&request), serde_json::to_string(&reply).unwrap()));
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
        assert!(pending.leases.is_empty());
        assert!(pending.source_requests.is_empty());
        assert!(pending.ready.is_empty());
        assert!(pending.errors.is_empty());
    }
}
