//! DivisionStatus.attack_stats and movement_stats: each division's stat-block attack
//! and movement operands (`engine_native::csub_unit_definition::capture_attack_stats`, one
//! read of the same block), carried while no
//! hooked update active stats touched the army since the
//! read they came from ([`crate::army_stat_changes`]), and the off-by-default
//! `attack_witness` diagnostic's native operands (DivisionStatus.attack).
//!
//! The [`ArmyStatStore`] holds the latest complete capture's operands in
//! census order, with each division's army and stat block addresses, under
//! the marks' epoch of the hour that read or carried them. The domain's
//! registry marks a registered army or name member a writer touches
//! ([`crate::army_stat_changes`]); each hour takes the marks once, before any
//! read ([`Captured::certify`]). A division is carried only when the take
//! returned the held epoch, its army and stat block are those of the held
//! entry, its army was not marked and its id is outside the hour's sampled
//! slice; every other one is read, its army registered first, and a read of
//! a carryable division is compared with the held operands (a difference
//! poisons the family). A held entry is found at the seed's own census index
//! when the census did not move, so a quiet hour costs no lookup per
//! division. A capture that does not complete holds nothing, and a failed
//! job forgets the store. Without a store (the family not configured) every
//! division is read.
//!
//! The same block holds the status pass's maxima (maximum organisation and
//! strength and the exfiltration byte), which the same writer alone rewrites:
//! a division the store carries gives that pass its held maxima instead of a
//! read of its block, and the attack pass holds the maxima the status pass
//! used beside its operands.
//!
//! The store also holds each division's name member span and cached-name lookup cache
//! text under the same marks, on the member's address
//! (`engine_mutations::army_stat_mutations`); the naming pass carries and rebuilds them
//! ([`super::division_presentation`]), and they are held with the attack
//! pass's operands.
use crate::army_stat_changes::{self, Changes, Counts};
use engine_native::{
    Memory, carmy::DivisionSeed, csub_unit_definition as stats,
    csub_unit_definition::DivisionMaxima, walk::Outcome,
};
use replay_data::{
    rows::AttackStats, rows::AttackTerrain, rows::DivisionAttackInputs, rows::DivisionAttackStats,
    rows::DivisionMovementStats, rows::PacketHour,
};

mod preparation;
pub use crate::division_experience::Order as ScalarOrder;
pub use preparation::{AttackPart, PreparationPart, PreparedDivision};

/// A stat block's scalars.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scalars {
    panel: [Option<i64>; 23],
    river: [i64; 2],
    fort: [i64; 2],
    snow: i64,
    night: i64,
    movement: Option<[i64; 3]>,
    fuel_rate: i64,
    air_defense: i64,
}

impl Scalars {
    fn of(stats: &stats::AttackStats) -> Self {
        Self {
            panel: stats.panel,
            river: stats.river,
            fort: stats.fort,
            snow: stats.snow,
            night: stats.night,
            movement: stats.movement,
            fuel_rate: stats.fuel_rate,
            air_defense: stats.air_defense,
        }
    }
}

/// One division's operands this hour: a span of [`Captured::terrain`] and
/// the scalars, or a read refusal.
#[derive(Clone, Copy, Debug, Default)]
struct Row {
    division: u64,
    start: usize,
    len: usize,
    scalars: Option<Scalars>,
    experience: Option<i64>,
    officer_experience: Option<i64>,
    career_counter: Option<i32>,
    pending_template: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
struct Held {
    division: u64,
    army: usize,
    stat_block: usize,
    start: usize,
    len: usize,
    scalars: Scalars,
    /// The status pass's maxima of the same hour; `None` when it did not
    /// establish them.
    maxima: Option<DivisionMaxima>,
}

/// One division's name member as the latest complete capture read it, and
/// its cache text's span of the store's name bytes.
#[derive(Clone, Copy)]
pub struct HeldName {
    division: u64,
    pub member: usize,
    pub span: [u8; engine_native::cname_group_member::INPUT_SPAN],
    text: (usize, usize),
}

/// Finds a seed's held entry in entries kept in census order: at the seed's
/// own index, shifted by the census insertions and removals found so far
/// this hour, and otherwise through a division index built on the first
/// miss.
#[derive(Default)]
struct Locator {
    order: Vec<(u64, u32)>,
    shift: isize,
}

impl Locator {
    fn clear(&mut self) {
        self.order.clear();
        self.shift = 0;
    }

