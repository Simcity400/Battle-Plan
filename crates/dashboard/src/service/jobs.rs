use content_reader_key_names::key_name_products::KeyNamesExt as _;
use super::{Inner, Lane, Lease, Request, queue::Receiver};
use dashboard_base::{Source, SourceKey, products::{ContentOptions, LoadContext}};
use dashboard_core::{map_settings, protocol::{CampaignMapSettings, replay_revision}, transport::{MapSettingsTarget, ProductRequest, SourceReply, bounded_json, bounded_text}};
use crate::products;
use job_priority::SpawnCarried as _;
use std::{
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, TryLockError, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc::TrySendError,
    },
};

pub(super) fn product_lane(product: &str, params: &serde_json::Value) -> Lane {
    let registered = products::find(product);
    if registered.is_some_and(|product| product.replays) {
        Lane::Replays
    } else if crate::answer::answers(product, params) {
        Lane::Replies
    } else if registered.is_some_and(|product| product.interactive) {
        Lane::Interactive
    } else {
        Lane::History
    }
}

fn work(content_dependent: bool, request: &Arc<Request>) -> dashboard_base::source::Work {
    if content_dependent {
        dashboard_base::source::Work::Content(Arc::clone(request) as Arc<dyn dashboard_base::presentation::Cancel>)
    } else {
        dashboard_base::source::Work::Recorded
    }
}

#[derive(Default)]
pub(super) struct Sources {
    next: u64,
    pub(super) entries: BTreeMap<(PathBuf, String), Weak<Source>>,
    pub(super) binding: Vec<Weak<Source>>,
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

const RECORDING_CHANGED: &str = "The recording changed. Reopen it before requesting another view.";

fn recording_open_error(error: std::io::Error) -> String {
    let sentence = if engine_allocation::is_allocation_error(&error) {
        "There isn't enough memory to open this recording."
    } else if matches!(
        error.kind(),
        std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof
    ) {
        "This recording is damaged or isn't a BattlePlan recording."
    } else {
        return error.to_string();
    };
    replay_data::codec::try_format(format_args!("{sentence} Details: {error}"))
        .unwrap_or_else(|_| sentence.to_owned())
}

impl Inner {
    pub(super) fn recording_revision(&self, path: &Path) -> std::io::Result<String> {
        replay_revision(path)
    }

    pub(super) fn content(&self) -> Option<ContentOptions> {
        self.content.get()
    }

    pub(super) fn upgrade(&self, path: &Path) {
        static UPGRADING: Mutex<()> = Mutex::new(());
        let lacks_facts = |path: &Path| {
            replay::campaign::Replay::open_path(path)
                .is_ok_and(|replay| replay.fact_packs.is_empty())
        };
        let Some(content) = self.content() else {
            return;
        };
        if !lacks_facts(path) {
            return;
        }
        let _upgrading = UPGRADING.lock().unwrap_or_else(|error| error.into_inner());
        if !lacks_facts(path) {
            return;
        }
        let mut games = vec![content.options.game_dir.clone()];
        games.extend(
            dashboard_core::config::Config::load()
                .map(|config| config.older_game_dirs)
                .unwrap_or_default(),
        );
        let folders = recording_upgrade::Folders {
            games,
            user: content.options.user_dir.clone(),
            cache: content.options.cache_dir,
        };
        match recording_upgrade::upgrade(path, &folders) {
            Ok(recording_upgrade::Outcome::Upgraded { steps }) => {
                eprintln!("Upgraded {}: {}", path.display(), steps.join("; "))
            }
            Ok(recording_upgrade::Outcome::Current) => {}
            Err(error) => eprintln!("{} is not upgraded: {error}", path.display()),
        }
    }

    pub(super) fn content_for(
        &self,
        source: &Source,
        work: dashboard_base::source::Work,
    ) -> Option<ContentOptions> {
        source.admit_resolved(work)
    }

    pub(super) fn bound_sources(&self) -> Vec<Arc<Source>> {
        let sources = self
            .sources
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        sources
            .entries
            .values()
            .chain(&sources.binding)
            .filter_map(Weak::upgrade)
            .collect()
    }

