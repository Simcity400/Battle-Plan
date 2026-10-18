//! Allocation-free notifications from the native writers of the unit leader
//! fields UnitLeaders records ([`engine_mutations::unit_leader_mutations`]). A detour
//! only notifies the shared watch registry ([`WatchRegistry`]) for the
//! `CUnitLeader` its first argument names, or revokes every certificate for
//! the two exile sites that name none, counts the call and calls the
//! original; it never changes game behaviour. The leader constructors and
//! destructor are [`crate::naval_tree_changes`]'s patches, which forward each
//! call ([`forwarded`]), so a reused address never keeps a freed leader's
//! certificate. Receivers are the leaders a quiet roster hour registered.
//! The domain observes only while this family's patches and the forwarder
//! are enabled; anything unobserved (disabled, poisoned, a refused
//! registration, an epoch change, a running writer) makes the capture read
//! that leader afresh.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::{
    unit_leader_mutations::EXILE_GENERATION, unit_leader_mutations::EXILE_REMOVAL,
    unit_leader_mutations::MUTATIONS,
};
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// This family's patches are enabled.
static PATCHED: AtomicBool = AtomicBool::new(false);
/// The naval tree family's leader lifetime patches are enabled.
static FORWARDED: AtomicBool = AtomicBool::new(false);
/// Reused, read, verified and mismatched leaders, and quiet hours.
static READS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "carried UnitLeaders rows differ from a fresh read",
]);
/// Leaders the latest quiet hour visited, for off-thread sizing.
static LEADERS: AtomicUsize = AtomicUsize::new(0);

/// The leader lifetime sites [`crate::naval_tree_changes`] patches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifetime {
    Construct,
    CopyConstruct,
    Destroy,
}
const LIFETIMES: [&str; 3] = ["construct", "copy_construct", "destroy"];

/// One observation domain: the watched leaders and each site's calls.
pub struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; SITES],
    lifetimes: [AtomicU64; LIFETIMES.len()],
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
            lifetimes: [const { AtomicU64::new(0) }; LIFETIMES.len()],
        }
    }

    /// Hook path: the writer of `site` ([`MUTATIONS`] index) is about to
    /// change `leader`; the exile sites name none and revoke every
    /// certificate. Dropping the guard notifies its return.
    pub fn notify(&self, leader: usize, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        if site == EXILE_GENERATION || site == EXILE_REMOVAL {
            self.registry.notify_every()
        } else {
            self.registry.notify(leader)
        }
    }

    /// Hook path: a forwarded lifetime site is about to build or free `leader`.
    pub fn lifetime(&self, leader: usize, site: Lifetime) -> Notification<'_> {
        self.lifetimes[site as usize].fetch_add(1, Ordering::Relaxed);
        self.registry.notify(leader)
    }

    /// An enabled domain with a table for `leaders` receivers installed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn enable(&self, leaders: usize) {
        self.registry.offer(leaders);
        self.registry.boundary(false);
        self.registry.set_enabled(true);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }
}

/// What a capture sees of the notifications.
pub trait Changes: Sync {
    /// `(epoch, revision)` while observing with no writer running; equal
    /// values prove that no observed writer ran in between.
    fn quiet(&self) -> Option<(u64, u64)>;
    /// `(epoch, sequence)` of one leader, registering it; `None` when it
    /// cannot be observed. Equal values across hours prove that no observed
    /// writer touched it.
    fn sequence(&self, leader: usize) -> Option<(u64, u64)>;
    /// A fresh read disagreed with a carried row: disable reuse for the
    /// attachment.
    fn mismatch(&self);
}

impl Changes for Watches {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }
    fn sequence(&self, leader: usize) -> Option<(u64, u64)> {
        self.registry.sequence(leader)
    }
    fn mismatch(&self) {
        self.registry.poison();
    }
}

/// The hooks' domain.
pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        WATCHES.registry.quiet()
    }
    fn sequence(&self, leader: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(leader)
    }
    fn mismatch(&self) {
        WATCHES.mismatch();
    }
}

/// Whether quiet hours read every carryable leader afresh and compare,
/// rather than the hour's sampled slice ([`crate::sampled_verify`]).
pub const VERIFY: bool = cfg!(feature = "unit_leader_verify");

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

/// Off the game thread: size the watches for the leaders the latest quiet
/// hour visited.
pub fn presize() {
    let leaders = LEADERS.load(Ordering::Relaxed);
    if leaders != 0 {
        WATCHES.registry.offer(leaders);
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
}

/// One quiet hour's leaders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub verified: u64,
    pub mismatches: u64,
}

/// The decode worker publishes one quiet hour's counts and the leaders it
/// visited.
pub fn record(counts: &Counts, leaders: usize) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.verified,
        counts.mismatches,
        1,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    SAMPLED.record(counts.verified, counts.mismatches, 0);
    LEADERS.store(leaders, Ordering::Relaxed);
}

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .map(|site| site.name)
            .chain(LIFETIMES)
            .zip(WATCHES.calls.iter().chain(&WATCHES.lifetimes))
            .map(|(name, calls)| (name, calls.load(Ordering::Relaxed))),
    );
    let reads = READS.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "unit_leader_changes"),
        "verify": VERIFY,
        "poisoned": WATCHES.registry.poisoned(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "reused_leaders": reads[0],
        "read_leaders": reads[1],
        "verified_leaders": reads[2],
        "mismatches": reads[3],
        "quiet_hours": reads[4],
        "events": events,
        "watches": WATCHES.registry.status(),
        "scope": "cumulative since attach; on a quiet roster hour a leader's UnitLeaders row is carried, without reading the leader, while the role census names it at the held address for the held character and no hooked writer touched it since the held read (events: each patched site's calls, and the naval tree family's forwarded construct, copy_construct and destroy calls); every other leader is read and its row held; every quiet hour also reads the carryable leaders of its slice afresh (one in rotation_hours by leader id; every one in verify mode), publishes the fresh rows and disables reuse at the first mismatch (sampled_*); hours that read the whole roster read every leader"
    })
}

