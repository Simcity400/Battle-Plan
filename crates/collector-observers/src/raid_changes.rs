use crate::{
    dirty_set::{Dirty, DirtySet},
    hook::Hook,
    hook_timing::CallCounters,
};
use engine_mutations::raid_mutations::{DAILY_CAPACITIES, MUTATIONS, ORIGIN_ROUTE_SITE, Receiver};
use engine_native_memory::Memory;
use std::{
    cell::Cell,
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
const CAPACITY_SITE: usize = 24;
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static DAILY_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static DAILY_FLIGHTS: AtomicUsize = AtomicUsize::new(0);
std::thread_local! {
    static CERTIFIED_PASS: Cell<bool> = const { Cell::new(false) };
    static REFRESHING_ORIGIN: Cell<Option<usize>> = const { Cell::new(None) };
}
static BASE: AtomicUsize = AtomicUsize::new(0);
static READS: [[AtomicU64; 6]; 4] = [const { [const { AtomicU64::new(0) }; 6] }; 4];
static IDS: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
static AIR_CLEARS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    States = 0,
    Countries = 1,
    Sites = 2,
    Routes = 3,
}

pub struct Watches {
    states: DirtySet,
    countries: DirtySet,
    sites: DirtySet,
    routes: DirtySet,
    origin_flights: AtomicUsize,
    calls: CallCounters<SITES>,
    origin_calls: AtomicU64,
    unnamed: AtomicU64,
    buildings: [AtomicU64; 2],
    unchanged_capacities: AtomicU64,
    daily_passes: [AtomicU64; 2],
    untimed_capacities: AtomicU64,
    hooks: AtomicBool,
    forwarder: AtomicBool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    State(Option<usize>),
    Country(Option<usize>),
    Site(Option<usize>),
    EverySite,
    Route(Option<usize>),
    EveryRoute,
    Nothing,
}

impl Default for Watches {
    fn default() -> Self {
        Self::new()
    }
}

impl Watches {
    pub const fn new() -> Self {
        Self {
            states: DirtySet::new(),
            countries: DirtySet::new(),
            sites: DirtySet::new(),
            routes: DirtySet::new(),
            origin_flights: AtomicUsize::new(0),
            calls: CallCounters::new(),
            origin_calls: AtomicU64::new(0),
            unnamed: AtomicU64::new(0),
            buildings: [const { AtomicU64::new(0) }; 2],
            unchanged_capacities: AtomicU64::new(0),
            daily_passes: [const { AtomicU64::new(0) }; 2],
            untimed_capacities: AtomicU64::new(0),
            hooks: AtomicBool::new(false),
            forwarder: AtomicBool::new(false),
        }
    }

    fn sets(&self) -> [&DirtySet; 4] {
        [&self.states, &self.countries, &self.sites, &self.routes]
    }

    fn mark<'a>(&'a self, set: &'a DirtySet, id: Option<usize>) -> Dirty<'a> {
        match id {
            Some(id) => set.notify(id),
            None => {
                self.unnamed.fetch_add(1, Ordering::Relaxed);
                set.notify_every()
            }
        }
    }