    pub(super) fn rebind(&self, source: &Arc<Source>, cancelled: &AtomicBool) -> Result<(), String> {
        use dashboard_base::presentation::Presentation;
        let moved = std::cell::Cell::new(false);
        let result = dashboard_base::presentation::rebind(
            &self.content,
            cancelled,
            |content| {
                let Some(content) = content else {
                    return Ok(None);
                };
                let selected = self
                    .map_settings
                    .lock()
                    .map_err(|error| std::io::Error::other(error.to_string()))?
                    .get(source.campaign_key())
                    .visual_mods
                    .unwrap_or_default();
                let identity = match Presentation::current(
                    &content.options,
                    source.replay_ref(),
                    &selected,
                    cancelled,
                ) {
                    Ok(identity) => identity,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                        return Err(error);
                    }
                    Err(error) => Presentation {
                        visual_mods: selected,
                        revision: format!("unreadable: {error}"),
                    },
                };
                #[cfg(test)]
                {
                    let hook = self
                        .test_rebind_derived
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .take();
                    if let Some(hook) = hook {
                        hook(self);
                    }
                }
                Ok(Some(identity))
            },
            |content, identity| {
                let bound = content
                    .as_ref()
                    .map(|content| (&content.options, content.allow_unverified));
                if source.bind(bound, identity.unwrap_or_default()) {
                    self.presentation_generation.fetch_add(1, Ordering::AcqRel);
                    moved.set(true);
                }
            },
        );
        match result {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                Err("Cancelled".into())
            }
            result => {
                result.map_err(|error| error.to_string())?;
                if moved.get() {
                    self.warm(source);
                }
                Ok(())
            }
        }
    }

    fn warm(&self, source: &Arc<Source>) {
        let Some(content) = self.content_for(source, dashboard_base::source::Work::Recorded) else {
            return;
        };
        let warming = Arc::clone(source);
        job_priority::spawn(content_reader_layers::content::carry_generation(move || {
            warming.in_scope(|| dashboard_base::content::warm(&warming, &content));
        }));
    }

    pub(super) fn configure_game(&self) {
        let _ = dashboard_base::presentation::rebind(
            &self.content,
            &self.closed,
            |content| {
                Ok(content.as_ref().map(|content| {
                    engine_executable::executable_tables::or_installed(
                        &content.options.game_dir,
                        || map_settings::read_game(&content.options.user_dir),
                    )
                }))
            },
            |_, game| {
                if let Some(game) = game {
                    self.map_settings
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .set_game(game);
                }
            },
        );
    }

    pub(super) fn catch_up(&self) -> Result<(), String> {
        self.present(&self.bound_sources(), &self.closed)
    }

    pub(super) fn present(
        &self,
        sources: &[Arc<Source>],
        cancelled: &AtomicBool,
    ) -> Result<(), String> {
        for source in sources {
            self.rebind(source, cancelled)?;
        }
        Ok(())
    }

    pub(super) fn presentation_generation(&self) -> u64 {
        self.presentation_generation.load(Ordering::Acquire)
    }

    pub(super) fn open(
        &self,
        path: &Path,
        expected: Option<&str>,
        cancelled: &AtomicBool,
    ) -> Result<Arc<Source>, String> {
        request_result(|| self.open_inner(path, expected, cancelled))
    }

    fn open_inner(
        &self,
        path: &Path,
        expected: Option<&str>,
        cancelled: &AtomicBool,
    ) -> Result<Arc<Source>, String> {
        self.check_open(cancelled)?;
        let path = path.canonicalize().map_err(|error| error.to_string())?;
        if expected.is_none() {
            self.upgrade(&path);
        }
        let revision = self
            .recording_revision(&path)
            .map_err(|error| error.to_string())?;
        if expected.is_some_and(|expected| expected != revision) {
            return Err(RECORDING_CHANGED.into());
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
        let _opening = gate.lock().unwrap_or_else(|error| error.into_inner());
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
        let source = Arc::new(Source::open(key, path, cancelled).map_err(recording_open_error)?);
        if self.recording_revision(source.path()).ok().as_ref() != Some(&source.key().revision) {
            return Err(RECORDING_CHANGED.into());
        }
        if let Some(previous) = previous {
            source.inherit_append_states(&previous);
        }
        let registered = Arc::downgrade(&source);
        self.sources
            .lock()
            .map_err(|error| error.to_string())?
            .binding
            .push(Weak::clone(&registered));
        let bound = source.in_scope(|| self.rebind(&source, cancelled));
        let unregister = || {
            self.sources
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .binding
                .retain(|opening| !Weak::ptr_eq(opening, &registered));
        };
        if let Err(error) = bound {
            unregister();
            return Err(error);
        }
        let checked = self.check_open(cancelled);
        let mut sources = self.sources.lock().map_err(|error| error.to_string())?;
        if checked.is_ok() {
            sources.entries.insert(cache_key, Weak::clone(&registered));
        }
        sources
            .binding
            .retain(|opening| !Weak::ptr_eq(opening, &registered));
        drop(sources);
        checked.map(|()| source)
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
        let lane = match &job {
            LoadJob::Product { body, .. } => product_lane(&body.product, &body.params),
            LoadJob::HistoryChoice { .. } => Lane::Interactive,
            LoadJob::MapSettings(_) | LoadJob::Open(_) => Lane::History,
        };
        self.submit_to(lane, request, job);
    }

    pub(super) fn submit_to(&self, lane: Lane, request: Arc<Request>, job: LoadJob) {
        if let Some(timing) = &request.timing {
            timing.queued(lane);
        }
        let result = {
            let loader = self
                .loader
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match loader.as_ref() {
                Some(lanes) => match lanes.sender(lane).try_send(job, LoadJob::cancelled) {
                    Ok(()) => Ok(()),
                    Err(TrySendError::Full(_)) => Err(dashboard_core::transport::CAPACITY_REACHED),
                    Err(TrySendError::Disconnected(_)) => Err("Dashboard data service is closing"),
                },
                None => Err("Dashboard data service is closing"),
            }
        };
        if let Err(message) = result {
            self.error(request, message.into());
        }
    }

    pub(super) fn map_settings_job(&self, job: MapSettingsJob) -> Result<String, String> {
        let MapSettingsJob {
            target,
            request,
            source_id,
            source,
            settings,
            flip,
            options,
        } = job;
        if request.cancelled() {
            return Err("Cancelled".into());
        }
        let campaign = source
            .as_ref()
            .map(|source| source.campaign_key())
            .unwrap_or_default();
        let key = target.key(campaign).map_err(str::to_owned)?;
        self.configure_game();
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
        let sample_counts = map_settings::sample_counts();
        if let Some(settings) = &settings {
            settings.validate().map_err(str::to_owned)?;
            if let (Some(samples), Some(counts)) = (settings.graphics.samples, &sample_counts)
                && !counts.contains(&samples)
            {
                return Err(
                    "This anti-aliasing setting is unavailable on the current renderer".into(),
                );
            }
        }
        if request.cancelled() {
            return Err("Cancelled".into());
        }
        let mut store = self.map_settings.lock().map_err(|e| e.to_string())?;
        store.check()?;
        let mut save_warning = None;
        if let Some(settings) = settings {
            save_warning = store.save(key.clone(), settings).map_err(|e| e.to_string())?;
        }
        if let Some(setting) = flip {
            save_warning = store.flip(key.clone(), setting).map_err(|e| e.to_string())?.or(save_warning);
        }
        let replay = source.as_ref().map(|source| source.replay_ref());
        let recorded = replay
            .map(dashboard_base::presentation::recorded_mods)
            .unwrap_or_default();
        let recorded_mods = recorded
            .iter()
            .map(|recorded| map_settings::RecordedMod {
                recorded_id: recorded.id.clone(),
                name: recorded.name.clone(),
            })
            .collect();
        let selected = store.get(&key).visual_mods.unwrap_or_default();
        let mut reply = map_settings::Reply {
            source_id,
            revision: source
                .as_ref()
                .map(|source| source.key().revision.clone())
                .unwrap_or_default(),
            settings: store.preferences(&key),
            inherited_settings: store.inherited(&key),
            mods,
            recorded_mods,
            applied_mods: Vec::new(),
            applied_notes: Vec::new(),
            sample_counts,
            game_graphics: store.game().map(|game| game.graphics),
            game_settings_error: store.game_error(),
            save_warning,
            presentation_generation: 0,
        };
        drop(store);
        self.content.update(|_| ());
        self.catch_up()?;
        reply.presentation_generation = self.presentation_generation();
        match content_reader_layers::content::visual::plan(
            &options,
            &selected,
            &recorded,
            &request.cancelled,
        ) {
            Ok(applied) => {
                reply.applied_mods = applied.order;
                reply.applied_notes = applied.warnings;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                return Err("Cancelled".into());
            }
            Err(error) => reply
                .applied_notes
                .push(format!("Installed mods could not be read: {error}.")),
        }
        bounded_json(&reply, &request.cancelled)
    }

    pub(super) fn run_loader(self: &Arc<Self>, jobs: &Mutex<Receiver<LoadJob>>, lane: Lane) {
        loop {
            let job = {
                let receiver = match jobs.try_lock() {
                    Ok(receiver) => receiver,
                    Err(TryLockError::Poisoned(error)) => error.into_inner(),
                    Err(TryLockError::WouldBlock) => {
                        engine_heap::return_freed();
                        jobs.lock().unwrap_or_else(|error| error.into_inner())
                    }
                };
                receiver.recv_idle(engine_heap::return_freed)
            };
            let Ok((job, _turn)) = job else {
                return;
            };
            if self.closed.load(Ordering::Acquire) {
                return;
            }
            let request = job.request().cloned();
            if let Some(timing) = request.as_ref().and_then(|request| request.timing.as_ref()) {
                timing.started();
            }
            let _cpu = match &job {
                LoadJob::Product { body, .. } => {
                    ::replay_frame::cpu::scope_named("product load", &body.product)
                }
                LoadJob::Open(_) => ::replay_frame::cpu::scope("service job", Some("open")),
                _ => ::replay_frame::cpu::scope("service job", Some("other")),
            };
            let tables = job.source().and_then(|source| source.tables());
            let result = request_result(|| {
                engine_executable::executable_tables::scope(tables, || self.run_job(job, lane));
                Ok(())
            });
            if let (Err(error), Some(request)) = (result, request) {
                self.error(request, error.into());
            }
        }
    }

    fn run_job(&self, job: LoadJob, lane: Lane) {
        let _ = lane;
        match job {
            LoadJob::HistoryChoice {
                request,
                path,
                segment_id,
                choice,
            } => {
                if request.cancelled() {
                    return;
                }
                let result = request_result(|| {
                    history_choice(Path::new(&path), &segment_id, choice.as_deref())
                        .map_err(|error| error.to_string())?;
                    bounded_json(&serde_json::json!({}), &request.cancelled)
                });
                self.complete(request, result.map_err(Into::into));
            }
            LoadJob::MapSettings(job) => {
                let request = Arc::clone(&job.request);
                let result = self.map_settings_job(*job);
                self.complete(request, result.map_err(Into::into));
            }
            LoadJob::Product {
                request,
                lease_id,
                source,
                body,
            } => {
                if request.cancelled() {
                    return;
                }
                let result = if products::find(&body.product).is_some() {
                    self.product(&request, &lease_id, &source, body)
                } else {
                    self.answer(&request, &lease_id, &source, &body)
                };
                self.complete(request, result);
            }
            LoadJob::Open(job) => {
                self.configure_game();
                self.open_job(job);
            }
        }
    }

    fn product(
        &self,
        request: &Arc<Request>,
        lease_id: &str,
        source: &Arc<Source>,
        body: ProductRequest,
    ) -> Result<String, dashboard_core::QueryError> {
        let Some(product) = products::find(&body.product) else {
            return Err(format!("Unknown data product: {}", body.product).into());
        };
        let context = LoadContext {
            content: self.content_for(source, work(product.content_dependent, request)),
        };
        (product.load)(source, lease_id, body.params, &context, &request.cancelled)
    }

    fn answer(
        &self,
        request: &Arc<Request>,
        lease_id: &str,
        source: &Arc<Source>,
        body: &ProductRequest,
    ) -> Result<String, dashboard_core::QueryError> {
        let context = LoadContext {
            content: self.content_for(source, work(false, request)),
        };
        let data = source
            .in_scope(|| crate::answer::answer(source, &body.product, &body.params, &context, &request.cancelled))
            .map_err(dashboard_core::QueryError::from)?;
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Envelope<'a> {
            source_id: &'a str,
            revision: &'a str,
            product: &'a str,
            data: serde_json::Value,
        }
        let reply = bounded_json(
            &Envelope {
                source_id: lease_id,
                revision: &source.key().revision,
                product: &body.product,
                data,
            },
            &request.cancelled,
        )?;
        if reply.len() > dashboard_core::transport::MAX_REPLY_BYTES {
            return Err("Projected response exceeds the transport byte budget".into());
        }
        Ok(reply)
    }

    fn open_job(&self, job: OpenJob) {
        if job.request.cancelled() {
            return;
        }
        let opened = self.open(
            Path::new(&job.path),
            Some(&job.revision),
            &job.request.cancelled,
        );
        let tables = opened.as_ref().ok().and_then(|source| source.tables());
        engine_executable::executable_tables::scope(tables, || self.opened(job, opened));
    }

    fn opened(&self, job: OpenJob, opened: Result<Arc<Source>, String>) {
        let result = opened.and_then(|source| {
            crate::answer::open_all(&source).map_err(|error| error.to_string())?;
            read_key_names_ahead(&source, self.content());
            read_naming_ahead(&source);
            read_feed_scope_ahead(&source);
            read_air_observed_ahead(&source);
            read_lexicons_ahead(&source);
            source
                .name_countries(self.content().as_ref())
                .map_err(|error| error.to_string())?;
            Ok(source)
        });
        let ahead = result.as_ref().ok().map(|source| {
            let content = source.admit(dashboard_base::source::Work::Ahead);
            (Arc::clone(source), content)
        });
        let ahead = ahead.and_then(|(source, content)| Some((source, content?)));
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
        let result = result.and_then(|source| {
            source
                .metadata()
                .map(|metadata| (source, metadata))
                .map_err(|error| error.to_string())
        });
        match result {
            Ok((source, metadata)) => {
                if let Some(next) = pending.next_lease.checked_add(1) {
                    pending.next_lease = next;
                    let id = self.lease_id(next);
                    let reply = SourceReply {
                        id: id.clone(),
                        path: source.path().to_string_lossy().into_owned(),
                        revision: source.key().revision.clone(),
                        metadata,
                        presentation_generation: self.presentation_generation(),
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
                            .push_back((job.request, bounded_text(&error, 2048).into())),
                    }
                } else {
                    pending
                        .errors
                        .push_back((job.request, "Source lease identity exhausted".into()));
                }
            }
            Err(error) => pending
                .errors
                .push_back((job.request, bounded_text(&error, 2048).into())),
        }
        drop(pending);
        self.signal.notify();
        if let Some((source, content)) = ahead {
            dashboard_base::products::read_ahead::warm_in_background(&source, content);
        }
    }
}

