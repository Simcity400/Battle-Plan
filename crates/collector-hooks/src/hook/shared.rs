//! Several hook families publish under one thread census and suspension.
//! Detach attempts every participating unrestored family independently.
//! Attach stops at its first busy or refused family: only the ordered ready
//! prefix writes, and verification and protection restoration precede the
//! next write.
//!
//! A family takes part through [`Hook::set_family_enabled_shared`], and in
//! attach through every publication entry point. The runtime runs one [`Round`]:
//! - a collect pass calls each taking-part family's own `set_enabled`, whose
//!   publications are recorded instead of performed. Detach reports "not
//!   ready", so the family stops before anything that depends on the outcome.
//!   Attach reports "ready" and the family records all its publications,
//!   running its atomic steps between them early; nothing samples an hour
//!   before the report pass, which repeats those steps with the real outcomes,
//!   and a family that does not publish is refused, which turns them off;
//! - [`publish`] checks and writes the eligible recorded families under one
//!   suspension, with exactly the checks a family's own publication makes;
//! - a report pass calls the families again, in their usual order and with
//!   their usual bookkeeping, and each publication returns its own recorded
//!   outcome without suspending anything.
//!
//! During detach, a family whose sites a thread holds, or whose bytes, verification or
//! protection refuse, gets the outcome its own publication would have given,
//! and the others still publish: every family is checked and, unless held or
//! refused before writing, written and verified, whatever another family's
//! outcome. Page protections are restored once, after every write and flush,
//! and nothing makes a page writable again before the threads resume. A
//! failed restore leaves its page as it was and every other restore still
//! runs, so a page shared by several families ends with its saved protection
//! whenever any of their restores of it succeeds; a family reports success
//! only when all of its own restores succeeded.
//!
//! Attach flushes and verifies each family before the next writes, and
//! restores its pages using each hook's immutable saved protection. A later
//! family reopens its sites if an earlier restore covered their shared page.
//! Atomic preparation runs when the publisher reaches its family. Report
//! runs observer admission and forwarding in chain order after resume,
//! before the runtime admits the first hour; those flags select no later patch.
//! Preparation involving allocation or native registration stays off hold.
//!
//! Two failures are shared rather than per family: a failed instruction-cache flush
//! restores the prior bytes of every family written in the round and each
//! reports that failure, and a failed thread resume is reported
//! by every family of the round, ahead of its own preparation error (its
//! own publication would not have suspended at all). Both leave every
//! family's bytes and retained executable storage as conservative as its
//! own failure would. Attach rollback reopens earlier restored pages; a
//! failed reopen leaves that family's bytes alone and retains its executable
//! storage for detach. A recording allocation failure abandons the collected
//! batch, so report can fall back to serial publication without writing a
//! suffix past the missing family. A publication the report pass reaches
//! unrecorded, such as a rollback after a refusal, publishes on its own.
//!
//! The recorded hook slices are raw pointers into the families' own hook
//! storage. Between the collect pass and the report pass nothing touches
//! those vectors except [`publish`], so the pointers stay valid and are the
//! only access to them while it runs.
use super::*;
use std::cell::{Cell, RefCell};

/// One family's publication: its hooks, the requested state and, once
/// published, its outcome and the QPC ticks of its checks and writes.
struct Recorded {
    hooks: RecordedHooks,
    family: Option<&'static str>,
    enabled: bool,
    outcome: Option<Result<bool, String>>,
    failure: Option<installation::Failure>,
    ticks: i64,
    prepare: Option<fn()>,
    allocation_failed: bool,
    track_busy: bool,
}

enum RecordedHooks {
    Slice(*mut Hook, usize),
    Pointers(Vec<*mut Hook>),
}

