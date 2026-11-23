use crate::{
    dirty_set::{Dirty, DirtySet},
    hook::Hook,
};
use engine_mutations::{railway_mutations::MUTATIONS, railway_mutations::Receiver};
use engine_native_memory::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static READS: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];
static SLOTS: AtomicUsize = AtomicUsize::new(0);

pub const COUNTDOWN: usize = 0;

pub struct Watches {
    set: DirtySet,
    calls: [AtomicU64; SITES],
    unnamed: AtomicU64,
    buildings: [AtomicU64; 2],
    hooks: AtomicBool,
    forwarder: AtomicBool,
}

impl Default for Watches {
    fn default() -> Self {
        Self::new()
    }
}

impl Watches {
    pub const fn new() -> Self {
        Self {
            set: DirtySet::new(),
            calls: [const { AtomicU64::new(0) }; SITES],
            unnamed: AtomicU64::new(0),
            buildings: [const { AtomicU64::new(0) }; 2],
            hooks: AtomicBool::new(false),
            forwarder: AtomicBool::new(false),
        }
    }

    pub fn notify(&self, site: usize, target: Target) -> Dirty<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        match target {
            Target::Province(Some(id)) => self.set.notify(id),
            Target::Province(None) => {
                self.unnamed.fetch_add(1, Ordering::Relaxed);
                self.set.notify_every()
            }
            Target::Countdown => self.set.notify(COUNTDOWN),
            Target::Every => self.set.notify_every(),
        }
    }

    fn building(&self, flags: u32, location: u32) -> Option<Dirty<'_>> {
        if flags & engine_native::cprovince::buildings::PROVINCE_FLAG == 0 {
            return None;
        }
        self.buildings[0].fetch_add(1, Ordering::Relaxed);
        Some(match location {
            0 => {
                self.unnamed.fetch_add(1, Ordering::Relaxed);
                self.set.notify_every()
            }
            id => self.set.notify(id as usize),
        })
    }

    fn building_unnamed(&self) -> Dirty<'_> {
        self.buildings[1].fetch_add(1, Ordering::Relaxed);
        self.set.notify_every()
    }

    fn apply(&self) {
        self.set.set_enabled(
            self.hooks.load(Ordering::SeqCst) && self.forwarder.load(Ordering::SeqCst),
        );
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(ids: usize) -> Self {
        let watches = Self::new();
        watches.set.offer(ids);
        watches.set.boundary(false);
        watches.hooks.store(true, Ordering::SeqCst);
        watches.forwarder.store(true, Ordering::SeqCst);
        watches.apply();
        watches
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.set.boundary(invalidate);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_forwarder(&self, enabled: bool) {
        self.forwarder.store(enabled, Ordering::SeqCst);
        self.apply();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.set.poisoned()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn mark_building(&self, flags: u32, location: u32) -> Option<Dirty<'_>> {
        self.building(flags, location)
    }
}

pub trait Changes: Sync {
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64>;
}

impl Changes for Watches {
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        self.set.take(visit)
    }
}

pub struct Observer;
impl Changes for Observer {
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        WATCHES.take(visit)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Province(Option<usize>),
    Countdown,
    Every,
}

fn target(receiver: Receiver, first: usize, second: usize) -> Target {
    match receiver {
        Receiver::Province => Target::Province(
            u32::try_from(second as u32 as i32)
                .ok()
                .map(|id| id as usize),
        ),
        Receiver::Info => {
            let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
            Target::Province(
                engine_mutations::railway_mutations::province(&memory, first).map(|id| id as usize),
            )
        }
        Receiver::CooldownList => Target::Countdown,
        Receiver::Every => Target::Every,
    }
}

pub fn active() -> usize {
    WATCHES.set.active()
}

pub fn boundary(invalidate: bool) {
    WATCHES.set.boundary(invalidate);
}

pub fn poison() {
    WATCHES.set.poison();
}

pub fn forwarder(enabled: bool) {
    WATCHES.forwarder.store(enabled, Ordering::SeqCst);
    WATCHES.apply();
}

pub fn building_changed(flags: u32, location: u32) -> Option<Dirty<'static>> {
    WATCHES.building(flags, location)
}

pub fn building_unnamed() -> Dirty<'static> {
    WATCHES.building_unnamed()
}

pub fn presize() {
    let slots = SLOTS.load(Ordering::Relaxed);
    if slots != 0 {
        WATCHES.set.offer(slots);
    }
}

pub fn collect() {
    WATCHES.set.collect();
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub reused_edges: u64,
    pub read_edges: u64,
    pub carried_batches: u64,
    pub read_batches: u64,
    pub marked: u64,
    pub countdown: bool,
    pub full: bool,
}

