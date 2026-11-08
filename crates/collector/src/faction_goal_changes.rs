//! Demand for stored faction goals. No writer callback walks goals or allocates.
//! Ordinary pending checks are observed only at their actual stores; progress
//! refresh compares the incoming value with the one cache its CALL names. Missing observation
//! refuses this family rather than authorizing an hourly native scan.
use crate::{
    hook::{Hook, Registers, Tap},
    watch_registry::WatchRegistry,
};
use engine_mutations::{
    faction_goal_mutations::MUTATIONS, faction_goal_mutations::Patch,
    faction_goal_mutations::Receiver,
};
use engine_native::{Memory, cfaction_goal_status as native};
use std::{
    mem::ManuallyDrop,
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: WatchRegistry = WatchRegistry::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static CALLS: [AtomicU64; SITES] = [const { AtomicU64::new(0) }; SITES];
static DEMAND: AtomicUsize = AtomicUsize::new(0);
pub(crate) trait Changes: Sync {
    fn quiet(&self) -> Option<(u64, u64)>;
    fn sequence(&self, status: usize) -> Option<(u64, u64)>;
}
pub(crate) struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        WATCHES.quiet()
    }
    fn sequence(&self, status: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(status)
    }
}
pub fn active() -> usize {
    WATCHES.active()
}
pub fn boundary(invalidate: bool) {
    WATCHES.boundary(invalidate);
}
pub fn presize() {
    WATCHES.offer(DEMAND.load(Ordering::Relaxed).max(WATCHES.registered()));
}
pub fn collect() {
    WATCHES.collect();
}
pub(crate) fn demand(owners: usize) {
    DEMAND.fetch_max(owners, Ordering::Relaxed);
}
pub(crate) fn prime() {
    presize();
    WATCHES.boundary(false);
}
/// # Safety
/// The worker owns the Factions lease and is this domain's only capture.
pub(crate) unsafe fn install_growth() {
    unsafe {
        WATCHES.install_growth_quiescent();
    }
}

enum OwnedHook {
    Entry(Hook),
    Store(Tap),
}
impl OwnedHook {
    fn hook(&mut self) -> &mut Hook {
        match self {
            Self::Entry(hook) => hook,
            Self::Store(tap) => tap.publication_hook(),
        }
    }
    fn ranges(&self) -> Vec<Range<usize>> {
        match self {
            Self::Entry(hook) => hook.executable_ranges().into_iter().collect(),
            Self::Store(tap) => tap.executable_ranges(),
        }
    }
}
fn site_error(site: &str, operation: &str, cause: impl std::fmt::Display) -> &'static str {
    Box::leak(format!("FactionGoals/{site} {operation}: {cause}").into_boxed_str())
}
pub struct Hooks {
    hooks: Vec<OwnedHook>,
}
impl Hooks {
    /// # Safety
    /// Attach owns the verified image and native patch publication lifetime.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "faction_changes") {
            let mut located = [(0, [0; 16]); SITES];
            for ((site, _, _), slot) in MUTATIONS.iter().zip(&mut located) {
                *slot = engine_sites::hook(base, site.rva)
                    .and_then(|code| Ok((code.address, code.sentinel::<16>()?)))
                    .map_err(|cause| site_error(site.name, "sentinel verification", cause))?;
            }
            let callbacks = [
                construct as *const () as usize,
                destroy as *const () as usize,
                add_slots as *const () as usize,
                activate as *const () as usize,
                insert as *const () as usize,
                remove as *const () as usize,
                manifest as *const () as usize,
                template as *const () as usize,
                load as *const () as usize,
                rebuild as *const () as usize,
                progress as *const () as usize,
                0,
                completed as *const () as usize,
                cancelled as *const () as usize,
                0,
                0,
                0,
                0,
                0,
            ];
            engine_allocation::reserve(&mut hooks, SITES)
                .map_err(|cause| site_error("all sites", "hook allocation", cause))?;
            for (index, ((site, _, patch), &(at, original))) in
                MUTATIONS.iter().zip(&located).enumerate()
            {
                let hook = match patch {
                    Patch::Entry => {
                        let hook = unsafe { Hook::new(at, callbacks[index], original) }
                            .map_err(|cause| site_error(site.name, "construction", cause))?;
                        ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                        OwnedHook::Entry(hook)
                    }
                    Patch::Call => {
                        let displacement = i32::from_le_bytes(original[1..5].try_into().unwrap());
                        let target = at
                            .checked_add(5)
                            .and_then(|return_pc| {
                                return_pc.checked_add_signed(displacement as isize)
                            })
                            .ok_or_else(|| {
                                site_error(site.name, "call target", "address overflow")
                            })?;
                        let hook = unsafe { Hook::new_call(at, callbacks[index], original) }
                            .map_err(|cause| site_error(site.name, "construction", cause))?;
                        ORIGINAL[index].store(target, Ordering::Release);
                        OwnedHook::Entry(hook)
                    }
                    Patch::Tap => OwnedHook::Store(
                        unsafe { Tap::new(at, pending, index, original) }
                            .map_err(|cause| site_error(site.name, "construction", cause))?,
                    ),
                };
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            WATCHES.set_enabled(false);
        }
        if !Hook::set_enabled_iter_attach_tracking_busy(
            self.hooks.iter_mut().map(OwnedHook::hook),
            enabled,
        )
        .map_err(|cause| {
            let sites = MUTATIONS
                .iter()
                .map(|(site, _, _)| site.name)
                .collect::<Vec<_>>()
                .join(", ");
            site_error(&sites, "batch publication", cause)
        })? {
            return Ok(false);
        }
        WATCHES.set_enabled(enabled && self.hooks.len() == SITES);
        Ok(true)
    }
    pub fn busy_sites(&mut self) -> Vec<&'static str> {
        self.hooks
            .iter_mut()
            .zip(MUTATIONS.iter())
            .filter_map(|(hook, (site, _, _))| hook.hook().publication_busy().then_some(site.name))
            .collect()
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(OwnedHook::ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: runtime owns these hooks until restoration and flight drainage.
        unsafe { WATCHES.release() };
    }
}

