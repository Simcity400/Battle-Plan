//! The native data service: one owner of opened recordings, loader pools
//! (lanes) that answer product requests, and a bounded response pump.
//!
//! Opening files and answering products never depends on a visible window.
//! Only compact JSON replies cross the desktop control pipe.

use crate::{
    Config, Source, map_settings,
    products::{self, ContentOptions},
    protocol::ResolvedMapSettings,
    transport::{DataCommand, DataEvent, MAX_REPLY_BYTES, MapSettingsTarget},
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
};

mod admission;
mod jobs;
mod pump;

use admission::{Lease, Pending, Request};
use jobs::{LoadJob, MapSettingsJob, OpenJob, Sources};
use pump::{Signal, run_pump};

const MAX_PENDING: usize = 64;
const MAX_SERIALIZED_BYTES: usize = MAX_REPLY_BYTES * 2;
const MAX_CANCELLED: usize = MAX_PENDING * 2;
/// The fewest whole-recording and sampled-read loaders on any machine.
const MIN_LOADER_THREADS: usize = 3;
/// Threads for `Product::INTERACTIVE` reads, apart from the scans.
const INTERACTIVE_THREADS: usize = 2;
/// Threads answering prepared queries from prepared pages alone.
const PREPARED_THREADS: usize = 2;

/// Where a job waits. Each lane has its own threads and its own bound of
/// [`MAX_PENDING`] queued jobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Lane {
    /// Whole-recording products, opening, map settings and preparation refreshes.
    History,
    /// Hover and label reads, the history cache's size and preparation actions.
    Interactive,
    /// Every prepared query first, answered without decoding recorded blocks;
    /// one that needs a block decode moves to [`Lane::Sampled`].
    Prepared,
    /// Prepared queries that sample recorded blocks.
    Sampled,
}

impl Lane {
    const ALL: [Self; 4] = [
        Self::History,
        Self::Interactive,
        Self::Prepared,
        Self::Sampled,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::History => "loader",
            Self::Interactive => "interactive",
            Self::Prepared => "prepared",
            Self::Sampled => "sampled",
        }
    }

    /// Threads serving this lane on a machine with `parallelism` hardware
    /// threads. Sampled reads take half of them, whole-recording loads a
    /// quarter, each at least [`MIN_LOADER_THREADS`]; the short lanes are fixed.
    /// A sampled read holds one block's selected frames and the few read ahead
    /// of it, so memory grows with this count, not with the recording.
    pub(super) fn threads(self, parallelism: usize) -> usize {
        match self {
            Self::History => (parallelism / 4).max(MIN_LOADER_THREADS),
            Self::Sampled => (parallelism / 2).max(MIN_LOADER_THREADS),
            Self::Interactive => INTERACTIVE_THREADS,
            Self::Prepared => PREPARED_THREADS,
        }
    }
}

/// The machine's hardware threads, as the lanes are sized from them.
fn parallelism() -> usize {
    thread::available_parallelism().map_or(1, |threads| threads.get())
}

/// Where jobs wait, one queue per [`Lane`].
struct Lanes {
    history: SyncSender<LoadJob>,
    interactive: SyncSender<LoadJob>,
    prepared: SyncSender<LoadJob>,
    sampled: SyncSender<LoadJob>,
    refreshes: std::collections::BTreeMap<crate::SourceKey, Arc<AtomicBool>>,
}

impl Lanes {
    fn sender(&self, lane: Lane) -> &SyncSender<LoadJob> {
        match lane {
            Lane::History => &self.history,
            Lane::Interactive => &self.interactive,
            Lane::Prepared => &self.prepared,
            Lane::Sampled => &self.sampled,
        }
    }
}

type VisualCatalogue = (
    PathBuf,
    PathBuf,
    Vec<installed_content::content::visual::InstalledMod>,
);

