
use job_priority::SpawnCarried as _;
use dashboard_core::{config::Config, map_settings, protocol::ResolvedMapSettings, transport::{DataCommand, DataEvent, MAX_REPLY_BYTES, MapSettingsTarget}};
use dashboard_base::{Source, products::ContentOptions};
use crate::products;
use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

mod admission;
mod jobs;
pub mod perf;
mod pump;
mod queue;
use queue::{Sender as SyncSender, channel};

use admission::{Lease, Pending, Request};
use jobs::{LoadJob, MapSettingsJob, OpenJob, Sources};
use pump::{Signal, run_pump};

const MAX_PENDING: usize = dashboard_core::transport::MAX_PENDING_REQUESTS;

const MAX_SERIALIZED_BYTES: usize = MAX_REPLY_BYTES * 2;
const MAX_CANCELLED: usize = MAX_PENDING * 2;
const MIN_LOADER_THREADS: usize = 2;
const INTERACTIVE_THREADS: usize = 2;
const MIN_REPLY_THREADS: usize = 2;
const MIN_REPLAY_THREADS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Lane {
    History,
    Interactive,
    Replies,
    Replays,
}

impl Lane {
    const ALL: [Self; 4] = [Self::History, Self::Interactive, Self::Replies, Self::Replays];

    fn name(self) -> &'static str {
        match self {
            Self::History => "loader",
            Self::Interactive => "interactive",
            Self::Replies => "replies",
            Self::Replays => "replays",
        }
    }

    pub(super) fn threads(self, parallelism: usize) -> usize {
        match self {
            Self::History => (parallelism / 4).max(MIN_LOADER_THREADS),
            Self::Interactive => INTERACTIVE_THREADS,
            Self::Replies => (parallelism / 2).max(MIN_REPLY_THREADS),
            Self::Replays => (parallelism / 8).max(MIN_REPLAY_THREADS),
        }
    }
}

fn parallelism() -> usize {
    thread::available_parallelism().map_or(1, |threads| threads.get())
}

struct Lanes {
    history: SyncSender<LoadJob>,
    interactive: SyncSender<LoadJob>,
    replies: SyncSender<LoadJob>,
    replays: SyncSender<LoadJob>,
}

impl Lanes {
    fn sender(&self, lane: Lane) -> &SyncSender<LoadJob> {
        match lane {
            Lane::History => &self.history,
            Lane::Interactive => &self.interactive,
            Lane::Replies => &self.replies,
            Lane::Replays => &self.replays,
        }
    }
}

type VisualCatalogue = (
    PathBuf,
    PathBuf,
    Vec<content_reader_layers::content::visual::InstalledMod>,
);

#[cfg(test)]
type RefreshHook = Box<dyn FnOnce(&Inner) + Send>;

