use collector_certificates::watch_registry::WatchRegistry;
use collector_hooks::hook::{Hook, Registers, Tap};
use engine_mutations::{
    faction_goal_mutations::MUTATIONS, faction_goal_mutations::Patch,
    faction_goal_mutations::Receiver,
};
use engine_native_memory::Memory;
use engine_native_parts::cfaction_goal_status as native;
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
pub trait Changes: Sync {
    fn quiet(&self) -> Option<(u64, u64)>;
    fn sequence(&self, status: usize) -> Option<(u64, u64)>;
}
pub struct Observer;
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
/// # Safety
/// Only the attaching prime pass or the admitted hour's sole observer boundary (or an isolated
/// test) may advance the faction-goal watch boundary.
pub unsafe fn boundary(invalidate: bool) {
    #[cfg(test)]
    let _exclusive = tests::exclusive();
    WATCHES.boundary(invalidate);
}
/// # Safety
/// Run only during attach or the active session's serial sizing pass (or an isolated test), while
/// the faction-goal watches remain live and teardown is excluded.
pub unsafe fn presize() {
    #[cfg(test)]
    let _exclusive = tests::exclusive();
    WATCHES.offer(DEMAND.load(Ordering::Relaxed).max(WATCHES.registered()));
}
pub fn collect() {
    #[cfg(test)]
    let _exclusive = tests::exclusive();
    WATCHES.collect();
}
/// # Safety
/// Run only during active session sizing or admitted faction capture (or an isolated test) with
/// measured owner demand.
pub unsafe fn demand(owners: usize) {
    #[cfg(test)]
    let _exclusive = tests::exclusive();
    DEMAND.fetch_max(owners, Ordering::Relaxed);
}
/// # Safety
/// Only the attaching runtime or an isolated collector fixture may prime the live faction-goal
/// watches, excluding concurrent observer boundaries, capture and detach.
pub unsafe fn prime() {
    #[cfg(test)]
    let _exclusive = tests::exclusive();
    // SAFETY: Caller guarantees exclusive attach/fixture priming with live watches.
    unsafe { presize() };
    WATCHES.boundary(false);
}
/// # Safety
/// The worker owns the Factions lease and is this domain's only capture.
pub unsafe fn install_growth() {
    #[cfg(test)]
    let _exclusive = tests::exclusive();
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
fn site_error(site: &str, operation: &str, cause: impl std::fmt::Display) -> String {
    format!("FactionGoals/{site} {operation}: {cause}")
}
pub struct Hooks {
    hooks: Vec<OwnedHook>,
}
impl Hooks {
    /// # Safety
    /// Attach owns the verified image and native patch publication lifetime.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, String> {
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
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        #[cfg(test)]
        let _exclusive = tests::exclusive();
        if !enabled {
            WATCHES.set_enabled(false);
        }
        if !Hook::set_enabled_iter_tracking_busy(
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
        #[cfg(test)]
        let _exclusive = tests::exclusive();
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
            let timing = collector_timing::hook_timing::enter(
                collector_timing::hook_timing::Family::FactionGoals,
            );
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
    let timing =
        collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::FactionGoals);
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
    let timing =
        collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::FactionGoals);
    count(10);
    let flight = ManuallyDrop::new(WATCHES.running());
    // SAFETY: the native caller owns this goal through the CALL's return.
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let address = first.checked_add(native::GOAL_PROGRESS - native::GOAL_PROGRESS_STATUS);
    let before = address.and_then(|at| engine_native_memory::read::<i64>(&memory, at));
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
    let _timing =
        collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::FactionGoals);
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
    use std::{cell::Cell, sync::Mutex};

    static TEST: Mutex<()> = Mutex::new(());
    thread_local! {
        static HELD: Cell<bool> = const { Cell::new(false) };
    }
    pub(super) struct Exclusive(Option<std::sync::MutexGuard<'static, ()>>);
    impl Drop for Exclusive {
        fn drop(&mut self) {
            if self.0.is_some() {
                HELD.set(false);
            }
        }
    }
    pub(super) fn exclusive() -> Exclusive {
        if HELD.get() {
            return Exclusive(None);
        }
        let turn = TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        HELD.set(true);
        Exclusive(Some(turn))
    }
    struct Serial {
        original: [usize; SITES],
        _turn: Exclusive,
    }
    fn serial() -> Serial {
        let turn = exclusive();
        Serial {
            original: ORIGINAL.each_ref().map(|site| site.load(Ordering::Acquire)),
            _turn: turn,
        }
    }
    impl Drop for Serial {
        fn drop(&mut self) {
            WATCHES.set_enabled(false);
            for (site, original) in ORIGINAL.iter().zip(self.original) {
                site.store(original, Ordering::Release);
            }
        }
    }

    unsafe extern "system" fn busy_original() -> usize {
        7
    }
    unsafe extern "system" fn busy_callback() -> usize {
        19
    }

    #[test]
    fn busy_attach_publication_remains_retryable() {
        static FLIGHT: AtomicUsize = AtomicUsize::new(1);
        let _guard = collector_hooks::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let code = collector_hooks::hook::TestCode::new();
        let (site, bytes) = code.stub(0, busy_original as *const () as usize);
        let callback = busy_callback as *const () as usize;
        let hook = unsafe {
            Hook::new_with_safety(
                site,
                callback,
                bytes,
                collector_hooks::hook::TapSafety {
                    in_flight: &FLIGHT,
                    stub: callback..callback + 1,
                },
            )
        }
        .unwrap();
        let mut hooks = ManuallyDrop::new(Hooks {
            hooks: vec![OwnedHook::Entry(hook)],
        });
        assert_eq!(hooks.set_enabled(true), Ok(false));
        assert_eq!(hooks.busy_sites(), [MUTATIONS[0].0.name]);
        {
            let mut round =
                collector_hooks::hook::shared::Round::collect_attach().expect("own attach round");
            collector_hooks::hook::publication::select_family(Some("faction_goals"));
            assert_eq!(
                hooks.set_enabled(true),
                Ok(true),
                "collect records it and goes on"
            );
            round.publish();
            assert_eq!(hooks.set_enabled(true), Ok(false), "the round held it back");
        }
        collector_hooks::hook::publication::select_family(None);
        assert!(!hooks.hooks[0].hook().is_enabled());
        FLIGHT.store(0, Ordering::SeqCst);
        assert_eq!(hooks.set_enabled(true), Ok(true));
        assert_eq!(hooks.set_enabled(false), Ok(true));
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
        let _serial = serial();
        let mut goal = [0_u8; native::GOAL_STRIDE];
        goal[native::GOAL_PROGRESS..native::GOAL_PROGRESS + 8]
            .copy_from_slice(&(-7_i64).to_le_bytes());
        let first = goal.as_ptr() as usize + native::GOAL_PROGRESS_STATUS;
        WATCHES.offer(2);
        WATCHES.boundary(false);
        WATCHES.set_enabled(true);
        let structural = WATCHES.sequence(owner(0x1000)).unwrap();
        let cache = WATCHES.sequence(first).unwrap();
        ORIGINAL[10].store(update as *const () as usize, Ordering::Release);
        let result = unsafe { progress(first, 123, 0, 0) };
        assert_eq!(result, 19);
        assert_eq!(
            ARGUMENTS.each_ref().map(|arg| arg.load(Ordering::Relaxed)),
            [first, 123, 0, 0]
        );
        assert_eq!(WATCHES.sequence(owner(0x1000)), Some(structural));
        assert_ne!(WATCHES.sequence(first), Some(cache));
        assert_eq!(WATCHES.active(), 0);
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
