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

use collector_observers::{
    country_input_changes as changes,
    country_input_changes::{Changes, Counts, Table},
};

mod owned;

pub const N: usize = 5;
pub const TABLES: [TableId; N] = [
    TableId::MovementStateTerms,
    TableId::CoreSupplyAccess,
    TableId::CountryUnitSpeedEntries,
    TableId::RiverRelations,
    TableId::DivisionTransportSpeeds,
];

const FAMILIES: usize = N + 1;
const COMBAT: usize = N;

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
const UNFETCHED_CONTROLLER: &str = "movement province controller not fetched";
const UNFETCHED_AREA: &str = "movement province area not fetched";
const DENSE_PROVINCE_LIMIT: usize = 32_768;

#[derive(Clone, Copy)]
struct Mover {
    division: u64,
    attacking: Result<bool, &'static str>,
    land: Result<bool, &'static str>,
    strategic: Option<u8>,
    roster: Option<u32>,
    owner: Option<u32>,
    logical: Option<u32>,
    location: Result<u32, &'static str>,
    template: Option<usize>,
    noncore: Option<bool>,
    transfer: Option<bool>,
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

fn state_keys(branch: u8) -> ([u32; 2], usize) {
    match branch {
        StateMovementTerms::FRIENDLY => ([STATE_KEYS[0], STATE_KEYS[1]], 2),
        StateMovementTerms::ENEMY => ([STATE_KEYS[0], STATE_KEYS[2]], 2),
        _ => ([STATE_KEYS[0], 0], 1),
    }
}

#[derive(Clone, Copy)]
struct HeldCountry {
    owner: u32,
    country: ccountry::Country,
    start: usize,
    len: usize,
}

#[derive(Default)]
struct Held {
    epoch: Option<u64>,
    arrays: [Array; 2],
    countries: Vec<HeldCountry>,
    keys: Vec<u32>,
    values: Vec<Option<i64>>,
    template_token: Option<((u64, u64), (u64, u64), usize)>,
    template_spans: Vec<(usize, usize, usize)>,
    template_keys: Vec<u32>,
}

impl Held {
    fn clear(&mut self) {
        self.epoch = None;
        self.countries.clear();
        self.keys.clear();
        self.values.clear();
        self.template_token = None;
        self.template_spans.clear();
        self.template_keys.clear();
    }

    fn bytes(&self) -> usize {
        self.countries.capacity() * size_of::<HeldCountry>()
            + self.keys.capacity() * size_of::<u32>()
            + self.values.capacity() * size_of::<Option<i64>>()
            + self.template_spans.capacity() * size_of::<(usize, usize, usize)>()
            + self.template_keys.capacity() * size_of::<u32>()
    }
}

#[derive(Default, Clone)]
struct FastBuildHasher;

struct FastHasher(u64);

impl std::hash::BuildHasher for FastBuildHasher {
    type Hasher = FastHasher;
    fn build_hasher(&self) -> Self::Hasher {
        FastHasher(0)
    }
}

impl std::hash::Hasher for FastHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0.rotate_left(5) ^ u64::from(byte)).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
    }
    fn write_u32(&mut self, value: u32) {
        self.0 = (self.0.rotate_left(5) ^ u64::from(value)).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    fn write_usize(&mut self, value: usize) {
        self.0 = (self.0.rotate_left(5) ^ value as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

type FastMap<K, V> = std::collections::HashMap<K, V, FastBuildHasher>;

#[derive(Default)]
struct HeldCrossings {
    world: Option<u64>,
    answers: FastMap<(u32, u32), Option<u8>>,
    charge: collector_primitives::raw_memory::Charge,
}
static HELD_CROSSINGS: Mutex<Option<HeldCrossings>> = Mutex::new(None);

fn held_crossings<R>(
    cache: &Mutex<Option<HeldCrossings>>,
    visit: impl FnOnce(&mut HeldCrossings) -> R,
) -> R {
    visit(
        cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_or_insert_with(HeldCrossings::default),
    )
}

#[derive(Default)]
struct HeldMap {
    provinces_key: Option<(u64, usize, Array)>,
    provinces: Vec<Option<cprovince::StaticMovementProvince>>,
    regions_key: Option<(u64, usize)>,
    regions: FastMap<usize, u32>,
    charge: collector_primitives::raw_memory::Charge,
}
#[cfg(not(test))]
static HELD_MAP: Mutex<Option<HeldMap>> = Mutex::new(None);
#[cfg(test)]
thread_local! {
    static HELD_MAP: Mutex<Option<HeldMap>> = const { Mutex::new(None) };
}

fn held_map<R>(visit: impl FnOnce(&mut HeldMap) -> R) -> R {
    let with = |slot: &Mutex<Option<HeldMap>>| {
        visit(
            slot.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get_or_insert_with(HeldMap::default),
        )
    };
    #[cfg(not(test))]
    return with(&HELD_MAP);
    #[cfg(test)]
    HELD_MAP.with(with)
}

impl HeldMap {
    fn update_charge(&mut self) {
        self.charge.set(collector_primitives::raw_memory::Capacity {
            index_vectors: (self.provinces.capacity()
                * size_of::<Option<cprovince::StaticMovementProvince>>()
                + self.regions.capacity() * (size_of::<(usize, u32)>() + 1))
                as u64,
            ..Default::default()
        });
    }
}

#[derive(Default)]
struct Hostility {
    provinces: Option<Array>,
    destinations: FastMap<u32, Option<i32>>,
    pairs: FastMap<(u32, u32), bool>,
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

    #[cfg(test)]
    fn attacking(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game: usize,
        last: Result<u32, &'static str>,
        logical: Option<u32>,
    ) -> Result<bool, &'static str> {
        let mut countries = None;
        self.attacking_with(memory, base, game, &mut countries, &[], &[], last, logical)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "reuses already-fetched movement provinces and country reader to avoid re-reading destination province memory"
    )]
    fn attacking_with<'a, M: Memory>(
        &mut self,
        memory: &'a M,
        base: usize,
        game: usize,
        countries: &mut Option<Result<ccountry::Reader<'a, M>, &'static str>>,
        provinces: &[(u32, Result<cprovince::MovementProvince, &'static str>)],
        province_slot: &[u32],
        last: Result<u32, &'static str>,
        logical: Option<u32>,
    ) -> Result<bool, &'static str> {
        let last = last?;
        let destination = if let Ok(facts) = province_at(provinces, province_slot, last)
            && let Ok(controller) = facts.controller
        {
            Some(controller as i32)
        } else {
            match self.destinations.get(&last) {
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
        let reader = countries
            .get_or_insert_with(|| ccountry::Reader::new(memory, base, game))
            .as_ref()
            .map_err(|reason| *reason)?;
        let hostile = ccountry::movement_enemy_with(memory, base, reader, pair.0, pair.1)?;
        if engine_allocation::advisory(|| engine_allocation::reserve_map(&mut self.pairs, 1))
            .is_ok()
        {
            self.pairs.insert(pair, hostile);
        }
        Ok(hostile)
    }
}

#[derive(Default)]
struct Scratch {
    ids: Vec<u32>,
    cached_pointers: Vec<usize>,
    ctrl_slots: Vec<usize>,
    ctrl_results: Vec<Result<u32, &'static str>>,
    area_slots: Vec<usize>,
    area_results: Vec<Result<usize, &'static str>>,
    missing_ids: Vec<(usize, u32)>,
    missing_query: Vec<u32>,
    states: Vec<usize>,
    templates: Vec<usize>,
    #[allow(
        clippy::type_complexity,
        reason = "reused per-hour scratch for deduplicated state movement term keys"
    )]
    wanted_states: Vec<(u32, u32, u32, usize, u32, bool)>,
    #[allow(
        clippy::type_complexity,
        reason = "reused per-hour scratch for owner-controller movement relation branches"
    )]
    branches: Vec<((u32, u32), Result<u8, &'static str>)>,
    wanted_rivers: Vec<(u32, u32)>,
    pending_rivers: Vec<(u32, usize)>,
    pointers: Vec<usize>,
    located: Vec<(usize, u32)>,
    wanted_transport: Vec<(u32, usize)>,
    indices: Vec<usize>,
}

