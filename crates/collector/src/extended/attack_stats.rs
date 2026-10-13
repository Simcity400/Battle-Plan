//! DivisionStatus.attack_stats: each division's stat-block attack operands
//! (`engine::csub_unit_definition::capture_attack_stats`), carried while no
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
//! (`engine::army_stat_mutations`); the naming pass carries and rebuilds them
//! ([`super::division_presentation`]), and they are held with the attack
//! pass's operands.
use crate::army_stat_changes::{self, Changes, Counts};
use engine::{
    Memory,
    carmy::DivisionSeed,
    csub_unit_definition::{self as stats, DivisionMaxima},
    walk::Outcome,
};
use replay::rows::{
    AttackStats, AttackTerrain, DivisionAttackInputs, DivisionAttackStats, PacketHour,
};

/// A stat block's scalars.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Scalars {
    river: [i64; 2],
    fort: [i64; 2],
    snow: i64,
    night: i64,
}

impl Scalars {
    fn of(stats: &stats::AttackStats) -> Self {
        Self {
            river: stats.river,
            fort: stats.fort,
            snow: stats.snow,
            night: stats.night,
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
pub(super) struct HeldName {
    division: u64,
    pub(super) member: usize,
    pub(super) span: [u8; engine::cname_group_member::INPUT_SPAN],
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
            engine::allocation::advisory(|| {
                engine::allocation::reserve(&mut self.order, divisions.len())
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
pub(crate) struct ArmyStatStore {
    /// The marks' epoch the held entries were certified in; `None` holds
    /// nothing.
    epoch: Option<u64>,
    held: Vec<Held>,
    /// The held entries' divisions, for [`Locator`].
    divisions: Vec<u64>,
    locator: Locator,
    terrain: Vec<i64>,
    next: Vec<Held>,
    next_terrain: Vec<i64>,
    names: Vec<HeldName>,
    name_divisions: Vec<u64>,
    name_locator: Locator,
    name_bytes: Vec<u8>,
    next_names: Vec<HeldName>,
    next_name_bytes: Vec<u8>,
}

impl Default for ArmyStatStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ArmyStatStore {
    pub(crate) fn new() -> Self {
        Self {
            epoch: None,
            held: Vec::new(),
            divisions: Vec::new(),
            locator: Locator::default(),
            terrain: Vec::new(),
            next: Vec::new(),
            next_terrain: Vec::new(),
            names: Vec::new(),
            name_divisions: Vec::new(),
            name_locator: Locator::default(),
            name_bytes: Vec::new(),
            next_names: Vec::new(),
            next_name_bytes: Vec::new(),
        }
    }

    /// A failed job: nothing it read certifies a carry, and its take
    /// consumed the marks.
    pub(crate) fn forget(&mut self) {
        self.epoch = None;
        self.divisions.clear();
        self.name_divisions.clear();
        self.locator.clear();
        self.name_locator.clear();
        self.held.clear();
        self.terrain.clear();
        self.next.clear();
        self.next_terrain.clear();
        self.names.clear();
        self.name_bytes.clear();
        self.next_names.clear();
        self.next_name_bytes.clear();
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        (self.held.capacity() + self.next.capacity()) * std::mem::size_of::<Held>()
            + (self.divisions.capacity() + self.name_divisions.capacity()) * 8
            + (self.locator.order.capacity() + self.name_locator.order.capacity()) * 16
            + (self.terrain.capacity() + self.next_terrain.capacity()) * 8
            + (self.names.capacity() + self.next_names.capacity()) * std::mem::size_of::<HeldName>()
            + self.name_bytes.capacity()
            + self.next_name_bytes.capacity()
    }

    /// The held name member of the seed at census `index`, `division`.
    pub(super) fn held_name(&mut self, index: usize, division: u64) -> Option<HeldName> {
        let at = self
            .name_locator
            .locate(&self.name_divisions, index, division)?;
        Some(self.names[at])
    }

    /// The cache text of a held name member.
    pub(super) fn held_name_text(&self, held: &HeldName) -> &[u8] {
        &self.name_bytes[held.text.0..held.text.0 + held.text.1]
    }

    /// Starts the hour's naming pass: what it keeps is held with the hour's
    /// attack operands.
    pub(super) fn begin_names(&mut self) {
        self.next_names.clear();
        self.next_name_bytes.clear();
    }

    /// Keeps a division's name member and cache text, carried this hour or
    /// read after its member was registered.
    pub(super) fn keep_name(
        &mut self,
        division: u64,
        member: usize,
        span: &[u8; engine::cname_group_member::INPUT_SPAN],
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

    /// The hour's capture completed under the marks' `epoch`: hold what it
    /// read and carried, in census order.
    pub(super) fn hold(&mut self, epoch: Option<u64>) {
        std::mem::swap(&mut self.held, &mut self.next);
        std::mem::swap(&mut self.terrain, &mut self.next_terrain);
        std::mem::swap(&mut self.names, &mut self.next_names);
        std::mem::swap(&mut self.name_bytes, &mut self.next_name_bytes);
        self.next.clear();
        self.next_terrain.clear();
        self.next_names.clear();
        self.next_name_bytes.clear();
        self.divisions.clear();
        self.name_divisions.clear();
        self.locator.clear();
        self.name_locator.clear();
        let indexed = engine::allocation::advisory(|| {
            engine::allocation::reserve(&mut self.divisions, self.held.len())?;
            engine::allocation::reserve(&mut self.name_divisions, self.names.len())
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

/// The hour's operands, sorted by division, and the witnessed packets.
#[derive(Default)]
pub(super) struct Captured {
    rows: Vec<Row>,
    terrain: Vec<i64>,
    scratch: Vec<i64>,
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
    pub(super) carryable: bool,
    /// Each seed's carried maxima for the status pass.
    pub(super) held_maxima: Vec<Option<DivisionMaxima>>,
    /// The carryable maxima of the sampled slice, to compare with its reads.
    sampled_maxima: Vec<(usize, DivisionMaxima)>,
    /// The status and naming passes' counts, published with the attack
    /// pass's.
    pub(super) status_counts: Counts,
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
    pub(super) fn reset(&mut self) {
        self.rows.clear();
        self.terrain.clear();
        self.witness.clear();
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * std::mem::size_of::<Row>()
            + (self.terrain.capacity() + self.scratch.capacity()) * 8
            + self.held_index.capacity() * 8
            + self.registered.capacity()
            + self.touched.capacity() * 8
            + self.held_maxima.capacity() * std::mem::size_of::<Option<DivisionMaxima>>()
            + self.sampled_maxima.capacity() * std::mem::size_of::<(usize, DivisionMaxima)>()
            + self.witness.capacity() * std::mem::size_of::<(u64, DivisionAttackInputs)>()
            + self.terrains.capacity() * std::mem::size_of::<(i32, i64)>()
    }

    /// The marks' epoch of the hour's take.
    #[cfg(test)]
    pub(super) fn epoch(&self) -> Option<u64> {
        self.epoch
    }

    /// Whether a writer touched `receiver` since the held capture.
    pub(super) fn touched(&self, receiver: usize) -> bool {
        self.touched.binary_search(&receiver).is_ok()
    }

    /// Before the status pass: the hour's take, then for each division the
    /// held entry it carries (its army and stat block those of the held read,
    /// the army registered in the held capture's epoch and untouched since)
    /// and the maxima the store carries for it (`held_maxima`); every other
    /// division's army is registered before its block is read. A division of
    /// the sampled slice carries nothing; its held maxima are compared with
    /// the pass's read by [`Self::verify_maxima`].
    pub(super) fn certify(
        &mut self,
        seeds: &[DivisionSeed],
        outcome: Outcome,
        store: Option<&mut ArmyStatStore>,
        changes: &dyn Changes,
    ) {
        self.held_index.clear();
        self.registered.clear();
        self.touched.clear();
        self.held_maxima.clear();
        self.sampled_maxima.clear();
        self.status_counts = Counts::default();
        self.epoch = None;
        self.carryable = false;
        let Some(store) = store else {
            return;
        };
        let touched = &mut self.touched;
        let mut kept = true;
        self.epoch = changes.take(&mut |receiver| {
            if engine::allocation::advisory(|| engine::allocation::reserve(touched, 1)).is_ok() {
                touched.push(receiver);
            } else {
                kept = false;
            }
        });
        self.touched.sort_unstable();
        self.carryable = kept && self.epoch.is_some() && store.epoch == self.epoch;
        // The take consumed the marks: only this hour's capture holds again.
        store.epoch = None;
        let check = crate::sampled_verify::current_burned_in(army_stat_changes::VERIFY);
        for (index, seed) in seeds[..outcome.len().min(seeds.len())].iter().enumerate() {
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
        self.status_counts.maxima_reused = self.held_maxima.iter().flatten().count() as u64;
    }

    /// After the status pass: its read of each sampled carryable division's
    /// maxima against the held ones; a difference poisons the family.
    pub(super) fn verify_maxima(
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
    pub(super) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        hour: i32,
        seeds: &[DivisionSeed],
        outcome: Outcome,
        mut store: Option<&mut ArmyStatStore>,
        maxima: impl Fn(usize) -> Option<DivisionMaxima>,
        changes: &dyn Changes,
    ) {
        self.reset();
        let check = crate::sampled_verify::current_burned_in(army_stat_changes::VERIFY);
        // The status and naming passes' counts publish with this pass's.
        let mut counts = self.status_counts;
        let seeds = &seeds[..outcome.len().min(seeds.len())];
        if let Some(store) = store.as_deref_mut() {
            store.next.clear();
            store.next_terrain.clear();
        }
        if army_stat_changes::WITNESS
            && seeds
                .iter()
                .any(|seed| witnessed(hour, seed.division >> 32))
        {
            self.terrains.clear();
            if engine::cterrain_database::terrain_attacks(memory, base, &mut self.terrains).is_err()
            {
                self.terrains.clear();
            }
        }
        for (index, seed) in seeds.iter().enumerate() {
            let id = seed.division >> 32;
            let refused = Row {
                division: seed.division,
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
                self.rows.push(Row {
                    division: seed.division,
                    start,
                    len: held.len,
                    scalars: Some(held.scalars),
                });
                let next = store.next_terrain.len();
                let span = held.start..held.start + held.len;
                store.next_terrain.extend_from_slice(&store.terrain[span]);
                store.next.push(Held {
                    start: next,
                    maxima: maxima(index),
                    ..held
                });
                continue;
            }
            counts.read += 1;
            let fresh = stats::capture_attack_stats(memory, base, stat_block, &mut self.scratch);
            if let (Some(held), Some(store)) = (held, store.as_deref())
                && let Ok(fresh) = &fresh
                && check.covers(id)
            {
                counts.verified += 1;
                if store.terrain(&held) != fresh.terrain.as_slice()
                    || held.scalars != Scalars::of(fresh)
                {
                    counts.mismatches += 1;
                    changes.mismatch();
                }
            }
            match fresh {
                Ok(fresh) => {
                    let start = self.terrain.len();
                    self.terrain.extend_from_slice(&fresh.terrain);
                    let scalars = Scalars::of(&fresh);
                    self.rows.push(Row {
                        division: seed.division,
                        start,
                        len: fresh.terrain.len(),
                        scalars: Some(scalars),
                    });
                    if witnessed(hour, id) && !self.terrains.is_empty() {
                        counts.witnessed += 1;
                        self.witness
                            .push((seed.division, witness(&fresh, &self.terrains)));
                    }
                    if let Some(store) = store.as_deref_mut()
                        && registered
                    {
                        let next = store.next_terrain.len();
                        store.next_terrain.extend_from_slice(&fresh.terrain);
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
                    // Return the entries' storage for the next read.
                    self.scratch = fresh.terrain;
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

    fn row(&self, division: u64) -> Option<&Row> {
        self.rows
            .binary_search_by_key(&division, |row| row.division)
            .ok()
            .map(|at| &self.rows[at])
    }

    /// The division's recorded operands; `None` is a refusal (or a division
    /// the capture did not reach).
    pub(super) fn stats(&self, division: u64) -> Option<DivisionAttackStats> {
        let row = self.row(division)?;
        let scalars = row.scalars?;
        Some(DivisionAttackStats {
            terrain: engine::allocation::owned_vec(engine::allocation::copy_vec(
                self.terrain.get(row.start..row.start + row.len)?,
            )),
            river: scalars.river,
            fort: scalars.fort,
            snow: scalars.snow,
            night: scalars.night,
        })
    }

    /// Whether `previous` holds exactly what [`Self::stats`] would publish.
    pub(super) fn same(
        &self,
        division: u64,
        previous: &replay::rows::Recorded<Option<DivisionAttackStats>>,
    ) -> bool {
        let replay::rows::Recorded::Value(previous) = previous else {
            return false;
        };
        let current = self.row(division).and_then(|row| {
            Some((
                row.scalars?,
                self.terrain.get(row.start..row.start + row.len)?,
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

    /// The witnessed native operands of a division, with the hour's
    /// acclimatization factor.
    pub(super) fn witness(
        &self,
        division: u64,
        factor: Option<i64>,
    ) -> Option<DivisionAttackInputs> {
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
            ..Captured::default()
        };
        captured.rows = vec![
            Row {
                division: 3,
                start: 0,
                len: 2,
                scalars: Some(Scalars {
                    river: [1, 2],
                    fort: [3, 4],
                    snow: 5,
                    night: 6,
                }),
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
        assert!(captured.same(3, &replay::rows::Recorded::Value(Some(stats.clone()))));
        let mut changed = stats;
        changed.terrain[1] = 1;
        assert!(!captured.same(3, &replay::rows::Recorded::Value(Some(changed))));
        // A refused read publishes None, as does a division never reached.
        assert_eq!(captured.stats(9), None);
        assert!(captured.same(9, &replay::rows::Recorded::Value(None)));
        assert!(captured.same(4, &replay::rows::Recorded::Value(None)));
        assert!(!captured.same(3, &replay::rows::Recorded::NotRecorded));
    }

    #[test]
    fn a_witness_adds_each_live_terrains_own_adjuster() {
        let fresh = stats::AttackStats {
            terrain: vec![-10_000, -20_000],
            river: [1, 2],
            fort: [3, 4],
            snow: 5,
            night: 6,
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
