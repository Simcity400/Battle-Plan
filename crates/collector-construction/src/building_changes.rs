use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::{building_mutations::MUTATIONS, building_mutations::Receiver};
use engine_native_memory::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static READS: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
static STATUSES: AtomicUsize = AtomicUsize::new(0);

pub struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; SITES],
    unnamed: AtomicU64,
    structure: AtomicU64,
}

impl Default for Watches {
    fn default() -> Self {
        Self::new()
    }
}

impl Watches {
    pub const fn new() -> Self {
        Self {
            registry: WatchRegistry::marking(),
            calls: [const { AtomicU64::new(0) }; SITES],
            unnamed: AtomicU64::new(0),
            structure: AtomicU64::new(0),
        }
    }

    pub fn notify(&self, status: Option<usize>, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        match status {
            Some(status) => self.registry.notify(status),
            None => {
                self.unnamed.fetch_add(1, Ordering::Relaxed);
                self.registry.notify_every()
            }
        }
    }

    pub fn structure(&self, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.structure.fetch_add(1, Ordering::Relaxed);
        self.registry.poison();
        self.registry.notify_every()
    }

    fn notify_site(&self, site: usize, first: usize) -> Notification<'_> {
        match MUTATIONS[site].1 {
            Receiver::Structure => self.structure(site),
            receiver => self.notify(status_of(receiver, first), site),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(statuses: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(statuses);
        watches.registry.boundary(false);
        watches.registry.set_enabled(true);
        watches
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn registered(&self) -> usize {
        self.registry.registered()
    }
}

pub trait Changes: Sync {
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64>;
    fn sequence(&self, status: usize) -> Option<(u64, u64)>;
}

impl Changes for Watches {
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        self.registry.take(visit)
    }
    fn sequence(&self, status: usize) -> Option<(u64, u64)> {
        self.registry.sequence(status)
    }
}

pub struct Observer;
impl Changes for Observer {
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        WATCHES.take(visit)
    }
    fn sequence(&self, status: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(status)
    }
}

fn status_of(receiver: Receiver, first: usize) -> Option<usize> {
    match receiver {
        Receiver::Status => Some(first),
        Receiver::Building => {
            let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
            first
                .checked_add(engine_native_parts::cbuilding::BUILDING_STATUS)
                .and_then(|address| engine_native_memory::read::<u64>(&memory, address))
                .and_then(|status| usize::try_from(status).ok())
                .filter(|&status| status != 0)
        }
        Receiver::Structure => None,
    }
}

#[must_use]
struct Forwarded(
    #[allow(dead_code)] Option<crate::territory_changes::Marks<'static>>,
    #[allow(dead_code)] Option<crate::dirty_set::Dirty<'static>>,
    #[allow(dead_code)] Option<crate::dirty_set::Dirty<'static>>,
);

fn forward(receiver: Receiver, first: usize) -> Forwarded {
    if receiver == Receiver::Structure {
        crate::territory_changes::poison();
        crate::rail_changes::poison();
        crate::raid_changes::poison();
        collector_observers::air_base_carry_changes::poison();
        return Forwarded(None, None, None);
    }
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let location = status_of(receiver, first)
        .and_then(|status| status.checked_add(engine_native::cprovince::buildings::FLAGS))
        .and_then(|address| engine_native_memory::read::<[u8; 8]>(&memory, address));
    match location {
        Some(bytes) => {
            let flags = u32::from_le_bytes(bytes[..4].try_into().unwrap());
            let location = u32::from_le_bytes(bytes[4..].try_into().unwrap());
            Forwarded(
                Some(crate::territory_changes::building_changed(flags, location)),
                crate::rail_changes::building_changed(flags, location),
                crate::raid_changes::building_changed(flags, location),
            )
        }
        None => Forwarded(
            Some(crate::territory_changes::building_unnamed()),
            Some(crate::rail_changes::building_unnamed()),
            Some(crate::raid_changes::building_unnamed()),
        ),
    }
}

pub fn active() -> usize {
    WATCHES.registry.active()
}

pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
}

pub fn presize() {
    let statuses = STATUSES.load(Ordering::Relaxed);
    if statuses != 0 {
        WATCHES.registry.offer(statuses);
    }
}

pub fn collect() {
    WATCHES.registry.collect();
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub reused_rows: u64,
    pub read_rows: u64,
    pub empty: u64,
    pub marked: u64,
    pub settled: bool,
    pub quiet: bool,
}

pub fn record(counts: &Counts, statuses: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.reused_rows,
        counts.read_rows,
        counts.empty,
        u64::from(counts.quiet),
        counts.marked,
        u64::from(counts.settled),
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if let Some(statuses) = statuses {
        STATUSES.store(statuses, Ordering::Relaxed);
    }
}


pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A withheld site leaves the family uninstalled; the building
    /// tables then read every status afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "building_changes")
            && MUTATIONS
                .iter()
                .all(|(site, _)| crate::located(base, site.rva).is_ok())
        {
            let callbacks: [usize; SITES] = [
                set_level as *const () as usize,
                apply_damage_step as *const () as usize,
                advance_repair as *const () as usize,
                destroy_healthy_levels as *const () as usize,
                get_or_create_building as *const () as usize,
                read_building_status as *const () as usize,
                clear_building_status as *const () as usize,
                enter_nudge_mode as *const () as usize,
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
        if hooks.is_empty() {
            crate::territory_changes::poison();
            crate::rail_changes::poison();
            crate::raid_changes::poison();
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            WATCHES.registry.set_enabled(false);
            collector_observers::world_input_changes::fog_forwarder(false);
            collector_observers::nuclear_strikes::forwarder(
                replay_data::rows::NuclearStrikePart::Buildings,
                false,
            );
            crate::rail_changes::forwarder(false);
            crate::raid_changes::forwarder(false);
            collector_observers::air_base_carry_changes::forwarder(
                collector_observers::air_base_carry_changes::Forwarder::Building,
                false,
            );
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        WATCHES
            .registry
            .set_enabled(enabled && !self.hooks.is_empty());
        collector_observers::nuclear_strikes::forwarder(
            replay_data::rows::NuclearStrikePart::Buildings,
            enabled && !self.hooks.is_empty(),
        );
        collector_observers::world_input_changes::fog_forwarder(
            enabled && self.hooks.len() == SITES,
        );
        crate::rail_changes::forwarder(enabled && !self.hooks.is_empty());
        crate::raid_changes::forwarder(enabled && !self.hooks.is_empty());
        collector_observers::air_base_carry_changes::forwarder(
            collector_observers::air_base_carry_changes::Forwarder::Building,
            enabled && self.hooks.len() == SITES,
        );
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        collector_observers::world_input_changes::fog_forwarder(false);
        // SAFETY: like each Hook's relay, the watch tables are dropped only
        // once the hooks were never enabled, or were removed and drained
        // after the last capture.
        unsafe { WATCHES.registry.release() };
    }
}

macro_rules! callback {
    ($name:ident, $site:expr, ($first:ident $(, $arg:ident)*)) => {
        unsafe extern "system" fn $name($first: usize $(, $arg: usize)*) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::BuildingChanges);
            let _notification = WATCHES.notify_site($site, $first);
            let _fog = ($site == 7).then(collector_observers::world_input_changes::fog_nudge_entered);
            let _forwarded = forward(MUTATIONS[$site].1, $first);
            let original: unsafe extern "system" fn($first: usize $(, $arg: usize)*) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original($first $(, $arg)*) })
        }
    };
}
callback!(set_level, 0, (building, level, notify));
callback!(
    apply_damage_step,
    1,
    (building, out, amount, first, cause, second)
);
callback!(advance_repair, 2, (building, out, progress, flag));
unsafe extern "system" fn destroy_healthy_levels(
    building: usize,
    out: usize,
    levels: usize,
    flag: usize,
    source: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::BuildingChanges);
    let _notification = WATCHES.notify_site(3, building);
    let _forwarded = forward(MUTATIONS[3].1, building);
    let _nuclear = collector_observers::nuclear_strikes::building(building, source);
    let original: unsafe extern "system" fn(usize, usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[3].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(building, out, levels, flag, source) })
}
callback!(get_or_create_building, 4, (status, template));
callback!(read_building_status, 5, (status, reader));
callback!(clear_building_status, 6, (status));
callback!(enter_nudge_mode, 7, (idler, a, b, c, d));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_building_writer_names_its_backlinked_status() {
        let mut building = vec![0_u8; engine_native_parts::cbuilding::TEMPLATE + 8];
        let status = 0x7654_3210_usize;
        let at = engine_native_parts::cbuilding::BUILDING_STATUS;
        building[at..at + 8].copy_from_slice(&(status as u64).to_le_bytes());
        let address = building.as_ptr() as usize;
        assert_eq!(status_of(Receiver::Building, address), Some(status));
        assert_eq!(status_of(Receiver::Status, address), Some(address));
        building[at..at + 8].fill(0);
        assert_eq!(
            status_of(Receiver::Building, building.as_ptr() as usize),
            None
        );
        let watches = Watches::enabled(16);
        let certified = Changes::sequence(&watches, status).unwrap();
        drop(watches.notify(None, 0));
        assert_ne!(Changes::sequence(&watches, status), Some(certified));
        assert_eq!(watches.unnamed.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn entering_nudge_mode_poisons_the_domain() {
        let site = MUTATIONS
            .iter()
            .position(|&(_, receiver)| receiver == Receiver::Structure)
            .unwrap();
        let watches = Watches::enabled(16);
        let certified = Changes::sequence(&watches, 0x1000).unwrap();
        let taken = Changes::take(&watches, &mut |_| {}).unwrap();
        let running = watches.notify_site(site, 0x1000);
        assert!(watches.poisoned());
        assert_eq!(Changes::take(&watches, &mut |_| {}), None);
        drop(running);
        watches.boundary(false);
        assert_eq!(Changes::sequence(&watches, 0x1000), None);
        assert_eq!(Changes::take(&watches, &mut |_| {}), None);
        assert_ne!(Some(certified), Changes::sequence(&watches, 0x1000));
        assert_ne!(Some(taken), Changes::take(&watches, &mut |_| {}));
        assert_eq!(watches.calls[site].load(Ordering::Relaxed), 1);
        assert_eq!(watches.structure.load(Ordering::Relaxed), 1);
        assert_eq!(watches.unnamed.load(Ordering::Relaxed), 0);
    }
}
