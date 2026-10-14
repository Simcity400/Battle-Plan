//! Allocation-free notifications from the native writers of the relation
//! opinion `CRelationStatus+0x300` and of the serializable relation vector
//! `CRelationStatus+0xE8` ([`engine::opinion_mutations`]). A detour only
//! records and calls the original; it never changes game behaviour.
//!
//! `RecomputeOpinion` appends its status to a log when the opinion it leaves
//! differs from the one it found; `Reset`, the fifteen relation slot
//! replacements ([`engine::opinion_mutations::RELATIONS`]) and the war body,
//! which the stability family patches and forwards ([`relation_replaced`]),
//! append theirs on every call, since they rewrite the status's relation
//! vector. The status's owner is then read again, opinions and relation
//! tuples alike. The observation holds only while that forwarder is enabled
//! ([`set_war_forward`]). `LoadToken` and the
//! constructor run only while a game state is loaded or rebuilt, so they end
//! the observation (a new epoch) instead. Each hour's relations refresh drains
//! the log since its last one ([`drain`]) and reads every owner a mark names;
//! anything unobserved (disabled, poisoned, a writer running, an overflowed or
//! missing log, a new epoch) makes it read every owner.
//!
//! The log is sized from observed demand: the refresh offers a larger one off
//! the game thread ([`offer`]) from the marks it found pending and the nonzero
//! opinions it holds, and the next drain installs it while no writer runs.
//! The hook path never allocates; a write with no log installed ends the
//! observation.
//!
//! A writer whose prologue differs from the pinned sentinel, or that cannot be
//! detoured, leaves every hook uninstalled: opinions have an exact full-read
//! fallback, so the collector still attaches and reads every status on every
//! refresh (`unhooked` and `unobserved_refreshes` in `status.json`).
use crate::hook::Hook;
use engine::{
    Memory,
    crelation_status::Mark,
    opinion_mutations::{MUTATIONS, RELATIONS},
};
use std::{
    ops::Range,
    ptr,
    sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
};

/// Marks between two drains, slot `n % len` holding mark `n`: each mark's
/// status and its owning diplomacy (`CRelationStatus+0x1F0`), from which the
/// refresh finds the owner to read.
struct Log {
    statuses: Box<[AtomicUsize]>,
    owners: Box<[AtomicUsize]>,
}

impl Log {
    /// Off the game thread.
    fn new(len: usize) -> Option<Box<Self>> {
        let slots = || {
            let mut slots = engine::allocation::vec(len).ok()?;
            slots.resize_with(len, || AtomicUsize::new(0));
            Some(slots.into_boxed_slice())
        };
        let (statuses, owners) = (slots()?, slots()?);
        Some(Box::new(Self { statuses, owners }))
    }

    fn len(&self) -> u64 {
        self.statuses.len() as u64
    }
}

/// The installed log; null until the first drain after an offer.
static LOG: AtomicPtr<Log> = AtomicPtr::new(ptr::null_mut());
/// A larger log allocated off-thread, awaiting a drain.
static OFFERED: AtomicPtr<Log> = AtomicPtr::new(ptr::null_mut());
/// A replaced log a writer begun before its replacement may still hold.
static DRAINING: AtomicPtr<Log> = AtomicPtr::new(ptr::null_mut());
/// A replaced log nothing holds, awaiting [`collect`].
static RETIRED: AtomicPtr<Log> = AtomicPtr::new(ptr::null_mut());
/// Logs installed.
static INSTALLS: AtomicU64 = AtomicU64::new(0);
/// Slots of the installed and the offered log, read without touching either.
static INSTALLED_LEN: AtomicUsize = AtomicUsize::new(0);
static OFFERED_LEN: AtomicUsize = AtomicUsize::new(0);

/// Free a log this module owns exclusively.
fn free(log: *mut Log) {
    if !log.is_null() {
        // SAFETY: each pointer came from Box::into_raw and is owned here.
        drop(unsafe { Box::from_raw(log) });
    }
}

/// The installed log's slots, while a writer or the consumer holds it.
fn installed() -> Option<&'static Log> {
    // SAFETY: a log is freed only after a drain replaced it and saw no
    // writer running, and the consumer is the only drain.
    unsafe { LOG.load(Ordering::SeqCst).as_ref() }
}
/// A value on its own cache line: writers on every TBB worker update it, so
/// it must not share a line with anything the capture or the writers read.
#[repr(align(64))]
pub struct Padded<T>(pub(crate) T);
impl<T> std::ops::Deref for Padded<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

