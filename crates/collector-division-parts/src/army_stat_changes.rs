//! Allocation-free notifications from the native writers of every division's stat block
//! (`cunit::STAT_BLOCK`) whose attack operands and maxima DivisionStatus records, and of its name
//! member (`carmy_combat_inputs::NAME_GROUP_MEMBER`), whose span and cached-name lookup cache it
//! records ([`engine_mutations::army_stat_mutations`]). A detour only notifies the shared marking watch
//! registry ([`WatchRegistry::marking`]) for the `CArmy` or member its first argument names, or
//! starts an epoch for the country-wide name refresh, counts the call and calls the original; it
//! never changes game behaviour. The writers run on the game thread and on the TBB workers of the
//! hourly update, which the registry's atomics serve. Receivers are the armies and members a
//! DivisionStatus capture registered. Anything unobserved (disabled, poisoned, a refused
//! registration, an epoch change, a running writer) makes the capture read that division afresh.
use crate::{
    hook::Hook,
    hook_timing::CallCounters,
    watch_registry::{Notification, WatchRegistry},
};
use collector_observers::unit_history_changes;
use engine_mutations::army_stat_mutations::MUTATIONS;
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const EXPERIENCE_FAILURE_NAMES: [&str; 7] = [
    "division experience census identity unavailable",
    "division experience writer certificate unavailable",
    "division experience address overflow",
    "division experience unreadable",
    "division experience cache capacity exhausted",
    "division experience cache allocation failed",
    "division experience index allocation failed",
];
static EXPERIENCE_FAILURES: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];
const PROGRESSION_FAILURE_NAMES: [&str; 12] = [
    EXPERIENCE_FAILURE_NAMES[0],
    EXPERIENCE_FAILURE_NAMES[1],
    EXPERIENCE_FAILURE_NAMES[2],
    EXPERIENCE_FAILURE_NAMES[3],
    EXPERIENCE_FAILURE_NAMES[4],
    EXPERIENCE_FAILURE_NAMES[5],
    EXPERIENCE_FAILURE_NAMES[6],
    "division previous template pointer unreadable",
    "module address overflow",
    "division template vtable mismatch",
    "division template handle unreadable",
    "non-null division template has null handle",
];
static PROGRESSION_FAILURES: [AtomicU64; 12] = [const { AtomicU64::new(0) }; 12];
pub fn progression_refused(reason: &'static str) {
    if let Some(index) = PROGRESSION_FAILURE_NAMES
        .iter()
        .position(|name| *name == reason)
    {
        PROGRESSION_FAILURES[index].fetch_add(1, Ordering::Relaxed);
    }
}
pub fn experience_refused(reason: &'static str) {
    if let Some(index) = EXPERIENCE_FAILURE_NAMES
        .iter()
        .position(|name| *name == reason)
    {
        EXPERIENCE_FAILURES[index].fetch_add(1, Ordering::Relaxed);
    }
}

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static EXPERIENCE_PATCHED: AtomicBool = AtomicBool::new(false);
static EXPERIENCE_FORWARDED: AtomicBool = AtomicBool::new(false);
fn experience_enabled() {
    WATCHES.experience.set_enabled(
        EXPERIENCE_PATCHED.load(Ordering::Acquire) && EXPERIENCE_FORWARDED.load(Ordering::Acquire),
    );
}
fn experience_patched(enabled: bool) {
    EXPERIENCE_PATCHED.store(enabled, Ordering::Release);
    experience_enabled();
}
/// The existing combat-damage patch must cover casualty XP loss.
pub fn experience_forwarder(enabled: bool) {
    EXPERIENCE_FORWARDED.store(enabled, Ordering::Release);
    experience_enabled();
}
/// The combat-damage patch forwards its receiver across the native original.
pub fn experience_damage(army: usize) -> (Notification<'static>, Notification<'static>) {
    // The same native damage writer updates the panel's retained counter.
    (
        WATCHES.experience.notify(army),
        unit_history_changes::notify(army),
    )
}