fn read_key_names_ahead(source: &Source, content: Option<ContentOptions>) {
    let Some(resolved) = content.and_then(|content| source.content(&content.options).ok()) else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name("key names ahead".into())
        .spawn_carried(move || drop(resolved.layers.key_names()));
}

fn read_naming_ahead(source: &Arc<Source>) {
    let Some(branch) = source.replay_ref().timelines().map(|timelines| timelines.main) else { return };
    let source = Arc::downgrade(source);
    let _ = std::thread::Builder::new().name("feed naming ahead".into()).spawn_carried(move || {
        job_priority::confined(|| {
            let Some(source) = source.upgrade() else { return };
            let _ = source.in_scope(|| dashboard_politics::products::feed::warm_naming(&source, branch));
        })
    });
}

fn read_feed_scope_ahead(source: &Arc<Source>) {
    let Some(branch) = source.replay_ref().timelines().map(|timelines| timelines.main) else { return };
    let source = Arc::downgrade(source);
    let _ = std::thread::Builder::new().name("feed scope ahead".into()).spawn_carried(move || {
        job_priority::confined(|| {
            let Some(source) = source.upgrade() else { return };
            let _ = source.in_scope(|| dashboard_politics::products::feed::index::warm(&source, branch));
        })
    });
}

fn read_air_observed_ahead(source: &Arc<Source>) {
    let Some(branch) = source.replay_ref().timelines().map(|timelines| timelines.main) else { return };
    let source = Arc::downgrade(source);
    let _ = std::thread::Builder::new().name("air observed ahead".into()).spawn_carried(move || {
        job_priority::confined(|| {
            let Some(source) = source.upgrade() else { return };
            let _ = source.in_scope(|| dashboard_air::products::air_ledger::warm_observed(&source, branch));
        })
    });
}

