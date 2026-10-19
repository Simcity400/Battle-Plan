//! Allocation-free notifications from the native writers of every province's ordered occupant
//! lists, which ProvinceUnitOrder records ([`engine_mutations::province_unit_mutations`]). A detour only
//! marks the province its first argument names in the family's [`DirtySet`] (by its guarded id
//! `cprovince::ID`), counts its site and calls the original; it never changes game behaviour. Both
//! writers run on the game thread. A receiver whose id does not read marks no id and moves the
//! set's epoch, so the consumer reads every province.
//!
//! The ProvinceUnitOrder capture ([`crate::extended::province_unit_order`])
//! is the set's one consumer: it reads only the provinces its take returns,
//! and the hour's sampled slice, and carries every other province's lists
//! from its held complete walk.
use crate::{
    dirty_set::{Dirty, DirtySet},
    hook::Hook,
};
use engine_mutations::province_unit_mutations::MUTATIONS;
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static DOMAIN: Domain = Domain::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Reused, read, verified and mismatched provinces, quiet hours (carried
/// with no marked province) and full reads.
static READS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
/// The family's sampled comparisons.
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "carried ProvinceUnitOrder rows differ from a fresh read",
]);
/// Slots of the province array the latest complete walk visited, for
/// off-thread sizing.
static SLOTS: AtomicUsize = AtomicUsize::new(0);

/// One observation domain: the marked provinces and each site's calls.
pub(crate) struct Domain {
    provinces: DirtySet,
    calls: [AtomicU64; SITES],
    /// Receivers whose id did not read.
    unnamed: AtomicU64,
}

impl Domain {
    pub(crate) const fn new() -> Self {
        Self {
            provinces: DirtySet::new(),
            calls: [const { AtomicU64::new(0) }; SITES],
            unnamed: AtomicU64::new(0),
        }
    }

    /// Hook path, allocation-free: the writer of `site` ([`MUTATIONS`]
    /// index) is about to rewrite the lists of province `id`; `None` when
    /// its id does not read. Dropping the guard marks its return.
    pub(crate) fn notify(&self, site: usize, id: Option<usize>) -> Dirty<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        match id {
            Some(id) => self.provinces.notify(id),
            None => {
                self.unnamed.fetch_add(1, Ordering::Relaxed);
                self.provinces.notify_every()
            }
        }
    }

    /// An enabled domain with a set for `provinces` ids installed.
    #[cfg(test)]
    pub(crate) fn enabled(provinces: usize) -> Self {
        let domain = Self::new();
        domain.provinces.offer(provinces);
        domain.provinces.boundary(false);
        domain.provinces.set_enabled(true);
        domain
    }

    /// The hourly boundary; `invalidate` starts an epoch.
    #[cfg(test)]
    pub(crate) fn boundary(&self, invalidate: bool) {
        self.provinces.boundary(invalidate);
    }

    #[cfg(test)]
    pub(crate) fn poisoned(&self) -> bool {
        self.provinces.poisoned()
    }
}

/// What a capture sees of the notifications.
pub(crate) trait Changes: Sync {
    /// The take of the marked province ids ([`DirtySet::take`]).
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64>;
    /// A fresh read disagreed with carried rows: disable reuse for the
    /// attachment.
    fn mismatch(&self);
}

impl Changes for Domain {
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        self.provinces.take(visit)
    }
    fn mismatch(&self) {
        self.provinces.poison();
    }
}

/// The hooks' domain.
pub(crate) struct Observer;
impl Changes for Observer {
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        DOMAIN.take(visit)
    }
    fn mismatch(&self) {
        DOMAIN.mismatch();
    }
}

/// Whether ProvinceUnitOrder carries unmarked provinces.
pub(crate) const CARRY: bool = cfg!(feature = "province_unit_changes");
/// Whether captures read every carryable province afresh and compare,
/// rather than the hour's sampled slice ([`crate::sampled_verify`]).
pub(crate) const VERIFY: bool = cfg!(feature = "province_unit_verify");

/// The guarded id of the province at `province`; zero, which no province
/// has, names none.
fn province_id(province: usize) -> Option<usize> {
    // SAFETY: a guarded read of live memory on the calling thread.
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    province
        .checked_add(engine_native::cprovince::ID)
        .and_then(|address| engine_native::read::<u32>(&memory, address))
        .filter(|&id| id != 0)
        .map(|id| id as usize)
}

/// Notifications running now.
pub fn active() -> usize {
    DOMAIN.provinces.active()
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    DOMAIN.provinces.boundary(invalidate);
}

/// Off the game thread: size the set for the slots the latest complete walk
/// visited. Nothing is offered before a walk: a later larger bitmap waits for
/// an invalidating hour, while the first installs at once.
pub fn presize() {
    let slots = SLOTS.load(Ordering::Relaxed);
    if slots != 0 {
        DOMAIN.provinces.offer(slots);
    }
}

/// Off the game thread: free a replaced bitmap.
pub fn collect() {
    DOMAIN.provinces.collect();
}

