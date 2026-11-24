use crate::army_stat_changes::{self, Changes, Counts};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::carmy::DivisionSeed;
use engine_native_parts::csub_unit_definition::{self as stats, DivisionMaxima};
use replay_data::{
    rows::AttackStats, rows::AttackTerrain, rows::DivisionAttackInputs, rows::DivisionAttackStats,
    rows::DivisionMovementStats, rows::PacketHour,
};

mod preparation;
pub use crate::division_experience::Order as ScalarOrder;
pub use preparation::{AttackPart, PreparationPart, PreparedDivision};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scalars {
    panel: [i64; 23],
    panel_mask: u32,
    flags: u8,
    suppression: i64,
    river: [i64; 2],
    fort: [i64; 2],
    snow: i64,
    night: i64,
    movement: [i64; 3],
    fuel_rate: i64,
    air_defense: i64,
}

impl Scalars {
    fn of(stats: &stats::AttackStats) -> Self {
        let mut panel = [0_i64; 23];
        let mut panel_mask = 0_u32;
        for (index, slot) in stats.panel.iter().enumerate() {
            if let Some(value) = slot {
                panel[index] = *value;
                panel_mask |= 1_u32 << index;
            }
        }
        let mut flags = 0_u8;
        let suppression = match stats.suppression {
            Some(value) => {
                flags |= 1;
                value
            }
            None => 0,
        };
        let movement = match stats.movement {
            Some(value) => {
                flags |= 2;
                value
            }
            None => [0; 3],
        };
        Self {
            panel,
            panel_mask,
            flags,
            suppression,
            river: stats.river,
            fort: stats.fort,
            snow: stats.snow,
            night: stats.night,
            movement,
            fuel_rate: stats.fuel_rate,
            air_defense: stats.air_defense,
        }
    }

    fn panel_at(&self, index: usize) -> Option<i64> {
        ((self.panel_mask & (1_u32 << index)) != 0).then_some(self.panel[index])
    }

    fn suppression(&self) -> Option<i64> {
        ((self.flags & 1) != 0).then_some(self.suppression)
    }

    fn movement(&self) -> Option<[i64; 3]> {
        ((self.flags & 2) != 0).then_some(self.movement)
    }
}

static NEXT_STAMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_stamp() -> u64 {
    let stamp = NEXT_STAMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if stamp == 0 || stamp > i64::MAX as u64 {
        0
    } else {
        stamp
    }
}

const COMPACT_SLABS: usize = 16;

#[derive(Default)]
struct TerrainSlab {
    terrain: Vec<i64>,
    movement: Vec<i64>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Row {
    division: u64,
    slab: u16,
    start: usize,
    len: usize,
    stamp: u64,
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
    slab: u16,
    start: usize,
    len: usize,
    stamp: u64,
    scalars: Scalars,
    maxima: Option<DivisionMaxima>,
}

static NEXT_NAME_STAMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_name_stamp() -> u64 {
    NEXT_NAME_STAMP
        .fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |next| next.checked_add(1),
        )
        .unwrap_or(0)
}

#[derive(Clone, Copy)]
pub struct HeldName {
    division: u64,
    pub member: usize,
    pub span: [u8; engine_native::cname_group_member::INPUT_SPAN],
    pub inputs: Option<engine_native::cname_group_member::Inputs>,
    pub group_null: bool,
    custom: (usize, usize),
    group: (usize, usize),
    text: (usize, usize),
    pub stamp: u64,
}

pub struct KeptName<'a> {
    pub division: u64,
    pub member: usize,
    pub span: &'a [u8; engine_native::cname_group_member::INPUT_SPAN],
    pub inputs: Option<engine_native::cname_group_member::Inputs>,
    pub group_null: bool,
    pub custom: &'a [u8],
    pub group: &'a [u8],
    pub text: &'a [u8],
    pub stamp: u64,
}

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
        if !self.index(divisions) {
            return None;
        }
        Self::find(&self.order, &mut self.shift, divisions, index, division)
    }

    fn index(&mut self, divisions: &[u64]) -> bool {
        if !self.order.is_empty() || divisions.is_empty() {
            return true;
        }
        if engine_allocation::advisory(|| engine_allocation::reserve(&mut self.order, divisions.len()))
            .is_err()
        {
            return false;
        }
        self.order.extend(
            divisions
                .iter()
                .enumerate()
                .map(|(at, &division)| (division, at as u32)),
        );
        self.order.sort_unstable();
        true
    }

    fn find(
        order: &[(u64, u32)],
        shift: &mut isize,
        divisions: &[u64],
        index: usize,
        division: u64,
    ) -> Option<usize> {
        if let Some(guess) = index.checked_add_signed(*shift)
            && divisions.get(guess) == Some(&division)
        {
            return Some(guess);
        }
        let at = order[order.partition_point(|entry| entry.0 < division)..]
            .first()
            .filter(|entry| entry.0 == division)?
            .1 as usize;
        *shift = at as isize - index as isize;
        Some(at)
    }
}

