pub mod registration;

use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::air_base_mutations::MUTATIONS;
use engine_native_memory::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static READS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static BASES: AtomicUsize = AtomicUsize::new(0);
static DECODED: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static WORK: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
static CENSUS: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];

pub const MEMBERSHIP: u64 = 1;

#[derive(Clone, Copy, Debug, Default)]
pub struct Work {
    bases: [u64; 3],
    ticks: [u64; 3],
    spliced: [u64; 2],
}

impl Work {
    pub const UNCARRIED: usize = 0;
    pub const CARRIED: usize = 1;
    pub const FRESH: usize = 2;

    pub fn note(&mut self, kind: usize, started: i64) {
        self.bases[kind] += 1;
        self.ticks[kind] += crate::clock::qpc().saturating_sub(started).max(0) as u64;
    }

    pub fn splice(&mut self, spans: usize, bytes: usize) {
        self.spliced[0] += spans as u64;
        self.spliced[1] += bytes as u64;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn bases(&self) -> [u64; 3] {
        self.bases
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn spliced(&self) -> [u64; 2] {
        self.spliced
    }
}

#[cfg_attr(any(test, feature = "test-support"), allow(dead_code))]
pub fn record_work(work: &Work) {
    let values = work.bases.into_iter().chain(work.ticks).chain(work.spliced);
    for (target, value) in WORK.iter().zip(values) {
        target.fetch_add(value, Ordering::Relaxed);
    }
}

pub struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; SITES],
    unnamed: AtomicU64,
    capacities: AtomicU64,
    hooks: AtomicBool,
    forwarders: [AtomicBool; 3],
}

#[derive(Clone, Copy)]
pub enum Forwarder {
    Raid = 0,
    ShipModel = 1,
    Building = 2,
}

impl Default for Watches {
    fn default() -> Self {
        Self::new()
    }
}

impl Watches {
    pub const fn new() -> Self {
        Self {
            registry: WatchRegistry::new(),
            calls: [const { AtomicU64::new(0) }; SITES],
            unnamed: AtomicU64::new(0),
            capacities: AtomicU64::new(0),
            hooks: AtomicBool::new(false),
            forwarders: [const { AtomicBool::new(false) }; 3],
        }
    }

    fn apply(&self) {
        self.registry.set_enabled(
            self.hooks.load(Ordering::SeqCst)
                && self
                    .forwarders
                    .iter()
                    .all(|forwarder| forwarder.load(Ordering::SeqCst)),
        );
    }

    #[inline]
    pub fn notify(&self, handle: Option<u64>) -> Notification<'_> {
        match handle.and_then(|handle| usize::try_from(handle).ok()) {
            Some(handle) => self.registry.notify(handle),
            None => {
                self.unnamed.fetch_add(1, Ordering::Relaxed);
                self.registry.notify_every()
            }
        }
    }

    #[inline]
    pub fn notify_site(&self, site: usize, first: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        match MUTATIONS[site].1.handle() {
            Some(offset) => self.notify(handle(offset, first)),
            None => self.registry.notify(MEMBERSHIP as usize),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(bases: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(bases);
        watches.registry.boundary(false);
        watches.hooks.store(true, Ordering::SeqCst);
        for forwarder in &watches.forwarders {
            forwarder.store(true, Ordering::SeqCst);
        }
        watches.apply();
        watches
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn without_table() -> Self {
        let watches = Self::new();
        watches.hooks.store(true, Ordering::SeqCst);
        for forwarder in &watches.forwarders {
            forwarder.store(true, Ordering::SeqCst);
        }
        watches.apply();
        watches
    }

    pub fn offer(&self, bases: usize) {
        self.registry.offer(bases.saturating_add(1));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }
}

#[inline]
fn handle(offset: usize, first: usize) -> Option<u64> {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    first
        .checked_add(offset)
        .and_then(|address| engine_native_memory::read::<u64>(&memory, address))
}

pub trait Changes: Sync {
    fn sequence(&self, handle: u64) -> Option<(u64, u64)>;

    fn membership(&self) -> Option<(u64, u64)> {
        self.sequence(MEMBERSHIP)
    }
}

impl Changes for Watches {
    fn sequence(&self, handle: u64) -> Option<(u64, u64)> {
        self.registry.sequence(usize::try_from(handle).ok()?)
    }
}

pub struct Observer;
impl Changes for Observer {
    fn sequence(&self, handle: u64) -> Option<(u64, u64)> {
        WATCHES.sequence(handle)
    }
}

pub fn capacity_written(base: usize) -> Notification<'static> {
    WATCHES.capacities.fetch_add(1, Ordering::Relaxed);
    WATCHES.notify(handle(engine_native_parts::cair_base_layout::HANDLE, base))
}

pub fn base_destroyed(base: usize) -> (Notification<'static>, Notification<'static>) {
    (
        WATCHES.notify(handle(engine_native_parts::cair_base_layout::HANDLE, base)),
        WATCHES.registry.notify(MEMBERSHIP as usize),
    )
}

pub fn poison() {
    WATCHES.registry.poison();
}

pub fn forwarder(forwarder: Forwarder, enabled: bool) {
    WATCHES.forwarders[forwarder as usize].store(enabled, Ordering::SeqCst);
    WATCHES.apply();
}

pub fn active() -> usize {
    WATCHES.registry.active()
}

pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
}

