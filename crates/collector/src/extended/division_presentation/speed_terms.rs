//! The land movement speed's shared operands, read beside the division status
//! pass from the same census seeds: MovementStateTerms, CoreSupplyAccess,
//! CountryUnitSpeedEntries, RiverRelations and DivisionTransportSpeeds for the
//! hour's movers (divisions with a nonempty ordinary route). Land families
//! exclude transfer, sea, invasion and unblocked redeployment branches; combat
//! movement is read whenever an itinerary edge uses ordinary land speed.
//!
//! Each operand is read once per sharing key, never per division: each
//! province's state, controller, controller area, strategic region and sea flag
//! once per unique province of the movers' locations and routes
//! ([`cprovince::movement_province`]); the state's id and controller once per
//! state; the state block once per (state, roster owner) and the controller
//! branch once per (owner, state controller); core supply access once per
//! (owner, controller area), with province-keyed rows, only for movers whose noncore-supply term can
//! apply (exile and noncore bypass bytes both zero); each combat once; the
//! relation river crossing entry once per (owner, controller) over route
//! provinces; the unit speed entries once per (owner country, key) over the
//! movement speed keys of the movers' templates; and the transport speed
//! through one batched query per logical owner sharing one [`SpeedCache`]
//! ([`engine_naval_history::cnaval_unit_transfer::movement_speeds`]).
//!
//! CountryUnitSpeedEntries are carried across hours under the unit speed set
//! ([`Table::UnitSpeeds`]), which every writer of the counter set marks: a
//! country unmarked since its held read keeps its held entries; a marked or new
//! country, or one needing a key its held read lacks, is read afresh.
//!
//! A refused read is that row's counted refusal; a mover whose key cannot be
//! formed (its location, state, route or template unreadable) has no row, and
//! the refusal is counted under the family.
use engine_executable::modifier_keys as keys;
use engine_native::{
    Memory, carmy::DivisionSeed, ccountry, cpdx_array::Array, cprovince,
    modifier_accumulator as accumulator, modifier_accumulator::Value, walk::Outcome,
};
use engine_naval_history::{
    cnaval_unit_transfer::SpeedCache, cnaval_unit_transfer::TransportRegion,
};
use replay_data::{
    rows::CoreSupplyAccessRow, rows::CountryUnitSpeedEntryRow, rows::MovementStateTermRow,
    rows::OwnerTransportSpeedRow, rows::RiverRelationRow, rows::StateMovementTerms, rows::TableId,
    rows::tables,
};
use replay_frame::frame::TableInput;
#[cfg(test)]
use replay_frame::frame::TableView;
use std::sync::Mutex;

use crate::country_input_changes::{self as changes, Changes, Counts, Table};

mod owned;

pub const N: usize = 5;
pub const TABLES: [TableId; N] = [
    TableId::MovementStateTerms,
    TableId::CoreSupplyAccess,
    TableId::CountryUnitSpeedEntries,
    TableId::RiverRelations,
    TableId::DivisionTransportSpeeds,
];

/// The count families: the five tables, then DivisionStatus.combat_movement.
const FAMILIES: usize = N + 1;
const COMBAT: usize = N;

/// The ordered state block keys: army speed, controller speed, enemy speed,
/// attack speed.
const STATE_KEYS: [u32; 4] = [
    keys::ARMY_SPEED_FACTOR,
    keys::ARMY_SPEED_FACTOR_FOR_CONTROLLER,
    keys::ENEMY_ARMY_SPEED_FACTOR,
    keys::ARMY_ATTACK_SPEED_FACTOR,
];
const _: () = assert!(
    STATE_KEYS[0] < STATE_KEYS[1] && STATE_KEYS[1] < STATE_KEYS[2] && STATE_KEYS[2] < STATE_KEYS[3]
);

const UNCAPTURED: Outcome = Outcome::Unavailable {
    reason: "division speed terms not captured",
};
const ALLOCATION: &str = "division speed term allocation failed";

/// One mover's keys, from its seed.
#[derive(Clone, Copy)]
struct Mover {
    division: u64,
    attacking: Result<bool, &'static str>,
    /// Whether any route edge reaches ordinary land speed.
    land: Result<bool, &'static str>,
    strategic: Option<u8>,
    roster: Option<u32>,
    /// Expeditionary owner if positive, else roster owner (raw).
    owner: Option<u32>,
    logical: Option<u32>,
    /// The location province's id.
    location: Result<u32, &'static str>,
    template: Option<usize>,
    /// Exile and noncore bypass bytes both zero: the noncore term can apply.
    noncore: Option<bool>,
    /// A nonnull naval transfer membership.
    transfer: Option<bool>,
    /// Its route provinces in the hour's route arena.
    route: Result<(usize, usize), &'static str>,
}

#[derive(Clone, Copy)]
struct CombatOperands {
    land: bool,
    start: usize,
    len: usize,
    word: Option<Result<Option<i64>, &'static str>>,
    complete: Result<(), &'static str>,
}

/// Common state entries only; the attack entry has its own conditional verdict.
fn state_keys(branch: u8) -> ([u32; 2], usize) {
    match branch {
        StateMovementTerms::FRIENDLY => ([STATE_KEYS[0], STATE_KEYS[1]], 2),
        StateMovementTerms::ENEMY => ([STATE_KEYS[0], STATE_KEYS[2]], 2),
        _ => ([STATE_KEYS[0], 0], 1),
    }
}

/// One country's held unit speed entries.
#[derive(Clone, Copy)]
struct HeldCountry {
    owner: u32,
    country: ccountry::Country,
    start: usize,
    len: usize,
}

/// The latest held read of CountryUnitSpeedEntries.
#[derive(Default)]
struct Held {
    /// The take epoch it was certified under; `None` holds nothing.
    epoch: Option<u64>,
    arrays: [Array; 2],
    /// By owner, ascending.
    countries: Vec<HeldCountry>,
    keys: Vec<u32>,
    values: Vec<Option<i64>>,
}

impl Held {
    fn clear(&mut self) {
        self.epoch = None;
        self.countries.clear();
        self.keys.clear();
        self.values.clear();
    }

    fn bytes(&self) -> usize {
        self.countries.capacity() * size_of::<HeldCountry>()
            + self.keys.capacity() * size_of::<u32>()
            + self.values.capacity() * size_of::<Option<i64>>()
    }
}

/// River crossing answers by (from, to) province id for one world. A province's
/// neighbour crossing kinds are written only by the map load and the authored
/// adjacency loader the straits reload runs again; both start a new world. One
/// world's answer serves its every later hour.
#[derive(Default)]
struct HeldCrossings {
    world: Option<u64>,
    answers: std::collections::HashMap<(u32, u32), Option<u8>>,
    charge: crate::raw_memory::Charge,
}
#[cfg(not(test))]
static HELD_CROSSINGS: Mutex<Option<HeldCrossings>> = Mutex::new(None);
#[cfg(test)]
thread_local! {
    // Each test thread is its own process.
    static HELD_CROSSINGS: Mutex<Option<HeldCrossings>> = const { Mutex::new(None) };
}

fn held_crossings<R>(visit: impl FnOnce(&mut HeldCrossings) -> R) -> R {
    let with = |slot: &Mutex<Option<HeldCrossings>>| {
        visit(
            slot.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get_or_insert_with(HeldCrossings::default),
        )
    };
    #[cfg(not(test))]
    return with(&HELD_CROSSINGS);
    #[cfg(test)]
    HELD_CROSSINGS.with(with)
}

