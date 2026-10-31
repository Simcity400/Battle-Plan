//! Allocation-free notifications from the native writers of every province's combat vector,
//! which NavalProvinceCombats records ([`engine_mutations::province_combat_mutations`]). A
//! detour only marks the province its first argument names in the family's [`DirtySet`] (by its
//! guarded id `cprovince::ID`), counts its site and calls the original; it never changes game
//! behaviour. Both writers run on the game thread. A receiver whose id does not read marks no
//! id and moves the set's epoch, so the consumer reads every province.
//!
//! The NavalProvinceCombats capture is the set's one consumer: it reads the provinces its held
//! walk found with combats and those its take returns, and every other province, empty in the
//! held walk and unwritten since, has no row.
use crate::{
    dirty_set::{Dirty, DirtySet},
    hook::Hook,
};
use engine_mutations::province_combat_mutations::MUTATIONS;
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static DOMAIN: Domain = Domain::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Reused and read provinces, quiet hours (no marked province) and full reads.
static READS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
/// Slots of the province array the latest complete walk visited, for off-thread sizing.
static SLOTS: AtomicUsize = AtomicUsize::new(0);

/// One observation domain: the marked provinces and each site's calls.
pub struct Domain {
    provinces: DirtySet,
    calls: [AtomicU64; SITES],
    /// Receivers whose id did not read.
    unnamed: AtomicU64,
}

impl Domain {
    pub const fn new() -> Self {
        Self {
            provinces: DirtySet::new(),
            calls: [const { AtomicU64::new(0) }; SITES],
            unnamed: AtomicU64::new(0),
        }
    }

    /// Hook path, allocation-free: the writer of `site` ([`MUTATIONS`] index) is about to
    /// rewrite the combat vector of province `id`; `None` when its id does not read. Dropping
    /// the guard marks its return.
    pub fn notify(&self, site: usize, id: Option<usize>) -> Dirty<'_> {
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
    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(provinces: usize) -> Self {
        let domain = Self::new();
        domain.provinces.offer(provinces);
        domain.provinces.boundary(false);
        domain.provinces.set_enabled(true);
        domain
    }

    /// The hourly boundary; `invalidate` starts an epoch.
    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.provinces.boundary(invalidate);
    }
}

impl Default for Domain {
    fn default() -> Self {
        Self::new()
    }
}

/// What a capture sees of the notifications.
pub trait Changes: Sync {
    /// The take of the marked province ids ([`DirtySet::take`]).
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64>;
}

impl Changes for Domain {
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        self.provinces.take(visit)
    }
}

/// The hooks' domain.
pub struct Observer;
impl Changes for Observer {
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        DOMAIN.take(visit)
    }
}

/// Whether NavalProvinceCombats reads only the provinces that may hold combats.
pub const CARRY: bool = cfg!(feature = "province_combat_changes");

/// The guarded id of the province at `province`; zero, which no province has, names none.
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

/// Game thread, in the hourly callback before any capture. `invalidate` after a load, gap or
/// recovery: nothing observed before it certifies anything.
pub fn boundary(invalidate: bool) {
    DOMAIN.provinces.boundary(invalidate);
}

/// Off the game thread: size the set for the slots the latest complete walk visited. Nothing is
/// offered before a walk: a later larger bitmap waits for an invalidating hour, while the first
/// installs at once.
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
pub struct Counts {
    /// Provinces the held walk vouched for without a read.
    pub reused: u64,
    pub read: u64,
    /// No province was marked.
    pub quiet: bool,
    /// Every province read: no held walk certified any.
    pub full: bool,
}

/// A worker publishes one capture's counts and, when it walked the whole province array, its
/// slots for sizing.
pub fn record(counts: &Counts, slots: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        u64::from(counts.quiet),
        u64::from(counts.full),
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
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
        "poisoned": DOMAIN.provinces.poisoned(),
        "reused_provinces": reads[0],
        "read_provinces": reads[1],
        "quiet_hours": reads[2],
        "full_reads": reads[3],
        "events": events,
        "unnamed_receivers": DOMAIN.unnamed.load(Ordering::Relaxed),
        "province_set": DOMAIN.provinces.status(),
        "scope": "cumulative since attach; the province array header is read each hour, and while it and the take's epoch are those of the held complete walk only the provinces that walk found with combats and the marked ones are read (slot, record and every combat entry); every other province, empty in the held walk and unwritten since, is reused with no row; otherwise every province is read"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction. A differing
    /// sentinel leaves the family uninstalled; NavalProvinceCombats then reads every province.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if CARRY
            && MUTATIONS.iter().all(|site| {
                crate::hook::installation::sentinel(memory, base + site.rva, site.sentinel)
            })
        {
            let callbacks: [usize; SITES] = [
                add_combat as *const () as usize,
                remove_combat as *const () as usize,
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
        // One census and suspension for the whole family, shared with the other families in a
        // runtime round (`hook::shared`): a thread at any of its sites, or in a tap, holds
        // every site back until the next attempt.
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
        // SAFETY: like each Hook's relay, the bitmaps are dropped only once the hooks were
        // never enabled, or were removed and drained after the last capture.
        unsafe { DOMAIN.provinces.release() };
    }
}

// Both sites take the province in RCX and the combat in RDX and return nothing a caller
// reads; a detour forwards RCX..R9 and RAX. The mark covers the original in flight.
macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            province: usize,
            combat: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing =
                crate::hook_timing::enter(crate::hook_timing::Family::ProvinceCombatChanges);
            let _mark = DOMAIN.notify($site, province_id(province));
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(province, combat, third, fourth) })
        }
    };
}
// Unique append (province, combat), from combat creation, joining and link rebuild.
callback!(add_combat, 0);
// Swap-remove (province, combat), from combat finalization.
callback!(remove_combat, 1);

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

    /// Each site marks the province it names; an unnamed receiver and an invalidating boundary
    /// move the epoch.
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
    }
}