/// One capture's provinces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub reused: u64,
    pub read: u64,
    pub verified: u64,
    pub mismatches: u64,
    /// Carried with no marked province.
    pub quiet: bool,
    /// Every province read: no held walk certified any.
    pub full: bool,
}

/// A worker publishes one capture's counts and, when it walked the whole
/// province array, its slots for sizing.
pub(crate) fn record(counts: &Counts, slots: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.verified,
        counts.mismatches,
        u64::from(counts.quiet),
        u64::from(counts.full),
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    SAMPLED.record(counts.verified, counts.mismatches, 0);
    if let Some(slots) = slots {
        SLOTS.store(slots, Ordering::Relaxed);
    }
}

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(&DOMAIN.calls)
            .map(|(site, n)| (site.name, n.load(Ordering::Relaxed))),
    );
    let reads = READS.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": CARRY,
        "verify": VERIFY,
        "poisoned": DOMAIN.provinces.poisoned(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "reused_provinces": reads[0],
        "read_provinces": reads[1],
        "verified_provinces": reads[2],
        "mismatched_provinces": reads[3],
        "quiet_hours": reads[4],
        "full_reads": reads[5],
        "events": events,
        "unnamed_receivers": DOMAIN.unnamed.load(Ordering::Relaxed),
        "province_set": DOMAIN.provinces.status(),
        "scope": "cumulative since attach; the province array header is read each hour, and while it and the take's epoch are those of the held complete walk only the marked provinces and the slice are read (slot, record, lanes and occupants) and every other province's ProvinceUnitOrder lists are carried from the held walk; otherwise every province is read; every hour also reads the carryable provinces of its slice afresh (one in rotation_hours by province id; every one in verify mode), publishes the fresh rows and disables reuse at the first mismatch (sampled_*)"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// A differing sentinel leaves the family uninstalled; ProvinceUnitOrder
    /// then reads every province.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if CARRY
            && MUTATIONS.iter().all(|site| {
                crate::hook::installation::sentinel(memory, base + site.rva, site.sentinel)
            })
        {
            let callbacks: [usize; SITES] = [
                add_unit as *const () as usize,
                remove_unit as *const () as usize,
            ];
            for (index, site) in MUTATIONS.iter().enumerate() {
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
            DOMAIN.provinces.set_enabled(false);
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
        DOMAIN
            .provinces
            .set_enabled(enabled && !self.hooks.is_empty());
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: like each Hook's relay, the bitmaps are dropped only once
        // the hooks were never enabled, or were removed and drained after
        // the last capture.
        unsafe { DOMAIN.provinces.release() };
    }
}

// Both sites take the province in RCX and the unit in RDX and return
// nothing a caller reads; a detour forwards RCX..R9 and RAX. The mark
// covers the original in flight.
macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            province: usize,
            unit: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::ProvinceUnitChanges);
            let _mark = DOMAIN.notify($site, province_id(province));
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(province, unit, third, fourth) })
        }
    };
}
// Append (province, unit), from unit location setter.
callback!(add_unit, 0);
// Remove unit (province, unit).
callback!(remove_unit, 1);

#[cfg(test)]
mod tests {
    use super::*;

    fn take(domain: &Domain) -> (Option<u64>, Vec<usize>) {
        let mut ids = Vec::new();
        let epoch = Changes::take(domain, &mut |id| ids.push(id));
        (epoch, ids)
    }

    /// A province's guarded id names it; a zero or unreadable id names none.
    #[test]
    fn a_receiver_names_its_province_through_the_live_object() {
        let mut province = vec![0_u8; 0x200];
        assert_eq!(province_id(province.as_ptr() as usize), None);
        province[engine_native::cprovince::ID..][..4].copy_from_slice(&517_u32.to_le_bytes());
        assert_eq!(province_id(province.as_ptr() as usize), Some(517));
        assert_eq!(province_id(0), None);
    }

    /// Each site marks the province it names; an unnamed receiver and an
    /// invalidating boundary move the epoch; a mismatch poisons the set.
    #[test]
    fn each_site_marks_its_province() {
        let domain = Domain::enabled(1024);
        let (epoch, _) = take(&domain);
        let epoch = epoch.unwrap();
        drop(domain.notify(0, Some(700)));
        drop(domain.notify(1, Some(12)));
        assert_eq!(take(&domain), (Some(epoch), vec![12, 700]));
        assert_eq!(take(&domain), (Some(epoch), vec![]));
        assert_eq!(
            [0, 1].map(|site| domain.calls[site].load(Ordering::Relaxed)),
            [1, 1]
        );
        drop(domain.notify(1, None));
        let (moved, ids) = take(&domain);
        assert!(moved.is_some_and(|moved| moved != epoch) && ids.is_empty());
        let moved = moved.unwrap();
        domain.boundary(true);
        assert_ne!(take(&domain).0, Some(moved));
        Changes::mismatch(&domain);
        assert!(domain.poisoned());
        assert_eq!(take(&domain).0, None);
    }
}
