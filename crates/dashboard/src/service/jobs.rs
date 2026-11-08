use super::{Inner, Lane, Lease, Request, queue::Receiver};
use crate::{
    Source, SourceKey, map_settings,
    products::{self, ContentOptions, LoadContext},
    protocol::{CampaignMapSettings, replay_revision},
    transport::{MapSettingsTarget, ProductRequest, SourceReply, bounded_json, bounded_text},
};
use job_priority::SpawnCarried as _;
use std::{
    collections::{BTreeMap, BTreeSet},
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, TryLockError, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc::TrySendError,
    },
};

/// What a request's work is: content work, cancelled by the request, or work that reads no
/// content, which reads the recording's own artwork and never mounts a binding's.
fn work(content_dependent: bool, request: &Arc<Request>) -> crate::source::Work {
    if content_dependent {
        crate::source::Work::Content(Arc::clone(request) as Arc<dyn crate::presentation::Cancel>)
    } else {
        crate::source::Work::Recorded
    }
}

#[derive(Default)]
pub(super) struct Sources {
    next: u64,
    // Only jobs that may need resuming hold a library source. Completed revisions
    // retain a cheap stamp; explicit Cancel/Clear suppress ticks across revisions.
    pub(super) library: BTreeMap<PathBuf, Arc<Source>>,
    /// Each finished recording's revision and its native recordings' stamp.
    pub(super) completed: BTreeMap<PathBuf, String>,
    pub(super) suppressed: BTreeSet<PathBuf>,
    pub(super) entries: BTreeMap<(PathBuf, String), Weak<Source>>,
    /// Sources opening: registered before their first binding so every configuration writer
    /// rebinds them too, and handed to no one until they are bound ([`Inner::bound_sources`]).
    pub(super) binding: Vec<Weak<Source>>,
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

/// The volume's free bytes as the library scan last measured it (None when unmeasured), and
/// how often it measures, which bounds how old a refresh lets the measurement get.
#[derive(Default)]
pub(super) struct Room {
    pub(super) free: Option<u64>,
    pub(super) measured: Option<std::time::Instant>,
    pub(super) interval: Option<std::time::Duration>,
}

#[cfg(test)]
type InnerHook = Box<dyn FnOnce(&super::Inner)>;

#[cfg(test)]
thread_local! {
    /// Run by the scan between reading a store and admitting it, so tests can
    /// change the content binding or the store in between.
    pub(super) static TEST_AFTER_CHECK: std::cell::RefCell<Option<InnerHook>> =
        const { std::cell::RefCell::new(None) };
}

/// What an admission's caller read before taking the admission lock, so
/// admission does no store or volume I/O: the store's restore outcome at one
/// store epoch, and whether the volume has room for its library work
/// ([`crate::prepared::space`]). The outcome
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

/// Whether `free` bytes cover what `source`'s store still needs, and the margin.
fn library_room(source: &Source, free: Option<u64>) -> std::io::Result<bool> {
    if free.is_none() {
        return Ok(true);
    }
    let folder = dashboard_base::prepared::recording_folder(source)?;
    let needed = crate::prepared::space::needed_for(source.path(), &folder).map_err(|error| {
        std::io::Error::other(format!("Its store's size can't be estimated: {error}"))
    })?;
    Ok(crate::prepared::space::admits(free, needed))
}

/// A check whose room is `room`; an estimate that failed fails the preparation it admits.
fn with_room(mut checked: Checked, room: std::io::Result<bool>) -> Checked {
    match room {
        Ok(room) => checked.room = room,
        Err(error) => checked.restored = Err(error),
    }
    checked
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
    /// background preference applies.
    Opened,
    /// An explicit Prepare.
    Manual,
}

impl Admission {
    fn automatic(self) -> bool {
        self != Self::Manual
    }
}

const RECORDING_CHANGED: &str = "The recording changed. Reopen it before requesting another view.";

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