static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Reused, read, verified and mismatched divisions; refused reads; witnessed
/// divisions; divisions whose status maxima, and whose name members, were
/// carried.
static READS: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
/// The family's sampled comparisons.
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "carried DivisionStatus attack operands, maxima or name member differ from a fresh read",
]);
/// Divisions the latest complete capture visited, for off-thread sizing.
static DIVISIONS: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
pub(crate) static HISTORY_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One observation domain: the watched armies and the writer's calls.
pub struct Watches {
    registry: WatchRegistry,
    experience: WatchRegistry,
    progression: WatchRegistry,
    calls: CallCounters<SITES>,
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
            experience: WatchRegistry::new(),
            progression: WatchRegistry::new(),
            calls: CallCounters::new(),
        }
    }

    /// Hook path, allocation-free: the writer of `site` ([`MUTATIONS`]
    /// index) is about to rewrite `receiver`, an army's stat block or a name
    /// member. Dropping the guard notifies its return.
    pub fn notify(
        &self,
        receiver: usize,
        site: usize,
    ) -> (Notification<'_>, Option<Notification<'_>>) {
        self.calls.increment(site);
        let progression = matches!(
            site,
            6 | 7 | engine_mutations::army_stat_mutations::EXPERIENCE_TEMPLATE_SITE
        )
        .then(|| self.progression.notify(receiver));
        let primary = if site == engine_mutations::army_stat_mutations::EXPERIENCE_TEMPLATE_SITE {
            self.experience.notify(receiver)
        } else if site >= engine_mutations::army_stat_mutations::HISTORY_FIRST {
            unit_history_changes::notify(receiver)
        } else if site >= engine_mutations::army_stat_mutations::EXPERIENCE_FIRST {
            self.experience.notify(receiver)
        } else {
            self.registry.notify(receiver)
        };
        (primary, progression)
    }

    /// Hook path, allocation-free: the writer of `site` rewrites members it
    /// does not name, revoking every certificate on entry and on return.
    pub fn notify_every(&self, site: usize) -> Notification<'_> {
        self.calls.increment(site);
        self.registry.notify_every()
    }

    /// An enabled domain with a table for `armies` receivers installed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(armies: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(armies);
        watches.registry.boundary(false);
        watches.registry.set_enabled(true);
        watches.experience.offer(armies);
        watches.experience.boundary(false);
        watches.experience.set_enabled(true);
        watches.progression.offer(armies);
        watches.progression.boundary(false);
        watches.progression.set_enabled(true);
        watches
    }

    /// The hourly boundary; `invalidate` starts an epoch.
    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
        self.experience.boundary(invalidate);
        self.progression.boundary(invalidate);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }
}

/// What a capture sees of the notifications.
pub trait Changes: Sync {
    fn progression_sequence(&self, _receiver: usize) -> Option<(u64, u64)> {
        None
    }
    /// Independent stored-experience certificate. An absent observer refuses
    /// acquisition instead of turning it into periodic polling.
    fn experience_sequence(&self, _receiver: usize) -> Option<(u64, u64)> {
        None
    }
    /// Quiet certificates of receivers already registered before range reads.
    /// Missing receivers still take ordinary registration in census order.
    fn experience_observed_sequence(&self, _receiver: usize) -> Option<(u64, u64)> {
        None
    }
    fn progression_observed_sequence(&self, _receiver: usize) -> Option<(u64, u64)> {
        None
    }
    /// Before any read of the hour: visits every registered army and member
    /// a hooked writer touched since the previous take, clearing its mark,
    /// and returns the epoch they were marked in ([`WatchRegistry::take`]).
    /// A receiver registered in that epoch and not visited since was touched
    /// by no observed writer. `None` when nothing taken certifies anything.
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64>;
    /// Registers a receiver before its read; whether it is now watched in
    /// the epoch of the hour's take.
    fn register(&self, receiver: usize) -> bool;
    /// A fresh read disagreed with carried operands: disable reuse for the
    /// attachment.
    fn mismatch(&self);
}

impl Changes for Watches {
    fn progression_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        self.progression.sequence(receiver)
    }
    fn experience_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        self.experience.sequence(receiver)
    }
    fn experience_observed_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        self.experience.observed_sequence(receiver)
    }
    fn progression_observed_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        self.progression.observed_sequence(receiver)
    }
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        self.registry.take(visit)
    }
    fn register(&self, receiver: usize) -> bool {
        self.registry.register(receiver).is_some()
    }
    fn mismatch(&self) {
        self.registry.poison();
    }
}

/// The hooks' domain.
pub struct Observer;
impl Changes for Observer {
    fn progression_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        WATCHES.progression.sequence(receiver)
    }
    fn experience_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        WATCHES.experience.sequence(receiver)
    }
    fn experience_observed_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        WATCHES.experience.observed_sequence(receiver)
    }
    fn progression_observed_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        WATCHES.progression.observed_sequence(receiver)
    }
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        WATCHES.take(visit)
    }
    fn register(&self, receiver: usize) -> bool {
        WATCHES.register(receiver)
    }
    fn mismatch(&self) {
        WATCHES.mismatch();
    }
}

