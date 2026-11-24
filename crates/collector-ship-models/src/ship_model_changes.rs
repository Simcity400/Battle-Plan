use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::{ship_model_mutations::MUTATIONS, ship_model_mutations::RECEIVER_ARGUMENT};
use engine_native_memory::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
const DESTROY_AIR_BASE: usize = 4;
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static READS: [AtomicU64; 9] = [const { AtomicU64::new(0) }; 9];
static RECEIVERS: AtomicUsize = AtomicUsize::new(0);
static VARIANT_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static VARIANT_AVAILABILITY: AtomicU64 = AtomicU64::new(0);
static VARIANT_VALIDATE: AtomicUsize = AtomicUsize::new(0);
static VARIANT_CALLS: AtomicU64 = AtomicU64::new(0);

pub struct Watches {
    registry: WatchRegistry,
    variants: WatchRegistry,
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
            variants: WatchRegistry::new(),
            calls: [const { AtomicU64::new(0) }; SITES],
        }
    }

    pub fn notify(&self, receiver: usize, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.registry.notify(receiver)
    }

    pub fn capacity_refit(&self, ship: usize) -> Notification<'_> {
        self.registry
            .notify(ship.wrapping_add(engine_native::cship::MODULE_PREDICATE))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(receivers: usize) -> Self {
        let watches = Self::new();
        for registry in [&watches.registry, &watches.variants] {
            registry.offer(receivers);
            registry.boundary(false);
            registry.set_enabled(true);
        }
        watches
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
        self.variants.boundary(invalidate);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn notify_variant(&self, variant: usize) -> Notification<'_> {
        self.variants.notify(variant)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn disable_variants(&self) {
        self.variants.set_enabled(false);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }
}

pub type Certificate = [(u64, u64); 3];

pub type Slots = [usize; 3];
pub const NO_SLOTS: Slots = [usize::MAX; 3];

pub trait Changes: Sync {
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)>;
    fn sequence_at(&self, receiver: usize, _held_slot: usize) -> Option<((u64, u64), usize)> {
        self.sequence(receiver)
            .map(|certificate| (certificate, usize::MAX))
    }
    fn quiet(&self) -> Option<(u64, u64)> {
        None
    }
    fn tree_sequence(&self, country: usize) -> Option<(u64, u64)>;

    fn variant_sequence(&self, _variant: usize) -> Option<(u64, u64)> {
        None
    }
    fn variant_quiet(&self) -> Option<(u64, u64)> {
        None
    }
    fn shared_store(&self) -> bool {
        false
    }

    fn certificate(&self, receivers: [usize; 3]) -> Option<Certificate> {
        let mut certificate = [(0, 0); 3];
        for (held, receiver) in certificate.iter_mut().zip(receivers) {
            if receiver != 0 {
                *held = self.sequence(receiver)?;
            }
        }
        Some(certificate)
    }

    fn certificate_at(
        &self,
        receivers: [usize; 3],
        _held_slots: Slots,
    ) -> Option<(Certificate, Slots)> {
        self.certificate(receivers)
            .map(|certificate| (certificate, NO_SLOTS))
    }
}

impl Changes for Watches {
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        self.registry.sequence(receiver)
    }
    fn sequence_at(&self, receiver: usize, held_slot: usize) -> Option<((u64, u64), usize)> {
        self.registry.sequence_at(receiver, held_slot)
    }
    fn quiet(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }
    fn certificate_at(
        &self,
        receivers: [usize; 3],
        held_slots: Slots,
    ) -> Option<(Certificate, Slots)> {
        let mut certificate = [(0, 0); 3];
        let mut slots = NO_SLOTS;
        for (index, receiver) in receivers.into_iter().enumerate() {
            if receiver != 0 {
                (certificate[index], slots[index]) =
                    self.sequence_at(receiver, held_slots[index])?;
            }
        }
        Some((certificate, slots))
    }
    fn tree_sequence(&self, country: usize) -> Option<(u64, u64)> {
        self.registry.sequence(country)
    }
    fn variant_sequence(&self, variant: usize) -> Option<(u64, u64)> {
        self.variants.sequence(variant)
    }
    fn variant_quiet(&self) -> Option<(u64, u64)> {
        self.variants.quiet()
    }
}

