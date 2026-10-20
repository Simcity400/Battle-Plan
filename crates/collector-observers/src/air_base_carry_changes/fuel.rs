//! Dynamic state-container scalars have a receiver certificate separate from
//! membership, so hourly fuel writers never force a registration-tree walk.
//! The demand updater owns one hook; the existing fuel award owner forwards
//! allocation stores and holds their notification through the native store.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::air_base_mutations::FUEL_DEMAND;
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

/// Notification whose lifetime spans a forwarded receipt store.
pub type Allocation = Notification<'static>;

static WATCHES: WatchRegistry = WatchRegistry::new();
static OWNED: AtomicBool = AtomicBool::new(false);
static AWARDS: AtomicBool = AtomicBool::new(false);
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static DEMAND: AtomicUsize = AtomicUsize::new(0);

fn apply() {
    WATCHES.set_enabled(OWNED.load(Ordering::SeqCst) && AWARDS.load(Ordering::SeqCst));
}
/// The existing fuel award owner's entire receipt family is observed.
pub fn forwarder(enabled: bool) {
    AWARDS.store(enabled, Ordering::SeqCst);
    apply();
}
/// Hold over a forwarded native allocation store. No native read or allocation.
pub fn allocated(container: usize) -> Notification<'static> {
    WATCHES.notify(container)
}
/// Register before reading a container's two current scalar words.
pub fn sequence(container: usize) -> Option<(u64, u64)> {
    WATCHES.sequence(container)
}
/// Worker demand, used by the existing observer's idle sizing pass.
pub fn record_demand(count: usize) {
    DEMAND.fetch_max(count, Ordering::Relaxed);
}
pub(super) fn presize() {
    let demand = DEMAND.load(Ordering::Relaxed);
    if demand != 0 {
        WATCHES.offer(demand);
    }
}
pub(super) fn collect() {
    WATCHES.collect();
}
pub(super) fn boundary(invalidate: bool) {
    WATCHES.boundary(invalidate);
}
pub(super) fn active() -> usize {
    WATCHES.active()
}

pub(super) fn status() -> impl serde::Serialize {
    crate::status::status!({
        "demand_observed": OWNED.load(Ordering::SeqCst),
        "allocation_observed": AWARDS.load(Ordering::SeqCst),
        "container_demand": DEMAND.load(Ordering::Relaxed),
        "receivers": WATCHES.status(),
    })
}

pub(super) struct Hooks {
    hook: Vec<Hook>,
}
impl Hooks {
    /// # Safety
    /// The caller owns the authenticated executable and the patch family.
    pub(super) unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hook = Vec::new();
        if cfg!(feature = "air_base_carry_changes")
            && engine_native::read::<[u8; 16]>(memory, base + FUEL_DEMAND.rva)
                == Some(FUEL_DEMAND.sentinel)
        {
            let installed = unsafe {
                Hook::new(
                    base + FUEL_DEMAND.rva,
                    demand as *const () as usize,
                    FUEL_DEMAND.sentinel,
                )
            }
            .map_err(|_| "air container fuel demand hook construction failed")?;
            ORIGINAL.store(installed.trampoline(), Ordering::Release);
            hook.push(installed);
        }
        Ok(Self { hook })
    }
    pub(super) fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            OWNED.store(false, Ordering::SeqCst);
            apply();
        }
        if !Hook::set_family_enabled_shared(&mut self.hook, enabled)
            .map_err(|_| "air container fuel demand hook publication failed")?
        {
            return Ok(false);
        }
        OWNED.store(enabled && !self.hook.is_empty(), Ordering::SeqCst);
        apply();
        Ok(true)
    }
    pub(super) fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hook.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // The host removes and quiesces the owning family before its drop.
        unsafe { WATCHES.release() };
    }
}
unsafe extern "system" fn demand(container: usize, base: usize, penalty: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::AirBaseCarryChanges);
    let _changed = WATCHES.notify(container);
    let original: unsafe extern "system" fn(usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL.load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(container, base, penalty) })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn demand_callback_preserves_arguments_and_brackets_only_its_container() {
        unsafe extern "system" fn original(container: usize, base: usize, penalty: usize) -> usize {
            assert_eq!([container, base, penalty], [0x1200, 0x2200, 33]);
            assert_eq!(WATCHES.active(), 1);
            assert_eq!(WATCHES.sequence(container), None);
            assert_eq!(WATCHES.quiet(), None);
            57
        }
        WATCHES.offer(2);
        WATCHES.boundary(false);
        WATCHES.set_enabled(true);
        let before = WATCHES.sequence(0x1200).unwrap();
        let other = WATCHES.sequence(0x1300).unwrap();
        ORIGINAL.store(original as *const () as usize, Ordering::Release);
        let result = collector_test_support::with_allocations_denied(|| unsafe {
            demand(0x1200, 0x2200, 33)
        });
        assert_eq!(result, 57);
        assert_eq!(WATCHES.active(), 0);
        assert_eq!(WATCHES.sequence(0x1200), Some((before.0, before.1 + 1)));
        assert_eq!(WATCHES.sequence(0x1300), Some(other));
    }

    #[test]
    #[ignore = "per-call air-container demand microbenchmark; run only without a live game benchmark"]
    fn hook_cost_air_container_demand_dispatch() {
        const CALLS: usize = 100_000;
        unsafe extern "system" fn original(container: usize, base: usize, penalty: usize) -> usize {
            std::hint::black_box(container ^ base ^ penalty)
        }
        WATCHES.offer(1);
        WATCHES.boundary(false);
        WATCHES.set_enabled(true);
        WATCHES.sequence(0x1200).unwrap();
        ORIGINAL.store(original as *const () as usize, Ordering::Release);
        let callback: unsafe extern "system" fn(usize, usize, usize) -> usize = demand;
        let nanos = collector_test_support::per_call_ns(CALLS, || {
            for _ in 0..CALLS {
                std::hint::black_box(unsafe { std::hint::black_box(callback)(0x1200, 0x2200, 33) });
            }
        });
        println!("hook_cost air_base container demand: callback={nanos:.1} ns/call");
    }

    #[test]
    fn demand_and_allocation_are_independent_of_registration_receivers() {
        let domain = WatchRegistry::enabled(4);
        let first = domain.sequence(0x1200).unwrap();
        let other = domain.sequence(0x1300).unwrap();
        let writing = domain.notify(0x1200);
        assert_eq!(domain.sequence(0x1200), None);
        drop(writing);
        assert_ne!(domain.sequence(0x1200), Some(first));
        assert_eq!(domain.sequence(0x1300), Some(other));
        domain.boundary(true);
        assert_ne!(domain.sequence(0x1300), Some(other));
    }
}
