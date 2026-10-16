//! Allocation-free notifications from the native writers of every ship byte
//! ShipModels serves from a held read ([`engine::ship_model_mutations`]): a
//! ship's fixed fields, pool entries, heap fallback sprite and experience
//! modifiers, its medal store's modifiers and its carrier air base pointer,
//! and of its task force's doctrine tree and its weather region's
//! modifiers, which ShipModels carries with the country's naval tree
//! certificate.
//! A detour only notifies the shared watch registry ([`WatchRegistry`]) for
//! the ship, air base or medal store its receiver argument names, counts the
//! call and calls the original; it never changes game behaviour. The writers
//! run on the game thread and on the TBB workers of the hourly update, which
//! the registry's atomics serve. Receivers are the ships, medal stores and
//! carrier bases a ShipModels capture registered. Anything unobserved
//! (disabled, poisoned, a refused registration, an epoch change, a running
//! writer) makes the capture read that ship afresh.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine::{
    Memory,
    ship_model_mutations::{MUTATIONS, RECEIVER_ARGUMENT},
};
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
/// Air base destruction's index in [`MUTATIONS`], forwarded to
/// [`crate::air_base_carry_changes`].
const DESTROY_AIR_BASE: usize = 4;
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Carried, read, verified and mismatched ships; ships read without a
/// certificate to hold; carried task-force terms; carried rows.
static READS: [AtomicU64; 9] = [const { AtomicU64::new(0) }; 9];
/// The family's sampled comparisons.
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "a carried ship's held fields or modifiers differ from a fresh read",
]);
/// Receivers the latest complete capture registered, for off-thread sizing.
static RECEIVERS: AtomicUsize = AtomicUsize::new(0);

/// One observation domain: the watched receivers and the writers' calls.
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

    /// Hook path, allocation-free: the writer of `site` ([`MUTATIONS`]
    /// index) is about to rewrite `receiver`. Dropping the guard notifies its
    /// return.
    pub fn notify(&self, receiver: usize, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.registry.notify(receiver)
    }

    /// An enabled domain with a table for `receivers` installed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(receivers: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(receivers);
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

/// A ship's certificate: the `(epoch, sequence)` of the ship, its medal
/// store and its carrier air base; an absent receiver is zero.
pub type Certificate = [(u64, u64); 3];

/// Hints for the ship, medal store and carrier base in the current watch
/// table. Only the registry's receiver and epoch checks can certify them.
pub type Slots = [usize; 3];
pub const NO_SLOTS: Slots = [usize::MAX; 3];

/// What a capture sees of the notifications.
pub trait Changes: Sync {
    /// `(epoch, sequence)` of one receiver, registering it; `None` when it
    /// cannot be observed. Equal values across hours prove that no observed
    /// writer touched it.
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)>;
    /// [`Self::sequence`] and an optional slot hint, validated by the
    /// observation domain. Implementations without slots use no hint.
    fn sequence_at(&self, receiver: usize, _held_slot: usize) -> Option<((u64, u64), usize)> {
        self.sequence(receiver)
            .map(|certificate| (certificate, usize::MAX))
    }
    /// `(epoch, revision)` with no observed writer running. Equal tokens
    /// across complete captures spare already-certified receivers a lookup.
    fn quiet(&self) -> Option<(u64, u64)> {
        None
    }
    /// A fresh read disagreed with carried bytes: disable reuse for the
    /// attachment.
    fn mismatch(&self);
    /// `(epoch, sequence)` of a country's (`CCountry+0x5B8`) or commander's
    /// (`CUnitLeader+0x418`) modifier tree, which the naval tree family
    /// certifies.
    fn tree_sequence(&self, country: usize) -> Option<(u64, u64)>;

    /// The certificate of a ship with `receivers` (ship, medal store,
    /// carrier base; zero for none), or `None` when one cannot be observed.
    fn certificate(&self, receivers: [usize; 3]) -> Option<Certificate> {
        let mut certificate = [(0, 0); 3];
        for (held, receiver) in certificate.iter_mut().zip(receivers) {
            if receiver != 0 {
                *held = self.sequence(receiver)?;
            }
        }
        Some(certificate)
    }

    /// [`Self::certificate`] with receiver-validated slot hints. An absent
    /// receiver keeps its zero certificate and has no slot.
    fn certificate_at(
        &self,
        receivers: [usize; 3],
        _held_slots: Slots,
    ) -> Option<(Certificate, Slots)> {
        self.certificate(receivers)
            .map(|certificate| (certificate, NO_SLOTS))
    }
}