impl Scratch {
    fn allocated_bytes(&self) -> usize {
        self.ids.capacity() * size_of::<u32>()
            + self.cached_pointers.capacity() * size_of::<usize>()
            + self.ctrl_slots.capacity() * size_of::<usize>()
            + self.ctrl_results.capacity() * size_of::<Result<u32, &'static str>>()
            + self.area_slots.capacity() * size_of::<usize>()
            + self.area_results.capacity() * size_of::<Result<usize, &'static str>>()
            + self.missing_ids.capacity() * size_of::<(usize, u32)>()
            + self.missing_query.capacity() * size_of::<u32>()
            + self.states.capacity() * size_of::<usize>()
            + self.templates.capacity() * size_of::<usize>()
            + self.wanted_states.capacity() * size_of::<(u32, u32, u32, usize, u32, bool)>()
            + self.branches.capacity() * size_of::<((u32, u32), Result<u8, &'static str>)>()
            + self.wanted_rivers.capacity() * size_of::<(u32, u32)>()
            + self.pending_rivers.capacity() * size_of::<(u32, usize)>()
            + self.pointers.capacity() * size_of::<usize>()
            + self.located.capacity() * size_of::<(usize, u32)>()
            + self.wanted_transport.capacity() * size_of::<(u32, usize)>()
            + self.indices.capacity() * size_of::<usize>()
    }
}

pub struct SpeedTerms {
    states: Vec<MovementStateTermRow>,
    access: Vec<CoreSupplyAccessRow>,
    entries: Vec<CountryUnitSpeedEntryRow>,
    rivers: Vec<RiverRelationRow>,
    transport: Vec<OwnerTransportSpeedRow>,
    outcomes: [Outcome; N],
    combat: Vec<(u64, Option<Option<i64>>)>,
    combat_order: Vec<(u64, u32)>,
    combat_captured: bool,
    combat_demand: Vec<(u64, Result<bool, &'static str>)>,
    movers: Vec<Mover>,
    route: Vec<Result<u32, &'static str>>,
    route_values: Vec<u32>,
    crossings: cprovince::GeometryPacket,
    provinces: Vec<(u32, Result<cprovince::MovementProvince, &'static str>)>,
    province_slot: Vec<u32>,
    province_pointers: Vec<usize>,
    #[allow(clippy::type_complexity)]
    state_facts: Vec<(usize, Result<(u32, u32), &'static str>)>,
    #[allow(clippy::type_complexity)]
    templates: Vec<(usize, Result<(usize, usize), &'static str>)>,
    keys: Vec<u32>,
    observed_entries: owned::Entries,
    observed_rows: owned::Rows,
    combats: Vec<Result<CombatOperands, &'static str>>,
    combat_index: FastMap<usize, usize>,
    hostility: Hostility,
    attackers: Vec<usize>,
    regions: Vec<TransportRegion>,
    transport_demand: Vec<(u32, usize)>,
    speed_cache: SpeedCache,
    scratch: Scratch,
    held: owned::Loan,
    settle_pending: bool,
    changes: &'static dyn Changes,
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
            province_slot: Vec::new(),
            province_pointers: Vec::new(),
            state_facts: Vec::new(),
            templates: Vec::new(),
            keys: Vec::new(),
            observed_entries: owned::Entries::default(),
            observed_rows: owned::Rows::default(),
            combats: Vec::new(),
            combat_index: FastMap::default(),
            hostility: Hostility::default(),
            attackers: Vec::new(),
            regions: Vec::new(),
            transport_demand: Vec::new(),
            speed_cache: SpeedCache::default(),
            scratch: Scratch::default(),
            held: owned::Loan::default(),
            settle_pending: false,
            changes: &changes::Observer,
            counts: [[0; 3]; FAMILIES],
        }
    }
}

#[inline]
fn province_slot_of(
    provinces: &[(u32, Result<cprovince::MovementProvince, &'static str>)],
    province_slot: &[u32],
    id: u32,
) -> Option<usize> {
    let idx = id as usize;
    if idx < province_slot.len() {
        let slot = province_slot[idx];
        return (slot < provinces.len() as u32).then_some(slot as usize);
    }
    provinces.binary_search_by_key(&id, |entry| entry.0).ok()
}

#[inline]
fn province_at(
    provinces: &[(u32, Result<cprovince::MovementProvince, &'static str>)],
    province_slot: &[u32],
    id: u32,
) -> Result<cprovince::MovementProvince, &'static str> {
    let at = province_slot_of(provinces, province_slot, id).ok_or("movement province unread")?;
    provinces[at].1
}

#[inline]
fn is_unfetched(result: &Result<u32, &'static str>) -> bool {
    matches!(result, Err(reason) if std::ptr::eq(*reason, UNFETCHED_CONTROLLER))
}

#[inline]
fn is_unfetched_area(result: &Result<usize, &'static str>) -> bool {
    matches!(result, Err(reason) if std::ptr::eq(*reason, UNFETCHED_AREA))
}

fn route<'a>(
    arena: &'a [Result<u32, &'static str>],
    mover: &Mover,
) -> Result<&'a [Result<u32, &'static str>], &'static str> {
    let (start, len) = mover.route?;
    arena
        .get(start..start + len)
        .ok_or("division route outside its arena")
}

fn memo<K: Ord + Copy, V: Copy>(memo: &[(K, V)], key: K) -> Option<V> {
    memo.binary_search_by_key(&key, |entry| entry.0)
        .ok()
        .map(|at| memo[at].1)
}

#[inline]
fn reserve<T>(vec: &mut Vec<T>, additional: usize) -> Result<(), &'static str> {
    if vec.capacity() - vec.len() < additional {
        engine_allocation::reserve(vec, additional).map_err(|_| ALLOCATION)?;
    }
    Ok(())
}

impl SpeedTerms {
    pub fn allocated_bytes(&self) -> usize {
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
            + self.province_slot.capacity() * size_of::<u32>()
            + self.province_pointers.capacity() * size_of::<usize>()
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
            + self.scratch.allocated_bytes()
            + self.held.bytes()
    }

    pub(super) fn reset(&mut self) {
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

    pub(super) fn fail(&mut self, reason: &'static str) {
        self.reset();
        self.outcomes = [Outcome::Unavailable { reason }; N];
        self.held.clear();
        owned::forget();
    }

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

    pub(super) fn combat_movement_at(&self, position: usize, division: u64) -> Option<Option<i64>> {
        if !self.combat_captured {
            return None;
        }
        match self.combat.get(position) {
            Some(&(at, value)) if at == division => value,
            _ => self.combat_movement(division),
        }
    }

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
        mut projection: Option<&mut collector_projection::projection::Projection<'_>>,
    ) -> [TableInput; N] {
        use collector_projection::projection::scalar_table;
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

    pub(super) fn acquire_with(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game: usize,
        rows: &[DivisionSeed],
        outcome: Outcome,
        parallel: Option<ParallelRunner<'_>>,
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
        self.capture_current_with(memory, base, game, rows, outcome, parallel)
    }

    pub(super) fn decode(&mut self) -> u64 {
        if !std::mem::take(&mut self.settle_pending) {
            return 0;
        }
        let started = collector_primitives::clock::qpc();
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
        collector_primitives::clock::qpc()
            .saturating_sub(started)
            .max(0) as u64
    }

    pub(super) fn discard_owned(&mut self) {
        self.held.abandon();
        self.settle_pending = false;
        self.observed_entries.clear();
        self.observed_rows.clear();
    }

    fn capture_current_with(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game: usize,
        rows: &[DivisionSeed],
        outcome: Outcome,
        parallel: Option<ParallelRunner<'_>>,
    ) -> [u64; N] {
        use collector_primitives::clock::qpc;
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
        let mut countries = None;
        let prepared = {
            let _reads = collector_memory::read_accounting::table(TABLES[0]);
            self.prepare_with(memory, base, game, &mut countries, seeds)
        };
        if let Err(reason) = prepared {
            self.outcomes = [Outcome::Unavailable { reason }; N];
            self.held.clear();
            return ticks;
        }
        if let Some(parallel) = parallel
            && !self.movers.is_empty()
        {
            let shared = std::sync::Arc::new(PreparedInputs {
                movers: std::mem::take(&mut self.movers),
                route: std::mem::take(&mut self.route),
                provinces: std::mem::take(&mut self.provinces),
                province_slot: std::mem::take(&mut self.province_slot),
                province_pointers: std::mem::take(&mut self.province_pointers),
                state_facts: std::mem::take(&mut self.state_facts),
                templates: std::mem::take(&mut self.templates),
                keys: std::mem::take(&mut self.keys),
                combat_demand: std::mem::take(&mut self.combat_demand),
            });
            let entries_work = SpeedTermsEntriesWork {
                shared: std::sync::Arc::clone(&shared),
                game,
                take,
                held: std::mem::take(&mut self.held),
                observed_entries: std::mem::take(&mut self.observed_entries),
                entries: std::mem::take(&mut self.entries),
                counts: self.counts[2],
                outcome: UNCAPTURED,
                ticks: 0,
            };
            let aux_work = SpeedTermsAuxWork {
                shared: std::sync::Arc::clone(&shared),
                game,
                settle_pending: self.settle_pending,
                crossings: std::mem::take(&mut self.crossings),
                access: std::mem::take(&mut self.access),
                rivers: std::mem::take(&mut self.rivers),
                transport: std::mem::take(&mut self.transport),
                observed_access: std::mem::take(&mut self.observed_rows.access),
                observed_access_areas: std::mem::take(&mut self.observed_rows.access_areas),
                observed_access_ready: std::mem::take(&mut self.observed_rows.access_ready),
                observed_rivers: std::mem::take(&mut self.observed_rows.rivers),
                observed_transport: std::mem::take(&mut self.observed_rows.transport),
                regions: std::mem::take(&mut self.regions),
                transport_demand: std::mem::take(&mut self.transport_demand),
                speed_cache: std::mem::take(&mut self.speed_cache),
                wanted_rivers: std::mem::take(&mut self.scratch.wanted_rivers),
                pending_rivers: std::mem::take(&mut self.scratch.pending_rivers),
                pointers: std::mem::take(&mut self.scratch.pointers),
                located: std::mem::take(&mut self.scratch.located),
                wanted_transport: std::mem::take(&mut self.scratch.wanted_transport),
                indices: std::mem::take(&mut self.scratch.indices),
                counts: [self.counts[1], self.counts[3], self.counts[4]],
                outcomes: [UNCAPTURED; 3],
                ticks: [0; 3],
            };
            let mut parent_ticks = 0_u64;
            let (entries_out, aux_out) = parallel(entries_work, aux_work, &mut || {
                {
                    let _reads = collector_memory::read_accounting::table(TableId::DivisionStatus);
                    self.combat_captured = self
                        .capture_combat_with(memory, base, seeds, &shared.combat_demand)
                        .is_ok();
                    if !self.combat_captured {
                        self.combat.clear();
                        self.combat_order.clear();
                        self.counts[COMBAT] = [0, 0, seeds.len() as u64];
                    }
                }
                self.outcomes[0] = {
                    let _reads = collector_memory::read_accounting::table(TABLES[0]);
                    self.capture_states_from(
                        memory,
                        base,
                        game,
                        &mut countries,
                        &shared.movers,
                        &shared.provinces,
                        &shared.province_slot,
                        &shared.state_facts,
                    )
                };
                parent_ticks = qpc().saturating_sub(start).max(0) as u64;
            });
            ticks[0] = parent_ticks;
            match entries_out {
                Some(entries_work) => {
                    self.held = entries_work.held;
                    self.observed_entries = entries_work.observed_entries;
                    self.entries = entries_work.entries;
                    self.counts[2] = entries_work.counts;
                    self.outcomes[2] = entries_work.outcome;
                    ticks[2] = entries_work.ticks;
                }
                None => {
                    owned::forget();
                    self.outcomes[2] = Outcome::Unavailable {
                        reason: "unit speed entries acquisition stopped before reading",
                    };
                }
            }
            match aux_out {
                Some(aux_work) => {
                    self.crossings = aux_work.crossings;
                    self.access = aux_work.access;
                    self.rivers = aux_work.rivers;
                    self.transport = aux_work.transport;
                    self.observed_rows.access = aux_work.observed_access;
                    self.observed_rows.access_areas = aux_work.observed_access_areas;
                    self.observed_rows.access_ready = aux_work.observed_access_ready;
                    self.observed_rows.rivers = aux_work.observed_rivers;
                    self.observed_rows.transport = aux_work.observed_transport;
                    self.regions = aux_work.regions;
                    self.transport_demand = aux_work.transport_demand;
                    self.speed_cache = aux_work.speed_cache;
                    self.scratch.wanted_rivers = aux_work.wanted_rivers;
                    self.scratch.pending_rivers = aux_work.pending_rivers;
                    self.scratch.pointers = aux_work.pointers;
                    self.scratch.located = aux_work.located;
                    self.scratch.wanted_transport = aux_work.wanted_transport;
                    self.scratch.indices = aux_work.indices;
                    self.counts[1] = aux_work.counts[0];
                    self.counts[3] = aux_work.counts[1];
                    self.counts[4] = aux_work.counts[2];
                    self.outcomes[1] = aux_work.outcomes[0];
                    self.outcomes[3] = aux_work.outcomes[1];
                    self.outcomes[4] = aux_work.outcomes[2];
                    ticks[1] = aux_work.ticks[0];
                    ticks[3] = aux_work.ticks[1];
                    ticks[4] = aux_work.ticks[2];
                }
                None => {
                    let reason = "division speed terms auxiliary acquisition stopped before reading";
                    self.outcomes[1] = Outcome::Unavailable { reason };
                    self.outcomes[3] = Outcome::Unavailable { reason };
                    self.outcomes[4] = Outcome::Unavailable { reason };
                }
            }
            if let Ok(prepared) = std::sync::Arc::try_unwrap(shared) {
                self.movers = prepared.movers;
                self.route = prepared.route;
                self.provinces = prepared.provinces;
                self.province_slot = prepared.province_slot;
                self.province_pointers = prepared.province_pointers;
                self.state_facts = prepared.state_facts;
                self.templates = prepared.templates;
                self.keys = prepared.keys;
                self.combat_demand = prepared.combat_demand;
            }
            return ticks;
        }
        {
            let _reads = collector_memory::read_accounting::table(TableId::DivisionStatus);
            self.combat_captured = self.capture_combat(memory, base, seeds).is_ok();
            if !self.combat_captured {
                self.combat.clear();
                self.combat_order.clear();
                self.counts[COMBAT] = [0, 0, seeds.len() as u64];
            }
        }
        let mut lap = |slot: usize, at: &mut i64| {
            let now = qpc();
            ticks[slot] += now.saturating_sub(*at).max(0) as u64;
            *at = now;
        };
        let mut at = start;
        self.outcomes[0] = {
            let _reads = collector_memory::read_accounting::table(TABLES[0]);
            self.capture_states_with(memory, base, game, &mut countries)
        };
        lap(0, &mut at);
        self.outcomes[1] = {
            let _reads = collector_memory::read_accounting::table(TABLES[1]);
            self.capture_access_with(memory, base, game, &mut countries)
        };
        lap(1, &mut at);
        let countries = countries.get_or_insert_with(|| ccountry::Reader::new(memory, base, game));
        self.outcomes[2] = {
            let _reads = collector_memory::read_accounting::table(TABLES[2]);
            self.acquire_entries(memory, countries, take)
        };
        lap(2, &mut at);
        self.outcomes[3] = {
            let _reads = collector_memory::read_accounting::table(TABLES[3]);
            self.capture_rivers(memory, base, game, countries)
        };
        lap(3, &mut at);
        self.outcomes[4] = {
            let _reads = collector_memory::read_accounting::table(TABLES[4]);
            self.capture_transport(memory, base, game)
        };
        lap(4, &mut at);
        self.regions.clear();
        ticks
    }

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

    fn capture_combat(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
    ) -> Result<(), &'static str> {
        let demand = std::mem::take(&mut self.combat_demand);
        let result = self.capture_combat_with(memory, base, seeds, &demand);
        self.combat_demand = demand;
        result
    }