/// `carmy::movement_attacking` for the hour's land movers from their route copy
/// and census owner. Each destination id and each (controller, logical owner)
/// pair is read once per pass rather than once per mover. Only a successful
/// read is kept: a failure stays the reading mover's and is read again after.
#[derive(Default)]
struct Hostility {
    provinces: Option<Array>,
    /// Each destination's controller, or `None` for an empty province slot.
    destinations: std::collections::HashMap<u32, Option<i32>>,
    pairs: std::collections::HashMap<(u32, u32), bool>,
}

impl Hostility {
    fn clear(&mut self) {
        self.provinces = None;
        self.destinations.clear();
        self.pairs.clear();
    }

    fn allocated_bytes(&self) -> usize {
        self.destinations.capacity() * (size_of::<(u32, Option<i32>)>() + 1)
            + self.pairs.capacity() * (size_of::<((u32, u32), bool)>() + 1)
    }

    /// The mover whose route ends at `last` and whose logical owner is `logical`.
    /// A memo that cannot grow only costs its repeated read.
    fn attacking(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game: usize,
        last: Result<u32, &'static str>,
        logical: Option<u32>,
    ) -> Result<bool, &'static str> {
        let last = last?;
        let destination = match self.destinations.get(&last) {
            Some(&destination) => destination,
            None => {
                let provinces = match self.provinces {
                    Some(provinces) => provinces,
                    None => *self
                        .provinces
                        .insert(engine_native::carmy::movement_province_array(memory, game)?),
                };
                let destination = match engine_native::carmy::movement_route_destination(
                    memory, base, provinces, last,
                )? {
                    0 => None,
                    province => Some(engine_native::carmy::movement_destination_controller(
                        memory, province,
                    )?),
                };
                if engine_allocation::advisory(|| {
                    engine_allocation::reserve_map(&mut self.destinations, 1)
                })
                .is_ok()
                {
                    self.destinations.insert(last, destination);
                }
                destination
            }
        };
        let Some(controller) = destination else {
            return Ok(false);
        };
        if controller <= 0 {
            return Ok(false);
        }
        let pair = (
            controller as u32,
            logical.ok_or("division presentation field unreadable")?,
        );
        if let Some(&hostile) = self.pairs.get(&pair) {
            return Ok(hostile);
        }
        let hostile = ccountry::movement_enemy(memory, base, game, pair.0, pair.1)?;
        if engine_allocation::advisory(|| engine_allocation::reserve_map(&mut self.pairs, 1))
            .is_ok()
        {
            self.pairs.insert(pair, hostile);
        }
        Ok(hostile)
    }
}

pub(super) struct SpeedTerms {
    states: Vec<MovementStateTermRow>,
    access: Vec<CoreSupplyAccessRow>,
    entries: Vec<CountryUnitSpeedEntryRow>,
    rivers: Vec<RiverRelationRow>,
    transport: Vec<OwnerTransportSpeedRow>,
    outcomes: [Outcome; N],
    /// DivisionStatus.combat_movement by division, ascending, when
    /// `combat_captured`.
    /// In census order; [`Self::combat_order`] finds a division.
    combat: Vec<(u64, Option<Option<i64>>)>,
    combat_order: Vec<(u64, u32)>,
    combat_captured: bool,
    combat_demand: Vec<(u64, Result<bool, &'static str>)>,
    movers: Vec<Mover>,
    route: Vec<Result<u32, &'static str>>,
    route_values: Vec<u32>,
    crossings: cprovince::GeometryPacket,
    /// Each unique province's movement facts, by id.
    provinces: Vec<(u32, Result<cprovince::MovementProvince, &'static str>)>,
    /// Each unique state's id and controller, by pointer.
    // Keep each pointer's id/controller result and refusal in the same memo entry.
    #[allow(clippy::type_complexity)]
    state_facts: Vec<(usize, Result<(u32, u32), &'static str>)>,
    /// Each unique template's movement speed keys in `keys`, by pointer.
    // The span into `keys` stays paired with its template pointer and refusal.
    #[allow(clippy::type_complexity)]
    templates: Vec<(usize, Result<(usize, usize), &'static str>)>,
    keys: Vec<u32>,
    observed_entries: owned::Entries,
    observed_rows: owned::Rows,
    /// Each unique combat's movement word and attacking side in `attackers`.
    combats: Vec<Result<CombatOperands, &'static str>>,
    combat_index: std::collections::HashMap<usize, usize>,
    hostility: Hostility,
    attackers: Vec<usize>,
    regions: Vec<TransportRegion>,
    /// Reused across hours; only qualifying sea/transfer edges append.
    transport_demand: Vec<(u32, usize)>,
    speed_cache: SpeedCache,
    held: owned::Loan,
    settle_pending: bool,
    changes: &'static dyn Changes,
    /// The hour's family counts: reads, carried, refusals.
    counts: [[u64; 3]; FAMILIES],
}

impl Default for SpeedTerms {
    fn default() -> Self {
        Self {
            states: Vec::new(),
            access: Vec::new(),
            entries: Vec::new(),
            rivers: Vec::new(),
            transport: Vec::new(),
            outcomes: [UNCAPTURED; N],
            combat: Vec::new(),
            combat_order: Vec::new(),
            combat_captured: false,
            combat_demand: Vec::new(),
            movers: Vec::new(),
            route: Vec::new(),
            route_values: Vec::new(),
            crossings: cprovince::GeometryPacket::default(),
            provinces: Vec::new(),
            state_facts: Vec::new(),
            templates: Vec::new(),
            keys: Vec::new(),
            observed_entries: owned::Entries::default(),
            observed_rows: owned::Rows::default(),
            combats: Vec::new(),
            combat_index: std::collections::HashMap::new(),
            hostility: Hostility::default(),
            attackers: Vec::new(),
            regions: Vec::new(),
            transport_demand: Vec::new(),
            speed_cache: SpeedCache::default(),
            held: owned::Loan::default(),
            settle_pending: false,
            changes: &changes::Observer,
            counts: [[0; 3]; FAMILIES],
        }
    }
}

/// The movement facts of province `id`, read once in [`SpeedTerms::prepare`].
fn province(
    provinces: &[(u32, Result<cprovince::MovementProvince, &'static str>)],
    id: u32,
) -> Result<cprovince::MovementProvince, &'static str> {
    memo(provinces, id).ok_or("movement province unread")?
}

/// A mover's route provinces in the hour's route arena.
fn route<'a>(
    arena: &'a [Result<u32, &'static str>],
    mover: &Mover,
) -> Result<&'a [Result<u32, &'static str>], &'static str> {
    let (start, len) = mover.route?;
    arena
        .get(start..start + len)
        .ok_or("division route outside its arena")
}

/// A sorted memo's entry for `key`.
fn memo<K: Ord + Copy, V: Copy>(memo: &[(K, V)], key: K) -> Option<V> {
    memo.binary_search_by_key(&key, |entry| entry.0)
        .ok()
        .map(|at| memo[at].1)
}

fn reserve<T>(vec: &mut Vec<T>, additional: usize) -> Result<(), &'static str> {
    engine_allocation::reserve(vec, additional).map_err(|_| ALLOCATION)
}

