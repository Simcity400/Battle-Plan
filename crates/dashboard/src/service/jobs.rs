use super::{Inner, Lane, Lease, Request, queue::Receiver};
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
        mpsc::TrySendError,
    },
};

#[derive(Default)]
pub(super) struct Sources {
    next: u64,
    // Only jobs that may need resuming hold a library source. Completed revisions
    // retain a cheap stamp; explicit Cancel/Clear suppress ticks across revisions.
    pub(super) library: BTreeMap<PathBuf, Arc<Source>>,
    pub(super) completed: BTreeMap<PathBuf, String>,
    pub(super) suppressed: BTreeSet<PathBuf>,
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

/// The volume's room as the library scan last measured it, and how often it
/// measures, which bounds how old a refresh lets the measurement get.
#[derive(Default)]
pub(super) struct Room {
    pub(super) room: bool,
    pub(super) measured: Option<std::time::Instant>,
    pub(super) interval: Option<std::time::Duration>,
}

#[cfg(test)]
thread_local! {
    /// Run by the scan between reading a store and admitting it, so tests can
    /// change the content binding or the store in between.
    pub(super) static TEST_AFTER_CHECK: std::cell::RefCell<Option<Box<dyn FnOnce(&super::Inner)>>> =
        const { std::cell::RefCell::new(None) };
}

/// What an admission's caller read before taking the admission lock, so
/// admission does no store or volume I/O: the store's restore outcome at one
/// store epoch, and whether the volume has room for library work. The outcome
/// is used only under the admission lock while the source's epoch is
/// unchanged (`publish_prepared`, `prepared::start`); otherwise it is dropped
/// and the store is prepared.
pub(super) struct Checked {
    pub(super) epoch: u64,
    pub(super) restored: std::io::Result<Option<crate::prepared::pipeline::Accepted>>,
    pub(super) room: bool,
    /// The store's manifest identity, read before the restore (`None` when it
    /// could not be read), to notice another writer after publication.
    pub(super) identity: StoreIdentity,
}

/// A store's on-disk manifest identity (`store::Store::manifest_identity`);
/// `None` when it could not be read, which never matches.
pub(super) type StoreIdentity = Option<Option<(u64, u64, u64)>>;

impl Checked {
    /// Nothing read: an admitted preparation reads the store itself.
    pub(super) fn unread(room: bool) -> Self {
        Self {
            epoch: 0,
            restored: Ok(None),
            room,
            identity: None,
        }
    }

    /// What confirming a publication from this check needs: only a check that
    /// found a complete store can be published.
    fn confirmation(&self) -> Option<(u64, StoreIdentity)> {
        matches!(self.restored, Ok(Some(_))).then_some((self.epoch, self.identity))
    }
}

fn store_identity(source: &Source) -> StoreIdentity {
    crate::prepared::recording_store(source.path())
        .and_then(|store| store.manifest_identity())
        .ok()
}

/// The modification time a revision stamp (`len:secs:nanos`) records, for
/// ordering recordings by age without reading files.
fn modified(revision: &str) -> Option<(i64, u32)> {
    let mut parts = revision.split(':').skip(1);
    let seconds = parts.next()?.parse().ok()?;
    let nanos = parts.next()?.parse().ok()?;
    Some((seconds, nanos))
}

/// Who admitted a preparation, which decides what it yields to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Admission {
    /// The library scan, ahead of the user: it yields to the other two and
    /// lowers its threads.
    Library,
    /// Opening the recording. Like the library scan it is automatic: the
    /// background preference and the game guard apply.
    Opened,
    /// An explicit Prepare.
    Manual,
}

impl Admission {
    fn automatic(self) -> bool {
        self != Self::Manual
    }
}

