use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::{script_flag_mutations::MUTATIONS, script_flag_mutations::manager};
use engine_native_memory::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static READS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
static SCOPES: AtomicUsize = AtomicUsize::new(0);
static EMBEDDED_SCOPES: AtomicUsize = AtomicUsize::new(0);

pub struct Watches {
    registry: WatchRegistry,
    structure: WatchRegistry,
    calls: crate::hook_timing::CallCounters<SITES>,
    #[cfg(any(test, feature = "test-support"))]
    roster: WatchRegistry,
}

impl Default for Watches {
    fn default() -> Self {
        Self::new()
    }
}

impl Watches {
    pub const fn new() -> Self {
        Self {
            registry: WatchRegistry::marking(),
            structure: WatchRegistry::new(),
            calls: crate::hook_timing::CallCounters::new(),
            #[cfg(any(test, feature = "test-support"))]
            roster: WatchRegistry::new(),
        }
    }

    pub fn notify(&self, manager: usize, site: usize) -> Notifications<'_> {
        self.calls.increment(site);
        Notifications(
            self.registry.notify(manager),
            STRUCTURE_SITES
                .contains(&site)
                .then(|| self.structure.notify(0)),
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(managers: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(managers);
        watches.registry.boundary(false);
        watches.registry.set_enabled(true);
        watches.structure.set_enabled(true);
        watches.roster.set_enabled(true);
        watches
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
        self.structure.boundary(invalidate);
        self.roster.boundary(invalidate);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn roster_write(&self) {
        drop(self.roster.notify(0));
    }
}

pub trait Changes: Sync {
    fn sequence(&self, manager: usize) -> Option<(u64, u64)>;
    fn structure(&self) -> Option<(u64, u64)>;
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64>;
    fn roster(&self) -> Option<(u64, u64)>;
}

impl Changes for Watches {
    fn sequence(&self, manager: usize) -> Option<(u64, u64)> {
        self.registry.sequence(manager)
    }
    fn structure(&self) -> Option<(u64, u64)> {
        self.structure.quiet()
    }
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        self.structure.quiet()?;
        self.registry.take(visit)
    }
    #[cfg(any(test, feature = "test-support"))]
    fn roster(&self) -> Option<(u64, u64)> {
        self.roster.quiet()
    }
    #[cfg(not(any(test, feature = "test-support")))]
    fn roster(&self) -> Option<(u64, u64)> {
        crate::roster_changes::roster()
    }
}

pub struct Observer;
impl Changes for Observer {
    fn sequence(&self, manager: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(manager)
    }
    fn structure(&self) -> Option<(u64, u64)> {
        WATCHES.structure()
    }
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        WATCHES.take(visit)
    }
    fn roster(&self) -> Option<(u64, u64)> {
        crate::roster_changes::roster()
    }
}

const STRUCTURE_SITES: [usize; 6] = [2, 5, 6, 7, 8, 9];

pub struct Notifications<'a>(
    #[expect(dead_code, reason = "held only so its drop delivers the notification")]
    Notification<'a>,
    #[expect(dead_code, reason = "held only so its drop delivers the notification")]
    Option<Notification<'a>>,
);

pub fn active() -> usize {
    WATCHES.registry.active() + WATCHES.structure.active()
}

#[cfg(test)]
pub(crate) fn capacity() -> usize {
    WATCHES.registry.capacity()
}

#[cfg(test)]
pub(crate) fn test_globals() -> std::sync::MutexGuard<'static, ()> {
    static TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = TEST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    unsafe {
        WATCHES.registry.release();
        WATCHES.structure.release();
    }
    WATCHES.registry.set_enabled(false);
    WATCHES.structure.set_enabled(false);
    SCOPES.store(0, Ordering::Relaxed);
    EMBEDDED_SCOPES.store(0, Ordering::Relaxed);
    for read in &READS {
        read.store(0, Ordering::Relaxed);
    }
    guard
}

pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
    WATCHES.structure.boundary(invalidate);
}

pub(crate) fn offer_scopes(scopes: usize) {
    SCOPES.fetch_max(scopes, Ordering::Relaxed);
    presize();
}

pub fn offer_embedded(scopes: usize) {
    EMBEDDED_SCOPES.fetch_max(scopes, Ordering::Relaxed);
    presize();
}

pub fn presize() {
    let Some(scopes) = SCOPES
        .load(Ordering::Relaxed)
        .checked_add(EMBEDDED_SCOPES.load(Ordering::Relaxed))
    else {
        let _ = engine_allocation::oom();
        return;
    };
    if scopes != 0 {
        WATCHES.registry.offer(scopes);
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
    pub quiet: bool,
}

pub fn record(counts: &Counts, scopes: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.reused_rows,
        counts.read_rows,
        u64::from(counts.quiet),
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if let Some(scopes) = scopes {
        SCOPES.store(scopes, Ordering::Relaxed);
    }
}


pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A withheld site leaves the family uninstalled; ScriptFlags then
    /// reads every manager afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "flag_changes")
            && MUTATIONS
                .iter()
                .all(|(site, _)| crate::located(base, site.rva).is_ok())
        {
            let callbacks: [usize; SITES] = [
                set_flag as *const () as usize,
                set_playthrough_flag as *const () as usize,
                read_flags as *const () as usize,
                daily_flags as *const () as usize,
                clear_flag as *const () as usize,
                clear_flags as *const () as usize,
                assign_flags as *const () as usize,
                construct_flags as *const () as usize,
                release_flags as *const () as usize,
                destroy_flags as *const () as usize,
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
            WATCHES.structure.set_enabled(false);
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        WATCHES
            .registry
            .set_enabled(enabled && !self.hooks.is_empty());
        WATCHES
            .structure
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
        unsafe {
            WATCHES.registry.release();
            WATCHES.structure.release();
        }
    }
}

macro_rules! callback {
    ($name:ident, $site:expr, ($first:ident $(, $arg:ident)*)) => {
        unsafe extern "system" fn $name($first: usize $(, $arg: usize)*) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::FlagChanges);
            let _notification = WATCHES.notify(manager(MUTATIONS[$site].1, $first), $site);
            let original: unsafe extern "system" fn($first: usize $(, $arg: usize)*) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original($first $(, $arg)*) })
        }
    };
}
callback!(set_flag, 0, (manager_, token, value, duration));
callback!(set_playthrough_flag, 1, (manager_, token, value, duration));
callback!(read_flags, 2, (manager_, reader));
unsafe extern "system" fn daily_flags(manager_: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::FlagChanges);
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let _notification = engine_scripts::cflag_manager::daily_expiry_writes(&memory, manager_)
        .then(|| WATCHES.notify(manager(MUTATIONS[3].1, manager_), 3));
    let original: unsafe extern "system" fn(usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[3].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(manager_) })
}
callback!(clear_flag, 4, (manager_, token));
callback!(clear_flags, 5, (manager_));
callback!(assign_flags, 6, (rows_, source, count));
callback!(construct_flags, 7, (manager_));
callback!(release_flags, 8, (manager_));
callback!(destroy_flags, 9, (manager_, flags));