    fn capture_combat_with(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
        combat_demand: &[(u64, Result<bool, &'static str>)],
    ) -> Result<(), &'static str> {
        let list =
            |seed: &DivisionSeed, demand: Result<bool, &'static str>| -> Result<Option<Array>, &'static str> {
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
            if seed
                .ordinary_route
                .and_then(|header| Array::from_header(header, 4))
                .is_some_and(|route| route.count == 0)
            {
                self.combat.push((seed.division, Some(None)));
                continue;
            }
            let demand = memo(combat_demand, seed.division).unwrap_or(Ok(false));
            let value = (|| -> Result<Option<i64>, &'static str> {
                let Some(list) = list(seed, demand)? else {
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
            if demand != Ok(false) {
                self.counts[COMBAT][usize::from(value.is_err()) * 2] += 1;
            }
            self.combat.push((seed.division, value.ok()));
        }
        if !self.settle_pending {
            self.index_combat();
        }
        Ok(())
    }

    #[cfg(test)]
    fn prepare(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game: usize,
        seeds: &[DivisionSeed],
    ) -> Result<(), &'static str> {
        let mut countries = None;
        self.prepare_with(memory, base, game, &mut countries, seeds)
    }

    fn prepare_with<'a, M: Memory>(
        &mut self,
        memory: &'a M,
        base: usize,
        game: usize,
        countries: &mut Option<Result<ccountry::Reader<'a, M>, &'static str>>,
        seeds: &[DivisionSeed],
    ) -> Result<(), &'static str> {
        self.movers.clear();
        self.route.clear();
        for &(id, _) in &self.provinces {
            let idx = id as usize;
            if idx < self.province_slot.len() {
                self.province_slot[idx] = u32::MAX;
            }
        }
        self.provinces.clear();
        self.province_pointers.clear();
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
        let ids = &mut self.scratch.ids;
        ids.clear();
        let mut max_dense = 0_usize;
        for id in self
            .movers
            .iter()
            .filter_map(|mover| mover.location.ok())
            .chain(self.route.iter().filter_map(|id| id.ok()))
        {
            let idx = id as usize;
            if idx < DENSE_PROVINCE_LIMIT && idx + 1 > max_dense {
                max_dense = idx + 1;
            }
        }
        if max_dense > self.province_slot.len() {
            let extra = max_dense - self.province_slot.len();
            reserve(&mut self.province_slot, extra)?;
            self.province_slot.resize(max_dense, u32::MAX);
        }
        let mut has_sparse = false;
        let seen_mark = u32::MAX - 1;
        for id in self
            .movers
            .iter()
            .filter_map(|mover| mover.location.ok())
            .chain(self.route.iter().filter_map(|id| id.ok()))
        {
            let idx = id as usize;
            if idx < self.province_slot.len() {
                if self.province_slot[idx] == u32::MAX {
                    self.province_slot[idx] = seen_mark;
                    reserve(ids, 1)?;
                    ids.push(id);
                }
            } else {
                has_sparse = true;
                reserve(ids, 1)?;
                ids.push(id);
            }
        }
        ids.sort_unstable();
        if has_sparse {
            ids.dedup();
        }
        for (slot, &id) in ids.iter().enumerate() {
            let idx = id as usize;
            if idx < self.province_slot.len() {
                self.province_slot[idx] = slot as u32;
            }
        }
        reserve(&mut self.provinces, ids.len())?;
        reserve(&mut self.province_pointers, ids.len())?;
        self.province_pointers.resize(ids.len(), 0);
        let provinces = game
            .checked_add(engine_native::cgame_state::PROVINCES)
            .and_then(|address| Array::read(memory, address, 8))
            .ok_or("movement province array invalid");
        match provinces {
            Ok(provinces) => {
                let world = collector_lifetime::world_lifetime::token()
                    .filter(|_| collector_observers::world_input_changes::map_content_stable());
                if let Some(world) = world {
                    let missing_ids = &mut self.scratch.missing_ids;
                    let missing_query = &mut self.scratch.missing_query;
                    missing_ids.clear();
                    missing_query.clear();
                    let key = (world, base, provinces);
                    held_map(|held| {
                        if held.provinces_key != Some(key) {
                            held.provinces.clear();
                            held.provinces_key = Some(key);
                        }
                        for (slot, &id) in ids.iter().enumerate() {
                            if let Some(entry) = held.provinces.get(id as usize).copied().flatten() {
                                self.province_pointers[slot] = entry.pointer;
                                self.provinces.push((
                                    id,
                                    Ok(cprovince::MovementProvince {
                                        state: Ok(entry.state),
                                        controller: Err(UNFETCHED_CONTROLLER),
                                        controller_area: Err(UNFETCHED_AREA),
                                        strategic_region: Ok(entry.strategic_region),
                                        terrain_flags: entry.terrain_flags,
                                    }),
                                ));
                            } else {
                                self.provinces.push((id, Err("movement province unread")));
                                if reserve(missing_ids, 1).is_ok()
                                    && reserve(missing_query, 1).is_ok()
                                {
                                    missing_ids.push((slot, id));
                                    missing_query.push(id);
                                }
                            }
                        }
                    });
                    if !missing_query.is_empty() {
                        let mut index = 0;
                        held_map(|held| {
                            let store = held.provinces_key == Some(key);
                            if store {
                                let target = provinces.count as usize;
                                let missing = target.saturating_sub(held.provinces.len());
                                if missing != 0
                                    && engine_allocation::advisory(|| {
                                        engine_allocation::reserve(&mut held.provinces, missing)
                                    })
                                    .is_ok()
                                {
                                    held.provinces.resize(target, None);
                                }
                            }
                            cprovince::movement_provinces_with_pointers(
                                memory,
                                base,
                                provinces,
                                missing_query,
                                |id, pointer, facts| {
                                    let (slot, expected) = missing_ids[index];
                                    debug_assert_eq!(id, expected);
                                    index += 1;
                                    if let Some(pointer) = pointer {
                                        self.province_pointers[slot] = pointer;
                                    }
                                    if store
                                        && let (Some(pointer), Ok(facts)) = (pointer, facts)
                                        && let (Ok(state), Ok(strategic_region)) =
                                            (facts.state, facts.strategic_region)
                                        && let Some(entry) = held.provinces.get_mut(id as usize)
                                    {
                                        *entry = Some(cprovince::StaticMovementProvince {
                                            pointer,
                                            state,
                                            strategic_region,
                                            terrain_flags: facts.terrain_flags,
                                        });
                                    }
                                    self.provinces[slot].1 = facts;
                                },
                            );
                            held.update_charge();
                        });
                    }
                } else {
                    cprovince::movement_provinces(memory, base, provinces, ids, |id, facts| {
                        self.provinces.push((id, facts))
                    });
                }
            }
            Err(reason) => self
                .provinces
                .extend(ids.iter().map(|&id| (id, Err(reason)))),
        }
        self.prepare_land_demand_with(memory, base, game, countries)?;
        let states = &mut self.scratch.states;
        states.clear();
        reserve(states, self.movers.len())?;
        states.extend(
            self.movers
                .iter()
                .filter(|mover| {
                    mover.land == Ok(true) && mover.roster.is_some() && mover.owner.is_some()
                })
                .filter_map(|mover| mover.location.ok())
                .filter_map(|id| province_at(&self.provinces, &self.province_slot, id).ok())
                .filter_map(|facts| facts.state.ok())
                .filter(|&state| state != 0),
        );
        states.sort_unstable();
        states.dedup();
        reserve(&mut self.state_facts, states.len())?;
        engine_native::cstate::ids_and_controllers(memory, states, &mut self.state_facts);
        let templates = &mut self.scratch.templates;
        templates.clear();
        reserve(templates, self.movers.len())?;
        templates.extend(
            self.movers
                .iter()
                .filter(|mover| mover.land == Ok(true) && mover.owner.is_some())
                .filter_map(|mover| mover.template),
        );
        templates.sort_unstable();
        templates.dedup();
        reserve(&mut self.templates, templates.len())?;
        let template_token = collector_division_templates::template_changes::Changes::quiet(
            &collector_division_templates::template_changes::Observer,
        )
        .zip(
            collector_division_templates::template_changes::Changes::core_dependencies(
                &collector_division_templates::template_changes::Observer,
            ),
        )
        .map(|(q, d)| (q, d, base));
        if template_token.is_none() || self.held.template_token != template_token {
            self.held.template_token = template_token;
            self.held.template_spans.clear();
            self.held.template_keys.clear();
        }
        let mut added_templates = false;
        for &template in templates.iter() {
            let start = self.keys.len();
            if template_token.is_some()
                && let Ok(at) = self
                    .held
                    .template_spans
                    .binary_search_by_key(&template, |entry| entry.0)
            {
                let (_, h_start, h_len) = self.held.template_spans[at];
                reserve(&mut self.keys, h_len)?;
                self.keys
                    .extend_from_slice(&self.held.template_keys[h_start..h_start + h_len]);
                self.templates.push((template, Ok((start, h_len))));
                continue;
            }
            if engine_native::carmy::movement_speed_key_demands(
                memory,
                base,
                template,
                &mut self.keys,
            )
            .is_err()
            {
                self.counts[2][2] += 1;
            } else if template_token.is_some() {
                let len = self.keys.len() - start;
                if engine_allocation::advisory(|| {
                    engine_allocation::reserve(&mut self.held.template_spans, 1)?;
                    engine_allocation::reserve(&mut self.held.template_keys, len)
                })
                .is_ok()
                {
                    let h_start = self.held.template_keys.len();
                    self.held
                        .template_keys
                        .extend_from_slice(&self.keys[start..start + len]);
                    self.held.template_spans.push((template, h_start, len));
                    added_templates = true;
                }
            }
            self.templates
                .push((template, Ok((start, self.keys.len() - start))));
        }
        if added_templates {
            self.held.template_spans.sort_unstable_by_key(|entry| entry.0);
        }
        Ok(())
    }