/// The first sentence is suitable for the sidebar; retain the technical reason
/// after it for bug reports. Cancellation and ordinary filesystem errors keep
/// their existing meaning.
fn recording_open_error(error: std::io::Error) -> String {
    let sentence = if engine_allocation::is_allocation_error(&error) {
        "There isn't enough memory to open this recording."
    } else if matches!(
        error.kind(),
        std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof
    ) {
        "This recording is damaged or isn't a Battle Plan recording."
    } else {
        return error.to_string();
    };
    replay_data::codec::try_format(format_args!("{sentence} Details: {error}"))
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
        let sources: Vec<_> = self
            .sources
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entries
            .values()
            .filter_map(Weak::upgrade)
            .collect();
        if !enabled {
            let _admission = self
                .preparation_actions
                .lock()
                .unwrap_or_else(|error| error.into_inner());
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
        // Revisions are read without the admission lock; each refresh takes it
        // when it runs.
        let mut latest = BTreeMap::new();
        for source in sources {
            if self.recording_revision(source.path()).ok().as_deref()
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
        match lanes.history.try_send(false, job) {
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

    /// Admit `source`'s preparation. Ownership is decided here alone, at
    /// admission time and under the admission lock every caller holds: the
    /// scan, refreshes and other automatic callers pass `Library`, which runs
    /// as the user's (`Opened`) exactly when the user holds the recording now
    /// ([`Self::held`]); only the user's own actions pass `Opened` or
    /// `Manual`. Library work waits while the user's preparation is queued or
    /// running and stops at the free-space floor.
    pub(super) fn start_preparation(
        &self,
        source: Arc<Source>,
        admission: Admission,
        checked: Checked,
        cancelled: &AtomicBool,
    ) -> std::io::Result<()> {
        // Read the lease table and the other jobs before this source's
        // progress lock, which neither is taken under. The volume and the
        // store were read by the caller before the admission lock.
        let library = admission == Admission::Library;
        let viewed = library && self.viewed(source.path());
        let interactive = library && self.interactive_preparation();
        let room = !library || checked.room;
        let suppressed = self
            .sources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .suppressed
            .contains(source.path());
        let suspended = self.preparation_suspended.load(Ordering::Acquire);
        let enabled = self.background_preparation.load(Ordering::Acquire);
        let progress = source.state::<crate::prepared::Progress>();
        let mut progress = progress.lock().unwrap_or_else(|error| error.into_inner());
        let admission = if library && (progress.claimed || viewed) {
            Admission::Opened
        } else {
            admission
        };
        let automatic = admission.automatic();
        let user = admission != Admission::Library;
        if user && progress.running {
            // The user joins the job, even while a pause is still stopping it:
            // it resumes as the user's, beyond the library's order, floor and
            // priority.
            progress.claimed = true;
            progress.background.store(false, Ordering::Release);
        }
        if automatic && suspended && !progress.cleared && !suppressed {
            // Kept for after the game, opened files outside the library too.
            self.sources
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .library
                .insert(source.path().to_owned(), Arc::clone(&source));
            self.preparation.notify_change();
            if user && enabled {
                progress.claimed = true;
                progress.background.store(false, Ordering::Release);
            }
        }
        if automatic
            && (suspended || !enabled || progress.cleared || progress.running || suppressed)
        {
            return Ok(());
        }
        if admission == Admission::Library && (interactive || !room) {
            return Ok(());
        }
        progress.automatic = automatic;
        progress.claimed |= user;
        progress
            .background
            .store(admission == Admission::Library, Ordering::Release);
        drop(progress);
        // `prepared::start` checks the outcome's epoch in this critical section.
        let started = crate::prepared::start(
            &self.preparation,
            source,
            self.content(),
            checked.restored,
            checked.epoch,
            cancelled,
        );
        // Every library admission, from the scan or a refresh, keeps
        // newest-first.
        if started.is_ok() && admission == Admission::Library {
            self.enforce_newest_first_locked();
        }
        started
    }

    /// Read a recording's revision. Never under the admission lock.
    pub(super) fn recording_revision(&self, path: &Path) -> std::io::Result<String> {
        #[cfg(test)]
        {
            self.test_revisions.fetch_add(1, Ordering::Relaxed);
            if self.preparation_actions.try_lock().is_err() {
                self.test_revisions_locked.fetch_add(1, Ordering::Relaxed);
            }
        }
        replay_revision(path)
    }

    /// Confirm a publication from a check outside the admission lock: if the
    /// store's manifest changed since the check (another service or process
    /// cleared or replaced it), withdraw the Ready it published, forget the
    /// revision as finished and keep the source for the next scan, which
    /// prepares it. Eventually consistent: a reader may see Ready until then.
    /// Nothing is withdrawn once the epoch moved on (a job, a rebind, a Clear),
    /// as that path reports its own outcome.
    fn confirm_checked(&self, source: &Arc<Source>, confirmation: Option<(u64, StoreIdentity)>) {
        let Some((epoch, identity)) = confirmation else {
            return;
        };
        if identity.is_some() && store_identity(source) == identity {
            return;
        }
        let _admission = self
            .preparation_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        {
            let progress = source.state::<crate::prepared::Progress>();
            let mut progress = progress.lock().unwrap_or_else(|e| e.into_inner());
            if progress.running
                || progress.store_epoch != epoch
                || progress.state != crate::prepared::State::Ready
            {
                return;
            }
            crate::prepared::invalidate(&mut progress);
            progress.state = crate::prepared::State::Idle;
            progress.sections.clear();
            progress.coverage.clear();
            progress.bytes = 0;
        }
        let mut sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
        sources.completed.remove(source.path());
        sources
            .library
            .insert(source.path().to_owned(), Arc::clone(source));
        drop(sources);
        self.preparation.notify_change();
    }

    /// The user's Cancel: the job stops, the path is suppressed from automatic
    /// retries, and work captured before it never overrides it. The caller
    /// holds the admission lock.
    pub(super) fn cancel_by_user(&self, source: &Source) {
        {
            let mut sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
            sources.suppressed.insert(source.path().to_owned());
            sources.library.remove(source.path());
        }
        {
            let progress = source.state::<crate::prepared::Progress>();
            let mut progress = progress.lock().unwrap_or_else(|e| e.into_inner());
            progress.user_stops = progress.user_stops.wrapping_add(1);
        }
        crate::prepared::cancel(source);
    }

    /// Restore `source`'s store for the current content, outside the admission
    /// lock; the caller passes the outcome to admission. The epoch is read
    /// before the content, so a rebind in between shows as a changed epoch.
    pub(super) fn check_store(&self, source: &Source, room: bool) -> Checked {
        let epoch = crate::prepared::store_epoch(source);
        let content = self.content();
        let identity = store_identity(source);
        Checked {
            epoch,
            restored: crate::prepared::pipeline::restore(source, content.as_ref(), &self.closed),
            room,
            identity,
        }
    }

    /// Invalidate every store check taken so far, for a content rebind: every
    /// live source's epoch moves on and no revision is known finished. The
    /// caller holds the admission lock.
    pub(super) fn invalidate_checks(&self) {
        let sources: Vec<_> = {
            let mut held = self.sources.lock().unwrap_or_else(|e| e.into_inner());
            held.completed.clear();
            held.entries.values().filter_map(Weak::upgrade).collect()
        };
        for source in sources {
            let progress = source.state::<crate::prepared::Progress>();
            crate::prepared::invalidate(&mut progress.lock().unwrap_or_else(|e| e.into_inner()));
        }
    }

    /// Of the jobs running as library work now, keep the newest recording's
    /// and pause the rest; the caller holds no lock. Recording age comes from
    /// each source's revision stamp, so this reads no files.
    pub(super) fn enforce_newest_first(&self) {
        let _admission = self
            .preparation_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        self.enforce_newest_first_locked();
    }

    /// [`Self::enforce_newest_first`] for a caller holding the admission lock.
    fn enforce_newest_first_locked(&self) {
        let sources: Vec<_> = self
            .sources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .values()
            .filter_map(Weak::upgrade)
            .collect();
        let mut library: Vec<_> = sources
            .into_iter()
            .filter(|source| {
                let progress = source.state::<crate::prepared::Progress>();
                let progress = progress.lock().unwrap_or_else(|e| e.into_inner());
                progress.running
                    && progress.background.load(Ordering::Acquire)
                    && !progress.cancelled.load(Ordering::Acquire)
            })
            .collect();
        if library.len() < 2 {
            return;
        }
        library.sort_by_cached_key(|source| {
            (
                std::cmp::Reverse(modified(&source.key().revision)),
                source.path().to_owned(),
            )
        });
        for older in &library[1..] {
            crate::prepared::pause(older, crate::prepared::PauseReason::NewerRecording);
        }
        self.preparation.notify_change();
    }

    /// Measure the volume's room for library work. Never under the admission
    /// lock: a slow or network volume must not stall opens, preparation
    /// actions or game pauses.
    fn measure_room(&self) -> bool {
        #[cfg(test)]
        {
            self.test_measures.fetch_add(1, Ordering::Relaxed);
            if self.preparation_actions.try_lock().is_err() {
                self.test_measures_locked.fetch_add(1, Ordering::Relaxed);
            }
        }
        super::has_room()
    }

    /// The scan's measurement, once per scan, before it admits anything.
    pub(super) fn room_for_scan(&self) -> bool {
        let room = self.measure_room();
        let now = std::time::Instant::now();
        let mut last = self.room.lock().unwrap_or_else(|e| e.into_inner());
        last.interval = last.measured.map(|measured| now.duration_since(measured));
        last.measured = Some(now);
        last.room = room;
        room
    }

    /// A refresh's measurement: the scan's last one, measured again outside
    /// the admission lock once it is older than the scans' own interval.
    pub(super) fn room_for_refresh(&self) -> bool {
        let fresh = {
            let last = self.room.lock().unwrap_or_else(|e| e.into_inner());
            match (last.measured, last.interval) {
                (Some(measured), Some(interval)) if measured.elapsed() <= interval => {
                    Some(last.room)
                }
                _ => None,
            }
        };
        fresh.unwrap_or_else(|| self.measure_room())
    }

    /// Whether an open view holds a source of the recording at `path`.
    pub(super) fn viewed(&self, path: &Path) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .leases
            .values()
            .any(|lease| lease.source.path() == path)
    }

    /// Whether the user currently holds `source`'s recording: an open view, or
    /// a job they claimed that has not reached a terminal state. A retained
    /// source's classification says nothing about current demand.
    pub(super) fn held(&self, source: &Source) -> bool {
        source
            .state::<crate::prepared::Progress>()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .claimed
            || self.viewed(source.path())
    }

    /// Publish a store the scan restored (another writer may have completed
    /// it, or it completed while deferred) as Ready with its sections,
    /// clearing any pause or error, under the admission lock. Whether it was
    /// published: a job that started meanwhile reports its own outcome.
    fn publish_prepared(
        &self,
        source: &Source,
        accepted: crate::prepared::pipeline::Accepted,
        epoch: u64,
    ) -> bool {
        let progress = source.state::<crate::prepared::Progress>();
        let mut progress = progress.lock().unwrap_or_else(|e| e.into_inner());
        // A check the epoch has moved past (a rebind, a Clear, a write) is
        // dropped, in the same critical section that would publish it.
        if progress.running || progress.store_epoch != epoch {
            return false;
        }
        crate::prepared::finish(&mut progress, crate::prepared::State::Ready);
        progress.error = None;
        progress.sections = accepted.sections;
        progress.coverage = accepted.coverage;
        progress.bytes = accepted.bytes;
        true
    }

    /// Whether preparation the user asked for (opened or explicit) is queued
    /// or running. Library admission waits until none is.
    pub(super) fn interactive_preparation(&self) -> bool {
        let sources: Vec<_> = self
            .sources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .values()
            .filter_map(Weak::upgrade)
            .collect();
        sources.iter().any(|source| {
            let progress = source.state::<crate::prepared::Progress>();
            let progress = progress.lock().unwrap_or_else(|e| e.into_inner());
            progress.running && !progress.background.load(Ordering::Acquire)
        })
    }

    /// Make way for the user's preparation of `source`, under the admission
    /// lock. Its own running job is joined and stops yielding; a job of another
    /// revision of the same recording is cancelled and waited for, since only
    /// one writer can hold the recording's store; library jobs of other
    /// recordings pause at their next block boundary and are kept for the
    /// library scan, which resumes them from their accepted prefix once no
    /// preparation of the user's runs. Returns the jobs that yielded.
    pub(super) fn take_over_preparation(
        &self,
        source: &Arc<Source>,
        cancelled: &AtomicBool,
    ) -> std::io::Result<Vec<Arc<Source>>> {
        let sources: Vec<_> = self
            .sources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .values()
            .filter_map(Weak::upgrade)
            .collect();
        let (mut yielded, mut replaced) = (Vec::new(), Vec::new());
        for other in sources {
            let progress = other.state::<crate::prepared::Progress>();
            let mut progress = progress.lock().unwrap_or_else(|e| e.into_inner());
            if !progress.running {
                continue;
            }
            if Arc::ptr_eq(&other, source) {
                progress.claimed = true;
                progress.background.store(false, Ordering::Release);
            } else if other.path() == source.path() {
                drop(progress);
                replaced.push(other);
            } else if progress.background.load(Ordering::Acquire) {
                drop(progress);
                yielded.push(other);
            }
        }
        if yielded.is_empty() && replaced.is_empty() {
            return Ok(yielded);
        }
        self.preparation.notify_change();
        for other in &yielded {
            crate::prepared::pause(other, crate::prepared::PauseReason::Requested);
        }
        for other in &replaced {
            crate::prepared::cancel(other);
        }
        {
            let mut sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
            for other in &yielded {
                sources
                    .library
                    .entry(other.path().to_owned())
                    .or_insert_with(|| Arc::clone(other));
            }
        }
        for other in &replaced {
            crate::prepared::wait_cancelled(other, cancelled)?;
        }
        Ok(yielded)
    }

    /// Admit the user's preparation of `source` after making way for it. A
    /// full queue is retried once after the jobs that yielded have left it.
    pub(super) fn admit_interactive(
        &self,
        source: &Arc<Source>,
        admission: Admission,
        checked: Checked,
        cancelled: &AtomicBool,
    ) -> std::io::Result<()> {
        let yielded = self.take_over_preparation(source, cancelled)?;
        let room = checked.room;
        match self.start_preparation(Arc::clone(source), admission, checked, cancelled) {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock && !yielded.is_empty() => {
                for other in &yielded {
                    crate::prepared::wait_finished(other, cancelled)?;
                }
                // Only a store that needed preparing reaches the queue.
                self.start_preparation(
                    Arc::clone(source),
                    admission,
                    Checked::unread(room),
                    cancelled,
                )
            }
            result => result,
        }
    }

    /// Pause library work for `reason` under the admission lock. Each job is
    /// checked again there: an open or an explicit Prepare may have made it
    /// the user's since the caller looked, and the user's work is never paused
    /// by the library's bounds.
    pub(super) fn pause_library<'a>(
        &self,
        sources: impl IntoIterator<Item = &'a Arc<Source>>,
        reason: crate::prepared::PauseReason,
    ) {
        let _admission = self
            .preparation_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut paused = false;
        for source in sources {
            let library = {
                let progress = source.state::<crate::prepared::Progress>();
                let progress = progress.lock().unwrap_or_else(|e| e.into_inner());
                progress.running && progress.background.load(Ordering::Acquire)
            };
            if library {
                crate::prepared::pause(source, reason);
                paused = true;
            }
        }
        if paused {
            self.preparation.notify_change();
        }
    }

    /// Show an open the game defers as paused for the game, so its panel keeps
    /// checking and follows the start once the game exits. A store that is
    /// already up to date needs nothing, and a running job reports its own
    /// pause when it stops. A store that cannot be checked is not known to be
    /// prepared, so it waits too; its preparation reports any real failure.
    fn show_deferred(
        &self,
        source: &Arc<Source>,
        restored: std::io::Result<Option<crate::prepared::pipeline::Accepted>>,
        epoch: u64,
    ) {
        use crate::prepared::{PauseReason, Progress, State};
        if !self.preparation_suspended.load(Ordering::Acquire)
            || !self.background_preparation.load(Ordering::Acquire)
            || !self
                .sources
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .library
                .get(source.path())
                .is_some_and(|held| Arc::ptr_eq(held, source))
        {
            return;
        }
        // A complete store, read before the admission lock, is shown Ready.
        if let Ok(Some(accepted)) = restored
            && self.publish_prepared(source, accepted, epoch)
        {
            return;
        }
        let progress = source.state::<Progress>();
        let mut progress = progress.lock().unwrap_or_else(|e| e.into_inner());
        if !progress.running && matches!(progress.state, State::Idle | State::Cancelled) {
            progress.state = State::Paused;
            progress.paused = Some(PauseReason::Game);
            progress.error = None;
        }
    }

    /// Whether opening `source` admits its preparation now.
    fn opening_admits(&self, source: &Source) -> bool {
        self.background_preparation.load(Ordering::Acquire)
            && !self.preparation_suspended.load(Ordering::Acquire)
            && !self
                .sources
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .suppressed
                .contains(source.path())
            && !source
                .state::<crate::prepared::Progress>()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .cleared
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
                crate::prepared::pause(&source, crate::prepared::PauseReason::Game);
                self.sources
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .library
                    .insert(source.path().to_owned(), source);
            }
        }
    }

    /// Admit one library recording for the scan, which measured the volume's
    /// `room` before taking any lock.
    pub(super) fn prepare_recording(&self, path: &Path, room: bool) -> std::io::Result<()> {
        if self.preparation_suspended.load(Ordering::Acquire)
            || !self.background_preparation.load(Ordering::Acquire)
        {
            return Ok(());
        }
        self.check_open(&self.preparation_suspended)
            .map_err(std::io::Error::other)?;
        let path = path.canonicalize()?;
        let revision = self.recording_revision(&path)?;
        let result = self.prepare_recording_inner(path.clone(), revision.clone(), room);
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

    fn prepare_recording_inner(
        &self,
        path: PathBuf,
        revision: String,
        room: bool,
    ) -> std::io::Result<()> {
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
        let progress = source.state::<Progress>();
        // Check the store before taking the admission lock, so restoring many
        // recordings on a first scan never delays opens, preparation actions
        // or game pauses. Only a restore that succeeds shows the recording
        // prepared; a store that cannot be checked or restored is not known to
        // be prepared, and its preparation succeeds or reports the real
        // failure through the ordinary terminal failure.
        let idle = {
            let state = progress.lock().unwrap_or_else(|e| e.into_inner());
            !state.running && !matches!(state.state, State::Ready | State::Failed)
        };
        let checked = if idle {
            self.check_store(&source, room)
        } else {
            Checked::unread(room)
        };
        #[cfg(test)]
        if let Some(hook) = TEST_AFTER_CHECK.with(|hook| hook.borrow_mut().take()) {
            hook(self);
        }
        let confirmation = checked.confirmation();
        let admission = self
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
        let Checked {
            epoch,
            restored,
            room,
            identity,
        } = checked;
        let restored = match restored {
            Ok(Some(accepted)) if !finished => {
                if self.publish_prepared(&source, accepted, epoch) {
                    {
                        let mut sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
                        sources.library.remove(&path);
                        sources.completed.insert(path, revision);
                    }
                    drop(admission);
                    self.confirm_checked(&source, confirmation);
                    return Ok(());
                }
                // Read for another content binding, or a job started: admit as
                // usual, and the preparation reads the store itself.
                Ok(None)
            }
            other => other,
        };
        if finished {
            let mut sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
            sources.library.remove(&path);
            sources.completed.insert(path, revision);
            return Ok(());
        }
        if let Err(error) = self.start_preparation(
            Arc::clone(&source),
            Admission::Library,
            Checked {
                epoch,
                restored,
                room,
                identity,
            },
            &self.preparation_suspended,
        ) {
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
        let revision = self
            .recording_revision(&path)
            .map_err(|error| error.to_string())?;
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
        let lane = match &job {
            LoadJob::Product { body, .. } => {
                if products::find(&body.product).is_some_and(|product| product.interactive) {
                    Lane::Interactive
                } else {
                    Lane::History
                }
            }
            LoadJob::PreparedQuery { .. } => Lane::Prepared,
            LoadJob::HistoryCache { .. }
            | LoadJob::Preparation { .. }
            | LoadJob::PreparationPreferences { .. } => Lane::Interactive,
            LoadJob::PreparationRefresh { .. } | LoadJob::MapSettings(_) | LoadJob::Open(_) => {
                Lane::History
            }
            #[cfg(test)]
            LoadJob::Fixture { lane, .. } => *lane,
        };
        self.submit_to(lane, request, job);
    }

    /// Queue `job` on `lane`, or refuse it with the retryable capacity error
    /// when the lane is full.
    pub(super) fn submit_to(&self, lane: Lane, request: Arc<Request>, job: LoadJob) {
        let result = {
            let loader = self
                .loader
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match loader.as_ref() {
                Some(lanes) => match lanes.sender(lane).try_send(job.is_refresh(), job) {
                    Ok(()) => Ok(()),
                    Err(TrySendError::Full(_)) => Err(crate::transport::CAPACITY_REACHED),
                    Err(TrySendError::Disconnected(_)) => Err("Dashboard data service is closing"),
                },
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
        // A flip reads the value it flips here, under the lock every save takes, so an edit or
        // reset saved before it is what it flips and no other choice is written.
        if let Some(setting) = flip {
            store
                .flip(key.clone(), setting)
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
            .map(|recorded| installed_content_core::content::visual::RecordedMod {
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
        match installed_content_core::content::visual::plan(
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
            // Measurement only: the job's CPU by product, when enabled.
            let _cpu = match &job {
                LoadJob::Product { body, .. } => {
                    ::replay_frame::cpu::scope_named("product load", &body.product)
                }
                LoadJob::PreparedQuery { body, .. } => {
                    ::replay_frame::cpu::scope_named("prepared query", &body.product)
                }
                LoadJob::Open(_) => ::replay_frame::cpu::scope("service job", Some("open")),
                _ => ::replay_frame::cpu::scope("service job", Some("other")),
            };
            let result = request_result(|| {
                self.run_job(job, lane);
                Ok(())
            });
            if let (Err(error), Some(request)) = (result, request) {
                self.error(request, error);
            }
        }
    }

    /// `answer` as `lane` runs it: on [`Lane::Prepared`] without decoding
    /// recorded blocks, `None` when it needed to and must run on [`Lane::Sampled`].
    fn attempt<T>(lane: Lane, answer: impl FnOnce() -> T) -> Option<T> {
        if lane != Lane::Prepared {
            return Some(answer());
        }
        let (answer, refused) = crate::prepared::query::progressive::without_block_decodes(answer);
        (!refused).then_some(answer)
    }

    fn run_job(&self, job: LoadJob, lane: Lane) {
        match job {
            LoadJob::PreparationRefresh { source, restart } => {
                let restart = restart.load(Ordering::Acquire);
                let progress = source.state::<crate::prepared::Progress>();
                let (admission, user_stops) = {
                    let _admission = self
                        .preparation_actions
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    // Whose job it is, read before cancelling clears `running`:
                    // a restart of the user's Prepare restarts as theirs.
                    let (state, manual, user_stops) = {
                        let progress = progress.lock().unwrap_or_else(|error| error.into_inner());
                        (
                            progress.state,
                            progress.running && !progress.automatic,
                            progress.user_stops,
                        )
                    };
                    // Paused library work resumes through the library scan,
                    // which yields to the user's preparation.
                    if state == crate::prepared::State::Paused
                        || restart
                            && matches!(
                                state,
                                crate::prepared::State::Idle | crate::prepared::State::Cancelled
                            )
                    {
                        return;
                    }
                    if restart && crate::prepared::wait_cancelled(&source, &self.closed).is_err() {
                        return;
                    }
                    // Library otherwise: admission promotes it when the user
                    // holds the recording.
                    let admission = if manual {
                        Admission::Manual
                    } else {
                        Admission::Library
                    };
                    (admission, user_stops)
                };
                // Read the store and the volume outside the admission lock, for
                // the latest options: several Apply requests may have replaced
                // the content while work stopped.
                let checked = self.check_store(&source, self.room_for_refresh());
                #[cfg(test)]
                if let Some(hook) = self
                    .test_refresh_checked
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                {
                    hook(self);
                }
                let confirmation = checked.confirmation();
                let lock = self
                    .preparation_actions
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                // A Cancel or Clear by the user since the capture wins over it.
                let stopped = {
                    let progress = progress.lock().unwrap_or_else(|error| error.into_inner());
                    progress.user_stops != user_stops || progress.cleared
                };
                if !stopped {
                    let _ = self.start_preparation(
                        Arc::clone(&source),
                        admission,
                        checked,
                        &self.closed,
                    );
                }
                drop(lock);
                if !stopped {
                    self.confirm_checked(&source, confirmation);
                }
                #[cfg(test)]
                self.test_refreshes.fetch_add(1, Ordering::Release);
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
                let answer = Self::attempt(lane, || {
                    crate::prepared::query::query(
                        &source,
                        &body,
                        self.content().as_ref(),
                        &request.cancelled,
                    )
                    .map_err(|error| error.to_string())
                });
                match answer {
                    Some(result) => self.complete(request, result),
                    // The answer samples recorded blocks: it waits for a sampled-read
                    // loader, and nothing this attempt read is kept.
                    None => self.submit_to(
                        Lane::Sampled,
                        Arc::clone(&request),
                        LoadJob::PreparedQuery {
                            request,
                            source,
                            body,
                        },
                    ),
                }
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
                            let checked = self.check_store(&source, true);
                            let confirmation = checked.confirmation();
                            let admission = self
                                .preparation_actions
                                .lock()
                                .map_err(|error| error.to_string())?;
                            {
                                let mut sources = self.sources.lock().map_err(|e| e.to_string())?;
                                sources.suppressed.remove(source.path());
                                sources.completed.remove(source.path());
                            }
                            let admitted = self.admit_interactive(
                                &source,
                                Admission::Manual,
                                checked,
                                &request.cancelled,
                            );
                            drop(admission);
                            self.confirm_checked(&source, confirmation);
                            admitted.map_err(|error| error.to_string())?;
                        }
                        crate::prepared::Action::Cancel => {
                            self.preparation.notify_change();
                            let _admission = self
                                .preparation_actions
                                .lock()
                                .map_err(|error| error.to_string())?;
                            self.cancel_by_user(&source);
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
                                let progress = revision.state::<crate::prepared::Progress>();
                                {
                                    let mut progress =
                                        progress.lock().unwrap_or_else(|e| e.into_inner());
                                    progress.user_stops = progress.user_stops.wrapping_add(1);
                                }
                                crate::prepared::cancel(revision);
                            }
                            for revision in &revisions {
                                crate::prepared::wait_cancelled(revision, &request.cancelled)
                                    .map_err(|error| error.to_string())?;
                            }
                            let cleared = crate::prepared::clear(
                                &source,
                                content.as_ref(),
                                &request.cancelled,
                            );
                            // Checks of any revision read the store Clear removes.
                            for revision in &revisions {
                                let progress = revision.state::<crate::prepared::Progress>();
                                crate::prepared::invalidate(
                                    &mut progress.lock().unwrap_or_else(|e| e.into_inner()),
                                );
                            }
                            cleared.map_err(|error| error.to_string())?;
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
            LoadJob::Fixture { request, run, .. } => {
                if Self::attempt(lane, || run()).is_none() {
                    self.submit_to(
                        Lane::Sampled,
                        Arc::clone(&request),
                        LoadJob::Fixture {
                            request,
                            lane: Lane::Sampled,
                            run,
                        },
                    );
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
        if let Ok(source) = &result {
            // Read the store before the admission lock.
            let checked = self.check_store(source, true);
            let confirmation = checked.confirmation();
            let admission = self
                .preparation_actions
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            // Metadata and independent products remain available when a
            // background worker cannot be admitted; Start exposes its status.
            if self.opening_admits(source) {
                let _ = self.admit_interactive(
                    source,
                    Admission::Opened,
                    checked,
                    &job.request.cancelled,
                );
            } else {
                // Not now (a running game, the preference, Clear or Cancel):
                // starting it keeps a recording the game defers for after the
                // game exits, and admits nothing otherwise.
                let Checked {
                    epoch,
                    restored,
                    room,
                    ..
                } = checked;
                let _ = self.start_preparation(
                    Arc::clone(source),
                    Admission::Opened,
                    Checked::unread(room),
                    &job.request.cancelled,
                );
                self.show_deferred(source, restored, epoch);
            }
            drop(admission);
            self.confirm_checked(source, confirmation);
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
    options: &installed_content_core::ResolveOptions,
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
    /// A setting to flip under the store's lock, the opposite of its value when it runs.
    pub(super) flip: Option<crate::protocol::MapSetting>,
    pub(super) options: installed_content_core::ResolveOptions,
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
    /// A test job on `lane`, run again on [`Lane::Sampled`] when its run on
    /// [`Lane::Prepared`] refused a block decode.
    #[cfg(test)]
    Fixture {
        request: Arc<Request>,
        lane: Lane,
        run: Arc<dyn Fn() + Send + Sync>,
    },
    MapSettings(MapSettingsJob),
    HistoryCache {
        request: Arc<Request>,
        clear: bool,
        options: installed_content_core::ResolveOptions,
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
    fn is_refresh(&self) -> bool {
        self.request()
            .is_some_and(|request| request.refresh.load(Ordering::Acquire))
    }
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