struct Inner {
    lease_namespace: uuid::Uuid,
    map_settings: Mutex<map_settings::Store>,
    content: dashboard_base::presentation::Config<Option<ContentOptions>>,
    visual_catalogue: Mutex<Option<VisualCatalogue>>,
    presentation_generation: std::sync::atomic::AtomicU64,
    sources: Mutex<Sources>,
    pending: Mutex<Pending>,
    loader: Mutex<Option<Lanes>>,
    signal: Arc<Signal>,
    closed: Arc<AtomicBool>,
    #[cfg(test)]
    test_rebind_derived: Mutex<Option<RefreshHook>>,
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
        let parallelism = parallelism();
        let lane = |lane: Lane| channel::<LoadJob>(MAX_PENDING, queue::share(lane.threads(parallelism)));
        let (history, history_jobs) = lane(Lane::History);
        let (interactive, interactive_jobs) = lane(Lane::Interactive);
        let (replies, replies_jobs) = lane(Lane::Replies);
        let (replays, replays_jobs) = lane(Lane::Replays);
        let inner = Arc::new(Inner {
            lease_namespace: uuid::Uuid::new_v4(),
            map_settings: Mutex::new(map_settings::Store::open()),
            content: dashboard_base::presentation::Config::new(None),
            visual_catalogue: Mutex::new(None),
            presentation_generation: std::sync::atomic::AtomicU64::new(0),
            sources: Mutex::new(Sources::default()),
            pending: Mutex::new(Pending::default()),
            loader: Mutex::new(Some(Lanes { history, interactive, replies, replays })),
            signal: Arc::new(Signal::default()),
            closed: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            test_rebind_derived: Mutex::new(None),
        });
        let mut threads = Vec::new();
        let spawn_result = (|| -> io::Result<()> {
            for (lane, jobs) in Lane::ALL.into_iter().zip([history_jobs, interactive_jobs, replies_jobs, replays_jobs]) {
                let jobs = Arc::new(Mutex::new(jobs));
                for index in 0..lane.threads(parallelism) {
                    let load_inner = Arc::clone(&inner);
                    let jobs = Arc::clone(&jobs);
                    threads.push(
                        thread::Builder::new()
                            .name(format!("dashboard {} {index}", lane.name()))
                            .spawn_carried(move || {
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
                    .spawn_carried(move || run_pump(pump_inner, emit))?,
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

    pub fn configure_content(&self, config: &Config, allow_unverified: bool) -> io::Result<()> {
        let options = match config.resolve_options() {
            Ok(options) => options,
            Err(_) => match Config::content_cache_dir() {
                Ok(cache_dir) => content_reader_layers::ResolveOptions {
                    game_dir: cache_dir.join("no game folder chosen"),
                    user_dir: cache_dir.join("no user folder chosen"),
                    cache_dir,
                },
                Err(_) => {
                    self.replace_content(None);
                    return Ok(());
                }
            },
        };
        self.replace_content(Some(ContentOptions {
            options,
            allow_unverified,
            presentation: Default::default(),
        }));
        self.inner.configure_game();
        Ok(())
    }

    pub fn upgrade_recording(&self, path: &std::path::Path) {
        if let Ok(path) = path.canonicalize() {
            self.inner.upgrade(&path);
        }
    }

    pub fn content_bound(&self) -> bool {
        self.inner
            .content()
            .is_some_and(|content| content.options.game_dir.join("hoi4.exe").is_file())
    }

    fn replace_content(&self, content: Option<ContentOptions>) {
        let key = |value: Option<&ContentOptions>| {
            value.map(|value| {
                (
                    dashboard_base::content::content_key(Some(value)),
                    value.options.cache_dir.clone(),
                )
            })
        };
        let changed = self.inner.content.update(|current| {
            let changed = key(current.as_ref()) != key(content.as_ref());
            *current = content.clone();
            changed
        });
        if !changed {
            return;
        }
        let _ = self.inner.catch_up();
    }

    pub fn reload_map_settings(&self) {
        self.inner
            .map_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reload();
    }

    pub fn map_settings(&self, replay: &replay::campaign::Replay) -> ResolvedMapSettings {
        self.inner
            .map_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&map_settings::campaign_key(replay))
    }

    pub fn widget_map_settings(
        &self,
        replay: &replay::campaign::Replay,
        id: &str,
    ) -> ResolvedMapSettings {
        self.scoped_map_settings(&map_settings::widget_key(
            &map_settings::campaign_key(replay),
            id,
        ))
    }

    pub fn scoped_map_settings(&self, scope: &str) -> ResolvedMapSettings {
        self.inner
            .map_settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(scope)
    }

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
                    format!("{what} source is unavailable or changed").into(),
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
                dashboard_core::QueryError::Refused {
                    text: "Choose the game and user folders in Settings first".into(),
                },
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
                self.inner.submit(request, LoadJob::MapSettings(Box::new(job)));
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
                if products::find(&body.product).is_none()
                    && !crate::answer::answers(&body.product, &body.params)
                {
                    self.inner.error(
                        request,
                        format!("Unknown data product: {}", body.product).into(),
                    );
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
            DataCommand::HistoryChoice {
                id,
                path,
                segment_id,
                choice,
            } => {
                let Some(request) = self.inner.begin(id) else {
                    return;
                };
                self.inner.submit(
                    Arc::clone(&request),
                    LoadJob::HistoryChoice {
                        request,
                        path,
                        segment_id,
                        choice,
                    },
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