    #[inline]
    pub fn notify(&self, site: usize, target: Target) -> Option<Dirty<'_>> {
        if site == ORIGIN_ROUTE_SITE {
            self.origin_calls.fetch_add(1, Ordering::Relaxed);
        } else {
            self.calls.increment(site);
        }
        match target {
            Target::State(id) => Some(self.mark(&self.states, id)),
            Target::Country(id) => Some(self.mark(&self.countries, id)),
            Target::Site(id) => Some(self.mark(&self.sites, id)),
            Target::EverySite => Some(self.sites.notify_every()),
            Target::Route(id) => Some(self.mark(&self.routes, id)),
            Target::EveryRoute => Some(self.routes.notify_every()),
            Target::Nothing => None,
        }
    }

    fn refresh_origin<R>(
        &self,
        origin: usize,
        mut kind: impl FnMut() -> Option<u8>,
        target: impl FnOnce() -> Target,
        original: impl FnOnce() -> R,
    ) -> R {
        struct Flight<'a> {
            active: &'a AtomicUsize,
            previous_origin: Option<usize>,
        }
        impl Drop for Flight<'_> {
            fn drop(&mut self) {
                REFRESHING_ORIGIN.with(|current| current.set(self.previous_origin));
                self.active.fetch_sub(1, Ordering::SeqCst);
            }
        }
        self.origin_flights.fetch_add(1, Ordering::SeqCst);
        let _flight = Flight {
            active: &self.origin_flights,
            previous_origin: REFRESHING_ORIGIN.with(|current| current.replace(Some(origin))),
        };
        let before = kind();
        let result = original();
        let after = kind();
        drop(self.notify(
            ORIGIN_ROUTE_SITE,
            if before.is_none() || after.is_none() || before != after {
                target()
            } else {
                Target::Nothing
            },
        ));
        result
    }

    pub fn capacity(
        &self,
        site: usize,
        changed: bool,
        target: impl FnOnce() -> Target,
    ) -> Option<Dirty<'_>> {
        if changed {
            return self.notify(site, target());
        }
        self.unchanged_capacities.fetch_add(1, Ordering::Relaxed);
        None
    }

    fn building(&self, flags: u32, location: u32) -> Option<Dirty<'_>> {
        if flags & engine_native::cprovince::buildings::STATE_FLAG == 0 {
            return None;
        }
        self.buildings[0].fetch_add(1, Ordering::Relaxed);
        Some(self.mark(&self.states, (location != 0).then_some(location as usize)))
    }

    fn building_unnamed(&self) -> Dirty<'_> {
        self.buildings[1].fetch_add(1, Ordering::Relaxed);
        self.states.notify_every()
    }

    fn apply(&self) {
        let hooks = self.hooks.load(Ordering::SeqCst);
        crate::division_panel_changes::state_forwarder(
            hooks && self.forwarder.load(Ordering::SeqCst) && !self.states.poisoned(),
        );
        self.states
            .set_enabled(hooks && self.forwarder.load(Ordering::SeqCst));
        self.countries.set_enabled(hooks);
        self.sites.set_enabled(hooks);
        self.routes.set_enabled(hooks);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(states: usize, countries: usize) -> Self {
        let watches = Self::new();
        watches.states.offer(states);
        watches.countries.offer(countries);
        watches.sites.offer(countries);
        watches.routes.offer(countries);
        for set in watches.sets() {
            set.boundary(false);
        }
        watches.hooks.store(true, Ordering::SeqCst);
        watches.forwarder.store(true, Ordering::SeqCst);
        watches.apply();
        watches
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        for set in self.sets() {
            set.boundary(invalidate);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_forwarder(&self, enabled: bool) {
        self.forward(enabled);
    }

    fn forward(&self, enabled: bool) {
        self.forwarder.store(enabled, Ordering::SeqCst);
        self.states
            .set_enabled(enabled && self.hooks.load(Ordering::SeqCst));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self, family: Family) -> bool {
        self.sets()[family as usize].poisoned()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn mark_building(&self, flags: u32, location: u32) -> Option<Dirty<'_>> {
        self.building(flags, location)
    }
}

pub trait Changes: Sync {
    fn take(&self, family: Family, visit: &mut dyn FnMut(usize)) -> Option<u64>;
}

impl Changes for Watches {
    fn take(&self, family: Family, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        let origin_calls =
            (family == Family::Routes).then(|| self.origin_calls.load(Ordering::Acquire));
        if family == Family::Routes && self.origin_flights.load(Ordering::SeqCst) != 0 {
            return None;
        }
        let epoch = self.sets()[family as usize].take(visit);
        if let Some(before) = origin_calls
            && (self.origin_flights.load(Ordering::SeqCst) != 0
                || self.origin_calls.load(Ordering::Acquire) != before)
        {
            return None;
        }
        epoch
    }
}

pub struct Observer;
impl Changes for Observer {
    fn take(&self, family: Family, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        WATCHES.take(family, visit)
    }
}

#[inline(always)]
fn target(receiver: Receiver, first: usize, second: usize, third: usize) -> Target {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    match receiver {
        Receiver::CountryResources => Target::Country(
            engine_mutations::raid_mutations::country(&memory, first).map(|raw| raw as usize),
        ),
        Receiver::StrategicAir | Receiver::CountryTag | Receiver::PoolCapacity => {
            match engine_mutations::raid_mutations::site_country(
                &memory, receiver, first, second, third,
            ) {
                Ok(Some(raw)) => Target::Site(Some(raw as usize)),
                Ok(None) => Target::Nothing,
                Err(engine_mutations::raid_mutations::Unreadable) => Target::Site(None),
            }
        }
        Receiver::EveryCountry => Target::EverySite,
        Receiver::Route | Receiver::RouteSender | Receiver::OriginRoute => {
            let (receiver, first) = if matches!(receiver, Receiver::Route) {
                REFRESHING_ORIGIN
                    .with(Cell::get)
                    .filter(|origin| {
                        origin.checked_add(engine_resources::cresource_origin::DELIVERY_ROUTE)
                            == Some(first)
                    })
                    .map_or((receiver, first), |origin| (Receiver::OriginRoute, origin))
            } else {
                (receiver, first)
            };
            match engine_mutations::raid_mutations::route_country(&memory, receiver, first, second)
            {
                Ok(Some(raw)) => Target::Route(Some(raw as usize)),
                Ok(None) => Target::Nothing,
                Err(engine_mutations::raid_mutations::Unreadable) => Target::Route(None),
            }
        }
        Receiver::EveryRoute => Target::EveryRoute,
        _ => match engine_mutations::raid_mutations::state(&memory, receiver, first, third) {
            Ok(Some(id)) => Target::State(Some(id as usize)),
            Ok(None) => Target::Nothing,
            Err(engine_mutations::raid_mutations::Unreadable) => Target::State(None),
        },
    }
}

#[inline(always)]
fn note_air_clear(site: usize) {
    if site == 21 || site == 22 {
        AIR_CLEARS.fetch_add(1, Ordering::SeqCst);
    }
}

pub fn air_clear_token() -> Option<u64> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(token) = TEST_AIR_CLEAR_TOKEN.with(std::cell::Cell::get) {
        return token;
    }
    WATCHES
        .hooks
        .load(Ordering::SeqCst)
        .then(|| AIR_CLEARS.load(Ordering::SeqCst))
}

#[cfg(any(test, feature = "test-support"))]
std::thread_local! {
    pub static TEST_AIR_CLEAR_TOKEN: std::cell::Cell<Option<Option<u64>>> =
        const { std::cell::Cell::new(Some(Some(0))) };
}

pub fn active() -> usize {
    WATCHES.sets().iter().map(|set| set.active()).sum::<usize>()
        + DAILY_FLIGHTS.load(Ordering::SeqCst)
        + WATCHES.origin_flights.load(Ordering::SeqCst)
}

pub fn boundary(invalidate: bool) {
    for set in WATCHES.sets() {
        set.boundary(invalidate);
    }
}

pub fn poison() {
    crate::division_panel_changes::state_forwarder(false);
    WATCHES.states.poison();
}

pub fn forwarder(enabled: bool) {
    WATCHES.forward(enabled);
}

pub fn building_changed(flags: u32, location: u32) -> Option<Dirty<'static>> {
    WATCHES.building(flags, location)
}

pub fn building_unnamed() -> Dirty<'static> {
    WATCHES.building_unnamed()
}

pub fn presize() {
    for (set, ids) in WATCHES.sets().into_iter().zip(&IDS) {
        let ids = ids.load(Ordering::Relaxed);
        if ids != 0 {
            set.offer(ids);
        }
    }
}