impl Changes for Watches {
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        self.registry.sequence(receiver)
    }
    fn sequence_at(&self, receiver: usize, held_slot: usize) -> Option<((u64, u64), usize)> {
        self.registry.sequence_at(receiver, held_slot)
    }
    fn quiet(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }
    fn certificate_at(
        &self,
        receivers: [usize; 3],
        held_slots: Slots,
    ) -> Option<(Certificate, Slots)> {
        let mut certificate = [(0, 0); 3];
        let mut slots = NO_SLOTS;
        for (index, receiver) in receivers.into_iter().enumerate() {
            if receiver != 0 {
                (certificate[index], slots[index]) =
                    self.sequence_at(receiver, held_slots[index])?;
            }
        }
        Some((certificate, slots))
    }
    fn mismatch(&self) {
        self.registry.poison();
    }
    /// Tests observe countries in the same domain.
    fn tree_sequence(&self, country: usize) -> Option<(u64, u64)> {
        self.registry.sequence(country)
    }
}

/// The hooks' domain.
pub struct Observer;
impl Changes for Observer {
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(receiver)
    }
    fn sequence_at(&self, receiver: usize, held_slot: usize) -> Option<((u64, u64), usize)> {
        WATCHES.sequence_at(receiver, held_slot)
    }
    fn quiet(&self) -> Option<(u64, u64)> {
        WATCHES.quiet()
    }
    fn certificate_at(
        &self,
        receivers: [usize; 3],
        held_slots: Slots,
    ) -> Option<(Certificate, Slots)> {
        WATCHES.certificate_at(receivers, held_slots)
    }
    fn mismatch(&self) {
        WATCHES.mismatch();
    }
    fn tree_sequence(&self, country: usize) -> Option<(u64, u64)> {
        crate::naval_tree_changes::sequence(country)
    }
}

/// Whether ShipModels holds ships' reads to carry them.
pub const CARRY: bool = cfg!(feature = "ship_model_changes");

/// Whether captures read every carryable ship afresh and compare, rather
/// than the hour's sampled slice ([`crate::sampled_verify`]).
pub const VERIFY: bool = cfg!(feature = "ship_model_verify");

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

/// Off the game thread: size the watches for the receivers the latest
/// capture registered. Nothing is offered before a capture registered any:
/// a later larger table waits for an invalidating hour, while the first
/// installs at once.
pub fn presize() {
    let receivers = RECEIVERS.load(Ordering::Relaxed);
    if receivers != 0 {
        WATCHES.registry.offer(receivers);
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
}

/// One capture range's ships.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub carried: u64,
    pub read: u64,
    pub verified: u64,
    pub mismatches: u64,
    pub uncertified: u64,
    /// Task-force doctrine contributions and modifier arrays served from a
    /// held read.
    pub task_terms_carried: u64,
    /// Ships whose finished row was carried.
    pub rows_carried: u64,
    /// Carried ships whose model part was carried, only their maximum
    /// organisation computed from certified ship operands and current
    /// task-force terms.
    pub models_carried: u64,
    /// Ships whose certificate moved but whose own objects and modifier
    /// arrays read this hour equal the held read's, carried again.
    pub revalidated: u64,
    /// Receivers the census's ships can register, for sizing.
    pub receivers: usize,
    /// Sampled ships (every [`PATH_SAMPLE`]th row) and their QPC ticks by
    /// path: row carried, model part carried, readers on carried inputs,
    /// read afresh, and (beside its path) revalidated.
    pub path_samples: [u64; PATHS],
    pub path_ticks: [u64; PATHS],
    /// Task-force terms read ahead of a first ship, and their QPC ticks.
    pub terms_reads: u64,
    pub terms_ticks: u64,
    /// Carryable rows that did not carry, by reason: each task-force term
    /// that differed (owner, country, pride ship, doctrine, location, fleet
    /// leader, HQ support), the fleet leader's tree certificate, terms not
    /// read for the task force, and equal terms with changed variants or
    /// organisation modifier eligibility or clamp metadata, and a revalidated
    /// critical header whose live entry payload must be read again.
    pub terms_changed: [u64; REASONS],
}

/// Rows between two timed ships (diagnostics; the timer costs two QPC reads).
pub const PATH_SAMPLE: usize = 16;
/// Sampled paths: row, model, readers, fresh, revalidated.
pub const PATHS: usize = 5;
/// Reasons a carryable row did not carry.
pub const REASONS: usize = 11;
static PATH_TOTALS: [AtomicU64; 2 * PATHS] = [const { AtomicU64::new(0) }; 2 * PATHS];
static TERM_TOTALS: [AtomicU64; 2 + REASONS] = [const { AtomicU64::new(0) }; 2 + REASONS];

