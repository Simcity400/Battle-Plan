//! Shared publication: several hook families restored under one thread
//! census and suspension, each with its own outcome.
//!
//! Only detach uses it: each restore round attempts every unrestored family
//! that takes part, and each family's outcome stands on its own (one that is
//! not restored is retried in the next round). Attach installs every family
//! on its own, in its chain's order, because a chain stops at its first
//! family that is not ready or refuses and must never have written a later
//! one.
//!
//! A family takes part through [`Hook::set_family_enabled_shared`]. The
//! runtime runs one [`Round`]:
//! - a collect pass calls each taking-part family's own `set_enabled`, whose
//!   publication is recorded instead of performed: it reports "not ready",
//!   so the family stops before anything that depends on the outcome;
//! - [`publish`] checks and writes every recorded family under one
//!   suspension, with exactly the checks a family's own publication makes;
//! - a report pass calls the families again, in their usual order and with
//!   their usual bookkeeping, and each publication returns its family's
//!   recorded outcome without suspending anything.
//!
//! A family whose sites a thread holds, or whose bytes, verification or
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
//! Two failures are shared rather than per family, because one suspension
//! and one flush serve every family: a failed instruction-cache flush
//! restores the prior bytes of every family written in the round and each
//! reports that failure (its own publication might have flushed and
//! succeeded before another failed), and a failed thread resume is reported
//! by every family of the round, ahead of its own preparation error (its
//! own publication would not have suspended at all). Both leave every
//! family's bytes and retained executable storage as conservative as its
//! own failure would. A publication the round did not record (outside a round,
//! a family's second publication in one call, or a recording that could not
//! be stored) publishes on its own as before.
//!
//! The recorded hook slices are raw pointers into the families' own hook
//! vectors. Between the collect pass and the report pass nothing touches
//! those vectors except [`publish`], so the pointers stay valid and are the
//! only access to them while it runs.
use super::*;
use std::cell::{Cell, RefCell};

/// One family's publication: its hooks, the requested state and, once
/// published, its outcome and the QPC ticks of its checks and writes.
struct Recorded {
    hooks: *mut Hook,
    len: usize,
    enabled: bool,
    outcome: Option<Result<bool, String>>,
    ticks: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Collect,
    Report,
}

thread_local! {
    static MODE: Cell<Option<Mode>> = const { Cell::new(None) };
    static RECORDED: RefCell<Vec<Recorded>> = const { RefCell::new(Vec::new()) };
    /// The checks-and-writes ticks of the family the last report returned.
    static REPORTED_TICKS: Cell<Option<i64>> = const { Cell::new(None) };
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
        MODE.set(Some(Mode::Collect));
        Some(Self {
            _thread: std::marker::PhantomData,
        })
    }

    /// Publish every family the collect pass recorded under one suspension,
    /// then answer their calls until the round is dropped.
    pub fn publish(&mut self) -> Shared {
        MODE.set(None);
        let shared = publish();
        MODE.set(Some(Mode::Report));
        shared
    }
}

impl Drop for Round {
    fn drop(&mut self) {
        MODE.set(None);
        RECORDED.with_borrow_mut(Vec::clear);
    }
}

/// [`Round`] as one call: `collect` (each taking-part family's
/// `set_enabled`), the shared publication, `shared` with its summary, then
/// `report` (the families' usual calls, which receive the recorded
/// outcomes). Inside another round, `report` runs alone.
#[cfg(test)]
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
    /// [`Hook::set_family_enabled`] that takes part in a shared [`Round`]:
    /// recorded in its collect pass (not ready), answered from the recording
    /// in its report pass, and published on its own otherwise.
    pub fn set_family_enabled_shared(hooks: &mut [Self], enabled: bool) -> Result<bool, String> {
        match MODE.get() {
            Some(Mode::Collect) => {
                if hooks.is_empty() || !enabled && hooks.iter().all(|hook| !hook.is_enabled()) {
                    // Nothing to write: the family proceeds as its own
                    // publication would let it.
                    return Ok(true);
                }
                RECORDED.with_borrow_mut(|recorded| {
                    // An unrecorded family publishes on its own in the
                    // report pass.
                    if recorded.try_reserve(1).is_ok() {
                        recorded.push(Recorded {
                            hooks: hooks.as_mut_ptr(),
                            len: hooks.len(),
                            enabled,
                            outcome: None,
                            ticks: 0,
                        });
                    }
                });
                Ok(false)
            }
            Some(Mode::Report) => {
                let found = RECORDED.with_borrow_mut(|recorded| {
                    recorded
                        .iter_mut()
                        .find(|entry| {
                            std::ptr::eq(entry.hooks.cast_const(), hooks.as_ptr())
                                && entry.len == hooks.len()
                                && entry.enabled == enabled
                                && entry.outcome.is_some()
                        })
                        .and_then(|entry| Some((entry.outcome.take()?, entry.ticks)))
                });
                if let Some((outcome, ticks)) = found {
                    REPORTED_TICKS.set(Some(ticks));
                    return outcome;
                }
                REPORTED_TICKS.set(None);
                Self::set_family_enabled(hooks, enabled)
            }
            None => {
                REPORTED_TICKS.set(None);
                Self::set_family_enabled(hooks, enabled)
            }
        }
    }
}