    fn locate(&mut self, divisions: &[u64], index: usize, division: u64) -> Option<usize> {
        if let Some(guess) = index.checked_add_signed(self.shift)
            && divisions.get(guess) == Some(&division)
        {
            return Some(guess);
        }
        if self.order.is_empty() {
            engine_allocation::advisory(|| {
                engine_allocation::reserve(&mut self.order, divisions.len())
            })
            .ok()?;
            self.order.extend(
                divisions
                    .iter()
                    .enumerate()
                    .map(|(at, &division)| (division, at as u32)),
            );
            self.order.sort_unstable();
        }
        let at = self.order[self.order.partition_point(|entry| entry.0 < division)..]
            .first()
            .filter(|entry| entry.0 == division)?
            .1 as usize;
        self.shift = at as isize - index as isize;
        Some(at)
    }
}

/// The latest complete capture's operands, lent to the Presentation job each
/// hour. Entries are kept in census order. Every held army and name member
/// was registered in the marks' `epoch` of the capture that holds it.
pub struct ArmyStatStore {
    experience: crate::division_experience::Store,
    progression: crate::division_experience::Store<u64>,
    officer: crate::division_experience::Store<i64>,
    career: crate::division_experience::Store<i32>,
    supply: crate::division_experience::Store<std::sync::Arc<crate::supply_inputs::Sample>>,
    daily: bool,
    pub history: crate::division_history::Store,
    pub sources: crate::panel_sources::Store,
    /// The marks' epoch the held entries were certified in; `None` holds
    /// nothing.
    epoch: Option<u64>,
    held: Vec<Held>,
    /// The held entries' divisions, for [`Locator`].
    divisions: Vec<u64>,
    locator: Locator,
    terrain: Vec<i64>,
    /// Movement words parallel to `terrain`.
    movement: Vec<i64>,
    next: Vec<Held>,
    next_terrain: Vec<i64>,
    next_movement: Vec<i64>,
    names: Vec<HeldName>,
    name_divisions: Vec<u64>,
    name_locator: Locator,
    name_bytes: Vec<u8>,
    next_names: Vec<HeldName>,
    next_name_bytes: Vec<u8>,
    #[cfg(any(test, feature = "test-support"))]
    pub name_lookups: usize,
}

impl Default for ArmyStatStore {
    fn default() -> Self {
        Self::new()
    }
}

// Footer carry uses daily/full revocation and census occurrence identity, not
// a native writer sequence. The common occurrence store still needs a token.
const DAILY_SUPPLY_SAMPLE: Option<(u64, u64)> = Some((0, 0));

impl ArmyStatStore {
    /// Owner-approved daily display samples; hourly certificates still protect
    /// every other operand, including the XP lent to exercise eligibility.
    pub fn daily_progress(&mut self, refresh: bool) {
        self.daily = true;
        self.experience.daily(refresh);
        self.officer.daily(refresh);
        if refresh {
            self.supply.forget();
        }
    }
    pub fn daily_supply(
        &self,
        base: usize,
        index: usize,
        seed: &DivisionSeed,
        order: &mut ScalarOrder,
    ) -> Option<std::sync::Arc<crate::supply_inputs::Sample>> {
        self.daily
            .then(|| {
                self.supply
                    .prefetch_value(base, index, seed, DAILY_SUPPLY_SAMPLE, order, || {
                        Err("division supply has no daily sample")
                    })
            })
            .flatten()
            .and_then(Result::ok)
    }
    pub fn forget_supply(&mut self) {
        self.supply.forget();
    }
    pub fn holds_daily_progress(&self) -> bool {
        self.daily
    }
    pub fn remember_supply(
        &mut self,
        base: usize,
        seeds: &[DivisionSeed],
        samples: &[Option<std::sync::Arc<crate::supply_inputs::Sample>>],
        complete: bool,
    ) {
        if !self.daily {
            return;
        }
        let stored = complete
            && self.supply.begin(seeds.len()).is_ok()
            && seeds.iter().zip(samples).all(|(seed, sample)| {
                self.supply
                    .observe_with(base, seed, DAILY_SUPPLY_SAMPLE, || {
                        sample
                            .clone()
                            .ok_or("division supply daily sample unavailable")
                    })
                    .is_ok()
            })
            && samples.len() == seeds.len();
        self.supply.finish(stored);
    }
    pub fn new() -> Self {
        Self {
            experience: crate::division_experience::Store::default(),
            progression: Default::default(),
            officer: Default::default(),
            career: Default::default(),
            supply: Default::default(),
            daily: false,
            history: Default::default(),
            sources: Default::default(),
            epoch: None,
            held: Vec::new(),
            divisions: Vec::new(),
            locator: Locator::default(),
            terrain: Vec::new(),
            movement: Vec::new(),
            next: Vec::new(),
            next_terrain: Vec::new(),
            next_movement: Vec::new(),
            names: Vec::new(),
            name_divisions: Vec::new(),
            name_locator: Locator::default(),
            name_bytes: Vec::new(),
            next_names: Vec::new(),
            next_name_bytes: Vec::new(),
            #[cfg(any(test, feature = "test-support"))]
            name_lookups: 0,
        }
    }

    /// A failed job: nothing it read certifies a carry, and its take
    /// consumed the marks.
    pub fn forget(&mut self) {
        self.experience.forget();
        self.progression.forget();
        self.officer.forget();
        self.career.forget();
        self.supply.forget();
        self.history.forget();
        self.sources.forget();
        self.epoch = None;
        self.divisions.clear();
        self.name_divisions.clear();
        self.locator.clear();
        self.name_locator.clear();
        self.held.clear();
        self.terrain.clear();
        self.movement.clear();
        self.next.clear();
        self.next_terrain.clear();
        self.next_movement.clear();
        self.names.clear();
        self.name_bytes.clear();
        self.next_names.clear();
        self.next_name_bytes.clear();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.experience.allocated_bytes()
            + self.supply.allocated_bytes()
            + self.progression.allocated_bytes()
            + self.officer.allocated_bytes()
            + self.career.allocated_bytes()
            + self.history.allocated_bytes()
            + self.sources.allocated_bytes()
            + (self.held.capacity() + self.next.capacity()) * std::mem::size_of::<Held>()
            + (self.divisions.capacity() + self.name_divisions.capacity()) * 8
            + (self.locator.order.capacity() + self.name_locator.order.capacity()) * 16
            + (self.terrain.capacity() + self.next_terrain.capacity()) * 8
            + (self.movement.capacity() + self.next_movement.capacity()) * 8
            + (self.names.capacity() + self.next_names.capacity()) * std::mem::size_of::<HeldName>()
            + self.name_bytes.capacity()
            + self.next_name_bytes.capacity()
    }

    /// The held name's index for the seed at census `index`, `division`.
    pub fn held_name(&mut self, index: usize, division: u64) -> Option<usize> {
        #[cfg(any(test, feature = "test-support"))]
        {
            self.name_lookups += 1;
        }
        self.name_locator
            .locate(&self.name_divisions, index, division)
    }

    /// Borrow the selected record without copying its owned member span.
    pub fn held_name_record(&self, index: usize) -> &HeldName {
        &self.names[index]
    }

    /// The cache text of a held name member.
    pub fn held_name_text(&self, held: &HeldName) -> &[u8] {
        &self.name_bytes[held.text.0..held.text.0 + held.text.1]
    }

    /// Starts the hour's naming pass: what it keeps is held with the hour's
    /// attack operands.
    pub fn begin_names(&mut self) {
        self.next_names.clear();
        self.next_name_bytes.clear();
    }

    /// Keeps a division's name member and cache text, carried this hour or
    /// read after its member was registered.
    pub fn keep_name(
        &mut self,
        division: u64,
        member: usize,
        span: &[u8; engine_native::cname_group_member::INPUT_SPAN],
        text: &[u8],
    ) {
        let start = self.next_name_bytes.len();
        self.next_name_bytes.extend_from_slice(text);
        self.next_names.push(HeldName {
            division,
            member,
            span: *span,
            text: (start, text.len()),
        });
    }

    /// The held operands of the seed at census `index`, `division`.
    fn locate(&mut self, index: usize, division: u64) -> Option<usize> {
        self.locator.locate(&self.divisions, index, division)
    }

    fn terrain(&self, held: &Held) -> &[i64] {
        &self.terrain[held.start..held.start + held.len]
    }

    fn movement(&self, held: &Held) -> &[i64] {
        &self.movement[held.start..held.start + held.len]
    }

