//! Bounded, allocation-free portrait mutation notifications. Only authenticated
//! reader addresses are registered; notifications never infer an object owner.
use crate::hook::Hook;
use engine_characters::ccharacter_portraits::MUTATIONS;
use engine_native::{Memory, portrait_cache::Changes, portrait_cache::Counts};
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const WATCHES: usize = 65536;
static REVISIONS: Revisions<WATCHES> = Revisions::new();
static EPOCH: AtomicU64 = AtomicU64::new(1);
/// Moved by every writer: guards a walk's fresh reads of unwatched holders.
static REVISION: AtomicU64 = AtomicU64::new(0);
/// Moved only by writers of a watched holder. The holder constructor and
/// two destructors also run every hour for holders no
/// Capture watches  (division construction,
/// ship construction, leader UI items such as
/// leader UI item construction). A quiet certificate covers only
/// holders watched when it was taken, and a watch is never removed during an
/// attachment, so no other writer can change what it certifies. A writer
/// that looked its holder up before the capture registered it is still
/// active when the capture checks `ACTIVE`, which then certifies nothing.
static WATCHED_REVISION: AtomicU64 = AtomicU64::new(0);
static PLANNED_PARTS: AtomicU64 = AtomicU64::new(0);
static QUIET_PARTS: AtomicU64 = AtomicU64::new(0);
static WORK: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static ENABLED: AtomicBool = AtomicBool::new(false);
static POISONED: AtomicBool = AtomicBool::new(false);
pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: [AtomicUsize; 6] = [const { AtomicUsize::new(0) }; 6];
static EVENTS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
static COUNTS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

struct Slot {
    address: AtomicUsize,
    sequence: AtomicU64,
}

/// Slots are never removed or reused during an attachment. Bounded probing
/// can refuse a new watch; that address then uses ordinary fresh reads. A
/// collision cannot dirty another address or certify an unobserved writer.
struct Revisions<const N: usize> {
    slots: [Slot; N],
}
impl<const N: usize> Revisions<N> {
    const fn new() -> Self {
        assert!(N.is_power_of_two());
        Self {
            slots: [const {
                Slot {
                    address: AtomicUsize::new(0),
                    sequence: AtomicU64::new(0),
                }
            }; N],
        }
    }

    fn sequence(&self, array: usize, register: bool) -> Option<&AtomicU64> {
        if array == 0 {
            return None;
        }
        let first = ((array >> 3).wrapping_mul(0x9e3779b97f4a7c15) >> 32) & (N - 1);
        for probe in 0..N.min(16) {
            let slot = &self.slots[(first + probe) & (N - 1)];
            let mut address = slot.address.load(Ordering::SeqCst);
            if address == 0 && register {
                address = match slot.address.compare_exchange(
                    0,
                    array,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => array,
                    Err(address) => address,
                };
            }
            if address == array {
                return Some(&slot.sequence);
            }
            if address == 0 {
                return None;
            }
        }
        None
    }
}

fn increment(value: &AtomicU64) {
    if value.fetch_add(1, Ordering::SeqCst) == u64::MAX {
        POISONED.store(true, Ordering::SeqCst);
    }
}

pub fn invalidate() {
    increment(&EPOCH);
}

