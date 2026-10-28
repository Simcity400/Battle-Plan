//! Allocation-free notifications from the native writers of every ship byte
//! ShipModels serves from a held read ([`engine_mutations::ship_model_mutations`]): a
//! ship's fixed fields, pool entries, heap fallback sprite and experience
//! modifiers, its medal store's modifiers and its carrier air base pointer,
//! and of its task force's doctrine tree and its weather region's
//! modifiers, which ShipModels carries with the country's naval tree
//! certificate, and of a fleet's leader binding, which selects the
//! commander whose tree terms ShipModels carries with its task force.
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
use engine_mutations::{ship_model_mutations::MUTATIONS, ship_model_mutations::RECEIVER_ARGUMENT};
use engine_native::Memory;
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
/// Carried and read ships; ships read without a certificate to hold; carried
/// task-force terms; carried rows; carried models; revalidated ships;
/// variants carried under their design certificate and variants gathered.
static READS: [AtomicU64; 9] = [const { AtomicU64::new(0) }; 9];
/// Receivers the latest complete capture registered, for off-thread sizing.
static RECEIVERS: AtomicUsize = AtomicUsize::new(0);
/// Design writers forwarded now (the design family's paired notifications).
static VARIANT_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static VARIANT_AVAILABILITY: AtomicU64 = AtomicU64::new(0);
static VARIANT_VALIDATE: AtomicUsize = AtomicUsize::new(0);
static VARIANT_CALLS: AtomicU64 = AtomicU64::new(0);

/// One observation domain: the watched receivers and the writers' calls,
/// and the equipment variants the design family's writers notify.
pub struct Watches {
    registry: WatchRegistry,
    variants: WatchRegistry,
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
            variants: WatchRegistry::new(),
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

