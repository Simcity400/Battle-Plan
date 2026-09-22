//! Bounded, allocation-free portrait mutation notifications. Only authenticated
//! reader addresses are registered; notifications never infer an object owner.
use crate::hook::Hook;
use engine::{
    Memory,
    ccharacter_portraits::MUTATIONS,
    portrait_cache::{Changes, Counts},
};
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const WATCHES: usize = 65536;
static REVISIONS: Revisions<WATCHES> = Revisions::new();
static EPOCH: AtomicU64 = AtomicU64::new(1);
static REVISION: AtomicU64 = AtomicU64::new(0);
static PLANNED_PARTS: AtomicU64 = AtomicU64::new(0);
static WORK: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static ENABLED: AtomicBool = AtomicBool::new(false);
static POISONED: AtomicBool = AtomicBool::new(false);
pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: [AtomicUsize; 6] = [const { AtomicUsize::new(0) }; 6];
static EVENTS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
static COUNTS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];

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
        increment(&REVISION);
        if let Some(sequence) = sequence {
            increment(sequence);
        }
        EVENTS[site].fetch_add(1, Ordering::Relaxed);
        Self(sequence)
    }
}
impl Drop for Mutation {
    fn drop(&mut self) {
        if let Some(sequence) = self.0 {
            increment(sequence);
        }
        increment(&REVISION);
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct Observer;
impl Observer {
    /// Whole-domain certificate for a prepared unchanged-result pass. Native
    /// roots, identities, country readability and headers are still read fresh.
    pub fn quiet_revision(&self) -> Option<(u64, u64)> {
        if !ENABLED.load(Ordering::SeqCst) || POISONED.load(Ordering::SeqCst) {
            return None;
        }
        let epoch = EPOCH.load(Ordering::SeqCst);
        let revision = REVISION.load(Ordering::SeqCst);
        (ACTIVE.load(Ordering::SeqCst) == 0).then_some((epoch, revision))
    }
}

pub fn record_planned_part() {
    PLANNED_PARTS.fetch_add(1, Ordering::Relaxed);
}

pub fn record_work(planned: bool, ticks: u64) {
    let slot = if planned { 0 } else { 2 };
    WORK[slot].fetch_add(1, Ordering::Relaxed);
    WORK[slot + 1].fetch_add(ticks, Ordering::Relaxed);
}
impl Changes for Observer {
    fn quiet_revision(&self) -> Option<(u64, u64)> {
        Observer::quiet_revision(self)
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
    fn verify(&self) -> bool {
        cfg!(feature = "portrait_verify")
    }
}

pub fn record(counts: Counts) {
    for (target, value) in COUNTS.iter().zip([
        counts.reused_characters,
        counts.reused_entries,
        counts.read_characters,
        counts.verified_entries,
        counts.mismatches,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    // A verification mismatch permanently disables reuse for this attachment.
    if counts.mismatches != 0 {
        POISONED.store(true, Ordering::SeqCst);
    }
}

pub fn status() -> serde_json::Value {
    let counts = COUNTS.each_ref().map(|v| v.load(Ordering::Relaxed));
    let events: serde_json::Map<String, serde_json::Value> = MUTATIONS
        .iter()
        .zip(&EVENTS)
        .map(|(site, n)| (site.name.to_string(), n.load(Ordering::Relaxed).into()))
        .collect();
    serde_json::json!({
        "configured": cfg!(feature = "portrait_changes"),
        "verify": cfg!(feature = "portrait_verify"),
        "poisoned": POISONED.load(Ordering::SeqCst),
        "reused_characters": counts[0], "reused_entries": counts[1],
        "read_characters": counts[2], "verified_entries": counts[3], "mismatches": counts[4],
        "planned_parts": PLANNED_PARTS.load(Ordering::Relaxed),
        "work": {
            "planned_calls": WORK[0].load(Ordering::Relaxed),
            "planned_ticks": WORK[1].load(Ordering::Relaxed),
            "walk_calls": WORK[2].load(Ordering::Relaxed),
            "walk_ticks": WORK[3].load(Ordering::Relaxed),
            "scope": "QPC ticks for complete part acquisition, including certification, source reads and row work; not game-thread pause"
        },
        "events": events,
        "scope": "cumulative acquisition observations since attach; verify mode reads all descriptors and compares would-be hits; pooled buffers have independent baselines; bounded watch exhaustion disables reuse for that address"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, String> {
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
                if engine::read::<[u8; 16]>(memory, base + site.rva) != Some(site.sentinel) {
                    return Err(format!("portrait {} sentinel differs", site.name));
                }
                let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
        }
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

unsafe extern "system" fn assign(array: usize, result: usize, key: usize, value: usize) -> usize {
    let _mutation = Mutation::enter(array, 0);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    unsafe { original(array, result, key, value) }
}
unsafe extern "system" fn copy(holder: usize, source: usize) -> usize {
    let _mutation = Mutation::enter(holder.wrapping_add(8), 1);
    let original: unsafe extern "system" fn(usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[1].load(Ordering::Acquire)) };
    unsafe { original(holder, source) }
}
unsafe extern "system" fn construct(holder: usize) -> usize {
    let _mutation = Mutation::enter(holder.wrapping_add(8), 2);
    let original: unsafe extern "system" fn(usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[2].load(Ordering::Acquire)) };
    unsafe { original(holder) }
}
unsafe extern "system" fn copy_construct(holder: usize, source: usize) -> usize {
    let _mutation = Mutation::enter(holder.wrapping_add(8), 3);
    let original: unsafe extern "system" fn(usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[3].load(Ordering::Acquire)) };
    unsafe { original(holder, source) }
}
unsafe extern "system" fn destroy(holder: usize) {
    let _mutation = Mutation::enter(holder.wrapping_add(8), 4);
    let original: unsafe extern "system" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL[4].load(Ordering::Acquire)) };
    unsafe { original(holder) }
}
unsafe extern "system" fn insert(array: usize, at: i32, source: usize, count: i32) {
    let _mutation = Mutation::enter(array, 5);
    let original: unsafe extern "system" fn(usize, i32, usize, i32) =
        unsafe { std::mem::transmute(ORIGINAL[5].load(Ordering::Acquire)) };
    unsafe { original(array, at, source, count) }
}

#[cfg(test)]
mod revision_tests {
    use super::*;

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
