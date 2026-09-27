//! Allocation-free notifications from the native writers of the relation
//! opinion `CRelationStatus+0x300` ([`engine::opinion_mutations`]). A detour
//! only records and calls the original; it never changes game behaviour.
//!
//! `RecomputeOpinion` and `Reset` append their status to a bounded log when
//! the opinion they leave differs from the one they found. `LoadToken` and the
//! constructor run only while a game state is loaded or rebuilt, so they end
//! the observation (a new epoch) instead. A relations refresh drains the log
//! since its last one ([`drain`]); anything unobserved (disabled, poisoned, a
//! writer running, an overflowed log, a new epoch) makes it read every status.
//!
//! A writer whose prologue differs from the pinned sentinel, or that cannot be
//! detoured, leaves every hook uninstalled: opinions have an exact full-read
//! fallback, so the collector still attaches and reads every status on every
//! refresh (`unhooked` and `unobserved_refreshes` in `status.json`).
use crate::hook::Hook;
use engine::{Memory, crelation_status::Mark, opinion_mutations::MUTATIONS};
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

/// Marks between two opinion refreshes before the log overflows. The daily
/// sweep changes at most every existing pair once a day.
const LOG: usize = 1 << 18;
static LOG_SLOTS: [AtomicUsize; LOG] = [const { AtomicUsize::new(0) }; LOG];
/// Each mark's owning diplomacy (`CRelationStatus+0x1F0`), in the slot of its
/// status: relations are read one owner slice per hour, so a mark waits for
/// its owner's slice.
static LOG_OWNERS: [AtomicUsize; LOG] = [const { AtomicUsize::new(0) }; LOG];
/// A value on its own cache line: writers on every TBB worker update it, so
/// it must not share a line with anything the capture or the writers read.
#[repr(align(64))]
pub struct Padded<T>(T);
impl<T> std::ops::Deref for Padded<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

/// Marks ever appended; the slot of mark `n` is `n % LOG`.
static CURSOR: Padded<AtomicU64> = Padded(AtomicU64::new(0));
static EPOCH: AtomicU64 = AtomicU64::new(1);
static ENABLED: AtomicBool = AtomicBool::new(false);
static POISONED: AtomicBool = AtomicBool::new(false);
pub static ACTIVE: Padded<AtomicUsize> = Padded(AtomicUsize::new(0));
static ORIGINAL: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
/// Calls per site, except `RecomputeOpinion`: it runs over every existing
/// pair each day, so only its changes are counted (as marks).
static EVENTS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
/// Why the hooks are not installed, when they are not.
static UNHOOKED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
/// `[observed refreshes, unobserved refreshes, overflowed logs, verified
/// refreshes, mismatches, unverified refreshes, refreshes that found the
/// store lent, marks drained]`.
static COUNTS: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
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
            let index = CURSOR.fetch_add(1, Ordering::SeqCst);
            publish(index, status, diplomacy(status));
        }
    }
}