    /// The existing refit patch also certifies the carrier-capacity predicate.
    /// Other ship writers do not move this independent inline-field receiver.
    pub fn capacity_refit(&self, ship: usize) -> Notification<'_> {
        self.registry
            .notify(ship.wrapping_add(engine_native::cship::MODULE_PREDICATE))
    }

    /// An enabled domain with a table for `receivers` installed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(receivers: usize) -> Self {
        let watches = Self::new();
        for registry in [&watches.registry, &watches.variants] {
            registry.offer(receivers);
            registry.boundary(false);
            registry.set_enabled(true);
        }
        watches
    }

    /// The hourly boundary; `invalidate` starts an epoch.
    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
        self.variants.boundary(invalidate);
    }

    /// Tests: a design writer is about to change `variant`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn notify_variant(&self, variant: usize) -> Notification<'_> {
        self.variants.notify(variant)
    }

    /// Tests: the design domain lost observation.
    #[cfg(any(test, feature = "test-support"))]
    pub fn disable_variants(&self) {
        self.variants.set_enabled(false);
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
    /// `(epoch, sequence)` of a country's or commander's modifier tree, which the naval tree family
    /// certifies.
    fn tree_sequence(&self, country: usize) -> Option<(u64, u64)>;

    /// `(epoch, sequence)` of an equipment variant, which the design family's
    /// writers move; `None` while that domain is unobserved.
    fn variant_sequence(&self, _variant: usize) -> Option<(u64, u64)> {
        None
    }
    /// `(epoch, revision)` of the variant domain with no design writer running.
    fn variant_quiet(&self) -> Option<(u64, u64)> {
        None
    }
    /// Whether captures under this domain share one held store across the
    /// frame pool's buffers, as the process-wide hooks' domain does.
    fn shared_store(&self) -> bool {
        false
    }

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
    /// Tests observe countries in the same domain.
    fn tree_sequence(&self, country: usize) -> Option<(u64, u64)> {
        self.registry.sequence(country)
    }
    fn variant_sequence(&self, variant: usize) -> Option<(u64, u64)> {
        self.variants.sequence(variant)
    }
    fn variant_quiet(&self) -> Option<(u64, u64)> {
        self.variants.quiet()
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
    fn tree_sequence(&self, country: usize) -> Option<(u64, u64)> {
        crate::naval_tree_changes::sequence(country)
    }
    fn variant_sequence(&self, variant: usize) -> Option<(u64, u64)> {
        variant_observed(|| WATCHES.variants.sequence(variant))
    }
    fn variant_quiet(&self) -> Option<(u64, u64)> {
        variant_observed(|| WATCHES.variants.quiet())
    }
    fn shared_store(&self) -> bool {
        true
    }
}

/// `read` while the design family's capability is current and none of its
/// forwarded writers runs, before and after.
fn variant_observed(read: impl FnOnce() -> Option<(u64, u64)>) -> Option<(u64, u64)> {
    let stamp = VARIANT_AVAILABILITY.load(Ordering::SeqCst);
    if !variant_capability_is_current(stamp) || VARIANT_ACTIVE.load(Ordering::SeqCst) != 0 {
        return None;
    }
    let result = read();
    (VARIANT_ACTIVE.load(Ordering::SeqCst) == 0 && variant_capability_is_current(stamp))
        .then_some(result)
        .flatten()
}

fn variant_capability_is_current(stamp: u64) -> bool {
    let validator = VARIANT_VALIDATE.load(Ordering::SeqCst);
    if stamp == 0 || validator == 0 || VARIANT_AVAILABILITY.load(Ordering::SeqCst) != stamp {
        return false;
    }
    // SAFETY: only `variant_availability` stores this, from a `fn(u64) -> bool`.
    let validate: fn(u64) -> bool = unsafe { std::mem::transmute(validator) };
    validate(stamp) && VARIANT_AVAILABILITY.load(Ordering::SeqCst) == stamp
}

/// The design family's forwarding capability, from its receiver; never waits.
pub fn variant_availability(stamp: u64, validate: fn(u64) -> bool) {
    VARIANT_VALIDATE.store(validate as *const () as usize, Ordering::SeqCst);
    VARIANT_AVAILABILITY.store(stamp, Ordering::SeqCst);
    WATCHES.variants.set_enabled(validate(stamp));
}

/// A forwarded design writer is about to change `variant`.
pub fn variant_before(variant: usize) {
    VARIANT_CALLS.fetch_add(1, Ordering::Relaxed);
    VARIANT_ACTIVE.fetch_add(1, Ordering::SeqCst);
    drop(WATCHES.variants.notify(variant));
}

/// Its return; an unwound writer leaves the domain unobserved.
pub fn variant_after(variant: usize, unwound: bool) {
    if unwound {
        WATCHES.variants.poison();
    }
    drop(WATCHES.variants.notify(variant));
    VARIANT_ACTIVE.fetch_sub(1, Ordering::SeqCst);
}

/// A forwarded writer whose variants cannot be named (bulk upgrades and
/// source, provider, country or content invalidation) is about to run.
pub fn variants_before() {
    VARIANT_CALLS.fetch_add(1, Ordering::Relaxed);
    VARIANT_ACTIVE.fetch_add(1, Ordering::SeqCst);
    drop(WATCHES.variants.notify_every());
}

pub fn variants_after(unwound: bool) {
    if unwound {
        WATCHES.variants.poison();
    }
    drop(WATCHES.variants.notify_every());
    VARIANT_ACTIVE.fetch_sub(1, Ordering::SeqCst);
}

/// Whether ShipModels holds ships' reads to carry them.
pub const CARRY: bool = cfg!(feature = "ship_model_changes");

/// Notifications running now.
pub fn active() -> usize {
    WATCHES
        .registry
        .active()
        .saturating_add(VARIANT_ACTIVE.load(Ordering::SeqCst))
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
    WATCHES.variants.boundary(invalidate);
}

/// Off the game thread: size the watches for the receivers the latest
/// capture registered. Nothing is offered before a capture registered any:
/// a later larger table waits for an invalidating hour, while the first
/// installs at once.
pub fn presize() {
    let receivers = RECEIVERS.load(Ordering::Relaxed);
    if receivers != 0 {
        WATCHES.registry.offer(receivers);
        WATCHES.variants.offer(receivers);
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
    WATCHES.variants.collect();
}

/// One capture range's ships.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub carried: u64,
    pub read: u64,
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
    /// Distinct held variants served under their design certificate, and
    /// held variants gathered again because a design writer touched them or
    /// the design domain was unobserved.
    pub variants_carried: u64,
    pub variants_read: u64,
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
        counts.uncertified,
        counts.task_terms_carried,
        counts.rows_carried,
        counts.models_carried,
        counts.revalidated,
        counts.variants_carried,
        counts.variants_read,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
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
        "poisoned": WATCHES.registry.poisoned(),
        "carried_ships": reads[0],
        "read_ships": reads[1],
        "uncertified_ships": reads[2],
        "carried_task_terms": reads[3],
        "carried_rows": reads[4],
        "carried_models": reads[5],
        "revalidated_ships": reads[6],
        "carried_variants": reads[7],
        "read_variants": reads[8],
        "timing": timing_status(),
        "events": events,
        "design_writers": VARIANT_CALLS.load(Ordering::Relaxed),
        "watches": WATCHES.registry.status(),
        "variant_watches": WATCHES.variants.status(),
        "variants_poisoned": WATCHES.variants.poisoned(),
        "scope": "cumulative since attach; a ship's finished row is carried when its handle, task force and receiver certificates hold, its task force's terms this hour equal those of its held row, its held variants are certified by the design writers' domain (or, after a design writer touched one or while that domain is unobserved, gathered again with unchanged bytes), its optional suffixes are complete, its present organisation modifier keys keep their current eligibility and clamp metadata, and it has no carrier base; carried_variants and read_variants count distinct held variants served under their design certificate and held variants gathered again; held reads live in one store per census range that the family's captures share across the frame pool, a complete range holding at once; compact held records follow census order with a sorted-index fallback, domain quiet tokens rechecked before each ship spare receiver lookups, and validated slot indices spare watch hashing otherwise; task-force terms are compared once per task force and organisation modifier eligibility and clamp metadata once per distinct guard per capture; under the same conditions with changed task-force terms or modifier eligibility (carried_models), the row's model part (every field but maximum organisation and roster ordinal) is carried and maximum organisation uses the shared reader arithmetic over certified ship operands and current shared terms, critical counts and pointees, and terrain-class and leader-subunit contributions; a ship whose certificate moved is gathered again and, when complete own-object and heap payload observations and modifier arrays equal its held read's, carried under the new certificate (revalidated_ships) with fresh critical counts and pointees when its critical header is nonempty, or with only organisation inputs changed in the same epoch keeps its model part after complete model-byte and census-class comparisons and certified or compared variants; otherwise its fixed fields, pool entries, heap fallback sprite, experience and medal-store modifiers are served from its held read while its handle, task force and certificates hold; each task force is gathered once per hour (vtable, handle, roster owner, location and fleet, mine exposure and the attached-headquarters header, which the game rebuilds every hour); its doctrine contribution requires the doctrine writer and country naval tree certificates to hold, its modifier accumulator unchanged mine exposure, its location weather terms the region writer and country tree certificates with unchanged applied night factor, its location's province definition and terrain the held location in the same epoch, and its fleet's leader and leader tree terms the held fleet with unchanged leader-binding and leader tree certificates; the applied night factor, pride ship, attached headquarters' commanders and uncertified sources are read each hour; a range whose every row was carried whole at the roster position it was last published with, one per row its store held, keeps that store's held-rows generation, and a complete family whose ranges all kept theirs witnesses the previous capture's rows (quiet_republish); an incomplete family forgets every range's held ships, tasks, variants, arrays, rows generation and quiet token, including truncation and terminal failure",
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
                crate::hook::installation::sentinel(memory, base + site.rva, site.sentinel)
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
                bind_fleet_leader as *const () as usize,
                reset_reserve_fleet as *const () as usize,
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
            crate::unit_history_changes::forwarder(false);
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
        crate::unit_history_changes::forwarder(enabled && !self.hooks.is_empty());
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
        // after the last capture. The variant table stays: the design
        // family's forwarder notifies it.
        unsafe { WATCHES.registry.release() };
    }
}

