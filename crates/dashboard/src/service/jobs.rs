use super::{Inner, Lease, MAX_SOURCE_LEASES, Request};
use crate::{
    Source, SourceKey, map_settings,
    products::{self, ContentOptions, LoadContext},
    protocol::{CampaignMapSettings, replay_revision},
    transport::{MapSettingsTarget, ProductRequest, SourceReply, bounded_json, bounded_text},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
    },
};

#[derive(Default)]
pub(super) struct Sources {
    next: u64,
    entries: BTreeMap<(PathBuf, String), Weak<Source>>,
    // Failed openers leave their waiters' gate discoverable; idle gates expire.
    opening: BTreeMap<(PathBuf, String), Weak<Mutex<()>>>,
}

impl Sources {
    pub(super) fn open_gate(&mut self, key: (PathBuf, String)) -> Arc<Mutex<()>> {
        self.opening.retain(|_, gate| gate.strong_count() > 0);
        if let Some(gate) = self.opening.get(&key).and_then(Weak::upgrade) {
            return gate;
        }
        let gate = Arc::new(Mutex::new(()));
        self.opening.insert(key, Arc::downgrade(&gate));
        gate
    }
}

impl Inner {
    pub(super) fn content(&self) -> Option<ContentOptions> {
        self.content
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Duplicate opens share a per-revision gate. Disk reads and opted-in cache
    /// inheritance never hold the global sources lock.
    pub(super) fn open(
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
        let cache_key = (path.clone(), revision.clone());
        let (gate, previous) = {
            let mut sources = self.sources.lock().map_err(|error| error.to_string())?;
            sources
                .entries
                .retain(|_, source| source.strong_count() > 0);
            if let Some(source) = sources.entries.get(&cache_key).and_then(Weak::upgrade) {
                return Ok(source);
            }
            let previous = sources
                .entries
                .iter()
                .filter(|((old_path, _), _)| old_path == &path)
                .filter_map(|(_, old)| old.upgrade())
                .max_by_key(|old| old.replay_ref().index.len());
            let gate = sources.open_gate(cache_key.clone());
            (gate, previous)
        };
        let _opening = gate.lock().map_err(|error| error.to_string())?;
        self.check_open(cancelled)?;
        let key = {
            let mut sources = self.sources.lock().map_err(|error| error.to_string())?;
            if let Some(source) = sources.entries.get(&cache_key).and_then(Weak::upgrade) {
                return Ok(source);
            }
            sources.next = sources
                .next
                .checked_add(1)
                .ok_or("Source identity exhausted")?;
            SourceKey {
                source_id: format!("source-{}", sources.next),
                revision,
            }
        };
        let source =
            Arc::new(Source::open(key, path, cancelled).map_err(|error| error.to_string())?);
        if let Some(previous) = previous {
            source.inherit_append_states(&previous);
        }
        self.check_open(cancelled)?;
        self.sources
            .lock()
            .map_err(|error| error.to_string())?
            .entries
            .insert(cache_key, Arc::downgrade(&source));
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

    pub(super) fn submit(&self, request: Arc<Request>, job: LoadJob) {
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

    fn map_settings_job(&self, job: MapSettingsJob) -> Result<String, String> {
        let MapSettingsJob {
            target,
            request,
            source_id,
            source,
            settings,
            options,
        } = job;
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

    pub(super) fn run_loader(self: &Arc<Self>, jobs: &Mutex<Receiver<LoadJob>>) {
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
                LoadJob::MapSettings(job) => {
                    let request = Arc::clone(&job.request);
                    let result = self.map_settings_job(job);
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
        let result = self
            .open(
                Path::new(&job.path),
                Some(&job.revision),
                &job.request.cancelled,
            )
            .inspect(|source| source.name_countries(self.content().as_ref()));
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
                        metadata: source.metadata(),
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
            Ok(_) => pending
                .errors
                .push_back((job.request, crate::transport::TOO_MANY_SOURCES.into())),
            Err(error) => pending
                .errors
                .push_back((job.request, bounded_text(&error, 2048))),
        }
        drop(pending);
        self.signal.notify();
    }
}

pub(super) struct MapSettingsJob {
    pub(super) target: MapSettingsTarget,
    pub(super) request: Arc<Request>,
    pub(super) source_id: String,
    pub(super) source: Option<Arc<Source>>,
    pub(super) settings: Option<CampaignMapSettings>,
    pub(super) options: installed_content::ResolveOptions,
}

pub(super) struct OpenJob {
    pub(super) request: Arc<Request>,
    pub(super) path: String,
    pub(super) revision: String,
}

pub(super) enum LoadJob {
    MapSettings(MapSettingsJob),
    Open(OpenJob),
    Product {
        request: Arc<Request>,
        lease_id: String,
        source: Arc<Source>,
        body: ProductRequest,
    },
}
