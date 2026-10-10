//! The native data service: one owner of opened recordings, a small loader
//! pool that answers product requests, and a bounded response pump.
//!
//! Opening files and answering products never depends on a visible window.
//! Only compact JSON replies cross the desktop control pipe.

use crate::{
    Config, Source, map_settings,
    products::{self, ContentOptions},
    protocol::CampaignMapSettings,
    transport::{DataCommand, DataEvent, MAX_REPLY_BYTES, MapSettingsTarget},
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
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
const MAX_SOURCE_LEASES: usize = 64;
const MAX_SERIALIZED_BYTES: usize = MAX_REPLY_BYTES * 2;
const MAX_CANCELLED: usize = MAX_PENDING * 2;
const LOADER_THREADS: usize = 3;

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
    pub fn configure_content(&self, config: &Config, allow_unverified: bool) -> io::Result<()> {
        let options = match config.resolve_options() {
            Ok(options) => options,
            Err(_) => {
                *self
                    .inner
                    .content
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = None;
                return Ok(());
            }
        };
        if let Err(error) = engine::executable_tables::initialize(&options.game_dir) {
            *self
                .inner
                .content
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = None;
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
        *self
            .inner
            .content
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(ContentOptions {
            options,
            allow_unverified,
        });
        Ok(())
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
                let job = MapSettingsJob {
                    target,
                    request: Arc::clone(&request),
                    source_id,
                    source,
                    settings,
                    options: content.options,
                };
                self.inner.submit(request, LoadJob::MapSettings(job));
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

#[cfg(test)]
mod tests;
