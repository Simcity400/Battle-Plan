use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::country_input_mutations as countries;
use engine_native_memory::Memory;
use engine_native::cwar_relation::goals::{
    inventory as input,
    mutations::{self, Receiver},
};
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

static DAILY_UPDATE_OBSERVED: AtomicBool = AtomicBool::new(false);
static EXPIRY_CALLS: AtomicU64 = AtomicU64::new(0);
static PATCHED: AtomicBool = AtomicBool::new(false);
static COUNTRY_RESET_FORWARDER: AtomicBool = AtomicBool::new(false);
static RESET_FORWARDER: AtomicBool = AtomicBool::new(false);
static WATCHES: [WatchRegistry; 2] = [const { WatchRegistry::new() }; 2];
pub type Certificate = [(u64, u64); 2];
pub struct Guard {
    _goals: Notification<'static>,
    _justifications: Notification<'static>,
}
static ORIGINAL: [AtomicUsize; mutations::SITES.len()] =
    [const { AtomicUsize::new(0) }; mutations::SITES.len()];
static BASE: AtomicUsize = AtomicUsize::new(0);
static DEMAND: AtomicUsize = AtomicUsize::new(0);
static READS: AtomicU64 = AtomicU64::new(0);
static CARRIED: AtomicU64 = AtomicU64::new(0);
static INVENTORY_BYTES: AtomicUsize = AtomicUsize::new(0);
static CALLS: [AtomicU64; mutations::SITES.len()] =
    [const { AtomicU64::new(0) }; mutations::SITES.len()];

pub fn reset_forwarder(enabled: bool) {
    RESET_FORWARDER.store(enabled, Ordering::SeqCst);
    observe();
}
pub fn country_reset_forwarder(enabled: bool) {
    COUNTRY_RESET_FORWARDER.store(enabled, Ordering::SeqCst);
    observe();
}
pub fn country_reset(country: usize) -> Guard {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let mut id = None;
    let named = countries::countries(
        &memory,
        BASE.load(Ordering::Acquire),
        countries::Receiver::Country,
        country,
        0,
        &mut |value| id = Some(value),
    );
    notify_both(if named { id } else { None })
}
fn observe() {
    let enabled = PATCHED.load(Ordering::SeqCst)
        && RESET_FORWARDER.load(Ordering::SeqCst)
        && COUNTRY_RESET_FORWARDER.load(Ordering::SeqCst)
        && DAILY_UPDATE_OBSERVED.load(Ordering::SeqCst);
    for watches in &WATCHES {
        watches.set_enabled(enabled);
    }
}

pub fn goals_quiet() -> Option<(u64, u64)> {
    WATCHES[0].quiet()
}

pub fn goal_sequence(country: u32) -> Option<(u64, u64)> {
    WATCHES[0].sequence(watch_key(country))
}

pub fn certificate(country: u32) -> Option<Certificate> {
    Some([
        WATCHES[0].sequence(watch_key(country))?,
        WATCHES[1].sequence(watch_key(country))?,
    ])
}

fn watch_key(country: u32) -> usize {
    (country as usize + 1) * size_of::<u64>()
}

pub fn note_countries(count: usize) {
    DEMAND.fetch_max(count, Ordering::Relaxed);
}
pub fn presize() {
    let demand = DEMAND.load(Ordering::Relaxed);
    if demand == 0 {
        return;
    }
    for watches in &WATCHES {
        watches.offer(demand);
    }
}
pub fn collect() {
    for watches in &WATCHES {
        watches.collect();
    }
}
pub fn boundary(invalidate: bool) {
    for watches in &WATCHES {
        watches.boundary(invalidate);
    }
}
pub fn active() -> usize {
    WATCHES.iter().map(WatchRegistry::active).sum()
}
pub fn record(carried: bool) {
    (if carried { &CARRIED } else { &READS }).fetch_add(1, Ordering::Relaxed);
}
pub fn note_inventory_bytes(bytes: usize) {
    INVENTORY_BYTES.store(bytes, Ordering::Relaxed);
}

pub fn diplomacy_reset(diplomacy: usize) -> Guard {
    notify_both(named(Receiver::Diplomacy, diplomacy, 0))
}
fn notify_both(country: Option<u32>) -> Guard {
    Guard {
        _goals: notify_country(country, 0),
        _justifications: notify_country(country, 1),
    }
}
fn notify_country(country: Option<u32>, part: usize) -> Notification<'static> {
    notify_in(&WATCHES, country, part)
}
fn notify_in(watches: &[WatchRegistry; 2], country: Option<u32>, part: usize) -> Notification<'_> {
    match country {
        Some(id) => watches[part].notify(watch_key(id)),
        None => watches[part].notify_every(),
    }
}
static DAILY_UPDATE_FORWARDER: crate::occupation_changes::LegitimacyUpdateForwarder =
    crate::occupation_changes::LegitimacyUpdateForwarder {
        notify: expire_goals,
        enabled: |enabled| {
            DAILY_UPDATE_OBSERVED.store(enabled, Ordering::SeqCst);
            observe();
        },
    };