/// Store mark `index`, from inside a [`Mutation`]. A writer that stalled until
/// writers had appended a whole log after its index may have overwritten a
/// newer mark in its slot. Every drain meanwhile saw it active and handed
/// nothing over, so it ends the observation (a new epoch) before it leaves,
/// and no drain after it can rely on that slot. Returns whether it was lapped.
fn publish(index: u64, status: usize, diplomacy: usize) -> bool {
    let slot = (index % LOG as u64) as usize;
    LOG_OWNERS[slot].store(diplomacy, Ordering::SeqCst);
    LOG_SLOTS[slot].store(status, Ordering::SeqCst);
    let lapped = CURSOR.load(Ordering::SeqCst).wrapping_sub(index) > LOG as u64;
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
/// they cannot all be handed over (a writer mid-append, an overflowed log or
/// short storage); the refresh then reads every status.
///
/// `consumed` always advances. A mark below the cursor read here belongs to a
/// write that completed before this call, so a walk that follows it and reads
/// every status sees that write; later writes land at or after the cursor.
/// `required` receives the marks pending, so the caller can grow `output` off
/// the game thread.
pub fn drain(
    consumed: &mut u64,
    output: &mut [Mark],
    required: &mut usize,
) -> Option<(u64, Option<usize>)> {
    // The cursor before the writer check: a writer that took an earlier
    // index is still active until its slot is stored.
    let cursor = CURSOR.load(Ordering::SeqCst);
    let epoch = EPOCH.load(Ordering::SeqCst);
    let observing = ENABLED.load(Ordering::SeqCst) && !POISONED.load(Ordering::SeqCst);
    #[cfg(test)]
    BETWEEN_READS.with_borrow_mut(|between| {
        if let Some(between) = between.take() {
            between();
        }
    });
    let settled = ACTIVE.load(Ordering::SeqCst) == 0;
    let first = std::mem::replace(consumed, cursor);
    let pending = cursor.wrapping_sub(first);
    *required = usize::try_from(pending).unwrap_or(usize::MAX).min(LOG);
    if !observing {
        COUNTS[1].fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let marks = if pending > LOG as u64 {
        COUNTS[2].fetch_add(1, Ordering::Relaxed);
        None
    } else if !settled || pending as usize > output.len() {
        None
    } else if !copy_marks(first, &mut output[..pending as usize], || {
        CURSOR.load(Ordering::SeqCst)
    }) {
        COUNTS[2].fetch_add(1, Ordering::Relaxed);
        None
    } else if EPOCH.load(Ordering::SeqCst) != epoch {
        // A writer that ended the observation (one lapped while it stalled,
        // say) left between the first epoch read and the writer check: the
        // copy may hold its stale mark, so none is handed over. A writer
        // still active is caught by the check; one that left before the
        // first read moved the epoch the caller's baseline was recorded
        // under.
        None
    } else {
        COUNTS[7].fetch_add(pending, Ordering::Relaxed);
        Some(pending as usize)
    };
    COUNTS[if marks.is_some() { 0 } else { 1 }].fetch_add(1, Ordering::Relaxed);
    Some((epoch, marks))
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
fn copy_marks(first: u64, output: &mut [Mark], cursor: impl Fn() -> u64) -> bool {
    for (offset, mark) in output.iter_mut().enumerate() {
        let slot = ((first + offset as u64) % LOG as u64) as usize;
        *mark = Mark {
            status: LOG_SLOTS[slot].load(Ordering::SeqCst),
            diplomacy: LOG_OWNERS[slot].load(Ordering::SeqCst),
        };
    }
    cursor().wrapping_sub(first) <= LOG as u64
}

/// The log position now: where a new consumer starts.
pub fn cursor() -> u64 {
    CURSOR.load(Ordering::SeqCst)
}

/// Whether refreshes compare every observed walk with one that reads every
/// status.
pub fn verify() -> bool {
    cfg!(feature = "opinion_verify")
}

/// A verify walk agreed (`Some(true)`), disagreed (`Some(false)`: the
/// observation is poisoned for the rest of the attachment) or could not run.
pub fn verified(result: Option<bool>) {
    match result {
        Some(true) => COUNTS[3].fetch_add(1, Ordering::Relaxed),
        Some(false) => {
            POISONED.store(true, Ordering::SeqCst);
            COUNTS[4].fetch_add(1, Ordering::Relaxed)
        }
        None => COUNTS[5].fetch_add(1, Ordering::Relaxed),
    };
}

/// A refresh ran without the shared baseline because another job held it.
pub fn store_lent() {
    COUNTS[6].fetch_add(1, Ordering::Relaxed);
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

pub fn status() -> serde_json::Value {
    let counts = COUNTS.each_ref().map(|value| value.load(Ordering::Relaxed));
    let walks = WALKS.each_ref().map(|value| value.load(Ordering::Relaxed));
    let events: serde_json::Map<String, serde_json::Value> = MUTATIONS
        .iter()
        .zip(&EVENTS)
        .skip(1)
        .map(|(site, count)| (site.name.to_owned(), count.load(Ordering::Relaxed).into()))
        .collect();
    serde_json::json!({
        "configured": cfg!(feature = "opinion_changes"),
        "unhooked": UNHOOKED.get(),
        "verify": verify(),
        "poisoned": POISONED.load(Ordering::SeqCst),
        "observed_refreshes": counts[0],
        "unobserved_refreshes": counts[1],
        "overflowed_logs": counts[2],
        "verified_refreshes": counts[3],
        "mismatches": counts[4],
        "unverified_refreshes": counts[5],
        "store_lent": counts[6],
        "marks": counts[7],
        "full_walks": walks[0],
        "incremental_walks": walks[1],
        "reused_owners": walks[2],
        "read_marks": walks[3],
        "refused_observations": walks[4],
        "read_owners": walks[5],
        "invalidated_owners": walks[6],
        "events": events,
        "scope": "cumulative since attach; relations are read one owner slice per hour; an incremental walk replays each selected owner whose country, diplomacy and matrix are unchanged and reads only the statuses a writer changed since its last read, and reads an invalid owner in full (read_owners); events count calls except recompute_opinion, whose changes are the marks"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// Never fails: without every hook, refreshes read every status.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, String> {
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
    unsafe fn install(memory: &impl Memory, base: usize) -> Result<Vec<Hook>, String> {
        let callbacks = [
            recompute_opinion as *const () as usize,
            reset as *const () as usize,
            load_token as *const () as usize,
            construct as *const () as usize,
        ];
        for site in &MUTATIONS {
            if engine::read::<[u8; 16]>(memory, base + site.rva) != Some(site.sentinel) {
                return Err(format!("opinion {} sentinel differs", site.name));
            }
        }
        let mut hooks = Vec::new();
        for (index, site) in MUTATIONS.iter().enumerate() {
            let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }?;
            ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
            hooks.push(hook);
        }
        Ok(hooks)
    }

    /// Writes while the hooks were not all enabled went unobserved, so every
    /// change of state starts a new epoch.
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
        }
        invalidate();
        for hook in &mut self.hooks {
            if !hook.set_enabled(enabled)? {
                return Ok(false);
            }
        }
        ENABLED.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        Ok(true)
    }

    pub fn executable_ranges(&self) -> Vec<Range<usize>> {
        self.hooks
            .iter()
            .flat_map(Hook::executable_ranges)
            .collect()
    }
}

fn original(site: usize) -> usize {
    ORIGINAL[site].load(Ordering::Acquire)
}

// Signatures from the pinned decompiles; every argument is an integer
// register and each is passed through unchanged.
unsafe extern "system" fn recompute_opinion(status: usize) {
    let mutation = Mutation::enter(0);
    let before = opinion(status);
    let original: unsafe extern "system" fn(usize) = unsafe { std::mem::transmute(original(0)) };
    unsafe { original(status) };
    mutation.mark(status, before);
}

unsafe extern "system" fn reset(status: usize, first: usize, second: usize, value: usize) {
    let mutation = Mutation::enter(1);
    let before = opinion(status);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) =
        unsafe { std::mem::transmute(original(1)) };
    unsafe { original(status, first, second, value) };
    mutation.mark(status, before);
}

unsafe extern "system" fn load_token(status: usize, reader: usize, token: i32) {
    // A refresh during the call sees the writer active and reads everything;
    // the epoch moves before it ends, so no baseline recorded then survives.
    let _mutation = Mutation::enter(2);
    let original: unsafe extern "system" fn(usize, usize, i32) =
        unsafe { std::mem::transmute(original(2)) };
    unsafe { original(status, reader, token) };
    invalidate();
}

unsafe extern "system" fn construct(storage: usize, diplomacy: usize, target: usize) -> usize {
    let _mutation = Mutation::enter(3);
    invalidate();
    let original: unsafe extern "system" fn(usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(original(3)) };
    let result = unsafe { original(storage, diplomacy, target) };
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
    ENABLED.store(enabled, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A copy that writers lapped is refused.
    #[test]
    fn a_lapped_log_refuses_its_copy() {
        let mut output = [Mark::default(); 2];
        assert!(copy_marks(5, &mut output, || 5 + LOG as u64));
        assert!(!copy_marks(5, &mut output, || 6 + LOG as u64));
    }

    /// A writer that stalls while writers append a whole log after its index
    /// may store its mark over a newer one. Drains meanwhile see it active and
    /// hand nothing over; it ends the observation before it leaves.
    #[test]
    fn a_writer_lapped_while_stalled_ends_the_observation() {
        let _lock = test_lock();
        observe_for_test(true);
        let mut consumed = cursor();
        let (mut output, mut required) = ([Mark::default(); 4], 0);
        // The stalled writer takes its index and pauses before storing.
        let stalled = Mutation::enter(0);
        let index = CURSOR.fetch_add(1, Ordering::SeqCst);
        // A whole log of marks lands after it, the last one in its slot.
        for offset in 0..LOG {
            let writer = Mutation::enter(0);
            let at = CURSOR.fetch_add(1, Ordering::SeqCst);
            assert!(!publish(at, 0x1000 + offset, 0x2000));
            drop(writer);
        }
        assert_eq!(
            LOG_SLOTS[(index % LOG as u64) as usize].load(Ordering::SeqCst),
            0x1000 + LOG - 1
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
        let mut consumed = cursor();
        let mut output = vec![Mark::default(); LOG];
        let mut required = 0;
        let stalled = Mutation::enter(0);
        let index = CURSOR.fetch_add(1, Ordering::SeqCst);
        // A drain while it is stalled hands nothing over and moves past it.
        assert_eq!(
            drain(&mut consumed, &mut output, &mut required).map(|(_, marks)| marks),
            Some(None)
        );
        assert_eq!(consumed, index + 1);
        for offset in 0..LOG {
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
        assert_eq!(required, LOG);
        let (epoch, marks) = drained.unwrap();
        assert_eq!(epoch, before, "the epoch read before the writer left");
        assert_eq!(marks, None, "no marks handed over under that epoch");
        // The next drain is under the new epoch, which no baseline recorded
        // before it can continue.
        let (next, _) = drain(&mut consumed, &mut output, &mut required).unwrap();
        assert!(next > before);
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
        assert!(hooks.executable_ranges().is_empty());
        assert_eq!(hooks.set_enabled(true), Ok(true));
        let (mut consumed, mut output, mut required) = (cursor(), [Mark::default(); 4], 0);
        assert_eq!(drain(&mut consumed, &mut output, &mut required), None);
        assert_eq!(
            status()["unhooked"],
            "opinion recompute_opinion sentinel differs"
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
        // A verify mismatch poisons: nothing is observed any more.
        verified(Some(false));
        assert_eq!(drain(&mut consumed, &mut output, &mut required), None);
        observe_for_test(false);
        assert_eq!(drain(&mut consumed, &mut output, &mut required), None);
    }
}