impl SpeedTerms {
    pub(super) fn allocated_bytes(&self) -> usize {
        self.states.capacity() * size_of::<MovementStateTermRow>()
            + self.access.capacity() * size_of::<CoreSupplyAccessRow>()
            + self.entries.capacity() * size_of::<CountryUnitSpeedEntryRow>()
            + self.rivers.capacity() * size_of::<RiverRelationRow>()
            + self.transport.capacity() * size_of::<OwnerTransportSpeedRow>()
            + self.combat.capacity() * size_of::<(u64, Option<Option<i64>>)>()
            + self.combat_order.capacity() * size_of::<(u64, u32)>()
            + self.movers.capacity() * size_of::<Mover>()
            + self.combat_demand.capacity() * size_of::<(u64, Result<bool, &'static str>)>()
            + self.route.capacity() * size_of::<Result<u32, &'static str>>()
            + self.route_values.capacity() * size_of::<u32>()
            + self.crossings.allocated_bytes()
            + self.provinces.capacity()
                * size_of::<(u32, Result<cprovince::MovementProvince, &'static str>)>()
            + self.state_facts.capacity() * size_of::<(usize, Result<(u32, u32), &'static str>)>()
            + self.templates.capacity() * size_of::<(usize, Result<(usize, usize), &'static str>)>()
            + self.keys.capacity() * size_of::<u32>()
            + self.observed_entries.bytes()
            + self.observed_rows.bytes()
            + self.combats.capacity() * size_of::<Result<CombatOperands, &'static str>>()
            + self.combat_index.capacity() * (size_of::<(usize, usize)>() + 1)
            + self.hostility.allocated_bytes()
            + self.attackers.capacity() * size_of::<usize>()
            + self.regions.capacity() * size_of::<TransportRegion>()
            + self.transport_demand.capacity() * size_of::<(u32, usize)>()
            + self.speed_cache.allocated_bytes()
            + self.held.bytes()
    }

    /// Before the speed pass: nothing of a previous hour is published. The
    /// held entries stay.
    pub(super) fn reset(&mut self) {
        // A reused/dropped frame cannot carry unfinished owned settlement.
        if self.settle_pending {
            self.held.clear();
        }
        self.held.abandon();
        self.settle_pending = false;
        self.observed_entries.clear();
        self.observed_rows.clear();
        self.states.clear();
        self.access.clear();
        self.entries.clear();
        self.rivers.clear();
        self.transport.clear();
        self.outcomes = [UNCAPTURED; N];
        self.combat.clear();
        self.combat_order.clear();
        self.combat_captured = false;
    }

    /// A failed or stopped acquisition: every table refuses and nothing held
    /// certifies the next hour.
    pub(super) fn fail(&mut self, reason: &'static str) {
        self.reset();
        self.outcomes = [Outcome::Unavailable { reason }; N];
        self.held.clear();
        owned::forget();
    }

    /// The hour's DivisionStatus.combat_movement of `division`; the outer
    /// `None` is its refusal.
    pub(super) fn combat_movement(&self, division: u64) -> Option<Option<i64>> {
        if !self.combat_captured {
            return None;
        }
        let at = self
            .combat_order
            .binary_search_by_key(&division, |entry| entry.0)
            .ok()?;
        self.combat.get(self.combat_order[at].1 as usize)?.1
    }

    /// [`Self::combat_movement`] of the division at census `position`, found there without a
    /// search when it is that division's.
    pub(super) fn combat_movement_at(&self, position: usize, division: u64) -> Option<Option<i64>> {
        if !self.combat_captured {
            return None;
        }
        match self.combat.get(position) {
            Some(&(at, value)) if at == division => value,
            _ => self.combat_movement(division),
        }
    }

    /// Orders [`Self::combat`] for lookups by division, within the capacity it was captured with.
    fn index_combat(&mut self) {
        self.combat_order.clear();
        self.combat_order.extend(
            self.combat
                .iter()
                .enumerate()
                .map(|(at, entry)| (entry.0, at as u32)),
        );
        self.combat_order.sort_unstable_by_key(|entry| entry.0);
    }

    pub(super) fn outcomes(&self) -> [Outcome; N] {
        self.outcomes
    }

    pub(super) fn table_inputs(
        &self,
        mut projection: Option<&mut crate::projection::Projection<'_>>,
    ) -> [TableInput; N] {
        use crate::projection::scalar_table;
        use replay_data::rows::RowCodec;
        [
            scalar_table::<tables::MovementStateTerms, _>(
                projection
                    .as_deref_mut()
                    .filter(|_| matches!(self.outcomes[0], Outcome::Rows { .. })),
                self.outcomes[0],
                &self.states,
                RowCodec::key,
                |r| *r,
            ),
            scalar_table::<tables::CoreSupplyAccess, _>(
                projection
                    .as_deref_mut()
                    .filter(|_| matches!(self.outcomes[1], Outcome::Rows { .. })),
                self.outcomes[1],
                &self.access,
                RowCodec::key,
                |r| *r,
            ),
            scalar_table::<tables::CountryUnitSpeedEntries, _>(
                projection
                    .as_deref_mut()
                    .filter(|_| matches!(self.outcomes[2], Outcome::Rows { .. })),
                self.outcomes[2],
                &self.entries,
                RowCodec::key,
                |r| *r,
            ),
            scalar_table::<tables::RiverRelations, _>(
                projection
                    .as_deref_mut()
                    .filter(|_| matches!(self.outcomes[3], Outcome::Rows { .. })),
                self.outcomes[3],
                &self.rivers,
                RowCodec::key,
                |r| *r,
            ),
            scalar_table::<tables::DivisionTransportSpeeds, _>(
                projection.filter(|_| matches!(self.outcomes[4], Outcome::Rows { .. })),
                self.outcomes[4],
                &self.transport,
                RowCodec::key,
                |r| *r,
            ),
        ]
    }

    #[cfg(test)]
    pub(super) fn tables(&self) -> [TableView; N] {
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("projection without a baseline always samples"),
        })
    }

    #[cfg(test)]
    fn capture_holding<T>(
        &mut self,
        store: &Mutex<Option<Held>>,
        capture: impl FnOnce(&mut Self) -> T,
    ) -> T {
        self.held.replace(
            store
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
                .unwrap_or_default(),
        );
        let result = capture(self);
        *store.lock().unwrap_or_else(|p| p.into_inner()) = Some(std::mem::take(&mut *self.held));
        result
    }

    /// Every family's native-demand selection and reads, timed in TABLES order.
    /// The held baseline remains lent until FIFO decode completes publication.
    pub(super) fn acquire(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game: usize,
        rows: &[DivisionSeed],
        outcome: Outcome,
    ) -> [u64; N] {
        self.reset();
        match owned::Loan::take() {
            Ok(loan) => self.held = loan,
            Err(reason) => {
                self.outcomes = [Outcome::Unavailable { reason }; N];
                return [0; N];
            }
        }
        self.settle_pending = true;
        self.capture_current(memory, base, game, rows, outcome)
    }

    pub(super) fn decode(&mut self) -> u64 {
        if !std::mem::take(&mut self.settle_pending) {
            return 0;
        }
        let started = crate::sampling::qpc();
        if self.observed_entries.pending {
            self.outcomes[2] = self.settle_entries();
        }
        for slot in [0, 1, 3, 4] {
            self.settle_family(slot);
            if let Outcome::Unavailable { reason } = self.outcomes[slot] {
                self.finish(Err(reason), slot);
            }
        }
        self.index_combat();
        self.speed_cache.finish();
        self.held.return_complete();
        crate::sampling::qpc().saturating_sub(started).max(0) as u64
    }

    /// The common failed-frame return path also runs when there is no army
    /// store loan. Release this independent loan before the next acquisition.
    pub(super) fn discard_owned(&mut self) {
        self.held.abandon();
        self.settle_pending = false;
        self.observed_entries.clear();
        self.observed_rows.clear();
    }

    fn capture_current(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game: usize,
        rows: &[DivisionSeed],
        outcome: Outcome,
    ) -> [u64; N] {
        use crate::sampling::qpc;
        let mut ticks = [0_u64; N];
        self.counts = [[0; 3]; FAMILIES];
        self.combat.clear();
        self.combat_order.clear();
        self.combat_captured = false;
        let census = match outcome {
            Outcome::Rows { len } => rows.get(..len).ok_or("division census outside its rows"),
            Outcome::Truncated { .. } => Err("division census truncated"),
            Outcome::Unavailable { reason } => Err(reason),
        };
        // The unit speed set's marks are taken every hour, so they never
        // accumulate across an hour that reads nothing.
        let take = self.take();
        let seeds = match census {
            Ok(seeds) => seeds,
            Err(reason) => {
                self.outcomes = [Outcome::Unavailable { reason }; N];
                self.held.clear();
                return ticks;
            }
        };
        let start = qpc();
        let prepared = {
            let _reads = crate::read_accounting::table(TABLES[0]);
            self.prepare(memory, base, game, seeds)
        };
        if let Err(reason) = prepared {
            self.outcomes = [Outcome::Unavailable { reason }; N];
            self.held.clear();
            return ticks;
        }
        {
            let _reads = crate::read_accounting::table(TableId::DivisionStatus);
            self.combat_captured = self.capture_combat(memory, base, seeds).is_ok();
            if !self.combat_captured {
                self.combat.clear();
        self.combat_order.clear();
                self.counts[COMBAT] = [0, 0, seeds.len() as u64];
            }
        }
        let countries = ccountry::Reader::new(memory, base, game);
        let mut lap = |slot: usize, at: &mut i64| {
            let now = qpc();
            ticks[slot] += now.saturating_sub(*at).max(0) as u64;
            *at = now;
        };
        let mut at = start;
        self.outcomes[0] = {
            let _reads = crate::read_accounting::table(TABLES[0]);
            self.capture_states(memory, base, game)
        };
        lap(0, &mut at);
        self.outcomes[1] = {
            let _reads = crate::read_accounting::table(TABLES[1]);
            self.capture_access(memory, base, game)
        };
        lap(1, &mut at);
        self.outcomes[2] = {
            let _reads = crate::read_accounting::table(TABLES[2]);
            self.acquire_entries(memory, &countries, take)
        };
        lap(2, &mut at);
        self.outcomes[3] = {
            let _reads = crate::read_accounting::table(TABLES[3]);
            self.capture_rivers(memory, base, game, &countries)
        };
        lap(3, &mut at);
        self.outcomes[4] = {
            let _reads = crate::read_accounting::table(TABLES[4]);
            self.capture_transport(memory, base, game)
        };
        lap(4, &mut at);
        // Region pointers never outlive the acquisition.
        self.regions.clear();
        ticks
    }

    /// The unit speed set's hourly take: its epoch, whether every mark was
    /// recorded, and the marked native ids, ascending.
    fn take(&mut self) -> Take {
        let mut marked = Vec::new();
        let mut complete = true;
        let epoch = self
            .changes
            .take(Table::UnitSpeeds, &mut |id| match u32::try_from(id) {
                Ok(id) if engine_allocation::reserve(&mut marked, 1).is_ok() => marked.push(id),
                _ => complete = false,
            });
        marked.sort_unstable();
        marked.dedup();
        Take {
            epoch,
            complete,
            marked,
        }
    }

    /// DivisionStatus.combat_movement for movers with any ordinary land edge.
    /// Stationary and early-return speed branches publish known absence without
    /// reading combat lists. Each demanded combat is read once.
    fn capture_combat(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
    ) -> Result<(), &'static str> {
        // The combat list of a division the field reads: `Ok(None)` when it
        // is not in combat or is withdrawing.
        let list = |seed: &DivisionSeed| -> Result<Option<Array>, &'static str> {
            let demand = memo(&self.combat_demand, seed.division).unwrap_or(Ok(false));
            if demand == Ok(false) || seed.withdrawal.is_some_and(|value| value != 0) {
                return Ok(None);
            }
            let list = seed
                .combat
                .and_then(|header| Array::from_header(header, 8))
                .ok_or("division combat list unreadable")?;
            if list.count == 0 {
                return Ok(None);
            }
            match seed.withdrawal.ok_or("division withdrawal unreadable")? {
                0 => demand.map(|needed| needed.then_some(list)),
                _ => Ok(None),
            }
        };
        let combat_at = |list: Array, index: u32| {
            list.element(index, 8)
                .and_then(|at| engine_native::read::<u64>(memory, at))
                .map(|pointer| pointer as usize)
                .ok_or("division combat reference unreadable")
        };
        self.combats.clear();
        self.combat_index.clear();
        self.attackers.clear();
        reserve(&mut self.combat, seeds.len())?;
        reserve(&mut self.combat_order, seeds.len())?;
        for seed in seeds {
            let value = (|| -> Result<Option<i64>, &'static str> {
                let Some(list) = list(seed)? else {
                    return Ok(None);
                };
                for index in 0..list.count {
                    let combat = combat_at(list, index)?;
                    let at = if let Some(&at) = self.combat_index.get(&combat) {
                        at
                    } else {
                        reserve(&mut self.combats, 1)?;
                        engine_allocation::reserve_map(&mut self.combat_index, 1)
                            .map_err(|_| ALLOCATION)?;
                        let start = self.attackers.len();
                        let side = engine_native::cland_combat::movement_combat_side(
                            memory,
                            base,
                            combat,
                            &mut self.attackers,
                        )
                        .map(|side| CombatOperands {
                            land: side.land,
                            complete: side.complete,
                            start,
                            len: self.attackers.len() - start,
                            word: None,
                        });
                        let at = self.combats.len();
                        self.combats.push(side);
                        self.combat_index.insert(combat, at);
                        at
                    };
                    let side = self.combats[at]?;
                    if self.attackers[side.start..side.start + side.len].contains(&seed.pointer) {
                        let word = side.word.unwrap_or_else(|| {
                            engine_native::cland_combat::movement_combat_word(
                                memory, combat, side.land,
                            )
                        });
                        self.combats[at] = Ok(CombatOperands {
                            word: Some(word),
                            ..side
                        });
                        return word;
                    }
                    side.complete?;
                }
                Ok(None)
            })();
            if memo(&self.combat_demand, seed.division).unwrap_or(Ok(false)) != Ok(false) {
                self.counts[COMBAT][usize::from(value.is_err()) * 2] += 1;
            }
            self.combat.push((seed.division, value.ok()));
        }
        if !self.settle_pending {
            self.index_combat();
        }
        Ok(())
    }

    /// The movers' keys, their routes and every province, state and template
    /// fact the families share.
    fn prepare(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game: usize,
        seeds: &[DivisionSeed],
    ) -> Result<(), &'static str> {
        self.movers.clear();
        self.route.clear();
        self.provinces.clear();
        self.state_facts.clear();
        self.templates.clear();
        self.keys.clear();
        self.hostility.clear();
        for seed in seeds {
            let route = match seed
                .ordinary_route
                .and_then(|header| Array::from_header(header, 4))
            {
                Some(route) if route.count == 0 => continue,
                Some(route) => copy_route(memory, route, &mut self.route, &mut self.route_values),
                None => Err("division route unreadable"),
            };
            // Missing roster fields do not suppress logical-owner transports.
            let roster = seed.roster_owner;
            let owner = seed.expeditionary_owner.and_then(|expeditionary| {
                if expeditionary as i32 > 0 {
                    Some(expeditionary)
                } else {
                    roster
                }
            });
            let location = match seed.location {
                None => Err("division location unreadable"),
                Some(0) => Err("division has no location"),
                // The census gathered the mover's location id once for every consumer.
                Some(location) => match seed.location_id {
                    Some(id) => id,
                    None => location
                        .checked_add(cprovince::ID)
                        .and_then(|address| engine_native::read::<u32>(memory, address)),
                }
                .ok_or("division location id unreadable"),
            };
            reserve(&mut self.movers, 1)?;
            self.movers.push(Mover {
                division: seed.division,
                attacking: Err("movement hostility not demanded"),
                land: Ok(false),
                strategic: seed.strategic_redeployment,
                roster,
                owner,
                logical: seed.logical_owner,
                location,
                template: seed.standard_template,
                noncore: noncore_demand(seed.exiled, seed.noncore_supply_bypass),
                transfer: seed
                    .naval_transfer_membership
                    .map(|membership| membership != 0),
                route,
            });
        }
        // Each unique province of the movers' locations and routes, once.
        let mut ids: Vec<u32> = Vec::new();
        reserve(&mut ids, self.movers.len() + self.route.len())?;
        ids.extend(self.movers.iter().filter_map(|mover| mover.location.ok()));
        ids.extend(self.route.iter().filter_map(|id| id.ok()));
        ids.sort_unstable();
        ids.dedup();
        reserve(&mut self.provinces, ids.len())?;
        let provinces = game
            .checked_add(engine_native::cgame_state::PROVINCES)
            .and_then(|address| Array::read(memory, address, 8))
            .ok_or("movement province array invalid");
        match provinces {
            Ok(provinces) => {
                cprovince::movement_provinces(memory, base, provinces, &ids, |id, facts| {
                    self.provinces.push((id, facts))
                })
            }
            Err(reason) => self
                .provinces
                .extend(ids.iter().map(|&id| (id, Err(reason)))),
        }
        self.prepare_land_demand(memory, base, game)?;
        // Only location states whose speed operands will be read.
        let mut states: Vec<usize> = Vec::new();
        reserve(&mut states, self.movers.len())?;
        states.extend(
            self.movers
                .iter()
                .filter(|mover| {
                    mover.land == Ok(true) && mover.roster.is_some() && mover.owner.is_some()
                })
                .filter_map(|mover| mover.location.ok())
                .filter_map(|id| province(&self.provinces, id).ok())
                .filter_map(|facts| facts.state.ok())
                .filter(|&state| state != 0),
        );
        states.sort_unstable();
        states.dedup();
        reserve(&mut self.state_facts, states.len())?;
        for &state in &states {
            self.state_facts.push((
                state,
                engine_native::cstate::id_and_controller(memory, state),
            ));
        }
        // Each unique template's speed keys, once.
        let mut templates: Vec<usize> = Vec::new();
        reserve(&mut templates, self.movers.len())?;
        templates.extend(
            self.movers
                .iter()
                .filter(|mover| mover.land == Ok(true) && mover.owner.is_some())
                .filter_map(|mover| mover.template),
        );
        templates.sort_unstable();
        templates.dedup();
        reserve(&mut self.templates, templates.len())?;
        for &template in &templates {
            let start = self.keys.len();
            if engine_native::carmy::movement_speed_key_demands(
                memory,
                base,
                template,
                &mut self.keys,
            )
            .is_err()
            {
                self.counts[2][2] += 1;
            }
            self.templates
                .push((template, Ok((start, self.keys.len() - start))));
        }
        Ok(())
    }

    /// Demand follows the itinerary, not just the first speed observation.
    /// Once one readable ordinary edge establishes demand, later edges cannot
    /// revoke it. Strategic redeployment retains its native early return.
    fn prepare_land_demand(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game: usize,
    ) -> Result<(), &'static str> {
        // Branch before demanding land families; sea/invasion and transfer
        // itineraries never read their state, category or supply operands.
        let countries = ccountry::Reader::new(memory, base, game);
        for mover in &mut self.movers {
            mover.land = (|| {
                if mover.transfer == Some(true) {
                    return Ok(false);
                }
                let path = route(&self.route, mover)?;
                let location = province(&self.provinces, mover.location?)?;
                let (normal, refused) =
                    engine_native::carmy::organisation_movement::land_speed_demand(
                        mover
                            .location
                            .and_then(|id| location.terrain_flags.map(|flags| (id, flags))),
                        path.iter().map(|id| {
                            id.and_then(|id| {
                                province(&self.provinces, id)
                                    .and_then(|p| p.terrain_flags.map(|flags| (id, flags)))
                            })
                        }),
                    );
                for family in &mut self.counts[..3] {
                    family[2] += refused.saturating_sub(u64::from(normal.is_err()));
                }
                if !normal? {
                    return Ok(false);
                }
                mover.transfer.ok_or("division naval transfer unreadable")?;
                if mover.strategic.ok_or("division redeployment unreadable")? != 0 {
                    let owner = countries
                        .as_ref()
                        .map_err(|reason| *reason)?
                        .resolve(mover.owner.ok_or("division speed owner unreadable")?)?;
                    let inputs = engine_native::carmy::strategic_redeployment_inputs(
                        memory,
                        base,
                        game,
                        mover.owner.ok_or("division speed owner unreadable")?,
                        owner.pointer,
                        || mover.roster.ok_or("division roster unreadable"),
                        || location.state,
                    )
                    .map_err(|failure| failure.reason())?;
                    if !inputs.blocked() {
                        return Ok(false);
                    }
                }
                Ok(true)
            })();
            if mover.land == Ok(true) {
                let last = route(&self.route, mover)
                    .and_then(|path| path.last().copied().unwrap_or(Err("division route empty")));
                mover.attacking = self
                    .hostility
                    .attacking(memory, base, game, last, mover.logical);
            }
            if mover.land.is_err() {
                // Charge the failed gate in this existing traversal. River
                // acquisition separately owns failures of reached edges.
                for family in &mut self.counts[..4] {
                    family[2] += 1;
                }
            }
        }
        self.combat_demand.clear();
        reserve(&mut self.combat_demand, self.movers.len())?;
        for mover in &self.movers {
            // ETA evaluates every edge: an embarkation can reach an ordinary
            // land edge later, where the current combat operand still applies.
            self.combat_demand.push((mover.division, mover.land));
        }
        self.combat_demand.sort_unstable_by_key(|entry| entry.0);
        Ok(())
    }

    /// MovementStateTerms: one row per (state, roster, owner).
    // Local tuples retain demand keys, selected branches and their independent refusals.
    #[allow(clippy::type_complexity)]
    fn capture_states(&mut self, memory: &impl Memory, base: usize, game: usize) -> Outcome {
        let result = (|| -> Result<usize, &'static str> {
            // (state id, roster, owner, state pointer, state controller)
            let mut wanted: Vec<(u32, u32, u32, usize, u32, bool)> = Vec::new();
            reserve(&mut wanted, self.movers.len())?;
            for mover in self.movers.iter().filter(|mover| mover.land == Ok(true)) {
                let key =
                    (|| -> Result<Option<(u32, u32, u32, usize, u32, bool)>, &'static str> {
                        let province = province(&self.provinces, mover.location?)?;
                        let state = province.state?;
                        if state == 0 {
                            return Ok(None);
                        }
                        let (id, controller) =
                            memo(&self.state_facts, state).ok_or("movement state unread")??;
                        Ok(Some((
                            id,
                            mover.roster.ok_or("division roster unreadable")?,
                            mover.owner.ok_or("division speed owner unreadable")?,
                            state,
                            controller,
                            mover.attacking != Ok(false),
                        )))
                    })();
                match key {
                    Ok(Some(key)) => wanted.push(key),
                    Ok(None) => {}
                    Err(_) => self.counts[0][2] += 1,
                }
            }
            wanted.sort_unstable_by_key(|key| (key.0, key.1, key.2));
            wanted.dedup_by(|later, kept| {
                if (later.0, later.1, later.2) != (kept.0, kept.1, kept.2) {
                    return false;
                }
                kept.5 |= later.5;
                true
            });
            // Each (owner, controller) branch once.
            let mut branches: Vec<((u32, u32), Result<u8, &'static str>)> = Vec::new();
            reserve(&mut branches, wanted.len())?;
            branches.extend(
                wanted
                    .iter()
                    .map(|&(_, _, owner, _, controller, _)| ((owner, controller), Ok(0))),
            );
            branches.sort_unstable_by_key(|entry| entry.0);
            branches.dedup_by_key(|entry| entry.0);
            for ((owner, controller), branch) in &mut branches {
                *branch = (|| -> Result<u8, &'static str> {
                    Ok(
                        if ccountry::movement_friendly(memory, base, game, *owner, *controller)? {
                            StateMovementTerms::FRIENDLY
                        } else if ccountry::movement_enemy(memory, base, game, *owner, *controller)?
                        {
                            StateMovementTerms::ENEMY
                        } else {
                            StateMovementTerms::NEITHER
                        },
                    )
                })();
            }
            // Each selected block/branch is shared; unselected relation entries
            // are neither queried nor allowed to refuse this row.
            let mut block: Option<(
                (usize, u32, u8, bool),
                Result<owned::StateInputs, &'static str>,
            )> = None;
            reserve(&mut self.states, wanted.len())?;
            reserve(&mut self.observed_rows.states, wanted.len())?;
            for &(state_id, roster, owner, state, controller, attacking) in &wanted {
                let branch =
                    memo(&branches, (owner, controller)).ok_or("movement branch unread")?;
                let terms = branch.and_then(|branch| {
                    let key = (state, roster, branch, attacking);
                    if let Some((held, terms)) = block
                        && held == key
                    {
                        return terms;
                    }
                    let terms = (|| {
                        let at = engine_native::cstate::country_modifier_accumulator(
                            memory, game, state, roster,
                        )?;
                        let (selected, len) = state_keys(branch);
                        let army = accumulator::lookup_requested_at(memory, at, selected[0])?;
                        let relation = if len == 2 {
                            let value = accumulator::lookup_requested_at(memory, at, selected[1])?;
                            Some(value)
                        } else {
                            None
                        };
                        let attack = if attacking {
                            accumulator::lookup_requested_at(memory, at, STATE_KEYS[3])
                        } else {
                            Ok(Value::default())
                        };
                        Ok(owned::StateInputs {
                            army,
                            attack,
                            relation,
                            branch,
                        })
                    })();
                    block = Some((key, terms));
                    terms
                });
                self.observed_rows.states.push(((state_id, roster, owner), terms));
            }
            Ok(self.states.len() + self.observed_rows.states.len())
        })();
        if !self.settle_pending {
            self.settle_family(0);
        }
        self.finish(result, 0)
    }

    /// CoreSupplyAccess: one row per (owner, location province) of a mover
    /// whose noncore-supply term can apply.
    fn capture_access(&mut self, memory: &impl Memory, base: usize, game: usize) -> Outcome {
        let mut wanted = std::mem::take(&mut self.observed_rows.access);
        let mut areas = std::mem::take(&mut self.observed_rows.access_areas);
        wanted.clear();
        areas.clear();
        let mut ready = false;
        let result = (|| -> Result<usize, &'static str> {
            reserve(&mut wanted, self.movers.len())?;
            for mover in self.movers.iter().filter(|mover| mover.land == Ok(true)) {
                match (mover.noncore, mover.location) {
                    (Some(false), _) => {}
                    (Some(true), Ok(province)) => match mover.owner {
                        Some(owner) => wanted.push((owner, province)),
                        None => self.counts[1][2] += 1,
                    },
                    _ => self.counts[1][2] += 1,
                }
            }
            wanted.sort_unstable();
            wanted.dedup();
            // Several location provinces can share the same controller area.
            // Query that predicate once per owner/area, while retaining the
            // province key playback joins without a second area column.
            reserve(&mut areas, wanted.len())?;
            for &(owner, province) in &wanted {
                if let Ok(facts) = self::province(&self.provinces, province)
                    && let Ok(area) = facts.controller_area
                {
                    areas.push(((owner, area), Ok(false)));
                }
            }
            areas.sort_unstable_by_key(|entry| entry.0);
            areas.dedup_by_key(|entry| entry.0);
            for ((owner, area), access) in &mut areas {
                *access = ccountry::has_core_supply_access(memory, base, game, *owner, *area);
            }
            reserve(&mut self.access, wanted.len())?;
            ready = true;
            Ok(self.access.len() + wanted.len())
        })();
        self.observed_rows.access = wanted;
        self.observed_rows.access_areas = areas;
        self.observed_rows.access_ready = ready;
        if !self.settle_pending {
            self.settle_family(1);
        }
        self.finish(result, 1)
    }

    /// CountryUnitSpeedEntries: one row per (owner, key) over the movement
    /// speed keys of the movers' templates, carried under the unit speed set.
    #[cfg(test)]
    fn capture_entries(
        &mut self,
        memory: &impl Memory,
        countries: &Result<ccountry::Reader<'_, impl Memory>, &'static str>,
        take: Take,
    ) -> Outcome {
        self.acquire_entries(memory, countries, take);
        self.settle_entries()
    }

    fn acquire_entries(
        &mut self,
        memory: &impl Memory,
        countries: &Result<ccountry::Reader<'_, impl Memory>, &'static str>,
        take: Take,
    ) -> Outcome {
        let arrays = countries.as_ref().ok().map(|reader| reader.arrays());
        let certified = take.complete
            && take.epoch.is_some()
            && take.epoch == self.held.epoch
            && arrays == Some(self.held.arrays);
        let counts = Counts {
            marked: take.marked.len() as u64,
            full: !certified,
            ..Counts::default()
        };
        self.observed_entries.clear();
        let mut wanted = std::mem::take(&mut self.observed_entries.wanted);
        let mut values = std::mem::take(&mut self.observed_entries.values);
        let mut observed = std::mem::take(&mut self.observed_entries.countries);
        let result = (|| -> Result<usize, &'static str> {
            for mover in self.movers.iter().filter(|mover| mover.land == Ok(true)) {
                let Some(owner) = mover.owner else {
                    self.counts[2][2] += 1;
                    continue;
                };
                let keys = mover
                    .template
                    .ok_or("division template unreadable")
                    .and_then(|template| {
                        memo(&self.templates, template).ok_or("division template unread")?
                    });
                match keys {
                    Ok((start, len)) => {
                        reserve(&mut wanted, len)?;
                        wanted.extend(
                            self.keys[start..start + len]
                                .iter()
                                .map(|&key| (owner, key)),
                        );
                    }
                    Err(_) => self.counts[2][2] += 1,
                }
            }
            wanted.sort_unstable();
            wanted.dedup();
            reserve(&mut self.entries, wanted.len())?;
            reserve(&mut values, wanted.len())?;

            let mut first = 0;
            while first < wanted.len() {
                reserve(&mut observed, 1)?;
                let owner = wanted[first].0;
                let end = first + wanted[first..].partition_point(|&(known, _)| known == owner);
                let start = first;
                let group = &wanted[first..end];
                first = end;
                let country = countries
                    .as_ref()
                    .map_err(|reason| *reason)
                    .and_then(|reader| reader.resolve(owner));
                // The held read that certifies this country, when unmarked.
                let held = country.ok().filter(|_| certified).and_then(|country| {
                    let at = self
                        .held
                        .countries
                        .binary_search_by_key(&owner, |held| held.owner)
                        .ok()?;
                    let held = self.held.countries[at];
                    let keys = &self.held.keys[held.start..held.start + held.len];
                    (held.country == country
                        && take.marked.binary_search(&country.id).is_err()
                        && group.iter().all(|(_, key)| keys.binary_search(key).is_ok()))
                    .then_some(held)
                });
                let read = match held {
                    Some(_) => Ok(values.len()),
                    None => country.and_then(|country| {
                        let at = country
                            .pointer
                            .checked_add(ccountry::MODIFIER_ACCUMULATOR)
                            .ok_or("country accumulator address overflow")?;
                        let at_values = values.len();
                        values.extend(group.iter().map(|&(_, key)| {
                            accumulator::lookup_requested_at(memory, at, key)
                        }));
                        Ok(at_values)
                    }),
                };
                observed.push(owned::Country {
                    owner,
                    country,
                    held,
                    start,
                    end,
                    read,
                });
            }
            Ok(wanted.len())
        })();
        self.observed_entries.wanted = wanted;
        self.observed_entries.values = values;
        self.observed_entries.countries = observed;
        self.observed_entries.epoch = take.epoch;
        self.observed_entries.arrays = arrays;
        self.observed_entries.complete = take.complete;
        self.observed_entries.counts = counts;
        self.observed_entries.result = Some(result.map(|_| ()));
        self.observed_entries.pending = true;
        result.map_or_else(
            |reason| Outcome::Unavailable { reason },
            |len| Outcome::Rows { len },
        )
    }

    /// RiverRelations: one row per (owner, controller) over the movers' route
    /// provinces.
    fn capture_rivers(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game: usize,
        countries: &Result<ccountry::Reader<'_, impl Memory>, &'static str>,
    ) -> Outcome {
        if !self.movers.iter().any(|mover| mover.land == Ok(true)) {
            return self.finish(Ok(0), 3);
        }
        let mut crossings = std::mem::take(&mut self.crossings);
        let ready = crossings.begin(memory, base, game);
        let result = held_crossings(|held| {
            let world = crate::world_lifetime::generation();
            if held.world != Some(world) || world == u64::MAX {
                held.answers.clear();
                held.world = (world != u64::MAX).then_some(world);
            }
            let result = self.capture_rivers_with_crossings(memory, base, countries, |from, to| {
                if let Some(&answer) = held.answers.get(&(from, to)) {
                    return Ok(answer);
                }
                let answer = ready.and_then(|()| crossings.acquire_crossing(memory, from, to))?;
                if held.world.is_some()
                    && engine_allocation::advisory(|| {
                        engine_allocation::reserve_map(&mut held.answers, 1)
                    })
                    .is_ok()
                {
                    held.answers.insert((from, to), answer);
                }
                Ok(answer)
            });
            held.charge.set(crate::raw_memory::Capacity {
                index_vectors: (held.answers.capacity()
                    * (size_of::<((u32, u32), Option<u8>)>() + 1))
                    as u64,
                ..Default::default()
            });
            result
        });
        crossings.clear();
        self.crossings = crossings;
        result
    }

    fn capture_rivers_with_crossings(
        &mut self,
        memory: &impl Memory,
        base: usize,
        countries: &Result<ccountry::Reader<'_, impl Memory>, &'static str>,
        mut crossing: impl FnMut(u32, u32) -> Result<Option<u8>, &'static str>,
    ) -> Outcome {
        let result = (|| -> Result<usize, &'static str> {
            let mut wanted: Vec<(u32, u32)> = Vec::new();
            for mover in self.movers.iter().filter(|mover| mover.land == Ok(true)) {
                let Ok(route) = route(&self.route, mover) else {
                    self.counts[3][2] += 1;
                    continue;
                };
                let mut from_id = mover.location;
                for (index, &to_id) in route.iter().enumerate() {
                    let edge = (|| -> Result<Option<(u32, u32)>, &'static str> {
                        let from_id = from_id?;
                        let to_id = to_id?;
                        if index != 0 && from_id == to_id {
                            return Ok(None);
                        }
                        let from = province(&self.provinces, from_id)?;
                        let to = province(&self.provinces, to_id)?;
                        if !from.land()? || !to.land()? {
                            return Ok(None);
                        }
                        match crossing(from_id, to_id)? {
                            Some(1..=3) => Ok(Some((
                                mover.owner.ok_or("division speed owner unreadable")?,
                                to.controller?,
                            ))),
                            _ => Ok(None),
                        }
                    })();
                    match edge {
                        Ok(Some(key)) => {
                            reserve(&mut wanted, 1)?;
                            wanted.push(key);
                        }
                        Ok(None) => {}
                        Err(_) => self.counts[3][2] += 1,
                    }
                    from_id = to_id;
                }
            }
            wanted.sort_unstable();
            wanted.dedup();
            reserve(&mut self.rivers, wanted.len())?;
            reserve(&mut self.observed_rows.rivers, wanted.len())?;
            let mut owner: Option<(u32, Result<ccountry::Country, &'static str>)> = None;
            for &(raw, controller) in &wanted {
                let country = match owner {
                    Some((known, country)) if known == raw => country,
                    _ => {
                        let country = countries
                            .as_ref()
                            .map_err(|reason| *reason)
                            .and_then(|reader| reader.resolve(raw));
                        owner = Some((raw, country));
                        country
                    }
                };
                let entry = countries
                    .as_ref()
                    .map_err(|reason| *reason)
                    .and_then(|reader| {
                        ccountry::river_crossing_against_with(
                            memory, base, reader, country?, controller,
                        )
                    });
                self.observed_rows.rivers.push(((raw, controller), entry));
            }
            Ok(self.rivers.len() + self.observed_rows.rivers.len())
        })();
        if !self.settle_pending {
            self.settle_family(3);
        }
        self.finish(result, 3)
    }

    /// DivisionTransportSpeeds: for each logical owner of a mover with a naval
    /// transfer or an edge to or from the sea, the transport speed in each
    /// such edge's source strategic region.
    fn capture_transport(&mut self, memory: &impl Memory, base: usize, game: usize) -> Outcome {
        let result = (|| -> Result<usize, &'static str> {
            self.transport_demand.clear();
            for mover in &self.movers {
                let provinces = append_transport_demand(
                    &mut self.transport_demand,
                    &self.route,
                    &self.provinces,
                    mover,
                );
                match provinces {
                    Ok(refused) => self.counts[4][2] += refused,
                    Err(ALLOCATION) => return Err(ALLOCATION),
                    Err(_) => {
                        self.counts[4][2] += 1;
                    }
                }
            }
            let demand = &mut self.transport_demand;
            demand.sort_unstable();
            demand.dedup();
            // Each unique region's identity once; a province without a
            // readable region is skipped and counted.
            let mut pointers: Vec<usize> = Vec::new();
            reserve(&mut pointers, demand.len())?;
            pointers.extend(demand.iter().map(|&(_, region)| region));
            pointers.sort_unstable();
            pointers.dedup();
            self.regions.clear();
            reserve(&mut self.regions, pointers.len())?;
            let mut located: Vec<(usize, u32)> = Vec::new();
            reserve(&mut located, pointers.len())?;
            for &pointer in &pointers {
                match engine_native::cstrategic_region::id(memory, base, pointer) {
                    Ok(Some(id)) => {
                        self.regions.push(TransportRegion { id, pointer });
                        located.push((pointer, id));
                    }
                    Ok(None) | Err(_) => self.counts[4][2] += 1,
                }
            }
            self.regions.sort_unstable_by_key(|region| region.id);
            self.regions.dedup_by_key(|region| region.id);
            let mut wanted: Vec<(u32, usize)> = Vec::new();
            reserve(&mut wanted, demand.len())?;
            for &(owner, pointer) in demand.iter() {
                let Some(id) = memo(&located, pointer) else {
                    continue;
                };
                let index = self
                    .regions
                    .binary_search_by_key(&id, |region| region.id)
                    .map_err(|_| "division transport region unlocated")?;
                wanted.push((owner, index));
            }
            wanted.sort_unstable();
            wanted.dedup();
            reserve(&mut self.transport, wanted.len())?;
            reserve(&mut self.observed_rows.transport, wanted.len())?;
            if wanted.is_empty() {
                return Ok(0);
            }
            self.speed_cache.begin(self.regions.len())?;
            let mut indices: Vec<usize> = Vec::new();
            reserve(&mut indices, self.regions.len())?;
            let (transport, cache) = (
                &mut self.observed_rows.transport,
                &mut self.speed_cache,
            );
            let mut first = 0;
            while first < wanted.len() {
                let owner = wanted[first].0;
                let end = first + wanted[first..].partition_point(|&(known, _)| known == owner);
                indices.clear();
                indices.extend(wanted[first..end].iter().map(|&(_, index)| index));
                first = end;
                engine_naval_history::cnaval_unit_transfer::movement_speeds_inputs(
                    memory,
                    base,
                    game,
                    owner,
                    &self.regions,
                    &indices,
                    cache,
                    |region, speed| {
                        transport.push(((owner, region.id), speed));
                    },
                );
            }
            Ok(self.transport.len() + transport.len())
        })();
        if !self.settle_pending {
            self.settle_family(4);
        }
        self.finish(result, 4)
    }

    /// The table's outcome; a failed table publishes no row.
    fn finish(&mut self, result: Result<usize, &'static str>, slot: usize) -> Outcome {
        match result {
            Ok(len) => Outcome::Rows { len },
            Err(reason) => {
                match slot {
                    0 => self.states.clear(),
                    1 => self.access.clear(),
                    2 => self.entries.clear(),
                    3 => self.rivers.clear(),
                    _ => self.transport.clear(),
                }
                Outcome::Unavailable { reason }
            }
        }
    }
}