fn expire_goals(diplomacy: usize) -> Option<Notification<'static>> {
    EXPIRY_CALLS.fetch_add(1, Ordering::Relaxed);
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let unused = diplomacy
        .checked_add(engine_native_parts::cdiplomacy_status_layout::OWNED_WARGOALS + 12)
        .and_then(|p| engine_native_memory::read::<u32>(&memory, p));
    expiry_in(&WATCHES, unused, || {
        named(Receiver::Diplomacy, diplomacy, 0)
    })
}

fn expiry_in(
    watches: &[WatchRegistry; 2],
    unused: Option<u32>,
    country: impl FnOnce() -> Option<u32>,
) -> Option<Notification<'_>> {
    (unused != Some(0)).then(|| notify_in(watches, country(), 0))
}

fn named(receiver: Receiver, first: usize, third: usize) -> Option<u32> {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Acquire);
    let (receiver, first) = match receiver {
        Receiver::Diplomacy => (countries::Receiver::DiplomacyStatus, first),
        Receiver::Politics | Receiver::ActivityInsert | Receiver::ActivityPrune => {
            (countries::Receiver::PoliticalStatus, first)
        }
        Receiver::Goal | Receiver::ConsumedGoal | Receiver::Activity => {
            let (p, offset) = match receiver {
                Receiver::ConsumedGoal => (third, input::ACTOR),
                Receiver::Goal => (first, input::ACTOR),
                _ => (first, input::ACTIVITY_SOURCE),
            };
            return p
                .checked_add(offset)
                .and_then(|p| engine_native_memory::read::<u32>(&memory, p))
                .and_then(|raw| countries::of_link(&memory, base, raw));
        }
    };
    let mut id = None;
    if countries::countries(&memory, base, receiver, first, 0, &mut |country| {
        id = Some(country)
    }) {
        id
    } else {
        None
    }
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// # Safety
    /// The host has authenticated the image and owns patch publication and
    /// quiescence for the enclosing country-input hook family.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, String> {
        let mut hooks = Vec::new();
        BASE.store(base, Ordering::Release);
        for site in &mutations::SITES {
            crate::verified(base, site.rva).map_err(|cause| {
                format!(
                    "wargoal_changes hook {} verification failed: {cause}",
                    site.name
                )
            })?;
        }
        {
            let callbacks = [
                grant_goal as *const () as usize,
                remove_country_goals as *const () as usize,
                invalidate_goals as *const () as usize,
                remove_goals as *const () as usize,
                consume_goal as *const () as usize,
                destroy_goal as *const () as usize,
                read_goal as *const () as usize,
                insert_activity as *const () as usize,
                cancel_activity as *const () as usize,
                advance_activity as *const () as usize,
                read_activity as *const () as usize,
                read_justification as *const () as usize,
                destroy_justification as *const () as usize,
                delete_justification as *const () as usize,
                delete_goal as *const () as usize,
                prune_activities as *const () as usize,
            ];
            for (i, site) in mutations::SITES.iter().enumerate() {
                let code = engine_sites::hook(base, site.rva)?;
                let hook = unsafe { Hook::new(code.address, callbacks[i], code.sentinel::<16>()?) }
                    .map_err(|cause| {
                        format!(
                            "wargoal_changes hook {} construction failed: {cause}",
                            site.name
                        )
                    })?;
                ORIGINAL[i].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        crate::occupation_changes::legitimacy_update_forwarder(Some(&DAILY_UPDATE_FORWARDER));
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !enabled {
            PATCHED.store(false, Ordering::SeqCst);
            observe();
        }
        if !Hook::set_family_enabled_shared_named(
            &mut self.hooks,
            enabled,
            "wargoal_changes",
            &mutations::SITES.map(|site| site.name),
        )? {
            return Ok(false);
        }
        PATCHED.store(
            enabled && self.hooks.len() == mutations::SITES.len(),
            Ordering::SeqCst,
        );
        observe();
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        crate::occupation_changes::legitimacy_update_forwarder(None);
        for watches in &WATCHES {
            unsafe { watches.release() };
        }
    }
}

fn justification(vtable: u64) -> bool {
    engine_sites::at(
        BASE.load(Ordering::Acquire),
        input::JUSTIFICATION_VTABLE_RVA,
    )
    .is_none_or(|expected| expected as u64 == vtable)
}
macro_rules! callback {
    ($name:ident,$site:expr) => {
        unsafe extern "system" fn $name(a: usize, b: usize, c: usize, d: usize) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::CountryInputChanges);
            CALLS[$site].fetch_add(1, Ordering::Relaxed);
            let site = &mutations::SITES[$site];
            let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
            let interested = !matches!(site.receiver, Receiver::Activity)
                || engine_native_memory::read::<u64>(&memory, a).is_none_or(justification)
                || site.name == "read_activity";
            let interested = interested
                && (!matches!(site.receiver, Receiver::ActivityInsert)
                    || engine_native_memory::read::<u64>(&memory, b).is_none_or(justification))
                && (!matches!(site.receiver, Receiver::ActivityPrune)
                    || a.checked_add(input::ACTIVITIES + 12)
                        .and_then(|p| engine_native_memory::read::<u32>(&memory, p))
                        != Some(0));
            let _guard = interested
                .then(|| notify_country(named(site.receiver, a, c), site.receiver.part()));
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(a, b, c, d) })
        }
    };
}