pub fn record(counts: &Counts, slots: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.reused_edges,
        counts.read_edges,
        counts.carried_batches,
        counts.read_batches,
        counts.marked,
        u64::from(counts.countdown),
        u64::from(counts.full),
        u64::from(counts.marked == 0 && !counts.countdown && counts.carried_batches != 0),
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
    /// The runtime's build gate admitted this family before construction.
    /// A differing sentinel leaves the family uninstalled; the rail tables
    /// then read every info afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "rail_changes")
            && MUTATIONS
                .iter()
                .all(|(site, _)| crate::located(base, site.rva).is_ok())
        {
            let callbacks: [usize; SITES] = [
                set_province_on_cooldown as *const () as usize,
                update_cooldowns as *const () as usize,
                set_railway as *const () as usize,
                construct_info as *const () as usize,
                destroy_info as *const () as usize,
                read_info as *const () as usize,
                construct_manager as *const () as usize,
                destroy_manager as *const () as usize,
                read_manager as *const () as usize,
            ];
            for (index, (site, _)) in MUTATIONS.iter().enumerate() {
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
            WATCHES.hooks.store(false, Ordering::SeqCst);
            WATCHES.apply();
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        WATCHES
            .hooks
            .store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        WATCHES.apply();
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: like each Hook's relay, the bitmap is dropped only once the
        // hooks were never enabled, or were removed and drained after the
        // last capture; the building family's forwarders are restored with
        // the other families before any hook is dropped.
        unsafe { WATCHES.set.release() };
    }
}

macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::RailChanges);
            let _mark = WATCHES.notify($site, target(MUTATIONS[$site].1, first, second));
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(first, second, third, fourth) })
        }
    };
}
callback!(set_province_on_cooldown, 0);
callback!(update_cooldowns, 1);
callback!(set_railway, 2);
callback!(construct_info, 3);
callback!(destroy_info, 4);
callback!(read_info, 5);
callback!(construct_manager, 6);
callback!(destroy_manager, 7);
callback!(read_manager, 8);

#[cfg(test)]
mod tests {
    use super::*;

    fn take(watches: &Watches) -> (Option<u64>, Vec<usize>) {
        let mut ids = Vec::new();
        let epoch = watches.take(&mut |id| ids.push(id));
        (epoch, ids)
    }

    fn site(name: &str) -> usize {
        MUTATIONS
            .iter()
            .position(|(site, _)| site.name == name)
            .unwrap()
    }

    #[test]
    fn receivers_name_their_province() {
        assert_eq!(
            target(Receiver::Province, 0, 0xdead_0000_0000_0207),
            Target::Province(Some(0x207))
        );
        assert_eq!(
            target(Receiver::Province, 0, u32::MAX as usize),
            Target::Province(None)
        );
        let mut info = vec![0_u8; 0x100];
        let mut definition = vec![0_u8; 0x100];
        let id = engine_native::cprovince::definition::ID;
        definition[id..id + 4].copy_from_slice(&517_u32.to_le_bytes());
        let at = engine_railways::cprovince_railway_info::PROVINCE_RECORD;
        info[at..at + 8].copy_from_slice(&(definition.as_ptr() as u64).to_le_bytes());
        assert_eq!(
            target(Receiver::Info, info.as_ptr() as usize, 7),
            Target::Province(Some(517))
        );
        info[at..at + 8].copy_from_slice(&0_u64.to_le_bytes());
        assert_eq!(
            target(Receiver::Info, info.as_ptr() as usize, 7),
            Target::Province(None)
        );
        assert_eq!(target(Receiver::CooldownList, 0, 0), Target::Countdown);
        assert_eq!(target(Receiver::Every, 0, 0), Target::Every);
        drop(definition);
    }

    #[test]
    fn each_site_marks_its_province_or_moves_the_epoch() {
        let watches = Watches::enabled(1_024);
        let (Some(epoch), _) = take(&watches) else {
            unreachable!("observing")
        };
        drop(watches.notify(site("set_province_on_cooldown"), Target::Province(Some(70))));
        drop(watches.notify(site("set_railway"), Target::Province(Some(3))));
        drop(watches.notify(site("update_cooldowns"), Target::Countdown));
        assert_eq!(take(&watches), (Some(epoch), vec![COUNTDOWN, 3, 70]));
        drop(watches.notify(site("set_railway"), Target::Province(None)));
        assert_ne!(take(&watches).0, Some(epoch));
        assert_eq!(watches.unnamed.load(Ordering::Relaxed), 1);
        let (Some(epoch), _) = take(&watches) else {
            unreachable!("observing")
        };
        for name in [
            "read_info",
            "construct_manager",
            "destroy_manager",
            "read_manager",
        ] {
            drop(watches.notify(site(name), Target::Every));
            assert_ne!(take(&watches).0, Some(epoch), "{name}");
        }
        assert_eq!(
            watches.calls[site("set_railway")].load(Ordering::Relaxed),
            2
        );
    }

    #[test]
    fn forwarded_building_writes_mark_province_statuses() {
        use engine_native::cprovince::buildings::{PROVINCE_FLAG, STATE_FLAG};
        let watches = Watches::enabled(1_024);
        let (Some(epoch), _) = take(&watches) else {
            unreachable!("observing")
        };
        drop(watches.mark_building(PROVINCE_FLAG, 300));
        assert!(watches.mark_building(STATE_FLAG, 12).is_none());
        assert_eq!(take(&watches), (Some(epoch), vec![300]));
        drop(watches.mark_building(PROVINCE_FLAG, 0));
        assert_ne!(take(&watches).0, Some(epoch));
        let (Some(epoch), _) = take(&watches) else {
            unreachable!("observing")
        };
        drop(watches.building_unnamed());
        assert_ne!(take(&watches).0, Some(epoch));
    }

    #[test]
    fn the_domain_observes_only_with_the_forwarder() {
        let watches = Watches::enabled(64);
        let (Some(epoch), _) = take(&watches) else {
            unreachable!("observing")
        };
        watches.set_forwarder(false);
        assert_eq!(take(&watches).0, None);
        watches.set_forwarder(true);
        assert_ne!(take(&watches).0, Some(epoch));
        watches.set.poison();
        assert!(watches.poisoned());
        assert_eq!(take(&watches).0, None);
    }
}
