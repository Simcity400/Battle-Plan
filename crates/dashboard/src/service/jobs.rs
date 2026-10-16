use super::{Inner, Lease, Request};
use crate::{
    Source, SourceKey, map_settings,
    products::{self, ContentOptions, LoadContext},
    protocol::{CampaignMapSettings, replay_revision},
    reply_cache,
    transport::{
        HistoryCacheReply, MapSettingsTarget, ProductRequest, SourceReply, bounded_json,
        bounded_text,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, TrySendError},
    },
};

#[derive(Default)]
pub(super) struct Sources {
    next: u64,
    // Only jobs that may need resuming hold a library source. Completed revisions
    // retain a cheap stamp; explicit Cancel/Clear suppress ticks across revisions.
    pub(super) library: BTreeMap<PathBuf, Arc<Source>>,
    pub(super) completed: BTreeMap<PathBuf, String>,
    suppressed: BTreeSet<PathBuf>,
    pub(super) entries: BTreeMap<(PathBuf, String), Weak<Source>>,
    // Failed openers leave their waiters' gate discoverable; idle gates expire.
    opening: BTreeMap<(PathBuf, String), Weak<Mutex<()>>>,
}

impl Sources {
    #[cfg(test)]
    pub(super) fn open_attempts(&self) -> u64 {
        self.next
    }

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

/// The first sentence is suitable for the sidebar; retain the technical reason
/// after it for bug reports. Cancellation and ordinary filesystem errors keep
/// their existing meaning.
fn recording_open_error(error: std::io::Error) -> String {
    let sentence = if engine::allocation::is_allocation_error(&error) {
        "There isn't enough memory to open this recording."
    } else if matches!(
        error.kind(),
        std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof
    ) {
        "This recording is damaged or isn't a Battle Plan recording."
    } else {
        return error.to_string();
    };
    replay::codec::try_format(format_args!("{sentence} Details: {error}"))
        .unwrap_or_else(|_| sentence.to_owned())
}

impl Inner {
    pub(super) fn preparation_preferences(&self) -> crate::prepared::Preferences {
        crate::prepared::Preferences {
            background_enabled: self.background_preparation.load(Ordering::Acquire),
        }
    }

    pub(super) fn set_background_preparation(&self, enabled: bool) {
        if self.background_preparation.swap(enabled, Ordering::AcqRel) == enabled {
            return;
        }
        self.preparation.notify_change();
        let _admission = self
            .preparation_actions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let sources: Vec<_> = self
            .sources
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entries
            .values()
            .filter_map(Weak::upgrade)
            .collect();
        if !enabled {
            for source in sources {
                let progress = source.state::<crate::prepared::Progress>();
                let progress = progress.lock().unwrap_or_else(|error| error.into_inner());
                let cancel = progress.running && progress.automatic;
                drop(progress);
                if cancel {
                    crate::prepared::cancel(&source);
                }
            }
            return;
        }
        let mut latest = BTreeMap::new();
        for source in sources {
            if replay_revision(source.path()).ok().as_deref()
                == Some(source.key().revision.as_str())
            {
                latest.insert(source.path().to_owned(), source);
            }
        }
        for source in latest.into_values() {
            if let Err(error) = self.refresh_preparation(source, false) {
                eprintln!("Preparation refresh: {error}");
            }
        }
    }