/// Publish every recorded family under one census and suspension.
fn publish() -> Shared {
    let begin = crate::sampling::qpc();
    let mut recorded = RECORDED.take();
    let families = recorded.len();
    if families != 0 {
        let mut groups: Vec<Group<'_>> = Vec::new();
        if groups.try_reserve_exact(families).is_ok() {
            for entry in &recorded {
                // SAFETY: a family's own hook slice, recorded in this round's
                // collect pass; nothing else reaches it until the report
                // pass, and no two families share a hook.
                let hooks = unsafe { std::slice::from_raw_parts_mut(entry.hooks, entry.len) };
                groups.push(Group::new(hooks, entry.enabled));
            }
            publish_groups(&mut groups, real_flush);
            for (entry, group) in recorded.iter_mut().zip(groups) {
                entry.ticks = group.ticks;
                entry.outcome = Some(group.outcome.unwrap_or(Ok(false)));
            }
        }
        // Without the groups' storage nothing recorded is answered: each
        // family publishes on its own in the report pass.
    }
    RECORDED.set(recorded);
    Shared {
        families,
        ticks: crate::sampling::qpc() - begin,
    }
}

#[cfg(test)]
thread_local! {
    /// Page-protection restores to let through, then to refuse, as a failed
    /// `VirtualProtect` would.
    static FAIL_RESTORES: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}
#[cfg(test)]
pub(super) fn fail_restores(skip: usize, count: usize) {
    FAIL_RESTORES.set((skip, count));
}
/// Whether a test refuses this page-protection restore.
#[cfg(test)]
fn restore_refused() -> bool {
    let (skip, count) = FAIL_RESTORES.get();
    if skip != 0 {
        FAIL_RESTORES.set((skip - 1, count));
        return false;
    }
    FAIL_RESTORES.set((0, count.saturating_sub(1)));
    count != 0
}
#[cfg(not(test))]
fn restore_refused() -> bool {
    false
}

fn real_flush() -> Result<(), u32> {
    #[cfg(test)]
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
    hooks: &'h [Hook],
    previous: &'h [(bool, bool)],
    enabled: bool,
) -> impl Iterator<Item = &'h Hook> + 'h {
    hooks
        .iter()
        .zip(previous)
        .filter(move |(_, (_, owned))| enabled || *owned)
        .map(|(hook, _)| hook)
}

/// One family in a shared publication.
pub(super) struct Group<'a> {
    hooks: &'a mut [Hook],
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
}

