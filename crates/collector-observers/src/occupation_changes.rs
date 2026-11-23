use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_native_memory::Memory;
use engine_native::cstate;
use engine_occupation::cresistance::changes::{Receiver, SITES};
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering},
};

static WATCHES: WatchRegistry = WatchRegistry::new();
static ORIGINAL: [AtomicUsize; SITES.len()] = [const { AtomicUsize::new(0) }; SITES.len()];
static OWN: AtomicBool = AtomicBool::new(false);
static FORWARDERS: [AtomicBool; 6] = [const { AtomicBool::new(false) }; 6];
static DEMAND: AtomicUsize = AtomicUsize::new(0);
static LEGITIMACY_FORWARDER: AtomicPtr<LegitimacyUpdateForwarder> =
    AtomicPtr::new(std::ptr::null_mut());

pub struct LegitimacyUpdateForwarder {
    pub notify: fn(usize) -> Option<Notification<'static>>,
    pub enabled: fn(bool),
}

#[doc = " Register before patch installation, or unregister after native callbacks"]
#[doc = " quiesce. Callbacks and their static registration live until DLL unload."]
pub fn legitimacy_update_forwarder(receiver: Option<&'static LegitimacyUpdateForwarder>) {
    let next = receiver.map_or(std::ptr::null_mut(), |value| {
        std::ptr::from_ref(value).cast_mut()
    });
    let previous = LEGITIMACY_FORWARDER.swap(next, Ordering::SeqCst);
    if let Some(previous) = unsafe { previous.as_ref() } {
        (previous.enabled)(false);
    }
    observe_legitimacy_forwarder();
}

fn observe_legitimacy_forwarder() {
    if let Some(receiver) = unsafe { LEGITIMACY_FORWARDER.load(Ordering::SeqCst).as_ref() } {
        (receiver.enabled)(OWN.load(Ordering::SeqCst));
    }
}