impl RecordedHooks {
    fn key(&self) -> (*const Hook, usize) {
        match self {
            Self::Slice(hooks, len) => (hooks.cast_const(), *len),
            Self::Pointers(hooks) => (
                hooks
                    .first()
                    .copied()
                    .unwrap_or(ptr::null_mut())
                    .cast_const(),
                hooks.len(),
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Collect,
    Report,
}

/// A family's enabling preparation, kept for the family that recorded it.
type Preparation = (Option<&'static str>, fn());

thread_local! {
    static MODE: Cell<Option<Mode>> = const { Cell::new(None) };
    static RECORDED: RefCell<Vec<Recorded>> = const { RefCell::new(Vec::new()) };
    /// The checks-and-writes ticks of the family the last report returned.
    static REPORTED_TICKS: Cell<Option<i64>> = const { Cell::new(None) };
    static PREFIX: Cell<bool> = const { Cell::new(false) };
    static COLLECT_STOPPED: Cell<bool> = const { Cell::new(false) };
    static PREPARATION: Cell<Option<Preparation>> = const { Cell::new(None) };
    static COLLECT_ALLOCATION_FAILED: Cell<bool> = const { Cell::new(false) };
}

/// One round's shared publication: the families recorded and its wall
/// ticks (census, protections, suspension, every family's checks and
/// writes, flush, protections and resume).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shared {
    pub families: usize,
    pub ticks: i64,
}

/// A round on this thread: [`Round::collect`] starts the collect pass,
/// [`Round::publish`] publishes what it recorded and starts the report pass,
/// and dropping it ends the round. Not `Send`: the round is the calling
/// thread's.
pub struct Round {
    _thread: std::marker::PhantomData<*const ()>,
}

impl Round {
    /// Start a collect pass; `None` inside another round, whose publications
    /// are then each their own.
    pub fn collect() -> Option<Self> {
        if MODE.get().is_some() {
            return None;
        }
        RECORDED.with_borrow_mut(Vec::clear);
        COLLECT_STOPPED.set(false);
        PREFIX.set(false);
        PREPARATION.set(None);
        COLLECT_ALLOCATION_FAILED.set(false);
        MODE.set(Some(Mode::Collect));
        Some(Self {
            _thread: std::marker::PhantomData,
        })
    }

    /// Attach checks, verifies and restores protections in chain order,
    /// stopping before the first family that cannot publish.
    pub fn collect_attach() -> Option<Self> {
        let round = Self::collect()?;
        PREFIX.set(true);
        Some(round)
    }

    /// Publish every family the collect pass recorded under one suspension,
    /// then answer their calls until the round is dropped.
    pub fn publish(&mut self) -> Shared {
        MODE.set(None);
        let previous = publication::current_family();
        if PREFIX.get() {
            publication::select_family(Some("attach_shared"));
        }
        let shared = publish();
        publication::select_family(previous);
        MODE.set(Some(Mode::Report));
        shared
    }

    /// Visit every retained attach rollback failure after native resume and
    /// before ordered reporting can return an earlier family's error.
    pub fn for_each_attach_rollback_failure(
        &self,
        mut visit: impl FnMut(Option<&'static str>, &installation::Failure),
    ) {
        if !attach_reporting() {
            return;
        }
        RECORDED.with_borrow(|recorded| {
            for entry in recorded {
                if let Some(failure) = &entry.failure
                    && failure.kind == installation::Kind::Rollback
                {
                    visit(entry.family, failure);
                }
            }
        });
    }
}

impl Drop for Round {
    fn drop(&mut self) {
        MODE.set(None);
        PREFIX.set(false);
        PREPARATION.set(None);
        COLLECT_ALLOCATION_FAILED.set(false);
        RECORDED.with_borrow_mut(Vec::clear);
    }
}

/// Enabling preparation already ran in the collect pass. Report only its
/// outcome and post-publication bookkeeping.
pub fn attach_reporting() -> bool {
    PREFIX.get() && MODE.get() == Some(Mode::Report)
}

/// Defer allocation-free, atomic preparation until the publisher reaches
/// its family. A report with no stored outcome is serial fallback and must
/// prepare normally. The temporary reference vector may still be built off
/// hold after this call.
pub fn defer_preparation(prepare: fn()) -> bool {
    if !PREFIX.get() {
        return false;
    }
    match MODE.get() {
        Some(Mode::Collect) => {
            PREPARATION.set(Some((publication::current_family(), prepare)));
            true
        }
        Some(Mode::Report) => RECORDED.with_borrow(|recorded| {
            recorded.iter().any(|entry| {
                entry.family == publication::current_family() && entry.outcome.is_some()
            })
        }),
        None => false,
    }
}

/// A reference-vector refusal occurs before its publication is recorded.
/// Only when all predecessors reported success does runtime reach that
/// family's saved refusal, and run its pending preparation once.
pub fn prepare_refused_family(name: &'static str) {
    if let Some((Some(family), prepare)) = PREPARATION.get()
        && family == name
    {
        PREPARATION.set(None);
        prepare();
    }
}

/// Candidate reference storage must not latch a terminal allocation failure
/// for a family an earlier busy publication prevents the chain reaching.
pub fn publication_storage_attach<T>(capacity: usize, enabled: bool) -> io::Result<Vec<T>> {
    if !PREFIX.get() || MODE.get() != Some(Mode::Collect) {
        return publication_storage(capacity, enabled);
    }
    let mut storage = Vec::new();
    storage.try_reserve_exact(capacity).map_err(|_| {
        COLLECT_ALLOCATION_FAILED.set(true);
        io::Error::from(io::ErrorKind::OutOfMemory)
    })?;
    Ok(storage)
}

pub fn take_collect_allocation_failure() -> bool {
    COLLECT_ALLOCATION_FAILED.take()
}

/// Reference-building families consume their report before rebuilding the
/// temporary reference vector, so report cannot introduce a new allocation
/// refusal after a later family's bytes were written.
pub fn attach_outcome() -> Option<Result<bool, String>> {
    if !attach_reporting() {
        return None;
    }
    answer(None, true)
}

/// The recorded outcome of the publication of `key`'s hooks; `None` for a family's single
/// recorded outcome, which an attach family takes before it rebuilds its references.
fn answer(key: Option<(*const Hook, usize)>, enabled: bool) -> Option<Result<bool, String>> {
    let found = RECORDED.with_borrow_mut(|recorded| {
        recorded
            .iter_mut()
            .find(|entry| {
                // A family records several publications; each answers only its own hooks.
                (!PREFIX.get() || entry.family == publication::current_family())
                    && key.is_none_or(|key| entry.hooks.key() == key)
                    && entry.enabled == enabled
                    && entry.outcome.is_some()
            })
            .and_then(|entry| {
                Some((
                    entry.outcome.take()?,
                    entry.failure.take(),
                    entry.ticks,
                    entry.allocation_failed,
                ))
            })
    });
    if let Some((outcome, failure, ticks, allocation_failed)) = found {
        REPORTED_TICKS.set(Some(ticks));
        if let Some(failure) = failure {
            installation::refused(failure.kind, failure.site, &failure.cause);
        }
        if allocation_failed {
            let _ = engine_allocation::oom();
        }
        Some(outcome)
    } else {
        REPORTED_TICKS.set(None);
        None
    }
}

fn collect(
    enabled: bool,
    prepare: Option<fn()>,
    track_busy: bool,
    hooks: impl FnOnce() -> Result<RecordedHooks, ()>,
) -> Result<bool, String> {
    if COLLECT_STOPPED.get() {
        return Ok(false);
    }
    let recorded = RECORDED.with_borrow_mut(|recorded| {
        // Attach abandons the whole batch, so no later family can publish
        // ahead of this one; detach leaves this family to publish on its own
        // in the report pass and keeps the others recorded.
        let abandon = |recorded: &mut Vec<Recorded>| {
            if PREFIX.get() {
                recorded.clear();
                COLLECT_STOPPED.set(true);
            }
        };
        if recorded.try_reserve(1).is_err() {
            abandon(recorded);
            return false;
        }
        let Ok(hooks) = hooks() else {
            abandon(recorded);
            return false;
        };
        let pending = PREPARATION
            .take()
            .filter(|(name, _)| *name == publication::current_family())
            .map(|(_, prepare)| prepare);
        recorded.push(Recorded {
            hooks,
            family: publication::current_family(),
            enabled,
            outcome: None,
            failure: None,
            ticks: 0,
            prepare: prepare.or(pending),
            allocation_failed: false,
            track_busy,
        });
        true
    });
    // Attach goes on to record the family's later publications, so all of them share the one
    // suspension; the report pass repeats every step after it with the real outcomes.
    Ok(recorded && PREFIX.get())
}

/// [`Round`] as one call: `collect` (each taking-part family's
/// `set_enabled`), the shared publication, `shared` with its summary, then
/// `report` (the families' usual calls, which receive the recorded
/// outcomes). Inside another round, `report` runs alone.
#[cfg(any(test, feature = "test-support"))]
pub fn round<R>(
    collect: impl FnOnce(),
    shared: impl FnOnce(Shared),
    report: impl FnOnce() -> R,
) -> R {
    let Some(mut round) = Round::collect() else {
        return report();
    };
    collect();
    shared(round.publish());
    report()
}

/// The checks-and-writes ticks (QPC) of the family the last report
/// answered, once: `None` when its publication was its own, it published
/// nothing, or it does not take part.
pub fn take_reported_ticks() -> Option<i64> {
    REPORTED_TICKS.take()
}

impl Hook {
    /// Preserve publication's cause and symbolic patch owners after threads resume.
    /// Site-specific checks retain only a flag while suspended. Failures shared by
    /// the whole publication (for example thread resume) name all participating sites.
    pub fn set_family_enabled_shared_named(
        hooks: &mut [Self],
        enabled: bool,
        family: &str,
        names: &[&str],
    ) -> Result<bool, String> {
        debug_assert!(hooks.len() <= names.len());
        Self::set_family_enabled_shared(hooks, enabled).map_err(|cause| {
            let failed = hooks.iter().any(|hook| hook.diagnostic_failure);
            let sites = hooks
                .iter()
                .zip(names)
                .filter(|(hook, _)| !failed || hook.diagnostic_failure)
                .map(|(_, name)| *name)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{family} hook sites [{sites}] state change failed: {cause}")
        })
    }

    /// [`Hook::set_family_enabled`] that takes part in a shared [`Round`]:
    /// recorded in its collect pass, answered from the recording
    /// in its report pass, and published on its own otherwise.
    pub fn set_family_enabled_shared(hooks: &mut [Self], enabled: bool) -> Result<bool, String> {
        Self::set_family_enabled_shared_with_prepare(hooks, enabled, None)
    }

    /// Atomic preparation runs only when attach reaches this family, before
    /// its checks and write. It must not allocate, lock or call native code.
    pub fn set_family_enabled_shared_prepared(
        hooks: &mut [Self],
        enabled: bool,
        prepare: fn(),
    ) -> Result<bool, String> {
        Self::set_family_enabled_shared_with_prepare(hooks, enabled, Some(prepare))
    }

    fn set_family_enabled_shared_with_prepare(
        hooks: &mut [Self],
        enabled: bool,
        prepare: Option<fn()>,
    ) -> Result<bool, String> {
        if !(PREFIX.get() && MODE.get().is_some())
            && let Some(prepare) = prepare
        {
            prepare();
        }
        match MODE.get() {
            Some(Mode::Collect) => {
                if !PREFIX.get()
                    && (hooks.is_empty() || !enabled && hooks.iter().all(|hook| !hook.is_enabled()))
                {
                    // Nothing to write: the family proceeds as its own
                    // publication would let it.
                    return Ok(true);
                }
                collect(enabled, prepare, false, || {
                    Ok(RecordedHooks::Slice(hooks.as_mut_ptr(), hooks.len()))
                })
            }
            Some(Mode::Report) => {
                if let Some(outcome) = answer(Some((hooks.as_ptr(), hooks.len())), enabled) {
                    return outcome;
                }
                REPORTED_TICKS.set(None);
                if PREFIX.get()
                    && let Some(prepare) = prepare
                {
                    prepare();
                }
                Self::set_family_enabled(hooks, enabled)
            }
            None => {
                REPORTED_TICKS.set(None);
                Self::set_family_enabled(hooks, enabled)
            }
        }
    }

}

/// [`Hook::set_enabled_batch`]: every publication an attach round reaches joins its one
/// suspension, whichever entry point a family publishes through. Detach publishes on its own.
pub(super) fn batch<const TRACK_BUSY: bool>(
    hooks: &mut [&mut Hook],
    enabled: bool,
) -> Result<bool, String> {
    if !PREFIX.get() {
        return Hook::publish_batch::<TRACK_BUSY>(hooks, enabled);
    }
    let key = (
        hooks
            .first()
            .map_or(ptr::null(), |hook| &**hook as *const Hook),
        hooks.len(),
    );
    match MODE.get() {
        Some(Mode::Collect) => collect(enabled, None, TRACK_BUSY, || {
            let mut pointers = Vec::new();
            pointers.try_reserve_exact(hooks.len()).map_err(|_| ())?;
            pointers.extend(hooks.iter_mut().map(|hook| &mut **hook as *mut Hook));
            Ok(RecordedHooks::Pointers(pointers))
        }),
        Some(Mode::Report) => answer(Some(key), enabled)
            .unwrap_or_else(|| Hook::publish_batch::<TRACK_BUSY>(hooks, enabled)),
        None => Hook::publish_batch::<TRACK_BUSY>(hooks, enabled),
    }
}

/// Publish every recorded family under one census and suspension.
fn publish() -> Shared {
    let begin = crate::clock::qpc();
    let mut recorded = RECORDED.take();
    let families = recorded.len();
    if families != 0 {
        let mut groups: Vec<Group<'_>> = Vec::new();
        if groups.try_reserve_exact(families).is_ok() {
            for entry in &recorded {
                // SAFETY: a family's own hook storage, recorded in this round's
                // collect pass; nothing else reaches it until the report
                // pass, and no two families share a hook.
                let hooks = match &entry.hooks {
                    RecordedHooks::Slice(hooks, len) => {
                        Hooks::Slice(unsafe { std::slice::from_raw_parts_mut(*hooks, *len) })
                    }
                    RecordedHooks::Pointers(hooks) => Hooks::Pointers(unsafe {
                        std::slice::from_raw_parts(hooks.as_ptr(), hooks.len())
                    }),
                };
                let mut group = Group::from_hooks(hooks, entry.enabled);
                group.prepare_callback = entry.prepare;
                group.track_busy = entry.track_busy;
                groups.push(group);
            }
            publish_groups_with_order(&mut groups, real_flush, PREFIX.get());
            for (entry, mut group) in recorded.iter_mut().zip(groups) {
                entry.ticks = group.ticks;
                entry.allocation_failed = group.prepare_allocation_failed;
                let outcome = group.outcome.take().unwrap_or(Ok(false));
                entry.failure = Hook::installation_failure(group.hooks.iter(), &outcome);
                entry.outcome = Some(outcome);
            }
        }
        // Without the groups' storage nothing recorded is answered: each
        // family publishes on its own in the report pass.
    }
    RECORDED.set(recorded);
    Shared {
        families,
        ticks: crate::clock::qpc() - begin,
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Page-protection restores to let through, then to refuse, as a failed
    /// `VirtualProtect` would.
    static FAIL_RESTORES: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
    static FAIL_REOPENS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}
#[cfg(test)]
pub(super) fn fail_restores(skip: usize, count: usize) {
    FAIL_RESTORES.set((skip, count));
}
#[cfg(any(test, feature = "test-support"))]
pub fn fail_reopens(skip: usize, count: usize) {
    FAIL_REOPENS.set((skip, count));
}
/// Whether a test refuses this page-protection restore.
#[cfg(any(test, feature = "test-support"))]
fn restore_refused() -> bool {
    let (skip, count) = FAIL_RESTORES.get();
    if skip != 0 {
        FAIL_RESTORES.set((skip - 1, count));
        return false;
    }
    FAIL_RESTORES.set((0, count.saturating_sub(1)));
    count != 0
}
#[cfg(not(any(test, feature = "test-support")))]
fn restore_refused() -> bool {
    false
}

pub(super) fn real_flush() -> Result<(), u32> {
    #[cfg(any(test, feature = "test-support"))]
    if FAIL_FLUSHES.with(|remaining| {
        let value = remaining.get();
        remaining.set(value.saturating_sub(1));
        value != 0
    }) {
        return Err(31);
    }
    if unsafe { FlushInstructionCache(GetCurrentProcess(), ptr::null(), 0) } == 0 {
        Err(unsafe { GetLastError() })
    } else {
        Ok(())
    }
}

/// The hooks a publication touches: all of them to enable, those enabled
/// before it to disable.
fn selected<'h>(
    hooks: &'h Hooks<'_>,
    previous: &'h [(bool, bool)],
    enabled: bool,
) -> impl Iterator<Item = &'h Hook> + 'h {
    hooks
        .iter()
        .zip(previous)
        .filter(move |(_, (_, owned))| enabled || *owned)
        .map(|(hook, _)| hook)
}

/// The owning family's stable storage, borrowed only by its publisher.
/// Scattered pointers come from disjoint exclusive hook references in the
/// collect pass, never from its temporary reference vector's storage.
enum Hooks<'a> {
    Slice(&'a mut [Hook]),
    Pointers(&'a [*mut Hook]),
}
impl Hooks<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Slice(hooks) => hooks.len(),
            Self::Pointers(hooks) => hooks.len(),
        }
    }
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn iter(&self) -> impl Iterator<Item = &Hook> {
        let (slice, pointers) = match self {
            Self::Slice(hooks) => (&hooks[..], &[][..]),
            Self::Pointers(hooks) => (&[][..], &hooks[..]),
        };
        // SAFETY: the collect/report contract gives this publisher the only
        // access to these stable hooks until it releases the groups.
        slice
            .iter()
            .chain(pointers.iter().map(|hook| unsafe { &**hook }))
    }
    fn iter_mut(&mut self) -> impl Iterator<Item = &mut Hook> {
        let (slice, pointers) = match self {
            Self::Slice(hooks) => (&mut hooks[..], &[][..]),
            Self::Pointers(hooks) => (&mut [][..], &hooks[..]),
        };
        // SAFETY: as above; each pointer was an exclusive reference to a
        // distinct hook, and this iterator borrows the group exclusively.
        slice
            .iter_mut()
            .chain(pointers.iter().map(|hook| unsafe { &mut **hook }))
    }
}

/// One family in a shared publication.
pub(super) struct Group<'a> {
    hooks: Hooks<'a>,
    enabled: bool,
    /// Prior `(enabled, is_enabled)` of each hook.
    previous: Vec<(bool, bool)>,
    /// The sites of the selected hooks.
    ranges: Vec<Range<usize>>,
    protected: usize,
    prepare_error: Option<String>,
    /// While suspended: `Ok(true)` checked and written, `Ok(false)` held,
    /// `Err(code)` refused.
    checked: Option<Result<bool, u32>>,
    snapshot_refusal: bool,
    publication_failed: bool,
    protection_ok: bool,
    outcome: Option<Result<bool, String>>,
    ticks: i64,
    /// A numeric protection error can be retained without formatting while
    /// suspended when an ordered publication reopens a shared page.
    protection_error: Option<u32>,
    prepare_callback: Option<fn()>,
    prepare_allocation_failed: bool,
    track_busy: bool,
}

impl<'a> Group<'a> {
    #[cfg(test)]
    pub(super) fn new(hooks: &'a mut [Hook], enabled: bool) -> Self {
        Self::from_hooks(Hooks::Slice(hooks), enabled)
    }

