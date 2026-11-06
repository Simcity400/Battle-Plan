//! Allocation-free notifications from the native writers of each country's
//! available- and appointed-advisor arrays and character records and of the
//! character manager's advisor idea-token set
//! ([`engine_mutations::advisor_pool_mutations`]). A detour only notifies the shared
//! watch registry ([`WatchRegistry`]) for the object its first argument
//! names, counts its site and calls the original; it never changes game
//! behaviour. remove_advisor_role names no pool, so it revokes
//! every certificate. A carried AvailableAdvisors row also needs its
//! character's certificate from [`crate::character_changes`], whose sites
//! cover every write of a listed `CAdvisor`. Anything unobserved (disabled,
//! poisoned, a refused registration, an epoch change, a running writer)
//! makes the capture read afresh.
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
/// The unappointment helper, the last site: country_input_changes
/// patches it and forwards each call ([`forwarded_unappoint`]); this family
/// patches the sites before it.
const UNAPPOINT: usize = SITES - 1;
/// This family's own patches are enabled.
static PATCHED: AtomicBool = AtomicBool::new(false);
/// country_input_changes' patch of the unappointment helper is enabled.
static UNAPPOINT_FORWARDER: AtomicBool = AtomicBool::new(false);
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Per table (AvailableAdvisors pools, AdvisorTokens tables, HiredAdvisors
/// and character record hours): reused and read; reused and read rows.
static READS: [[AtomicU64; 4]; 4] = [const { [const { AtomicU64::new(0) }; 4] }; 4];
/// Pools the latest complete capture walked, for off-thread sizing.
static POOLS: AtomicUsize = AtomicUsize::new(0);

/// The carried tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    AvailableAdvisors,
    AdvisorTokens,
    /// Counted per hour: a quiet hour is reused, any other read.
    HiredAdvisors,
    /// CountryCharacters and CountryRetiredCharacters, counted per hour as
    /// HiredAdvisors.
    CharacterRecords,
}
const TABLES: [&str; 4] = [
    "AvailableAdvisors",
    "AdvisorTokens",
    "HiredAdvisors",
    "CharacterRecords",
];

/// One observation domain: the watched pools and token sets and each writer
/// site's calls.
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

    /// Hook path, allocation-free: a writer of `site` ([`MUTATIONS`] index)
    /// is about to change `object`; `None` names none and revokes every
    /// certificate. Dropping the guard notifies its return.
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

    /// An enabled domain with a table for `objects` receivers installed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(objects: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(objects);
        watches.registry.boundary(false);
        watches.registry.set_enabled(true);
        watches
    }

    /// The hourly boundary; `invalidate` starts an epoch.
    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }
}

/// What a capture sees of the notifications: this family's objects and the
/// characters of listed advisors.
pub trait Changes: Sync {
    /// `(epoch, sequence)` of a pool or token set, registering it; `None`
    /// when it cannot be observed.
    fn sequence(&self, object: usize) -> Option<(u64, u64)>;
    /// `(epoch, sequence)` of a character in the character domain,
    /// registering it; `None` when it cannot be observed.
    fn character(&self, character: usize) -> Option<(u64, u64)>;
    /// The character domain's `(epoch, revision)` with no writer running;
    /// equal values around a read prove no character writer ran during it.
    fn characters_quiet(&self) -> Option<(u64, u64)>;
}

/// The hooks' domains.
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

/// A test domain over its own pool and character watches.
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

/// The domain's quiet token: `(epoch, revision)` while observed with no
/// writer running. Every notification of any pool, appointed list or token
/// set moves it, so equal tokens prove no observed writer ran between them.
pub fn quiet() -> Option<(u64, u64)> {
    WATCHES.registry.quiet()
}

/// Notifications running now.
pub fn active() -> usize {
    WATCHES.registry.active()
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
}

/// Off the game thread: size the watches for the pools the latest capture
/// walked and the token set. Nothing is offered before a capture walked
/// them: a later larger table waits for an invalidating hour, while the
/// first installs at once.
pub fn presize() {
    let pools = POOLS.load(Ordering::Relaxed);
    if pools != 0 {
        WATCHES.registry.offer(pools + 1);
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
}

/// One capture's pools or token tables, and rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub reused_rows: u64,
    pub read_rows: u64,
}