/// Preserve each readable edge's source demand, counting refused positions
/// separately. Transfers read only their source region; they do not need the
/// destination's flags or any roster-country fields.
fn append_transport_demand(
    demand: &mut Vec<(u32, usize)>,
    arena: &[Result<u32, &'static str>],
    provinces: &[(u32, Result<cprovince::MovementProvince, &'static str>)],
    mover: &Mover,
) -> Result<u64, &'static str> {
    let path = match route(arena, mover) {
        Ok(path) => path,
        Err(_) => return Ok(1),
    };
    let mut from_id = mover.location;
    let mut refused = 0;
    for (index, &to_id) in path.iter().enumerate() {
        let from = from_id.and_then(|id| province(provinces, id));
        let to = to_id.and_then(|id| province(provinces, id));
        let edge = (|| -> Result<Option<(u32, usize)>, &'static str> {
            if index != 0 && from_id.is_ok() && from_id == to_id {
                return Ok(None);
            }
            let from = from?;
            if mover.transfer == Some(true) || !from.land()? || !to?.land()? {
                Ok(Some((
                    mover.logical.ok_or("division logical owner unreadable")?,
                    from.strategic_region?,
                )))
            } else {
                mover.transfer.ok_or("division naval transfer unreadable")?;
                Ok(None)
            }
        })();
        match edge {
            Ok(Some(key)) => {
                reserve(demand, 1)?;
                demand.push(key);
            }
            Ok(None) => {}
            Err(_) => refused += 1,
        }
        // A later bad position is a diagnostic refusal, never rollback.
        if to.is_err() && edge.is_ok() {
            refused += 1;
        }
        from_id = to_id;
    }
    Ok(refused)
}

fn noncore_demand(exiled: Option<u8>, bypass: Option<u8>) -> Option<bool> {
    if exiled.is_some_and(|value| value != 0) || bypass.is_some_and(|value| value != 0) {
        Some(false)
    } else {
        exiled.zip(bypass).map(|_| true)
    }
}

/// Bulk copy on healthy routes. A failed bulk read falls back to individual
/// positions without erasing readable prefixes, suffixes or other movers.
fn copy_route(
    memory: &impl Memory,
    path: Array,
    arena: &mut Vec<Result<u32, &'static str>>,
    values: &mut Vec<u32>,
) -> Result<(usize, usize), &'static str> {
    let start = arena.len();
    let len = path.count as usize;
    reserve(arena, len)?;
    values.clear();
    reserve(values, len)?;
    values.resize(len, 0);
    if engine_native::cpdx_array::copy_u32s(memory, path, values, "division route unreadable")
        .is_ok()
    {
        arena.extend(values.iter().copied().map(Ok));
    } else {
        for index in 0..path.count {
            arena.push(
                path.element(index, 4)
                    .and_then(|at| engine_native::read::<u32>(memory, at))
                    .ok_or("division route position unreadable"),
            );
        }
    }
    Ok((start, len))
}

/// The unit speed set's hourly take.
struct Take {
    epoch: Option<u64>,
    complete: bool,
    marked: Vec<u32>,
}

#[cfg(test)]
mod tests;