#[derive(Clone, Copy)]
pub enum Forwarder {
    Countries,
    States,
    Territory,
    Relations,
    Exile,
    Templates,
}
pub fn forwarder(which: Forwarder, enabled: bool) {
    FORWARDERS[which as usize].store(enabled, Ordering::SeqCst);
    observe();
}
fn observe() {
    WATCHES.set_enabled(
        OWN.load(Ordering::SeqCst) && FORWARDERS.iter().all(|v| v.load(Ordering::SeqCst)),
    );
}
#[cfg(any(test, feature = "test-support"))]
pub fn observe_for_tests(own: bool) {
    OWN.store(own, Ordering::SeqCst);
    observe();
}
pub fn quiet() -> Option<(u64, u64)> {
    WATCHES.quiet()
}
pub fn sequence(receiver: usize) -> Option<(u64, u64)> {
    WATCHES.sequence(receiver)
}
pub fn changed(receiver: usize) -> Notification<'static> {
    WATCHES.notify(receiver)
}
pub fn holder_changed(receiver: Option<usize>) -> Notification<'static> {
    match receiver.filter(|&receiver| receiver != 0) {
        Some(receiver) => changed(receiver),
        None => WATCHES.notify_every(),
    }
}
pub fn active() -> usize {
    WATCHES.active()
}
pub fn boundary(invalidate: bool) {
    WATCHES.boundary(invalidate);
}
pub fn demand(receivers: usize) {
    DEMAND.fetch_max(receivers, Ordering::Relaxed);
}
pub fn prepare() {
    let n = DEMAND.load(Ordering::Relaxed);
    if n != 0 {
        WATCHES.offer(n);
    }
    WATCHES.collect();
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// # Safety
    /// The caller has verified this process's executable and owns installation and quiescence.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, String> {
        let mut hooks = Vec::new();
        if cfg!(feature = "naval_tree_changes")
            && SITES.iter().all(|s| crate::verified(base, s.rva).is_ok())
        {
            let callbacks: [usize; SITES.len()] = [
                update_levels as *const () as usize,
                update_speeds as *const () as usize,
                reset as *const () as usize,
                stop as *const () as usize,
                start as *const () as usize,
                change_occupied_country as *const () as usize,
                transfer_compliance as *const () as usize,
                reset_target as *const () as usize,
                add_spread as *const () as usize,
                update_effects as *const () as usize,
                build_local_modifiers as *const () as usize,
                change_compliance as *const () as usize,
                change_resistance as *const () as usize,
                set_compliance as *const () as usize,
                set_resistance as *const () as usize,
                remove_added_target as *const () as usize,
                add_target as *const () as usize,
                daily_diplomacy_update as *const () as usize,
                rebuild_relation_modifiers as *const () as usize,
                check_activation as *const () as usize,
                initialize_occupation as *const () as usize,
                rebuild_state_active_definitions as *const () as usize,
                clear_state_active_definitions as *const () as usize,
                rebuild_core_active_definitions as *const () as usize,
                clear_holder_active_definitions as *const () as usize,
            ];
            engine_allocation::reserve(&mut hooks, SITES.len())
                .map_err(|_| "occupation_changes hooks allocation failed")?;
            for (i, site) in SITES.iter().enumerate() {
                let code = engine_sites::hook(base, site.rva)?;
                let hook = unsafe { Hook::new(code.address, callbacks[i], code.sentinel::<16>()?) }
                    .map_err(|cause| {
                        format!(
                            "occupation_changes hook {} construction failed: {cause}",
                            site.name
                        )
                    })?;
                ORIGINAL[i].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !enabled {
            OWN.store(false, Ordering::SeqCst);
            observe_legitimacy_forwarder();
            observe();
        }
        if !Hook::set_family_enabled_shared_named(
            &mut self.hooks,
            enabled,
            "occupation_changes",
            &SITES.map(|site| site.name),
        )? {
            return Ok(false);
        }
        OWN.store(enabled && self.hooks.len() == SITES.len(), Ordering::SeqCst);
        observe_legitimacy_forwarder();
        observe();
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        unsafe {
            WATCHES.release();
        }
    }
}
fn receiver(site: usize, first: usize) -> usize {
    match SITES[site].receiver {
        Receiver::Resistance => first.wrapping_sub(cstate::RESISTANCE),
        Receiver::Diplomacy | Receiver::Relation | Receiver::OccupationHolder => first,
        Receiver::OccupationData => 0,
    }
}
fn core_holder(memory: &impl Memory, first: usize) -> Option<usize> {
    first
        .checked_add(engine_occupation::ccountry_occupation_data::PARENT_STATUS)
        .and_then(|at| engine_native_memory::read::<u64>(memory, at))
        .and_then(|value| usize::try_from(value).ok())
        .filter(|&value| first != 0 && value != 0)
}
macro_rules! callback {
    ($name:ident,$site:expr) => {
        unsafe extern "system" fn $name(a: usize, b: usize, c: usize, d: usize) -> usize {
            let _hook = crate::hook_timing::label();
            let _mark = changed(receiver($site, a));
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            unsafe { original(a, b, c, d) }
        }
    };
}
callback!(update_levels, 0);
callback!(update_speeds, 1);
callback!(reset, 2);
callback!(stop, 3);
callback!(start, 4);
callback!(change_occupied_country, 5);
callback!(transfer_compliance, 6);
callback!(reset_target, 7);
callback!(add_spread, 8);
callback!(update_effects, 9);
callback!(build_local_modifiers, 10);
callback!(change_compliance, 11);
callback!(change_resistance, 12);
callback!(set_compliance, 13);
callback!(set_resistance, 14);
callback!(remove_added_target, 15);
unsafe extern "system" fn add_target(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
    f: usize,
    g: usize,
) -> usize {
    let _hook = crate::hook_timing::label();
    let _mark = changed(receiver(16, a));
    let original: unsafe extern "system" fn(
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
    ) -> usize = unsafe { std::mem::transmute(ORIGINAL[16].load(Ordering::Acquire)) };
    unsafe { original(a, b, c, d, e, f, g) }
}
unsafe extern "system" fn daily_diplomacy_update(a: usize, b: usize, c: usize, d: usize) -> usize {
    let _hook = crate::hook_timing::label();
    let _mark = changed(receiver(17, a));
    let _forwarded = unsafe { LEGITIMACY_FORWARDER.load(Ordering::SeqCst).as_ref() }
        .and_then(|receiver| (receiver.notify)(a));
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[17].load(Ordering::Acquire)) };
    unsafe { original(a, b, c, d) }
}
callback!(rebuild_relation_modifiers, 18);
callback!(check_activation, 19);
callback!(initialize_occupation, 20);
macro_rules! marker_callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system-unwind" fn $name(a: usize, b: usize, c: usize, d: usize) -> usize {
            let _hook = crate::hook_timing::label();
            let mark = std::mem::ManuallyDrop::new(changed(receiver($site, a)));
            let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            let result = unsafe { original(a, b, c, d) };
            drop(std::mem::ManuallyDrop::into_inner(mark));
            result
        }
    };
}
marker_callback!(rebuild_state_active_definitions, 21);
marker_callback!(clear_state_active_definitions, 22);
unsafe extern "system-unwind" fn rebuild_core_active_definitions(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
) -> usize {
    let _hook = crate::hook_timing::label();
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let mark = std::mem::ManuallyDrop::new(holder_changed(core_holder(&memory, a)));
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[23].load(Ordering::Acquire)) };
    let result = unsafe { original(a, b, c, d) };
    drop(std::mem::ManuallyDrop::into_inner(mark));
    result
}
marker_callback!(clear_holder_active_definitions, 24);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn occupation_marker_writers_select_state_and_guarded_holder_receivers() {
        let mut bytes = [0u8; 16];
        bytes[8..].copy_from_slice(&0x2000u64.to_le_bytes());
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(core_holder(&memory, 0x1000), Some(0x2000));
        assert_eq!(core_holder(&memory, 0x3000), None);
        assert_eq!(receiver(21, 123 + cstate::RESISTANCE), 123);
        assert_eq!(receiver(22, 456 + cstate::RESISTANCE), 456);
        assert_eq!(receiver(24, 789), 789);
        let watches = WatchRegistry::enabled(8);
        let old = watches.sequence(0x2000).unwrap();
        {
            let _mark = watches.notify_every();
            assert!(watches.sequence(0x2000).is_none());
        }
        assert_ne!(watches.sequence(0x2000).unwrap(), old);
    }
    #[test]
    fn legitimacy_update_forwarder_holds_receiver_over_original_and_tracks_admission() {
        static WATCHES: std::sync::LazyLock<WatchRegistry> =
            std::sync::LazyLock::new(|| WatchRegistry::enabled(64));
        static ADMITTED: AtomicBool = AtomicBool::new(true);
        static FORWARDED: AtomicUsize = AtomicUsize::new(0);
        static RECEIVER: LegitimacyUpdateForwarder = LegitimacyUpdateForwarder {
            notify: |receiver| {
                FORWARDED.store(receiver, Ordering::SeqCst);
                Some(WATCHES.notify(receiver))
            },
            enabled: |enabled| ADMITTED.store(enabled, Ordering::SeqCst),
        };
        unsafe extern "system" fn original(a: usize, b: usize, c: usize, d: usize) -> usize {
            assert_eq!([a, b, c, d], [333, 2, 3, 4]);
            assert_eq!(FORWARDED.load(Ordering::SeqCst), a);
            assert!(WATCHES.sequence(a).is_none());
            a + d
        }
        let before = WATCHES.sequence(333).unwrap();
        ORIGINAL[17].store(original as *const () as usize, Ordering::Release);
        legitimacy_update_forwarder(Some(&RECEIVER));
        assert!(!ADMITTED.load(Ordering::SeqCst));
        OWN.store(true, Ordering::SeqCst);
        observe_legitimacy_forwarder();
        assert!(ADMITTED.load(Ordering::SeqCst));
        assert_eq!(unsafe { daily_diplomacy_update(333, 2, 3, 4) }, 337);
        assert_ne!(WATCHES.sequence(333).unwrap(), before);
        OWN.store(false, Ordering::SeqCst);
        observe_legitimacy_forwarder();
        assert!(!ADMITTED.load(Ordering::SeqCst));
        legitimacy_update_forwarder(None);
    }

    #[test]
    fn occupation_forwarding_preserves_all_seven_script_target_arguments() {
        unsafe extern "system" fn original(
            a: usize,
            b: usize,
            c: usize,
            d: usize,
            e: usize,
            f: usize,
            g: usize,
        ) -> usize {
            assert_eq!([b, c, d, e, f, g], [2, 3, 4, 5, 6, 7]);
            a.wrapping_add(g)
        }
        ORIGINAL[16].store(original as *const () as usize, Ordering::Release);
        assert_eq!(
            unsafe { add_target(cstate::RESISTANCE + 100, 2, 3, 4, 5, 6, 7) },
            cstate::RESISTANCE + 107
        );
        assert_eq!(receiver(16, cstate::RESISTANCE + 100), 100);
        assert_eq!(receiver(17, 333), 333);
        assert_eq!(receiver(18, 555), 555);
    }
}