    /// Asks for the timeline `body` reads of `source` to be prepared while no preparation took it
    /// up ([`crate::prepared::driver::native::ask`]): a refresh does, or the preparation running
    /// does before it ends. A refresh the history lane has no room for waits for its next job.
    fn ask_timeline(&self, source: &Arc<Source>, body: &ProductRequest) {
        let Some(branch) = crate::prepared::query::asked_branch(source, body) else {
            return;
        };
        let content = self.content_for(source, crate::source::Work::Recorded);
        match crate::prepared::driver::native::ask(source, content.as_ref(), branch) {
            Ok(true) => {
                if let Err(error) = self.queue_refresh(Arc::clone(source), false, true) {
                    eprintln!("Timeline preparation: {error}");
                }
            }
            Ok(false) => {}
            Err(error) => eprintln!("Timeline preparation: {error}"),
        }
    }

    /// Refreshes share history capacity and one queued item per source. A later
    /// restart request upgrades that item rather than retaining another source.
    pub(super) fn refresh_preparation(
        &self,
        source: Arc<Source>,
        restart: bool,
    ) -> std::io::Result<()> {
        self.queue_refresh(source, restart, false)
    }

    /// [`Self::refresh_preparation`]; with `defer`, a full history queue keeps the refresh in
    /// `deferred` under the same lock, so the worker that next takes a job queues it.
    fn queue_refresh(
        &self,
        source: Arc<Source>,
        restart: bool,
        defer: bool,
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
            source: Arc::clone(&source),
            restart: Arc::clone(&restart),
        };
        match lanes.history.try_send(false, job) {
            Ok(()) => {
                lanes.refreshes.insert(key, restart);
                Ok(())
            }
            Err(TrySendError::Full(_)) if defer => {
                lanes.deferred.insert(key, source);
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
        Arc::clone(&source).in_scope(|| self.start_scoped(source, admission, checked, cancelled))
    }

    /// [`Self::start_preparation`] with the recording's executable tables in scope.
    fn start_scoped(
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
        if automatic && (!enabled || progress.cleared || progress.running || suppressed) {
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
        let content = self.content_for(&source, crate::source::Work::Recorded);
        let started = crate::prepared::start(
            &self.preparation,
            source,
            content,
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
    /// `free` is the volume's free bytes, which admit library work while they
    /// cover what this store still needs.
    pub(super) fn check_store(&self, source: &Source, free: Option<u64>) -> Checked {
        source.in_scope(|| {
            let epoch = crate::prepared::store_epoch(source);
            let content = self.content_for(source, crate::source::Work::Recorded);
            let identity = store_identity(source);
            let room = library_room(source, free);
            let checked = Checked {
                epoch,
                restored: crate::prepared::pipeline::restore(
                    source,
                    content.as_ref(),
                    &self.closed,
                ),
                room: true,
                identity,
            };
            with_room(checked, room)
        })
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

    /// Measure the volume's free bytes. Never under the admission lock: a slow
    /// or network volume must not stall opens or preparation actions.
    fn measure_free(&self) -> Option<u64> {
        #[cfg(test)]
        {
            self.test_measures.fetch_add(1, Ordering::Relaxed);
            if self.preparation_actions.try_lock().is_err() {
                self.test_measures_locked.fetch_add(1, Ordering::Relaxed);
            }
        }
        crate::prepared::space::free()
    }

    /// The scan's measurement, once per scan, before it admits anything.
    pub(super) fn space_for_scan(&self) -> Option<u64> {
        let free = self.measure_free();
        let now = std::time::Instant::now();
        let mut last = self.room.lock().unwrap_or_else(|e| e.into_inner());
        last.interval = last.measured.map(|measured| now.duration_since(measured));
        last.measured = Some(now);
        last.free = free;
        free
    }

    /// A refresh's measurement: the scan's last one, measured again outside
    /// the admission lock once it is older than the scans' own interval.
    pub(super) fn space_for_refresh(&self) -> Option<u64> {
        let fresh = {
            let last = self.room.lock().unwrap_or_else(|e| e.into_inner());
            match (last.measured, last.interval) {
                (Some(measured), Some(interval)) if measured.elapsed() <= interval => {
                    Some(last.free)
                }
                _ => None,
            }
        };
        fresh.unwrap_or_else(|| self.measure_free())
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
    /// it) as Ready with its sections, clearing any pause or error, under the
    /// admission lock. Whether it was published: a job that started meanwhile reports its own outcome.
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

    /// Whether opening `source` admits its preparation now.
    fn opening_admits(&self, source: &Source) -> bool {
        self.background_preparation.load(Ordering::Acquire)
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

    /// Admit one library recording for the scan, which measured the volume's
    /// `free` bytes before taking any lock.
    pub(super) fn prepare_recording(&self, path: &Path, free: Option<u64>) -> std::io::Result<()> {
        if !self.background_preparation.load(Ordering::Acquire) {
            return Ok(());
        }
        self.check_open(&self.closed)
            .map_err(std::io::Error::other)?;
        let path = path.canonicalize()?;
        self.upgrade(&path);
        let revision = self.recording_revision(&path)?;
        let result = self.prepare_recording_inner(path.clone(), revision.clone(), free);
        if let Err(error) = &result
            && !matches!(
                error.kind(),
                std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
            )
            && !self.closed.load(Ordering::Acquire)
        {
            // An unchanged failed input cannot make progress on a later worker
            // completion. Only a new revision, binding or manual Start retries it.
            let mut sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
            sources.library.remove(&path);
            sources.completed.insert(path, revision);
        }
        result
    }

    /// [`Self::prepare_recording`] of the recording at `path` of `revision`.
    fn prepare_recording_inner(
        &self,
        path: PathBuf,
        revision: String,
        free: Option<u64>,
    ) -> std::io::Result<()> {
        use crate::prepared::Progress;
        let previous = {
            let sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
            if sources.suppressed.contains(&path)
                || sources
                    .completed
                    .get(&path)
                    .is_some_and(|held| *held == revision)
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
        // Opening may read an initial catalogue; only closing the service
        // interrupts it.
        let source = self
            .open(&path, Some(&revision), &self.closed)
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
        // Checked and admitted with the recording's own executable tables in scope.
        Arc::clone(&source).in_scope(|| self.admit_opened(path, revision, free, source))
    }

    /// [`Self::prepare_recording_inner`] of the opened `source`, its tables in scope.
    fn admit_opened(
        &self,
        path: PathBuf,
        revision: String,
        free: Option<u64>,
        source: Arc<Source>,
    ) -> std::io::Result<()> {
        use crate::prepared::{Progress, State};
        let finished = |state: &Progress| {
            matches!(state.state, State::Ready | State::Failed)
        };
        let progress = source.state::<Progress>();
        // Check the store before taking the admission lock, so restoring many
        // recordings on a first scan never delays opens or preparation
        // actions. Only a restore that succeeds shows the recording
        // prepared; a store that cannot be checked or restored is not known to
        // be prepared, and its preparation succeeds or reports the real
        // failure through the ordinary terminal failure.
        let idle = {
            let state = progress.lock().unwrap_or_else(|e| e.into_inner());
            !state.running && !finished(&state)
        };
        let checked = if idle {
            self.check_store(&source, free)
        } else {
            let unread = Checked {
                epoch: crate::prepared::store_epoch(&source),
                ..Checked::unread(true)
            };
            with_room(unread, library_room(&source, free))
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
        if !self.background_preparation.load(Ordering::Acquire) {
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
        let finished = finished(&state);
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
            &self.closed,
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
        self.content.get()
    }

    /// Gives a recording made before recordings carried their game facts the facts it depends
    /// on, proven from the game folders, as a revision of its own ([`recording_upgrade`]). A
    /// recording no folder holds any of stays as it is, and opens with its facts withheld.
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
            cache: content.options.cache_dir.clone(),
        };
        match recording_upgrade::upgrade(path, &folders) {
            Ok(recording_upgrade::Outcome::Upgraded { steps }) => {
                eprintln!("Upgraded {}: {}", path.display(), steps.join("; "))
            }
            Ok(recording_upgrade::Outcome::Current) => {}
            Err(error) => eprintln!("{} is not upgraded: {error}", path.display()),
        }
    }

    /// The content options of work on `source` admitted now (`Source::admit`): the source's
    /// binding as it is, which everything the work reads and every reply it gives keeps to, on
    /// any thread. This is the work's entry: the mount it draws is resolved here, before any
    /// product code runs, waiting on another request's mount only while the work is not
    /// cancelled (`Source::admit_resolved`).
    pub(super) fn content_for(
        &self,
        source: &Source,
        work: crate::source::Work,
    ) -> Option<ContentOptions> {
        source.admit_resolved(work)
    }

    /// Every source a configuration writer must rebind: those open and those still opening
    /// (registered before their first binding, not yet handed to anyone).
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

    /// Binds `source` to the configuration as it is ([`crate::presentation::rebind`]): the
    /// configured content and its campaign's visual mods, as the Campaign map shows them, with
    /// what they mount now ([`crate::presentation::Presentation::current`]), or no binding
    /// without configured content. The identity is derived outside every lock and installed
    /// (`Source::bind`, one swap of the whole binding) only while the configuration (folders,
    /// unverified choice, cache folder, saved map settings) is still the one it was derived
    /// from; otherwise it is derived again, so the newest configuration always wins. A change
    /// advances the presentation generation, which clients refetch on.
    pub(super) fn rebind(&self, source: &Source, cancelled: &AtomicBool) -> Result<(), String> {
        use crate::presentation::Presentation;
        let result = crate::presentation::rebind(
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
                    // Shown as unavailable when mounted; a later check that reads the mods is
                    // another identity.
                    Err(error) => Presentation {
                        visual_mods: selected,
                        revision: format!("unreadable: {error}"),
                    },
                };
                #[cfg(test)]
                {
                    // Taken before it runs: a hook that rebinds reaches this lock again.
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
                }
            },
        );
        match result {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                Err("Cancelled".into())
            }
            result => result.map_err(|error| error.to_string()),
        }
    }

    /// Reads the game's own settings from the user folder configured now into the map settings,
    /// installed only while that folder is still the one configured
    /// ([`crate::presentation::rebind`]): work admitted under other folders never restores theirs.
    /// A changed `settings.txt`, whose game language prepared text is made in, moves every
    /// source's prepared key and prepares it again, as a change of folders does.
    pub(super) fn configure_game(&self) {
        let _ = crate::presentation::rebind(
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
        // Folders are replaced under the admission lock, so these are the ones committed now.
        let admission = self
            .preparation_actions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(content) = self.content() else {
            return;
        };
        let user_dir = &content.options.user_dir;
        // Recordings no source holds open, completed ones included, are bound to the stamp
        // seen last; new folders are rebound by `replace_content`.
        let stamp = crate::prepared::settings_stamp(user_dir);
        let mut moved = {
            let mut bound = self
                .game_settings
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let moved = bound
                .as_ref()
                .is_some_and(|(folder, seen)| folder == user_dir && *seen != stamp);
            *bound = Some((user_dir.clone(), stamp));
            moved
        };
        for source in self.bound_sources() {
            moved |= crate::prepared::rebind_settings(&source, user_dir);
        }
        if moved {
            self.reprepare(admission);
        }
    }

    /// After every source's prepared binding moved on: the store checks taken so far are void,
    /// and each open recording's running or finished preparation is refreshed off-thread against
    /// the new binding. Takes the admission lock the caller rebound under.
    pub(super) fn reprepare(&self, admission: std::sync::MutexGuard<'_, ()>) {
        self.invalidate_checks();
        drop(admission);
        self.preparation.notify_change();
        let sources: Vec<_> = self
            .sources
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entries
            .values()
            .filter_map(Weak::upgrade)
            .collect();
        let mut latest = BTreeMap::<PathBuf, Arc<Source>>::new();
        let mut revisions = BTreeMap::new();
        for source in sources {
            let path = source.path().to_owned();
            let revision = revisions
                .entry(path.clone())
                .or_insert_with(|| self.recording_revision(&path).ok());
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
                crate::prepared::State::Idle
                    | crate::prepared::State::Cancelled
                    | crate::prepared::State::Paused
            ) {
                self.refresh_or_defer(source);
            }
        }
    }

    /// Queues a restart of `source`'s preparation, or keeps it until the history queue has
    /// room, so a binding that moved on is always prepared again as its job would have been.
    pub(super) fn refresh_or_defer(&self, source: Arc<Source>) {
        if let Err(error) = self.queue_refresh(source, true, true) {
            eprintln!("Preparation refresh: {error}");
        }
    }

    /// Queues the restarts kept while the history queue was full, now that a job has left it.
    fn queue_deferred(&self) {
        let deferred = match self
            .loader
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            Some(lanes) => std::mem::take(&mut lanes.deferred),
            None => return,
        };
        for source in deferred.into_values() {
            self.refresh_or_defer(source);
        }
    }

    /// Catches every source up to the configuration as committed now ([`Self::present`]). Once
    /// a change is committed the catch-up belongs to the service: only its closing stops it,
    /// never the request that made the change, so no source stays on a configuration that is
    /// no longer the one saved. A newer change landing meanwhile is caught up by the same loop.
    pub(super) fn catch_up(&self) -> Result<(), String> {
        self.present(&self.bound_sources(), &self.closed)
    }

    /// [`Self::rebind`] for each of `sources`.
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
        // A publication between the revision read and the open leaves other bytes under it.
        if self.recording_revision(source.path()).ok().as_ref() != Some(&source.key().revision) {
            return Err(RECORDING_CHANGED.into());
        }
        if let Some(previous) = previous {
            source.inherit_append_states(&previous);
        }
        // Registered before its first binding, so a configuration change from here on rebinds
        // it too; handed to no one until it is bound to the configuration as it is.
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
            LoadJob::Product { body, .. } => {
                if products::find(&body.product).is_some_and(|product| product.interactive) {
                    Lane::Interactive
                } else {
                    Lane::History
                }
            }
            LoadJob::PreparedQuery { .. } => Lane::Prepared,
            LoadJob::Preparation { .. }
            | LoadJob::PreparationPreferences { .. }
            | LoadJob::HistoryChoice { .. } => Lane::Interactive,
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
        if let Some(timing) = &request.timing {
            timing.queued(lane);
        }
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
        // Any installed mod can be chosen, and a chosen mod that is no longer installed keeps
        // its place by name; what cannot be shown is named when it is mounted.
        if let Some(settings) = &settings {
            settings.validate().map_err(str::to_owned)?;
            // Until the renderer has said, it is the one to refuse a count its adapter lacks.
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
        let recorded = replay
            .map(crate::presentation::recorded_mods)
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
            presentation_generation: 0,
        };
        drop(store);
        // Widgets follow every source's campaign selection as saved now, and the mods as they
        // are now: a save, a Refresh or a mod installed, updated or removed since. Saved
        // settings are part of the configuration bindings are derived from.
        self.content.update(|_| ());
        self.catch_up()?;
        reply.presentation_generation = self.presentation_generation();
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
                // Before it can block, a loader returns what its last jobs freed: here when
                // another loader holds the receiver, else as the receiver finds no job.
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
            if lane == Lane::History {
                self.queue_deferred();
            }
            let request = job.request().cloned();
            if let Some(timing) = request.as_ref().and_then(|request| request.timing.as_ref()) {
                timing.started();
            }
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
            // Work on a recording reads its own executable tables.
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
                    // A user's Cancel or Clear holds until their own Start, even while the job
                    // it stopped is still ending.
                    if self
                        .sources
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .suppressed
                        .contains(source.path())
                    {
                        return;
                    }
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
                let checked = self.check_store(&source, self.space_for_refresh());
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
            LoadJob::PreparationPreferences { request, enabled } => {
                if request.cancelled() {
                    return;
                }
                let result = request_result(|| {
                    if let Some(enabled) = enabled {
                        crate::Config::save_background_preparation(enabled, |enabled| {
                            self.set_background_preparation(enabled)
                        })
                        .map_err(|error| error.to_string())?;
                    }
                    bounded_json(&self.preparation_preferences(), &request.cancelled)
                });
                self.complete(request, result.map_err(Into::into));
            }
            LoadJob::MapSettings(job) => {
                let request = Arc::clone(&job.request);
                let result = self.map_settings_job(job);
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
                let Some(product) = products::find(&body.product) else {
                    self.error(
                        request,
                        format!("Unknown data product: {}", body.product).into(),
                    );
                    return;
                };
                let context = LoadContext {
                    content: self.content_for(&source, work(product.content_dependent, &request)),
                };
                let result = (product.load)(
                    &source,
                    &lease_id,
                    body.params,
                    &context,
                    &request.cancelled,
                );
                self.complete(request, result.map_err(Into::into));
            }
            LoadJob::PreparedQuery {
                request,
                source,
                body,
            } => {
                if request.cancelled() {
                    return;
                }
                if lane != Lane::Sampled {
                    self.ask_timeline(&source, &body);
                }
                let answer = Self::attempt(lane, || {
                    crate::prepared::query::query(
                        &source,
                        &body,
                        self.content_for(
                            &source,
                            work(
                                products::find(&body.product)
                                    .is_none_or(|product| product.content_dependent),
                                &request,
                            ),
                        )
                        .as_ref(),
                        &request.cancelled,
                    )
                    .map_err(crate::QueryError::from)
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
                    let content = self.content_for(&source, crate::source::Work::Recorded);
                    match action {
                        crate::prepared::Action::Status => {}
                        crate::prepared::Action::Start => {
                            self.preparation.notify_change();
                            let checked = self.check_store(&source, None);
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
                self.complete(request, result.map_err(Into::into));
            }
            LoadJob::Open(job) => {
                self.configure_game();
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
        let opened = self.open(
            Path::new(&job.path),
            Some(&job.revision),
            &job.request.cancelled,
        );
        // Everything the open goes on to read is the recording's, with its own tables.
        let tables = opened.as_ref().ok().and_then(|source| source.tables());
        engine_executable::executable_tables::scope(tables, || self.opened(job, opened));
    }

    /// The rest of [`Self::open_job`] once the source is open, in its tables' scope.
    fn opened(&self, job: OpenJob, opened: Result<Arc<Source>, String>) {
        let result = opened.and_then(|source| {
            read_key_names_ahead(&source, self.content());
            source
                .name_countries(self.content().as_ref())
                .map_err(|error| error.to_string())?;
            Ok(source)
        });
        if let Ok(source) = &result {
            // Read the store before the admission lock.
            let checked = self.check_store(source, None);
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
            }
            drop(admission);
            self.confirm_checked(source, confirmation);
        }
        // What the source's first game window reads is read ahead at low priority, once the
        // open has answered.
        let ahead = result.as_ref().ok().map(|source| {
            // The read-ahead is cancelled once its binding is retired: a swap, an unbinding,
            // the source dropped or the service closing.
            // Its mount is resolved on the read-ahead's own thread, not before this open answers.
            let content = source.admit(crate::source::Work::Ahead);
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
            products::read_ahead::warm_in_background(&source, content.clone());
            products::game_window::warm_in_background(&source, content);
        }
    }
}

/// Reads the key names of `content`'s stack on a thread of its own while the open names the
/// countries: the store check after the naming waits for them (through the icon content), as do
/// most first replies. The content is resolved here, as the naming next resolves it, so the
/// thread binds nothing.
fn read_key_names_ahead(source: &Source, content: Option<ContentOptions>) {
    let Some(resolved) = content.and_then(|content| source.content(&content.options).ok()) else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name("key names ahead".into())
        .spawn_carried(move || drop(resolved.layers.key_names()));
}

/// Keeps the viewer's choice of the recorded segment `segment` of the recording at `path` goes
/// on from, then marks the recording changed, so every reader opens it again under the choice.
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
    HistoryChoice {
        request: Arc<Request>,
        path: String,
        segment_id: String,
        choice: Option<String>,
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
    Open(OpenJob),
    Product {
        request: Arc<Request>,
        lease_id: String,
        source: Arc<Source>,
        body: ProductRequest,
    },
}

impl LoadJob {
    /// The recording this job works on, whose executable tables it reads.
    fn source(&self) -> Option<&Arc<Source>> {
        match self {
            Self::PreparationRefresh { source, .. }
            | Self::PreparedQuery { source, .. }
            | Self::Preparation { source, .. }
            | Self::Product { source, .. } => Some(source),
            _ => None,
        }
    }

    fn is_refresh(&self) -> bool {
        self.request()
            .is_some_and(|request| request.refresh.load(Ordering::Acquire))
    }
    fn request(&self) -> Option<&Arc<Request>> {
        Some(match self {
            Self::PreparationRefresh { .. } => return None,
            Self::MapSettings(job) => &job.request,
            Self::Open(job) => &job.request,
            Self::Product { request, .. } => request,
            Self::PreparedQuery { request, .. } | Self::Preparation { request, .. } => request,
            Self::PreparationPreferences { request, .. } | Self::HistoryChoice { request, .. } => {
                request
            }
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