pub fn presize() {
    let bases = BASES.load(Ordering::Relaxed);
    if bases != 0 {
        WATCHES.offer(bases);
    }
}

pub fn collect() {
    WATCHES.registry.collect();
}

#[cfg_attr(any(test, feature = "test-support"), allow(dead_code))]
pub fn record(counts: &registration::Counts, bases: Option<usize>) {
    for (target, value) in READS.iter().zip([counts.carried, counts.read]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if let Some(bases) = bases {
        BASES.store(bases, Ordering::Relaxed);
    }
}

#[cfg_attr(any(test, feature = "test-support"), allow(dead_code))]
pub fn record_census(census: &registration::Census) {
    let values = census
        .arrays
        .into_iter()
        .chain(census.rows)
        .chain(census.spans)
        .chain([1]);
    for (target, value) in CENSUS.iter().zip(values) {
        target.fetch_add(value, Ordering::Relaxed);
    }
}

pub fn record_decoded(reused: u64, walked: u64) {
    DECODED[0].fetch_add(reused, Ordering::Relaxed);
    DECODED[1].fetch_add(walked, Ordering::Relaxed);
}


pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A differing sentinel leaves the family uninstalled; AirBaseRegistrations
    /// then reads every base afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "air_base_carry_changes")
            && MUTATIONS
                .iter()
                .all(|(site, _)| crate::located(base, site.rva).is_ok())
        {
            let callbacks: [usize; SITES] = [
                create_country_container as *const () as usize,
                remove_country_access as *const () as usize,
                register_pool as *const () as usize,
                erase_pool as *const () as usize,
                add_wing as *const () as usize,
                remove_wing as *const () as usize,
                destroy_all_wings as *const () as usize,
                split_wing as *const () as usize,
                destroy_pool as *const () as usize,
                rebind_pool_definition as *const () as usize,
                ensure_state_air_base as *const () as usize,
                ensure_rocket_air_base as *const () as usize,
                ensure_gun_air_base as *const () as usize,
                refresh_capacity as *const () as usize,
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
            crate::wing_preferences::publication_enabled(false);
        }
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
        crate::wing_preferences::publication_enabled(enabled && self.hooks.len() == SITES);
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        crate::wing_preferences::publication_enabled(false);
        // SAFETY: like each Hook's relay, the watch tables are dropped only
        // once the hooks were never enabled, or were removed and drained
        // after the last capture.
        unsafe { WATCHES.registry.release() };
    }
}

macro_rules! callback {
    ($name:ident, $site:expr, ($first:ident $(, $arg:ident)*)) => {
        unsafe extern "system-unwind" fn $name($first: usize $(, $arg: usize)*) -> usize {
            let timing =
                crate::hook_timing::enter(crate::hook_timing::Family::AirBaseCarryChanges);
            let _notification = WATCHES.notify_site($site, $first);
            let preference = crate::wing_preferences::base_site($site, &[$first $(, $arg)*]);
            let original: unsafe extern "system-unwind" fn($first: usize $(, $arg: usize)*) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            let result = timing.original(|| unsafe { original($first $(, $arg)*) });
            if let Some(preference) = preference { preference.finish(result); }
            result
        }
    };
}
callback!(create_country_container, 0, (base, tag));
callback!(remove_country_access, 1, (base, tag));
callback!(register_pool, 2, (base, pool, dedupe));
callback!(erase_pool, 3, (base, pool));
callback!(add_wing, 4, (pool, wing, dedupe));
callback!(remove_wing, 5, (pool, wing, destroy));
callback!(destroy_all_wings, 6, (pool));
callback!(split_wing, 7, (pool, wing, size, force));
callback!(destroy_pool, 8, (pool, flags));
callback!(rebind_pool_definition, 9, (pool));
callback!(ensure_state_air_base, 10, (manager, state));
callback!(ensure_rocket_air_base, 11, (manager, state));
callback!(ensure_gun_air_base, 12, (manager, state));
callback!(refresh_capacity, 13, (base));

