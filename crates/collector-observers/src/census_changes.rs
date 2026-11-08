//! The role census domain: a quiet token over every native writer of the
//! fields the roster job gathers into its role census (each rostered
//! character's country and role span, [`crate::extended::characters::census`]).
//! The domain registers no receiver: the roster job compares the token it
//! took before its latest complete gather with the hour's, and equal tokens
//! prove that no observed writer ran in between ([`WatchRegistry::quiet`]).
//!
//! This family patches the writers no other family does
//! ([`engine_mutations::census_mutations`]); the advisor family forwards its map,
//! parser, merge and destructor sites and the country-leader map insert, and
//! the roster family its scientist attach and clear ([`forwarded`]). The
//! roster array writers move the roster domain's token, which a census carry
//! also requires. The domain observes only while this family's patches and
//! both forwarders are enabled; anything unobserved (disabled, poisoned, an
//! epoch change, a running writer) yields no token, and the census is
//! gathered.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::census_mutations::MUTATIONS;
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static CENSUS: WatchRegistry = WatchRegistry::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// This family's patches, then each forwarding family's, are enabled.
static ENABLED: [AtomicBool; 1 + Forwarder::ALL.len()] =
    [const { AtomicBool::new(false) }; 1 + Forwarder::ALL.len()];
static CALLS: [AtomicU64; SITES] = [const { AtomicU64::new(0) }; SITES];
static FORWARDED: [AtomicU64; Site::ALL.len()] = [const { AtomicU64::new(0) }; Site::ALL.len()];
/// Carried and gathered censuses.
static READS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

/// The families whose patches forward here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forwarder {
    /// [`crate::advisor_changes`].
    Advisors,
    /// [`crate::roster_changes`].
    Roster,
}

impl Forwarder {
    const ALL: [Self; 2] = [Self::Advisors, Self::Roster];
}

/// The forwarded writers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Site {
    LeaderMapInsert,
    AdvisorMapInsert,
    AdvisorMapMove,
    AdvisorMapErase,
    Merge,
    Parse,
    Destroy,
    ScientistAttach,
    ScientistClear,
}

impl Site {
    const ALL: [Self; 9] = [
        Self::LeaderMapInsert,
        Self::AdvisorMapInsert,
        Self::AdvisorMapMove,
        Self::AdvisorMapErase,
        Self::Merge,
        Self::Parse,
        Self::Destroy,
        Self::ScientistAttach,
        Self::ScientistClear,
    ];

}

/// What the roster job sees of the domain.
pub trait Changes: Sync {
    /// The domain's token: `(epoch, revision)` while observed and no writer
    /// runs.
    fn quiet(&self) -> Option<(u64, u64)>;
}

/// The hooks' domain.
pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        CENSUS.quiet()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Changes for WatchRegistry {
    fn quiet(&self) -> Option<(u64, u64)> {
        WatchRegistry::quiet(self)
    }
}

/// Hook path, allocation-free: a forwarded writer is about to run. Hold the
/// result over the native call.
pub fn forwarded(site: Site) -> Option<Notification<'static>> {
    if !cfg!(feature = "census_changes") {
        return None;
    }
    FORWARDED[site as usize].fetch_add(1, Ordering::Relaxed);
    Some(CENSUS.notify(0))
}

/// A forwarding family's patches were enabled or disabled; outside attach
/// and detach only. It disables before its patches go and enables after
/// they are in place.
pub fn forwarder(family: Forwarder, enabled: bool) {
    ENABLED[1 + family as usize].store(enabled, Ordering::SeqCst);
    observe();
}

/// Observe while this family's patches and every forwarder's are enabled.
/// Enabling starts a new epoch, so nothing held from before a gap certifies.
fn observe() {
    CENSUS.set_enabled(
        cfg!(feature = "census_changes")
            && ENABLED.iter().all(|enabled| enabled.load(Ordering::SeqCst)),
    );
}

/// Notifications running now.
pub fn active() -> usize {
    CENSUS.active()
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    CENSUS.boundary(invalidate);
}

/// One role census acquisition.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// The held census was kept without a gather.
    pub carried: bool,
    /// The whole census was gathered.
    pub gathered: bool,
}

pub fn record(counts: &Counts) {
    for (target, value) in READS
        .iter()
        .zip([u64::from(counts.carried), u64::from(counts.gathered)])
    {
        target.fetch_add(value, Ordering::Relaxed);
    }
}


pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A differing sentinel leaves the family uninstalled; the census is then
    /// gathered every hour.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "census_changes")
            && MUTATIONS
                .iter()
                .all(|site| crate::located(base, site.rva).is_ok())
        {
            let callbacks: [usize; SITES] = [
                set_country as *const () as usize,
                create_army_role as *const () as usize,
                create_navy_role as *const () as usize,
                clear_unit_role as *const () as usize,
                add_country_leader_role as *const () as usize,
                erase_leader_roles as *const () as usize,
                add_scientist_role as *const () as usize,
                set_operative_reference as *const () as usize,
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
            ENABLED[0].store(false, Ordering::SeqCst);
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
        ENABLED[0].store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        observe();
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: the domain holds no table; release only starts an epoch
        // once these hooks and the forwarding ones were removed and drained.
        unsafe { CENSUS.release() };
    }
}

// Every site takes at most three integer or pointer arguments; the detour
// forwards RCX..R9 and returns RAX. The notification covers the original in
// flight.
macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::CensusChanges);
            CALLS[$site].fetch_add(1, Ordering::Relaxed);
            let _notification = CENSUS.notify(0);
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(first, second, third, fourth) })
        }
    };
}
callback!(set_country, 0);
callback!(create_army_role, 1);
callback!(create_navy_role, 2);
callback!(clear_unit_role, 3);
callback!(add_country_leader_role, 4);
callback!(erase_leader_roles, 5);
callback!(add_scientist_role, 6);
callback!(set_operative_reference, 7);

#[cfg(test)]
mod tests {
    use super::*;

    /// The domain needs this family and every forwarder, and each forwarded
    /// writer moves the token at its entry and refuses it until its return.
    #[cfg(feature = "census_changes")]
    #[test]
    fn every_forwarder_enables_and_each_writer_moves_the_token() {
        ENABLED[0].store(true, Ordering::SeqCst);
        for family in Forwarder::ALL {
            forwarder(family, false);
        }
        assert_eq!(Observer.quiet(), None);
        forwarder(Forwarder::Advisors, true);
        assert_eq!(Observer.quiet(), None);
        forwarder(Forwarder::Roster, true);
        let before = Observer.quiet().expect("observed");
        let call = forwarded(Site::ScientistAttach);
        assert_eq!(Observer.quiet(), None, "a running writer certifies nothing");
        drop(call);
        assert_ne!(Observer.quiet(), Some(before));
        forwarder(Forwarder::Roster, false);
        assert_eq!(Observer.quiet(), None);
    }
}