struct Mutation(Option<&'static AtomicU64>);
impl Mutation {
    fn enter(array: usize, site: usize) -> Self {
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        let sequence = REVISIONS.sequence(array, false);
        note(sequence);
        EVENTS[site].fetch_add(1, Ordering::Relaxed);
        Self(sequence)
    }
}
impl Drop for Mutation {
    fn drop(&mut self) {
        note(self.0);
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One writer boundary of a holder: `sequence` is its watch, `None` for a
/// holder no capture watches.
fn note(sequence: Option<&AtomicU64>) {
    increment(&REVISION);
    if let Some(sequence) = sequence {
        increment(&WATCHED_REVISION);
        increment(sequence);
    }
}

pub struct Observer;
impl Observer {
    fn domain(revision: &AtomicU64) -> Option<(u64, u64)> {
        if !ENABLED.load(Ordering::SeqCst) || POISONED.load(Ordering::SeqCst) {
            return None;
        }
        let epoch = EPOCH.load(Ordering::SeqCst);
        let revision = revision.load(Ordering::SeqCst);
        (ACTIVE.load(Ordering::SeqCst) == 0).then_some((epoch, revision))
    }
}

pub fn record_planned_part() {
    PLANNED_PARTS.fetch_add(1, Ordering::Relaxed);
}

/// A share carried whole from a quiet domain and an unchanged census,
/// without its prepared program's reads.
pub fn record_quiet_part() {
    QUIET_PARTS.fetch_add(1, Ordering::Relaxed);
}

pub fn record_work(planned: bool, ticks: u64) {
    let slot = if planned { 0 } else { 2 };
    WORK[slot].fetch_add(1, Ordering::Relaxed);
    WORK[slot + 1].fetch_add(ticks, Ordering::Relaxed);
}
impl Changes for Observer {
    fn quiet_revision(&self) -> Option<(u64, u64)> {
        Observer::domain(&WATCHED_REVISION)
    }
    fn any_revision(&self) -> Option<(u64, u64)> {
        Observer::domain(&REVISION)
    }
    fn revision(&self, array: usize) -> Option<(u64, u64)> {
        if !ENABLED.load(Ordering::SeqCst) || POISONED.load(Ordering::SeqCst) {
            return None;
        }
        let epoch = EPOCH.load(Ordering::SeqCst);
        let sequence = REVISIONS.sequence(array, true)?.load(Ordering::SeqCst);
        if ACTIVE.load(Ordering::SeqCst) != 0 {
            return None;
        }
        Some((epoch, sequence))
    }
}

pub fn record(counts: Counts) {
    for (target, value) in COUNTS.iter().zip([
        counts.reused_characters,
        counts.reused_entries,
        counts.read_characters,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
}

pub fn status() -> impl serde::Serialize {
    let counts = COUNTS.each_ref().map(|v| v.load(Ordering::Relaxed));
    let event_counts = EVENTS.each_ref().map(|value| value.load(Ordering::Relaxed));
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(event_counts)
            .map(|(site, n)| (site.name, n)),
    );
    crate::status::status!({
        "configured": cfg!(feature = "portrait_changes"),
        "poisoned": POISONED.load(Ordering::SeqCst),
        "reused_characters": counts[0], "reused_entries": counts[1],
        "read_characters": counts[2],
        "planned_parts": PLANNED_PARTS.load(Ordering::Relaxed),
        "quiet_parts": QUIET_PARTS.load(Ordering::Relaxed),
        "work": {
            "planned_calls": WORK[0].load(Ordering::Relaxed),
            "planned_ticks": WORK[1].load(Ordering::Relaxed),
            "walk_calls": WORK[2].load(Ordering::Relaxed),
            "walk_ticks": WORK[3].load(Ordering::Relaxed),
            "scope": "QPC ticks for complete part acquisition, including certification, source reads and row work; not game-thread pause"
        },
        "events": events,
        "scope": "cumulative acquisition observations since attach; a quiet part (quiet_parts, also counted in planned_parts) repeats a census share with no writer of a watched holder since its last complete capture and reads nothing; pooled buffers have independent baselines; bounded watch exhaustion disables reuse for that address"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "portrait_changes") {
            let callbacks = [
                assign as *const () as usize,
                copy as *const () as usize,
                construct as *const () as usize,
                copy_construct as *const () as usize,
                destroy as *const () as usize,
                insert as *const () as usize,
            ];
            for (index, site) in MUTATIONS.iter().enumerate() {
                if !crate::hook::installation::sentinel(memory, base + site.rva, site.sentinel) {
                    return Err("portrait hook sentinel differs");
                }
                let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
                    .map_err(|_| "native hook construction failed")?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
        }
        // One census and suspension for the whole family, shared with the
        // other families in a runtime round (`hook::shared`): a thread at any
        // of its sites, or in a tap, holds every site back until the next
        // attempt.
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        ENABLED.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}

unsafe extern "system" fn assign(array: usize, result: usize, key: usize, value: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::PortraitChanges);
    let _mutation = Mutation::enter(array, 0);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(array, result, key, value) })
}
unsafe extern "system" fn copy(holder: usize, source: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::PortraitChanges);
    let _mutation = Mutation::enter(holder.wrapping_add(8), 1);
    let original: unsafe extern "system" fn(usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[1].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(holder, source) })
}
unsafe extern "system" fn construct(holder: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::PortraitChanges);
    let _mutation = Mutation::enter(holder.wrapping_add(8), 2);
    let original: unsafe extern "system" fn(usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[2].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(holder) })
}
unsafe extern "system" fn copy_construct(holder: usize, source: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::PortraitChanges);
    let _mutation = Mutation::enter(holder.wrapping_add(8), 3);
    let original: unsafe extern "system" fn(usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[3].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(holder, source) })
}
unsafe extern "system" fn destroy(holder: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::PortraitChanges);
    let _mutation = Mutation::enter(holder.wrapping_add(8), 4);
    let original: unsafe extern "system" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL[4].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(holder) })
}
unsafe extern "system" fn insert(array: usize, at: i32, source: usize, count: i32) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::PortraitChanges);
    let _mutation = Mutation::enter(array, 5);
    let original: unsafe extern "system" fn(usize, i32, usize, i32) =
        unsafe { std::mem::transmute(ORIGINAL[5].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(array, at, source, count) })
}