    fn from_hooks(mut hooks: Hooks<'a>, enabled: bool) -> Self {
        for hook in hooks.iter_mut() {
            hook.diagnostic_failure = false;
            hook.guard_held = false;
        }
        Self {
            hooks,
            enabled,
            previous: Vec::new(),
            ranges: Vec::new(),
            protected: 0,
            prepare_error: None,
            checked: None,
            snapshot_refusal: false,
            publication_failed: false,
            protection_ok: true,
            outcome: None,
            ticks: 0,
            protection_error: None,
            prepare_callback: None,
            prepare_allocation_failed: false,
            track_busy: false,
        }
    }

    #[cfg(test)]
    pub(super) fn outcome(&self) -> Option<&Result<bool, String>> {
        self.outcome.as_ref()
    }

    fn prepare_observation(&mut self) {
        if let Some(prepare) = self.prepare_callback.take() {
            prepare();
        }
    }

    /// Before the suspension: prior states, ranges and page protections,
    /// in a family's own publication's order.
    fn prepare(&mut self, prefix: bool) {
        if self.track_busy {
            for hook in self.hooks.iter() {
                hook.publication_busy.store(false, Ordering::Relaxed);
            }
        }
        let len = self.hooks.len();
        let Ok(mut previous) = publication_storage(len, self.enabled && !prefix) else {
            self.prepare_allocation_failed = prefix;
            self.prepare_error = Some("hook state allocation failed".into());
            return;
        };
        previous.extend(
            self.hooks
                .iter()
                .map(|hook| (hook.enabled, hook.is_enabled())),
        );
        let Ok(mut ranges) = publication_storage(len, self.enabled && !prefix) else {
            self.prepare_allocation_failed = prefix;
            self.prepare_error = Some("hook ranges allocation failed".into());
            return;
        };
        ranges.extend(
            selected(&self.hooks, &previous, self.enabled)
                .map(|hook| hook.site..hook.site + hook.sentinel_len),
        );
        self.previous = previous;
        self.ranges = ranges;
        for (hook, &(_, owned)) in self.hooks.iter_mut().zip(&self.previous) {
            if !(self.enabled || owned) {
                continue;
            }
            let mut prior = 0;
            if unsafe {
                VirtualProtect(
                    hook.site as *const c_void,
                    hook.sentinel_len,
                    PAGE_EXECUTE_READWRITE,
                    &mut prior,
                )
            } == 0
            {
                hook.diagnostic_failure = true;
                self.prepare_error = Some(io::Error::last_os_error().to_string());
                return;
            }
            self.protected += 1;
        }
    }