/// Marks ever appended; the slot of mark `n` is `n % len` of the log.
static CURSOR: Padded<AtomicU64> = Padded(AtomicU64::new(0));
static EPOCH: AtomicU64 = AtomicU64::new(1);
static ENABLED: AtomicBool = AtomicBool::new(false);
/// The stability family forwards replace relation (the war body).
static WAR_FORWARD: AtomicBool = AtomicBool::new(false);
static POISONED: AtomicBool = AtomicBool::new(false);
pub static ACTIVE: Padded<AtomicUsize> = Padded(AtomicUsize::new(0));
const SITES: usize = MUTATIONS.len() + RELATIONS.len();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Calls per site, except `RecomputeOpinion`: it runs over every existing
/// pair each day, so only its changes are counted (as marks). Then the
/// forwarded war body.
static EVENTS: [AtomicU64; SITES + 1] = [const { AtomicU64::new(0) }; SITES + 1];
/// Why the hooks are not installed, when they are not.
static UNHOOKED: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
/// `[observed refreshes, unobserved refreshes, overflowed logs, refreshes
/// whose sampled comparison could not run, refreshes that found the store
/// lent, marks drained]`.
static COUNTS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
static SAMPLED: crate::sampled_verify::Sampled =
    crate::sampled_verify::Sampled::new(&["replayed opinions differ from a full read"]);
/// The walks' own counts (`crelation_status::BaselineCounts`), accumulated.
static WALKS: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];

/// A load, gap or recovery: no baseline recorded before it is current.
pub fn invalidate() {
    EPOCH.fetch_add(1, Ordering::SeqCst);
}

/// The opinion a live status holds. The receiver is the status the native
/// writer is about to read and write.
fn opinion(status: usize) -> i32 {
    let address = status.wrapping_add(engine::crelation_status::OPINION);
    unsafe { std::ptr::read_volatile(address as *const i32) }
}

/// The owning diplomacy of a live status the native writer is writing. Only
/// the status constructor writes it (`engine::crelation_status::DIPLOMACY_STATUS`).
fn diplomacy(status: usize) -> usize {
    let address = status.wrapping_add(engine::crelation_status::DIPLOMACY_STATUS);
    unsafe { std::ptr::read_volatile(address as *const usize) }
}

struct Mutation;
impl Mutation {
    fn enter(site: usize) -> Self {
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        if site != 0 {
            EVENTS[site].fetch_add(1, Ordering::Relaxed);
        }
        Self
    }

    /// Append `status` and its owning diplomacy when its opinion changed.
    fn mark(&self, status: usize, before: i32) {
        if opinion(status) != before {
            self.mark_always(status);
        }
    }

    /// Append `status` and its owning diplomacy: a writer of its relation
    /// vector.
    fn mark_always(&self, status: usize) {
        let index = CURSOR.fetch_add(1, Ordering::SeqCst);
        publish(index, status, diplomacy(status));
    }
}

/// A forwarded writer of a status's relation vector, running; dropping it
/// marks the status, after the original returns.
pub(crate) struct Replaced {
    mutation: Mutation,
    status: usize,
}

impl Drop for Replaced {
    fn drop(&mut self) {
        self.mutation.mark_always(self.status);
    }
}

/// Hook path: the stability family's detour of replace relation
/// (`status`, new relation, slot) is about to run the original. Without this
/// family's hooks nothing is observed and nothing is marked.
pub(crate) fn relation_replaced(status: usize) -> Option<Replaced> {
    if !ENABLED.load(Ordering::SeqCst) {
        return None;
    }
    EVENTS[SITES].fetch_add(1, Ordering::Relaxed);
    Some(Replaced {
        mutation: Mutation::enter(0),
        status,
    })
}

/// The stability family's patch of replace relation is enabled.
/// Writes while it was not went unobserved.
pub(crate) fn set_war_forward(enabled: bool) {
    WAR_FORWARD.store(enabled, Ordering::SeqCst);
    invalidate();
}