pub struct Observer;
impl Changes for Observer {
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(receiver)
    }
    fn sequence_at(&self, receiver: usize, held_slot: usize) -> Option<((u64, u64), usize)> {
        WATCHES.sequence_at(receiver, held_slot)
    }
    fn quiet(&self) -> Option<(u64, u64)> {
        WATCHES.quiet()
    }
    fn certificate_at(
        &self,
        receivers: [usize; 3],
        held_slots: Slots,
    ) -> Option<(Certificate, Slots)> {
        WATCHES.certificate_at(receivers, held_slots)
    }
    fn tree_sequence(&self, country: usize) -> Option<(u64, u64)> {
        crate::naval_tree_changes::sequence(country)
    }
    fn variant_sequence(&self, variant: usize) -> Option<(u64, u64)> {
        variant_observed(|| WATCHES.variants.sequence(variant))
    }
    fn variant_quiet(&self) -> Option<(u64, u64)> {
        variant_observed(|| WATCHES.variants.quiet())
    }
    fn shared_store(&self) -> bool {
        true
    }
}

fn variant_observed(read: impl FnOnce() -> Option<(u64, u64)>) -> Option<(u64, u64)> {
    let stamp = VARIANT_AVAILABILITY.load(Ordering::SeqCst);
    if !variant_capability_is_current(stamp) || VARIANT_ACTIVE.load(Ordering::SeqCst) != 0 {
        return None;
    }
    let result = read();
    (VARIANT_ACTIVE.load(Ordering::SeqCst) == 0 && variant_capability_is_current(stamp))
        .then_some(result)
        .flatten()
}

fn variant_capability_is_current(stamp: u64) -> bool {
    let validator = VARIANT_VALIDATE.load(Ordering::SeqCst);
    if stamp == 0 || validator == 0 || VARIANT_AVAILABILITY.load(Ordering::SeqCst) != stamp {
        return false;
    }
    // SAFETY: only `variant_availability` stores this, from a `fn(u64) -> bool`.
    let validate: fn(u64) -> bool = unsafe { std::mem::transmute(validator) };
    validate(stamp) && VARIANT_AVAILABILITY.load(Ordering::SeqCst) == stamp
}

pub fn variant_availability(stamp: u64, validate: fn(u64) -> bool) {
    VARIANT_VALIDATE.store(validate as *const () as usize, Ordering::SeqCst);
    VARIANT_AVAILABILITY.store(stamp, Ordering::SeqCst);
    WATCHES.variants.set_enabled(validate(stamp));
}

pub fn variant_before(_variant: usize) {
    VARIANT_CALLS.fetch_add(1, Ordering::Relaxed);
    VARIANT_ACTIVE.fetch_add(1, Ordering::SeqCst);
}

pub fn variant_after(variant: usize, unwound: bool) {
    if unwound {
        WATCHES.variants.poison();
    }
    WATCHES.variants.touch(variant);
    VARIANT_ACTIVE.fetch_sub(1, Ordering::SeqCst);
}

pub fn variants_before() {
    VARIANT_CALLS.fetch_add(1, Ordering::Relaxed);
    VARIANT_ACTIVE.fetch_add(1, Ordering::SeqCst);
}

pub fn variants_after(unwound: bool) {
    if unwound {
        WATCHES.variants.poison();
    }
    WATCHES.variants.touch_every();
    VARIANT_ACTIVE.fetch_sub(1, Ordering::SeqCst);
}

pub const CARRY: bool = cfg!(feature = "ship_model_changes");

pub fn active() -> usize {
    WATCHES
        .registry
        .active()
        .saturating_add(VARIANT_ACTIVE.load(Ordering::SeqCst))
}

pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
    WATCHES.variants.boundary(invalidate);
}

pub fn presize() {
    let receivers = RECEIVERS.load(Ordering::Relaxed);
    if receivers != 0 {
        WATCHES.registry.offer(receivers);
        WATCHES.variants.offer(receivers);
    }
}

pub fn collect() {
    WATCHES.registry.collect();
    WATCHES.variants.collect();
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub carried: u64,
    pub read: u64,
    pub uncertified: u64,
    pub task_terms_carried: u64,
    pub rows_carried: u64,
    pub models_carried: u64,
    pub revalidated: u64,
    pub variants_carried: u64,
    pub variants_read: u64,
    pub receivers: usize,
    pub path_samples: [u64; PATHS],
    pub path_ticks: [u64; PATHS],
    pub terms_reads: u64,
    pub terms_ticks: u64,
    pub terms_changed: [u64; REASONS],
}

