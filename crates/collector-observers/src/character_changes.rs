use crate::watch_registry::{Notification, WatchRegistry};
use engine_mutations::character_role_mutations::MUTATIONS;
use engine_native::ccharacter;
use std::sync::atomic::{AtomicU64, Ordering};

static WATCHES: Watches = Watches::new();
static READS: [[AtomicU64; 3]; TABLES.len()] =
    [const { [const { AtomicU64::new(0) }; 3] }; TABLES.len()];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    CharacterRoleTraits,
    CountryLeaderRoles,
    Characters,
    AdvisorRoles,
}
const TABLES: [&str; 4] = [
    "CharacterRoleTraits",
    "CountryLeaderRoles",
    "Characters",
    "AdvisorRoles",
];

pub struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; MUTATIONS.len()],
    advisor_calls: AtomicU64,
    unnamed: AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    scientists: WatchRegistry,
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
            calls: [const { AtomicU64::new(0) }; MUTATIONS.len()],
            advisor_calls: AtomicU64::new(0),
            unnamed: AtomicU64::new(0),
            #[cfg(any(test, feature = "test-support"))]
            scientists: WatchRegistry::new(),
        }
    }

    pub fn notify(&self, character: Option<usize>, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.receiver(character)
    }

    pub fn quiet(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }

    fn receiver(&self, character: Option<usize>) -> Notification<'_> {
        match character {
            Some(character) => self.registry.notify(character),
            None => {
                self.unnamed.fetch_add(1, Ordering::Relaxed);
                self.registry.notify_every()
            }
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(characters: usize) -> Self {
        let watches = Self::new();
        watches.enable(characters);
        watches
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enable(&self, characters: usize) {
        self.registry.offer(characters);
        self.registry.boundary(false);
        self.registry.set_enabled(true);
        self.scientists.set_enabled(true);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
        self.scientists.boundary(invalidate);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn scientist_write(&self) {
        drop(self.scientists.notify(0));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn scientists_poisoned(&self) -> bool {
        self.scientists.poisoned()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }
}

pub trait Changes: Sync {
    fn quiet(&self) -> Option<(u64, u64)>;
    fn sequence(&self, character: usize) -> Option<(u64, u64)>;
    fn scientists(&self) -> Option<(u64, u64)>;
}

impl Changes for Watches {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }
    fn sequence(&self, character: usize) -> Option<(u64, u64)> {
        self.registry.sequence(character)
    }
    #[cfg(any(test, feature = "test-support"))]
    fn scientists(&self) -> Option<(u64, u64)> {
        self.scientists.quiet()
    }
    #[cfg(not(any(test, feature = "test-support")))]
    fn scientists(&self) -> Option<(u64, u64)> {
        crate::roster_changes::scientists()
    }
}

#[cfg(any(test, feature = "test-support"))]
pub struct Counting {
    pub watches: Watches,
    lookups: AtomicU64,
}

#[cfg(any(test, feature = "test-support"))]
impl Default for Counting {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Counting {
    pub const fn new() -> Self {
        Self {
            watches: Watches::new(),
            lookups: AtomicU64::new(0),
        }
    }

    pub fn lookups(&self) -> u64 {
        self.lookups.swap(0, Ordering::Relaxed)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Changes for Counting {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.watches.quiet()
    }
    fn sequence(&self, character: usize) -> Option<(u64, u64)> {
        self.lookups.fetch_add(1, Ordering::Relaxed);
        self.watches.sequence(character)
    }
    fn scientists(&self) -> Option<(u64, u64)> {
        self.watches.scientists()
    }
}

pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        WATCHES.registry.quiet()
    }
    fn sequence(&self, character: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(character)
    }
    fn scientists(&self) -> Option<(u64, u64)> {
        crate::roster_changes::scientists()
    }
}

pub fn quiet() -> Option<(u64, u64)> {
    WATCHES.quiet()
}

pub fn notify(character: Option<usize>, site: usize) -> Notification<'static> {
    WATCHES.notify(character, site)
}

pub fn advisor_map(map: usize) -> Notification<'static> {
    WATCHES.advisor_calls.fetch_add(1, Ordering::Relaxed);
    if map == 0 {
        return WATCHES.receiver(None);
    }
    WATCHES
        .registry
        .notify(map.saturating_sub(ccharacter::ADVISOR_MAP_HEAD))
}

pub fn role_character(role: usize, offset: usize) -> Option<usize> {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    role.checked_add(offset)
        .and_then(|address| engine_native_memory::read::<u64>(&memory, address))
        .and_then(|character| usize::try_from(character).ok())
        .filter(|&character| character != 0)
}

pub fn active() -> usize {
    WATCHES.registry.active()
}

pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
}

pub fn presize(characters: usize) {
    if characters != 0 {
        WATCHES.registry.offer(characters);
    }
}

pub fn collect() {
    WATCHES.registry.collect();
}

pub fn set_enabled(enabled: bool) {
    WATCHES.registry.set_enabled(enabled);
}

/// Free the watch tables.
///
/// # Safety
/// The hooks were never enabled, or were removed and drained after the last
/// capture.
pub unsafe fn release() {
    unsafe { WATCHES.registry.release() };
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub quiet: bool,
}

static OPERATIVES: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

pub fn record_operatives(carried: u64, read: u64) {
    for (target, value) in OPERATIVES.iter().zip([carried, read]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
}

pub fn record(table: Table, counts: &Counts) {
    for (target, value) in READS[table as usize].iter().zip([
        counts.reused,
        counts.read,
        u64::from(counts.quiet),
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
}