/// Store mark `index`, from inside a [`Mutation`]. A writer that stalled until
/// writers had appended a whole log after its index may have overwritten a
/// newer mark in its slot. Every drain meanwhile saw it active and handed
/// nothing over, so it ends the observation (a new epoch) before it leaves,
/// and no drain after it can rely on that slot; so does a write with no log.
/// Returns whether it ended the observation.
fn publish(index: u64, status: usize, diplomacy: usize) -> bool {
    let Some(log) = installed() else {
        invalidate();
        return true;
    };
    let slot = (index % log.len()) as usize;
    log.owners[slot].store(diplomacy, Ordering::SeqCst);
    log.statuses[slot].store(status, Ordering::SeqCst);
    let lapped = CURSOR.load(Ordering::SeqCst).wrapping_sub(index) > log.len();
    if lapped {
        invalidate();
    }
    lapped
}
impl Drop for Mutation {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

/// What a refresh may rely on: `None` when writes go unobserved (disabled or
/// poisoned). Otherwise the epoch the observation continues, with the count
/// of marks since `consumed` copied into `output`, or `None` for them when
/// they cannot all be handed over (a writer mid-append, an overflowed or
/// missing log or short storage); the refresh then reads every owner.
///
/// `consumed` always advances. A mark below the cursor read here belongs to a
/// write that completed before this call, so a walk that follows it and reads
/// every status sees that write; later writes land at or after the cursor.
/// `required` receives the marks pending, so the caller can grow `output` and
/// [`offer`] a larger log off the game thread. A drain that saw no writer
/// running then installs an offered log.
pub fn drain(
    consumed: &mut u64,
    output: &mut [Mark],
    required: &mut usize,
) -> Option<(u64, Option<usize>)> {
    // The cursor before the writer check: a writer that took an earlier
    // index is still active until its slot is stored.
    let cursor = CURSOR.load(Ordering::SeqCst);
    let epoch = EPOCH.load(Ordering::SeqCst);
    let observing = ENABLED.load(Ordering::SeqCst)
        && WAR_FORWARD.load(Ordering::SeqCst)
        && !POISONED.load(Ordering::SeqCst);
    #[cfg(test)]
    BETWEEN_READS.with_borrow_mut(|between| {
        if let Some(between) = between.take() {
            between();
        }
    });
    let settled = ACTIVE.load(Ordering::SeqCst) == 0;
    let first = std::mem::replace(consumed, cursor);
    let pending = cursor.wrapping_sub(first);
    *required = usize::try_from(pending).unwrap_or(usize::MAX);
    let log = installed();
    if settled {
        install_offered();
    }
    if !observing {
        COUNTS[1].fetch_add(1, Ordering::Relaxed);
        return None;
    }
    // Without a log nothing was appended: every write ended the observation.
    let fits = log.map_or(pending == 0, |log| pending <= log.len());
    let marks = if fits {
        if !settled || pending as usize > output.len() {
            None
        } else if !log.is_none_or(|log| {
            copy_marks(log, first, &mut output[..pending as usize], || {
                CURSOR.load(Ordering::SeqCst)
            })
        }) {
            COUNTS[2].fetch_add(1, Ordering::Relaxed);
            None
        } else if EPOCH.load(Ordering::SeqCst) != epoch {
            // A writer that ended the observation (one lapped while it
            // stalled, say) left between the first epoch read and the
            // writer check: the copy may hold its stale mark, so none is
            // handed over. A writer still active is caught by the check;
            // one that left before the first read moved the epoch the
            // caller's baseline was recorded under.
            None
        } else {
            COUNTS[5].fetch_add(pending, Ordering::Relaxed);
            Some(pending as usize)
        }
    } else {
        COUNTS[2].fetch_add(1, Ordering::Relaxed);
        None
    };
    COUNTS[if marks.is_some() { 0 } else { 1 }].fetch_add(1, Ordering::Relaxed);
    Some((epoch, marks))
}

/// The consumer, after copying the marks under the installed log while no
/// writer ran: install an offered larger log. Marks after the cursor land in
/// it. A writer that began meanwhile may still store into the replaced log,
/// so it ends the observation, and the replaced log is freed only once a
/// later drain sees no writer running.
fn install_offered() {
    let previous = DRAINING.swap(ptr::null_mut(), Ordering::SeqCst);
    if !previous.is_null()
        && RETIRED
            .compare_exchange(
                ptr::null_mut(),
                previous,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_err()
    {
        DRAINING.store(previous, Ordering::SeqCst);
        return;
    }
    let offered = OFFERED.swap(ptr::null_mut(), Ordering::SeqCst);
    if offered.is_null() {
        return;
    }
    INSTALLED_LEN.store(OFFERED_LEN.swap(0, Ordering::SeqCst), Ordering::SeqCst);
    let replaced = LOG.swap(offered, Ordering::SeqCst);
    DRAINING.store(replaced, Ordering::SeqCst);
    INSTALLS.fetch_add(1, Ordering::Relaxed);
    if ACTIVE.load(Ordering::SeqCst) != 0 {
        invalidate();
    }
}

/// Off the game thread: allocate a log for `marks` marks between two drains
/// when the installed and offered logs are smaller. A drain installs it.
pub fn offer(marks: usize) {
    let Some(len) = marks
        .checked_add(marks / 4)
        .and_then(|len| len.max(1).checked_next_power_of_two())
    else {
        return;
    };
    if len <= INSTALLED_LEN.load(Ordering::SeqCst) || len <= OFFERED_LEN.load(Ordering::SeqCst) {
        return;
    }
    let Some(log) = Log::new(len) else {
        return;
    };
    // A drain that takes the previous offer between these stores installs
    // it with this length; the next drain installs this larger one.
    OFFERED_LEN.store(len, Ordering::SeqCst);
    free(OFFERED.swap(Box::into_raw(log), Ordering::SeqCst));
}

/// Off the game thread: free a replaced log nothing holds any more.
pub fn collect() {
    free(RETIRED.swap(ptr::null_mut(), Ordering::SeqCst));
}

/// Free every log. No writer or drain may run: the hooks were removed and
/// drained and the acquisition pool has stopped.
fn release() {
    for log in [&LOG, &OFFERED, &DRAINING, &RETIRED] {
        free(log.swap(ptr::null_mut(), Ordering::SeqCst));
    }
    INSTALLED_LEN.store(0, Ordering::SeqCst);
    OFFERED_LEN.store(0, Ordering::SeqCst);
    invalidate();
}

#[cfg(test)]
thread_local! {
    /// Tests: runs inside [`drain`] between its epoch read and its writer check.
    static BETWEEN_READS: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Copy the marks from `first` into `output`. False when writers appended a
/// whole log's worth since `first` by the end of the copy (`cursor` reads the
/// position then): a slot copied above may hold a later mark instead.
fn copy_marks(log: &Log, first: u64, output: &mut [Mark], cursor: impl Fn() -> u64) -> bool {
    for (offset, mark) in output.iter_mut().enumerate() {
        let slot = ((first + offset as u64) % log.len()) as usize;
        *mark = Mark {
            status: log.statuses[slot].load(Ordering::SeqCst),
            diplomacy: log.owners[slot].load(Ordering::SeqCst),
        };
    }
    cursor().wrapping_sub(first) <= log.len()
}

/// The log position now: where a new consumer starts.
pub fn cursor() -> u64 {
    CURSOR.load(Ordering::SeqCst)
}

/// One refresh's sampled comparison ([`crate::sampled_verify`]): `verified`
/// replayed owners read in full, `mismatches` of them differing. A mismatch
/// poisons the observation for the rest of the attachment.
pub fn sampled(verified: u64, mismatches: u64) {
    if SAMPLED.record(verified, mismatches, 0) {
        POISONED.store(true, Ordering::SeqCst);
    }
}

/// A refresh's sampled comparison could not run.
pub fn unverified() {
    COUNTS[3].fetch_add(1, Ordering::Relaxed);
}

/// A refresh ran without the shared baseline because another job held it.
pub fn store_lent() {
    COUNTS[4].fetch_add(1, Ordering::Relaxed);
}

/// The baseline's cumulative counts after a walk; `previous` is what was
/// last reported for this baseline.
pub fn record(
    counts: engine::crelation_status::BaselineCounts,
    previous: &mut engine::crelation_status::BaselineCounts,
) {
    let now = [
        counts.full_walks,
        counts.incremental_walks,
        counts.reused_owners,
        counts.marks,
        counts.refused_observations,
        counts.read_owners,
        counts.invalidated_owners,
    ];
    let before = [
        previous.full_walks,
        previous.incremental_walks,
        previous.reused_owners,
        previous.marks,
        previous.refused_observations,
        previous.read_owners,
        previous.invalidated_owners,
    ];
    for ((total, now), before) in WALKS.iter().zip(now).zip(before) {
        total.fetch_add(now.saturating_sub(before), Ordering::Relaxed);
    }
    *previous = counts;
}

pub fn status() -> impl serde::Serialize {
    let counts = COUNTS.each_ref().map(|value| value.load(Ordering::Relaxed));
    let walks = WALKS.each_ref().map(|value| value.load(Ordering::Relaxed));
    let event_counts = EVENTS.each_ref().map(|value| value.load(Ordering::Relaxed));
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .chain(&RELATIONS)
            .map(|site| site.name)
            .chain(["replace_relation_forwarded"])
            .zip(event_counts)
            .skip(1),
    );
    crate::status::status!({
        "configured": cfg!(feature = "opinion_changes"),
        "unhooked": UNHOOKED.get(),
        "war_forward": WAR_FORWARD.load(Ordering::SeqCst),
        "verify": cfg!(feature = "opinion_verify"),
        "poisoned": POISONED.load(Ordering::SeqCst),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "unverified_refreshes": counts[3],
        "observed_refreshes": counts[0],
        "unobserved_refreshes": counts[1],
        "overflowed_logs": counts[2],
        "store_lent": counts[4],
        "marks": counts[5],
        "log_capacity": INSTALLED_LEN.load(Ordering::Relaxed),
        "log_installs": INSTALLS.load(Ordering::Relaxed),
        "full_walks": walks[0],
        "incremental_walks": walks[1],
        "reused_owners": walks[2],
        "read_marks": walks[3],
        "refused_observations": walks[4],
        "read_owners": walks[5],
        "invalidated_owners": walks[6],
        "events": events,
        "scope": "cumulative since attach; each hour reads one owner slice and every owner a drained mark names (every owner when the marks are not handed over: overflowed_logs counts a log too small for the hour's marks, which offers a larger one); an incremental walk replays each selected owner whose country, diplomacy and matrix are unchanged and reads only the statuses a writer changed since its last read, and reads an invalid owner in full (read_owners); each refresh also reads in full the replayed owners of its sampled slice, which rotates over days so each owner is compared at one refresh in rotation_hours (every replayed owner in verify mode), publishes a full read and disables reuse at the first mismatch (sampled_*, owners); events count calls except recompute_opinion, whose changes are the marks"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// Never fails: without every hook, refreshes read every status.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let hooks = if cfg!(feature = "opinion_changes") {
            unsafe { Self::install(memory, base) }.unwrap_or_else(|reason| {
                let _ = UNHOOKED.set(reason);
                Vec::new()
            })
        } else {
            Vec::new()
        };
        Ok(Self { hooks })
    }

    /// Every sentinel is checked before any detour is created.
    unsafe fn install(memory: &impl Memory, base: usize) -> Result<Vec<Hook>, &'static str> {
        let callbacks: [usize; SITES] = [
            recompute_opinion as *const () as usize,
            reset as *const () as usize,
            load_token as *const () as usize,
            construct as *const () as usize,
            air_relation as *const () as usize,
            attache_relation as *const () as usize,
            boost_party_relation as *const () as usize,
            subject_relation as *const () as usize,
            docking_relation as *const () as usize,
            embargo_relation as *const () as usize,
            equipment_purchase_relation as *const () as usize,
            guarantee_relation as *const () as usize,
            improve_relation as *const () as usize,
            lend_lease_relation as *const () as usize,
            licence_relation as *const () as usize,
            market_access_relation as *const () as usize,
            military_access_relation as *const () as usize,
            blockade_relation as *const () as usize,
            non_aggression_relation as *const () as usize,
        ];
        let sites = || MUTATIONS.iter().chain(&RELATIONS);
        for site in sites() {
            if engine::read::<[u8; 16]>(memory, base + site.rva) != Some(site.sentinel) {
                return Err("opinion hook sentinel differs");
            }
        }
        let mut hooks = Vec::new();
        for (index, site) in sites().enumerate() {
            let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
                .map_err(|_| "native hook construction failed")?;
            ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
            hooks.push(hook);
        }
        Ok(hooks)
    }

    /// Writes while the hooks were not all enabled went unobserved, so every
    /// change of state starts a new epoch.
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
            crate::war_changes::forwarder(crate::war_changes::Forwarder::Opinions, false);
            crate::country_input_changes::forwarder(
                crate::country_input_changes::Forwarder::Opinions,
                false,
            );
        }
        // One census and suspension for the whole family, shared with the
        // other families in a runtime round (`hook::shared`): a thread at any
        // of its sites, or in a tap, holds every site back until the next
        // attempt.
        if !Hook::set_family_enabled_shared_prepared(&mut self.hooks, enabled, invalidate)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        ENABLED.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        // The war domain observes the status reset, constructor and reader and
        // the relation slot replacements, which move a war's place in its
        // owners' vectors ([`crate::war_changes`]).
        crate::war_changes::forwarder(
            crate::war_changes::Forwarder::Opinions,
            enabled && !self.hooks.is_empty(),
        );
        // The country counter inputs observe the status reset and the
        // relation slot replacements, which rewrite the owner's
        // co-belligerent byte and slot-zero relations
        // ([`crate::country_input_changes`]).
        crate::country_input_changes::forwarder(
            crate::country_input_changes::Forwarder::Opinions,
            enabled && !self.hooks.is_empty(),
        );
        Ok(true)
    }

    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // Like each Hook's relay, the logs are dropped only once the hooks
        // were never enabled, or were removed and drained after the last
        // refresh.
        release();
    }
}