pub const PATH_SAMPLE: usize = 16;
pub const PATHS: usize = 5;
pub const REASONS: usize = 11;
static PATH_TOTALS: [AtomicU64; 2 * PATHS] = [const { AtomicU64::new(0) }; 2 * PATHS];
static TERM_TOTALS: [AtomicU64; 2 + REASONS] = [const { AtomicU64::new(0) }; 2 + REASONS];

pub fn record(counts: &Counts, complete: bool) {
    for (target, value) in READS.iter().zip([
        counts.carried,
        counts.read,
        counts.uncertified,
        counts.task_terms_carried,
        counts.rows_carried,
        counts.models_carried,
        counts.revalidated,
        counts.variants_carried,
        counts.variants_read,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    for (target, value) in PATH_TOTALS
        .iter()
        .zip(counts.path_samples.into_iter().chain(counts.path_ticks))
    {
        target.fetch_add(value, Ordering::Relaxed);
    }
    for (target, value) in TERM_TOTALS.iter().zip(
        [counts.terms_reads, counts.terms_ticks]
            .into_iter()
            .chain(counts.terms_changed),
    ) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if complete {
        RECEIVERS.fetch_max(counts.receivers, Ordering::Relaxed);
    }
}


fn located(base: usize, key: engine_sites::Rva) -> Result<(), &'static str> {
    engine_sites::hook(base, key)
        .and_then(|code| code.sentinel::<16>())
        .map(drop)
        .inspect_err(|cause| {
            crate::hook::installation::refused(crate::hook::installation::Kind::Sentinel, None, cause)
        })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A site that cannot be located leaves the family uninstalled; ShipModels then
    /// reads every ship afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if CARRY && MUTATIONS.iter().all(|site| located(base, site.rva).is_ok()) {
            let callbacks: [usize; SITES] = [
                recalculate_equipment_stats as *const () as usize,
                refit as *const () as usize,
                rebuild_experience_modifiers as *const () as usize,
                add_carrier_air_base as *const () as usize,
                destroy_air_base as *const () as usize,
                rebuild_medal_modifiers as *const () as usize,
                rebuild_doctrine_tree as *const () as usize,
                rebuild_region_weather_modifiers as *const () as usize,
                bind_fleet_leader as *const () as usize,
                reset_reserve_fleet as *const () as usize,
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
            crate::unit_history_changes::forwarder(false);
            WATCHES.registry.set_enabled(false);
            crate::air_base_carry_changes::forwarder(
                crate::air_base_carry_changes::Forwarder::ShipModel,
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
        crate::unit_history_changes::forwarder(enabled && !self.hooks.is_empty());
        crate::air_base_carry_changes::forwarder(
            crate::air_base_carry_changes::Forwarder::ShipModel,
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
        // after the last capture. The variant table stays: the design
        // family's forwarder notifies it.
        unsafe { WATCHES.registry.release() };
    }
}

/// Notify `site`'s receiver, then run the original with RCX..R9 and return its RAX.
///
/// # Safety
/// Only `site`'s detour calls this, on the native thread running the hooked call, with its four
/// register arguments. None of the writers takes a stack argument.
unsafe fn forward(site: usize, arguments: [usize; 4]) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ShipModelChanges);
    let _notification = WATCHES.notify(arguments[RECEIVER_ARGUMENT[site]], site);
    let _history = (site == 5).then(|| crate::unit_history_changes::notify(arguments[0]));
    let _capacity = (site == 1).then(|| WATCHES.capacity_refit(arguments[0]));
    let _carry = (site == DESTROY_AIR_BASE)
        .then(|| crate::air_base_carry_changes::base_destroyed(arguments[0]));
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[site].load(Ordering::Acquire)) };
    let [first, second, third, fourth] = arguments;
    timing.original(|| unsafe { original(first, second, third, fourth) })
}

macro_rules! detours {
    ($($name:ident = $site:expr;)*) => {$(
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            unsafe { forward($site, [first, second, third, fourth]) }
        }
    )*};
}