/// A worker publishes one range's counts; a complete one also its receivers
/// for sizing, the largest any range reported (the family capture's, whose
/// census is the whole one).
pub fn record(counts: &Counts, complete: bool) {
    for (target, value) in READS.iter().zip([
        counts.carried,
        counts.read,
        counts.verified,
        counts.mismatches,
        counts.uncertified,
        counts.task_terms_carried,
        counts.rows_carried,
        counts.models_carried,
        counts.revalidated,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    SAMPLED.record(counts.verified, counts.mismatches, 0);
    for (target, value) in PATH_TOTALS
        .iter()
        .zip(counts.path_samples.into_iter().chain(counts.path_ticks))
    {
        target.fetch_add(value, Ordering::Relaxed);
    }
    for (target, value) in TERM_TOTALS.iter().zip(
        [counts.terms_reads, counts.terms_ticks]
            .into_iter()
            .chain(counts.terms_changed),
    ) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if complete {
        RECEIVERS.fetch_max(counts.receivers, Ordering::Relaxed);
    }
}

/// QPC ticks to nanoseconds; `None` without a frequency.
fn nanoseconds(ticks: u64) -> Option<u64> {
    let mut frequency = 0;
    // SAFETY: an out pointer to a live local.
    let known = unsafe {
        windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency)
    } != 0;
    (known && frequency > 0).then(|| (u128::from(ticks) * 1_000_000_000 / frequency as u128) as u64)
}

/// Where the ShipModels time goes: sampled ships by path and the
/// task-force terms read ahead of first ships, and why carryable rows did
/// not carry.
fn timing_status() -> impl serde::Serialize {
    let path = |index: usize| {
        crate::status::status!({
            "samples": PATH_TOTALS[index].load(Ordering::Relaxed),
            "ns": nanoseconds(PATH_TOTALS[PATHS + index].load(Ordering::Relaxed)),
        })
    };
    let reason = |index: usize| TERM_TOTALS[2 + index].load(Ordering::Relaxed);
    crate::status::status!({
        "sample_every": PATH_SAMPLE,
        "row_carried": path(0),
        "model_carried": path(1),
        "readers_on_carried_inputs": path(2),
        "read_afresh": path(3),
        "revalidated": path(4),
        "task_terms_read": crate::status::status!({
            "count": TERM_TOTALS[0].load(Ordering::Relaxed),
            "ns": nanoseconds(TERM_TOTALS[1].load(Ordering::Relaxed)),
        }),
        "rows_not_carried": crate::status::status!({
            "owner": reason(0),
            "country": reason(1),
            "pride": reason(2),
            "doctrine": reason(3),
            "location": reason(4),
            "fleet_leader": reason(5),
            "support": reason(6),
            "leader_certificate": reason(7),
            "terms_unread": reason(8),
            "variants": reason(9),
            "revalidated_criticals": reason(10),
        }),
        "scope": "cumulative since attach; every sample_every-th census row is timed by the path it took (a revalidated ship also counts under its path); task_terms_read times the task-force terms read without a ship; rows_not_carried counts carried ships with a carryable held row (no carrier base) whose row did not carry, by each term that differed from the held row's, an unobservable leader certificate change, terms not read for the task force, equal terms with changed or ungathered variant bytes, incomplete optional suffixes, or organisation modifier eligibility or clamp metadata (variants), or a writer notification requiring fresh critical counts and pointees despite equal certified own bytes (revalidated_criticals)",
    })
}

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(&WATCHES.calls)
            .map(|(site, n)| (site.name, n.load(Ordering::Relaxed))),
    );
    let reads = READS.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": CARRY,
        "verify": VERIFY,
        "poisoned": WATCHES.registry.poisoned(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "carried_ships": reads[0],
        "read_ships": reads[1],
        "verified_ships": reads[2],
        "mismatches": reads[3],
        "uncertified_ships": reads[4],
        "carried_task_terms": reads[5],
        "carried_rows": reads[6],
        "carried_models": reads[7],
        "revalidated_ships": reads[8],
        "timing": timing_status(),
        "events": events,
        "watches": WATCHES.registry.status(),
        "scope": "cumulative since attach; a ship's finished row is carried when its handle, task force and receiver certificates hold, its task force's terms read this hour equal those of its held row, its selected variant gathers and optional suffixes are complete, its variant bytes are unchanged, its present organisation modifier keys keep their current eligibility and clamp metadata, and it has no carrier base; compact held records follow census order with a sorted-index fallback, domain quiet tokens rechecked before each ship spare receiver lookups, and validated slot indices spare watch hashing otherwise; task-force terms are compared once per task force, variants once per distinct held observation and organisation modifier eligibility and clamp metadata once per distinct guard per capture; under the same conditions with changed task-force terms or modifier eligibility (carried_models), the row's model part (every field but maximum organisation and roster ordinal) is carried and maximum organisation uses the shared reader arithmetic over certified ship operands and current shared terms, critical counts and pointees, and terrain-class and leader-subunit contributions; a ship whose certificate moved is gathered again and, when complete own-object and heap payload observations and modifier arrays equal its held read's, carried under the new certificate (revalidated_ships) with fresh critical counts and pointees when its critical header is nonempty, or with only its arrays changed keeps its model part; otherwise its fixed fields, pool entries, heap fallback sprite, experience and medal-store modifiers are served from its held read while its handle, task force and certificates hold; task-force doctrine contributions require the doctrine writer and country naval tree certificates to hold, its modifier accumulator requires unchanged mine exposure (+0x650), and location weather terms require the region writer and country tree certificates to hold with unchanged applied night factor (+0x268); location, fleet, headquarters and uncertified sources are read each hour; every hour also reads the carryable ships of its slice afresh (one in rotation_hours by ship id; every one in verify mode), compares every own field, payload, modifier array and model suffix observed on both sides independently and maximum organisation under equal shared terms and modifier guard, publishes the fresh row and disables reuse at the first mismatch (sampled_*); an incomplete family forgets every range's held ships, tasks, variants, arrays and quiet token, including truncation and terminal failure"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// A differing sentinel leaves the family uninstalled; ShipModels then
    /// reads every ship afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if CARRY
            && MUTATIONS.iter().all(|site| {
                engine::read::<[u8; 16]>(memory, base + site.rva) == Some(site.sentinel)
            })
        {
            let callbacks: [usize; SITES] = [
                recalculate_equipment_stats as *const () as usize,
                refit as *const () as usize,
                rebuild_experience_modifiers as *const () as usize,
                add_carrier_air_base as *const () as usize,
                destroy_air_base as *const () as usize,
                rebuild_medal_modifiers as *const () as usize,
                rebuild_doctrine_tree as *const () as usize,
                rebuild_region_weather_modifiers as *const () as usize,
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
            WATCHES.registry.set_enabled(false);
            crate::air_base_carry_changes::forwarder(
                crate::air_base_carry_changes::Forwarder::ShipModel,
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
        WATCHES
            .registry
            .set_enabled(enabled && !self.hooks.is_empty());
        // The air base registrations observe base destruction only through
        // this family's destructor patch.
        crate::air_base_carry_changes::forwarder(
            crate::air_base_carry_changes::Forwarder::ShipModel,
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

/// Notify `site`'s receiver, then run the original with RCX..R9 and return
/// its RAX. None of the writers takes a stack argument.
unsafe fn forward(site: usize, arguments: [usize; 4]) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ShipModelChanges);
    let _notification = WATCHES.notify(arguments[RECEIVER_ARGUMENT[site]], site);
    // Air base destruction retires the base's registration vectors
    // ([`crate::air_base_carry_changes`]).
    let _carry = (site == DESTROY_AIR_BASE)
        .then(|| crate::air_base_carry_changes::base_destroyed(arguments[0]));
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[site].load(Ordering::Acquire)) };
    let [first, second, third, fourth] = arguments;
    timing.original(|| unsafe { original(first, second, third, fourth) })
}

macro_rules! detours {
    ($($name:ident = $site:expr;)*) => {$(
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            unsafe { forward($site, [first, second, third, fourth]) }
        }
    )*};
}