fn original(site: usize) -> usize {
    ORIGINAL[site].load(Ordering::Acquire)
}

// Every argument is an integer register and each is passed through unchanged.
unsafe extern "system" fn recompute_opinion(status: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::OpinionChanges);
    let mutation = Mutation::enter(0);
    let before = opinion(status);
    let original: unsafe extern "system" fn(usize) = unsafe { std::mem::transmute(original(0)) };
    timing.original(|| unsafe { original(status) });
    mutation.mark(status, before);
}

unsafe extern "system" fn reset(status: usize, first: usize, second: usize, value: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::OpinionChanges);
    let mutation = Mutation::enter(1);
    let _wars = crate::war_changes::forwarded(crate::war_changes::Site::StatusReset);
    let _inputs = crate::country_input_changes::forwarded(
        crate::country_input_changes::Forward::RelationReset,
        status,
        0,
    );
    let original: unsafe extern "system" fn(usize, usize, usize, usize) =
        unsafe { std::mem::transmute(original(1)) };
    timing.original(|| unsafe { original(status, first, second, value) });
    // It also clears and deletes the residual relation vector.
    mutation.mark_always(status);
}

// Each relation slot replacement takes (status, new relation, slot) in
// RCX, RDX and R8 and returns nothing; it marks its status on every call.
macro_rules! relation {
    ($name:ident, $index:expr) => {
        unsafe extern "system" fn $name(status: usize, relation: usize, slot: usize) {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::OpinionChanges);
            let site = MUTATIONS.len() + $index;
            let mutation = Mutation::enter(site);
            let _wars = crate::war_changes::forwarded(crate::war_changes::Site::RelationSlot);
            let _inputs = crate::country_input_changes::forwarded(
                crate::country_input_changes::Forward::RelationSlot,
                status,
                0,
            );
            let original: unsafe extern "system" fn(usize, usize, usize) =
                unsafe { std::mem::transmute(original(site)) };
            timing.original(|| unsafe { original(status, relation, slot) });
            mutation.mark_always(status);
        }
    };
}
relation!(air_relation, 0);
relation!(attache_relation, 1);
relation!(boost_party_relation, 2);
relation!(subject_relation, 3);
relation!(docking_relation, 4);
relation!(embargo_relation, 5);
relation!(equipment_purchase_relation, 6);
relation!(guarantee_relation, 7);
relation!(improve_relation, 8);
relation!(lend_lease_relation, 9);
relation!(licence_relation, 10);
relation!(market_access_relation, 11);
relation!(military_access_relation, 12);
relation!(blockade_relation, 13);
relation!(non_aggression_relation, 14);