    /// The hour's capture completed under the marks' `epoch`: hold what it
    /// read and carried, in census order.
    pub fn hold(&mut self, epoch: Option<u64>) {
        std::mem::swap(&mut self.held, &mut self.next);
        std::mem::swap(&mut self.terrain, &mut self.next_terrain);
        std::mem::swap(&mut self.movement, &mut self.next_movement);
        std::mem::swap(&mut self.names, &mut self.next_names);
        std::mem::swap(&mut self.name_bytes, &mut self.next_name_bytes);
        self.next.clear();
        self.next_terrain.clear();
        self.next_movement.clear();
        self.next_names.clear();
        self.next_name_bytes.clear();
        self.divisions.clear();
        self.name_divisions.clear();
        self.locator.clear();
        self.name_locator.clear();
        let indexed = engine_allocation::advisory(|| {
            engine_allocation::reserve(&mut self.divisions, self.held.len())?;
            engine_allocation::reserve(&mut self.name_divisions, self.names.len())
        });
        if indexed.is_err() {
            self.forget();
            return;
        }
        self.divisions
            .extend(self.held.iter().map(|held| held.division));
        self.name_divisions
            .extend(self.names.iter().map(|held| held.division));
        self.epoch = epoch;
    }
}

pub const CERTIFY_PHASE_NAMES: [&str; 3] = ["certify_setup", "certify_ordered", "certify_finish"];

/// The hour's operands, sorted by division, and the witnessed packets.
#[derive(Default)]
pub struct Captured {
    /// Recorded XP in census order, possibly held at the daily sample.
    pub experience: Vec<Option<i64>>,
    /// Only certificate-current XP may be consumed by the hourly native predicate.
    pub exercise_experience: Vec<Option<i64>>,
    certify_ticks: [u64; CERTIFY_PHASE_NAMES.len()],
    scalars_deferred: bool,
    certified_check: crate::sampled_verify::Check,
    history_finish: Option<Result<(), &'static str>>,
    pending_templates: Vec<Option<u64>>,
    officer: Vec<Option<i64>>,
    career: Vec<Option<i32>>,
    pub sources: Option<Result<std::sync::Arc<crate::panel_sources::SourceSnapshot>, &'static str>>,
    pub selected_sources: Vec<(u64, Option<[u64; 3]>)>,
    pub history:
        Option<Result<std::sync::Arc<crate::division_history::HistorySnapshot>, &'static str>>,
    rows: Vec<Row>,
    terrain: Vec<i64>,
    /// Movement words parallel to `terrain`.
    movement: Vec<i64>,
    scratch: Vec<i64>,
    movement_scratch: Vec<i64>,
    /// Per seed: its held operands' index when the held capture certifies
    /// them this hour ([`Self::certify`]).
    held_index: Vec<Option<u32>>,
    /// Per seed: carried, or its army registered before the read.
    registered: Vec<bool>,
    /// The armies and name members the hour's take visited, sorted.
    touched: Vec<usize>,
    /// The marks' epoch of the hour's take.
    epoch: Option<u64>,
    /// The held capture's certificates hold this hour: its epoch is the
    /// take's.
    pub carryable: bool,
    /// Each seed's carried maxima for the status pass.
    pub held_maxima: Vec<Option<DivisionMaxima>>,
    /// The carryable maxima of the sampled slice, to compare with its reads.
    sampled_maxima: Vec<(usize, DivisionMaxima)>,
    /// The status and naming passes' counts, published with the attack
    /// pass's.
    pub status_counts: Counts,
    witness: Vec<(u64, DivisionAttackInputs)>,
    /// The live terrains' database indices and own adjuster attacks, read
    /// once in a witnessing hour.
    terrains: Vec<(i32, i64)>,
}

/// The witness slice: the sampled slice of twelve hours later, so a
/// witnessed division was carried, not read, when it was carryable.
fn witnessed(hour: i32, id: u64) -> bool {
    army_stat_changes::WITNESS
        && crate::sampled_verify::Check::hourly(hour.wrapping_add(12), false).covers(id)
}

impl Captured {
    pub fn reset(&mut self) {
        self.rows.clear();
        self.terrain.clear();
        self.movement.clear();
        self.witness.clear();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.selected_sources.capacity() * std::mem::size_of::<(u64, Option<[u64; 3]>)>()
            + self.experience.capacity() * std::mem::size_of::<Option<i64>>()
            + self.exercise_experience.capacity() * std::mem::size_of::<Option<i64>>()
            + self.pending_templates.capacity() * std::mem::size_of::<Option<u64>>()
            + self.officer.capacity() * std::mem::size_of::<Option<i64>>()
            + self.career.capacity() * std::mem::size_of::<Option<i32>>()
            + self.rows.capacity() * std::mem::size_of::<Row>()
            + (self.terrain.capacity() + self.scratch.capacity()) * 8
            + (self.movement.capacity() + self.movement_scratch.capacity()) * 8
            + self.held_index.capacity() * 8
            + self.registered.capacity()
            + self.touched.capacity() * 8
            + self.held_maxima.capacity() * std::mem::size_of::<Option<DivisionMaxima>>()
            + self.sampled_maxima.capacity() * std::mem::size_of::<(usize, DivisionMaxima)>()
            + self.witness.capacity() * std::mem::size_of::<(u64, DivisionAttackInputs)>()
            + self.terrains.capacity() * std::mem::size_of::<(i32, i64)>()
    }

    /// The marks' epoch of the hour's take.
    #[cfg(any(test, feature = "test-support"))]
    pub fn epoch(&self) -> Option<u64> {
        self.epoch
    }

    /// Whether a writer touched `receiver` since the held capture.
    pub fn touched(&self, receiver: usize) -> bool {
        self.touched.binary_search(&receiver).is_ok()
    }

    /// Before the status pass: the hour's take, then for each division the
    /// held entry it carries (its army and stat block those of the held read,
    /// the army registered in the held capture's epoch and untouched since)
    /// and the maxima the store carries for it (`held_maxima`); every other
    /// division's army is registered before its block is read. A division of
    /// the sampled slice carries nothing; its held maxima are compared with
    /// the pass's read by [`Self::verify_maxima`].
    pub fn certify(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
        outcome: Outcome,
        store: Option<&mut ArmyStatStore>,
        changes: &dyn Changes,
    ) {
        self.certify_prepared(memory, base, seeds, outcome, store, changes, |_| None);
    }