/// Observe while this family's patches and the forwarder's are enabled.
/// Enabling starts a new epoch, so nothing held from before a gap certifies.
fn observe() {
    WATCHES.registry.set_enabled(
        cfg!(feature = "unit_leader_changes")
            && PATCHED.load(Ordering::SeqCst)
            && FORWARDED.load(Ordering::SeqCst),
    );
}

/// The naval tree family enabled or disabled its leader lifetime patches;
/// outside attach and detach only. It disables before its patches go and
/// enables after they are in place.
pub fn forwarder(enabled: bool) {
    FORWARDED.store(enabled, Ordering::SeqCst);
    observe();
}

/// Hook path, allocation-free: a forwarded lifetime site is about to build or
/// free `leader`. Hold the result over the native call.
pub fn forwarded(leader: usize, site: Lifetime) -> Notification<'static> {
    WATCHES.lifetime(leader, site)
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// A differing sentinel leaves the family uninstalled; quiet hours then
    /// read every leader.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "unit_leader_changes")
            && MUTATIONS.iter().all(|site| {
                engine_native::read::<[u8; 16]>(memory, base + site.rva) == Some(site.sentinel)
            })
        {
            let callbacks: [usize; SITES] = [
                formation as *const () as usize,
                skill as *const () as usize,
                category as *const () as usize,
                army_attack as *const () as usize,
                army_defense as *const () as usize,
                army_planning as *const () as usize,
                army_logistics as *const () as usize,
                navy_attack as *const () as usize,
                navy_defense as *const () as usize,
                navy_maneuvering as *const () as usize,
                navy_coordination as *const () as usize,
                capture as *const () as usize,
                release as *const () as usize,
                owner as *const () as usize,
                operative_owner as *const () as usize,
                exile_generation as *const () as usize,
                exile_removal as *const () as usize,
                set_name as *const () as usize,
                rebuild_derived as *const () as usize,
                operative_rebuild as *const () as usize,
                link_character as *const () as usize,
                operative_codename as *const () as usize,
                operative_admission as *const () as usize,
            ];
            for (index, site) in MUTATIONS.iter().enumerate() {
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
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: like each Hook's relay, the watch tables are dropped only
        // once these hooks and the forwarding naval tree hooks (dropped
        // before these) were never enabled, or were removed and drained after
        // the last capture.
        unsafe { WATCHES.registry.release() };
    }
}

// Every site takes its leader (or, for the exile sites, an unrelated first
// argument) in RCX and at most three more integer or pointer arguments; the
// detour forwards RCX..R9 and returns RAX. The notification covers the
// original in flight.
macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            leader: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::UnitLeaderChanges);
            let _notification = WATCHES.notify(leader, $site);
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(leader, second, third, fourth) })
        }
    };
}
callback!(formation, 0);
callback!(skill, 1);
callback!(category, 2);
callback!(army_attack, 3);
callback!(army_defense, 4);
callback!(army_planning, 5);
callback!(army_logistics, 6);
callback!(navy_attack, 7);
callback!(navy_defense, 8);
callback!(navy_maneuvering, 9);
callback!(navy_coordination, 10);
callback!(capture, 11);
callback!(release, 12);
callback!(owner, 13);
callback!(operative_owner, 14);
callback!(exile_generation, EXILE_GENERATION);
callback!(exile_removal, EXILE_REMOVAL);
callback!(set_name, 17);
callback!(rebuild_derived, 18);
callback!(operative_rebuild, 19);
callback!(link_character, 20);
callback!(operative_codename, 21);
callback!(operative_admission, 22);

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer moves only the leader it names, a forwarded lifetime site
    /// too, and an exile site every leader; each counts its calls; a running
    /// writer certifies nothing, and a mismatch poisons the domain.
    #[test]
    fn writers_move_their_leaders_certificates() {
        let watches = Watches::new();
        watches.enable(16);
        let held = |leader| Changes::sequence(&watches, leader).unwrap();
        let [first, second] = [0x1000, 0x2000].map(held);
        drop(watches.notify(0x2000, 0));
        assert_eq!(held(0x1000), first);
        assert_ne!(held(0x2000), second);
        let second = held(0x2000);
        drop(watches.lifetime(0x1000, Lifetime::Destroy));
        assert_ne!(held(0x1000), first);
        assert_eq!(held(0x2000), second);
        let first = held(0x1000);
        drop(watches.notify(0, EXILE_REMOVAL));
        assert_ne!(held(0x1000), first);
        assert_ne!(held(0x2000), second);
        assert_eq!(watches.calls[0].load(Ordering::Relaxed), 1);
        assert_eq!(watches.calls[EXILE_REMOVAL].load(Ordering::Relaxed), 1);
        assert_eq!(
            watches.lifetimes[Lifetime::Destroy as usize].load(Ordering::Relaxed),
            1
        );
        let running = watches.notify(0x1000, 1);
        assert!(Changes::sequence(&watches, 0x1000).is_none());
        drop(running);
        Changes::mismatch(&watches);
        assert!(watches.poisoned());
        assert!(Changes::sequence(&watches, 0x1000).is_none());
    }
}
