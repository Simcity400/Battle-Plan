use crate::{
    dirty_set::{Dirty, DirtySet},
    hook::Hook,
};
use engine_mutations::province_combat_mutations::MUTATIONS;
use engine_native_memory::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static DOMAIN: Domain = Domain::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static READS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static SLOTS: AtomicUsize = AtomicUsize::new(0);

pub struct Domain {
    provinces: DirtySet,
    calls: [AtomicU64; SITES],
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(provinces: usize) -> Self {
        let domain = Self::new();
        domain.provinces.offer(provinces);
        domain.provinces.boundary(false);
        domain.provinces.set_enabled(true);
        domain
    }

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

pub trait Changes: Sync {
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64>;
}

impl Changes for Domain {
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        self.provinces.take(visit)
    }
}

pub struct Observer;
impl Changes for Observer {
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        DOMAIN.take(visit)
    }
}

pub const CARRY: bool = cfg!(feature = "province_combat_changes");

fn province_id(province: usize) -> Option<usize> {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    province
        .checked_add(engine_native::cprovince::ID)
        .and_then(|address| engine_native_memory::read::<u32>(&memory, address))
        .filter(|&id| id != 0)
        .map(|id| id as usize)
}

pub fn active() -> usize {
    DOMAIN.provinces.active()
}

pub fn boundary(invalidate: bool) {
    DOMAIN.provinces.boundary(invalidate);
}

pub fn presize() {
    let slots = SLOTS.load(Ordering::Relaxed);
    if slots != 0 {
        DOMAIN.provinces.offer(slots);
    }
}

pub fn collect() {
    DOMAIN.provinces.collect();
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub quiet: bool,
    pub full: bool,
}

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


pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction. A differing
    /// sentinel leaves the family uninstalled; NavalProvinceCombats then reads every province.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if CARRY
            && MUTATIONS
                .iter()
                .all(|site| crate::located(base, site.rva).is_ok())
        {
            let callbacks: [usize; SITES] = [
                add_combat as *const () as usize,
                remove_combat as *const () as usize,
            ];
            for (index, site) in MUTATIONS.iter().enumerate() {
                let code = engine_sites::hook(base, site.rva)?;
                let hook =
                    unsafe { Hook::new(code.address, callbacks[index], code.sentinel::<16>()?) }
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
callback!(add_combat, 0);
callback!(remove_combat, 1);

#[cfg(test)]
mod tests {
    use super::*;

    fn take(domain: &Domain) -> (Option<u64>, Vec<usize>) {
        let mut ids = Vec::new();
        let epoch = Changes::take(domain, &mut |id| ids.push(id));
        (epoch, ids)
    }

    #[test]
    fn a_receiver_names_its_province_through_the_live_object() {
        let mut province = vec![0_u8; 0x200];
        assert_eq!(province_id(province.as_ptr() as usize), None);
        province[engine_native::cprovince::ID..][..4].copy_from_slice(&517_u32.to_le_bytes());
        assert_eq!(province_id(province.as_ptr() as usize), Some(517));
        assert_eq!(province_id(0), None);
    }

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