    /// Settle range-local observations in the original census order. Taking
    /// marks, registration, cache admission and history edits still happen once.
    #[allow(clippy::too_many_arguments)]
    pub fn certify_prepared(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
        outcome: Outcome,
        store: Option<&mut ArmyStatStore>,
        changes: &dyn Changes,
        prepared: impl Fn(usize) -> Option<PreparedDivision>,
    ) {
        self.certify_staged(
            memory, base, seeds, outcome, store, changes, prepared, false,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn certify_staged(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
        outcome: Outcome,
        store: Option<&mut ArmyStatStore>,
        changes: &dyn Changes,
        prepared: impl Fn(usize) -> Option<PreparedDivision>,
        defer_owned: bool,
    ) {
        let begin = crate::clock::qpc();
        self.certify_ticks = [0; CERTIFY_PHASE_NAMES.len()];
        self.experience.clear();
        self.exercise_experience.clear();
        self.pending_templates.clear();
        self.officer.clear();
        self.career.clear();
        self.history = None;
        self.held_index.clear();
        self.registered.clear();
        self.touched.clear();
        self.held_maxima.clear();
        self.sampled_maxima.clear();
        self.status_counts = Counts::default();
        self.epoch = None;
        self.carryable = false;
        self.certified_check = crate::sampled_verify::current_burned_in(army_stat_changes::VERIFY);
        self.scalars_deferred = false;
        self.history_finish = None;
        let Some(store) = store else {
            return;
        };
        let touched = &mut self.touched;
        let mut kept = true;
        self.epoch = changes.take(&mut |receiver| {
            if engine_allocation::advisory(|| engine_allocation::reserve(touched, 1)).is_ok() {
                touched.push(receiver);
            } else {
                kept = false;
            }
        });
        self.touched.sort_unstable();
        self.carryable = kept && self.epoch.is_some() && store.epoch == self.epoch;
        // The take consumed the marks: only this hour's capture holds again.
        store.epoch = None;
        let seeds = &seeds[..outcome.len().min(seeds.len())];
        self.scalars_deferred = defer_owned
            && seeds
                .iter()
                .enumerate()
                .all(|(index, _)| prepared(index).is_some_and(|p| p.owned_scalars()));
        self.history_finish = None;
        let experience_ready = store
            .experience
            .begin(seeds.len())
            .map_err(army_stat_changes::experience_refused)
            .is_ok()
            && engine_allocation::reserve(&mut self.experience, seeds.len())
                .map_err(|_| {
                    army_stat_changes::experience_refused(
                        "division experience cache allocation failed",
                    )
                })
                .is_ok();
        // This optional loan only avoids the native exercise reader's own XP
        // read. Allocation failure must not withdraw the recorded XP sample.
        let exercise_ready = engine_allocation::advisory(|| {
            engine_allocation::reserve(&mut self.exercise_experience, seeds.len())
        })
        .is_ok();
        let progression_ready = !self.scalars_deferred
            && store
                .progression
                .begin(seeds.len())
                .map_err(army_stat_changes::progression_refused)
                .is_ok()
            && engine_allocation::reserve(&mut self.pending_templates, seeds.len())
                .map_err(|_| {
                    army_stat_changes::progression_refused(
                        "division experience cache allocation failed",
                    )
                })
                .is_ok();
        let officer_ready = !self.scalars_deferred
            && store.officer.begin(seeds.len()).is_ok()
            && engine_allocation::reserve(&mut self.officer, seeds.len()).is_ok();
        let career_ready = !self.scalars_deferred
            && store.career.begin(seeds.len()).is_ok()
            && engine_allocation::reserve(&mut self.career, seeds.len()).is_ok();
        let mut history_result = if defer_owned {
            store.history.begin_owned(seeds.len())
        } else {
            store.history.begin(seeds.len())
        };
        let check = crate::sampled_verify::current_burned_in(army_stat_changes::VERIFY);
        self.certify_ticks[0] = crate::clock::qpc().saturating_sub(begin).max(0) as u64;
        let ordered_begin = crate::clock::qpc();
        for (index, seed) in seeds.iter().enumerate() {
            use collector_observers::unit_history_changes as history_changes;
            let prepared = prepared(index);
            if officer_ready {
                let receiver = seed
                    .pointer
                    .checked_add(engine_native::unit_history::OFFICER);
                let certificate = match prepared.filter(|p| p.acquired[2]) {
                    Some(p) => p.officer_certificate,
                    None => receiver.and_then(history_changes::sequence),
                };
                self.officer.push(
                    store
                        .officer
                        .observe_with(base, seed, certificate, || {
                            prepared
                                .filter(|p| {
                                    certificate.is_some() && p.officer_certificate == certificate
                                })
                                .and_then(|p| p.officer)
                                .unwrap_or_else(|| {
                                    engine_native::unit_history::officer_experience(
                                        memory,
                                        base,
                                        seed.pointer,
                                    )
                                })
                        })
                        .map_err(|_| history_changes::refused())
                        .ok(),
                );
            }
            // Career and stored XP consult the same independent writer
            // domain within this quiescent pass. Observe it only once.
            let experience_certificate = (career_ready || experience_ready)
                .then(|| match prepared.filter(|p| p.acquired[0]) {
                    Some(p) => p.experience_certificate,
                    None => changes.experience_sequence(seed.pointer),
                })
                .flatten();
            if career_ready {
                let certificate = match prepared.filter(|p| p.acquired[3]) {
                    Some(p) => p.career_certificate,
                    None => {
                        experience_certificate.and_then(|_| history_changes::sequence(seed.pointer))
                    }
                };
                self.career.push(
                    store
                        .career
                        .observe_with(base, seed, certificate, || {
                            prepared
                                .filter(|p| {
                                    certificate.is_some() && p.career_certificate == certificate
                                })
                                .and_then(|p| p.career)
                                .unwrap_or_else(|| {
                                    engine_native::unit_history::career_counter(memory, seed.pointer)
                                })
                        })
                        .map_err(|_| history_changes::refused())
                        .ok(),
                );
            }
            if progression_ready {
                let certificate = match prepared.filter(|p| p.acquired[1]) {
                    Some(p) => p.progression_certificate,
                    None => changes.progression_sequence(seed.pointer),
                };
                self.pending_templates.push(
                    store
                        .progression
                        .observe_with(base, seed, certificate, || {
                            prepared
                                .filter(|p| {
                                    certificate.is_some()
                                        && p.progression_certificate == certificate
                                })
                                .and_then(|p| p.progression)
                                .unwrap_or_else(|| {
                                    engine_native::carmy::pending_template(memory, base, seed.pointer)
                                })
                        })
                        .map_err(army_stat_changes::progression_refused)
                        .ok(),
                );
            }
            if history_result.is_ok() {
                let _reads = collector_memory::read_accounting::table(
                    replay_data::rows::TableId::DivisionHistory,
                );
                history_result = store.history.observe_prepared(
                    memory,
                    base,
                    seed,
                    prepared.and_then(|p| p.history),
                );
            }
            if experience_ready {
                let certificate = experience_certificate;
                self.experience.push(
                    store
                        .experience
                        .observe_with(base, seed, certificate, || {
                            prepared
                                .filter(|p| {
                                    certificate.is_some() && p.experience_certificate == certificate
                                })
                                .and_then(|p| p.experience)
                                .unwrap_or_else(|| {
                                    engine_native::read::<i64>(
                                        memory,
                                        seed.pointer
                                            .checked_add(engine_native::carmy::EXPERIENCE)
                                            .ok_or("division experience address overflow")?,
                                    )
                                    .ok_or("division experience unreadable")
                                })
                        })
                        .map_err(army_stat_changes::experience_refused)
                        .ok(),
                );
                if exercise_ready {
                    self.exercise_experience
                        .push(store.experience.current(seed, certificate));
                }
            }
            let held = self
                .carryable
                .then(|| store.locate(index, seed.division))
                .flatten()
                .filter(|&at| {
                    let held = &store.held[at];
                    held.army == seed.pointer
                        && Some(held.stat_block) == seed.stat_block
                        && self.touched.binary_search(&seed.pointer).is_err()
                });
            let maxima = held.and_then(|at| store.held[at].maxima);
            self.held_index.push(held.map(|at| at as u32));
            self.registered
                .push(held.is_some() || changes.register(seed.pointer));
            let sampled = check.covers(seed.division >> 32);
            if let (Some(held), true) = (maxima, sampled) {
                self.sampled_maxima.push((index, held));
            }
            self.held_maxima.push(maxima.filter(|_| !sampled));
        }
        self.certify_ticks[1] = crate::clock::qpc().saturating_sub(ordered_begin).max(0) as u64;
        let begin = crate::clock::qpc();
        store
            .experience
            .finish(matches!(outcome, Outcome::Rows { .. }) && experience_ready);
        if !self.scalars_deferred {
            store
                .progression
                .finish(matches!(outcome, Outcome::Rows { .. }) && progression_ready);
            store
                .officer
                .finish(matches!(outcome, Outcome::Rows { .. }) && officer_ready);
            store
                .career
                .finish(matches!(outcome, Outcome::Rows { .. }) && career_ready);
        }
        if defer_owned {
            self.history_finish = Some(if matches!(outcome, Outcome::Rows { .. }) {
                history_result
            } else {
                Err("complete division history census unavailable")
            });
        } else {
            self.history = Some(if matches!(outcome, Outcome::Rows { .. }) {
                history_result.and_then(|()| store.history.finish())
            } else {
                Err("complete division history census unavailable")
            });
            if !matches!(self.history, Some(Ok(_))) {
                store.history.forget();
            }
        }
        self.status_counts.maxima_reused = self.held_maxima.iter().flatten().count() as u64;
        self.certify_ticks[2] = crate::clock::qpc().saturating_sub(begin).max(0) as u64;
    }
    pub fn has_owned_certification(&self) -> bool {
        self.scalars_deferred || self.history_finish.is_some()
    }
    /// Frozen receiver observations only. No native read, registration or mark
    /// drain may occur here after the simulation resumes.
    pub fn settle_certification(
        &mut self,
        base: usize,
        seeds: &[DivisionSeed],
        outcome: Outcome,
        store: &mut ArmyStatStore,
        prepared: impl Fn(usize) -> Option<PreparedDivision>,
    ) {
        use collector_observers::unit_history_changes as history_changes;
        if std::mem::take(&mut self.scalars_deferred) {
            let seeds = &seeds[..outcome.len().min(seeds.len())];
            let progression_ready = store
                .progression
                .begin(seeds.len())
                .map_err(army_stat_changes::progression_refused)
                .is_ok()
                && engine_allocation::reserve(&mut self.pending_templates, seeds.len())
                    .map_err(|_| {
                        army_stat_changes::progression_refused(
                            "division experience cache allocation failed",
                        )
                    })
                    .is_ok();
            let officer_ready = store.officer.begin(seeds.len()).is_ok()
                && engine_allocation::reserve(&mut self.officer, seeds.len()).is_ok();
            let career_ready = store.career.begin(seeds.len()).is_ok()
                && engine_allocation::reserve(&mut self.career, seeds.len()).is_ok();
            for (index, seed) in seeds.iter().enumerate() {
                let p = prepared(index).expect("owned scalar range coverage");
                if officer_ready {
                    self.officer.push(
                        store
                            .officer
                            .observe_with(base, seed, p.officer_certificate, || {
                                p.officer.expect("owned officer operand")
                            })
                            .map_err(|_| history_changes::refused())
                            .ok(),
                    );
                }
                if career_ready {
                    self.career.push(
                        store
                            .career
                            .observe_with(base, seed, p.career_certificate, || {
                                p.career.expect("owned career operand")
                            })
                            .map_err(|_| history_changes::refused())
                            .ok(),
                    );
                }
                if progression_ready {
                    self.pending_templates.push(
                        store
                            .progression
                            .observe_with(base, seed, p.progression_certificate, || {
                                p.progression.expect("owned progression operand")
                            })
                            .map_err(army_stat_changes::progression_refused)
                            .ok(),
                    );
                }
            }
            let complete = matches!(outcome, Outcome::Rows { .. });
            store.progression.finish(complete && progression_ready);
            store.officer.finish(complete && officer_ready);
            store.career.finish(complete && career_ready);
        }
        if let Some(result) = self.history_finish.take() {
            self.history = Some(result.and_then(|()| store.history.finish()));
            if !matches!(self.history, Some(Ok(_))) {
                store.history.forget();
            }
        }
    }

    pub fn certify_ticks(&self) -> [u64; CERTIFY_PHASE_NAMES.len()] {
        self.certify_ticks
    }

    /// After the status pass: its read of each sampled carryable division's
    /// maxima against the held ones; a difference poisons the family.
    pub fn verify_maxima(
        &mut self,
        read: impl Fn(usize) -> Option<DivisionMaxima>,
        changes: &dyn Changes,
    ) {
        for &(index, held) in &self.sampled_maxima {
            let Some(fresh) = read(index) else {
                continue;
            };
            self.status_counts.verified += 1;
            if fresh != held {
                self.status_counts.mismatches += 1;
                changes.mismatch();
            }
        }
    }

    /// Reads or carries every division of a census `outcome` prefix whose
    /// stat block the seed names. `hour` decides the sampled and witnessed
    /// slices. `maxima` gives the status pass's maxima of a seed, held with
    /// its operands; the carried entries are [`Self::certify`]'s.
    #[allow(clippy::too_many_arguments)] // The census, its seeds, the held store and the status maxima.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        hour: i32,
        seeds: &[DivisionSeed],
        outcome: Outcome,
        store: Option<&mut ArmyStatStore>,
        maxima: impl Fn(usize) -> Option<DivisionMaxima>,
        changes: &dyn Changes,
    ) {
        self.capture_prepared(
            memory,
            base,
            hour,
            seeds,
            outcome,
            store,
            maxima,
            changes,
            |_| None,
        );
    }

    /// Consume range-local fresh reads only where the original ordered walk
    /// chooses acquisition. Carry, verification, row order and store settlement
    /// are still performed by that walk.
    #[allow(clippy::too_many_arguments)]
    pub fn capture_prepared<'a>(
        &mut self,
        memory: &impl Memory,
        base: usize,
        hour: i32,
        seeds: &[DivisionSeed],
        outcome: Outcome,
        store: Option<&mut ArmyStatStore>,
        maxima: impl Fn(usize) -> Option<DivisionMaxima>,
        changes: &dyn Changes,
        prepared: impl Fn(usize) -> Option<Result<&'a stats::AttackStats, &'static str>>,
    ) {
        self.prepare_witness(memory, base, hour, seeds, outcome);
        let check = crate::sampled_verify::current_burned_in(army_stat_changes::VERIFY);
        self.capture_values(
            memory, base, hour, seeds, outcome, store, maxima, changes, prepared, check,
        );
    }