    /// While suspended, exactly as a family's own publication checks: a held
    /// site or tap is `Ok(false)` and a changed snapshot `Err(0)`, before any
    /// write; then every selected hook that changes state is written.
    fn check_and_write(&mut self, threads: &mut Threads) -> Result<bool, u32> {
        if self.track_busy {
            self.check_and_write_tracking::<true>(threads)
        } else {
            self.check_and_write_tracking::<false>(threads)
        }
    }

    fn check_and_write_tracking<const TRACK_BUSY: bool>(
        &mut self,
        threads: &mut Threads,
    ) -> Result<bool, u32> {
        if TRACK_BUSY {
            if let Some(index) = threads.colliding_range(&self.ranges)? {
                selected(&self.hooks, &self.previous, self.enabled)
                    .nth(index)
                    .expect("range belongs to selected hook")
                    .publication_busy
                    .store(true, Ordering::Relaxed);
                return Ok(false);
            }
        } else if threads.contains_ip(&self.ranges)? {
            return Ok(false);
        }
        let enabled = self.enabled;
        for (hook, &(_, owned)) in self.hooks.iter_mut().zip(&self.previous) {
            if !(enabled || owned) {
                continue;
            }
            if hook
                .tap_busy_with(enabled, |ranges| threads.contains_ip(ranges))
                .inspect_err(|_| hook.diagnostic_failure = true)?
            {
                if TRACK_BUSY {
                    hook.publication_busy.store(true, Ordering::Relaxed);
                }
                return Ok(false);
            }
            if hook
                .guard_busy(enabled, threads)
                .inspect_err(|_| hook.diagnostic_failure = true)?
            {
                return Ok(false);
            }
            if !hook.bytes_match(hook.enabled) {
                hook.diagnostic_failure = true;
                self.snapshot_refusal = enabled && self.previous.iter().all(|(_, owned)| !*owned);
                return Err(0);
            }
        }
        for (hook, (_, owned)) in self.hooks.iter_mut().zip(&self.previous) {
            if !(enabled || *owned) || hook.enabled == enabled {
                continue;
            }
            let bytes = if enabled {
                &hook.patch[..hook.patch_len]
            } else {
                &hook.original[..hook.patch_len]
            };
            unsafe {
                ptr::copy_nonoverlapping(bytes.as_ptr(), hook.site as *mut u8, hook.patch_len);
            }
            hook.enabled = enabled;
            hook.needs_flush = true;
        }
        Ok(true)
    }