pub struct ArmyStatStore {
    experience: crate::division_experience::Store,
    progression: crate::division_experience::Store<u64>,
    officer: crate::division_experience::Store<i64>,
    career: crate::division_experience::Store<i32>,
    supply: crate::division_experience::Store<std::sync::Arc<crate::supply_inputs::Sample>>,
    daily: bool,
    pub history: crate::division_history::Store,
    pub sources: crate::panel_sources::Store,
    pub observations: crate::observations::HourObservations,
    epoch: Option<u64>,
    held: Vec<Held>,
    divisions: Vec<u64>,
    locator: Locator,
    slabs: Vec<std::sync::Arc<TerrainSlab>>,
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

const DAILY_SUPPLY_SAMPLE: Option<(u64, u64)> = Some((0, 0));

impl ArmyStatStore {
    pub fn daily_progress(&mut self, refresh: bool) {
        self.observations.finish();
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
            observations: Default::default(),
            epoch: None,
            held: Vec::new(),
            divisions: Vec::new(),
            locator: Locator::default(),
            slabs: Vec::new(),
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

    pub fn forget(&mut self) {
        self.experience.forget();
        self.progression.forget();
        self.officer.forget();
        self.career.forget();
        self.supply.forget();
        self.history.forget();
        self.sources.forget();
        self.observations.forget();
        self.epoch = None;
        self.divisions.clear();
        self.name_divisions.clear();
        self.locator.clear();
        self.name_locator.clear();
        self.held.clear();
        self.slabs.clear();
        self.next.clear();
        self.next_terrain.clear();
        self.next_movement.clear();
        self.names.clear();
        self.name_bytes.clear();
        self.next_names.clear();
        self.next_name_bytes.clear();
    }

    pub fn allocated_bytes(&self) -> usize {
        let slab_bytes: usize = self
            .slabs
            .iter()
            .map(|slab| (slab.terrain.capacity() + slab.movement.capacity()) * 8)
            .sum();
        self.experience.allocated_bytes()
            + self.supply.allocated_bytes()
            + self.progression.allocated_bytes()
            + self.officer.allocated_bytes()
            + self.career.allocated_bytes()
            + self.history.allocated_bytes()
            + self.sources.allocated_bytes()
            + self.observations.allocated_bytes()
            + (self.held.capacity() + self.next.capacity()) * std::mem::size_of::<Held>()
            + (self.divisions.capacity() + self.name_divisions.capacity()) * 8
            + (self.locator.order.capacity() + self.name_locator.order.capacity()) * 16
            + self.slabs.capacity() * std::mem::size_of::<std::sync::Arc<TerrainSlab>>()
            + slab_bytes
            + (self.next_terrain.capacity() + self.next_movement.capacity()) * 8
            + (self.names.capacity() + self.next_names.capacity()) * std::mem::size_of::<HeldName>()
            + self.name_bytes.capacity()
            + self.next_name_bytes.capacity()
    }

    pub fn held_name(&mut self, index: usize, division: u64) -> Option<usize> {
        #[cfg(any(test, feature = "test-support"))]
        {
            self.name_lookups += 1;
        }
        self.name_locator
            .locate(&self.name_divisions, index, division)
    }

    pub fn find_held_name(&self, shift: &mut isize, index: usize, division: u64) -> Option<usize> {
        Locator::find(
            &self.name_locator.order,
            shift,
            &self.name_divisions,
            index,
            division,
        )
    }

    pub fn held_name_record(&self, index: usize) -> &HeldName {
        &self.names[index]
    }

    pub fn held_name_text(&self, held: &HeldName) -> &[u8] {
        &self.name_bytes[held.text.0..held.text.0 + held.text.1]
    }

    pub fn held_name_custom(&self, held: &HeldName) -> &[u8] {
        &self.name_bytes[held.custom.0..held.custom.0 + held.custom.1]
    }

    pub fn held_name_group(&self, held: &HeldName) -> &[u8] {
        &self.name_bytes[held.group.0..held.group.0 + held.group.1]
    }

    pub fn begin_names(&mut self) {
        self.next_names.clear();
        self.next_name_bytes.clear();
    }

    pub fn keep_name(
        &mut self,
        division: u64,
        member: usize,
        span: &[u8; engine_native::cname_group_member::INPUT_SPAN],
        text: &[u8],
        stamp: u64,
    ) {
        self.keep_name_with_inputs(KeptName {
            division,
            member,
            span,
            inputs: None,
            group_null: false,
            custom: &[],
            group: &[],
            text,
            stamp,
        });
    }

    pub fn keep_name_with_inputs(&mut self, kept: KeptName<'_>) {
        let custom_start = self.next_name_bytes.len();
        self.next_name_bytes.extend_from_slice(kept.custom);
        let group_start = self.next_name_bytes.len();
        self.next_name_bytes.extend_from_slice(kept.group);
        let text_start = self.next_name_bytes.len();
        self.next_name_bytes.extend_from_slice(kept.text);
        self.next_names.push(HeldName {
            division: kept.division,
            member: kept.member,
            span: *kept.span,
            inputs: kept.inputs,
            group_null: kept.group_null,
            custom: (custom_start, kept.custom.len()),
            group: (group_start, kept.group.len()),
            text: (text_start, kept.text.len()),
            stamp: if kept.stamp != 0 {
                kept.stamp
            } else {
                next_name_stamp()
            },
        });
    }

    fn locate(&mut self, index: usize, division: u64) -> Option<usize> {
        self.locator.locate(&self.divisions, index, division)
    }

    #[cfg(test)]
    fn terrain(&self, held: &Held) -> &[i64] {
        &self.slabs[usize::from(held.slab - 1)].terrain[held.start..held.start + held.len]
    }

    fn flush_next_slab(&mut self) {
        if self.next_terrain.is_empty() && self.next_movement.is_empty() {
            return;
        }
        let slab_id = (self.slabs.len() + 1) as u16;
        let terrain = std::mem::take(&mut self.next_terrain);
        let movement = std::mem::take(&mut self.next_movement);
        self.slabs
            .push(std::sync::Arc::new(TerrainSlab { terrain, movement }));
        for held in &mut self.next {
            if held.slab == 0 {
                held.slab = slab_id;
            }
        }
    }

    fn compact_slabs(&mut self) {
        let live_words: usize = self.held.iter().map(|held| held.len).sum();
        if live_words == 0 {
            self.slabs.clear();
            return;
        }
        let stored_words: usize = self.slabs.iter().map(|slab| slab.terrain.len()).sum();
        if self.slabs.len() < COMPACT_SLABS
            && stored_words <= live_words.saturating_mul(2).saturating_add(8192)
        {
            return;
        }
        let mut terrain = Vec::new();
        let mut movement = Vec::new();
        let reserved = engine_allocation::advisory(|| {
            engine_allocation::reserve(&mut terrain, live_words)?;
            engine_allocation::reserve(&mut movement, live_words)
        });
        if reserved.is_err() {
            if self.slabs.len() >= usize::from(u16::MAX - 1) {
                self.forget();
            }
            return;
        }
        for held in &mut self.held {
            let slab = &self.slabs[usize::from(held.slab - 1)];
            let start = terrain.len();
            let span = held.start..held.start + held.len;
            terrain.extend_from_slice(&slab.terrain[span.clone()]);
            if !slab.movement.is_empty() {
                movement.extend_from_slice(&slab.movement[span]);
            }
            held.slab = 1;
            held.start = start;
        }
        self.slabs.clear();
        self.slabs
            .push(std::sync::Arc::new(TerrainSlab { terrain, movement }));
    }

    pub fn hold(&mut self, epoch: Option<u64>) {
        self.flush_next_slab();
        std::mem::swap(&mut self.held, &mut self.next);
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
        self.locator.index(&self.divisions);
        self.name_locator.index(&self.name_divisions);
        self.compact_slabs();
        self.epoch = epoch;
    }

    pub fn may_carry_maxima(&self, shift: &mut isize, index: usize, seed: &DivisionSeed) -> bool {
        self.epoch.is_some()
            && Locator::find(&self.locator.order, shift, &self.divisions, index, seed.division)
                .is_some_and(|at| {
                    let held = &self.held[at];
                    held.army == seed.pointer
                        && Some(held.stat_block) == seed.stat_block
                        && held.maxima.is_some()
                })
    }
}

pub const CERTIFY_PHASE_NAMES: [&str; 3] = ["certify_setup", "certify_ordered", "certify_finish"];

#[derive(Default)]
pub struct Captured {
    pub experience: Vec<Option<i64>>,
    pub exercise_experience: Vec<Option<i64>>,
    certify_ticks: [u64; CERTIFY_PHASE_NAMES.len()],
    scalars_deferred: bool,
    history_finish: Option<Result<(), &'static str>>,
    pending_templates: Vec<Option<u64>>,
    officer: Vec<Option<i64>>,
    career: Vec<Option<i32>>,
    pub sources: Option<Result<std::sync::Arc<crate::panel_sources::SourceSnapshot>, &'static str>>,
    pub selected_sources: Vec<(u64, Option<[u64; 3]>)>,
    selected_order: Vec<(u64, u32)>,
    pub history:
        Option<Result<std::sync::Arc<crate::division_history::HistorySnapshot>, &'static str>>,
    rows: Vec<Row>,
    order: Vec<(u64, u32)>,
    slabs: Vec<std::sync::Arc<TerrainSlab>>,
    terrain: Vec<i64>,
    movement: Vec<i64>,
    scratch: Vec<i64>,
    movement_scratch: Vec<i64>,
    held_index: Vec<Option<u32>>,
    registered: Vec<bool>,
    touched: Vec<usize>,
    epoch: Option<u64>,
    pub carryable: bool,
    pub held_maxima: Vec<Option<DivisionMaxima>>,
    pub status_counts: Counts,
    witness: Vec<(u64, DivisionAttackInputs)>,
    terrains: Vec<(i32, i64)>,
}

fn witnessed(hour: i32, id: u64) -> bool {
    let period = replay_data::cadence::STAGGER_PERIOD;
    army_stat_changes::WITNESS && id % period as u64 == hour.rem_euclid(period) as u64
}

impl Captured {
    pub fn reset(&mut self) {
        self.rows.clear();
        self.order.clear();
        self.slabs.clear();
        self.terrain.clear();
        self.movement.clear();
        self.witness.clear();
    }