detours! {
    recalculate_equipment_stats = 0;
    refit = 1;
    rebuild_experience_modifiers = 2;
    add_carrier_air_base = 3;
    destroy_air_base = 4;
    rebuild_medal_modifiers = 5;
    rebuild_doctrine_tree = 6;
    rebuild_region_weather_modifiers = 7;
    bind_fleet_leader = 8;
    reset_reserve_fleet = 9;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_refit_moves_the_independent_carrier_capacity_certificate() {
        let watches = Watches::enabled(16);
        let ship = 0x1000;
        let receiver = ship + engine_native::cship::MODULE_PREDICATE;
        let capacity = watches.sequence(receiver).unwrap();
        for site in [0, 2, 3, 5] {
            drop(watches.notify(ship, site));
            assert_eq!(watches.sequence(receiver), Some(capacity));
        }
        let own = watches.sequence(ship).unwrap();
        let notification = watches.capacity_refit(ship);
        assert_eq!(watches.sequence(receiver), None);
        drop(notification);
        assert_ne!(watches.sequence(receiver), Some(capacity));
        assert_eq!(watches.sequence(ship), Some(own));
    }

    #[test]
    fn a_ship_certificate_moves_with_each_of_its_receivers() {
        let watches = Watches::enabled(16);
        let (ship, store, base, other) = (0x1000, 0x2000, 0x3000, 0x4000);
        let held = watches.certificate([ship, store, base]).unwrap();
        let unrelated = watches.certificate([other, 0, 0]).unwrap();
        assert_eq!(held[0].0, unrelated[0].0, "one epoch");
        assert_eq!(watches.certificate([ship, store, base]), Some(held));
        for (receiver, site) in [(ship, 0), (store, 5), (base, 4)] {
            let before = watches.certificate([ship, store, base]).unwrap();
            drop(watches.notify(receiver, site));
            assert_ne!(watches.certificate([ship, store, base]), Some(before));
        }
        assert_eq!(watches.certificate([other, 0, 0]), Some(unrelated));
        let running = watches.notify(ship, 1);
        assert!(watches.certificate([ship, 0, 0]).is_none());
        drop(running);
        let held = watches.certificate([ship, 0, 0]).unwrap();
        watches.boundary(true);
        assert_ne!(watches.certificate([ship, 0, 0]), Some(held));
    }

    #[test]
    fn ship_certificate_slots_validate_each_receiver_and_keep_absence() {
        let watches = Watches::enabled(16);
        let receivers = [0x1000, 0x2000, 0x3000];
        let (certificate, slots) = watches.certificate_at(receivers, NO_SLOTS).unwrap();
        assert_eq!(watches.certificate(receivers), Some(certificate));
        assert!(slots.into_iter().all(|slot| slot != usize::MAX));
        assert_eq!(
            watches.certificate_at(receivers, slots),
            Some((certificate, slots))
        );
        assert_eq!(
            watches.certificate_at(receivers, [slots[2], slots[0], slots[1]]),
            Some((certificate, slots))
        );
        let absent = watches.certificate_at([receivers[0], 0, 0], slots).unwrap();
        assert_eq!(absent.0, [certificate[0], (0, 0), (0, 0)]);
        assert_eq!(absent.1, [slots[0], usize::MAX, usize::MAX]);
        assert_eq!(watches.registry.registered(), 3);
        for (index, site) in [0, 5, 4].into_iter().enumerate() {
            let writer = watches.notify(receivers[index], site);
            assert!(watches.certificate_at(receivers, slots).is_none());
            assert!(watches.quiet().is_none());
            drop(writer);
            let next = watches.certificate_at(receivers, slots).unwrap();
            assert_eq!(next.1, slots);
            assert_ne!(next.0[index], certificate[index]);
            assert_eq!(watches.certificate(receivers), Some(next.0));
        }
        let held = watches.certificate_at(receivers, slots).unwrap();
        watches.boundary(true);
        assert_ne!(watches.certificate_at(receivers, slots).unwrap().0, held.0);
    }

    #[test]
    fn changes_without_slot_hints_keep_the_existing_certificate_contract() {
        struct Legacy;
        impl Changes for Legacy {
            fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
                (receiver != 0).then_some((7, receiver as u64))
            }
            fn tree_sequence(&self, _receiver: usize) -> Option<(u64, u64)> {
                None
            }
            fn certificate(&self, _receivers: [usize; 3]) -> Option<Certificate> {
                Some([(9, 1); 3])
            }
        }
        let changes = Legacy;
        assert_eq!(changes.sequence_at(8, 2), Some(((7, 8), usize::MAX)));
        assert!(changes.sequence_at(0, 2).is_none());
        assert_eq!(
            changes.certificate_at([8, 0, 0], [2, 3, 4]),
            Some(([(9, 1); 3], NO_SLOTS))
        );
        assert!(changes.quiet().is_none());
    }

}