    /// The witness database is native; acquire it before the owned handoff.
    pub fn prepare_witness(
        &mut self,
        memory: &impl Memory,
        base: usize,
        hour: i32,
        seeds: &[DivisionSeed],
        outcome: Outcome,
    ) {
        let seeds = &seeds[..outcome.len().min(seeds.len())];
        if army_stat_changes::WITNESS
            && seeds
                .iter()
                .any(|seed| witnessed(hour, seed.division >> 32))
        {
            self.terrains.clear();
            if engine_native::cterrain_database::terrain_attacks(memory, base, &mut self.terrains)
                .is_err()
            {
                self.terrains.clear();
            }
        }
    }

    /// Every non-carried block has an owned range result. No reader or registry
    /// registration is reachable after resume, including witness-only hours.
    #[allow(clippy::too_many_arguments)]
    pub fn capture_owned<'a>(
        &mut self,
        base: usize,
        hour: i32,
        seeds: &[DivisionSeed],
        outcome: Outcome,
        store: Option<&mut ArmyStatStore>,
        maxima: impl Fn(usize) -> Option<DivisionMaxima>,
        changes: &dyn Changes,
        prepared: impl Fn(usize) -> Option<Result<&'a stats::AttackStats, &'static str>>,
    ) {
        // The callback may already have advanced the global verification hour.
        let check = self.certified_check;
        self.capture_values(
            &OwnedAttack,
            base,
            hour,
            seeds,
            outcome,
            store,
            maxima,
            changes,
            prepared,
            check,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn capture_values<'a>(
        &mut self,
        memory: &impl Memory,
        base: usize,
        hour: i32,
        seeds: &[DivisionSeed],
        outcome: Outcome,
        mut store: Option<&mut ArmyStatStore>,
        maxima: impl Fn(usize) -> Option<DivisionMaxima>,
        changes: &dyn Changes,
        prepared: impl Fn(usize) -> Option<Result<&'a stats::AttackStats, &'static str>>,
        check: crate::sampled_verify::Check,
    ) {
        self.reset();
        // The status and naming passes' counts publish with this pass's.
        let mut counts = self.status_counts;
        let seeds = &seeds[..outcome.len().min(seeds.len())];
        if let Some(store) = store.as_deref_mut() {
            store.next.clear();
            store.next_terrain.clear();
            store.next_movement.clear();
        }
        for (index, seed) in seeds.iter().enumerate() {
            let experience = self.experience.get(index).copied().flatten();
            let pending_template = self.pending_templates.get(index).copied().flatten();
            let officer_experience = self.officer.get(index).copied().flatten();
            let career_counter = self.career.get(index).copied().flatten();
            let id = seed.division >> 32;
            let refused = Row {
                division: seed.division,
                experience,
                officer_experience,
                career_counter,
                pending_template,
                ..Default::default()
            };
            let Some(stat_block) = seed.stat_block.filter(|_| seed.division != 0) else {
                counts.refused += 1;
                self.rows.push(refused);
                continue;
            };
            // Certified before the status pass, before any read of the block.
            let held = store.as_deref().and_then(|store| {
                let at = self.held_index.get(index).copied().flatten()?;
                Some(store.held[at as usize]).filter(|held| held.stat_block == stat_block)
            });
            let registered = self.registered.get(index).copied().unwrap_or(false);
            if let (Some(held), Some(store)) = (held, store.as_deref_mut())
                && !check.covers(id)
                && !witnessed(hour, id)
            {
                counts.reused += 1;
                let start = self.terrain.len();
                self.terrain.extend_from_slice(store.terrain(&held));
                self.movement.extend_from_slice(store.movement(&held));
                self.rows.push(Row {
                    division: seed.division,
                    start,
                    len: held.len,
                    scalars: Some(held.scalars),
                    experience,
                    officer_experience,
                    career_counter,
                    pending_template,
                });
                let next = store.next_terrain.len();
                let span = held.start..held.start + held.len;
                store
                    .next_terrain
                    .extend_from_slice(&store.terrain[span.clone()]);
                store.next_movement.extend_from_slice(&store.movement[span]);
                store.next.push(Held {
                    start: next,
                    maxima: maxima(index),
                    ..held
                });
                continue;
            }
            counts.read += 1;
            let staged = prepared(index);
            let acquired = staged.is_none().then(|| {
                stats::capture_attack_stats_holding(
                    memory,
                    base,
                    stat_block,
                    &mut self.scratch,
                    &mut self.movement_scratch,
                    maxima(index),
                )
            });
            let fresh = match staged {
                Some(value) => value,
                None => acquired
                    .as_ref()
                    .expect("fresh acquisition")
                    .as_ref()
                    .map_err(|&reason| reason),
            };
            if let (Some(held), Some(store)) = (held, store.as_deref())
                && let Ok(fresh) = &fresh
                && check.covers(id)
            {
                counts.verified += 1;
                if store.terrain(&held) != fresh.terrain.as_slice()
                    || store.movement(&held) != fresh.movement_terrain.as_slice()
                    || held.scalars != Scalars::of(fresh)
                {
                    counts.mismatches += 1;
                    changes.mismatch();
                }
            }
            match fresh {
                Ok(fresh) => {
                    if fresh.panel.iter().any(Option::is_none) {
                        counts.refused += 1;
                    }
                    let start = self.terrain.len();
                    self.terrain.extend_from_slice(&fresh.terrain);
                    self.movement.extend_from_slice(&fresh.movement_terrain);
                    let scalars = Scalars::of(fresh);
                    self.rows.push(Row {
                        division: seed.division,
                        start,
                        len: fresh.terrain.len(),
                        scalars: Some(scalars),
                        experience,
                        officer_experience,
                        career_counter,
                        pending_template,
                    });
                    if witnessed(hour, id) && !self.terrains.is_empty() {
                        counts.witnessed += 1;
                        self.witness
                            .push((seed.division, witness(fresh, &self.terrains)));
                    }
                    if let Some(store) = store.as_deref_mut()
                        && registered
                    {
                        let next = store.next_terrain.len();
                        store.next_terrain.extend_from_slice(&fresh.terrain);
                        store
                            .next_movement
                            .extend_from_slice(&fresh.movement_terrain);
                        store.next.push(Held {
                            division: seed.division,
                            army: seed.pointer,
                            stat_block,
                            start: next,
                            len: fresh.terrain.len(),
                            scalars,
                            maxima: maxima(index),
                        });
                    }
                    // Staged entries retain their own buffers across hours.
                    if staged.is_none()
                        && let Some(Ok(fresh)) = acquired
                    {
                        self.scratch = fresh.terrain;
                        self.movement_scratch = fresh.movement_terrain;
                    }
                }
                Err(_) => {
                    counts.refused += 1;
                    self.rows.push(refused);
                }
            }
        }
        self.rows.sort_unstable_by_key(|row| row.division);
        self.witness.sort_unstable_by_key(|(division, _)| *division);
        let complete = matches!(outcome, Outcome::Rows { .. });
        if let Some(store) = store {
            if complete {
                store.hold(self.epoch);
            } else {
                store.forget();
            }
        }
        army_stat_changes::record(&counts, complete.then_some(seeds.len()));
    }

    pub fn pending_template(&self, division: u64) -> Option<u64> {
        self.row(division)?.pending_template
    }

    pub fn officer(&self, division: u64) -> Option<i64> {
        self.row(division)?.officer_experience
    }
    pub fn career(&self, division: u64) -> Option<i32> {
        self.row(division)?.career_counter
    }

    pub fn panel_source(&self, division: u64) -> Option<[u64; 3]> {
        self.selected_sources
            .binary_search_by_key(&division, |row| row.0)
            .ok()
            .and_then(|at| self.selected_sources[at].1)
    }
    pub fn panel(&self, division: u64) -> [Option<i64>; 22] {
        self.row(division)
            .and_then(|row| row.scalars)
            .map_or([None; 22], |scalars| {
                // Anti-air is already recorded in movement_stats; keep one stored operand.
                std::array::from_fn(|index| scalars.panel[index + usize::from(index >= 11)])
            })
    }

    pub fn experience(&self, division: u64) -> Option<i64> {
        self.row(division)?.experience
    }

    fn row(&self, division: u64) -> Option<&Row> {
        self.rows
            .binary_search_by_key(&division, |row| row.division)
            .ok()
            .map(|at| &self.rows[at])
    }

    /// Resolve the attack-row key once for this row's scalar and terrain fields.
    pub fn view(&self, division: u64) -> RowView<'_> {
        RowView {
            captured: self,
            division,
            row: self.row(division),
        }
    }
    pub fn stats(&self, division: u64) -> Option<DivisionAttackStats> {
        self.view(division).stats()
    }
    pub fn movement(&self, division: u64) -> Option<DivisionMovementStats> {
        self.view(division).movement()
    }
    pub fn same(
        &self,
        division: u64,
        previous: &replay_data::rows::Recorded<Option<DivisionAttackStats>>,
    ) -> bool {
        self.view(division).same(previous)
    }
    pub fn same_movement(
        &self,
        division: u64,
        previous: &replay_data::rows::Recorded<Option<DivisionMovementStats>>,
    ) -> bool {
        self.view(division).same_movement(previous)
    }
    pub fn has_witness(&self, division: u64) -> bool {
        self.witness
            .binary_search_by_key(&division, |(division, _)| *division)
            .is_ok()
    }