    /// Refreshes share history capacity and one queued item per source. A later
    /// restart request upgrades that item rather than retaining another source.
    pub(super) fn refresh_preparation(
        &self,
        source: Arc<Source>,
        restart: bool,
    ) -> std::io::Result<()> {
        let mut loader = self.loader.lock().unwrap_or_else(|e| e.into_inner());
        let lanes = loader
            .as_mut()
            .ok_or_else(|| std::io::Error::other("Dashboard data service is closing"))?;
        let key = source.key().clone();
        if let Some(pending) = lanes.refreshes.get(&key) {
            if restart {
                pending.store(true, Ordering::Release);
            }
            return Ok(());
        }
        let restart = Arc::new(AtomicBool::new(restart));
        let job = LoadJob::PreparationRefresh {
            source,
            restart: Arc::clone(&restart),
        };
        match lanes.history.try_send(job) {
            Ok(()) => {
                lanes.refreshes.insert(key, restart);
                Ok(())
            }
            Err(TrySendError::Full(_)) => Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                crate::transport::CAPACITY_REACHED,
            )),
            Err(TrySendError::Disconnected(_)) => {
                Err(std::io::Error::other("Dashboard data service is closing"))
            }
        }
    }

    pub(super) fn start_preparation(
        &self,
        source: Arc<Source>,
        automatic: bool,
        cancelled: &AtomicBool,
    ) -> std::io::Result<()> {
        let progress = source.state::<crate::prepared::Progress>();
        let mut progress = progress.lock().unwrap_or_else(|error| error.into_inner());
        if automatic
            && self.preparation_suspended.load(Ordering::Acquire)
            && !progress.cleared
            && !self
                .sources
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .suppressed
                .contains(source.path())
        {
            // An opened file outside the configured library also resumes after exit.
            self.sources
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .library
                .insert(source.path().to_owned(), Arc::clone(&source));
            self.preparation.notify_change();
        }
        if automatic
            && (self.preparation_suspended.load(Ordering::Acquire)
                || !self.background_preparation.load(Ordering::Acquire)
                || progress.cleared
                || progress.running)
        {
            return Ok(());
        }
        if automatic
            && self
                .sources
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .suppressed
                .contains(source.path())
        {
            return Ok(());
        }
        progress.automatic = automatic;
        drop(progress);
        crate::prepared::start(&self.preparation, source, self.content(), cancelled)
    }

    pub(super) fn set_preparation_suspended(&self, suspended: bool) {
        if self.preparation_suspended.swap(suspended, Ordering::AcqRel) == suspended {
            return;
        }
        self.preparation.notify_change();
        if !suspended {
            // The library monitor retries the unaccepted tail, including sources
            // opened outside the library which were interrupted by game presence.
            return;
        }
        let _admission = self
            .preparation_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let sources: Vec<_> = self
            .sources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .values()
            .filter_map(Weak::upgrade)
            .collect();
        for source in sources {
            let progress = source.state::<crate::prepared::Progress>();
            let progress = progress.lock().unwrap_or_else(|e| e.into_inner());
            if progress.running && progress.automatic {
                drop(progress);
                crate::prepared::cancel(&source);
                self.sources
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .library
                    .insert(source.path().to_owned(), source);
            }
        }
    }

    pub(super) fn prepare_recording(&self, path: &Path) -> std::io::Result<()> {
        if self.preparation_suspended.load(Ordering::Acquire)
            || !self.background_preparation.load(Ordering::Acquire)
        {
            return Ok(());
        }
        self.check_open(&self.preparation_suspended)
            .map_err(std::io::Error::other)?;
        let path = path.canonicalize()?;
        let revision = replay_revision(&path)?;
        let result = self.prepare_recording_inner(path.clone(), revision.clone());
        if let Err(error) = &result
            && !matches!(
                error.kind(),
                std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
            )
            && !self.closed.load(Ordering::Acquire)
            && !self.preparation_suspended.load(Ordering::Acquire)
        {
            // An unchanged failed input cannot make progress on a later worker
            // completion. Only a new revision, binding or manual Start retries it.
            let mut sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
            sources.library.remove(&path);
            sources.completed.insert(path, revision);
        }
        result
    }

    fn prepare_recording_inner(&self, path: PathBuf, revision: String) -> std::io::Result<()> {
        use crate::prepared::{Progress, State};
        let previous = {
            let sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
            if sources.suppressed.contains(&path) || sources.completed.get(&path) == Some(&revision)
            {
                return Ok(());
            }
            sources.library.get(&path).cloned()
        };
        if let Some(previous) = &previous {
            let progress = previous.state::<Progress>();
            let progress = progress.lock().unwrap_or_else(|e| e.into_inner());
            if progress.running {
                if previous.key().revision != revision && progress.automatic {
                    drop(progress);
                    crate::prepared::cancel(previous);
                }
                return Ok(());
            }
        }
        // Opening may read an initial catalogue. Game polling stays independent
        // and interrupts opening before it can admit any preparation work.
        let source = self
            .open(&path, Some(&revision), &self.preparation_suspended)
            .map_err(|error| {
                if matches!(
                    error.as_str(),
                    "Dashboard request cancelled" | "Dashboard source request was cancelled"
                ) {
                    std::io::Error::new(std::io::ErrorKind::Interrupted, error)
                } else {
                    std::io::Error::other(error)
                }
            })?;
        let _admission = self
            .preparation_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if self.preparation_suspended.load(Ordering::Acquire)
            || !self.background_preparation.load(Ordering::Acquire)
        {
            return Ok(());
        }
        {
            let sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
            if sources.suppressed.contains(&path) {
                return Ok(());
            }
        }
        let progress = source.state::<Progress>();
        let state = progress.lock().unwrap_or_else(|e| e.into_inner());
        if state.cleared {
            return Ok(());
        }
        if state.running {
            drop(state);
            self.sources
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .library
                .insert(path, source);
            return Ok(());
        }
        let finished = matches!(state.state, State::Ready | State::Failed);
        drop(state);
        if finished
            || crate::prepared::unprepared_blocks(&source, self.content().as_ref())? == Some(0)
        {
            let mut sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
            sources.library.remove(&path);
            sources.completed.insert(path, revision);
            return Ok(());
        }
        if let Err(error) =
            self.start_preparation(Arc::clone(&source), true, &self.preparation_suspended)
        {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                let mut state = progress.lock().unwrap_or_else(|e| e.into_inner());
                state.state = State::Idle;
                state.sections.clear();
                state.error = None;
                drop(state);
                // Retry a full queue without reopening the initial catalogue.
                self.sources
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .library
                    .insert(path, source);
            }
            return Err(error);
        }
        self.sources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .library
            .insert(path, source);
        Ok(())
    }

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
        let interactive = match &job {
            LoadJob::Product { body, .. } => {
                products::find(&body.product).is_some_and(|product| product.interactive)
            }
            LoadJob::HistoryCache { .. } => true,
            LoadJob::PreparedQuery { .. } | LoadJob::Preparation { .. } => true,
            LoadJob::PreparationRefresh { .. } => false,
            LoadJob::PreparationPreferences { .. } => true,
            LoadJob::MapSettings(_) | LoadJob::Open(_) => false,
            #[cfg(test)]
            LoadJob::Fixture { interactive, .. } => *interactive,
        };
        let result = {
            let loader = self
                .loader
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match loader.as_ref() {
                Some(lanes) => {
                    let lane = if interactive {
                        &lanes.interactive
                    } else {
                        &lanes.history
                    };
                    match lane.try_send(job) {
                        Ok(()) => Ok(()),
                        Err(TrySendError::Full(_)) => Err(crate::transport::CAPACITY_REACHED),
                        Err(TrySendError::Disconnected(_)) => {
                            Err("Dashboard data service is closing")
                        }
                    }
                }
                None => Err("Dashboard data service is closing"),
            }
        };
        if let Err(message) = result {
            self.error(request, message.into());
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
        let campaign = source
            .as_ref()
            .map(|source| source.campaign_key())
            .unwrap_or_default();
        let key = target.key(campaign).map_err(str::to_owned)?;
        self.map_settings
            .lock()
            .map_err(|e| e.to_string())?
            .configure_game(&options.user_dir);
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
        // Any installed mod can be chosen, and a chosen mod that is no longer installed keeps
        // its place by name; what cannot be shown is named when it is mounted.
        if let Some(settings) = &settings {
            settings.validate().map_err(str::to_owned)?;
            if settings
                .graphics
                .samples
                .is_some_and(|samples| !map_settings::sample_counts().contains(&samples))
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
        if let Some(settings) = settings {
            store
                .save(key.clone(), settings)
                .map_err(|e| e.to_string())?;
        }
        let replay = source.as_ref().map(|source| source.replay_ref());
        let recorded_mods: Vec<_> = replay
            .as_ref()
            .into_iter()
            .flat_map(|replay| replay.identity.mods.iter())
            .map(|recorded| map_settings::RecordedMod {
                recorded_id: recorded.id.clone(),
                name: recorded.display_name.clone(),
            })
            .collect();
        let recorded: Vec<_> = recorded_mods
            .iter()
            .map(|recorded| installed_content::content::visual::RecordedMod {
                id: recorded.recorded_id.clone(),
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
            sample_counts: map_settings::sample_counts(),
            game_graphics: store.game().map(|game| game.graphics),
            game_settings_error: store.game_error(),
        };
        drop(store);
        // The order the resolved selection mounts in, read from the descriptors alone. The
        // settings are saved by now, so a failure to read the mods is a note, not an error.
        match installed_content::content::visual::plan(&options, &selected, &recorded, &request.cancelled) {
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

    pub(super) fn run_loader(self: &Arc<Self>, jobs: &Mutex<Receiver<LoadJob>>) {
        loop {
            let job = {
                let receiver = jobs.lock().unwrap_or_else(|error| error.into_inner());
                receiver.recv()
            };
            let Ok(job) = job else {
                return;
            };
            if let LoadJob::PreparationRefresh { source, .. } = &job
                && let Some(lanes) = self
                    .loader
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_mut()
            {
                lanes.refreshes.remove(source.key());
            }
            if self.closed.load(Ordering::Acquire) {
                return;
            }
            let request = job.request().cloned();
            let result = request_result(|| {
                self.run_job(job);
                Ok(())
            });
            if let (Err(error), Some(request)) = (result, request) {
                self.error(request, error);
            }
        }
    }

    fn run_job(&self, job: LoadJob) {
        match job {
            LoadJob::PreparationRefresh { source, restart } => {
                let restart = restart.load(Ordering::Acquire);
                let _admission = self
                    .preparation_actions
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let progress = source.state::<crate::prepared::Progress>();
                let (state, automatic) = {
                    let progress = progress.lock().unwrap_or_else(|error| error.into_inner());
                    (progress.state, progress.automatic || !progress.running)
                };
                if restart
                    && matches!(
                        state,
                        crate::prepared::State::Idle | crate::prepared::State::Cancelled
                    )
                {
                    return;
                }
                if !restart || crate::prepared::wait_cancelled(&source, &self.closed).is_ok() {
                    // Read the latest options after waiting: several Apply
                    // requests may have replaced the content while work stopped.
                    let _ = self.start_preparation(source, automatic, &self.closed);
                }
            }
            LoadJob::PreparationPreferences { request, enabled } => {
                if request.cancelled() {
                    return;
                }
                let result = request_result(|| {
                    if let Some(enabled) = enabled {
                        crate::Config::save_background_preparation(enabled)
                            .map_err(|error| error.to_string())?;
                        self.set_background_preparation(enabled);
                    }
                    bounded_json(&self.preparation_preferences(), &request.cancelled)
                });
                self.complete(request, result);
            }
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
                    return;
                }
                let Some(product) = products::find(&body.product) else {
                    self.error(request, format!("Unknown data product: {}", body.product));
                    return;
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
            LoadJob::HistoryCache {
                request,
                clear,
                options,
            } => {
                if request.cancelled() {
                    return;
                }
                let result = history_cache(&options, clear, &request.cancelled);
                self.complete(request, result);
            }
            LoadJob::PreparedQuery {
                request,
                source,
                body,
            } => {
                if request.cancelled() {
                    return;
                }
                let result = crate::prepared::query::query(
                    &source,
                    &body,
                    self.content().as_ref(),
                    &request.cancelled,
                )
                .map_err(|error| error.to_string());
                self.complete(request, result);
            }
            LoadJob::Preparation {
                request,
                source,
                source_id,
                action,
            } => {
                if request.cancelled() {
                    return;
                }
                let result = request_result(|| {
                    let content = self.content();
                    match action {
                        crate::prepared::Action::Status => {}
                        crate::prepared::Action::Start => {
                            self.preparation.notify_change();
                            let _admission = self
                                .preparation_actions
                                .lock()
                                .map_err(|error| error.to_string())?;
                            {
                                let mut sources = self.sources.lock().map_err(|e| e.to_string())?;
                                sources.suppressed.remove(source.path());
                                sources.completed.remove(source.path());
                            }
                            self.start_preparation(Arc::clone(&source), false, &request.cancelled)
                                .map_err(|error| error.to_string())?;
                        }
                        crate::prepared::Action::Cancel => {
                            self.preparation.notify_change();
                            let _admission = self
                                .preparation_actions
                                .lock()
                                .map_err(|error| error.to_string())?;
                            {
                                let mut sources = self.sources.lock().map_err(|e| e.to_string())?;
                                sources.suppressed.insert(source.path().to_owned());
                                sources.library.remove(source.path());
                            }
                            crate::prepared::cancel(&source);
                        }
                        crate::prepared::Action::Clear => {
                            self.preparation.notify_change();
                            let _admission = self
                                .preparation_actions
                                .lock()
                                .map_err(|error| error.to_string())?;
                            let revisions: Vec<_> = self
                                .sources
                                .lock()
                                .map_err(|error| error.to_string())?
                                .entries
                                .iter()
                                .filter(|((path, _), _)| path == source.path())
                                .filter_map(|(_, entry)| entry.upgrade())
                                .collect();
                            for revision in &revisions {
                                crate::prepared::cancel(revision);
                            }
                            for revision in &revisions {
                                crate::prepared::wait_cancelled(revision, &request.cancelled)
                                    .map_err(|error| error.to_string())?;
                            }
                            crate::prepared::clear(&source, content.as_ref(), &request.cancelled)
                                .map_err(|error| error.to_string())?;
                            let mut sources = self.sources.lock().map_err(|e| e.to_string())?;
                            sources.suppressed.insert(source.path().to_owned());
                            sources.completed.remove(source.path());
                            sources.library.remove(source.path());
                        }
                    }
                    let mut reply = crate::prepared::status(&source, &source_id, &request.id);
                    reply.background_enabled = self.background_preparation.load(Ordering::Acquire);
                    bounded_json(&reply, &request.cancelled)
                });
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
            #[cfg(test)]
            LoadJob::Fixture { run, .. } => run(),
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
        if let Ok(source) = &result {
            let _admission = self
                .preparation_actions
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            // Metadata and independent products remain available when a
            // background worker cannot be admitted; Start exposes its status.
            let _ = self.start_preparation(Arc::clone(source), true, &job.request.cancelled);
        }
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
            Ok(source) => {
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
            Err(error) => pending
                .errors
                .push_back((job.request, bounded_text(&error, 2048))),
        }
        drop(pending);
        self.signal.notify();
    }
}

fn request_result<T>(read: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(read))
        .unwrap_or_else(|_| Err("Dashboard request failed while reading the recording".into()))
}

/// The history cache's size, after clearing it when asked.
fn history_cache(
    options: &installed_content::ResolveOptions,
    clear: bool,
    cancelled: &AtomicBool,
) -> Result<String, String> {
    let folder = reply_cache::folder(options);
    if clear {
        reply_cache::clear(&folder).map_err(|error| error.to_string())?;
    }
    let reply = HistoryCacheReply {
        bytes: reply_cache::size(&folder),
        limit: reply_cache::LIMIT,
    };
    bounded_json(&reply, cancelled)
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
    PreparationRefresh {
        source: Arc<Source>,
        restart: Arc<AtomicBool>,
    },
    PreparationPreferences {
        request: Arc<Request>,
        enabled: Option<bool>,
    },
    PreparedQuery {
        request: Arc<Request>,
        source: Arc<Source>,
        body: ProductRequest,
    },
    Preparation {
        request: Arc<Request>,
        source: Arc<Source>,
        source_id: String,
        action: crate::prepared::Action,
    },
    #[cfg(test)]
    Fixture {
        request: Arc<Request>,
        interactive: bool,
        run: Box<dyn FnOnce() + Send>,
    },
    MapSettings(MapSettingsJob),
    HistoryCache {
        request: Arc<Request>,
        clear: bool,
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

impl LoadJob {
    fn request(&self) -> Option<&Arc<Request>> {
        Some(match self {
            Self::PreparationRefresh { .. } => return None,
            Self::MapSettings(job) => &job.request,
            Self::Open(job) => &job.request,
            Self::HistoryCache { request, .. } | Self::Product { request, .. } => request,
            Self::PreparedQuery { request, .. } | Self::Preparation { request, .. } => request,
            Self::PreparationPreferences { request, .. } => request,
            #[cfg(test)]
            Self::Fixture { request, .. } => request,
        })
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
                "This recording is damaged or isn't a Battle Plan recording. Details: synthetic reason"
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
