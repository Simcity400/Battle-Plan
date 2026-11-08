//! Allocation-free notifications from the native writers of every scripted
//! flag manager ([`engine_mutations::script_flag_mutations`]). A detour only notifies
//! the shared watch registry ([`WatchRegistry`]) for the manager its first
//! argument names, counts its site and calls the original; it never changes
//! game behaviour. Receivers are the `CFlagManager` addresses a ScriptFlags
//! capture registered; a notification never infers a scope. Anything
//! unobserved (disabled, poisoned, a refused registration, an epoch change, a
//! running writer) makes the capture read that manager afresh.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::{script_flag_mutations::MUTATIONS, script_flag_mutations::manager};
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Reused and read managers; reused and read rows; quiet hours.
static READS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
/// Scopes observed by the complete sizing census or capture, for off-thread sizing.
static SCOPES: AtomicUsize = AtomicUsize::new(0);
static EMBEDDED_SCOPES: AtomicUsize = AtomicUsize::new(0);

/// One observation domain: the watched managers, which every writer marks,
/// the structure token, and each writer site's calls.
pub struct Watches {
    registry: WatchRegistry,
    /// Moved by the [`STRUCTURE_SITES`]: the writers that build, release,
    /// destroy, clear or replace a whole manager, which every writer of a
    /// scope root calls in the same call.
    structure: WatchRegistry,
    calls: [AtomicU64; SITES],
    /// Tests: stands in for the roster domain ([`crate::roster_changes`]).
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
            calls: [const { AtomicU64::new(0) }; SITES],
            #[cfg(any(test, feature = "test-support"))]
            roster: WatchRegistry::new(),
        }
    }

    /// Hook path, allocation-free: a writer of `site` ([`MUTATIONS`] index)
    /// is about to change `manager`, which it marks; a [`STRUCTURE_SITES`]
    /// writer also moves the structure token. Dropping the guards notifies
    /// its return.
    pub fn notify(&self, manager: usize, site: usize) -> Notifications<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        Notifications(
            self.registry.notify(manager),
            STRUCTURE_SITES
                .contains(&site)
                .then(|| self.structure.notify(0)),
        )
    }

    /// An enabled domain with a table for `managers` receivers installed.
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

    /// The hourly boundary; `invalidate` starts an epoch.
    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
        self.structure.boundary(invalidate);
        self.roster.boundary(invalidate);
    }

    /// Tests: a roster writer runs and returns.
    #[cfg(any(test, feature = "test-support"))]
    pub fn roster_write(&self) {
        drop(self.roster.notify(0));
    }
}

/// What a capture sees of the notifications.
pub trait Changes: Sync {
    /// `(epoch, sequence)` of one manager, registering it; `None` when it
    /// cannot be observed. Equal values across hours prove that no observed
    /// writer touched it.
    fn sequence(&self, manager: usize) -> Option<(u64, u64)>;
    /// `(epoch, revision)` of the structure token while it is observed and
    /// no writer runs: equal values across hours prove that no flag manager
    /// was built, released, destroyed, cleared or replaced whole in between,
    /// and with them that no global, country or state scope root changed
    /// (every root writer builds, destroys or clears a manager in the same
    /// call).
    fn structure(&self) -> Option<(u64, u64)>;
    /// The single consumer's hourly take of the managers a writer marked
    /// ([`WatchRegistry::take`]); `None` certifies nothing.
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64>;
    /// The character roster's token ([`crate::roster_changes::roster`]):
    /// equal values prove the roster's occurrences, order and ids unchanged.
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
        // A running writer withholds the token and the take alike.
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

/// The hooks' domain.
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

/// The writers that build, release, destroy, clear or replace a whole
/// Manager  ([`MUTATIONS`] indices: the save reader, clear flags, the row
/// array assignment, the constructor, the embedded release and the
/// destructor). Every writer of a scope root calls one of them in the same
/// call, so they move the structure token besides marking their manager.
const STRUCTURE_SITES: [usize; 6] = [2, 5, 6, 7, 8, 9];

/// A writer's notifications: its manager's mark and, for a
/// [`STRUCTURE_SITES`] writer, the structure token's.
pub struct Notifications<'a>(
    #[allow(dead_code)] Notification<'a>,
    #[allow(dead_code)] Option<Notification<'a>>,
);

/// Notifications running now.
pub fn active() -> usize {
    WATCHES.registry.active() + WATCHES.structure.active()
}

/// Slots of the installed manager table.
#[cfg(test)]
pub(crate) fn capacity() -> usize {
    WATCHES.registry.capacity()
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
    WATCHES.structure.boundary(invalidate);
}

/// The existing pre-install sizing walk observed every ScriptFlags scope,
/// including empty managers. Replace an embedded-only offer before the
/// first boundary installs it: later growth waits for an invalidating hour.
pub(crate) fn offer_scopes(scopes: usize) {
    SCOPES.fetch_max(scopes, Ordering::Relaxed);
    presize();
}

/// Off the game thread: size watches from the complete scope census plus
/// embedded stores. A later larger table waits for an invalidating hour,
/// while the first installs at once. Existing daily MIO sizing contributes
/// embedded stores without an hourly scan.
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

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
}

/// One capture's managers and rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub reused_rows: u64,
    pub read_rows: u64,
    /// The walk published the held rows of a quiet hour without walking a
    /// scope.
    pub quiet: bool,
}

/// A worker publishes one capture's counts and, when it walked every scope,
/// their number for sizing.
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
        // One census and suspension for the whole family, shared with the
        // other families in a runtime round (`hook::shared`): a thread at any
        // of its sites, or in a tap, holds every site back until the next
        // attempt.
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

// Each site takes at most four integer or pointer arguments (RCX..R9) and
// returns RAX unchanged from its original; the first names the manager as
// its [`engine_mutations::script_flag_mutations::Receiver`] says. The notification
// covers the original in flight.
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
// Set flag (manager, token, value, duration).
callback!(set_flag, 0, (manager_, token, value, duration));
// Set playthrough flag (manager, token, value, duration).
callback!(set_playthrough_flag, 1, (manager_, token, value, duration));
// Read flags (manager, reader).
callback!(read_flags, 2, (manager_, reader));
// Daily flags (manager): every scope's daily update calls it, but it writes only
// a manager holding a positive remaining duration
// ([`engine_scripts::cflag_manager::daily_expiry_writes`]), so only such a call
// notifies; the others leave the quiet token standing.
unsafe extern "system" fn daily_flags(manager_: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::FlagChanges);
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let _notification = engine_scripts::cflag_manager::daily_expiry_writes(&memory, manager_)
        .then(|| WATCHES.notify(manager(MUTATIONS[3].1, manager_), 3));
    let original: unsafe extern "system" fn(usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[3].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(manager_) })
}
// Clear flag (manager, token).
callback!(clear_flag, 4, (manager_, token));
// Clear flags (manager).
callback!(clear_flags, 5, (manager_));
// Assign flags (the manager's row array, rows, count).
callback!(assign_flags, 6, (rows_, source, count));
// Construct flags (manager).
callback!(construct_flags, 7, (manager_));
// Release flags (manager).
callback!(release_flags, 8, (manager_));
// Destroy flags (manager, flags).
callback!(destroy_flags, 9, (manager_, flags));