callback!(grant_goal, 0);
callback!(remove_country_goals, 1);
callback!(invalidate_goals, 2);
callback!(remove_goals, 3);
callback!(consume_goal, 4);
callback!(destroy_goal, 5);
callback!(read_goal, 6);
callback!(insert_activity, 7);
callback!(cancel_activity, 8);
callback!(advance_activity, 9);
callback!(read_activity, 10);
callback!(read_justification, 11);
callback!(destroy_justification, 12);
callback!(delete_justification, 13);
callback!(delete_goal, 14);
callback!(prune_activities, 15);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forwarded_daily_expiry_keeps_the_nonempty_demand_and_before_after_goal_guard() {
        let watches = [WatchRegistry::new(), WatchRegistry::new()];
        for watch in &watches {
            watch.offer(64);
            watch.set_enabled(true);
            watch.boundary(true);
        }
        let before = watches[0].sequence(watch_key(1)).unwrap();
        let other = watches[0].sequence(watch_key(2)).unwrap();
        let activity = watches[1].sequence(watch_key(1)).unwrap();
        assert!(
            expiry_in(&watches, Some(0), || panic!(
                "empty goals do not demand an owner lookup"
            ))
            .is_none()
        );
        assert_eq!(watches[0].sequence(watch_key(1)), Some(before));
        let guard = expiry_in(&watches, Some(1), || Some(1));
        assert!(
            watches[0].sequence(watch_key(1)).is_none(),
            "notification must enclose the original update"
        );
        drop(guard);
        assert_ne!(watches[0].sequence(watch_key(1)), Some(before));
        assert_eq!(watches[0].sequence(watch_key(2)), Some(other));
        assert_eq!(watches[1].sequence(watch_key(1)), Some(activity));
        let before = watches[0].sequence(watch_key(1));
        drop(expiry_in(&watches, None, || None));
        assert_ne!(watches[0].sequence(watch_key(1)), before);
        assert_ne!(watches[0].sequence(watch_key(2)), Some(other));
        for watch in &watches {
            unsafe { watch.release() };
        }
    }

    #[test]
    fn hook_verification_errors_name_family_site_and_cause() {
        let first = &mutations::SITES[0];
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let error = unsafe { Hooks::new(&memory, 0) }
            .err()
            .expect("an unlocated site must name the failed hook");
        assert!(error.contains("wargoal_changes"));
        assert!(error.contains(first.name));
        assert!(error.contains("withheld"));
    }

    #[test]
    fn deleting_the_only_invalid_justification_revokes_carry_without_a_progress_write() {
        let watches = [WatchRegistry::new(), WatchRegistry::new()];
        for watch in &watches {
            watch.offer(64);
            watch.set_enabled(true);
            watch.boundary(true);
        }
        let goal_before = watches[0].sequence(watch_key(1)).unwrap();
        let activity_before = watches[1].sequence(watch_key(1)).unwrap();
        let other_before = watches[1].sequence(watch_key(2)).unwrap();
        let deletion = mutations::SITES
            .iter()
            .find(|s| s.name == "delete_justification")
            .expect("virtual invalid-activity deletion needs its own hook");
        let mut activities = vec![17];
        let guard = notify_in(&watches, Some(1), deletion.receiver.part());
        activities.clear();
        assert!(watches[1].sequence(watch_key(1)).is_none());
        drop(guard);
        assert!(activities.is_empty());
        assert_ne!(watches[1].sequence(watch_key(1)), Some(activity_before));
        assert_eq!(watches[0].sequence(watch_key(1)), Some(goal_before));
        assert_eq!(watches[1].sequence(watch_key(2)), Some(other_before));
        let deletion = mutations::SITES
            .iter()
            .find(|s| s.name == "delete_goal")
            .unwrap();
        drop(notify_in(&watches, Some(1), deletion.receiver.part()));
        assert_ne!(watches[0].sequence(watch_key(1)), Some(goal_before));
        let prune = mutations::SITES
            .iter()
            .find(|s| s.name == "prune_activities")
            .unwrap();
        let before = watches[1].sequence(watch_key(1));
        drop(notify_in(&watches, Some(1), prune.receiver.part()));
        assert_ne!(
            watches[1].sequence(watch_key(1)),
            before,
            "removing another activity can shift a surviving justification ordinal"
        );
        for watch in &watches {
            unsafe { watch.release() };
        }
    }
}
