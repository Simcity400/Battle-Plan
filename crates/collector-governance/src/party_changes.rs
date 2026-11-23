use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::political_party_mutations::MUTATIONS;
use engine_native_memory::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static READS: [[AtomicU64; 4]; 2] = [const { [const { AtomicU64::new(0) }; 4] }; 2];
static RECEIVERS: AtomicUsize = AtomicUsize::new(0);

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

    pub fn notify(&self, party: usize, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.registry.notify(party)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(receivers: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(receivers);
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
    fn quiet(&self) -> Option<(u64, u64)>;
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)>;
}

impl Changes for Watches {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        self.registry.sequence(receiver)
    }
}

pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        quiet()
    }
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(receiver)
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

pub fn presize() {
    let receivers = RECEIVERS.load(Ordering::Relaxed);
    if receivers != 0 {
        WATCHES.registry.offer(receivers);
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

pub fn record(counts: &[Counts; 2], receivers: Option<usize>) {
    for (table, counts) in counts.iter().enumerate() {
        for (target, value) in READS[table].iter().zip([
            counts.reused,
            counts.read,
            counts.reused_rows,
            counts.read_rows,
        ]) {
            target.fetch_add(value, Ordering::Relaxed);
        }
    }
    if let Some(receivers) = receivers {
        RECEIVERS.store(receivers, Ordering::Relaxed);
    }
}


pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A withheld site leaves the family uninstalled; PoliticalParties
    /// then reads every party afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "party_changes")
            && MUTATIONS
                .iter()
                .all(|site| crate::located(base, site.rva).is_ok())
        {
            let callbacks: [usize; SITES] = [
                party_names as *const () as usize,
                construct_party as *const () as usize,
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

macro_rules! callback {
    ($name:ident, $site:expr, ($party:ident $(, $arg:ident)*)) => {
        unsafe extern "system" fn $name($party: usize $(, $arg: usize)*) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::PartyChanges);
            let _notification = WATCHES.notify($party, $site);
            let original: unsafe extern "system" fn($party: usize $(, $arg: usize)*) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original($party $(, $arg)*) })
        }
    };
}
callback!(party_names, 0, (party, long_name, name));
callback!(construct_party, 1, (party, link, ideology));