type Function = unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize;
type Activation = unsafe extern "system-unwind" fn(usize, usize, usize, usize, usize) -> usize;
fn count(site: usize) {
    CALLS[site].fetch_add(1, Ordering::Relaxed);
}
fn owner(faction: usize) -> usize {
    faction.checked_add(native::STATUS).unwrap_or(0)
}
fn receiver(site: usize, first: usize) -> usize {
    match MUTATIONS[site].1 {
        Receiver::Cancelled => first.saturating_sub(native::CANCELLED),
        _ => first,
    }
}
macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system-unwind" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::FactionGoals);
            count($site);
            let notification = ManuallyDrop::new(WATCHES.notify(receiver($site, first)));
            let original: Function =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            let result = timing.original(|| unsafe { original(first, second, third, fourth) });
            drop(ManuallyDrop::into_inner(notification));
            result
        }
    };
}
callback!(construct, 0);
callback!(destroy, 1);
callback!(add_slots, 2);
callback!(insert, 4);
callback!(remove, 5);
callback!(manifest, 6);
callback!(template, 7);
callback!(load, 8);
callback!(rebuild, 9);
callback!(completed, 12);
callback!(cancelled, 13);

unsafe extern "system-unwind" fn activate(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
    fifth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::FactionGoals);
    count(3);
    let notification = ManuallyDrop::new(WATCHES.notify(first));
    let original: Activation = unsafe { std::mem::transmute(ORIGINAL[3].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(first, second, third, fourth, fifth) });
    drop(ManuallyDrop::into_inner(notification));
    result
}

