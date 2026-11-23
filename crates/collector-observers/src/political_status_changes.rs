use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::{
    political_status_mutations::Domain, political_status_mutations::MUTATIONS,
    political_status_mutations::Patch, political_status_mutations::SET_POPULARITY_RVA,
};
use engine_native_memory::Memory;
use engine_native::ccountry;
use engine_native_politics::cpolitical_status;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static STRUCTURE: WatchRegistry = WatchRegistry::new();
static POPULARITY: WatchRegistry = WatchRegistry::new();
static CALLS: [AtomicU64; SITES] = [const { AtomicU64::new(0) }; SITES];
static FORWARDED: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static CAPITAL_CHANGES: AtomicU64 = AtomicU64::new(0);
const SET_CAPITAL: usize = 0;
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static SET_POPULARITY: AtomicUsize = AtomicUsize::new(0);
static OWN: AtomicBool = AtomicBool::new(false);
static FORWARD: AtomicBool = AtomicBool::new(false);

static READS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forwarded {
    ResetCountry,
    PoliticsDaily,
}

pub fn structure() -> Option<(u64, u64)> {
    STRUCTURE.quiet()
}

pub fn popularity() -> Option<(u64, u64)> {
    POPULARITY.quiet()
}

pub trait Changes: Sync {
    fn structure(&self) -> Option<(u64, u64)>;
    fn popularity(&self) -> Option<(u64, u64)>;
}

pub struct Observer;
impl Changes for Observer {
    fn structure(&self) -> Option<(u64, u64)> {
        structure()
    }
    fn popularity(&self) -> Option<(u64, u64)> {
        popularity()
    }
}

pub struct Guard<'a> {
    structure_domain: &'a WatchRegistry,
    watched: Option<Watched>,
    _structure: Option<Notification<'a>>,
    _popularity: Option<Notification<'a>>,
}

#[derive(Clone, Copy)]
enum Watched {
    LastElection(usize, i32),
    Capitals(usize, u64),
}

impl Watched {
    fn changed(self) -> bool {
        let (changed, counter) = match self {
            Self::LastElection(status, before) => {
                (last_election(status) != Some(before), &FORWARDED[2])
            }
            Self::Capitals(country, before) => {
                (capitals(country) != Some(before), &CAPITAL_CHANGES)
            }
        };
        if changed {
            counter.fetch_add(1, Ordering::Relaxed);
        }
        changed
    }
}

impl<'a> Guard<'a> {
    fn new(structure: &'a WatchRegistry, popularity: &'a WatchRegistry, domain: Domain) -> Self {
        Self {
            structure_domain: structure,
            watched: None,
            _structure: domain.structure().then(|| structure.notify(0)),
            _popularity: domain.popularity().then(|| popularity.notify(0)),
        }
    }

    fn capital(structure: &'a WatchRegistry, country: usize) -> Self {
        let before = capitals(country);
        Self {
            structure_domain: structure,
            watched: before.map(|before| Watched::Capitals(country, before)),
            _structure: before.is_none().then(|| structure.notify(0)),
            _popularity: None,
        }
    }

    fn daily(structure: &'a WatchRegistry, popularity: &'a WatchRegistry, status: usize) -> Self {
        let before = last_election(status);
        Self {
            structure_domain: structure,
            watched: before.map(|before| Watched::LastElection(status, before)),
            _structure: before.is_none().then(|| structure.notify(0)),
            _popularity: Some(popularity.notify(0)),
        }
    }
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        if self.watched.is_some_and(Watched::changed) {
            drop(self.structure_domain.notify(0));
        }
    }
}

fn last_election(status: usize) -> Option<i32> {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    status
        .checked_add(cpolitical_status::LAST_ELECTION)
        .and_then(|address| engine_native_memory::read::<i32>(&memory, address))
}

fn capitals(country: usize) -> Option<u64> {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    country
        .checked_add(ccountry::CAPITAL)
        .and_then(|address| engine_native_memory::read::<u64>(&memory, address))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub quiet: bool,
    pub popularity: bool,
    pub read: bool,
}