unsafe extern "system" fn load_token(status: usize, reader: usize, token: i32) {
    // A refresh during the call sees the writer active and reads everything;
    // the epoch moves before it ends, so no baseline recorded then survives.
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::OpinionChanges);
    let _mutation = Mutation::enter(2);
    let _wars = crate::war_changes::forwarded(crate::war_changes::Site::StatusLoad);
    let original: unsafe extern "system" fn(usize, usize, i32) =
        unsafe { std::mem::transmute(original(2)) };
    timing.original(|| unsafe { original(status, reader, token) });
    invalidate();
}

unsafe extern "system" fn construct(storage: usize, diplomacy: usize, target: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::OpinionChanges);
    let _mutation = Mutation::enter(3);
    let _wars = crate::war_changes::forwarded(crate::war_changes::Site::StatusConstruct);
    invalidate();
    let original: unsafe extern "system" fn(usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(original(3)) };
    let result = timing.original(|| unsafe { original(storage, diplomacy, target) });
    invalidate();
    result
}

/// Tests that touch the process-wide observation state take this first.
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Observation on (true) or off, unpoisoned. Hold [`test_lock`].
#[cfg(test)]
pub(crate) fn observe_for_test(enabled: bool) {
    POISONED.store(false, Ordering::SeqCst);
    WAR_FORWARD.store(enabled, Ordering::SeqCst);
    ENABLED.store(enabled, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Slots of the log the tests install.
    const LEN: usize = 8;

    /// Install a fresh `LEN`-slot log, as an offer and a drain would, with no
    /// replaced or retired log left by an earlier test to defer the next
    /// install.
    fn install_for_test() -> &'static Log {
        release();
        let log = Box::into_raw(Log::new(LEN).unwrap());
        free(OFFERED.swap(log, Ordering::SeqCst));
        OFFERED_LEN.store(LEN, Ordering::SeqCst);
        install_offered();
        installed().unwrap()
    }

    /// A copy that writers lapped is refused.
    #[test]
    fn a_lapped_log_refuses_its_copy() {
        let log = Log::new(LEN).unwrap();
        let mut output = [Mark::default(); 2];
        assert!(copy_marks(&log, 5, &mut output, || 5 + LEN as u64));
        assert!(!copy_marks(&log, 5, &mut output, || 6 + LEN as u64));
    }

    /// A writer that stalls while writers append a whole log after its index
    /// may store its mark over a newer one. Drains meanwhile see it active and
    /// hand nothing over; it ends the observation before it leaves.
    #[test]
    fn a_writer_lapped_while_stalled_ends_the_observation() {
        let _lock = test_lock();
        observe_for_test(true);
        let log = install_for_test();
        let mut consumed = cursor();
        let (mut output, mut required) = ([Mark::default(); 4], 0);
        // The stalled writer takes its index and pauses before storing.
        let stalled = Mutation::enter(0);
        let index = CURSOR.fetch_add(1, Ordering::SeqCst);
        // A whole log of marks lands after it, the last one in its slot.
        for offset in 0..LEN {
            let writer = Mutation::enter(0);
            let at = CURSOR.fetch_add(1, Ordering::SeqCst);
            assert!(!publish(at, 0x1000 + offset, 0x2000));
            drop(writer);
        }
        assert_eq!(
            log.statuses[(index % LEN as u64) as usize].load(Ordering::SeqCst),
            0x1000 + LEN - 1
        );
        // A drain meanwhile sees the stalled writer and hands nothing over.
        assert_eq!(
            drain(&mut consumed, &mut output, &mut required).map(|(_, marks)| marks),
            Some(None)
        );
        // It stores its stale mark over the newer one and ends the observation.
        assert!(publish(index, 0xdead, 0xbeef));
        drop(stalled);
        // A writer that was not lapped does not.
        let writer = Mutation::enter(0);
        let at = CURSOR.fetch_add(1, Ordering::SeqCst);
        assert!(!publish(at, 0x3000, 0x2000));
        drop(writer);
        observe_for_test(false);
    }

    /// The lapped writer finishes between a drain's epoch read and its writer
    /// check: the drain sees no active writer and a whole log of pending marks
    /// that fits, yet hands none over under the epoch it read first.
    #[test]
    fn a_lapped_writer_leaving_mid_drain_hands_over_no_marks() {
        let _lock = test_lock();
        observe_for_test(true);
        install_for_test();
        let mut consumed = cursor();
        let mut output = vec![Mark::default(); LEN];
        let mut required = 0;
        let stalled = Mutation::enter(0);
        let index = CURSOR.fetch_add(1, Ordering::SeqCst);
        // A drain while it is stalled hands nothing over and moves past it.
        assert_eq!(
            drain(&mut consumed, &mut output, &mut required).map(|(_, marks)| marks),
            Some(None)
        );
        assert_eq!(consumed, index + 1);
        for offset in 0..LEN {
            let writer = Mutation::enter(0);
            let at = CURSOR.fetch_add(1, Ordering::SeqCst);
            publish(at, 0x1000 + offset, 0x2000);
            drop(writer);
        }
        let before = EPOCH.load(Ordering::SeqCst);
        BETWEEN_READS.with_borrow_mut(|between| {
            *between = Some(Box::new(move || {
                // Its stale store over the newest mark, then it leaves.
                assert!(publish(index, 0xdead, 0xbeef));
                drop(stalled);
            }))
        });
        let drained = drain(&mut consumed, &mut output, &mut required);
        assert_eq!(required, LEN);
        let (epoch, marks) = drained.unwrap();
        assert_eq!(epoch, before, "the epoch read before the writer left");
        assert_eq!(marks, None, "no marks handed over under that epoch");
        // The next drain is under the new epoch, which no baseline recorded
        // before it can continue.
        let (next, _) = drain(&mut consumed, &mut output, &mut required).unwrap();
        assert!(next > before);
        observe_for_test(false);
    }

    /// The log is sized from demand: an hour with more marks than it holds
    /// hands none over and reports them, an offer sized from them is
    /// installed by the next drain, and the same burst then fits. A write
    /// with no log installed ends the observation.
    #[test]
    fn an_overflowing_hour_offers_a_log_the_next_drain_installs() {
        let _lock = test_lock();
        observe_for_test(true);
        install_for_test();
        let mut consumed = cursor();
        let mut output = vec![Mark::default(); 4 * LEN];
        let mut required = 0;
        let burst = |count: usize| {
            for offset in 0..count {
                let writer = Mutation::enter(0);
                let at = CURSOR.fetch_add(1, Ordering::SeqCst);
                publish(at, 0x1000 + offset, 0x2000);
                drop(writer);
            }
        };
        burst(3 * LEN);
        assert_eq!(
            drain(&mut consumed, &mut output, &mut required).map(|(_, marks)| marks),
            Some(None)
        );
        assert_eq!(required, 3 * LEN);
        offer(required);
        assert_eq!(INSTALLED_LEN.load(Ordering::SeqCst), LEN);
        // The drain that installs it hands over the hour's marks, which
        // landed in the new log.
        assert_eq!(
            drain(&mut consumed, &mut output, &mut required).map(|(_, marks)| marks),
            Some(Some(0))
        );
        assert_eq!(INSTALLED_LEN.load(Ordering::SeqCst), 32);
        burst(3 * LEN);
        assert_eq!(
            drain(&mut consumed, &mut output, &mut required).map(|(_, marks)| marks),
            Some(Some(3 * LEN))
        );
        assert_eq!(output[3 * LEN - 1].status, 0x1000 + 3 * LEN - 1);
        collect();
        // No log: a write ends the observation instead of being logged.
        release();
        let before = EPOCH.load(Ordering::SeqCst);
        let writer = Mutation::enter(0);
        assert!(publish(
            CURSOR.fetch_add(1, Ordering::SeqCst),
            0x1000,
            0x2000
        ));
        drop(writer);
        assert!(EPOCH.load(Ordering::SeqCst) > before);
        observe_for_test(false);
    }

    /// An unrecognised prologue installs nothing and fails nothing: every
    /// refresh then reads every status, and the reason is reported.
    #[test]
    fn unrecognised_prologues_degrade_to_full_reads() {
        let _lock = test_lock();
        let bytes = vec![0_u8; 0x10];
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut hooks = unsafe { Hooks::new(&memory, 0) }.unwrap();
        assert!(hooks.executable_ranges().next().is_none());
        assert_eq!(hooks.set_enabled(true), Ok(true));
        let (mut consumed, mut output, mut required) = (cursor(), [Mark::default(); 4], 0);
        assert_eq!(drain(&mut consumed, &mut output, &mut required), None);
        assert_eq!(
            serde_json::to_value(status()).unwrap()["unhooked"],
            "opinion hook sentinel differs"
        );
    }

    /// The log hands each mark over once, refuses when a writer is active,
    /// overflowed or disabled, and always advances the consumer.
    #[test]
    fn drained_marks_are_exact_and_refuse_what_they_cannot_account_for() {
        let _lock = test_lock();
        let mut status = [0_i32; 0x400];
        let address = status.as_mut_ptr() as usize;
        let at = engine::crelation_status::OPINION / 4;
        observe_for_test(true);
        install_for_test();
        let mut consumed = cursor();
        let (mut output, mut required) = ([Mark::default(); 4], 0);
        // The status's owning diplomacy, as the constructor stores it.
        status[engine::crelation_status::DIPLOMACY_STATUS / 4..][..2]
            .copy_from_slice(&[0x5678, 0x1234]);
        // Other tests may start epochs concurrently: compare the marks only.
        let drained = |consumed: &mut u64, output: &mut [Mark], required: &mut usize| {
            drain(consumed, output, required).map(|(_, marks)| marks)
        };
        // A write that leaves the value is not marked; one that changes it is.
        let mutation = Mutation::enter(0);
        mutation.mark(address, 0);
        status[at] = 7;
        mutation.mark(address, 0);
        // Active: no marks are handed over, but the consumer advances.
        assert_eq!(
            drained(&mut consumed, &mut output, &mut required),
            Some(None)
        );
        drop(mutation);
        assert_eq!(
            drained(&mut consumed, &mut output, &mut required),
            Some(Some(0))
        );
        let mutation = Mutation::enter(1);
        status[at] = 8;
        mutation.mark(address, 7);
        drop(mutation);
        assert_eq!(
            drained(&mut consumed, &mut output, &mut required),
            Some(Some(1))
        );
        assert_eq!(
            output[0],
            Mark {
                status: address,
                diplomacy: 0x1234_0000_5678,
            }
        );
        // Too many for the caller's storage: refused, and its demand noted.
        for _ in 0..5 {
            let mutation = Mutation::enter(0);
            status[at] += 1;
            mutation.mark(address, status[at] - 1);
        }
        assert_eq!(
            drained(&mut consumed, &mut output, &mut required),
            Some(None)
        );
        assert_eq!(required, 5);
        let (epoch, _) = drain(&mut consumed, &mut output, &mut required).unwrap();
        invalidate();
        let (next, marks) = drain(&mut consumed, &mut output, &mut required).unwrap();
        assert!(next > epoch);
        assert_eq!(marks, Some(0));
        // A sampled mismatch poisons: nothing is observed any more.
        sampled(1, 1);
        assert_eq!(drain(&mut consumed, &mut output, &mut required), None);
        observe_for_test(false);
        assert_eq!(drain(&mut consumed, &mut output, &mut required), None);
    }
}