    /// The witnessed native operands of a division, with the hour's
    /// acclimatization factor.
    pub fn witness(&self, division: u64, factor: Option<i64>) -> Option<DivisionAttackInputs> {
        let at = self
            .witness
            .binary_search_by_key(&division, |(division, _)| *division)
            .ok()?;
        Some(DivisionAttackInputs {
            acclimatization: factor,
            ..self.witness[at].1.clone()
        })
    }
}

/// Borrowed projection of one current owned attack row; no new read or carry.
pub struct RowView<'a> {
    captured: &'a Captured,
    division: u64,
    row: Option<&'a Row>,
}
impl RowView<'_> {
    pub fn experience(&self) -> Option<i64> {
        self.row?.experience
    }
    pub fn pending_template(&self) -> Option<u64> {
        self.row?.pending_template
    }
    pub fn officer(&self) -> Option<i64> {
        self.row?.officer_experience
    }
    pub fn career(&self) -> Option<i32> {
        self.row?.career_counter
    }
    pub fn panel_source(&self) -> Option<[u64; 3]> {
        self.captured.panel_source(self.division)
    }
    pub fn witness(&self, factor: Option<i64>) -> Option<DivisionAttackInputs> {
        self.captured.witness(self.division, factor)
    }
    /// The division's recorded operands; `None` is a refusal (or a division
    /// the capture did not reach).
    pub fn stats(&self) -> Option<DivisionAttackStats> {
        let row = self.row?;
        let scalars = row.scalars?;
        Some(DivisionAttackStats {
            terrain: engine_allocation::owned_vec(engine_allocation::copy_vec(
                self.captured.terrain.get(row.start..row.start + row.len)?,
            )),
            river: scalars.river,
            fort: scalars.fort,
            snow: scalars.snow,
            night: scalars.night,
        })
    }

    /// The division's recorded movement operands, held with its attack
    /// operands; `None` is a refusal.
    pub fn movement(&self) -> Option<DivisionMovementStats> {
        let row = self.row?;
        let scalars = row.scalars?;
        Some(DivisionMovementStats {
            terrain: engine_allocation::owned_vec(engine_allocation::copy_vec(
                self.captured.movement.get(row.start..row.start + row.len)?,
            )),
            scalars: scalars.movement,
            fuel_rate: scalars.fuel_rate,
            air_defense: scalars.air_defense,
        })
    }

    /// Whether `previous` holds exactly what [`Self::movement`] would publish.
    pub fn same_movement(
        &self,
        previous: &replay_data::rows::Recorded<Option<DivisionMovementStats>>,
    ) -> bool {
        let replay_data::rows::Recorded::Value(previous) = previous else {
            return false;
        };
        let current = self.row.and_then(|row| {
            Some((
                row.scalars?,
                self.captured.movement.get(row.start..row.start + row.len)?,
            ))
        });
        match (current, previous) {
            (None, None) => true,
            (Some((scalars, terrain)), Some(previous)) => {
                terrain == previous.terrain.as_slice()
                    && scalars.movement == previous.scalars
                    && scalars.fuel_rate == previous.fuel_rate
                    && scalars.air_defense == previous.air_defense
            }
            _ => false,
        }
    }

    /// Whether `previous` holds exactly what [`Self::stats`] would publish.
    pub fn same(
        &self,
        previous: &replay_data::rows::Recorded<Option<DivisionAttackStats>>,
    ) -> bool {
        let replay_data::rows::Recorded::Value(previous) = previous else {
            return false;
        };
        let current = self.row.and_then(|row| {
            Some((
                row.scalars?,
                self.captured.terrain.get(row.start..row.start + row.len)?,
            ))
        });
        match (current, previous) {
            (None, None) => true,
            (Some((scalars, terrain)), Some(previous)) => {
                terrain == previous.terrain.as_slice()
                    && scalars.river == previous.river
                    && scalars.fort == previous.fort
                    && scalars.snow == previous.snow
                    && scalars.night == previous.night
            }
            _ => false,
        }
    }
}