    fn terrain_slice(&self, row: &Row) -> Option<&[i64]> {
        let span = row.start..row.start.saturating_add(row.len);
        if row.slab == 0 {
            self.terrain.get(span)
        } else {
            self.slabs
                .get(usize::from(row.slab - 1))?
                .terrain
                .get(span)
        }
    }

    fn movement_slice(&self, row: &Row) -> Option<&[i64]> {
        let span = row.start..row.start.saturating_add(row.len);
        if row.slab == 0 {
            self.movement.get(span)
        } else {
            self.slabs
                .get(usize::from(row.slab - 1))?
                .movement
                .get(span)
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.selected_sources.capacity() * std::mem::size_of::<(u64, Option<[u64; 3]>)>()
            + self.selected_order.capacity() * std::mem::size_of::<(u64, u32)>()
            + self.experience.capacity() * std::mem::size_of::<Option<i64>>()
            + self.exercise_experience.capacity() * std::mem::size_of::<Option<i64>>()
            + self.pending_templates.capacity() * std::mem::size_of::<Option<u64>>()
            + self.officer.capacity() * std::mem::size_of::<Option<i64>>()
            + self.career.capacity() * std::mem::size_of::<Option<i32>>()
            + self.rows.capacity() * std::mem::size_of::<Row>()
            + self.order.capacity() * std::mem::size_of::<(u64, u32)>()
            + self.slabs.capacity() * std::mem::size_of::<std::sync::Arc<TerrainSlab>>()
            + (self.terrain.capacity() + self.scratch.capacity()) * 8
            + (self.movement.capacity() + self.movement_scratch.capacity()) * 8
            + self.held_index.capacity() * 8
            + self.registered.capacity()
            + self.touched.capacity() * 8
            + self.held_maxima.capacity() * std::mem::size_of::<Option<DivisionMaxima>>()
            + self.witness.capacity() * std::mem::size_of::<(u64, DivisionAttackInputs)>()
            + self.terrains.capacity() * std::mem::size_of::<(i32, i64)>()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn epoch(&self) -> Option<u64> {
        self.epoch
    }

    pub fn touched(&self, receiver: usize) -> bool {
        self.touched.binary_search(&receiver).is_ok()
    }

    pub fn held_index(&self) -> &[Option<u32>] {
        &self.held_index
    }

    pub fn touched_receivers(&self) -> &[usize] {
        &self.touched
    }

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

    #[expect(
        clippy::too_many_arguments,
        reason = "certification takes the seeds, outcome, store, change source and preparation separately"
    )]
    pub fn certify_prepared<'a>(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
        outcome: Outcome,
        store: Option<&mut ArmyStatStore>,
        changes: &dyn Changes,
        prepared: impl Fn(usize) -> Option<&'a PreparedDivision>,
    ) {
        self.certify_staged(
            memory, base, seeds, outcome, store, changes, prepared, false,
        );
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "certification takes the seeds, outcome, store, change source, preparation and deferral separately"
    )]
    pub fn certify_staged<'a>(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
        outcome: Outcome,
        store: Option<&mut ArmyStatStore>,
        changes: &dyn Changes,
        prepared: impl Fn(usize) -> Option<&'a PreparedDivision>,
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
        self.status_counts = Counts::default();
        self.epoch = None;
        self.carryable = false;
        self.scalars_deferred = false;
        self.history_finish = None;
        let Some(store) = store else {
            return;
        };
        store.observations.take(changes);
        let armies = &store.observations.armies;
        self.epoch = armies.epoch();
        if engine_allocation::advisory(|| {
            engine_allocation::reserve(&mut self.touched, armies.touched_len())
        })
        .is_err()
        {
            self.epoch = None;
        } else {
            self.touched.extend_from_slice(armies.touched_receivers());
        }
        self.carryable = self.epoch.is_some() && store.epoch == self.epoch;
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
            store
                .history
                .begin_owned_with(seeds.len(), &store.observations.edits)
        } else {
            store
                .history
                .begin_with(seeds.len(), &store.observations.edits)
        };
        self.certify_ticks[0] = crate::clock::qpc().saturating_sub(begin).max(0) as u64;
        let ordered_begin = crate::clock::qpc();
        for (index, seed) in seeds.iter().enumerate() {
            use collector_observers::unit_history_changes as history_changes;
            let prepared = prepared(index);
            let observations = &store.observations;
            if officer_ready {
                let receiver = seed
                    .pointer
                    .checked_add(engine_native_land::unit_history::OFFICER);
                let quiet = receiver.filter(|_| prepared.is_none_or(|p| !p.acquired[2])).and_then(
                    |receiver| store.officer.quiet(seed, receiver, &observations.history),
                );
                let value = match quiet {
                    Some((at, _)) => store.officer.keep(
                        base,
                        seed,
                        at,
                        receiver.is_some_and(|receiver| observations.history.touched(receiver)),
                    ),
                    None => {
                        let certificate = match prepared.filter(|p| p.acquired[2]) {
                            Some(p) => p.officer_certificate,
                            None => receiver.and_then(history_changes::sequence),
                        };
                        store.officer.observe_with(base, seed, certificate, || {
                            prepared
                                .filter(|p| {
                                    certificate.is_some() && p.officer_certificate == certificate
                                })
                                .and_then(|p| p.officer)
                                .unwrap_or_else(|| {
                                    engine_native_land::unit_history::officer_experience(
                                        memory,
                                        base,
                                        seed.pointer,
                                    )
                                })
                        })
                    }
                };
                self.officer
                    .push(value.map_err(|_| history_changes::refused()).ok());
            }
            let experience_quiet = ((career_ready || experience_ready)
                && prepared.is_none_or(|p| !p.acquired[0]))
            .then(|| {
                store
                    .experience
                    .quiet(seed, seed.pointer, &observations.experience)
            })
            .flatten();
            let experience_certificate = (career_ready || experience_ready)
                .then(|| match (experience_quiet, prepared.filter(|p| p.acquired[0])) {
                    (Some((_, certificate)), _) => Some(certificate),
                    (None, Some(p)) => p.experience_certificate,
                    (None, None) => changes.experience_sequence(seed.pointer),
                })
                .flatten();
            if career_ready {
                let quiet = experience_certificate
                    .filter(|_| prepared.is_none_or(|p| !p.acquired[3]))
                    .and_then(|_| store.career.quiet(seed, seed.pointer, &observations.history));
                let value = match quiet {
                    Some((at, _)) => store.career.keep(base, seed, at, false),
                    None => {
                        let certificate = match prepared.filter(|p| p.acquired[3]) {
                            Some(p) => p.career_certificate,
                            None => experience_certificate
                                .and_then(|_| history_changes::sequence(seed.pointer)),
                        };
                        store.career.observe_with(base, seed, certificate, || {
                            prepared
                                .filter(|p| {
                                    certificate.is_some() && p.career_certificate == certificate
                                })
                                .and_then(|p| p.career)
                                .unwrap_or_else(|| {
                                    engine_native_land::unit_history::career_counter(memory, seed.pointer)
                                })
                        })
                    }
                };
                self.career
                    .push(value.map_err(|_| history_changes::refused()).ok());
            }
            if progression_ready {
                let quiet = prepared
                    .is_none_or(|p| !p.acquired[1])
                    .then(|| {
                        store
                            .progression
                            .quiet(seed, seed.pointer, &observations.progression)
                    })
                    .flatten();
                let value = match quiet {
                    Some((at, _)) => store.progression.keep(base, seed, at, false),
                    None => {
                        let certificate = match prepared.filter(|p| p.acquired[1]) {
                            Some(p) => p.progression_certificate,
                            None => changes.progression_sequence(seed.pointer),
                        };
                        store.progression.observe_with(base, seed, certificate, || {
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
                    }
                };
                self.pending_templates
                    .push(value.map_err(army_stat_changes::progression_refused).ok());
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
                    Some(&store.observations.history),
                );
            }
            if experience_ready {
                let certificate = experience_certificate;
                let value = match experience_quiet {
                    Some((at, _)) => store.experience.keep(
                        base,
                        seed,
                        at,
                        store.observations.experience.touched(seed.pointer),
                    ),
                    None => store.experience.observe_with(base, seed, certificate, || {
                        prepared
                            .filter(|p| {
                                certificate.is_some() && p.experience_certificate == certificate
                            })
                            .and_then(|p| p.experience)
                            .unwrap_or_else(|| {
                                engine_native_memory::read::<i64>(
                                    memory,
                                    seed.pointer
                                        .checked_add(engine_native::carmy::EXPERIENCE)
                                        .ok_or("division experience address overflow")?,
                                )
                                .ok_or("division experience unreadable")
                            })
                    }),
                };
                self.experience
                    .push(value.map_err(army_stat_changes::experience_refused).ok());
                if exercise_ready {
                    self.exercise_experience
                        .push(store.experience.current(seed));
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
            self.held_maxima.push(maxima);
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
    pub fn settle_certification<'a>(
        &mut self,
        base: usize,
        seeds: &[DivisionSeed],
        outcome: Outcome,
        store: &mut ArmyStatStore,
        prepared: impl Fn(usize) -> Option<&'a PreparedDivision>,
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

    #[expect(
        clippy::too_many_arguments,
        reason = "the capture takes the seeds, outcome, store and maxima beside the read operands"
    )]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        hour: i32,
        seeds: &[DivisionSeed],
        outcome: Outcome,
        store: Option<&mut ArmyStatStore>,
        maxima: impl Fn(usize) -> Option<DivisionMaxima>,
    ) {
        self.capture_prepared(memory, base, hour, seeds, outcome, store, maxima, |_| None);
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the capture takes the seeds, outcome, store, maxima and preparation beside the read operands"
    )]
    pub fn capture_prepared<'a>(
        &mut self,
        memory: &impl Memory,
        base: usize,
        hour: i32,
        seeds: &[DivisionSeed],
        outcome: Outcome,
        store: Option<&mut ArmyStatStore>,
        maxima: impl Fn(usize) -> Option<DivisionMaxima>,
        prepared: impl Fn(usize) -> Option<Result<&'a stats::AttackStats, &'static str>>,
    ) {
        self.prepare_witness(memory, base, hour, seeds, outcome);
        self.capture_values(memory, base, hour, seeds, outcome, store, maxima, prepared);
    }

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
            if engine_native_parts::cterrain_database::terrain_attacks(memory, base, &mut self.terrains)
                .is_err()
            {
                self.terrains.clear();
            }
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the owned capture takes the seeds, outcome, store, maxima and preparation separately"
    )]
    pub fn capture_owned<'a>(
        &mut self,
        base: usize,
        hour: i32,
        seeds: &[DivisionSeed],
        outcome: Outcome,
        store: Option<&mut ArmyStatStore>,
        maxima: impl Fn(usize) -> Option<DivisionMaxima>,
        prepared: impl Fn(usize) -> Option<Result<&'a stats::AttackStats, &'static str>>,
    ) {
        self.capture_values(
            &OwnedAttack,
            base,
            hour,
            seeds,
            outcome,
            store,
            maxima,
            prepared,
        );
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the value capture takes the seeds, outcome, store, maxima and preparation beside the read operands"
    )]
    fn capture_values<'a>(
        &mut self,
        memory: &impl Memory,
        base: usize,
        hour: i32,
        seeds: &[DivisionSeed],
        outcome: Outcome,
        mut store: Option<&mut ArmyStatStore>,
        maxima: impl Fn(usize) -> Option<DivisionMaxima>,
        prepared: impl Fn(usize) -> Option<Result<&'a stats::AttackStats, &'static str>>,
    ) {
        self.reset();
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
            let held = store.as_deref().and_then(|store| {
                let at = self.held_index.get(index).copied().flatten()?;
                Some(store.held[at as usize]).filter(|held| held.stat_block == stat_block)
            });
            let registered = self.registered.get(index).copied().unwrap_or(false);
            if let (Some(held), Some(store)) = (held, store.as_deref_mut())
                && !witnessed(hour, id)
            {
                counts.reused += 1;
                self.rows.push(Row {
                    division: seed.division,
                    slab: held.slab,
                    start: held.start,
                    len: held.len,
                    stamp: held.stamp,
                    scalars: Some(held.scalars),
                    experience,
                    officer_experience,
                    career_counter,
                    pending_template,
                });
                store.next.push(Held {
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
            match fresh {
                Ok(fresh) => {
                    if fresh.panel.iter().any(Option::is_none) || fresh.suppression.is_none() {
                        counts.refused += 1;
                    }
                    let scalars = Scalars::of(fresh);
                    let stamp = if store.is_some() && registered {
                        next_stamp()
                    } else {
                        0
                    };
                    if let Some(store) = store.as_deref_mut()
                        && registered
                        && store.slabs.len() < usize::from(u16::MAX - 1)
                    {
                        let slab = (store.slabs.len() + 1) as u16;
                        let start = store.next_terrain.len();
                        store.next_terrain.extend_from_slice(&fresh.terrain);
                        store
                            .next_movement
                            .extend_from_slice(&fresh.movement_terrain);
                        self.rows.push(Row {
                            division: seed.division,
                            slab,
                            start,
                            len: fresh.terrain.len(),
                            stamp,
                            scalars: Some(scalars),
                            experience,
                            officer_experience,
                            career_counter,
                            pending_template,
                        });
                        store.next.push(Held {
                            division: seed.division,
                            army: seed.pointer,
                            stat_block,
                            slab,
                            start,
                            len: fresh.terrain.len(),
                            stamp,
                            scalars,
                            maxima: maxima(index),
                        });
                    } else {
                        let start = self.terrain.len();
                        self.terrain.extend_from_slice(&fresh.terrain);
                        self.movement.extend_from_slice(&fresh.movement_terrain);
                        self.rows.push(Row {
                            division: seed.division,
                            slab: 0,
                            start,
                            len: fresh.terrain.len(),
                            stamp: 0,
                            scalars: Some(scalars),
                            experience,
                            officer_experience,
                            career_counter,
                            pending_template,
                        });
                    }
                    if witnessed(hour, id) && !self.terrains.is_empty() {
                        counts.witnessed += 1;
                        self.witness
                            .push((seed.division, witness(fresh, &self.terrains)));
                    }
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
        self.order.clear();
        if engine_allocation::advisory(|| engine_allocation::reserve(&mut self.order, self.rows.len()))
            .is_ok()
        {
            self.order.extend(
                self.rows
                    .iter()
                    .enumerate()
                    .map(|(at, row)| (row.division, at as u32)),
            );
            self.order.sort_unstable_by_key(|entry| entry.0);
        } else {
            self.rows.sort_unstable_by_key(|row| row.division);
        }
        self.witness.sort_unstable_by_key(|(division, _)| *division);
        let complete = matches!(outcome, Outcome::Rows { .. });
        if let Some(store) = store {
            store.flush_next_slab();
            self.slabs.extend(store.slabs.iter().cloned());
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

    pub fn index_selected_sources(&mut self) {
        self.selected_order.clear();
        if engine_allocation::advisory(|| {
            engine_allocation::reserve(&mut self.selected_order, self.selected_sources.len())
        })
        .is_err()
        {
            self.selected_sources.sort_unstable_by_key(|row| row.0);
            return;
        }
        self.selected_order.extend(
            self.selected_sources
                .iter()
                .enumerate()
                .map(|(at, row)| (row.0, at as u32)),
        );
        self.selected_order.sort_unstable_by_key(|entry| entry.0);
    }
    pub fn clear_selected_sources(&mut self) {
        self.selected_sources.clear();
        self.selected_order.clear();
    }
    fn indexed_sources(&self) -> bool {
        self.selected_order.len() == self.selected_sources.len()
    }
    pub fn panel_source(&self, division: u64) -> Option<[u64; 3]> {
        if !self.indexed_sources() {
            return self
                .selected_sources
                .binary_search_by_key(&division, |row| row.0)
                .ok()
                .and_then(|at| self.selected_sources[at].1);
        }
        let at = self
            .selected_order
            .binary_search_by_key(&division, |entry| entry.0)
            .ok()?;
        self.selected_sources[self.selected_order[at].1 as usize].1
    }
    fn panel_source_at(&self, position: usize, division: u64) -> Option<[u64; 3]> {
        match self.selected_sources.get(position) {
            Some(&(at, source)) if at == division && self.indexed_sources() => source,
            _ => self.panel_source(division),
        }
    }
    pub fn panel(&self, division: u64) -> [Option<i64>; 23] {
        self.view(division).panel()
    }

    pub fn experience(&self, division: u64) -> Option<i64> {
        self.row(division)?.experience
    }

    pub fn view_at(&self, position: usize, division: u64) -> RowView<'_> {
        let row = self
            .rows
            .get(position)
            .filter(|row| row.division == division && self.order.len() == self.rows.len())
            .or_else(|| self.row(division));
        RowView {
            captured: self,
            division,
            position: Some(position),
            row,
        }
    }

    fn row(&self, division: u64) -> Option<&Row> {
        if self.order.len() != self.rows.len() {
            return self
                .rows
                .binary_search_by_key(&division, |row| row.division)
                .ok()
                .map(|at| &self.rows[at]);
        }
        self.order
            .binary_search_by_key(&division, |entry| entry.0)
            .ok()
            .map(|at| &self.rows[self.order[at].1 as usize])
    }

    pub fn view(&self, division: u64) -> RowView<'_> {
        RowView {
            captured: self,
            division,
            position: None,
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
        !self.witness.is_empty()
            && self
                .witness
                .binary_search_by_key(&division, |(division, _)| *division)
                .is_ok()
    }

    pub fn has_unique_divisions(&self, rows: &[engine_native::carmy::StatusRow]) -> bool {
        self.order.len() == rows.len()
            && self.rows.len() == rows.len()
            && self
                .rows
                .iter()
                .zip(rows)
                .all(|(a, r)| a.division == r.division)
            && self.order.windows(2).all(|pair| pair[0].0 < pair[1].0)
    }

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

pub struct RowView<'a> {
    captured: &'a Captured,
    division: u64,
    position: Option<usize>,
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
        match self.position {
            Some(position) => self.captured.panel_source_at(position, self.division),
            None => self.captured.panel_source(self.division),
        }
    }
    pub fn panel(&self) -> [Option<i64>; 23] {
        self.row
            .and_then(|row| row.scalars)
            .map_or([None; 23], |scalars| {
                std::array::from_fn(|index| match index {
                    22 => scalars.suppression(),
                    _ => scalars.panel_at(index + usize::from(index >= 11)),
                })
            })
    }
    pub fn panel_stamp(&self) -> Option<u64> {
        match self.row.and_then(|row| row.scalars.map(|_| row.stamp)) {
            None => Some(u64::MAX),
            Some(0) => None,
            Some(stamp) => Some(stamp),
        }
    }
    pub fn panel_stat(&self, index: usize) -> Option<i64> {
        let scalars = self.row?.scalars?;
        match index {
            22 => scalars.suppression(),
            index if index < 22 => scalars.panel_at(index + usize::from(index >= 11)),
            _ => None,
        }
    }
    pub fn witness(&self, factor: Option<i64>) -> Option<DivisionAttackInputs> {
        self.captured.witness(self.division, factor)
    }
    pub fn stats(&self) -> Option<DivisionAttackStats> {
        let row = self.row?;
        let scalars = row.scalars?;
        Some(DivisionAttackStats {
            terrain: engine_allocation::owned_vec(engine_allocation::copy_vec(
                self.captured.terrain_slice(row)?,
            )),
            river: scalars.river,
            fort: scalars.fort,
            snow: scalars.snow,
            night: scalars.night,
        })
    }

    pub fn movement(&self) -> Option<DivisionMovementStats> {
        let row = self.row?;
        let scalars = row.scalars?;
        Some(DivisionMovementStats {
            terrain: engine_allocation::owned_vec(engine_allocation::copy_vec(
                self.captured.movement_slice(row)?,
            )),
            scalars: scalars.movement(),
            fuel_rate: scalars.fuel_rate,
            air_defense: scalars.air_defense,
        })
    }

    pub fn record(&self, record: &mut collector_projection::projection::Record<'_>) {
        let Some((row, scalars)) = self.row.and_then(|row| Some((row, row.scalars?))) else {
            record.word(-1);
            return;
        };
        if row.stamp != 0 {
            record.word(row.stamp as i64);
            return;
        }
        record.word(0);
        let movement = scalars.movement().unwrap_or_default();
        let words = [
            scalars.river[0],
            scalars.river[1],
            scalars.fort[0],
            scalars.fort[1],
            scalars.snow,
            scalars.night,
            i64::from(scalars.movement().is_some()),
            movement[0],
            movement[1],
            movement[2],
            scalars.fuel_rate,
            scalars.air_defense,
        ];
        record.words(Some(words.as_slice()));
        record.words(self.captured.terrain_slice(row));
        record.words(self.captured.movement_slice(row));
    }

    pub fn same_movement(
        &self,
        previous: &replay_data::rows::Recorded<Option<DivisionMovementStats>>,
    ) -> bool {
        let replay_data::rows::Recorded::Value(previous) = previous else {
            return false;
        };
        let current = self.row.and_then(|row| {
            Some((row.scalars?, self.captured.movement_slice(row)?))
        });
        match (current, previous) {
            (None, None) => true,
            (Some((scalars, terrain)), Some(previous)) => {
                terrain == previous.terrain.as_slice()
                    && scalars.movement() == previous.scalars
                    && scalars.fuel_rate == previous.fuel_rate
                    && scalars.air_defense == previous.air_defense
            }
            _ => false,
        }
    }

    pub fn same(
        &self,
        previous: &replay_data::rows::Recorded<Option<DivisionAttackStats>>,
    ) -> bool {
        let replay_data::rows::Recorded::Value(previous) = previous else {
            return false;
        };
        let current = self.row.and_then(|row| {
            Some((row.scalars?, self.captured.terrain_slice(row)?))
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
                slab: 0,
                start: 0,
                len: terrain.len(),
                stamp: 0,
                scalars: Scalars::default(),
                maxima: None,
            },
            terrain.to_vec(),
        )
    }

    #[test]
    fn the_locator_follows_census_shifts_and_misses_gone_divisions() {
        let held: Vec<u64> = (1..=6).map(|id| id << 32).collect();
        let mut locator = Locator::default();
        assert_eq!(locator.locate(&held, 2, 3 << 32), Some(2));
        assert!(locator.order.is_empty());
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
    fn the_stored_panel_ends_with_the_suppression_word() {
        let mut panel = [None; 23];
        for (index, word) in panel.iter_mut().enumerate() {
            *word = Some(index as i64 * 10);
        }
        let mut captured = Captured::default();
        captured.rows = vec![Row {
            division: 3,
            scalars: Some(Scalars::of(&stats::AttackStats {
                panel,
                suppression: Some(-7),
                ..Default::default()
            })),
            ..Default::default()
        }];
        let stored = captured.panel(3);
        assert_eq!(stored[10], Some(100));
        assert_eq!(stored[11], Some(120));
        assert_eq!(stored[21], Some(220));
        assert_eq!(stored[22], Some(-7));
        assert_eq!(captured.panel(9), [None; 23]);
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
                scalars: Some(Scalars::of(&stats::AttackStats {
                    river: [1, 2],
                    fort: [3, 4],
                    snow: 5,
                    night: 6,
                    movement: Some([-1, -2, -3]),
                    fuel_rate: 40,
                    air_defense: 50,
                    ..Default::default()
                })),
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
        assert_eq!(captured.stats(9), None);
        assert!(captured.same(9, &replay_data::rows::Recorded::Value(None)));
        assert!(captured.same(4, &replay_data::rows::Recorded::Value(None)));
        assert!(!captured.same(3, &replay_data::rows::Recorded::NotRecorded));
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
                ..movement
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
                AttackTerrain {
                    terrain: 3,
                    attack: None
                },
            ]
        );
        assert_eq!(packet.stats.night, Some(6));
    }
}