pub fn collect() {
    for set in WATCHES.sets() {
        set.collect();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub marked: u64,
    pub read: u64,
    pub carried: u64,
    pub early: u64,
    pub full: bool,
}

pub fn record(family: Family, counts: &Counts, ids: Option<usize>) {
    for (target, value) in READS[family as usize].iter().zip([
        1,
        counts.marked,
        counts.read,
        counts.carried,
        counts.early,
        u64::from(counts.full),
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if let Some(ids) = ids {
        IDS[family as usize].store(ids, Ordering::Relaxed);
    }
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A differing sentinel leaves the family uninstalled; the raid state
    /// tables and RaidAirSites then read every owner every hour.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "raid_changes")
            && MUTATIONS
                .iter()
                .all(|(site, _)| crate::located(base, site.rva).is_ok())
        {
            BASE.store(base, Ordering::Release);
            let callbacks: [usize; SITES] = [
                refresh_resource_modifier_gates as *const () as usize,
                add_building_dynamic_modifier as *const () as usize,
                remove_building_dynamic_modifier as *const () as usize,
                add_resource as *const () as usize,
                expire_timed_resource as *const () as usize,
                destroy_state as *const () as usize,
                add_core as *const () as usize,
                remove_core as *const () as usize,
                construct_resource_origin as *const () as usize,
                destroy_resource_origin as *const () as usize,
                add_building_resource_modifier as *const () as usize,
                clear_country_resources as *const () as usize,
                construct_strategic_air as *const () as usize,
                append_air_site as *const () as usize,
                append_missile_site as *const () as usize,
                refresh_air_base_access as *const () as usize,
                populate_air_sites as *const () as usize,
                repair_air_base_containers as *const () as usize,
                withdraw_air_site as *const () as usize,
                withdraw_missile_site as *const () as usize,
                transfer_air_base as *const () as usize,
                clear_air_sites as *const () as usize,
                clear_missile_sites as *const () as usize,
                destroy_strategic_air as *const () as usize,
                set_country_pool_capacity as *const () as usize,
                add_claim as *const () as usize,
                remove_claim as *const () as usize,
                add_state_strategic_location as *const () as usize,
                add_province_strategic_location as *const () as usize,
                update_trade_route as *const () as usize,
                construct_trade_route as *const () as usize,
                construct_country_resources as *const () as usize,
                load_trade_route as *const () as usize,
                update_supply_trade_route as *const () as usize,
                refresh_origin_trade_route as *const () as usize,
            ];
            for (index, (site, _)) in MUTATIONS.iter().enumerate() {
                let code = engine_sites::hook(base, site.rva)?;
                let hook =
                    unsafe { Hook::new(code.address, callbacks[index], code.sentinel::<16>()?) }
                        .map_err(|_| "native hook construction failed")?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
            let daily = DAILY_CAPACITIES;
            if crate::located(base, daily.rva).is_ok() {
                let code = engine_sites::hook(base, daily.rva)?;
                let hook = unsafe {
                    Hook::new(
                        code.address,
                        daily_refresh_base_capacities as *const () as usize,
                        code.sentinel::<16>()?,
                    )
                }
                .map_err(|_| "native hook construction failed")?;
                DAILY_ORIGINAL.store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            crate::occupation_changes::forwarder(
                crate::occupation_changes::Forwarder::States,
                false,
            );
            crate::division_panel_changes::state_forwarder(false);
            crate::territory_changes::modifier_forwarder(false);
            AIR_CLEARS.fetch_add(1, Ordering::SeqCst);
            WATCHES.hooks.store(false, Ordering::SeqCst);
            WATCHES.apply();
            crate::country_input_changes::forwarder(
                crate::country_input_changes::Forwarder::Raid,
                false,
            );
            crate::air_base_carry_changes::forwarder(
                crate::air_base_carry_changes::Forwarder::Raid,
                false,
            );
        }
        let publication = if enabled {
            Hook::set_family_enabled_shared_prepared(&mut self.hooks, enabled, || {
                AIR_CLEARS.fetch_add(1, Ordering::SeqCst);
            })
        } else {
            Hook::set_family_enabled_shared(&mut self.hooks, enabled)
        };
        if !publication.map_err(|_| "native hook state change failed")? {
            return Ok(false);
        }
        WATCHES
            .hooks
            .store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        WATCHES.apply();
        crate::country_input_changes::forwarder(
            crate::country_input_changes::Forwarder::Raid,
            enabled && !self.hooks.is_empty(),
        );
        crate::air_base_carry_changes::forwarder(
            crate::air_base_carry_changes::Forwarder::Raid,
            enabled && !self.hooks.is_empty(),
        );
        crate::occupation_changes::forwarder(
            crate::occupation_changes::Forwarder::States,
            enabled && !self.hooks.is_empty(),
        );
        crate::territory_changes::modifier_forwarder(enabled && !self.hooks.is_empty());
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: like each Hook's relay, the bitmaps are dropped only once
        // the hooks were never enabled, or were removed and drained after
        // the last capture; the building family's forwarders are restored
        // with the other families before any hook is dropped.
        for set in WATCHES.sets() {
            unsafe { set.release() };
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
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::RaidChanges);
            let _occupation = matches!($site, 0 | 5).then(|| {
                crate::occupation_changes::changed_pair(
                    first,
                    first.wrapping_add(engine_native::cstate::COUNTRY_MODIFIERS),
                )
            });
            let target = target(MUTATIONS[$site].1, first, second, third);
            let _panel = match target {
                Target::State(id) => Some(crate::division_panel_changes::state_notify(id)),
                _ => None,
            };
            let _modifiers = match target {
                Target::State(id) if matches!($site, 0 | 5) => {
                    Some(crate::territory_changes::modifier_state(id))
                }
                _ => None,
            };
            let _mark = WATCHES.notify($site, target);
            let _inputs = crate::country_input_changes::forwarded_raid($site, first, second);
            note_air_clear($site);
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(first, second, third, fourth) })
        }
    };
}
callback!(refresh_resource_modifier_gates, 0);
callback!(add_building_dynamic_modifier, 1);
callback!(remove_building_dynamic_modifier, 2);
callback!(add_resource, 3);
callback!(expire_timed_resource, 4);
callback!(destroy_state, 5);
callback!(add_core, 6);
callback!(remove_core, 7);
callback!(construct_resource_origin, 8);
callback!(destroy_resource_origin, 9);
callback!(add_building_resource_modifier, 10);
callback!(clear_country_resources, 11);
callback!(construct_strategic_air, 12);
callback!(append_air_site, 13);
callback!(append_missile_site, 14);
callback!(refresh_air_base_access, 15);
callback!(populate_air_sites, 16);
callback!(repair_air_base_containers, 17);
callback!(withdraw_air_site, 18);
callback!(withdraw_missile_site, 19);
callback!(transfer_air_base, 20);
callback!(clear_air_sites, 21);
callback!(clear_missile_sites, 22);
callback!(destroy_strategic_air, 23);
callback!(add_claim, 25);
callback!(remove_claim, 26);
callback!(add_state_strategic_location, 27);
callback!(add_province_strategic_location, 28);
callback!(update_trade_route, 29);
callback!(construct_trade_route, 30);
callback!(construct_country_resources, 31);
callback!(load_trade_route, 32);
callback!(update_supply_trade_route, 33);

unsafe extern "system" fn refresh_origin_trade_route(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::RaidChanges);
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[ORIGIN_ROUTE_SITE].load(Ordering::Acquire)) };
    WATCHES.refresh_origin(
        first,
        || engine_mutations::raid_mutations::origin_route_kind(&memory, first),
        || target(Receiver::OriginRoute, first, second, third),
        || timing.original(|| unsafe { original(first, second, third, fourth) }),
    )
}