fn read_lexicons_ahead(source: &Arc<Source>) {
    let source = Arc::downgrade(source);
    let _ = std::thread::Builder::new().name("lexicons ahead".into()).spawn_carried(move || {
        job_priority::confined(|| {
            let Some(source) = source.upgrade() else { return };
            let _ = source.in_scope(|| dashboard_base::queries::warm_lexicons(&source));
        })
    });
}

fn history_choice(path: &Path, segment: &str, parent: Option<&str>) -> std::io::Result<()> {
    use replay::campaign::{Replay, timelines};
    use std::io;
    let id = |text: &str| {
        text.parse::<u64>()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid recorded segment"))
    };
    let segment = id(segment)?;
    let parent = parent.map(id).transpose()?;
    let campaign = Replay::read_campaign_id(path)?;
    timelines::write_choice(&campaign, segment, parent)?;
    std::fs::File::options()
        .write(true)
        .open(path)?
        .set_modified(std::time::SystemTime::now())
}

fn request_result<T>(read: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(read))
        .unwrap_or_else(|_| Err("Dashboard request failed while reading the recording".into()))
}

pub(super) struct MapSettingsJob {
    pub(super) target: MapSettingsTarget,
    pub(super) request: Arc<Request>,
    pub(super) source_id: String,
    pub(super) source: Option<Arc<Source>>,
    pub(super) settings: Option<CampaignMapSettings>,
    pub(super) flip: Option<dashboard_core::protocol::MapSetting>,
    pub(super) options: content_reader_layers::ResolveOptions,
}