// [`MUTATIONS`] order: recalculate equipment stats(ship, flag), refit
//(ship, ...), ship experience modifier rebuild(ship), add carrier air base(manager,
// ship), air base destruction(base), medal modifier rebuild(store), task-force doctrine rebuild(task force) and
// Weather region modifier rebuild (weather region).
detours! {
    recalculate_equipment_stats = 0;
    refit = 1;
    rebuild_experience_modifiers = 2;
    add_carrier_air_base = 3;
    destroy_air_base = 4;
    rebuild_medal_modifiers = 5;
    rebuild_doctrine_tree = 6;
    rebuild_region_weather_modifiers = 7;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer moves only its receiver's certificate, so a ship's
    /// certificate moves with its own, its medal store's or its carrier
    /// base's writer; a running writer certifies nothing, an invalidating
    /// hour revokes everything and a mismatch poisons the domain.
    #[test]
    fn a_ship_certificate_moves_with_each_of_its_receivers() {
        let watches = Watches::enabled(16);
        let (ship, store, base, other) = (0x1000, 0x2000, 0x3000, 0x4000);
        let held = watches.certificate([ship, store, base]).unwrap();
        let unrelated = watches.certificate([other, 0, 0]).unwrap();
        assert_eq!(held[0].0, unrelated[0].0, "one epoch");
        assert_eq!(watches.certificate([ship, store, base]), Some(held));
        for (receiver, site) in [(ship, 0), (store, 5), (base, 4)] {
            let before = watches.certificate([ship, store, base]).unwrap();
            drop(watches.notify(receiver, site));
            assert_ne!(watches.certificate([ship, store, base]), Some(before));
        }
        assert_eq!(watches.certificate([other, 0, 0]), Some(unrelated));
        let running = watches.notify(ship, 1);
        assert!(watches.certificate([ship, 0, 0]).is_none());
        drop(running);
        let held = watches.certificate([ship, 0, 0]).unwrap();
        watches.boundary(true);
        assert_ne!(watches.certificate([ship, 0, 0]), Some(held));
        watches.mismatch();
        assert!(watches.poisoned());
        assert!(watches.certificate([ship, 0, 0]).is_none());
    }

    #[test]
    fn ship_certificate_slots_validate_each_receiver_and_keep_absence() {
        let watches = Watches::enabled(16);
        let receivers = [0x1000, 0x2000, 0x3000];
        let (certificate, slots) = watches.certificate_at(receivers, NO_SLOTS).unwrap();
        assert_eq!(watches.certificate(receivers), Some(certificate));
        assert!(slots.into_iter().all(|slot| slot != usize::MAX));
        assert_eq!(
            watches.certificate_at(receivers, slots),
            Some((certificate, slots))
        );
        assert_eq!(
            watches.certificate_at(receivers, [slots[2], slots[0], slots[1]]),
            Some((certificate, slots))
        );
        let absent = watches.certificate_at([receivers[0], 0, 0], slots).unwrap();
        assert_eq!(absent.0, [certificate[0], (0, 0), (0, 0)]);
        assert_eq!(absent.1, [slots[0], usize::MAX, usize::MAX]);
        assert_eq!(watches.registry.registered(), 3);
        for (index, site) in [0, 5, 4].into_iter().enumerate() {
            let writer = watches.notify(receivers[index], site);
            assert!(watches.certificate_at(receivers, slots).is_none());
            assert!(watches.quiet().is_none());
            drop(writer);
            let next = watches.certificate_at(receivers, slots).unwrap();
            assert_eq!(next.1, slots);
            assert_ne!(next.0[index], certificate[index]);
            assert_eq!(watches.certificate(receivers), Some(next.0));
        }
        let held = watches.certificate_at(receivers, slots).unwrap();
        watches.boundary(true);
        assert_ne!(watches.certificate_at(receivers, slots).unwrap().0, held.0);
        watches.mismatch();
        assert!(watches.certificate_at(receivers, slots).is_none());
        assert!(watches.quiet().is_none());
    }

    #[test]
    fn changes_without_slot_hints_keep_the_existing_certificate_contract() {
        struct Legacy;
        impl Changes for Legacy {
            fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
                (receiver != 0).then_some((7, receiver as u64))
            }
            fn mismatch(&self) {}
            fn tree_sequence(&self, _receiver: usize) -> Option<(u64, u64)> {
                None
            }
            fn certificate(&self, _receivers: [usize; 3]) -> Option<Certificate> {
                Some([(9, 1); 3])
            }
        }
        let changes = Legacy;
        assert_eq!(changes.sequence_at(8, 2), Some(((7, 8), usize::MAX)));
        assert!(changes.sequence_at(0, 2).is_none());
        assert_eq!(
            changes.certificate_at([8, 0, 0], [2, 3, 4]),
            Some(([(9, 1); 3], NO_SLOTS))
        );
        assert!(changes.quiet().is_none());
    }

    /// The detour order is the site table's, and each site's receiver
    /// argument is the ship's but for the manager-first carrier base call.
    #[test]
    fn receivers_follow_the_site_table() {
        let names = MUTATIONS.map(|site| site.name);
        assert_eq!(
            names,
            [
                "recalculate_equipment_stats",
                "refit",
                "rebuild_experience_modifiers",
                "add_carrier_air_base",
                "destroy_air_base",
                "rebuild_medal_modifiers",
                "rebuild_doctrine_tree",
                "rebuild_region_weather_modifiers",
            ]
        );
        assert_eq!(RECEIVER_ARGUMENT, [0, 0, 0, 1, 0, 0, 0, 0]);
        assert_eq!(MUTATIONS[DESTROY_AIR_BASE].rva, 0xc3_9000);
    }
}