#[cfg(test)]
mod revision_tests {
    use super::*;

    /// Army, ship and UI holders, which no capture watches, move only the
    /// revision a walk guards its fresh reads with; a watched holder's writer
    /// moves the quiet revision and its own sequence too.
    #[test]
    fn unwatched_holder_writers_leave_the_quiet_revision_still() {
        let (watched, unwatched) = (0x7ff0_1234_5680, 0x7ff0_6543_2180);
        let sequence = REVISIONS.sequence(watched, true).unwrap();
        let revisions =
            || [&REVISION, &WATCHED_REVISION, sequence].map(|value| value.load(Ordering::SeqCst));
        let [any, quiet, own] = revisions();
        note(REVISIONS.sequence(unwatched, false));
        assert_eq!(revisions(), [any + 1, quiet, own]);
        note(REVISIONS.sequence(watched, false));
        assert_eq!(revisions(), [any + 2, quiet + 1, own + 1]);
    }

    #[test]
    fn exact_watches_survive_collisions_and_refuse_overflow_without_aliasing() {
        let revisions = Revisions::<4>::new();
        assert!(revisions.sequence(0, true).is_none());
        for address in [8, 16, 24, 32] {
            assert!(revisions.sequence(address, false).is_none());
            revisions
                .sequence(address, true)
                .unwrap()
                .store(address as u64, Ordering::SeqCst);
        }
        assert!(revisions.sequence(40, true).is_none());
        assert!(revisions.sequence(40, false).is_none());
        for address in [8, 16, 24, 32] {
            assert_eq!(
                revisions
                    .sequence(address, true)
                    .unwrap()
                    .load(Ordering::SeqCst),
                address as u64
            );
        }
        revisions
            .sequence(8, false)
            .unwrap()
            .fetch_add(2, Ordering::SeqCst);
        assert_eq!(
            revisions.sequence(8, true).unwrap().load(Ordering::SeqCst),
            10
        );
        assert_eq!(
            revisions.sequence(16, true).unwrap().load(Ordering::SeqCst),
            16
        );
    }
}