pub(super) struct OpenJob {
    pub(super) request: Arc<Request>,
    pub(super) path: String,
    pub(super) revision: String,
}

pub(super) enum LoadJob {
    HistoryChoice {
        request: Arc<Request>,
        path: String,
        segment_id: String,
        choice: Option<String>,
    },
    MapSettings(Box<MapSettingsJob>),
    Open(OpenJob),
    Product {
        request: Arc<Request>,
        lease_id: String,
        source: Arc<Source>,
        body: ProductRequest,
    },
}

impl super::queue::Classed for LoadJob {
    fn class(&self) -> &str {
        match self {
            Self::Product { body, .. } => &body.product,
            Self::HistoryChoice { .. } => "history choice",
            Self::MapSettings(_) => "map settings",
            Self::Open(_) => "open",
        }
    }
}

impl LoadJob {
    fn source(&self) -> Option<&Arc<Source>> {
        match self {
            Self::Product { source, .. } => Some(source),
            _ => None,
        }
    }

    fn request(&self) -> Option<&Arc<Request>> {
        Some(match self {
            Self::MapSettings(job) => &job.request,
            Self::Open(job) => &job.request,
            Self::Product { request, .. } | Self::HistoryChoice { request, .. } => request,
        })
    }

    fn cancelled(&self) -> bool {
        self.request().is_some_and(|request| request.cancelled())
    }
}