/// Terrain attack lookup's attack for every live terrain: the stat block's entry at
/// the database index minus one plus the terrain's own adjuster.
fn witness(fresh: &stats::AttackStats, terrains: &[(i32, i64)]) -> DivisionAttackInputs {
    DivisionAttackInputs {
        sample_hour: PacketHour::Host,
        template: None,
        stats: AttackStats {
            river: fresh.river.map(Some),
            fort: fresh.fort.map(Some),
            snow: Some(fresh.snow),
            night: Some(fresh.night),
        },
        terrain: Some(
            terrains
                .iter()
                .map(|&(index, attack)| AttackTerrain {
                    terrain: index,
                    attack: usize::try_from(index)
                        .ok()
                        .and_then(|index| index.checked_sub(1))
                        .and_then(|at| fresh.terrain.get(at))
                        .map(|entry| entry.wrapping_add(attack)),
                })
                .collect(),
        ),
        acclimatization: None,
    }
}

/// A missing range is a programming error, never an invented native refusal.
struct OwnedAttack;
impl Memory for OwnedAttack {
    fn copy(&self, _: usize, _: &mut [u8]) -> bool {
        panic!("owned attack settlement attempted a native read")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(division: u64, terrain: &[i64]) -> (Held, Vec<i64>) {
        (
            Held {
                division,
                army: 0x1000,
                stat_block: 0x2000,
                start: 0,
                len: terrain.len(),
                scalars: Scalars::default(),
                maxima: None,
            },
            terrain.to_vec(),
        )
    }

    /// A held entry is found at the seed's own index, then at the index the
    /// census's insertions and removals shift it to, and not at all once
    /// the division is gone.
    #[test]
    fn the_locator_follows_census_shifts_and_misses_gone_divisions() {
        let held: Vec<u64> = (1..=6).map(|id| id << 32).collect();
        let mut locator = Locator::default();
        assert_eq!(locator.locate(&held, 2, 3 << 32), Some(2));
        assert!(locator.order.is_empty());
        // A division inserted before the fourth: every later seed is one on.
        assert_eq!(locator.locate(&held, 4, 4 << 32), Some(3));
        assert_eq!(locator.shift, -1);
        assert_eq!(locator.locate(&held, 5, 5 << 32), Some(4));
        assert_eq!(locator.locate(&held, 6, 9 << 32), None);
        assert_eq!(locator.locate(&held, 0, 1 << 32), Some(0));
        locator.clear();
        assert_eq!((locator.order.len(), locator.shift), (0, 0));
    }

    #[test]
    fn the_store_holds_only_a_completed_capture() {
        let mut store = ArmyStatStore::new();
        let (entry, terrain) = held(7 << 32, &[1, 2]);
        store.next.push(entry);
        store.next_terrain = terrain;
        store.hold(Some(3));
        assert_eq!(store.epoch, Some(3));
        let at = store.locate(0, 7 << 32).unwrap();
        assert_eq!(store.held[at].len, 2);
        assert_eq!(store.terrain(&store.held[at]), [1, 2]);
        assert!(store.next.is_empty() && store.next_terrain.is_empty());
        store.forget();
        assert_eq!(store.epoch, None);
        assert!(store.locate(0, 7 << 32).is_none());
    }

    #[test]
    fn published_operands_compare_with_the_previous_row() {
        let mut captured = Captured {
            terrain: vec![-10_000, 0],
            movement: vec![-5_000, 7],
            ..Captured::default()
        };
        captured.rows = vec![
            Row {
                division: 3,
                start: 0,
                len: 2,
                experience: None,
                pending_template: None,
                scalars: Some(Scalars {
                    river: [1, 2],
                    fort: [3, 4],
                    snow: 5,
                    night: 6,
                    movement: Some([-1, -2, -3]),
                    fuel_rate: 40,
                    air_defense: 50,
                    ..Default::default()
                }),
                ..Default::default()
            },
            Row {
                division: 9,
                ..Default::default()
            },
        ];
        let stats = captured.stats(3).unwrap();
        assert_eq!(stats.terrain, [-10_000, 0]);
        assert_eq!(
            (stats.river, stats.fort, stats.snow, stats.night),
            ([1, 2], [3, 4], 5, 6)
        );
        assert!(captured.same(3, &replay_data::rows::Recorded::Value(Some(stats.clone()))));
        let mut changed = stats;
        changed.terrain[1] = 1;
        assert!(!captured.same(3, &replay_data::rows::Recorded::Value(Some(changed))));
        // A refused read publishes None, as does a division never reached.
        assert_eq!(captured.stats(9), None);
        assert!(captured.same(9, &replay_data::rows::Recorded::Value(None)));
        assert!(captured.same(4, &replay_data::rows::Recorded::Value(None)));
        assert!(!captured.same(3, &replay_data::rows::Recorded::NotRecorded));
        // The movement operands are published from the same held read.
        let movement = captured.movement(3).unwrap();
        assert_eq!(
            movement,
            DivisionMovementStats {
                terrain: vec![-5_000, 7],
                scalars: Some([-1, -2, -3]),
                fuel_rate: 40,
                air_defense: 50,
            }
        );
        assert!(captured.same_movement(
            3,
            &replay_data::rows::Recorded::Value(Some(movement.clone()))
        ));
        for changed in [
            DivisionMovementStats {
                fuel_rate: 41,
                ..movement.clone()
            },
            DivisionMovementStats {
                air_defense: 0,
                ..movement.clone()
            },
            DivisionMovementStats {
                terrain: vec![-5_000, 8],
                ..movement.clone()
            },
        ] {
            assert!(!captured.same_movement(3, &replay_data::rows::Recorded::Value(Some(changed))));
        }
        assert_eq!(captured.movement(9), None);
        assert!(captured.same_movement(9, &replay_data::rows::Recorded::Value(None)));
    }

    #[test]
    fn a_witness_adds_each_live_terrains_own_adjuster() {
        let fresh = stats::AttackStats {
            terrain: vec![-10_000, -20_000],
            river: [1, 2],
            fort: [3, 4],
            snow: 5,
            night: 6,
            ..Default::default()
        };
        let packet = witness(&fresh, &[(1, 5_000), (2, 0), (3, 7)]);
        assert_eq!(
            packet.terrain.unwrap(),
            [
                AttackTerrain {
                    terrain: 1,
                    attack: Some(-5_000)
                },
                AttackTerrain {
                    terrain: 2,
                    attack: Some(-20_000)
                },
                // A terrain past the stat block's entries is unread.
                AttackTerrain {
                    terrain: 3,
                    attack: None
                },
            ]
        );
        assert_eq!(packet.stats.night, Some(6));
    }
}
