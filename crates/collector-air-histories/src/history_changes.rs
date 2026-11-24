use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_air_history::air_history_mutations::MUTATIONS;
use engine_native_memory::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; 2] = [const { AtomicUsize::new(0) }; 2];
static READS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
static SCENARIO_REFUSALS: [AtomicU64; engine_air_history::air_kamikaze::SCENARIO_REFUSALS.len()] =
    [const { AtomicU64::new(0) }; engine_air_history::air_kamikaze::SCENARIO_REFUSALS.len()];
static MENU_REFUSALS: [AtomicU64; engine_air_history::menu_history::REFUSALS.len()] =
    [const { AtomicU64::new(0) }; engine_air_history::menu_history::REFUSALS.len()];
static RESTARTS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

pub struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; 2],
}

impl Default for Watches {
    fn default() -> Self {
        Self::new()
    }
}

impl Watches {
    pub const fn new() -> Self {
        Self {
            registry: WatchRegistry::retaining(),
            calls: [const { AtomicU64::new(0) }; 2],
        }
    }

    pub fn notify(&self, receiver: usize, site: usize) -> Notification<'_> {
        let notification = self.registry.notify(receiver);
        self.calls[site].fetch_add(1, Ordering::SeqCst);
        notification
    }

    fn offer(&self, receivers: usize) {
        self.registry
            .offer(receivers.max(self.registry.registered()));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(slots: usize, regions: usize) -> Self {
        let watches = Self::new();
        watches.offer(watches_for(slots, regions));
        watches.registry.boundary(false);
        watches.registry.set_enabled(true);
        watches
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn grow(&self, receivers: usize) {
        self.offer(receivers);
        self.registry.boundary(false);
        self.registry.collect();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn capacity(&self) -> usize {
        self.registry.capacity()
    }
}

fn watches_for(slots: usize, regions: usize) -> usize {
    slots.saturating_mul(regions.saturating_add(1))
}

pub fn touch_key(air: usize, region: u32) -> usize {
    air ^ (((region as usize & 0xffff) + 1) << 48)
}

pub fn active() -> usize {
    WATCHES.registry.active()
}

pub fn boundary(invalidate: bool) {
    let installed = WATCHES.registry.boundary(invalidate).installed();
    let cause = if invalidate {
        Some(0)
    } else if installed {
        Some(1)
    } else {
        crate::air_histories::opening().withdrawn().then_some(2)
    };
    if let Some(cause) = cause.filter(|_| cfg!(feature = "history_changes")) {
        RESTARTS[cause].fetch_add(1, Ordering::Relaxed);
    }
}

pub fn presize(slots: usize, regions: usize) {
    WATCHES.offer(watches_for(slots, regions.min(16)));
}

pub fn observe(receivers: usize) {
    WATCHES.offer(receivers);
}

pub fn collect() {
    WATCHES.registry.collect();
}

pub trait Changes {
    fn quiet(&self) -> Option<(u64, u64)>;
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)>;
    fn site(&self, index: usize) -> Option<(u64, u64)>;
}

impl Changes for Watches {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        self.registry.sequence(receiver)
    }
    fn site(&self, index: usize) -> Option<(u64, u64)> {
        let (epoch, _) = self.registry.quiet()?;
        let calls = self.calls[index].load(Ordering::SeqCst);
        (self.registry.quiet()?.0 == epoch).then_some((epoch, calls))
    }
}

pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        WATCHES.quiet()
    }
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(receiver)
    }
    fn site(&self, index: usize) -> Option<(u64, u64)> {
        WATCHES.site(index)
    }
}

pub struct Unobserved;
impl Changes for Unobserved {
    fn quiet(&self) -> Option<(u64, u64)> {
        None
    }
    fn sequence(&self, _: usize) -> Option<(u64, u64)> {
        None
    }
    fn site(&self, _: usize) -> Option<(u64, u64)> {
        None
    }
}

pub fn record(counts: &crate::air_histories::Counts) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.advanced,
        counts.refreshed,
        u64::from(counts.quiet),
        counts.patched,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
}

pub fn record_scenario_refusals(
    counts: &[u64; engine_air_history::air_kamikaze::SCENARIO_REFUSALS.len()],
) {
    for (target, &count) in SCENARIO_REFUSALS.iter().zip(counts) {
        target.fetch_add(count, Ordering::Relaxed);
    }
}

pub fn record_menu_refusals(counts: &[u64; engine_air_history::menu_history::REFUSALS.len()]) {
    for (target, &count) in MENU_REFUSALS.iter().zip(counts) {
        target.fetch_add(count, Ordering::Relaxed);
    }
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "history_changes") {
            let callbacks = [
                region_history as *const () as usize,
                advance as *const () as usize,
            ];
            for (index, site) in MUTATIONS.iter().enumerate() {
                let (address, original) = engine_sites::hook(base, site.rva)
                    .and_then(|code| Ok((code.address, code.sentinel::<16>()?)))
                    .inspect_err(|cause| {
                        crate::hook::installation::refused(
                            crate::hook::installation::Kind::Sentinel,
                            None,
                            cause,
                        )
                    })?;
                let hook = unsafe { Hook::new(address, callbacks[index], original) }
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
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        WATCHES
            .registry
            .set_enabled(enabled && !self.hooks.is_empty());
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

unsafe extern "system" fn region_history(air: usize, region: i32) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::HistoryChanges);
    let _notification = WATCHES.notify(air, 0);
    let _region = WATCHES.registry.notify(touch_key(air, region as u32));
    let original: unsafe extern "system" fn(usize, i32) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(air, region) })
}
unsafe extern "system" fn advance(windows: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::HistoryChanges);
    let _notification = WATCHES.notify(windows.wrapping_sub(8), 1);
    let original: unsafe extern "system" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL[1].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(windows) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_certificate_moves_with_its_calls_and_refuses_a_running_writer() {
        let watches = Watches::enabled(1, 1);
        let before = watches.site(1).unwrap();
        drop(watches.notify(0x40, 0));
        assert_eq!(watches.site(1), Some(before));
        let running = watches.notify(0x40, 1);
        assert_eq!(watches.site(1), None);
        drop(running);
        assert_eq!(watches.site(1), Some((before.0, before.1 + 1)));
        watches.registry.boundary(true);
        assert_ne!(watches.site(1).unwrap().0, before.0);
    }

}