#[cfg(test)]
mod lane_tests {
    use super::*;

    #[test]
    fn pages_that_replay_hours_wait_apart_from_every_reply() {
        let params = serde_json::json!({});
        for key in ["event-feed-tail", "event-choices-tail", "decisions-tail"] {
            assert!(products::find(key).is_some_and(|product| product.replays), "{key} is not marked as replaying");
            assert_eq!(product_lane(key, &params), Lane::Replays, "{key}");
        }
        for product in products::PRODUCTS.iter().filter(|product| !product.replays) {
            assert_ne!(product_lane(product.key, &params), Lane::Replays, "{}", product.key);
        }
        for key in ["idea-history", "civil-wars", "country-flags", "log-page", "division-log-tail"] {
            assert_ne!(product_lane(key, &params), Lane::Replays, "{key} waits behind pages");
        }
    }
}

#[cfg(test)]
mod recording_error_tests {
    use super::*;
    use std::io;

    #[test]
    fn recording_open_errors_start_with_a_plain_sentence_and_keep_details() {
        for kind in [io::ErrorKind::InvalidData, io::ErrorKind::UnexpectedEof] {
            assert_eq!(
                recording_open_error(io::Error::new(kind, "synthetic reason")),
                "This recording is damaged or isn't a BattlePlan recording. Details: synthetic reason"
            );
        }
        let error = recording_open_error(io::Error::from(io::ErrorKind::OutOfMemory));
        assert!(error.starts_with("There isn't enough memory to open this recording. Details: "));
        assert_eq!(
            recording_open_error(io::Error::new(io::ErrorKind::Interrupted, "cancelled")),
            "cancelled"
        );
        assert_eq!(
            recording_open_error(io::Error::new(io::ErrorKind::PermissionDenied, "access")),
            "access"
        );
    }
}