#[cfg(test)]
mod tests {
    use super::*;
    use engine_mutations::air_base_mutations::Receiver;

    #[test]
    #[ignore = "per-call air-base callback microbenchmark; run only without a live game benchmark"]
    fn hook_cost_air_base_carry_dispatch() {
        const CALLS: usize = 100_000;
        unsafe extern "system-unwind" fn original(pool: usize) -> usize {
            std::hint::black_box(pool)
        }
        let handle = (7_u64 << 32) | 0x43;
        let offset = Receiver::Pool.handle().unwrap();
        let mut pool = vec![0_u8; offset + 8];
        pool[offset..].copy_from_slice(&handle.to_ne_bytes());
        let pointer = pool.as_ptr() as usize;
        WATCHES.registry.offer(1);
        WATCHES.registry.boundary(false);
        WATCHES.registry.set_enabled(true);
        WATCHES.sequence(handle).unwrap();
        ORIGINAL[9].store(original as *const () as usize, Ordering::Release);
        let callback: unsafe extern "system-unwind" fn(usize) -> usize = rebind_pool_definition;
        let nanos = collector_test_support::per_call_ns(CALLS, || {
            for _ in 0..CALLS {
                std::hint::black_box(unsafe { std::hint::black_box(callback)(pointer) });
            }
        });
        let registry = collector_test_support::per_call_ns(CALLS, || {
            for _ in 0..CALLS {
                drop(std::hint::black_box(
                    WATCHES.registry.notify(handle as usize),
                ));
            }
        });
        println!(
            "hook_cost air_base pool rebind: callback={nanos:.1} ns/call, registry={registry:.1} ns/call"
        );
    }

    #[test]
    fn a_site_names_its_base_by_handle_and_an_unread_receiver_names_none() {
        let site = |name: &str| {
            MUTATIONS
                .iter()
                .position(|(site, _)| site.name == name)
                .unwrap()
        };
        let handle: u64 = (7 << 32) | 0x43;
        let mut base = vec![0_u8; 0x20];
        base[8..16].copy_from_slice(&handle.to_le_bytes());
        let mut pool = vec![0_u8; 0x20];
        pool[0x18..0x20].copy_from_slice(&handle.to_le_bytes());
        let watches = Watches::enabled(16);
        for (name, object) in [("register_pool", &base), ("add_wing", &pool)] {
            let certified = watches.sequence(handle).unwrap();
            drop(watches.notify_site(site(name), object.as_ptr() as usize));
            assert_ne!(watches.sequence(handle), Some(certified), "{name}");
        }
        let other = watches.sequence(handle + 1).unwrap();
        drop(watches.notify(None));
        assert_ne!(watches.sequence(handle + 1), Some(other));
        assert_eq!(watches.unnamed.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn creators_name_the_slots_and_capacity_refresh_names_its_base() {
        let site = |name: &str| {
            MUTATIONS
                .iter()
                .position(|(site, _)| site.name == name)
                .unwrap()
        };
        let handle: u64 = (9 << 32) | 0x41;
        let mut base = vec![0_u8; 0x20];
        base[8..16].copy_from_slice(&handle.to_le_bytes());
        let manager = vec![0_u8; 0x20];
        let watches = Watches::enabled(16);
        for name in [
            "ensure_state_air_base",
            "ensure_rocket_air_base",
            "ensure_gun_air_base",
        ] {
            assert_eq!(MUTATIONS[site(name)].1, Receiver::Manager);
            let (slots, base_certificate) = (
                watches.membership().unwrap(),
                watches.sequence(handle).unwrap(),
            );
            drop(watches.notify_site(site(name), manager.as_ptr() as usize));
            assert_ne!(watches.membership(), Some(slots), "{name}");
            assert_eq!(watches.sequence(handle), Some(base_certificate), "{name}");
        }
        let (slots, base_certificate) = (
            watches.membership().unwrap(),
            watches.sequence(handle).unwrap(),
        );
        drop(watches.notify_site(site("refresh_capacity"), base.as_ptr() as usize));
        assert_ne!(watches.sequence(handle), Some(base_certificate));
        assert_eq!(watches.membership(), Some(slots));
        assert_eq!(watches.unnamed.load(Ordering::Relaxed), 0);
    }
}