/// A worker publishes one capture's counts and, when it walked every pool,
/// their number for sizing.
pub fn record(table: Table, counts: &Counts, pools: Option<usize>) {
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

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(&WATCHES.calls)
            .map(|((site, _), n)| (site.name, n.load(Ordering::Relaxed))),
    );
    let tables = crate::status::map(TABLES.iter().zip(&READS).map(|(&name, reads)| {
        let reads = reads.each_ref().map(|value| value.load(Ordering::Relaxed));
        (
            name,
            crate::status::status!({
                "reused": reads[0],
                "read": reads[1],
                "reused_rows": reads[2],
                "read_rows": reads[3],
            }),
        )
    }));
    crate::status::status!({
        "configured": cfg!(feature = "advisor_pool_changes"),
        "poisoned": WATCHES.registry.poisoned(),
        "tables": tables,
        "events": events,
        "unnamed_receivers": WATCHES.unnamed.load(Ordering::Relaxed),
        "watches": WATCHES.registry.status(),
        "scope": "cumulative since attach; HiredAdvisors and CharacterRecords (CountryCharacters with CountryRetiredCharacters) count hours: a quiet hour (reused) publishes the latest complete walk's rows without reading any country when this domain's token (every pool, appointed list, character record and token set writer), the character domain's and the roster's tokens and the country and link array headers equal those taken before that walk; every other hour (read) walks every country; AvailableAdvisors counts pools (one per country), each read once a day at its country's owner slice: a pool's rows are carried when its CCountryCharacters, country and array header are those of the held read a day earlier, no hooked writer touched it since and every listed advisor's character certificate is unchanged; AdvisorTokens counts token tables, read every hour: the table is carried when its manager and header are those of the held read and no hooked writer touched it since; the slice's countries, pool objects and headers and the token manager are read at every read"
    })
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
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "advisor_pool_changes")
            && MUTATIONS[..UNAPPOINT].iter().all(|(site, _)| {
                crate::hook::installation::sentinel(memory, base + site.rva, site.sentinel)
            })
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
                let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
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
        // One census and suspension for the whole family, shared with the
        // other families in a runtime round (`hook::shared`): a thread at any
        // of its sites, or in a tap, holds every site back until the next
        // attempt.
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        PATCHED.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        observe();
        // The roster family observes the manager constructor through this
        // patch.
        crate::roster_changes::set_manager_forward(enabled && !self.hooks.is_empty());
        // The country counter inputs observe the pool constructor and destructor, which replace a
        // country's characters pointer, through this patch.
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

/// Observe while this family's patches and the unappointment forwarder's are
/// enabled. Enabling starts a new epoch, so nothing held from before a gap
/// certifies.
fn observe() {
    WATCHES
        .registry
        .set_enabled(PATCHED.load(Ordering::SeqCst) && UNAPPOINT_FORWARDER.load(Ordering::SeqCst));
}

/// country_input_changes' unappointment patch enabled or disabled; outside
/// attach and detach only. It disables before its patch goes and enables
/// after it is in place.
pub fn unappoint_forwarder(enabled: bool) {
    UNAPPOINT_FORWARDER.store(enabled, Ordering::SeqCst);
    observe();
}

/// Hook path, allocation-free: the forwarded unappointment helper
/// `unappoint advisor(pool, advisor, flag)` is about to change `pool`'s appointed
/// advisors. The patching family's detour holds the guard across the
/// original.
pub fn forwarded_unappoint(pool: usize) -> Notification<'static> {
    WATCHES.notify(receiver(MUTATIONS[UNAPPOINT].1, pool), UNAPPOINT)
}

// Each site takes at most four integer or pointer arguments (RCX..R9) and
// returns RAX unchanged from its original; the first names the object as
// its [`Receiver`] says. The notification covers the original in flight.
macro_rules! callback {
    ($name:ident, $site:expr, ($first:ident $(, $arg:ident)*)) => {
        unsafe extern "system" fn $name($first: usize $(, $arg: usize)*) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::AdvisorPoolChanges);
            let _notification = WATCHES.notify(receiver(MUTATIONS[$site].1, $first), $site);
            // The pool constructor and destructor also replace a country's The characters pointer,
            // which the country counter inputs observe.
            let _inputs = crate::country_input_changes::forwarded_pool($site);
            let original: unsafe extern "system" fn($first: usize $(, $arg: usize)*) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original($first $(, $arg)*) })
        }
    };
}
// Update advisor status (pool, status record).
callback!(update_advisor_status, 0, (pool, record));
// Add advisor (pool, character key, template).
callback!(add_advisor, 1, (pool, key, template));
// Retire character (pool, character, record, flags) -> bool.
callback!(retire_character, 2, (pool, character, record, flags));
// Transfer characters (source pool, list, destination party).
callback!(transfer_characters, 3, (pool, list, party));
// Remove advisor role (effect, scope): names no pool.
callback!(remove_advisor_role, 4, (effect, scope));
// Country character pool construction(pool, country) -> pool.
callback!(construct_pool, 5, (pool, country));
// Country character pool destruction(pool).
callback!(destroy_pool, 6, (pool));
// Update advisor tokens (manager, character, erase list, insert list).
callback!(
    update_advisor_tokens,
    7,
    (manager, character, erase, insert)
);
// Clear character manager (manager, flags).
callback!(clear_character_manager, 8, (manager, flags));
// Character manager construction(manager) -> manager. It also
// builds a new roster, which the roster family observes through this patch.
unsafe extern "system" fn construct_character_manager(manager: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::AdvisorPoolChanges);
    let _roster = crate::roster_changes::manager_constructed();
    let _notification = WATCHES.notify(receiver(MUTATIONS[9].1, manager), 9);
    let original: unsafe extern "system" fn(usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[9].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(manager) })
}
// Character manager destruction(manager, flags) -> manager.
callback!(destroy_character_manager, 10, (manager, flags));
// Insert appointed advisor (pool, advisor).
callback!(insert_appointed_advisor, 11, (pool, advisor));
// Character registration (pool, character, flag).
callback!(register_character, 12, (pool, character, flag));
// Civil-war retired-record copy (destination pool, source pool).
callback!(copy_retired_characters, 13, (pool, source));
