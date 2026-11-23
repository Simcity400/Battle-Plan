use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::{
    local_subunit_mutations::BONUS_LIST, local_subunit_mutations::CLONE_COUNT,
    local_subunit_mutations::MUTATIONS, local_subunit_mutations::Receiver,
};
use engine_native_memory::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static READS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
static COUNTRIES: AtomicUsize = AtomicUsize::new(0);

pub struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; SITES],
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
        }
    }

    pub fn notify(&self, status: Option<usize>, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        match status {
            Some(status) => self.registry.notify(status),
            None => self.registry.notify_every(),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(objects: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(objects);
        watches.registry.boundary(false);
        watches.registry.set_enabled(true);
        watches
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }
}

pub trait Changes: Sync {
    fn quiet(&self) -> Option<(u64, u64)>;
    fn sequence(&self, status: usize) -> Option<(u64, u64)>;
}

impl Changes for Watches {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }
    fn sequence(&self, status: usize) -> Option<(u64, u64)> {
        self.registry.sequence(status)
    }
}

pub struct Observer;
#[cfg(any(test, feature = "test-support"))]
pub fn unsized_observer_for_tests() -> Observer {
    WATCHES.registry.set_enabled(true);
    Observer
}
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        WATCHES.quiet()
    }
    fn sequence(&self, status: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(status)
    }
}

pub const CARRY: bool = cfg!(feature = "local_subunit_changes");

pub fn active() -> usize {
    WATCHES.registry.active()
}

pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
}

pub fn prepare(countries: usize) {
    if countries != 0 {
        WATCHES.registry.offer(countries);
    }
}

pub fn presize() {
    prepare(COUNTRIES.load(Ordering::Relaxed));
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
    pub quiet: bool,
}

pub fn record(counts: &Counts, countries: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.reused_rows,
        counts.read_rows,
        u64::from(counts.quiet),
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if let Some(countries) = countries {
        COUNTRIES.store(countries, Ordering::Relaxed);
    }
}


pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A differing sentinel leaves the family uninstalled; CountryLocalSubunits
    /// then reads every country afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "local_subunit_changes")
            && MUTATIONS
                .iter()
                .all(|(site, _)| crate::located(base, site.rva).is_ok())
        {
            let callbacks: [usize; SITES] = [
                ensure_local_subunit as *const () as usize,
                rebuild_local_subunit as *const () as usize,
                clear_local_subunits as *const () as usize,
                initialize_default_subunits as *const () as usize,
                find_or_create_subunit_definition as *const () as usize,
                reload_equipment_content as *const () as usize,
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
            WATCHES.registry.set_enabled(false);
            collector_certificates::content_changes::enabled(
                collector_certificates::content_changes::Writer::Equipment,
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
        collector_certificates::content_changes::enabled(
            collector_certificates::content_changes::Writer::Equipment,
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
        // SAFETY: like each Hook's relay, the watch tables are dropped only
        // once the hooks were never enabled, or were removed and drained
        // after the last capture.
        unsafe { WATCHES.registry.release() };
    }
}

fn receiver(receiver: Receiver, first: usize, third: usize) -> Option<usize> {
    match receiver {
        Receiver::Status => Some(first),
        Receiver::BonusList => Some(third.wrapping_sub(BONUS_LIST)),
        Receiver::Unnamed => None,
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
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::LocalSubunitChanges);
            let _notification = WATCHES.notify(receiver(MUTATIONS[$site].1, first, third), $site);
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe {
                if $site == 5 {
                    collector_certificates::content_changes::invoke(
                        collector_certificates::content_changes::Writer::Equipment,
                        [first, second, third, fourth],
                        original,
                    )
                } else {
                    original(first, second, third, fourth)
                }
            })
        }
    };
}
fn clone_count(status: usize) -> i32 {
    unsafe { std::ptr::read_volatile(status.wrapping_add(CLONE_COUNT) as *const i32) }
}

unsafe extern "system" fn ensure_local_subunit(
    status: usize,
    global: usize,
    flag: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::LocalSubunitChanges);
    WATCHES.calls[0].fetch_add(1, Ordering::Relaxed);
    let _running = WATCHES.registry.running();
    let before = clone_count(status);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(status, global, flag, fourth) });
    if clone_count(status) != before {
        drop(WATCHES.registry.notify(status));
    }
    result
}
callback!(rebuild_local_subunit, 1);
callback!(clear_local_subunits, 2);
callback!(initialize_default_subunits, 3);
callback!(find_or_create_subunit_definition, 4);
callback!(reload_equipment_content, 5);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_writer_moves_only_its_statuss_certificate_and_the_quiet_token() {
        let watches = Watches::enabled(16);
        let [first, second] =
            [0x1000, 0x2000].map(|status| Changes::sequence(&watches, status).unwrap());
        let quiet = Changes::quiet(&watches).unwrap();
        let named = receiver(MUTATIONS[1].1, 0x9000, 0x2000 + BONUS_LIST);
        assert_eq!(named, Some(0x2000));
        drop(watches.notify(named, 1));
        assert_eq!(Changes::sequence(&watches, 0x1000), Some(first));
        assert_ne!(Changes::sequence(&watches, 0x2000), Some(second));
        assert_ne!(Changes::quiet(&watches), Some(quiet));
        assert_eq!(watches.calls[1].load(Ordering::Relaxed), 1);
        for site in [0, 2, 3] {
            assert_eq!(receiver(MUTATIONS[site].1, 0x1000, 0), Some(0x1000));
        }
        let running = watches.notify(Some(0x1000), 0);
        assert!(Changes::sequence(&watches, 0x1000).is_none());
        assert!(Changes::quiet(&watches).is_none());
        drop(running);
        let first = Changes::sequence(&watches, 0x1000).unwrap();
        assert_eq!(receiver(MUTATIONS[4].1, 0x1000, 0), None);
        drop(watches.notify(receiver(MUTATIONS[4].1, 0x1000, 0), 4));
        assert_ne!(Changes::sequence(&watches, 0x1000), Some(first));
        assert!(!watches.poisoned());
    }
}
