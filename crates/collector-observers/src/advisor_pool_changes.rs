use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::{advisor_pool_mutations::MUTATIONS, advisor_pool_mutations::Receiver};
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
const UNAPPOINT: usize = SITES - 1;
static PATCHED: AtomicBool = AtomicBool::new(false);
static UNAPPOINT_FORWARDER: AtomicBool = AtomicBool::new(false);
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static READS: [[AtomicU64; 4]; 4] = [const { [const { AtomicU64::new(0) }; 4] }; 4];
static POOLS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    AvailableAdvisors,
    AdvisorTokens,
    HiredAdvisors,
    CharacterRecords,
}

pub struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; SITES],
    unnamed: AtomicU64,
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
        }
    }

    pub fn notify(&self, object: Option<usize>, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        match object {
            Some(object) => self.registry.notify(object),
            None => {
                self.unnamed.fetch_add(1, Ordering::Relaxed);
                self.registry.notify_every()
            }
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
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }
}

pub trait Changes: Sync {
    fn sequence(&self, object: usize) -> Option<(u64, u64)>;
    fn character(&self, character: usize) -> Option<(u64, u64)>;
    fn characters_quiet(&self) -> Option<(u64, u64)>;
}

pub struct Observer;
impl Changes for Observer {
    fn sequence(&self, object: usize) -> Option<(u64, u64)> {
        WATCHES.registry.sequence(object)
    }
    fn character(&self, character: usize) -> Option<(u64, u64)> {
        crate::character_changes::Changes::sequence(&crate::character_changes::Observer, character)
    }
    fn characters_quiet(&self) -> Option<(u64, u64)> {
        crate::character_changes::quiet()
    }
}

#[cfg(any(test, feature = "test-support"))]
pub struct Pair<'a> {
    pub pools: &'a Watches,
    pub characters: &'a crate::character_changes::Watches,
}
#[cfg(any(test, feature = "test-support"))]
impl Changes for Pair<'_> {
    fn sequence(&self, object: usize) -> Option<(u64, u64)> {
        self.pools.registry.sequence(object)
    }
    fn character(&self, character: usize) -> Option<(u64, u64)> {
        crate::character_changes::Changes::sequence(self.characters, character)
    }
    fn characters_quiet(&self) -> Option<(u64, u64)> {
        self.characters.quiet()
    }
}

fn receiver(receiver: Receiver, first: usize) -> Option<usize> {
    match receiver {
        Receiver::Pool | Receiver::Tokens => Some(first),
        Receiver::Unnamed => None,
    }
}

pub fn quiet() -> Option<(u64, u64)> {
    WATCHES.registry.quiet()
}

pub fn active() -> usize {
    WATCHES.registry.active()
}

pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
}

/// # Safety
/// Run only during attach or the active session's serial sizing pass (or an isolated test), while
/// advisor-pool watches remain live and teardown is excluded.
pub unsafe fn presize() {
    let pools = POOLS.load(Ordering::Relaxed);
    if pools != 0 {
        WATCHES.registry.offer(pools + 1);
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
}

/// # Safety
/// Run only during admitted advisor capture (or an isolated test) with measured pool demand for
/// the active session.
pub unsafe fn record(table: Table, counts: &Counts, pools: Option<usize>) {
    for (target, value) in READS[table as usize].iter().zip([
        counts.reused,
        counts.read,
        counts.reused_rows,
        counts.read_rows,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if let Some(pools) = pools {
        POOLS.store(pools, Ordering::Relaxed);
    }
}


pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A differing sentinel leaves the family uninstalled; both tables then
    /// read afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "advisor_pool_changes")
            && MUTATIONS[..UNAPPOINT]
                .iter()
                .all(|(site, _)| crate::located(base, site.rva).is_ok())
        {
            let callbacks: [usize; UNAPPOINT] = [
                update_advisor_status as *const () as usize,
                add_advisor as *const () as usize,
                retire_character as *const () as usize,
                transfer_characters as *const () as usize,
                remove_advisor_role as *const () as usize,
                construct_pool as *const () as usize,
                destroy_pool as *const () as usize,
                update_advisor_tokens as *const () as usize,
                clear_character_manager as *const () as usize,
                construct_character_manager as *const () as usize,
                destroy_character_manager as *const () as usize,
                insert_appointed_advisor as *const () as usize,
                register_character as *const () as usize,
                copy_retired_characters as *const () as usize,
            ];
            for (index, (site, _)) in MUTATIONS[..UNAPPOINT].iter().enumerate() {
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
            PATCHED.store(false, Ordering::SeqCst);
            observe();
            crate::roster_changes::set_manager_forward(false);
            crate::country_input_changes::forwarder(
                crate::country_input_changes::Forwarder::AdvisorPool,
                false,
            );
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        PATCHED.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        observe();
        crate::roster_changes::set_manager_forward(enabled && !self.hooks.is_empty());
        crate::country_input_changes::forwarder(
            crate::country_input_changes::Forwarder::AdvisorPool,
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

fn observe() {
    WATCHES
        .registry
        .set_enabled(PATCHED.load(Ordering::SeqCst) && UNAPPOINT_FORWARDER.load(Ordering::SeqCst));
}

pub fn unappoint_forwarder(enabled: bool) {
    UNAPPOINT_FORWARDER.store(enabled, Ordering::SeqCst);
    observe();
}

pub fn forwarded_unappoint(pool: usize) -> Notification<'static> {
    WATCHES.notify(receiver(MUTATIONS[UNAPPOINT].1, pool), UNAPPOINT)
}

macro_rules! callback {
    ($name:ident, $site:expr, ($first:ident $(, $arg:ident)*)) => {
        unsafe extern "system" fn $name($first: usize $(, $arg: usize)*) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::AdvisorPoolChanges);
            let _notification = WATCHES.notify(receiver(MUTATIONS[$site].1, $first), $site);
            let _inputs = crate::country_input_changes::forwarded_pool($site);
            let original: unsafe extern "system" fn($first: usize $(, $arg: usize)*) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original($first $(, $arg)*) })
        }
    };
}
callback!(update_advisor_status, 0, (pool, record));
callback!(add_advisor, 1, (pool, key, template));
callback!(retire_character, 2, (pool, character, record, flags));
callback!(transfer_characters, 3, (pool, list, party));
callback!(remove_advisor_role, 4, (effect, scope));
callback!(construct_pool, 5, (pool, country));
callback!(destroy_pool, 6, (pool));
callback!(
    update_advisor_tokens,
    7,
    (manager, character, erase, insert)
);
callback!(clear_character_manager, 8, (manager, flags));
unsafe extern "system" fn construct_character_manager(manager: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::AdvisorPoolChanges);
    let _roster = crate::roster_changes::manager_constructed();
    let _notification = WATCHES.notify(receiver(MUTATIONS[9].1, manager), 9);
    let original: unsafe extern "system" fn(usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[9].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(manager) })
}
callback!(destroy_character_manager, 10, (manager, flags));
callback!(insert_appointed_advisor, 11, (pool, advisor));
callback!(register_character, 12, (pool, character, flag));
callback!(copy_retired_characters, 13, (pool, source));