unsafe extern "system-unwind" fn progress(
    first: usize,
    second: usize,
    third: usize,
    _fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::FactionGoals);
    count(10);
    let flight = ManuallyDrop::new(WATCHES.running());
    // SAFETY: the native caller owns this goal through the CALL's return.
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let address = first.checked_add(native::GOAL_PROGRESS - native::GOAL_PROGRESS_STATUS);
    let before = address.and_then(|at| engine_native::read::<i64>(&memory, at));
    let notification =
        ManuallyDrop::new(progress_changed(before, second as i64).then(|| WATCHES.notify(first)));
    let original: Function = unsafe { std::mem::transmute(ORIGINAL[10].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(first, second, third, 0) });
    drop(ManuallyDrop::into_inner(notification));
    drop(ManuallyDrop::into_inner(flight));
    result
}
fn progress_changed(before: Option<i64>, incoming: i64) -> bool {
    before != Some(incoming)
}

unsafe extern "system-unwind" fn pending(registers: &Registers, site: usize) {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::FactionGoals);
    count(site);
    let status = match MUTATIONS[site].1 {
        Receiver::PendingRbp => registers.rbp,
        Receiver::PendingRsi => registers.rsi,
        Receiver::FinishedFactionRsi => owner(registers.rsi),
        _ => unreachable!(),
    };
    drop(WATCHES.notify(status));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn busy_attach_publication_remains_retryable() {
        let _round = crate::hook::shared::Round::collect_attach().expect("own attach round");
        let mut hooks = ManuallyDrop::new(Hooks { hooks: Vec::new() });
        assert_eq!(hooks.set_enabled(true), Ok(false));
    }

    static ARGUMENTS: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
    unsafe extern "system-unwind" fn update(
        first: usize,
        incoming: usize,
        flags: usize,
        unused: usize,
    ) -> usize {
        for (target, value) in ARGUMENTS.iter().zip([first, incoming, flags, unused]) {
            target.store(value, Ordering::Relaxed);
        }
        19
    }
    #[test]
    fn faction_goal_progress_call_preserves_target_operands_and_return() {
        let mut goal = [0_u8; native::GOAL_STRIDE];
        goal[native::GOAL_PROGRESS..native::GOAL_PROGRESS + 8]
            .copy_from_slice(&(-7_i64).to_le_bytes());
        let first = goal.as_ptr() as usize + native::GOAL_PROGRESS_STATUS;
        WATCHES.offer(2);
        WATCHES.boundary(false);
        WATCHES.set_enabled(true);
        let structural = WATCHES.sequence(owner(0x1000)).unwrap();
        let cache = WATCHES.sequence(first).unwrap();
        let previous = ORIGINAL[10].swap(update as *const () as usize, Ordering::AcqRel);
        // SAFETY: the callback receives a stable synthetic goal cache and an
        // original with the same native operands.
        let result = unsafe { progress(first, 123, 0, 0) };
        ORIGINAL[10].store(previous, Ordering::Release);
        assert_eq!(result, 19);
        assert_eq!(
            ARGUMENTS.each_ref().map(|arg| arg.load(Ordering::Relaxed)),
            [first, 123, 0, 0]
        );
        assert_eq!(WATCHES.sequence(owner(0x1000)), Some(structural));
        assert_ne!(WATCHES.sequence(first), Some(cache));
        assert_eq!(WATCHES.active(), 0);
        WATCHES.set_enabled(false);
    }
    #[test]
    fn faction_goal_progress_change_uses_the_cache_not_modifier_change_flags() {
        assert!(!progress_changed(Some(-7), -7));
        assert!(progress_changed(Some(0), 1));
        assert!(progress_changed(None, 0));
    }
    #[test]
    fn faction_goal_writers_move_only_the_selected_status_certificate() {
        let watches = WatchRegistry::new();
        watches.offer(5000);
        watches.boundary(false);
        watches.set_enabled(true);
        let first = watches.sequence(0x1000).unwrap();
        let second = watches.sequence(0x2000).unwrap();
        assert_eq!(receiver(13, 0x2000 + native::CANCELLED), 0x2000);
        drop(watches.notify(receiver(13, 0x2000 + native::CANCELLED)));
        assert_eq!(watches.sequence(0x1000), Some(first));
        assert_ne!(watches.sequence(0x2000), Some(second));
        assert_eq!(owner(0x1000), 0x1000 + native::STATUS);
        let pending = watches.notify(0x1000);
        assert!(watches.quiet().is_none());
        drop(pending);
    }
}