unsafe extern "system" fn set_country_pool_capacity(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[CAPACITY_SITE].load(Ordering::Acquire)) };
    if CERTIFIED_PASS.get() {
        return unsafe { original(first, second, third, fourth) };
    }
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::RaidChanges);
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let held = engine_mutations::raid_mutations::pool_capacity(
        &memory,
        BASE.load(Ordering::Relaxed),
        first,
        third,
    );
    let changed = held != Some(second as i64);
    let _mark = WATCHES.capacity(CAPACITY_SITE, changed, || {
        target(MUTATIONS[CAPACITY_SITE].1, first, second, third)
    });
    let _carry = changed.then(|| crate::air_base_carry_changes::capacity_written(first));
    timing.original(|| unsafe { original(first, second, third, fourth) })
}

unsafe extern "system" fn daily_refresh_base_capacities(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::RaidChanges);
    DAILY_FLIGHTS.fetch_add(1, Ordering::SeqCst);
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let unchanged = collector_extended_support::hourly::ticking()
        .then(|| {
            engine_mutations::raid_mutations::daily_unchanged(
                &memory,
                BASE.load(Ordering::Relaxed),
                first,
                second,
            )
        })
        .flatten();
    WATCHES.daily_passes[usize::from(unchanged.is_none())].fetch_add(1, Ordering::Relaxed);
    let _pass = unchanged.map(|stores| {
        WATCHES
            .unchanged_capacities
            .fetch_add(stores, Ordering::Relaxed);
        CertifiedPass::enter()
    });
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(DAILY_ORIGINAL.load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(first, second, third, fourth) });
    if let Some(stores) = unchanged {
        WATCHES
            .untimed_capacities
            .fetch_add(stores, Ordering::Relaxed);
    }
    DAILY_FLIGHTS.fetch_sub(1, Ordering::SeqCst);
    result
}

