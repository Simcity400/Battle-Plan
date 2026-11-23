use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_events::diplomacy_mutations::MUTATIONS;
use engine_native_memory::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

static WATCHES: WatchRegistry = WatchRegistry::new();
const RECEIVERS_PER_COUNTRY: usize = 2;
static ORIGINAL: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
static EVENTS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static READS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

pub fn active() -> usize {
    WATCHES.active()
}

pub fn boundary(invalidate: bool) {
    WATCHES.boundary(invalidate);
}

pub fn presize(slots: usize) {
    WATCHES.offer(slots.saturating_mul(RECEIVERS_PER_COUNTRY));
}

pub fn collect() {
    WATCHES.collect();
}

fn mutation(receiver: usize, site: usize) -> Notification<'static> {
    EVENTS[site].fetch_add(1, Ordering::Relaxed);
    WATCHES.notify(receiver)
}

pub trait Changes {
    fn quiet(&self) -> Option<(u64, u64)>;
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)>;
}

pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        WATCHES.quiet()
    }
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(receiver)
    }
}

pub fn record(reused: u64, read: u64, quiet_hours: u64) {
    for (target, value) in READS.iter().zip([reused, read, quiet_hours]) {
        target.fetch_add(value, Ordering::Relaxed);
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
        if cfg!(feature = "relation_changes") {
            let callbacks = [
                rebuild as *const () as usize,
                reset as *const () as usize,
                add as *const () as usize,
                remove as *const () as usize,
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
            WATCHES.set_enabled(false);
            crate::country_input_changes::forwarder(
                crate::country_input_changes::Forwarder::Relations,
                false,
            );
            crate::war_changes::forwarder(crate::war_changes::Forwarder::Relations, false);
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        WATCHES.set_enabled(enabled && !self.hooks.is_empty());
        crate::country_input_changes::forwarder(
            crate::country_input_changes::Forwarder::Relations,
            enabled && !self.hooks.is_empty(),
        );
        crate::war_changes::forwarder(
            crate::war_changes::Forwarder::Relations,
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
        unsafe { WATCHES.release() };
    }
}

unsafe extern "system" fn rebuild(diplomacy: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::RelationChanges);
    let _notification = mutation(diplomacy, 0);
    let _inputs = crate::country_input_changes::forwarded(
        crate::country_input_changes::Forward::WartimeAllies,
        diplomacy,
        0,
    );
    let original: unsafe extern "system" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(diplomacy) })
}
unsafe extern "system" fn reset(diplomacy: usize, options: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::RelationChanges);
    let _notification = mutation(diplomacy, 1);
    let _inputs = crate::country_input_changes::forwarded(
        crate::country_input_changes::Forward::DiplomacyReset,
        diplomacy,
        0,
    );
    let _wars = crate::war_changes::forwarded(crate::war_changes::Site::DiplomacyReset);
    let _participation = crate::war_changes::participation_change(
        crate::war_changes::Site::DiplomacyReset,
        diplomacy,
        diplomacy,
    );
    let original: unsafe extern "system" fn(usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[1].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(diplomacy, options) })
}
unsafe extern "system" fn add(country: usize, tag: usize) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::RelationChanges);
    let _notification = mutation(country, 2);
    let original: unsafe extern "system" fn(usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[2].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(country, tag) })
}
unsafe extern "system" fn remove(country: usize, tag: usize, notify: u8) {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::RelationChanges);
    let _notification = mutation(country, 3);
    let original: unsafe extern "system" fn(usize, usize, u8) =
        unsafe { std::mem::transmute(ORIGINAL[3].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(country, tag, notify) })
}