pub fn record(counts: &Counts) {
    for (target, value) in READS.iter().zip([
        u64::from(counts.quiet),
        u64::from(counts.popularity),
        u64::from(counts.read),
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
}

pub fn active() -> usize {
    STRUCTURE.active() + POPULARITY.active()
}

pub fn boundary(invalidate: bool) {
    STRUCTURE.boundary(invalidate);
    POPULARITY.boundary(invalidate);
}

fn refresh() {
    let enabled = OWN.load(Ordering::SeqCst) && FORWARD.load(Ordering::SeqCst);
    STRUCTURE.set_enabled(enabled);
    POPULARITY.set_enabled(enabled);
}

pub fn set_forward(enabled: bool) {
    FORWARD.store(enabled, Ordering::SeqCst);
    refresh();
}

pub fn forwarded(site: Forwarded, receiver: usize) -> Guard<'static> {
    FORWARDED[site as usize].fetch_add(1, Ordering::Relaxed);
    match site {
        Forwarded::ResetCountry => Guard::new(&STRUCTURE, &POPULARITY, Domain::Both),
        Forwarded::PoliticsDaily => Guard::daily(&STRUCTURE, &POPULARITY, receiver),
    }
}

#[doc = " `first` is the site's first argument: `capital setter`'s country."]
fn notify(site: usize, first: usize) -> Guard<'static> {
    CALLS[site].fetch_add(1, Ordering::Relaxed);
    if site == SET_CAPITAL {
        Guard::capital(&STRUCTURE, first)
    } else {
        Guard::new(&STRUCTURE, &POPULARITY, MUTATIONS[site].2)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn enable_for_test() {
    OWN.store(true, Ordering::SeqCst);
    FORWARD.store(true, Ordering::SeqCst);
    refresh();
}

#[cfg(any(test, feature = "test-support"))]
pub fn write_for_test(domain: Domain) {
    drop(Guard::new(&STRUCTURE, &POPULARITY, domain));
}

#[cfg(any(test, feature = "test-support"))]
pub struct Watches {
    structure: WatchRegistry,
    popularity: WatchRegistry,
}

#[cfg(any(test, feature = "test-support"))]
impl Watches {
    pub fn enabled() -> Self {
        let watches = Self {
            structure: WatchRegistry::new(),
            popularity: WatchRegistry::new(),
        };
        watches.structure.set_enabled(true);
        watches.popularity.set_enabled(true);
        watches
    }

    pub fn write(&self, domain: Domain) {
        drop(Guard::new(&self.structure, &self.popularity, domain));
    }

    pub fn daily(&self, status: usize) -> Guard<'_> {
        Guard::daily(&self.structure, &self.popularity, status)
    }

    pub fn capital(&self, country: usize) -> Guard<'_> {
        Guard::capital(&self.structure, country)
    }

    pub fn boundary(&self, invalidate: bool) {
        self.structure.boundary(invalidate);
        self.popularity.boundary(invalidate);
    }

    pub fn poisoned(&self) -> bool {
        self.structure.poisoned() && self.popularity.poisoned()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Changes for Watches {
    fn structure(&self) -> Option<(u64, u64)> {
        self.structure.quiet()
    }
    fn popularity(&self) -> Option<(u64, u64)> {
        self.popularity.quiet()
    }
}


pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A differing sentinel leaves the family uninstalled; the Politics walk
    /// then reads every hour.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "political_status_changes")
            && MUTATIONS
                .iter()
                .all(|(site, _, _)| crate::located(base, site.rva).is_ok())
            && let Ok(set_popularity) = engine_sites::hook(base, SET_POPULARITY_RVA)
        {
            SET_POPULARITY.store(set_popularity.address, Ordering::Release);
            let callbacks: [usize; SITES] = [
                set_capital as *const () as usize,
                set_politics as *const () as usize,
                force_ruling_party as *const () as usize,
                set_ruling_party as *const () as usize,
                set_ideas as *const () as usize,
                add_popularity as *const () as usize,
                set_popularity_redistributing as *const () as usize,
                set_political_party_popularity as *const () as usize,
                set_popularities as *const () as usize,
            ];
            for (index, (site, patch, _)) in MUTATIONS.iter().enumerate() {
                let code = engine_sites::hook(base, site.rva)?;
                let original = code.sentinel::<16>()?;
                let hook = match patch {
                    Patch::Entry => unsafe { Hook::new(code.address, callbacks[index], original) },
                    Patch::Call => unsafe {
                        Hook::new_call(code.address, callbacks[index], original)
                    },
                }
                .map_err(|_| "native hook construction failed")?;
                if *patch == Patch::Entry {
                    ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                }
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            OWN.store(false, Ordering::SeqCst);
            refresh();
            crate::country_input_changes::forwarder(
                crate::country_input_changes::Forwarder::PoliticalStatus,
                false,
            );
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        OWN.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        refresh();
        crate::country_input_changes::forwarder(
            crate::country_input_changes::Forwarder::PoliticalStatus,
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
        // SAFETY: the domains hold no table; release only starts an epoch
        // once the hooks were removed and drained.
        unsafe {
            STRUCTURE.release();
            POPULARITY.release();
        }
    }
}

macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
            fifth: usize,
        ) -> usize {
            let timing =
                crate::hook_timing::enter(crate::hook_timing::Family::PoliticalStatusChanges);
            let _notification = notify($site, first);
            let _inputs = crate::country_input_changes::forwarded_status($site, first);
            let original: unsafe extern "system" fn(usize, usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(first, second, third, fourth, fifth) })
        }
    };
}
callback!(set_capital, 0);
callback!(set_politics, 1);
callback!(force_ruling_party, 2);
callback!(set_ruling_party, 3);
callback!(set_ideas, 4);
callback!(add_popularity, 5);
callback!(set_popularity_redistributing, 6);