    /// After a failed flush: put the family's prior bytes back.
    fn roll_back(&mut self) {
        let enabled = self.enabled;
        for (hook, &(was_enabled, owned)) in self.hooks.iter_mut().zip(&self.previous) {
            if !(enabled || owned) || hook.enabled == was_enabled && !hook.needs_flush {
                continue;
            }
            let bytes = if was_enabled {
                &hook.patch[..hook.patch_len]
            } else {
                &hook.original[..hook.patch_len]
            };
            unsafe {
                ptr::copy_nonoverlapping(bytes.as_ptr(), hook.site as *mut u8, hook.patch_len);
            }
            hook.enabled = was_enabled;
            hook.needs_flush = true;
        }
    }

    /// Still suspended: restore the page protections this group changed.
    fn restore_protection(&mut self) {
        let mut ok = true;
        for (hook, _) in self
            .hooks
            .iter_mut()
            .zip(&self.previous)
            .filter(|(_, previous)| self.enabled || previous.1)
            .take(self.protected)
        {
            if restore_refused() {
                hook.diagnostic_failure = true;
                ok = false;
                continue;
            }
            let mut ignored = 0;
            let restored = unsafe {
                VirtualProtect(
                    hook.site as *const c_void,
                    hook.sentinel_len,
                    hook.page_protection,
                    &mut ignored,
                )
            } != 0;
            hook.diagnostic_failure |= !restored;
            ok &= restored;
        }
        self.protection_ok &= ok;
        self.protected = 0;
    }