/// Whether captures read every carryable division afresh and compare,
/// rather than the hour's sampled slice ([`crate::sampled_verify`]).
pub const VERIFY: bool = cfg!(feature = "army_stat_verify");

/// Whether the off-by-default diagnostic records the native operands
/// (`DivisionStatus.attack`) of another slice of divisions each hour, which
/// the recorder's derivation check compares with the derived ones.
pub const WITNESS: bool = cfg!(feature = "attack_witness");

/// Notifications running now.
pub fn active() -> usize {
    WATCHES.registry.active()
        + WATCHES.experience.active()
        + WATCHES.progression.active()
        + unit_history_changes::active()
        + collector_observers::division_panel_changes::active()
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    unit_history_changes::boundary(invalidate);
    collector_observers::division_panel_changes::boundary(invalidate);
    WATCHES.registry.boundary(invalidate);
    WATCHES.experience.boundary(invalidate);
    WATCHES.progression.boundary(invalidate);
}

/// The existing live naming census prepares all division receiver domains
/// before the first boundary, independently of naming-payload availability.
pub fn demand_divisions(divisions: usize) {
    DIVISIONS.fetch_max(divisions, Ordering::Relaxed);
    presize();
}

/// Off the game thread: size the watches from the live census and later
/// complete captures. A larger offered table waits for an invalidating hour;
/// the initial offer must already cover every division receiver domain.
pub fn presize() {
    let divisions = DIVISIONS.load(Ordering::Relaxed);
    unit_history_changes::demand(divisions);
    unit_history_changes::presize();
    collector_observers::division_panel_changes::demand(
        divisions,
        divisions,
        divisions.saturating_mul(2),
    );
    collector_observers::division_panel_changes::presize();
    if divisions != 0 {
        // Each division registers its army and its name member.
        WATCHES.registry.offer(divisions.saturating_mul(2));
        WATCHES.experience.offer(divisions);
        WATCHES.progression.offer(divisions);
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
    WATCHES.experience.collect();
    WATCHES.progression.collect();
}

/// One capture's divisions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub verified: u64,
    pub mismatches: u64,
    pub refused: u64,
    pub witnessed: u64,
    pub maxima_reused: u64,
    pub names_reused: u64,
}

/// A worker publishes one capture's counts and, when it visited every
/// division, their number for sizing.
pub fn record(counts: &Counts, divisions: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.verified,
        counts.mismatches,
        counts.refused,
        counts.witnessed,
        counts.maxima_reused,
        counts.names_reused,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    SAMPLED.record(counts.verified, counts.mismatches, 0);
    if let Some(divisions) = divisions {
        DIVISIONS.store(divisions, Ordering::Relaxed);
    }
}

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .enumerate()
            .map(|(index, site)| (site.name, WATCHES.calls.total(index))),
    );
    let reads = READS.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "army_stat_changes"),
        "verify": VERIFY,
        "witness": WITNESS,
        "poisoned": WATCHES.registry.poisoned(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "reused_divisions": reads[0],
        "read_divisions": reads[1],
        "verified_divisions": reads[2],
        "mismatches": reads[3],
        "refused_divisions": reads[4],
        "witnessed_divisions": reads[5],
        "maxima_reused_divisions": reads[6],
        "names_reused_divisions": reads[7],
        "events": events,
        "watches": WATCHES.registry.status(),
        "unit_history": unit_history_changes::status(),
        "division_panel": collector_observers::division_panel_changes::status(),
        "experience_watches": WATCHES.experience.status(),
        "progression_watches": WATCHES.progression.status(),
        "progression_refusals": crate::status::map(PROGRESSION_FAILURE_NAMES.into_iter().zip(PROGRESSION_FAILURES.iter().map(|n| n.load(Ordering::Relaxed)))),
        "experience_refusals": crate::status::map(EXPERIENCE_FAILURE_NAMES.into_iter().zip(EXPERIENCE_FAILURES.iter().map(|n| n.load(Ordering::Relaxed)))),
        "scope": "cumulative since attach; a division's DivisionStatus.attack_stats are carried when the hour's take of the marks returned the held capture's epoch, its army has the address and stat block of the held read and no hooked update_active_stats marked it since; every other division's stat block is read; every hour also reads the carryable divisions of its slice afresh (one in rotation_hours by division id; every one in verify mode), publishes the fresh operands and disables reuse at the first mismatch (sampled_*); witnessed_divisions counts the attack_witness diagnostic's native operands; maxima_reused_divisions counts divisions whose DivisionStatus maxima (same block, same writer, same marks) were carried rather than read, and names_reused_divisions those whose name member span and cached-name lookup cache text (marked on the member by cached-name refresh, the country-wide refresh moving the epoch) were; the slice's carryable maxima and members are read and compared too"
    })
}