/// Notify `site`'s receiver, then run the original with RCX..R9 and return
/// its RAX. None of the writers takes a stack argument.
unsafe fn forward(site: usize, arguments: [usize; 4]) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ShipModelChanges);
    let _notification = WATCHES.notify(arguments[RECEIVER_ARGUMENT[site]], site);
    let _history = (site == 5).then(|| crate::unit_history_changes::notify(arguments[0]));
    let _capacity = (site == 1).then(|| WATCHES.capacity_refit(arguments[0]));
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
// ship), air base destruction(base), medal modifier rebuild(store), task-force doctrine rebuild(task force),
// Weather region modifier rebuild (weather region), fleet leader binding (fleet, leader)
// and reserve fleet conversion (fleet).
detours! {
    recalculate_equipment_stats = 0;
    refit = 1;
    rebuild_experience_modifiers = 2;
    add_carrier_air_base = 3;
    destroy_air_base = 4;
    rebuild_medal_modifiers = 5;
    rebuild_doctrine_tree = 6;
    rebuild_region_weather_modifiers = 7;
    bind_fleet_leader = 8;
    reset_reserve_fleet = 9;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_refit_moves_the_independent_carrier_capacity_certificate() {
        let watches = Watches::enabled(16);
        let ship = 0x1000;
        let receiver = ship + engine_native::cship::MODULE_PREDICATE;
        let capacity = watches.sequence(receiver).unwrap();
        for site in [0, 2, 3, 5] {
            drop(watches.notify(ship, site));
            assert_eq!(watches.sequence(receiver), Some(capacity));
        }
        let own = watches.sequence(ship).unwrap();
        let notification = watches.capacity_refit(ship);
        assert_eq!(watches.sequence(receiver), None);
        drop(notification);
        assert_ne!(watches.sequence(receiver), Some(capacity));
        assert_eq!(watches.sequence(ship), Some(own));
    }

    /// A writer moves only its receiver's certificate, so a ship's
    /// certificate moves with its own, its medal store's or its carrier
    /// base's writer; a running writer certifies nothing and an invalidating
    /// hour revokes everything.
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
    }

    #[test]
    fn changes_without_slot_hints_keep_the_existing_certificate_contract() {
        struct Legacy;
        impl Changes for Legacy {
            fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
                (receiver != 0).then_some((7, receiver as u64))
            }
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

}