impl<'a> Group<'a> {
    pub(super) fn new(hooks: &'a mut [Hook], enabled: bool) -> Self {
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
        }
    }

    #[cfg(test)]
    pub(super) fn outcome(&self) -> Option<&Result<bool, String>> {
        self.outcome.as_ref()
    }

    /// Before the suspension: prior states, ranges and page protections,
    /// in a family's own publication's order.
    fn prepare(&mut self) {
        let len = self.hooks.len();
        let Ok(mut previous) = publication_storage(len, self.enabled) else {
            self.prepare_error = Some("hook state allocation failed".into());
            return;
        };
        previous.extend(
            self.hooks
                .iter()
                .map(|hook| (hook.enabled, hook.is_enabled())),
        );
        let Ok(mut ranges) = publication_storage(len, self.enabled) else {
            self.prepare_error = Some("hook ranges allocation failed".into());
            return;
        };
        ranges.extend(
            selected(self.hooks, &previous, self.enabled)
                .map(|hook| hook.site..hook.site + hook.sentinel_len),
        );
        self.previous = previous;
        self.ranges = ranges;
        for hook in selected(self.hooks, &self.previous, self.enabled) {
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
                self.prepare_error = Some(io::Error::last_os_error().to_string());
                return;
            }
            self.protected += 1;
        }
    }

    /// While suspended, exactly as a family's own publication checks: a held
    /// site or tap is `Ok(false)` and a changed snapshot `Err(0)`, before any
    /// write; then every selected hook that changes state is written.
    fn check_and_write(&mut self, threads: &Threads) -> Result<bool, u32> {
        if threads.contains_ip(&self.ranges)? {
            return Ok(false);
        }
        let enabled = self.enabled;
        for hook in selected(self.hooks, &self.previous, enabled) {
            let [relay, trampoline] = hook.executable_ranges();
            if let Some(tap) = &hook.tap
                && (!enabled || !hook.enabled)
                && (tap.in_flight.load(Ordering::SeqCst) != 0
                    || threads.contains_ip(&[relay, trampoline, tap.stub.clone()])?)
            {
                // A native target can hold site+5 on its stack while its
                // current IP is outside every collector range.
                return Ok(false);
            }
            if !hook.bytes_match(hook.enabled) {
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
        for hook in selected(self.hooks, &self.previous, self.enabled).take(self.protected) {
            if restore_refused() {
                ok = false;
                continue;
            }
            let mut ignored = 0;
            ok &= unsafe {
                VirtualProtect(
                    hook.site as *const c_void,
                    hook.sentinel_len,
                    hook.page_protection,
                    &mut ignored,
                )
            } != 0;
        }
        self.protection_ok = ok;
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
pub(super) fn publish_groups(groups: &mut [Group<'_>], mut flush: impl FnMut() -> Result<(), u32>) {
    for group in groups.iter_mut() {
        if group.hooks.is_empty()
            || !group.enabled && group.hooks.iter().all(|hook| !hook.is_enabled())
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
        group.prepare();
    }
    let ready = |group: &Group<'_>| group.outcome.is_none() && group.prepare_error.is_none();
    if groups.iter().any(ready) {
        // No allocation, formatting, locks or destruction while suspended.
        match threads.suspend() {
            Err(code) => {
                for group in groups.iter_mut().filter(|group| ready(group)) {
                    group.checked = Some(Err(code));
                }
            }
            Ok(()) => {
                let mut wrote = false;
                for group in groups.iter_mut().filter(|group| ready(group)) {
                    let begin = crate::sampling::qpc();
                    let checked = group.check_and_write(&threads);
                    wrote |= checked == Ok(true);
                    group.checked = Some(checked);
                    group.ticks = crate::sampling::qpc() - begin;
                }
                if wrote {
                    settle(groups, &mut flush);
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

/// Still suspended, after the writes: one flush for every written group,
/// then each group's verification; a failed flush restores every written
/// group's prior bytes, and a second failed flush keeps their executable
/// storage for detach to retry.
fn settle(groups: &mut [Group<'_>], flush: &mut impl FnMut() -> Result<(), u32>) {
    let written = |group: &Group<'_>| group.checked == Some(Ok(true));
    if let Err(code) = flush() {
        for group in groups.iter_mut().filter(|group| written(group)) {
            group.roll_back();
            group.publication_failed = true;
            group.checked = Some(Err(code));
        }
        if flush().is_ok() {
            for group in groups.iter_mut().filter(|group| group.publication_failed) {
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
        return;
    }
    for group in groups.iter_mut().filter(|group| written(group)) {
        let verified = selected(group.hooks, &group.previous, group.enabled)
            .all(|hook| hook.bytes_match(group.enabled));
        if verified {
            for hook in group.hooks.iter_mut() {
                hook.needs_flush = false;
            }
        } else {
            group.checked = Some(Err(0));
        }
    }
}

#[cfg(test)]
#[path = "shared_tests.rs"]
mod tests;