// Only a failed attach/detach allocates this diagnostic, retained for the runtime's
// static failure interface. Publication is atomic across the listed sites.
fn hook_failure(site: Option<usize>, action: &str, detail: &str) -> &'static str {
    let sites = site.map_or_else(
        || {
            MUTATIONS
                .iter()
                .map(|site| site.name)
                .collect::<Vec<_>>()
                .join(", ")
        },
        |index| MUTATIONS[index].name.to_owned(),
    );
    Box::leak(
        format!("army_stat_changes [{sites}] hook {action} failed: {detail}").into_boxed_str(),
    )
}

fn validate_sites(memory: &impl Memory, base: usize) -> Result<(), &'static str> {
    for (index, site) in MUTATIONS.iter().enumerate() {
        if engine_native::read::<[u8; 16]>(memory, base + site.rva) != Some(site.sentinel) {
            return Err(hook_failure(
                Some(index),
                "sentinel validation",
                "native bytes differ",
            ));
        }
    }
    Ok(())
}

pub struct Hooks {
    hooks: Vec<Hook>,
    operands: crate::panel_operand_hooks::Hooks,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// A differing sentinel refuses installation with the owning family and site.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "army_stat_changes") {
            validate_sites(memory, base)?;
            let callbacks: [usize; SITES] = [
                update_active_stats as *const () as usize,
                refresh_cached_name as *const () as usize,
                refresh_country_names as *const () as usize,
                award_experience as *const () as usize,
                add_manpower_experience as *const () as usize,
                refresh_experience as *const () as usize,
                rebind_experience as *const () as usize,
                construct_division as *const () as usize,
                construct_division_history as *const () as usize,
                destroy_division_history as *const () as usize,
                append_division_history as *const () as usize,
                apply_division_award as *const () as usize,
                clear_medal_inheritance as *const () as usize,
                construct_medal_store as *const () as usize,
                destroy_medal_store as *const () as usize,
                promote_division_officer as *const () as usize,
                bind_division_template as *const () as usize,
                add_officer_experience as *const () as usize,
                set_officer_experience as *const () as usize,
            ];
            for (index, site) in MUTATIONS.iter().enumerate() {
                let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
                    .map_err(|reason| hook_failure(Some(index), "construction", &reason))?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self {
            hooks,
            operands: unsafe { crate::panel_operand_hooks::Hooks::new(memory, base) }?,
        })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !self.operands.set_enabled(enabled)? {
            return Ok(false);
        }
        if !enabled {
            WATCHES.registry.set_enabled(false);
            experience_patched(false);
            WATCHES.progression.set_enabled(false);
            unit_history_changes::patched(false);
        }
        // One census and suspension for the whole family, shared with the
        // other families in a runtime round (`hook::shared`): a thread at any
        // of its sites, or in a tap, holds every site back until the next
        // attempt.
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|reason| hook_failure(None, "publication", &reason))?
        {
            return Ok(false);
        }
        WATCHES
            .registry
            .set_enabled(enabled && !self.hooks.is_empty());
        experience_patched(enabled && !self.hooks.is_empty());
        WATCHES
            .progression
            .set_enabled(enabled && !self.hooks.is_empty());
        unit_history_changes::patched(enabled && !self.hooks.is_empty());
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks
            .iter()
            .flat_map(Hook::executable_ranges)
            .chain(self.operands.executable_ranges())
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: like each Hook's relay, the watch tables are dropped only
        // once the hooks were never enabled, or were removed and drained
        // after the last capture.
        unsafe {
            WATCHES.registry.release();
            WATCHES.experience.release();
            WATCHES.progression.release();
            unit_history_changes::release();
        };
    }
}

// `update active stats(army)` takes the primary CArmy in
// RCX and returns nothing a caller reads; the detour forwards RCX..R9 and
// RAX. The notification covers the original in flight.
unsafe extern "system" fn update_active_stats(
    army: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ArmyStatChanges);
    let _notification = WATCHES.notify(army, 0);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(army, second, third, fourth) })
}