    /// A prior family's restore can cover the same page. Reopen this
    /// family's selected sites using its immutable saved protection for the
    /// later restore; do not save the temporary writable protection.
    fn reopen(&mut self) -> Result<(), u32> {
        let mut protected = 0;
        for (hook, &(_, owned)) in self.hooks.iter_mut().zip(&self.previous) {
            if !(self.enabled || owned) {
                continue;
            }
            #[cfg(any(test, feature = "test-support"))]
            if FAIL_REOPENS.with(|failures| {
                let (skip, count) = failures.get();
                failures.set(if skip != 0 {
                    (skip - 1, count)
                } else {
                    (0, count.saturating_sub(1))
                });
                skip == 0 && count != 0
            }) {
                hook.diagnostic_failure = true;
                return Err(31);
            }
            let mut ignored = 0;
            if unsafe {
                VirtualProtect(
                    hook.site as *const c_void,
                    hook.sentinel_len,
                    PAGE_EXECUTE_READWRITE,
                    &mut ignored,
                )
            } == 0
            {
                hook.diagnostic_failure = true;
                return Err(unsafe { GetLastError() });
            }
            protected += 1;
            self.protected = self.protected.max(protected);
        }
        Ok(())
    }

    /// The family's outcome, with its own publication's messages and order.
    fn finish(
        &mut self,
        census_refusal: Option<&'static str>,
        resume: &Result<(), u32>,
    ) -> Result<bool, String> {
        if !self.protection_ok {
            return Err("failed to restore hook-page protection".into());
        }
        if let Err(code) = resume {
            return Err(format!("ResumeThread failed: {code}"));
        }
        if let Some(reason) = self.prepare_error.take() {
            return Err(reason);
        }
        if let Some(code) = self.protection_error {
            let reason = io::Error::from_raw_os_error(code as i32);
            return Err(if self.publication_failed {
                format!(
                    "{reason}; instruction-cache publication failed; rollback page preparation failed; executable storage retained for detach"
                )
            } else {
                reason.to_string()
            });
        }
        self.checked.unwrap_or(Ok(false)).map_err(|code| {
            let publication = if self.hooks.iter().any(|hook| hook.needs_flush) {
                "; restored bytes remain unflushed or unverified; executable storage retained for detach"
            } else {
                ""
            };
            if self.publication_failed {
                format!(
                    "instruction-cache publication failed (Windows error {code}); prior bytes restored{publication}"
                )
            } else if self.snapshot_refusal {
                BYTE_VERIFICATION_FAILURE.into()
            } else if let Some(reason) = census_refusal {
                format!("{reason} (code {code:#x})")
            } else {
                format!("patch refused or verification failed (Windows error {code}){publication}")
            }
        })
    }
}

/// Every group under one census and suspension, each with the outcome its
/// own publication ([`Hook::set_family_enabled`]) would have given (but for
/// the shared flush and resume failures in the module docs). A group with
/// nothing to write is done without suspending.
#[cfg(test)]
pub(super) fn publish_groups(groups: &mut [Group<'_>], flush: impl FnMut() -> Result<(), u32>) {
    publish_groups_with_order(groups, flush, false);
}

fn publish_groups_with_order(
    groups: &mut [Group<'_>],
    mut flush: impl FnMut() -> Result<(), u32>,
    prefix: bool,
) {
    if prefix {
        // Empty predecessors still prepare once, without a controller. The
        // first real family's preparation precedes even a census refusal.
        for group in groups.iter_mut() {
            group.prepare_observation();
            if !group.hooks.is_empty() {
                break;
            }
            group.outcome = Some(Ok(true));
        }
    }
    for group in groups.iter_mut() {
        if !prefix
            && (group.hooks.is_empty()
                || !group.enabled && group.hooks.iter().all(|hook| !hook.is_enabled()))
        {
            group.outcome = Some(Ok(true));
        }
    }
    if groups.iter().all(|group| group.outcome.is_some()) {
        return;
    }
    // Taken before any protection change and released after the last
    // restore and resume (see `PUBLICATION`).
    let _publication = PUBLICATION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut threads = match Threads::open() {
        Ok(threads) => threads,
        Err(reason) => {
            for group in groups.iter_mut().filter(|group| group.outcome.is_none()) {
                group.outcome = Some(Err(reason.clone()));
            }
            return;
        }
    };
    for group in groups.iter_mut().filter(|group| group.outcome.is_none()) {
        group.prepare(prefix);
        if prefix && group.prepare_error.is_some() {
            break;
        }
    }
    let ready = |group: &Group<'_>| group.outcome.is_none() && group.prepare_error.is_none();
    let can_publish = if prefix {
        groups
            .iter()
            .take_while(|group| group.prepare_error.is_none())
            .any(ready)
    } else {
        groups.iter().any(ready)
    };
    if can_publish {
        // No allocation, formatting, locks or destruction while suspended.
        match threads.suspend() {
            Err(code) => {
                for group in groups.iter_mut().filter(|group| ready(group)) {
                    group.checked = Some(Err(code));
                }
            }
            Ok(()) => {
                if prefix {
                    publish_prefix(groups, &mut threads, &mut flush);
                } else {
                    let mut wrote = false;
                    for group in groups.iter_mut().filter(|group| ready(group)) {
                        let begin = crate::clock::qpc();
                        let checked = group.check_and_write(&mut threads);
                        wrote |= checked == Ok(true);
                        group.checked = Some(checked);
                        group.ticks = crate::clock::qpc() - begin;
                    }
                    if wrote {
                        settle(groups, &mut flush);
                    }
                }
            }
        }
    }
    // Protections are restored before any thread runs again, as a family's
    // own publication does.
    for group in groups.iter_mut().filter(|group| group.outcome.is_none()) {
        group.restore_protection();
    }
    let census_refusal = threads.census_refusal();
    let resume = threads.resume();
    drop(threads);
    for group in groups.iter_mut().filter(|group| group.outcome.is_none()) {
        let outcome = group.finish(census_refusal, &resume);
        group.outcome = Some(outcome);
    }
}

/// One stable IP snapshot, but each family flushes, verifies and restores
/// its protections before the next can write. On any failed flush, all
/// written families roll back before resume, including earlier successes.
fn publish_prefix(
    groups: &mut [Group<'_>],
    threads: &mut Threads,
    flush: &mut impl FnMut() -> Result<(), u32>,
) {
    for index in 0..groups.len() {
        let group = &mut groups[index];
        if group.outcome.is_some() {
            continue;
        }
        group.prepare_observation();
        if group.prepare_error.is_some() {
            break;
        }
        if group.hooks.is_empty() {
            group.outcome = Some(Ok(true));
            continue;
        }
        let begin = crate::clock::qpc();
        if let Err(code) = group.reopen() {
            group.protection_error = Some(code);
            break;
        }
        let checked = group.check_and_write(threads);
        group.checked = Some(checked);
        if checked != Ok(true) {
            group.ticks = crate::clock::qpc() - begin;
            break;
        }
        if let Err(code) = flush() {
            rollback(&mut groups[..=index], code, flush, true);
            break;
        }
        verify(group);
        group.restore_protection();
        group.ticks = crate::clock::qpc() - begin;
        if group.checked != Some(Ok(true)) || !group.protection_ok {
            break;
        }
    }
}

/// Still suspended, after the writes: one flush for every written group,
/// then each group's verification; a failed flush restores every written
/// group's prior bytes, and a second failed flush keeps their executable
/// storage for detach to retry.
fn settle(groups: &mut [Group<'_>], flush: &mut impl FnMut() -> Result<(), u32>) {
    let written = |group: &Group<'_>| group.checked == Some(Ok(true));
    if let Err(code) = flush() {
        rollback(groups, code, flush, false);
        return;
    }
    for group in groups.iter_mut().filter(|group| written(group)) {
        verify(group);
    }
}

fn verify(group: &mut Group<'_>) {
    let mut verified = true;
    for (hook, &(_, owned)) in group.hooks.iter_mut().zip(&group.previous) {
        if (group.enabled || owned) && !hook.bytes_match(group.enabled) {
            hook.diagnostic_failure = true;
            verified = false;
            break;
        }
    }
    if verified {
        for hook in group.hooks.iter_mut() {
            hook.needs_flush = false;
        }
    } else {
        group.checked = Some(Err(0));
    }
}

fn rollback(
    groups: &mut [Group<'_>],
    code: u32,
    flush: &mut impl FnMut() -> Result<(), u32>,
    reopen: bool,
) {
    for group in groups
        .iter_mut()
        .filter(|group| group.checked == Some(Ok(true)))
    {
        group.publication_failed = true;
        group.checked = Some(Err(code));
        if reopen && let Err(error) = group.reopen() {
            group.protection_error = Some(error);
            // A page that could not be made writable must not be written.
            // Keep every executable owner reachable for detach to retry.
            for hook in group.hooks.iter_mut() {
                hook.needs_flush = true;
            }
            continue;
        }
        group.roll_back();
    }
    if flush().is_ok() {
        for group in groups
            .iter_mut()
            .filter(|group| group.publication_failed && group.protection_error.is_none())
        {
            if group
                .hooks
                .iter()
                .filter(|hook| hook.needs_flush)
                .all(|hook| hook.bytes_match(hook.enabled))
            {
                for hook in group.hooks.iter_mut() {
                    hook.needs_flush = false;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "shared_tests.rs"]
mod tests;
