use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::{party_leader_mutations::MUTATIONS, party_leader_mutations::VECTOR};
use engine_native_memory::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static READS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static PARTIES: AtomicUsize = AtomicUsize::new(0);

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
    pub fn enabled(parties: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(parties);
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
    fn party(&self, party: usize) -> Option<(u64, u64)>;
    fn character(&self, character: usize) -> Option<(u64, u64)>;
}

pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        quiet()
    }
    fn party(&self, party: usize) -> Option<(u64, u64)> {
        WATCHES.registry.sequence(party)
    }
    fn character(&self, character: usize) -> Option<(u64, u64)> {
        crate::character_changes::Changes::sequence(&crate::character_changes::Observer, character)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub struct Pair<'a> {
    pub parties: &'a Watches,
    pub characters: &'a crate::character_changes::Watches,
}
#[cfg(any(test, feature = "test-support"))]
impl Changes for Pair<'_> {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.parties.registry.quiet()
    }
    fn party(&self, party: usize) -> Option<(u64, u64)> {
        self.parties.registry.sequence(party)
    }
    fn character(&self, character: usize) -> Option<(u64, u64)> {
        crate::character_changes::Changes::sequence(self.characters, character)
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
    let parties = PARTIES.load(Ordering::Relaxed);
    if parties != 0 {
        WATCHES.registry.offer(parties);
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

pub fn record(counts: &Counts, parties: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.reused_rows,
        counts.read_rows,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if let Some(parties) = parties {
        PARTIES.store(parties, Ordering::Relaxed);
    }
}


pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A withheld site leaves the family uninstalled; PartyLeaders then
    /// reads every country afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "party_leader_changes")
            && MUTATIONS
                .iter()
                .all(|(site, _)| crate::located(base, site.rva).is_ok())
        {
            let callbacks: [usize; SITES] = [
                insert_leader_front as *const () as usize,
                append_leader as *const () as usize,
                remove_leader as *const () as usize,
                promote_leader as *const () as usize,
                retire_first_leader as *const () as usize,
                kill_first_leader as *const () as usize,
                clear_leaders as *const () as usize,
                transfer_leaders as *const () as usize,
                validate_leaders as *const () as usize,
                read_leaders as *const () as usize,
                destroy_party as *const () as usize,
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
            crate::country_input_changes::forwarder(
                crate::country_input_changes::Forwarder::PartyLeaders,
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
        crate::country_input_changes::forwarder(
            crate::country_input_changes::Forwarder::PartyLeaders,
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

macro_rules! callback {
    ($name:ident, $site:expr, ($($arg:ident),+), [$($party:expr),+]) => {
        unsafe extern "system" fn $name($($arg: usize),+) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::PartyLeaderChanges);
            let _notifications = [$(WATCHES.notify($party, $site)),+];
            let _inputs = [$(crate::country_input_changes::forwarded(
                crate::country_input_changes::Forward::PartyLeaders,
                $party,
                0,
            )),+];
            let original: unsafe extern "system" fn($($arg: usize),+) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original($($arg),+) })
        }
    };
}
callback!(insert_leader_front, 0, (party, leader), [party]);
callback!(append_leader, 1, (party, leader), [party]);
callback!(remove_leader, 2, (party, leader, flag), [party]);
callback!(promote_leader, 3, (party, leader), [party]);
callback!(retire_first_leader, 4, (party), [party]);
callback!(kill_first_leader, 5, (party), [party]);
callback!(clear_leaders, 6, (party), [party]);
callback!(
    transfer_leaders,
    7,
    (source, destination, list, out, first, second),
    [source, destination]
);
callback!(validate_leaders, 8, (party), [party]);
callback!(
    read_leaders,
    9,
    (reader, token, vector),
    [vector.wrapping_sub(VECTOR)]
);
callback!(destroy_party, 10, (party, flags), [party]);