struct Inner {
    lease_namespace: uuid::Uuid,
    map_settings: Mutex<map_settings::Store>,
    content: Mutex<Option<ContentOptions>>,
    visual_catalogue: Mutex<Option<VisualCatalogue>>,
    sources: Mutex<Sources>,
    pending: Mutex<Pending>,
    loader: Mutex<Option<Lanes>>,
    signal: Arc<Signal>,
    closed: AtomicBool,
    preparation: crate::prepared::runner::Runner,
    preparation_actions: Mutex<()>,
    background_preparation: AtomicBool,
    preparation_suspended: AtomicBool,
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
        crate::prepared::pipeline::install_gates();
        let (history, history_jobs) = mpsc::sync_channel::<LoadJob>(MAX_PENDING);
        let (interactive, interactive_jobs) = mpsc::sync_channel::<LoadJob>(MAX_PENDING);
        let (prepared, prepared_jobs) = mpsc::sync_channel::<LoadJob>(MAX_PENDING);
        let (sampled, sampled_jobs) = mpsc::sync_channel::<LoadJob>(MAX_PENDING);
        let parallelism = parallelism();
        let inner = Arc::new(Inner {
            lease_namespace: uuid::Uuid::new_v4(),
            map_settings: Mutex::new(map_settings::Store::open()),
            content: Mutex::new(None),
            visual_catalogue: Mutex::new(None),
            sources: Mutex::new(Sources::default()),
            pending: Mutex::new(Pending::default()),
            loader: Mutex::new(Some(Lanes {
                history,
                interactive,
                prepared,
                sampled,
                refreshes: std::collections::BTreeMap::new(),
            })),
            signal: Arc::new(Signal::default()),
            closed: AtomicBool::new(false),
            preparation: crate::prepared::runner::Runner::new()?,
            preparation_actions: Mutex::new(()),
            background_preparation: AtomicBool::new(true),
            preparation_suspended: AtomicBool::new(false),
        });
        let mut threads = Vec::new();
        let spawn_result = (|| -> io::Result<()> {
            for (lane, jobs) in Lane::ALL.into_iter().zip([
                history_jobs,
                interactive_jobs,
                prepared_jobs,
                sampled_jobs,
            ]) {
                let jobs = Arc::new(Mutex::new(jobs));
                for index in 0..lane.threads(parallelism) {
                    let load_inner = Arc::clone(&inner);
                    let jobs = Arc::clone(&jobs);
                    threads.push(
                        crate::prepared::runner::worker_thread(format!(
                            "dashboard {} {index}",
                            lane.name()
                        ))
                        .spawn(move || {
                            let _exit = ThreadExit(Arc::clone(&load_inner));
                            load_inner.run_loader(&jobs, lane);
                        })?,
                    );
                }
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
    pub fn configure_content(&self, config: &Config, allow_unverified: bool) -> io::Result<()> {
        self.set_background_preparation(config.background_preparation_enabled());
        let options = match config.resolve_options() {
            Ok(options) => options,
            Err(_) => {
                self.replace_content(None);
                return Ok(());
            }
        };
        if let Err(error) = engine::executable_tables::initialize(&options.game_dir) {
            self.replace_content(None);
            return Err(io::Error::new(
                error.kind(),
                format!("Game tables unavailable: {error}"),
            ));
        }
        self.inner
            .map_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .configure_game(&config.user_dir);
        self.replace_content(Some(ContentOptions {
            options,
            allow_unverified,
        }));
        Ok(())
    }

    fn replace_content(&self, content: Option<ContentOptions>) {
        let mut current = self
            .inner
            .content
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let key = |value: Option<&ContentOptions>| {
            value.map(|value| {
                (
                    crate::content::content_key(Some(value)),
                    value.options.cache_dir.clone(),
                )
            })
        };
        let changed = key(current.as_ref()) != key(content.as_ref());
        *current = content;
        drop(current);
        if !changed {
            return;
        }
        self.inner.preparation.notify_change();
        self.inner
            .sources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .completed
            .clear();
        let sources: Vec<_> = self
            .inner
            .sources
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entries
            .values()
            .filter_map(std::sync::Weak::upgrade)
            .collect();
        let mut latest = std::collections::BTreeMap::<PathBuf, Arc<Source>>::new();
        let mut revisions = std::collections::BTreeMap::new();
        for source in sources {
            let path = source.path().to_owned();
            let revision = revisions
                .entry(path.clone())
                .or_insert_with(|| crate::protocol::replay_revision(&path).ok());
            if revision.as_deref() == Some(source.key().revision.as_str()) {
                latest.insert(path, source);
            }
        }
        for source in latest.into_values() {
            // Manual Cancel and Clear remain effective until an explicit
            // Start or reopen. Refresh active/completed preparation off-thread.
            let progress = source.state::<crate::prepared::Progress>();
            let state = progress
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .state;
            if !matches!(
                state,
                crate::prepared::State::Idle | crate::prepared::State::Cancelled
            ) && let Err(error) = self.inner.refresh_preparation(source, true)
            {
                eprintln!("Preparation refresh: {error}");
            }
        }
    }

    pub fn preparation_preferences(&self) -> crate::prepared::Preferences {
        self.inner.preparation_preferences()
    }

    pub fn set_background_preparation(&self, enabled: bool) {
        self.inner.set_background_preparation(enabled);
    }

    /// Suspend automatic admission while the host observes a game process.
    /// Manual Prepare remains available. Cancellation uses the existing block boundary.
    pub fn set_preparation_suspended(&self, suspended: bool) {
        self.inner.set_preparation_suspended(suspended);
    }

    /// Changes only for worker completion or a changed preparation condition.
    /// Hosts use it to end no-progress admission polling until something changes.
    pub fn preparation_revision(&self) -> u64 {
        self.inner.preparation.revision()
    }

    /// Admit unfinished library recordings, newest first, through the same runner.
    /// Busy workers leave the path eligible for the next library scan.
    pub fn prepare_library(&self, paths: &[PathBuf]) -> io::Result<()> {
        let mut paths: std::collections::BTreeSet<_> = paths
            .iter()
            .filter_map(|path| path.canonicalize().ok())
            .collect();
        paths.extend(
            self.inner
                .sources
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .library
                .keys()
                .cloned(),
        );
        let mut paths: Vec<_> = paths.into_iter().collect();
        paths.sort_by_cached_key(|path| {
            (
                std::cmp::Reverse(std::fs::metadata(path).and_then(|m| m.modified()).ok()),
                path.clone(),
            )
        });
        for path in paths {
            if self.inner.closed.load(Ordering::Acquire)
                || self.inner.preparation_suspended.load(Ordering::Acquire)
                || !self.inner.background_preparation.load(Ordering::Acquire)
            {
                break;
            }
            if let Err(error) = self.inner.prepare_recording(&path) {
                if error.kind() == io::ErrorKind::WouldBlock {
                    return Err(error);
                }
                // A damaged or unavailable recording must not starve the others.
                eprintln!("Background preparation of {}: {error}", path.display());
            }
        }
        Ok(())
    }

    /// Read the map preferences the desktop host saved since this process last read them.
    pub fn reload_map_settings(&self) {
        self.inner
            .map_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reload();
    }

    pub fn map_settings(&self, replay: &replay::container::Replay) -> ResolvedMapSettings {
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
    ) -> ResolvedMapSettings {
        self.scoped_map_settings(&map_settings::widget_key(
            &map_settings::campaign_key(replay),
            id,
        ))
    }

    /// The resolved settings of a scope key ([`map_settings::campaign_key`],
    /// [`map_settings::widget_key`]).
    pub fn scoped_map_settings(&self, scope: &str) -> ResolvedMapSettings {
        self.inner
            .map_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(scope)
    }

    /// Changes whenever a scope's resolved settings may have.
    pub fn map_settings_generation(&self) -> u64 {
        self.inner
            .map_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .generation()
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

    fn configured_content(&self, request: &Arc<Request>) -> Option<ContentOptions> {
        let content = self.inner.content();
        if content.is_none() {
            self.inner.error(
                Arc::clone(request),
                "Choose the game and user folders in Settings first".into(),
            );
        }
        content
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
                flip,
            } => {
                let Some(request) = self.inner.begin(id) else {
                    return;
                };
                if settings.is_some() && flip.is_some() {
                    self.inner.error(
                        request,
                        "A map settings request saves settings or flips one, not both".into(),
                    );
                    return;
                }
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
                let Some(content) = self.configured_content(&request) else {
                    return;
                };
                let job = MapSettingsJob {
                    target,
                    request: Arc::clone(&request),
                    source_id,
                    source,
                    settings,
                    flip,
                    options: content.options,
                };
                self.inner.submit(request, LoadJob::MapSettings(job));
            }
            DataCommand::HistoryCache { id, clear } => {
                let Some(request) = self.inner.begin(id) else {
                    return;
                };
                let Some(content) = self.configured_content(&request) else {
                    return;
                };
                let job = LoadJob::HistoryCache {
                    request: Arc::clone(&request),
                    clear,
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
            DataCommand::PreparedQuery { id, request: body } => {
                let Some(request) = self.inner.begin(id) else {
                    return;
                };
                if let Err(error) = body.validate() {
                    self.inner.error(request, error.into());
                    return;
                }
                let Some(source) =
                    self.leased_source(&request, &body.source_id, &body.revision, "Prepared query")
                else {
                    return;
                };
                self.inner.submit(
                    Arc::clone(&request),
                    LoadJob::PreparedQuery {
                        request,
                        source,
                        body,
                    },
                );
            }
            DataCommand::Preparation {
                id,
                source_id,
                revision,
                action,
            } => {
                let Some(request) = self.inner.begin(id) else {
                    return;
                };
                let Some(source) =
                    self.leased_source(&request, &source_id, &revision, "Preparation")
                else {
                    return;
                };
                self.inner.submit(
                    Arc::clone(&request),
                    LoadJob::Preparation {
                        request,
                        source,
                        source_id,
                        action,
                    },
                );
            }
            DataCommand::PreparationPreferences { id, enabled } => {
                let Some(request) = self.inner.begin(id) else {
                    return;
                };
                self.inner.submit(
                    Arc::clone(&request),
                    LoadJob::PreparationPreferences { request, enabled },
                );
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
        self.inner.preparation.shutdown();
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

#[cfg(test)]
mod tests;