macro_rules! call_site {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(party: usize, value: usize) -> usize {
            let timing =
                crate::hook_timing::enter(crate::hook_timing::Family::PoliticalStatusChanges);
            let set: unsafe extern "system" fn(usize, usize) -> usize =
                unsafe { std::mem::transmute(SET_POPULARITY.load(Ordering::Acquire)) };
            let _notification = notify($site, party);
            let _inputs = crate::country_input_changes::forwarded_status($site, party);
            timing.original(|| unsafe { set(party, value) })
        }
    };
}
call_site!(set_political_party_popularity, 7);
call_site!(set_popularities, 8);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_daily_forward_moves_the_structure_only_when_the_last_election_changes() {
        let domains = Watches::enabled();
        let mut status = vec![0_u8; 0x100].into_boxed_slice();
        let at = cpolitical_status::LAST_ELECTION;
        status[at..at + 4].copy_from_slice(&1_000_i32.to_le_bytes());
        let address = status.as_ptr() as usize;
        let before = (domains.structure(), domains.popularity());
        let guard = domains.daily(address);
        assert_eq!(domains.popularity(), None);
        assert_eq!(domains.structure(), before.0);
        drop(guard);
        assert_eq!(
            domains.structure(),
            before.0,
            "no election: structure quiet"
        );
        assert!(domains.popularity().is_some());
        assert_ne!(domains.popularity(), before.1);

        let quiet = domains.structure();
        let guard = domains.daily(address);
        status[at..at + 4].copy_from_slice(&1_024_i32.to_le_bytes());
        assert_eq!(domains.structure(), quiet, "the store is seen on return");
        drop(guard);
        assert!(domains.structure().is_some());
        assert_ne!(
            domains.structure(),
            quiet,
            "an election moves the structure"
        );

        let quiet = domains.structure();
        let guard = domains.daily(0x10);
        assert_eq!(
            domains.structure(),
            None,
            "an unreadable status covers the call"
        );
        drop(guard);
        assert_ne!(domains.structure(), quiet);
        drop(status);
    }

    #[test]
    fn the_capital_setter_moves_the_structure_only_when_a_capital_changes() {
        let domains = Watches::enabled();
        let mut country = vec![0_u8; ccountry::ORIGINAL_CAPITAL + 4].into_boxed_slice();
        let address = country.as_ptr() as usize;
        let quiet = (domains.structure(), domains.popularity());
        let guard = domains.capital(address);
        assert_eq!(domains.structure(), quiet.0);
        drop(guard);
        assert_eq!((domains.structure(), domains.popularity()), quiet);
        for at in [ccountry::CAPITAL, ccountry::ORIGINAL_CAPITAL] {
            let quiet = domains.structure();
            let guard = domains.capital(address);
            country[at..at + 4].copy_from_slice(&7_i32.to_le_bytes());
            assert_eq!(domains.structure(), quiet, "the store is seen on return");
            drop(guard);
            assert!(domains.structure().is_some());
            assert_ne!(domains.structure(), quiet, "a capital change moves it");
        }
        let quiet = domains.structure();
        let guard = domains.capital(0x10);
        assert_eq!(
            domains.structure(),
            None,
            "an unreadable country covers the call"
        );
        drop(guard);
        assert_ne!(domains.structure(), quiet);
        drop(country);
    }

    #[test]
    fn sites_move_their_own_domains() {
        let domains = Watches::enabled();
        for (site, _, domain) in &MUTATIONS {
            let before = (domains.structure(), domains.popularity());
            domains.write(*domain);
            let after = (domains.structure(), domains.popularity());
            assert_eq!(before.0 != after.0, domain.structure(), "{}", site.name);
            assert_eq!(before.1 != after.1, domain.popularity(), "{}", site.name);
        }
    }

    #[test]
    fn a_writer_moves_the_hooks_domain() {
        enable_for_test();
        for domain in [Domain::Structure, Domain::Popularity] {
            let token = || match domain {
                Domain::Structure => Observer.structure(),
                _ => Observer.popularity(),
            };
            let before = token();
            write_for_test(domain);
            if before.is_some() {
                assert_ne!(token(), before);
            }
        }
    }
}