struct CertifiedPass(bool);
impl CertifiedPass {
    fn enter() -> Self {
        Self(CERTIFIED_PASS.replace(true))
    }
}
impl Drop for CertifiedPass {
    fn drop(&mut self) {
        CERTIFIED_PASS.set(self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[doc = " Tests and benches driving the global detours take this first."]
    static DAILY_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn take(watches: &Watches, family: Family) -> (Option<u64>, Vec<usize>) {
        let mut ids = Vec::new();
        let epoch = watches.take(family, &mut |id| ids.push(id));
        (epoch, ids)
    }

    fn site(name: &str) -> usize {
        MUTATIONS
            .iter()
            .position(|(site, _)| site.name == name)
            .unwrap()
    }

    #[test]
    fn receivers_name_their_state_or_country() {
        let mut state = vec![0_u8; 0x400];
        state[engine_native::cstate::ID..][..4].copy_from_slice(&42_u32.to_le_bytes());
        let state_at = state.as_ptr() as usize;
        let mut origin = vec![0_u8; 0x100];
        origin[engine_resources::cresource_origin::STATE..][..8]
            .copy_from_slice(&(state_at as u64).to_le_bytes());
        let mut resources = vec![0_u8; 0x100];
        resources[engine_resources::ccountry_resources::OWNER..][..4]
            .copy_from_slice(&9_u32.to_le_bytes());
        assert_eq!(
            target(Receiver::State, state_at, 0, 0),
            Target::State(Some(42))
        );
        assert_eq!(
            target(
                Receiver::StatePool,
                state_at + engine_native::cstate::RESOURCE_POOL,
                0,
                0
            ),
            Target::State(Some(42))
        );
        assert_eq!(
            target(Receiver::OriginState, 0, 0, state_at),
            Target::State(Some(42))
        );
        assert_eq!(target(Receiver::OriginState, 0, 0, 0), Target::Nothing);
        assert_eq!(
            target(Receiver::Origin, origin.as_ptr() as usize, 0, 0),
            Target::State(Some(42))
        );
        origin[engine_resources::cresource_origin::STATE..][..8]
            .copy_from_slice(&0_u64.to_le_bytes());
        assert_eq!(
            target(Receiver::Origin, origin.as_ptr() as usize, 0, 0),
            Target::Nothing
        );
        assert_eq!(
            target(
                Receiver::CountryResources,
                resources.as_ptr() as usize,
                0,
                0
            ),
            Target::Country(Some(9))
        );
        assert_eq!(target(Receiver::State, 0, 0, 0), Target::State(None));
        assert_eq!(
            target(Receiver::CountryResources, 0, 0, 0),
            Target::Country(None)
        );
        let mut air = vec![0_u8; 0x100];
        air[engine_native_air::cstrategic_air::COUNTRY..][..4].copy_from_slice(&7_u32.to_le_bytes());
        let tag = 11_u32.to_le_bytes();
        let zero = 0_u32.to_le_bytes();
        assert_eq!(
            target(Receiver::StrategicAir, air.as_ptr() as usize, 0, 0),
            Target::Site(Some(7))
        );
        assert_eq!(
            target(Receiver::CountryTag, 0, tag.as_ptr() as usize, 0),
            Target::Site(Some(11))
        );
        assert_eq!(
            target(Receiver::PoolCapacity, 0, 0, tag.as_ptr() as usize),
            Target::Site(Some(11))
        );
        assert_eq!(
            target(Receiver::CountryTag, 0, zero.as_ptr() as usize, 0),
            Target::Nothing
        );
        assert_eq!(target(Receiver::StrategicAir, 0, 0, 0), Target::Site(None));
        assert_eq!(target(Receiver::EveryCountry, 0, 0, 0), Target::EverySite);
        let mut route = [0_u8; 0x40];
        route[engine_resources::resource_map::ROUTE_LINKS[0]..][..4]
            .copy_from_slice(&6_u32.to_le_bytes());
        assert_eq!(
            target(Receiver::Route, route.as_ptr() as usize, 0, 0),
            Target::Route(Some(6))
        );
        assert_eq!(
            target(Receiver::RouteSender, 0, tag.as_ptr() as usize, 0),
            Target::Route(Some(11))
        );
        assert_eq!(
            target(Receiver::RouteSender, 0, zero.as_ptr() as usize, 0),
            Target::Nothing
        );
        assert_eq!(target(Receiver::Route, 0, 0, 0), Target::Route(None));
        assert_eq!(target(Receiver::EveryRoute, 0, 0, 0), Target::EveryRoute);
    }

    fn family_of(receiver: Receiver) -> Family {
        match receiver {
            Receiver::CountryResources => Family::Countries,
            Receiver::StrategicAir
            | Receiver::CountryTag
            | Receiver::EveryCountry
            | Receiver::PoolCapacity => Family::Sites,
            Receiver::Route
            | Receiver::RouteSender
            | Receiver::OriginRoute
            | Receiver::EveryRoute => Family::Routes,
            _ => Family::States,
        }
    }

    #[test]
    fn supply_route_updater_marks_its_sender() {
        let site = site("update_supply_trade_route");
        assert_eq!(MUTATIONS[site].1, Receiver::Route);
        let watches = Watches::enabled(64, 64);
        let epoch = take(&watches, Family::Routes).0;
        let mut route = [0_u8; 0x40];
        route[engine_resources::resource_map::ROUTE_LINKS[0]..][..4]
            .copy_from_slice(&6_u32.to_le_bytes());
        route[engine_resources::resource_map::ROUTE_LINKS[1]..][..4]
            .copy_from_slice(&11_u32.to_le_bytes());
        let mark = watches.notify(
            site,
            target(MUTATIONS[site].1, route.as_ptr() as usize, 0, 0),
        );
        assert_eq!(take(&watches, Family::Routes).0, None);
        drop(mark);
        assert_eq!(take(&watches, Family::Routes), (epoch, vec![6]));
    }

    #[test]
    fn origin_inline_clear_marks_only_a_changed_owner_and_refuses_a_running_refresh() {
        assert_eq!(site("refresh_origin_trade_route"), ORIGIN_ROUTE_SITE);
        assert_eq!(MUTATIONS[ORIGIN_ROUTE_SITE].1, Receiver::OriginRoute);
        let watches = Watches::enabled(64, 64);
        let epoch = take(&watches, Family::Routes).0;
        for (before, after, marked) in [
            (Some(2), Some(0), true),
            (Some(0), Some(0), false),
            (Some(2), Some(2), false),
            (None, Some(0), true),
            (Some(2), None, true),
        ] {
            let kind = Cell::new(before);
            assert_eq!(
                watches.refresh_origin(
                    0,
                    || kind.get(),
                    || Target::Route(Some(6)),
                    || {
                        assert_eq!(take(&watches, Family::Routes).0, None);
                        kind.set(after);
                        123
                    },
                ),
                123
            );
            assert_eq!(
                take(&watches, Family::Routes),
                (epoch, if marked { vec![6] } else { vec![] })
            );
        }
        watches.refresh_origin(
            0,
            || Some(2),
            || unreachable!("unchanged refresh does not name an owner"),
            || {
                drop(watches.notify(
                    engine_mutations::raid_mutations::FIRST_ROUTE_SITE,
                    Target::Route(Some(6)),
                ))
            },
        );
        assert_eq!(take(&watches, Family::Routes), (epoch, vec![6]));
        let mut origin = [0_u8; 0x400];
        origin[engine_mutations::raid_mutations::ORIGIN_OWNER..][..4]
            .copy_from_slice(&6_u32.to_le_bytes());
        assert_eq!(
            target(Receiver::OriginRoute, origin.as_ptr() as usize, 0, 0),
            Target::Route(Some(6))
        );
        assert_eq!(target(Receiver::OriginRoute, 0, 0, 0), Target::Route(None));
    }

    #[test]
    fn an_origin_refresh_completed_inside_the_route_take_refuses_the_certificate() {
        let watches = Watches::enabled(64, 64);
        let epoch = take(&watches, Family::Routes).0;
        drop(watches.notify(
            engine_mutations::raid_mutations::FIRST_ROUTE_SITE,
            Target::Route(Some(3)),
        ));
        assert_eq!(
            watches.take(Family::Routes, &mut |id| {
                assert_eq!(id, 3);
                let kind = Cell::new(Some(2));
                watches.refresh_origin(
                    0,
                    || kind.get(),
                    || Target::Route(Some(6)),
                    || kind.set(Some(0)),
                );
            }),
            None
        );
        assert_eq!(take(&watches, Family::Routes), (epoch, vec![6]));
    }

    #[test]
    fn rights_origin_clear_and_nested_repath_mark_the_holder_instead_of_the_sender() {
        let watches = Watches::enabled(64, 64);
        let epoch = take(&watches, Family::Routes).0;
        let mut origin = [0_u8; 0x400];
        origin[engine_mutations::raid_mutations::ORIGIN_OWNER..][..4]
            .copy_from_slice(&11_u32.to_le_bytes());
        origin[engine_resources::cresource_origin::DELIVERY_ROUTE
            + engine_resources::resource_map::ROUTE_LINKS[0]..][..4]
            .copy_from_slice(&6_u32.to_le_bytes());
        let origin = origin.as_ptr() as usize;
        let route = origin + engine_resources::cresource_origin::DELIVERY_ROUTE;
        let mut unrelated = [0_u8; 0x40];
        unrelated[engine_resources::resource_map::ROUTE_LINKS[0]..][..4]
            .copy_from_slice(&6_u32.to_le_bytes());
        let unrelated = unrelated.as_ptr() as usize;
        for repath in [false, true] {
            let kind = Cell::new(Some(2));
            assert_eq!(
                watches.refresh_origin(
                    origin,
                    || kind.get(),
                    || target(Receiver::OriginRoute, origin, 0, 0),
                    || {
                        assert_eq!(take(&watches, Family::Routes).0, None);
                        assert_eq!(
                            target(Receiver::Route, unrelated, 0, 0),
                            Target::Route(Some(6))
                        );
                        if repath {
                            drop(watches.notify(
                                engine_mutations::raid_mutations::FIRST_ROUTE_SITE,
                                target(Receiver::Route, route, 0, 0),
                            ));
                        } else {
                            kind.set(Some(0));
                        }
                        123
                    },
                ),
                123
            );
            assert_eq!(take(&watches, Family::Routes), (epoch, vec![11]));
            assert_eq!(REFRESHING_ORIGIN.with(Cell::get), None);
            assert_eq!(target(Receiver::Route, route, 0, 0), Target::Route(Some(6)));
        }
    }

    #[test]
    fn origin_owner_context_is_restored_after_nested_refresh_and_unwind() {
        let watches = Watches::enabled(64, 64);
        let epoch = take(&watches, Family::Routes).0;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            watches.refresh_origin(
                100,
                || Some(2),
                || unreachable!(),
                || {
                    watches.refresh_origin(
                        200,
                        || Some(2),
                        || unreachable!(),
                        || assert_eq!(REFRESHING_ORIGIN.with(Cell::get), Some(200)),
                    );
                    assert_eq!(REFRESHING_ORIGIN.with(Cell::get), Some(100));
                    panic!("synthetic original unwinds");
                },
            );
        }));
        assert!(result.is_err());
        assert_eq!(REFRESHING_ORIGIN.with(Cell::get), None);
        assert_eq!(take(&watches, Family::Routes), (epoch, vec![]));
    }

    #[test]
    fn each_site_marks_its_set() {
        const FAMILIES: [Family; 4] = [
            Family::States,
            Family::Countries,
            Family::Sites,
            Family::Routes,
        ];
        for (index, (mutation, receiver)) in MUTATIONS.iter().enumerate() {
            let watches = Watches::enabled(64, 64);
            let epochs = FAMILIES.map(|family| take(&watches, family).0.unwrap());
            let family = family_of(*receiver);
            let named = match family {
                Family::States => Target::State(Some(9)),
                Family::Countries => Target::Country(Some(9)),
                Family::Sites => Target::Site(Some(9)),
                Family::Routes => Target::Route(Some(9)),
            };
            drop(watches.notify(index, named));
            for (other, epoch) in FAMILIES.into_iter().zip(epochs) {
                let marked = if other == family { vec![9] } else { vec![] };
                assert_eq!(
                    take(&watches, other),
                    (Some(epoch), marked),
                    "{}",
                    mutation.name
                );
            }
            let unnamed = match family {
                Family::States => Target::State(None),
                Family::Countries => Target::Country(None),
                Family::Sites => Target::Site(None),
                Family::Routes => Target::Route(None),
            };
            drop(watches.notify(index, unnamed));
            for (other, epoch) in FAMILIES.into_iter().zip(epochs) {
                assert_eq!(
                    take(&watches, other).0 == Some(epoch),
                    other != family,
                    "{}",
                    mutation.name
                );
            }
        }
        let watches = Watches::enabled(64, 64);
        let states = take(&watches, Family::States).0;
        assert!(
            watches
                .notify(site("destroy_resource_origin"), Target::Nothing)
                .is_none()
        );
        assert_eq!(take(&watches, Family::States), (states, vec![]));
    }

    #[test]
    fn strategic_location_writers_mark_their_state() {
        let index = site("add_state_strategic_location");
        assert_eq!(MUTATIONS[index].1, Receiver::State);
        let index = site("add_province_strategic_location");
        assert_eq!(MUTATIONS[index].1, Receiver::Province);
        let watches = Watches::enabled(64, 64);
        let epoch = take(&watches, Family::States).0.unwrap();
        drop(watches.notify(index, Target::State(Some(12))));
        assert_eq!(take(&watches, Family::States), (Some(epoch), vec![12]));
        let mut memory = vec![0_u8; 0x200];
        let state = memory.as_ptr() as usize + 0x100;
        let state_slot = engine_native::cprovince::STATE;
        memory[state_slot..state_slot + 8].copy_from_slice(&(state as u64).to_le_bytes());
        memory[0x100 + engine_native::cstate::ID..0x100 + engine_native::cstate::ID + 4]
            .copy_from_slice(&12_u32.to_le_bytes());
        let province = memory.as_ptr() as usize;
        assert_eq!(
            target(Receiver::Province, province, 0, 0),
            Target::State(Some(12))
        );
        memory[state_slot..state_slot + 8].fill(0);
        assert_eq!(target(Receiver::Province, province, 0, 0), Target::Nothing);
    }

    #[test]
    fn forwarded_building_writes_mark_state_statuses() {
        use engine_native::cprovince::buildings::{PROVINCE_FLAG, STATE_FLAG};
        let watches = Watches::enabled(64, 64);
        let (Some(epoch), _) = take(&watches, Family::States) else {
            unreachable!("observing")
        };
        drop(watches.mark_building(STATE_FLAG, 12));
        assert!(watches.mark_building(PROVINCE_FLAG, 300).is_none());
        assert_eq!(take(&watches, Family::States), (Some(epoch), vec![12]));
        drop(watches.mark_building(STATE_FLAG, 0));
        assert_ne!(take(&watches, Family::States).0, Some(epoch));
        let (Some(epoch), _) = take(&watches, Family::States) else {
            unreachable!("observing")
        };
        drop(watches.building_unnamed());
        assert_ne!(take(&watches, Family::States).0, Some(epoch));
    }

    #[test]
    fn the_state_set_observes_only_with_the_forwarder() {
        let watches = Watches::enabled(64, 64);
        let (Some(states), _) = take(&watches, Family::States) else {
            unreachable!("observing")
        };
        let (Some(countries), _) = take(&watches, Family::Countries) else {
            unreachable!("observing")
        };
        watches.set_forwarder(false);
        assert_eq!(take(&watches, Family::States).0, None);
        assert_eq!(take(&watches, Family::Countries).0, Some(countries));
        watches.set_forwarder(true);
        assert_ne!(take(&watches, Family::States).0, Some(states));
        watches.states.poison();
        assert!(watches.poisoned(Family::States));
        assert!(!watches.poisoned(Family::Countries));
        assert_eq!(take(&watches, Family::States).0, None);
    }

    #[test]
    fn only_a_changing_capacity_marks_its_country() {
        let site = MUTATIONS
            .iter()
            .position(|(site, _)| site.name == "set_country_pool_capacity")
            .unwrap();
        assert_eq!(site, CAPACITY_SITE);
        let watches = Watches::enabled(64, 64);
        let epoch = take(&watches, Family::Sites).0;
        assert!(
            watches
                .capacity(site, false, || unreachable!(
                    "an unchanged store is not named"
                ))
                .is_none()
        );
        assert_eq!(take(&watches, Family::Sites), (epoch, vec![]));
        drop(watches.capacity(site, true, || Target::Site(Some(5))));
        assert_eq!(take(&watches, Family::Sites), (epoch, vec![5]));
        assert_eq!(watches.unchanged_capacities.load(Ordering::Relaxed), 1);
        drop(watches.notify(site, Target::EverySite));
        assert_ne!(take(&watches, Family::Sites).0, epoch);
    }

    struct DailyWorld {
        memory: Vec<u64>,
    }
    impl DailyWorld {
        const GAME: usize = 0x100;
        const LINKS: usize = 0x800;
        const AIR: usize = 0x1000;
        const BASES: usize = 0x1100;
        const PAIRS: usize = 0x5000;
        const VECTORS: usize = 0x5100;
        fn base(index: usize) -> usize {
            0x2000 + index * 0x1000
        }
        fn container(base: usize, slot: usize) -> usize {
            0x8000 + base * 0x400 + slot * 0x100
        }
        fn entries(base: usize) -> usize {
            0x6000 + base * 0x100
        }
        fn at(&self, offset: usize) -> usize {
            self.memory.as_ptr() as usize + offset
        }
        fn word(&mut self, offset: usize, value: u64) {
            self.memory[offset / 8] = value;
        }
        fn half(&mut self, offset: usize, value: u32) {
            let word = &mut self.memory[offset / 8];
            let shift = (offset % 8) * 8;
            *word = (*word & !(0xffff_ffff << shift)) | (u64::from(value) << shift);
        }
        fn array(&mut self, offset: usize, data: usize, count: u32) {
            self.word(offset, self.at(data) as u64);
            self.word(offset + 8, (u64::from(count) << 32) | u64::from(count));
        }
        fn capacity(&mut self, base: usize, slot: usize, value: i64) {
            self.word(
                Self::container(base, slot) + engine_native_air::ccountry_air_container::CAPACITY_RAW,
                value as u64,
            );
        }
        fn new() -> Self {
            let mut world = Self {
                memory: vec![0; 0xa000 / 8],
            };
            world.word(0, world.at(Self::GAME) as u64);
            world.array(
                Self::GAME + engine_native::cgame_state::COUNTRY_LINKS,
                Self::LINKS,
                4,
            );
            for raw in 1..4 {
                world.half(Self::LINKS + raw * 4, raw as u32);
            }
            world.array(Self::AIR + 0x10, Self::BASES, 2);
            world.half(Self::AIR + engine_native_air::cstrategic_air::COUNTRY, 1);
            world.word(Self::PAIRS, world.at(Self::VECTORS) as u64);
            for index in 0..2 {
                world.word(Self::BASES + index * 8, world.at(Self::base(index)) as u64);
                let containers = Self::base(index) + 0x100;
                world.array(
                    Self::base(index) + engine_native_air::cair_base::COUNTRY_CONTAINERS,
                    containers,
                    4,
                );
                for slot in 1..4 {
                    world.word(
                        containers + slot * 8,
                        world.at(Self::container(index, slot)) as u64,
                    );
                    world.capacity(index, slot, 100_000);
                }
                world.array(Self::VECTORS + index * 0x18, Self::entries(index), 2);
                for (entry, raw) in [2, 3].into_iter().enumerate() {
                    world.half(Self::entries(index) + entry * 16, raw);
                    world.word(Self::entries(index) + entry * 16 + 8, 100_000);
                }
            }
            world.word(Self::base(1) + engine_native_air::cair_base::STATE, 0x77);
            world
        }
        fn pass(&self) {
            BASE.store(
                self.at(0) - engine_native::cgame_state::SINGLETON_RVA.0,
                Ordering::Release,
            );
            ORIGINAL[CAPACITY_SITE].store(store as *const () as usize, Ordering::Release);
            DAILY_ORIGINAL.store(native_daily as *const () as usize, Ordering::Release);
            unsafe {
                daily_refresh_base_capacities(self.at(Self::AIR), self.at(Self::PAIRS), 0, 0)
            };
        }
    }

    #[doc = " The native capacity store: the slot container's capacity."]
    unsafe extern "system" fn store(base: usize, capacity: usize, tag: usize, _: usize) -> usize {
        let containers = unsafe {
            *((base + engine_native_air::cair_base::COUNTRY_CONTAINERS) as *const *const usize)
        };
        let slot = unsafe { *(tag as *const u32) } as usize;
        let container = unsafe { *containers.add(slot) };
        unsafe {
            *((container + engine_native_air::ccountry_air_container::CAPACITY_RAW) as *mut i64) =
                capacity as i64
        };
        0
    }

    static FLIGHTS_SEEN: AtomicUsize = AtomicUsize::new(0);

    #[doc = " Daily air base capacity refresh without its withdrawal: every pair through the patched"]
    #[doc = " setter, in base order."]
    unsafe extern "system" fn native_daily(air: usize, pairs: usize, _: usize, _: usize) -> usize {
        FLIGHTS_SEEN.store(active(), Ordering::SeqCst);
        unsafe {
            let bases = *((air + 0x10) as *const usize);
            let count = *((air + 0x1c) as *const i32) as usize;
            let vectors = *(pairs as *const usize);
            for index in 0..count {
                let base = *((bases + index * 8) as *const usize);
                let vector = vectors + index * 0x18;
                let data = *(vector as *const usize);
                for entry in 0..*((vector + 0xc) as *const i32) as usize {
                    let tag = data + entry * 16;
                    set_country_pool_capacity(base, *((tag + 8) as *const usize), tag, 0);
                }
            }
        }
        0
    }

    #[test]
    fn a_certified_pass_is_thread_local_and_restores_nested_scopes() {
        assert!(!CERTIFIED_PASS.get());
        let outer = CertifiedPass::enter();
        {
            let _nested = CertifiedPass::enter();
            assert!(CERTIFIED_PASS.get());
        }
        assert!(CERTIFIED_PASS.get());
        assert!(!std::thread::spawn(|| CERTIFIED_PASS.get()).join().unwrap());
        drop(outer);
        assert!(!CERTIFIED_PASS.get());
    }

    #[test]
    fn a_certified_daily_pass_counts_its_stores_and_a_changing_pass_marks_per_call() {
        let _serial = DAILY_TEST.lock().unwrap();
        WATCHES.sites.offer(64);
        WATCHES.sites.boundary(true);
        WATCHES.hooks.store(true, Ordering::SeqCst);
        WATCHES.apply();
        let sites = |visit: &mut Vec<usize>| WATCHES.take(Family::Sites, &mut |id| visit.push(id));
        let mut marked = Vec::new();
        let epoch = sites(&mut marked);
        assert!(epoch.is_some());
        let counts = || {
            (
                WATCHES
                    .daily_passes
                    .each_ref()
                    .map(|n| n.load(Ordering::Relaxed)),
                WATCHES.unchanged_capacities.load(Ordering::Relaxed),
            )
        };
        let ([certified, per_call], unchanged) = counts();
        let timed = || crate::hook_timing::counts(crate::hook_timing::Family::RaidChanges).calls;
        let timed_before = timed();
        let untimed_before = WATCHES.untimed_capacities.load(Ordering::Relaxed);
        let world = DailyWorld::new();
        collector_extended_support::hourly::in_tick(|| world.pass());
        assert_eq!(counts(), ([certified + 1, per_call], unchanged + 4));
        assert_eq!(timed(), timed_before + 1);
        assert_eq!(
            WATCHES.untimed_capacities.load(Ordering::Relaxed),
            untimed_before + 4
        );
        assert_eq!(FLIGHTS_SEEN.load(Ordering::SeqCst), 1);
        assert_eq!(active(), 0);
        marked.clear();
        assert_eq!(sites(&mut marked), epoch);
        assert!(marked.is_empty());
        assert!(!CERTIFIED_PASS.get());

        world.pass();
        assert_eq!(counts(), ([certified + 1, per_call + 1], unchanged + 8));
        assert_eq!(timed(), timed_before + 6);
        assert_eq!(
            WATCHES.untimed_capacities.load(Ordering::Relaxed),
            untimed_before + 4
        );
        assert_eq!(FLIGHTS_SEEN.load(Ordering::SeqCst), 1);
        assert_eq!(active(), 0);

        let mut world = DailyWorld::new();
        world.word(DailyWorld::entries(1) + 16 + 8, 120_000);
        collector_extended_support::hourly::in_tick(|| world.pass());
        assert_eq!(counts(), ([certified + 1, per_call + 2], unchanged + 8 + 3));
        assert_eq!(timed(), timed_before + 11);
        assert_eq!(
            WATCHES.untimed_capacities.load(Ordering::Relaxed),
            untimed_before + 4
        );
        assert_eq!(FLIGHTS_SEEN.load(Ordering::SeqCst), 1);
        assert_eq!(active(), 0);
        marked.clear();
        assert_eq!(sites(&mut marked), epoch);
        assert_eq!(marked, [3]);
        assert_eq!(
            world.memory[(DailyWorld::container(1, 3)
                + engine_native_air::ccountry_air_container::CAPACITY_RAW)
                / 8],
            120_000
        );
        WATCHES.hooks.store(false, Ordering::SeqCst);
        WATCHES.apply();
    }

    #[test]
    fn an_invalidating_boundary_revokes_both_sets() {
        let watches = Watches::enabled(64, 64);
        let states = take(&watches, Family::States).0;
        let countries = take(&watches, Family::Countries).0;
        watches.boundary(false);
        assert_eq!(take(&watches, Family::States).0, states);
        watches.boundary(true);
        assert_ne!(take(&watches, Family::States).0, states);
        assert_ne!(take(&watches, Family::Countries).0, countries);
    }
}