    #[cfg(test)]
    fn prepare_land_demand(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game: usize,
    ) -> Result<(), &'static str> {
        let mut countries = None;
        self.prepare_land_demand_with(memory, base, game, &mut countries)
    }

    fn prepare_land_demand_with<'a, M: Memory>(
        &mut self,
        memory: &'a M,
        base: usize,
        game: usize,
        countries: &mut Option<Result<ccountry::Reader<'a, M>, &'static str>>,
    ) -> Result<(), &'static str> {
        let ctrl_slots = &mut self.scratch.ctrl_slots;
        let area_slots = &mut self.scratch.area_slots;
        ctrl_slots.clear();
        area_slots.clear();
        for mover in &mut self.movers {
            mover.land = (|| {
                if mover.transfer == Some(true) {
                    return Ok(false);
                }
                let path = route(&self.route, mover)?;
                let location = province_at(&self.provinces, &self.province_slot, mover.location?)?;
                let (normal, refused) =
                    engine_native::carmy::organisation_movement::land_speed_demand(
                        mover
                            .location
                            .and_then(|id| location.terrain_flags.map(|flags| (id, flags))),
                        path.iter().map(|id| {
                            id.and_then(|id| {
                                province_at(&self.provinces, &self.province_slot, id)
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
                        .get_or_insert_with(|| ccountry::Reader::new(memory, base, game))
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
                if let Ok(path) = route(&self.route, mover)
                    && let Some(&Ok(last_id)) = path.last()
                    && let Some(slot) = province_slot_of(&self.provinces, &self.province_slot, last_id)
                    && let Ok(facts) = self.provinces[slot].1
                    && is_unfetched(&facts.controller)
                    && self.province_pointers.get(slot).copied().unwrap_or(0) != 0
                {
                    reserve(ctrl_slots, 1)?;
                    ctrl_slots.push(slot);
                }
                if mover.noncore == Some(true)
                    && let Ok(loc_id) = mover.location
                    && let Some(slot) = province_slot_of(&self.provinces, &self.province_slot, loc_id)
                    && let Ok(facts) = self.provinces[slot].1
                    && is_unfetched_area(&facts.controller_area)
                    && self.province_pointers.get(slot).copied().unwrap_or(0) != 0
                {
                    reserve(area_slots, 1)?;
                    area_slots.push(slot);
                }
            }
            if mover.land.is_err() {
                for family in &mut self.counts[..4] {
                    family[2] += 1;
                }
            }
        }
        if !ctrl_slots.is_empty() {
            ctrl_slots.sort_unstable();
            ctrl_slots.dedup();
            let pointers = &mut self.scratch.cached_pointers;
            let results = &mut self.scratch.ctrl_results;
            pointers.clear();
            results.clear();
            reserve(pointers, ctrl_slots.len())?;
            reserve(results, ctrl_slots.len())?;
            pointers.extend(ctrl_slots.iter().map(|&slot| self.province_pointers[slot]));
            results.resize(
                ctrl_slots.len(),
                Err("movement province controller unreadable"),
            );
            cprovince::movement_province_controllers_only(memory, pointers, results);
            for (&slot, &controller) in ctrl_slots.iter().zip(results.iter()) {
                if let Ok(facts) = &mut self.provinces[slot].1 {
                    facts.controller = controller;
                }
            }
        }
        if !area_slots.is_empty() {
            area_slots.sort_unstable();
            area_slots.dedup();
            let pointers = &mut self.scratch.cached_pointers;
            let results = &mut self.scratch.area_results;
            pointers.clear();
            results.clear();
            reserve(pointers, area_slots.len())?;
            reserve(results, area_slots.len())?;
            pointers.extend(area_slots.iter().map(|&slot| self.province_pointers[slot]));
            results.resize(area_slots.len(), Err("movement province field unreadable"));
            cprovince::movement_province_controller_areas(memory, pointers, results);
            for (&slot, &area) in area_slots.iter().zip(results.iter()) {
                if let Ok(facts) = &mut self.provinces[slot].1 {
                    facts.controller_area = area;
                }
            }
        }
        for mover in &mut self.movers {
            if mover.land == Ok(true) {
                let last = route(&self.route, mover)
                    .and_then(|path| path.last().copied().unwrap_or(Err("division route empty")));
                mover.attacking = self.hostility.attacking_with(
                    memory,
                    base,
                    game,
                    countries,
                    &self.provinces,
                    &self.province_slot,
                    last,
                    mover.logical,
                );
            }
        }
        self.combat_demand.clear();
        reserve(&mut self.combat_demand, self.movers.len())?;
        for mover in &self.movers {
            self.combat_demand.push((mover.division, mover.land));
        }
        self.combat_demand.sort_unstable_by_key(|entry| entry.0);
        Ok(())
    }

    #[cfg(test)]
    fn capture_states(&mut self, memory: &impl Memory, base: usize, game: usize) -> Outcome {
        let mut countries = None;
        self.capture_states_with(memory, base, game, &mut countries)
    }

    fn capture_states_with<'a, M: Memory>(
        &mut self,
        memory: &'a M,
        base: usize,
        game: usize,
        countries: &mut Option<Result<ccountry::Reader<'a, M>, &'static str>>,
    ) -> Outcome {
        let movers = std::mem::take(&mut self.movers);
        let provinces = std::mem::take(&mut self.provinces);
        let province_slot = std::mem::take(&mut self.province_slot);
        let state_facts = std::mem::take(&mut self.state_facts);
        let outcome = self.capture_states_from(
            memory,
            base,
            game,
            countries,
            &movers,
            &provinces,
            &province_slot,
            &state_facts,
        );
        self.movers = movers;
        self.provinces = provinces;
        self.province_slot = province_slot;
        self.state_facts = state_facts;
        outcome
    }

    #[allow(
        clippy::type_complexity,
        clippy::too_many_arguments,
        reason = "shares state-term capture between serial and parallel prepared-input slices"
    )]
    fn capture_states_from<'a, M: Memory>(
        &mut self,
        memory: &'a M,
        base: usize,
        game: usize,
        countries: &mut Option<Result<ccountry::Reader<'a, M>, &'static str>>,
        movers: &[Mover],
        provinces: &[(u32, Result<cprovince::MovementProvince, &'static str>)],
        province_slot: &[u32],
        state_facts: &[(usize, Result<(u32, u32), &'static str>)],
    ) -> Outcome {
        let result = (|| -> Result<usize, &'static str> {
            let wanted = &mut self.scratch.wanted_states;
            wanted.clear();
            reserve(wanted, movers.len())?;
            for mover in movers.iter().filter(|mover| mover.land == Ok(true)) {
                let key =
                    (|| -> Result<Option<(u32, u32, u32, usize, u32, bool)>, &'static str> {
                        let province = province_at(provinces, province_slot, mover.location?)?;
                        let state = province.state?;
                        if state == 0 {
                            return Ok(None);
                        }
                        let (id, controller) =
                            memo(state_facts, state).ok_or("movement state unread")??;
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
            let branches = &mut self.scratch.branches;
            branches.clear();
            reserve(branches, wanted.len())?;
            branches.extend(
                wanted
                    .iter()
                    .map(|&(_, _, owner, _, controller, _)| ((owner, controller), Ok(0))),
            );
            branches.sort_unstable_by_key(|entry| entry.0);
            branches.dedup_by_key(|entry| entry.0);
            if !branches.is_empty() {
                let countries = countries.get_or_insert_with(|| ccountry::Reader::new(memory, base, game));
                for ((owner, controller), branch) in branches.iter_mut() {
                    *branch = countries
                        .as_ref()
                        .map_err(|reason| *reason)
                        .and_then(|reader| {
                            Ok(
                                match ccountry::movement_relation_with(
                                    memory,
                                    base,
                                    reader,
                                    *owner,
                                    *controller,
                                )? {
                                    ccountry::MovementRelation::Friendly => {
                                        StateMovementTerms::FRIENDLY
                                    }
                                    ccountry::MovementRelation::Enemy => StateMovementTerms::ENEMY,
                                    ccountry::MovementRelation::Neither => {
                                        StateMovementTerms::NEITHER
                                    }
                                },
                            )
                        });
                }
            }
            let links = countries
                .as_ref()
                .and_then(|result| result.as_ref().ok())
                .map(|reader| reader.arrays()[1]);
            let mut acc_cache: Option<((usize, u32), Result<usize, &'static str>)> = None;
            let mut block: Option<(
                (usize, u32, u8, bool),
                Result<owned::StateInputs, &'static str>,
            )> = None;
            reserve(&mut self.states, wanted.len())?;
            reserve(&mut self.observed_rows.states, wanted.len())?;
            for &(state_id, roster, owner, state, controller, attacking) in wanted.iter() {
                let branch =
                    memo(branches, (owner, controller)).ok_or("movement branch unread")?;
                let terms = branch.and_then(|branch| {
                    let key = (state, roster, branch, attacking);
                    if let Some((held, terms)) = block
                        && held == key
                    {
                        return terms;
                    }
                    let terms = (|| {
                        let at = match acc_cache {
                            Some((cached, at)) if cached == (state, roster) => at,
                            _ => {
                                let at =
                                    engine_native::cstate::country_modifier_accumulator_with_links(
                                        memory, links, state, roster,
                                    );
                                acc_cache = Some(((state, roster), at));
                                at
                            }
                        }?;
                        let (selected, len) = state_keys(branch);
                        let mut query = [0_u32; 3];
                        let mut found = [Value::default(); 3];
                        query[0] = selected[0];
                        let mut count = 1;
                        if len == 2 {
                            query[count] = selected[1];
                            count += 1;
                        }
                        if attacking {
                            query[count] = STATE_KEYS[3];
                            count += 1;
                        }
                        let (army, relation, attack) = if accumulator::lookup_requested_many_at(
                            memory,
                            at,
                            &query[..count],
                            &mut found[..count],
                        )
                        .is_ok()
                        {
                            let army = found[0];
                            let relation = (len == 2).then_some(found[1]);
                            let attack = if attacking {
                                Ok(found[count - 1])
                            } else {
                                Ok(Value::default())
                            };
                            (army, relation, attack)
                        } else {
                            let army = accumulator::lookup_requested_at(memory, at, selected[0])?;
                            let relation = if len == 2 {
                                Some(accumulator::lookup_requested_at(memory, at, selected[1])?)
                            } else {
                                None
                            };
                            let attack = if attacking {
                                accumulator::lookup_requested_at(memory, at, STATE_KEYS[3])
                            } else {
                                Ok(Value::default())
                            };
                            (army, relation, attack)
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

    #[cfg(test)]
    fn capture_access(&mut self, memory: &impl Memory, base: usize, game: usize) -> Outcome {
        let mut countries = None;
        self.capture_access_with(memory, base, game, &mut countries)
    }

    fn capture_access_with<'a, M: Memory>(
        &mut self,
        memory: &'a M,
        base: usize,
        game: usize,
        countries: &mut Option<Result<ccountry::Reader<'a, M>, &'static str>>,
    ) -> Outcome {
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
            reserve(&mut areas, wanted.len())?;
            for &(owner, province) in &wanted {
                if let Ok(facts) = province_at(&self.provinces, &self.province_slot, province)
                    && let Ok(area) = facts.controller_area
                {
                    areas.push(((owner, area), Ok(false)));
                }
            }
            areas.sort_unstable_by_key(|entry| entry.0);
            areas.dedup_by_key(|entry| entry.0);
            if !areas.is_empty() {
                let mut first_unhooked = areas.len();
                for (index, ((owner, area), access)) in areas.iter_mut().enumerate() {
                    if let Some(result) = memory.core_supply_access(base, game, *owner, *area) {
                        *access = result;
                    } else {
                        first_unhooked = index;
                        break;
                    }
                }
                if first_unhooked < areas.len() {
                    let reader = countries
                        .get_or_insert_with(|| ccountry::Reader::new(memory, base, game));
                    match reader {
                        Ok(reader) => {
                            let mut cache =
                                ccountry::CoreSupplyAccessCache::new(memory, base, game, reader);
                            for ((owner, area), access) in &mut areas[first_unhooked..] {
                                *access = cache.has_access(*owner, *area);
                            }
                        }
                        Err(reason) => {
                            for ((owner, area), access) in &mut areas[first_unhooked..] {
                                *access = memory
                                    .core_supply_access(base, game, *owner, *area)
                                    .unwrap_or(Err(*reason));
                            }
                        }
                    }
                }
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
        let mut pairs = std::mem::take(&mut self.observed_entries.pairs);
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
                        reserve(&mut pairs, 1)?;
                        pairs.push((owner, start, len));
                    }
                    Err(_) => self.counts[2][2] += 1,
                }
            }
            pairs.sort_unstable();
            pairs.dedup();
            for &(owner, start, len) in &pairs {
                reserve(&mut wanted, len)?;
                wanted.extend(
                    self.keys[start..start + len]
                        .iter()
                        .map(|&key| (owner, key)),
                );
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
                        let mut key_buf = [0_u32; 64];
                        let mut val_buf = [Value::default(); 64];
                        let mut offset = 0;
                        while offset < group.len() {
                            let count = (group.len() - offset).min(64);
                            let slice = &group[offset..offset + count];
                            for (dst, &(_, key)) in key_buf[..count].iter_mut().zip(slice) {
                                *dst = key;
                            }
                            if accumulator::lookup_requested_many_at(
                                memory,
                                at,
                                &key_buf[..count],
                                &mut val_buf[..count],
                            )
                            .is_ok()
                            {
                                values.extend(val_buf[..count].iter().copied().map(Ok));
                            } else {
                                values.extend(slice.iter().map(|&(_, key)| {
                                    accumulator::lookup_requested_at(memory, at, key)
                                }));
                            }
                            offset += count;
                        }
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
        self.observed_entries.pairs = pairs;
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

    fn capture_rivers(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game: usize,
        countries: &Result<ccountry::Reader<'_, impl Memory>, &'static str>,
    ) -> Outcome {
        self.capture_rivers_using(memory, base, game, countries, &HELD_CROSSINGS)
    }

    fn capture_rivers_using(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game: usize,
        countries: &Result<ccountry::Reader<'_, impl Memory>, &'static str>,
        cache: &Mutex<Option<HeldCrossings>>,
    ) -> Outcome {
        if !self.movers.iter().any(|mover| mover.land == Ok(true)) {
            return self.finish(Ok(0), 3);
        }
        let mut crossings = std::mem::take(&mut self.crossings);
        let mut ready: Option<Result<(), &'static str>> = None;
        let result = held_crossings(cache, |held| {
            let world = collector_lifetime::world_lifetime::generation();
            if held.world != Some(world) || world == u64::MAX {
                held.answers.clear();
                held.world = (world != u64::MAX).then_some(world);
            }
            let result = self.capture_rivers_with_crossings(memory, base, countries, |from, to| {
                if let Some(&answer) = held.answers.get(&(from, to)) {
                    return Ok(answer);
                }
                let begun = *ready.get_or_insert_with(|| crossings.begin(memory, base, game));
                let answer = begun.and_then(|()| crossings.acquire_crossing(memory, from, to))?;
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
            held.charge.set(collector_primitives::raw_memory::Capacity {
                index_vectors: (held.answers.capacity()
                    * (size_of::<((u32, u32), Option<u8>)>() + 1))
                    as u64,
                ..Default::default()
            });
            result
        });
        if ready.is_some() {
            crossings.clear();
        }
        self.crossings = crossings;
        result
    }

    fn capture_rivers_with_crossings(
        &mut self,
        memory: &impl Memory,
        base: usize,
        countries: &Result<ccountry::Reader<'_, impl Memory>, &'static str>,
        crossing: impl FnMut(u32, u32) -> Result<Option<u8>, &'static str>,
    ) -> Outcome {
        let result = capture_rivers_with_crossings_into(
            memory,
            base,
            countries,
            &self.movers,
            &self.route,
            &self.provinces,
            &self.province_slot,
            &self.province_pointers,
            &mut self.scratch.wanted_rivers,
            &mut self.scratch.pending_rivers,
            &mut self.rivers,
            &mut self.observed_rows.rivers,
            &mut self.counts[3],
            crossing,
        );
        if !self.settle_pending {
            self.settle_family(3);
        }
        self.finish(result, 3)
    }

    fn capture_transport(&mut self, memory: &impl Memory, base: usize, game: usize) -> Outcome {
        let result = (|| -> Result<usize, &'static str> {
            self.transport_demand.clear();
            for mover in &self.movers {
                let provinces = append_transport_demand(
                    &mut self.transport_demand,
                    &self.route,
                    &self.provinces,
                    &self.province_slot,
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
            let pointers = &mut self.scratch.pointers;
            pointers.clear();
            reserve(pointers, demand.len())?;
            pointers.extend(demand.iter().map(|&(_, region)| region));
            pointers.sort_unstable();
            pointers.dedup();
            self.regions.clear();
            reserve(&mut self.regions, pointers.len())?;
            let located = &mut self.scratch.located;
            located.clear();
            reserve(located, pointers.len())?;
            let world = collector_lifetime::world_lifetime::token()
                .filter(|_| collector_observers::world_input_changes::map_content_stable());
            if let Some(world) = world {
                let key = (world, base);
                held_map(|held| {
                    if held.regions_key != Some(key) {
                        held.regions.clear();
                        held.regions_key = Some(key);
                    }
                    let mut inserted = false;
                    for &pointer in pointers.iter() {
                        if let Some(&id) = held.regions.get(&pointer) {
                            self.regions.push(TransportRegion { id, pointer });
                            located.push((pointer, id));
                            continue;
                        }
                        match engine_native::cstrategic_region::id(memory, base, pointer) {
                            Ok(Some(id)) => {
                                if engine_allocation::advisory(|| {
                                    engine_allocation::reserve_map(&mut held.regions, 1)
                                })
                                .is_ok()
                                {
                                    held.regions.insert(pointer, id);
                                    inserted = true;
                                }
                                self.regions.push(TransportRegion { id, pointer });
                                located.push((pointer, id));
                            }
                            Ok(None) | Err(_) => self.counts[4][2] += 1,
                        }
                    }
                    if inserted {
                        held.update_charge();
                    }
                });
            } else {
                for &pointer in pointers.iter() {
                    match engine_native::cstrategic_region::id(memory, base, pointer) {
                        Ok(Some(id)) => {
                            self.regions.push(TransportRegion { id, pointer });
                            located.push((pointer, id));
                        }
                        Ok(None) | Err(_) => self.counts[4][2] += 1,
                    }
                }
            }
            self.regions.sort_unstable_by_key(|region| region.id);
            self.regions.dedup_by_key(|region| region.id);
            let wanted = &mut self.scratch.wanted_transport;
            wanted.clear();
            reserve(wanted, demand.len())?;
            for &(owner, pointer) in demand.iter() {
                let Some(id) = memo(located, pointer) else {
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
            let indices = &mut self.scratch.indices;
            indices.clear();
            reserve(indices, self.regions.len())?;
            let (transport, cache) = (&mut self.observed_rows.transport, &mut self.speed_cache);
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

fn append_transport_demand(
    demand: &mut Vec<(u32, usize)>,
    arena: &[Result<u32, &'static str>],
    provinces: &[(u32, Result<cprovince::MovementProvince, &'static str>)],
    province_slot: &[u32],
    mover: &Mover,
) -> Result<u64, &'static str> {
    let path = match route(arena, mover) {
        Ok(path) => path,
        Err(_) => return Ok(1),
    };
    let mut from_id = mover.location;
    let mut refused = 0;
    for (index, &to_id) in path.iter().enumerate() {
        let from = from_id.and_then(|id| province_at(provinces, province_slot, id));
        let to = to_id.and_then(|id| province_at(provinces, province_slot, id));
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

#[derive(Default)]
struct Take {
    epoch: Option<u64>,
    complete: bool,
    marked: Vec<u32>,
}

struct PreparedInputs {
    movers: Vec<Mover>,
    route: Vec<Result<u32, &'static str>>,
    provinces: Vec<(u32, Result<cprovince::MovementProvince, &'static str>)>,
    province_slot: Vec<u32>,
    province_pointers: Vec<usize>,
    state_facts: Vec<(usize, Result<(u32, u32), &'static str>)>,
    templates: Vec<(usize, Result<(usize, usize), &'static str>)>,
    keys: Vec<u32>,
    combat_demand: Vec<(u64, Result<bool, &'static str>)>,
}

pub type ParallelRunner<'a> = &'a mut dyn FnMut(
    SpeedTermsEntriesWork,
    SpeedTermsAuxWork,
    &mut dyn FnMut(),
) -> (Option<SpeedTermsEntriesWork>, Option<SpeedTermsAuxWork>);

pub struct SpeedTermsEntriesWork {
    shared: std::sync::Arc<PreparedInputs>,
    game: usize,
    take: Take,
    held: owned::Loan,
    observed_entries: owned::Entries,
    entries: Vec<CountryUnitSpeedEntryRow>,
    counts: [u64; 3],
    outcome: Outcome,
    ticks: u64,
}

impl SpeedTermsEntriesWork {
    pub fn capture(&mut self, memory: &impl Memory, base: usize) {
        let _allocation_phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
        let _reads = collector_memory::read_accounting::table(TABLES[2]);
        let start = collector_primitives::clock::qpc();
        let countries = ccountry::Reader::new(memory, base, self.game);
        let take = std::mem::take(&mut self.take);
        self.outcome = acquire_entries_into(
            memory,
            &countries,
            take,
            &self.shared.movers,
            &self.shared.templates,
            &self.shared.keys,
            EntriesParts {
                held: &self.held,
                observed: &mut self.observed_entries,
                entries: &mut self.entries,
                counts: &mut self.counts,
            },
        );
        self.ticks = collector_primitives::clock::qpc().saturating_sub(start).max(0) as u64;
    }
}

struct EntriesParts<'a> {
    held: &'a owned::Loan,
    observed: &'a mut owned::Entries,
    entries: &'a mut Vec<CountryUnitSpeedEntryRow>,
    counts: &'a mut [u64; 3],
}

#[allow(
    clippy::too_many_arguments,
    reason = "shares entry acquisition between serial SpeedTerms and parallel SpeedTermsEntriesWork"
)]
fn acquire_entries_into(
    memory: &impl Memory,
    countries: &Result<ccountry::Reader<'_, impl Memory>, &'static str>,
    take: Take,
    movers: &[Mover],
    templates: &[(usize, Result<(usize, usize), &'static str>)],
    keys: &[u32],
    parts: EntriesParts<'_>,
) -> Outcome {
    let arrays = countries.as_ref().ok().map(|reader| reader.arrays());
    let certified = take.complete
        && take.epoch.is_some()
        && take.epoch == parts.held.epoch
        && arrays == Some(parts.held.arrays);
    let counts = Counts {
        marked: take.marked.len() as u64,
        full: !certified,
        ..Counts::default()
    };
    parts.observed.clear();
    let mut pairs = std::mem::take(&mut parts.observed.pairs);
    let mut wanted = std::mem::take(&mut parts.observed.wanted);
    let mut values = std::mem::take(&mut parts.observed.values);
    let mut observed = std::mem::take(&mut parts.observed.countries);
    let result = (|| -> Result<usize, &'static str> {
        for mover in movers.iter().filter(|mover| mover.land == Ok(true)) {
            let Some(owner) = mover.owner else {
                parts.counts[2] += 1;
                continue;
            };
            let span = mover
                .template
                .ok_or("division template unreadable")
                .and_then(|template| memo(templates, template).ok_or("division template unread")?);
            match span {
                Ok((start, len)) => {
                    reserve(&mut pairs, 1)?;
                    pairs.push((owner, start, len));
                }
                Err(_) => parts.counts[2] += 1,
            }
        }
        pairs.sort_unstable();
        pairs.dedup();
        for &(owner, start, len) in &pairs {
            reserve(&mut wanted, len)?;
            wanted.extend(keys[start..start + len].iter().map(|&key| (owner, key)));
        }
        wanted.sort_unstable();
        wanted.dedup();
        reserve(parts.entries, wanted.len())?;
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
            let held = country.ok().filter(|_| certified).and_then(|country| {
                let at = parts
                    .held
                    .countries
                    .binary_search_by_key(&owner, |held| held.owner)
                    .ok()?;
                let held = parts.held.countries[at];
                let held_keys = &parts.held.keys[held.start..held.start + held.len];
                (held.country == country
                    && take.marked.binary_search(&country.id).is_err()
                    && group.iter().all(|(_, key)| held_keys.binary_search(key).is_ok()))
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
                    let mut key_buf = [0_u32; 64];
                    let mut val_buf = [Value::default(); 64];
                    let mut offset = 0;
                    while offset < group.len() {
                        let count = (group.len() - offset).min(64);
                        let slice = &group[offset..offset + count];
                        for (dst, &(_, key)) in key_buf[..count].iter_mut().zip(slice) {
                            *dst = key;
                        }
                        if accumulator::lookup_requested_many_at(
                            memory,
                            at,
                            &key_buf[..count],
                            &mut val_buf[..count],
                        )
                        .is_ok()
                        {
                            values.extend(val_buf[..count].iter().copied().map(Ok));
                        } else {
                            values.extend(slice.iter().map(|&(_, key)| {
                                accumulator::lookup_requested_at(memory, at, key)
                            }));
                        }
                        offset += count;
                    }
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
    parts.observed.pairs = pairs;
    parts.observed.wanted = wanted;
    parts.observed.values = values;
    parts.observed.countries = observed;
    parts.observed.epoch = take.epoch;
    parts.observed.arrays = arrays;
    parts.observed.complete = take.complete;
    parts.observed.counts = counts;
    parts.observed.result = Some(result.map(|_| ()));
    parts.observed.pending = true;
    result.map_or_else(
        |reason| Outcome::Unavailable { reason },
        |len| Outcome::Rows { len },
    )
}

#[allow(
    clippy::type_complexity,
    reason = "holds disjoint output and scratch buffers for parallel access/river/transport worker"
)]
pub struct SpeedTermsAuxWork {
    shared: std::sync::Arc<PreparedInputs>,
    game: usize,
    settle_pending: bool,
    crossings: cprovince::GeometryPacket,
    access: Vec<CoreSupplyAccessRow>,
    rivers: Vec<RiverRelationRow>,
    transport: Vec<OwnerTransportSpeedRow>,
    observed_access: Vec<(u32, u32)>,
    observed_access_areas: Vec<((u32, usize), Result<bool, &'static str>)>,
    observed_access_ready: bool,
    observed_rivers: Vec<((u32, u32), Result<Option<i64>, &'static str>)>,
    observed_transport: Vec<(
        (u32, u32),
        Result<engine_naval_history::cnaval_unit_transfer::SpeedInputs, &'static str>,
    )>,
    regions: Vec<TransportRegion>,
    transport_demand: Vec<(u32, usize)>,
    speed_cache: SpeedCache,
    wanted_rivers: Vec<(u32, u32)>,
    pending_rivers: Vec<(u32, usize)>,
    pointers: Vec<usize>,
    located: Vec<(usize, u32)>,
    wanted_transport: Vec<(u32, usize)>,
    indices: Vec<usize>,
    counts: [[u64; 3]; 3],
    outcomes: [Outcome; 3],
    ticks: [u64; 3],
}

impl SpeedTermsAuxWork {
    pub fn capture(&mut self, memory: &impl Memory, base: usize) {
        let _allocation_phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
        let game = self.game;
        let mut countries = None;
        let mut at = collector_primitives::clock::qpc();
        self.outcomes[0] = {
            let _reads = collector_memory::read_accounting::table(TABLES[1]);
            capture_access_into(
                memory,
                base,
                game,
                &mut countries,
                &self.shared.movers,
                &self.shared.provinces,
                &self.shared.province_slot,
                AccessParts {
                    access: &mut self.access,
                    wanted: &mut self.observed_access,
                    areas: &mut self.observed_access_areas,
                    ready: &mut self.observed_access_ready,
                    counts: &mut self.counts[0],
                },
            )
        };
        let now = collector_primitives::clock::qpc();
        self.ticks[0] = now.saturating_sub(at).max(0) as u64;
        at = now;
        let countries = countries.get_or_insert_with(|| ccountry::Reader::new(memory, base, game));
        self.outcomes[1] = {
            let _reads = collector_memory::read_accounting::table(TABLES[3]);
            capture_rivers_into(
                memory,
                base,
                game,
                countries,
                &self.shared.movers,
                &self.shared.route,
                &self.shared.provinces,
                &self.shared.province_slot,
                &self.shared.province_pointers,
                RiversParts {
                    crossings: &mut self.crossings,
                    wanted: &mut self.wanted_rivers,
                    pending: &mut self.pending_rivers,
                    rivers: &mut self.rivers,
                    observed: &mut self.observed_rivers,
                    counts: &mut self.counts[1],
                },
            )
        };
        let now = collector_primitives::clock::qpc();
        self.ticks[1] = now.saturating_sub(at).max(0) as u64;
        at = now;
        self.outcomes[2] = {
            let _reads = collector_memory::read_accounting::table(TABLES[4]);
            capture_transport_into(
                memory,
                base,
                game,
                &self.shared.movers,
                &self.shared.route,
                &self.shared.provinces,
                &self.shared.province_slot,
                TransportParts {
                    demand: &mut self.transport_demand,
                    pointers: &mut self.pointers,
                    regions: &mut self.regions,
                    located: &mut self.located,
                    wanted: &mut self.wanted_transport,
                    indices: &mut self.indices,
                    cache: &mut self.speed_cache,
                    transport: &mut self.transport,
                    observed: &mut self.observed_transport,
                    counts: &mut self.counts[2],
                },
            )
        };
        let now = collector_primitives::clock::qpc();
        self.ticks[2] = now.saturating_sub(at).max(0) as u64;
        self.regions.clear();
        let _ = self.settle_pending;
    }
}

#[allow(
    clippy::type_complexity,
    reason = "borrows disjoint access buffers for shared serial and parallel helper"
)]
struct AccessParts<'a> {
    access: &'a mut Vec<CoreSupplyAccessRow>,
    wanted: &'a mut Vec<(u32, u32)>,
    areas: &'a mut Vec<((u32, usize), Result<bool, &'static str>)>,
    ready: &'a mut bool,
    counts: &'a mut [u64; 3],
}

#[allow(
    clippy::too_many_arguments,
    reason = "shares core supply access capture between serial and parallel paths"
)]
fn capture_access_into<'a, M: Memory>(
    memory: &'a M,
    base: usize,
    game: usize,
    countries: &mut Option<Result<ccountry::Reader<'a, M>, &'static str>>,
    movers: &[Mover],
    provinces: &[(u32, Result<cprovince::MovementProvince, &'static str>)],
    province_slot: &[u32],
    parts: AccessParts<'_>,
) -> Outcome {
    parts.wanted.clear();
    parts.areas.clear();
    *parts.ready = false;
    let result = (|| -> Result<usize, &'static str> {
        reserve(parts.wanted, movers.len())?;
        for mover in movers.iter().filter(|mover| mover.land == Ok(true)) {
            match (mover.noncore, mover.location) {
                (Some(false), _) => {}
                (Some(true), Ok(province)) => match mover.owner {
                    Some(owner) => parts.wanted.push((owner, province)),
                    None => parts.counts[2] += 1,
                },
                _ => parts.counts[2] += 1,
            }
        }
        parts.wanted.sort_unstable();
        parts.wanted.dedup();
        reserve(parts.areas, parts.wanted.len())?;
        for &(owner, province) in parts.wanted.iter() {
            if let Ok(facts) = province_at(provinces, province_slot, province)
                && let Ok(area) = facts.controller_area
            {
                parts.areas.push(((owner, area), Ok(false)));
            }
        }
        parts.areas.sort_unstable_by_key(|entry| entry.0);
        parts.areas.dedup_by_key(|entry| entry.0);
        if !parts.areas.is_empty() {
            let mut first_unhooked = parts.areas.len();
            for (index, ((owner, area), access)) in parts.areas.iter_mut().enumerate() {
                if let Some(result) = memory.core_supply_access(base, game, *owner, *area) {
                    *access = result;
                } else {
                    first_unhooked = index;
                    break;
                }
            }
            if first_unhooked < parts.areas.len() {
                let reader =
                    countries.get_or_insert_with(|| ccountry::Reader::new(memory, base, game));
                match reader {
                    Ok(reader) => {
                        let mut cache =
                            ccountry::CoreSupplyAccessCache::new(memory, base, game, reader);
                        for ((owner, area), access) in &mut parts.areas[first_unhooked..] {
                            *access = cache.has_access(*owner, *area);
                        }
                    }
                    Err(reason) => {
                        for ((owner, area), access) in &mut parts.areas[first_unhooked..] {
                            *access = memory
                                .core_supply_access(base, game, *owner, *area)
                                .unwrap_or(Err(*reason));
                        }
                    }
                }
            }
        }
        reserve(parts.access, parts.wanted.len())?;
        *parts.ready = true;
        Ok(parts.access.len() + parts.wanted.len())
    })();
    match result {
        Ok(len) => Outcome::Rows { len },
        Err(reason) => {
            parts.access.clear();
            Outcome::Unavailable { reason }
        }
    }
}

#[allow(
    clippy::type_complexity,
    reason = "borrows disjoint river buffers for shared serial and parallel helper"
)]
struct RiversParts<'a> {
    crossings: &'a mut cprovince::GeometryPacket,
    wanted: &'a mut Vec<(u32, u32)>,
    pending: &'a mut Vec<(u32, usize)>,
    rivers: &'a mut Vec<RiverRelationRow>,
    observed: &'a mut Vec<((u32, u32), Result<Option<i64>, &'static str>)>,
    counts: &'a mut [u64; 3],
}

#[allow(
    clippy::too_many_arguments,
    reason = "shares river relation capture between serial and parallel paths"
)]
fn capture_rivers_into(
    memory: &impl Memory,
    base: usize,
    game: usize,
    countries: &Result<ccountry::Reader<'_, impl Memory>, &'static str>,
    movers: &[Mover],
    route_arena: &[Result<u32, &'static str>],
    provinces: &[(u32, Result<cprovince::MovementProvince, &'static str>)],
    province_slot: &[u32],
    province_pointers: &[usize],
    parts: RiversParts<'_>,
) -> Outcome {
    if !movers.iter().any(|mover| mover.land == Ok(true)) {
        return Outcome::Rows { len: 0 };
    }
    let mut ready: Option<Result<(), &'static str>> = None;
    let result = held_crossings(&HELD_CROSSINGS, |held| {
        let world = collector_lifetime::world_lifetime::generation();
        if held.world != Some(world) || world == u64::MAX {
            held.answers.clear();
            held.world = (world != u64::MAX).then_some(world);
        }
        let result = capture_rivers_with_crossings_into(
            memory,
            base,
            countries,
            movers,
            route_arena,
            provinces,
            province_slot,
            province_pointers,
            parts.wanted,
            parts.pending,
            parts.rivers,
            parts.observed,
            parts.counts,
            |from, to| {
                if let Some(&answer) = held.answers.get(&(from, to)) {
                    return Ok(answer);
                }
                let begun = *ready.get_or_insert_with(|| parts.crossings.begin(memory, base, game));
                let answer =
                    begun.and_then(|()| parts.crossings.acquire_crossing(memory, from, to))?;
                if held.world.is_some()
                    && engine_allocation::advisory(|| {
                        engine_allocation::reserve_map(&mut held.answers, 1)
                    })
                    .is_ok()
                {
                    held.answers.insert((from, to), answer);
                }
                Ok(answer)
            },
        );
        held.charge.set(collector_primitives::raw_memory::Capacity {
            index_vectors: (held.answers.capacity() * (size_of::<((u32, u32), Option<u8>)>() + 1))
                as u64,
            ..Default::default()
        });
        result
    });
    if ready.is_some() {
        parts.crossings.clear();
    }
    match result {
        Ok(len) => Outcome::Rows { len },
        Err(reason) => {
            parts.rivers.clear();
            Outcome::Unavailable { reason }
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    reason = "shares crossing walk between serial and parallel river capture"
)]
fn capture_rivers_with_crossings_into(
    memory: &impl Memory,
    base: usize,
    countries: &Result<ccountry::Reader<'_, impl Memory>, &'static str>,
    movers: &[Mover],
    route_arena: &[Result<u32, &'static str>],
    provinces: &[(u32, Result<cprovince::MovementProvince, &'static str>)],
    province_slot: &[u32],
    province_pointers: &[usize],
    wanted: &mut Vec<(u32, u32)>,
    pending: &mut Vec<(u32, usize)>,
    rivers: &mut Vec<RiverRelationRow>,
    observed: &mut Vec<((u32, u32), Result<Option<i64>, &'static str>)>,
    counts: &mut [u64; 3],
    mut crossing: impl FnMut(u32, u32) -> Result<Option<u8>, &'static str>,
) -> Result<usize, &'static str> {
    wanted.clear();
    pending.clear();
    for mover in movers.iter().filter(|mover| mover.land == Ok(true)) {
        let Ok(route) = route(route_arena, mover) else {
            counts[2] += 1;
            continue;
        };
        let mut from_id = mover.location;
        for (index, &to_id) in route.iter().enumerate() {
            enum Edge {
                None,
                Ready((u32, u32)),
                Pending((u32, usize)),
            }
            let edge = (|| -> Result<Edge, &'static str> {
                let from_id = from_id?;
                let to_id = to_id?;
                if index != 0 && from_id == to_id {
                    return Ok(Edge::None);
                }
                let from = province_at(provinces, province_slot, from_id)?;
                let to_slot = province_slot_of(provinces, province_slot, to_id)
                    .ok_or("movement province unread")?;
                let to = provinces[to_slot].1?;
                if !from.land()? || !to.land()? {
                    return Ok(Edge::None);
                }
                match crossing(from_id, to_id)? {
                    Some(1..=3) => {
                        let owner = mover.owner.ok_or("division speed owner unreadable")?;
                        if is_unfetched(&to.controller) {
                            Ok(Edge::Pending((owner, to_slot)))
                        } else {
                            Ok(Edge::Ready((owner, to.controller?)))
                        }
                    }
                    _ => Ok(Edge::None),
                }
            })();
            match edge {
                Ok(Edge::Ready(key)) => {
                    reserve(wanted, 1)?;
                    wanted.push(key);
                }
                Ok(Edge::Pending(key)) => {
                    reserve(pending, 1)?;
                    pending.push(key);
                }
                Ok(Edge::None) => {}
                Err(_) => counts[2] += 1,
            }
            from_id = to_id;
        }
    }
    if !pending.is_empty() {
        let mut slots = Vec::new();
        reserve(&mut slots, pending.len())?;
        slots.extend(pending.iter().map(|&(_, slot)| slot));
        slots.sort_unstable();
        slots.dedup();
        let mut pointers = Vec::new();
        let mut fetched = Vec::new();
        reserve(&mut pointers, slots.len())?;
        reserve(&mut fetched, slots.len())?;
        pointers.extend(
            slots
                .iter()
                .map(|&slot| province_pointers.get(slot).copied().unwrap_or(0)),
        );
        fetched.resize(slots.len(), Err("movement province controller unreadable"));
        cprovince::movement_province_controllers_only(memory, &pointers, &mut fetched);
        for &(owner, slot) in pending.iter() {
            let at = slots
                .binary_search(&slot)
                .expect("pending river slot collected");
            match fetched[at] {
                Ok(controller) => {
                    reserve(wanted, 1)?;
                    wanted.push((owner, controller));
                }
                Err(_) => counts[2] += 1,
            }
        }
    }
    wanted.sort_unstable();
    wanted.dedup();
    reserve(rivers, wanted.len())?;
    reserve(observed, wanted.len())?;
    let mut owner: Option<(u32, Result<ccountry::Country, &'static str>)> = None;
    for &(raw, controller) in wanted.iter() {
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
                ccountry::river_crossing_against_with(memory, base, reader, country?, controller)
            });
        observed.push(((raw, controller), entry));
    }
    Ok(rivers.len() + observed.len())
}

#[allow(
    clippy::type_complexity,
    reason = "borrows disjoint transport buffers for shared serial and parallel helper"
)]
struct TransportParts<'a> {
    demand: &'a mut Vec<(u32, usize)>,
    pointers: &'a mut Vec<usize>,
    regions: &'a mut Vec<TransportRegion>,
    located: &'a mut Vec<(usize, u32)>,
    wanted: &'a mut Vec<(u32, usize)>,
    indices: &'a mut Vec<usize>,
    cache: &'a mut SpeedCache,
    transport: &'a mut Vec<OwnerTransportSpeedRow>,
    observed: &'a mut Vec<(
        (u32, u32),
        Result<engine_naval_history::cnaval_unit_transfer::SpeedInputs, &'static str>,
    )>,
    counts: &'a mut [u64; 3],
}

#[allow(
    clippy::too_many_arguments,
    reason = "shares transport speed capture between serial and parallel paths"
)]
fn capture_transport_into(
    memory: &impl Memory,
    base: usize,
    game: usize,
    movers: &[Mover],
    route_arena: &[Result<u32, &'static str>],
    provinces: &[(u32, Result<cprovince::MovementProvince, &'static str>)],
    province_slot: &[u32],
    parts: TransportParts<'_>,
) -> Outcome {
    let result = (|| -> Result<usize, &'static str> {
        parts.demand.clear();
        for mover in movers {
            let rows =
                append_transport_demand(parts.demand, route_arena, provinces, province_slot, mover);
            match rows {
                Ok(refused) => parts.counts[2] += refused,
                Err(ALLOCATION) => return Err(ALLOCATION),
                Err(_) => {
                    parts.counts[2] += 1;
                }
            }
        }
        parts.demand.sort_unstable();
        parts.demand.dedup();
        parts.pointers.clear();
        reserve(parts.pointers, parts.demand.len())?;
        parts
            .pointers
            .extend(parts.demand.iter().map(|&(_, region)| region));
        parts.pointers.sort_unstable();
        parts.pointers.dedup();
        parts.regions.clear();
        reserve(parts.regions, parts.pointers.len())?;
        parts.located.clear();
        reserve(parts.located, parts.pointers.len())?;
        let world = collector_lifetime::world_lifetime::token()
            .filter(|_| collector_observers::world_input_changes::map_content_stable());
        if let Some(world) = world {
            let key = (world, base);
            held_map(|held| {
                if held.regions_key != Some(key) {
                    held.regions.clear();
                    held.regions_key = Some(key);
                }
                let mut inserted = false;
                for &pointer in parts.pointers.iter() {
                    if let Some(&id) = held.regions.get(&pointer) {
                        parts.regions.push(TransportRegion { id, pointer });
                        parts.located.push((pointer, id));
                        continue;
                    }
                    match engine_native::cstrategic_region::id(memory, base, pointer) {
                        Ok(Some(id)) => {
                            if engine_allocation::advisory(|| {
                                engine_allocation::reserve_map(&mut held.regions, 1)
                            })
                            .is_ok()
                            {
                                held.regions.insert(pointer, id);
                                inserted = true;
                            }
                            parts.regions.push(TransportRegion { id, pointer });
                            parts.located.push((pointer, id));
                        }
                        Ok(None) | Err(_) => parts.counts[2] += 1,
                    }
                }
                if inserted {
                    held.update_charge();
                }
            });
        } else {
            for &pointer in parts.pointers.iter() {
                match engine_native::cstrategic_region::id(memory, base, pointer) {
                    Ok(Some(id)) => {
                        parts.regions.push(TransportRegion { id, pointer });
                        parts.located.push((pointer, id));
                    }
                    Ok(None) | Err(_) => parts.counts[2] += 1,
                }
            }
        }
        parts.regions.sort_unstable_by_key(|region| region.id);
        parts.regions.dedup_by_key(|region| region.id);
        parts.wanted.clear();
        reserve(parts.wanted, parts.demand.len())?;
        for &(owner, pointer) in parts.demand.iter() {
            let Some(id) = memo(parts.located, pointer) else {
                continue;
            };
            let index = parts
                .regions
                .binary_search_by_key(&id, |region| region.id)
                .map_err(|_| "division transport region unlocated")?;
            parts.wanted.push((owner, index));
        }
        parts.wanted.sort_unstable();
        parts.wanted.dedup();
        reserve(parts.transport, parts.wanted.len())?;
        reserve(parts.observed, parts.wanted.len())?;
        if parts.wanted.is_empty() {
            return Ok(0);
        }
        parts.cache.begin(parts.regions.len())?;
        parts.indices.clear();
        reserve(parts.indices, parts.regions.len())?;
        let mut first = 0;
        while first < parts.wanted.len() {
            let owner = parts.wanted[first].0;
            let end =
                first + parts.wanted[first..].partition_point(|&(known, _)| known == owner);
            parts.indices.clear();
            parts
                .indices
                .extend(parts.wanted[first..end].iter().map(|&(_, index)| index));
            first = end;
            engine_naval_history::cnaval_unit_transfer::movement_speeds_inputs(
                memory,
                base,
                game,
                owner,
                parts.regions,
                parts.indices,
                parts.cache,
                |region, speed| {
                    parts.observed.push(((owner, region.id), speed));
                },
            );
        }
        Ok(parts.transport.len() + parts.observed.len())
    })();
    match result {
        Ok(len) => Outcome::Rows { len },
        Err(reason) => {
            parts.transport.clear();
            Outcome::Unavailable { reason }
        }
    }
}

#[cfg(test)]
mod tests;