// `refresh cached name(member, ...)` takes the
// member in RCX; members of ships, operatives and lines are never registered.
unsafe extern "system" fn refresh_cached_name(
    member: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ArmyStatChanges);
    let _notification = WATCHES.notify(member, 1);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[1].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(member, second, third, fourth) })
}

// `refresh country names(group, country, ...)` rewrites the members of a country's
// divisions without naming them: every certificate is revoked.
unsafe extern "system" fn refresh_country_names(
    first: usize,
    country: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ArmyStatChanges);
    let _notification = WATCHES.notify_every(2);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[2].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(first, country, third, fourth) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_division_demand_prepares_all_career_receivers_before_first_capture() {
        let _serial = HISTORY_TEST.lock().unwrap();
        unit_history_changes::patched(true);
        unit_history_changes::forwarder(true);
        experience_patched(true);
        experience_forwarder(true);
        WATCHES.progression.set_enabled(true);
        demand_divisions(5_000);
        boundary(true);
        for index in 0..5_000 {
            let army = 0x1000 + index * 0x1000;
            for receiver in [
                army,
                army + engine_native::unit_history::OFFICER,
                army + engine_native::unit_history::DIVISION_HISTORY,
                0x1000_0000 + index * 0x1000,
            ] {
                assert!(unit_history_changes::sequence(receiver).is_some());
            }
            assert!(WATCHES.experience.sequence(army).is_some());
            assert!(WATCHES.progression.sequence(army).is_some());
        }
        let receiver = 0x1000 + engine_native::unit_history::OFFICER;
        let held = unit_history_changes::sequence(receiver).unwrap();
        boundary(false);
        assert_eq!(unit_history_changes::sequence(receiver), Some(held));
        drop(unit_history_changes::notify(receiver));
        assert_ne!(unit_history_changes::sequence(receiver), Some(held));
        unit_history_changes::patched(false);
        unit_history_changes::forwarder(false);
        experience_patched(false);
        experience_forwarder(false);
        WATCHES.progression.set_enabled(false);
    }

    #[test]
    fn sentinel_refusals_identify_the_exact_site_before_any_patch_is_built() {
        struct Refuses(usize);
        impl Memory for Refuses {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                let Some((index, site)) = MUTATIONS
                    .iter()
                    .enumerate()
                    .find(|(_, site)| site.rva == address)
                else {
                    return false;
                };
                if index == self.0 || destination.len() != site.sentinel.len() {
                    return false;
                }
                destination.copy_from_slice(&site.sentinel);
                true
            }
        }
        for (index, site) in MUTATIONS.iter().enumerate() {
            let error = validate_sites(&Refuses(index), 0).unwrap_err();
            assert!(error.contains("army_stat_changes"));
            assert!(error.contains(site.name));
            assert!(error.contains("sentinel validation"));
        }
    }

    #[test]
    fn hook_errors_name_the_family_sites_and_original_failure() {
        for (index, site) in MUTATIONS.iter().enumerate() {
            let error = hook_failure(Some(index), "construction", "synthetic refusal");
            assert!(error.contains("army_stat_changes"));
            assert!(error.contains(site.name));
            assert!(error.contains("synthetic refusal"));
        }
        let error = hook_failure(None, "publication", "byte verification failed");
        for site in MUTATIONS {
            assert!(error.contains(site.name));
        }
        assert!(error.contains("byte verification failed"));
    }

    #[test]
    fn experience_awards_carry_the_independent_template_conversion_certificate() {
        let watches = Watches::enabled(2);
        let mut held = watches.progression_sequence(0x1000).unwrap();
        let other = watches.progression_sequence(0x2000).unwrap();
        for site in [3, 4, 5, 0] {
            drop(watches.notify(0x1000, site));
            assert_eq!(watches.progression_sequence(0x1000), Some(held));
        }
        for site in [
            6,
            7,
            engine_mutations::army_stat_mutations::EXPERIENCE_TEMPLATE_SITE,
        ] {
            let running = watches.notify(0x1000, site);
            assert!(watches.progression_sequence(0x1000).is_none());
            drop(running);
            let next = watches.progression_sequence(0x1000).unwrap();
            assert_ne!(next, held);
            assert_eq!(watches.progression_sequence(0x2000), Some(other));
            held = next;
        }
    }

    #[test]
    fn experience_writers_do_not_dirty_stat_operands_or_other_divisions() {
        let watches = Watches::enabled(16);
        let first = watches.experience_sequence(0x1000).unwrap();
        let second = watches.experience_sequence(0x2000).unwrap();
        let mut ignored = |_| {};
        let epoch = watches.take(&mut ignored).unwrap();
        assert!(watches.register(0x1000));
        for site in [
            engine_mutations::army_stat_mutations::EXPERIENCE_FIRST,
            engine_mutations::army_stat_mutations::EXPERIENCE_TEMPLATE_SITE,
        ] {
            drop(watches.notify(0x1000, site));
        }
        assert_ne!(watches.experience_sequence(0x1000), Some(first));
        assert_eq!(watches.experience_sequence(0x2000), Some(second));
        let mut touched = Vec::new();
        assert_eq!(
            watches.take(&mut |receiver| touched.push(receiver)),
            Some(epoch)
        );
        assert!(touched.is_empty());
        let held = watches.experience_sequence(0x1000);
        drop(watches.notify(0x1000, 0));
        assert_eq!(watches.experience_sequence(0x1000), held);
        watches.boundary(true);
        assert_ne!(watches.experience_sequence(0x1000), held);
    }

    /// The writer moves only the army it names; a running writer certifies
    /// nothing, and a mismatch poisons the domain.
    #[test]
    fn the_writer_marks_only_its_registered_receiver() {
        let watches = Watches::enabled(16);
        let take = |watches: &Watches| {
            let mut touched = Vec::new();
            let epoch = Changes::take(watches, &mut |receiver| touched.push(receiver));
            (epoch, touched)
        };
        let (epoch, _) = take(&watches);
        assert!([0x1000, 0x2000].map(|army| Changes::register(&watches, army)) == [true; 2]);
        drop(watches.notify(0x2000, 0));
        // An unregistered receiver is not watched.
        drop(watches.notify(0x3000, 1));
        assert_eq!(take(&watches), (epoch, vec![0x2000]));
        assert_eq!(take(&watches), (epoch, vec![]));
        assert_eq!(watches.calls.total(0), 1);
        // A running writer certifies nothing.
        let running = watches.notify(0x1000, 0);
        assert_eq!(take(&watches).0, None);
        drop(running);
        assert_eq!(take(&watches), (epoch, vec![0x1000]));
        // The country-wide refresh and an invalidating hour move the epoch.
        drop(watches.notify_every(2));
        let (moved, _) = take(&watches);
        assert!(moved.is_some() && moved != epoch);
        watches.boundary(true);
        assert_ne!(take(&watches).0, moved);
        Changes::mismatch(&watches);
        assert!(watches.poisoned());
        assert_eq!(take(&watches).0, None);
    }
}

macro_rules! experience_callback {
    ($name:ident, $index:expr) => {
        unsafe extern "system" fn $name(
            army: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::ArmyStatChanges);
            let _notification = WATCHES.notify(army, $index);
            let _edit = matches!($index, 8 | 9 | 12 | 13 | 14)
                .then(|| unit_history_changes::notify_edit(army));
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$index].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(army, second, third, fourth) })
        }
    };
}
experience_callback!(award_experience, 3);
experience_callback!(add_manpower_experience, 4);
experience_callback!(refresh_experience, 5);
experience_callback!(rebind_experience, 6);
experience_callback!(construct_division, 7);

experience_callback!(construct_division_history, 8);
experience_callback!(destroy_division_history, 9);
experience_callback!(append_division_history, 10);
experience_callback!(clear_medal_inheritance, 12);
experience_callback!(construct_medal_store, 13);
experience_callback!(destroy_medal_store, 14);
experience_callback!(add_officer_experience, 17);
experience_callback!(set_officer_experience, 18);

// Award application has two stack arguments in addition to the register arguments.
unsafe extern "system" fn apply_division_award(
    store: usize,
    second: usize,
    third: usize,
    fourth: usize,
    entry: usize,
    pay_cost: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ArmyStatChanges);
    let _notification = WATCHES.notify(store, 11);
    let _edit = unit_history_changes::notify_edit(entry);
    let original: unsafe extern "system" fn(usize, usize, usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[11].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(store, second, third, fourth, entry, pay_cost) })
}

// Officer promotion mutates the history in its second argument, including its store binding.
unsafe extern "system" fn promote_division_officer(
    leader: usize,
    history: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ArmyStatChanges);
    let _notification = WATCHES.notify(history, 15);
    let _edit = unit_history_changes::notify_edit(history);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[15].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(leader, history, third, fourth) })
}

experience_callback!(
    bind_division_template,
    engine_mutations::army_stat_mutations::EXPERIENCE_TEMPLATE_SITE
);
