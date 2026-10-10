//! Per-division operands a replayer folds into the full ETA. The moving census
//! is assigned to stable handle partitions, one acquisition each with its own
//! packets and geometry memo: the composed speed graph once per division, the
//! edge constants once, and a thin route scan for the per-leg vectors.
use super::production::Sources;
use crate::{sampling::qpc, spooling::table_iter, table_capture::grow_capacity};
use engine::allocation::{copy_vec, owned_vec};
use engine::{
    Memory, carmy,
    carmy::DivisionSeed,
    cprovince,
    memory::{RecordLayout, RecordSources},
    modifier_accumulator::{
        SNAPSHOT_ENTRY_CAPACITY, SNAPSHOT_NODE_CAPACITY, SnapshotArena, SnapshotCapacity, Value,
    },
    modifier_definition::PassDefinitions,
    walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{DivisionEtaInputsRow, Recorded, TableId, TerrainSpeed, tables},
};
use std::cell::RefCell;
use std::sync::Arc;
mod cadence;
mod fresh;
pub(crate) use cadence::Store as CadenceStore;
pub(crate) use fresh::Work as FreshWork;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::DivisionEtaInputs];
// The shared division census is charged to Divisions with the other consumers.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// One range per pool worker at most.
pub(crate) const MAX_RANGES: usize = 8;

fn grow_snapshot_capacity(capacity: &mut SnapshotCapacity, required: SnapshotCapacity) {
    grow_capacity(&mut capacity.nodes, required.nodes);
    grow_capacity(&mut capacity.entries, required.entries);
}

#[derive(Clone, Copy)]
struct AirDefenseEntry {
    arguments: [i64; 3],
    value: i64,
}

/// Only the deterministic integer series is reused across captures. The engine
/// acquires all three signed operands before consulting this memo, so neither a
/// hit nor a collision changes native reads or their first failure.
struct AirDefenseMemo {
    slots: Vec<Option<AirDefenseEntry>>,
    requests: u64,
    hits: u64,
}

impl AirDefenseMemo {
    fn with_capacity(rows: usize) -> Self {
        Self {
            // One bounded slot per range row; a vacant range allocates nothing.
            slots: owned_vec(engine::allocation::filled(rows, None)),
            requests: 0,
            hits: 0,
        }
    }

    fn reset_counts(&mut self) {
        self.requests = 0;
        self.hits = 0;
    }

    fn reserve(&mut self, rows: usize) {
        if self.slots.len() < rows {
            let additional = rows - self.slots.len();
            if engine::allocation::reserve(&mut self.slots, additional).is_err() {
                return;
            }
            self.slots.resize(rows, None);
            // Changing the modulus changes slot placement. Discard old entries
            // off capture rather than rehashing with another allocation.
            self.slots.fill(None);
        }
    }

    fn allocated_bytes(&self) -> usize {
        self.slots.capacity() * std::mem::size_of::<Option<AirDefenseEntry>>()
    }

    fn value(
        &mut self,
        defense: i64,
        denominator: i64,
        reduction: i64,
    ) -> Result<i64, &'static str> {
        self.requests = self.requests.saturating_add(1);
        let arguments = [defense, denominator, reduction];
        let slot = if self.slots.is_empty() {
            None
        } else {
            let hash = arguments.iter().fold(0_u64, |hash, &argument| {
                (hash.rotate_left(21) ^ argument as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
            });
            let index = ((hash ^ (hash >> 32)) as usize) % self.slots.len();
            if let Some(entry) = self.slots[index]
                && entry.arguments == arguments
            {
                self.hits = self.hits.saturating_add(1);
                return Ok(entry.value);
            }
            Some(index)
        };
        let value = engine::csub_unit_definition::movement_air_defense_value(
            defense,
            denominator,
            reduction,
        )?;
        if let Some(index) = slot {
            self.slots[index] = Some(AirDefenseEntry { arguments, value });
        }
        Ok(value)
    }
}

/// One walk's raw modifier accumulators. Every division of the same country
/// asks the same accumulator for the same movement keys, and the native lookup
/// scans the whole array for each one; the first lookup reads exactly what it
/// read and the rest are answered from those bytes. Only raw arrays and each
/// key's static metadata flags are shared: graph authentication and every
/// other read stay with the source, and both are cleared before each capture
/// to acquire current values.
struct ModifierMemory<'a, M> {
    inner: &'a M,
    arena: &'a RefCell<SnapshotArena>,
    definitions: &'a PassDefinitions,
    air_defense: &'a RefCell<AirDefenseMemo>,
    countries: Option<(usize, usize, engine::ccountry::AuthenticatedRoster<'a>)>,
    reserved: Option<(usize, usize, engine::ccountry::ReservedCountryObservation)>,
}

impl<M: Memory> Memory for ModifierMemory<'_, M> {
    #[inline]
    fn read_scalar<T: engine::memory::Raw>(&self, address: usize) -> Option<T> {
        self.inner.read_scalar::<T>(address)
    }

    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.inner.copy(address, destination)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.inner.copy_scattered(addresses, destinations)
    }
    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.inner
            .copy_scattered_prefetched(addresses, destinations)
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.inner.copy_records(sources, layout, destination)
    }
    fn authenticated_countries(
        &self,
        base: usize,
        state: usize,
    ) -> Option<engine::ccountry::AuthenticatedRoster<'_>> {
        self.countries
            .filter(|(module, game, _)| (*module, *game) == (base, state))
            .map(|(_, _, roster)| roster)
            .or_else(|| self.inner.authenticated_countries(base, state))
    }

    fn authenticated_reserved_country(
        &self,
        base: usize,
        state: usize,
    ) -> Option<Result<engine::ccountry::Country, &'static str>> {
        self.reserved
            .filter(|(module, game, _)| (*module, *game) == (base, state))
            .map(|(_, _, root)| root.result())
            .or_else(|| self.inner.authenticated_reserved_country(base, state))
    }

    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        state: usize,
    ) -> Option<engine::ccountry::ReservedCountryObservation> {
        self.reserved
            .filter(|(module, game, _)| (*module, *game) == (base, state))
            .map(|(_, _, root)| root)
            .or_else(|| {
                self.inner
                    .authenticated_reserved_country_observation(base, state)
            })
    }
    fn modifier_definitions(&self) -> Option<&PassDefinitions> {
        Some(self.definitions)
    }
    fn modifier_accumulator_value(
        &self,
        address: usize,
        key: u32,
    ) -> Option<Result<Value, &'static str>> {
        let mut arena = self.arena.borrow_mut();
        match arena.capture_at(self.inner, address, key) {
            Ok(observation) => Some(arena.value(observation)),
            // Out of arena storage: the caller reads the array directly.
            Err(SNAPSHOT_NODE_CAPACITY | SNAPSHOT_ENTRY_CAPACITY) => None,
            Err(reason) => Some(Err(reason)),
        }
    }
    fn movement_air_defense_value(
        &self,
        defense: i64,
        denominator: i64,
        reduction: i64,
    ) -> Option<Result<i64, &'static str>> {
        Some(
            self.air_defense
                .borrow_mut()
                .value(defense, denominator, reduction),
        )
    }
}

/// Ranges to dispatch. The pool's worker count is fixed for the session.
fn range_count() -> usize {
    static COUNT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    // Each range repeats country, rail and geometry acquisition. Two ranges
    // retain overlap with other jobs without duplicating that work per core;
    // a single range reduced total work but increased measured callback waits.
    *COUNT.get_or_init(|| crate::acquisition::recommended_worker_count().clamp(1, 2))
}

/// Everything the walk needs that the callback cannot lend a worker by reference.
#[derive(Clone, Copy)]
pub(crate) struct CaptureContext {
    pub hour: i32,
    pub force_full: bool,
    pub base: usize,
    pub game_state: usize,
    /// Census bounds; workers admit only their stable handle partition.
    pub span: (usize, usize),
    /// Worker index and fixed worker count for this attachment.
    pub partition: (usize, usize),
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CaptureReport {
    pub ticks: [u64; N],
}

pub(crate) type Counters = carmy::EtaInputCounters;

/// Every worker reserves the largest observed per-partition demand.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    terrain: usize,
    fresh_terrain: usize,
    legs: usize,
    geometry: cprovince::GeometryCapacities,
    modifiers: SnapshotCapacity,
    ranges: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self::with_ranges(range_count())
    }
}
impl Capacities {
    fn with_ranges(ranges: usize) -> Self {
        let ranges = ranges.clamp(1, MAX_RANGES);
        Self {
            rows: 0,
            terrain: 0,
            fresh_terrain: 0,
            legs: 0,
            geometry: cprovince::GeometryCapacities {
                provinces: 0,
                neighbour_entries: 0,
            },
            modifiers: SnapshotCapacity {
                nodes: 0,
                entries: 0,
            },
            ranges,
        }
    }

    /// Live map sizing is observed before hooks and between callbacks. Every
    /// range needs the whole province domain; its terrain operands use the
    /// completed terrain count, not one entry for every province and unit.
    pub(crate) fn raise_for_fresh_map(
        &mut self,
        provinces: usize,
        terrain_count: usize,
    ) -> std::io::Result<()> {
        let terrain = engine::allocation::mul(self.rows, terrain_count)?;
        self.geometry.provinces = self.geometry.provinces.max(provinces);
        self.fresh_terrain = self.fresh_terrain.max(terrain);
        Ok(())
    }
}

fn packet(c: &Capacities) -> carmy::EtaInputPacket {
    carmy::EtaInputPacket::new(c.rows, c.terrain, c.legs)
        .unwrap_or_else(|_| carmy::EtaInputPacket::vacant())
}

/// One stable partition of the census, acquired by one worker with its own
/// buffers and geometry memo.
pub struct RangeCaptured {
    pub(crate) cadence: Option<Box<CadenceStore>>,
    carried: Option<Vec<Arc<DivisionEtaInputsRow>>>,
    /// The latest full-census demand, so a forced full read fits.
    census: cadence::Census,
    packet: carmy::EtaInputPacket,
    fresh: carmy::FreshCounterEtaPacket,
    fresh_hour: Option<i32>,
    fresh_failure: Option<&'static str>,
    fresh_terrain_demand: usize,
    /// Completed immutable LAND terrain domain, valid only under statics' key.
    land_terrains: Vec<cprovince::StaticProvince>,
    land_len: Option<usize>,
    required_provinces: usize,
    geometry: cprovince::GeometryPacket,
    /// Session-static map data, re-keyed from current roots before each capture.
    statics: cprovince::StaticGeometry,
    /// Walk-local raw modifier accumulators, cleared before each capture.
    modifiers: RefCell<SnapshotArena>,
    /// Walk-local modifier metadata flags, cleared before each capture.
    definitions: PassDefinitions,
    /// Pure results keyed by every signed argument, retained across captures.
    air_defense: RefCell<AirDefenseMemo>,
    /// This capture's modifier demand, retained for off-thread growth.
    required_modifiers: SnapshotCapacity,
    captured_current: bool,
    counters: Counters,
    /// An accepted worker that panicked or stopped. The table stays unavailable
    /// for the hour rather than retrying a live read.
    failure: Option<&'static str>,
}

impl RangeCaptured {
    fn with_capacities(c: &Capacities) -> Self {
        let mut statics = cprovince::StaticGeometry::default();
        let _ = statics.reserve_provinces(c.geometry.provinces);
        Self {
            cadence: None,
            carried: None,
            census: cadence::Census::default(),
            packet: packet(c),
            fresh: carmy::FreshCounterEtaPacket::try_new(c.rows, c.fresh_terrain)
                .unwrap_or_else(|_| carmy::FreshCounterEtaPacket::vacant()),
            fresh_hour: None,
            fresh_failure: None,
            fresh_terrain_demand: 0,
            land_terrains: owned_vec(engine::allocation::filled(
                c.geometry.provinces,
                fresh::EMPTY_PROVINCE,
            )),
            land_len: None,
            required_provinces: 0,
            geometry: cprovince::GeometryPacket::try_with_capacities(c.geometry)
                .unwrap_or_default(),
            statics,
            modifiers: RefCell::new(SnapshotArena::try_new(c.modifiers).unwrap_or_default()),
            definitions: PassDefinitions::default(),
            air_defense: RefCell::new(AirDefenseMemo::with_capacity(c.rows)),
            required_modifiers: SnapshotCapacity::default(),
            captured_current: false,
            counters: Counters::default(),
            failure: None,
        }
    }

    /// The placeholder the callback holds while a worker owns the buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            terrain: 0,
            fresh_terrain: 0,
            legs: 0,
            geometry: cprovince::GeometryCapacities {
                provinces: 0,
                neighbour_entries: 0,
            },
            modifiers: SnapshotCapacity::default(),
            ranges: 1,
        })
    }

    fn reset(&mut self) {
        // Retire carried row allocations on the acquisition worker, not here:
        // reset also runs on the hourly game callback.
        self.air_defense.get_mut().reset_counts();
        self.required_modifiers = SnapshotCapacity::default();
        self.captured_current = false;
        self.counters = Counters::default();
        self.failure = None;
        self.fresh.clear();
        self.fresh_hour = None;
        self.fresh_failure = None;
        self.fresh_terrain_demand = 0;
        self.required_provinces = 0;
        self.packet.clear_fresh_observations();
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        self.carried.as_ref().map_or(0, |rows| {
            rows.capacity() * size_of::<Arc<DivisionEtaInputsRow>>()
        }) + self.packet.allocated_bytes()
            + self.fresh.allocated_bytes()
            + self.land_terrains.capacity() * size_of::<cprovince::StaticProvince>()
            + self.geometry.allocated_bytes()
            + self.statics.allocated_bytes()
            + self.modifiers.borrow().allocated_bytes()
            + self.definitions.allocated_bytes()
            + self.air_defense.borrow().allocated_bytes()
    }

    /// Fold this range's shortfalls into the shared per-range capacities.
    fn grow(&self, c: &mut Capacities) {
        let walk = self.packet.outcome();
        match walk.outcome {
            Outcome::Rows { len } if len > 0 && len >= c.rows.saturating_sub(c.rows / 4) => {
                let required = c.rows.saturating_add(1);
                grow_capacity(&mut c.rows, required);
            }
            Outcome::Truncated { len, unread_rows } => {
                grow_capacity(&mut c.rows, len.saturating_add(unread_rows as usize));
            }
            _ => {}
        }
        grow_capacity(&mut c.terrain, walk.required_terrain);
        grow_capacity(&mut c.legs, walk.required_legs);
        // Unchanged divisions are not read on most hours, but a load, gap or
        // failure re-reads every one of them.
        grow_capacity(&mut c.rows, self.census.rows);
        grow_capacity(&mut c.terrain, self.census.terrain.saturating_add(1));
        grow_capacity(&mut c.legs, self.census.legs);
        let fresh = self.fresh.outcome();
        grow_capacity(&mut c.rows, fresh.required_rows);
        let Ok(terrain) = self.required_fresh_terrain() else {
            return;
        };
        grow_capacity(&mut c.fresh_terrain, terrain);
        grow_capacity(&mut c.geometry.provinces, self.required_provinces);
        let required = self.geometry.required_capacities();
        grow_capacity(&mut c.geometry.provinces, required.provinces);
        grow_capacity(
            &mut c.geometry.neighbour_entries,
            required.neighbour_entries,
        );
        grow_snapshot_capacity(&mut c.modifiers, self.required_modifiers);
    }

    /// Apply grown capacities. Allocation happens outside acquisition.
    fn reserve(&mut self, c: &Capacities) {
        if self.geometry.grow(c.geometry).is_err()
            || self.packet.reserve(c.rows, c.terrain, c.legs).is_err()
            || self.fresh.reserve(c.rows, c.fresh_terrain).is_err()
            || self
                .statics
                .reserve_provinces(c.geometry.provinces)
                .is_err()
            || engine::allocation::resize(
                &mut self.land_terrains,
                c.geometry.provinces,
                fresh::EMPTY_PROVINCE,
            )
            .is_err()
            || self.modifiers.get_mut().try_grow(c.modifiers).is_err()
        {
            return;
        }
        self.air_defense.get_mut().reserve(c.rows);
    }

    /// One worker's whole job: acquire this range and report it.
    pub(crate) fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        sources: Sources<'_, DivisionSeed>,
        countries: Option<engine::ccountry::AuthenticatedRoster<'_>>,
        reserved: Option<engine::ccountry::ReservedCountryObservation>,
    ) -> CaptureReport {
        let started = qpc();
        self.carried = None;
        let mut store = self.cadence.take().expect("ETA cadence loan");
        self.reset();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if sources.outcome.len() != sources.rows.len()
                || context.span.0 > context.span.1
                || context.span.1 > sources.rows.len()
            {
                return Err("division ETA witness census range invalid");
            }
            store.select(
                memory,
                context.base,
                context.hour,
                context.force_full,
                context.partition,
                &sources.rows[context.span.0..context.span.1],
                sources.outcome,
            )?;
            let selected = Sources {
                rows: &store.selected,
                outcome: Outcome::Rows {
                    len: store.selected.len(),
                },
            };
            self.capture_mode(
                memory, context, selected, sources, countries, reserved, true,
            );
            match self.outcome() {
                Outcome::Rows { .. } => {}
                Outcome::Unavailable { reason } => return Err(reason),
                Outcome::Truncated { .. } => {
                    return Err("division ETA selected operands truncated");
                }
            }
            if !self.spans_valid() {
                return Err("division ETA input spans invalid");
            }
            let terrain = self.rows().iter().map(|row| row.terrain_len);
            let rows = self
                .project(context.hour)
                .map_err(|_| "collector allocation failed")?
                .into_iter()
                .zip(terrain);
            self.carried = Some(store.finish(context.hour, rows)?);
            self.census = store.census();
            Ok(())
        }));
        let result = match result {
            Ok(result) => result,
            Err(payload) => {
                store.invalidate();
                self.packet.clear_fresh_observations();
                self.land_len = None;
                self.cadence = Some(store);
                std::panic::resume_unwind(payload);
            }
        };
        if let Err(reason) = result {
            store.invalidate();
            self.failure = Some(reason);
            self.packet.clear_fresh_observations();
        }
        self.cadence = Some(store);
        CaptureReport {
            ticks: [qpc().saturating_sub(started).max(0) as u64],
        }
    }

    /// Finish an accepted worker failure without retrying the read. A denied
    /// range denies the table, exactly as a denied single job did.
    pub(crate) fn fail_owned_capture(&mut self, reason: &'static str) -> CaptureReport {
        // An interrupted acquisition can already have counted a short arena.
        // Keep those demands for growth while retiring every pointer context.
        self.packet.clear_fresh_observations();
        self.air_defense.get_mut().reset_counts();
        self.required_modifiers = self.modifiers.borrow().required_capacity();
        self.captured_current = false;
        self.counters = Counters::default();
        self.fresh_hour = None;
        self.fresh_failure = Some(reason);
        self.land_len = None;
        if let Some(store) = &mut self.cadence {
            store.invalidate();
        }
        self.failure = Some(reason);
        CaptureReport::default()
    }

    #[cfg(test)]
    fn capture(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        sources: Sources<'_, DivisionSeed>,
        fresh_sources: Sources<'_, DivisionSeed>,
        countries: Option<engine::ccountry::AuthenticatedRoster<'_>>,
        reserved: Option<engine::ccountry::ReservedCountryObservation>,
    ) -> u64 {
        self.capture_mode(
            memory,
            context,
            sources,
            fresh_sources,
            countries,
            reserved,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn capture_mode(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        sources: Sources<'_, DivisionSeed>,
        fresh_sources: Sources<'_, DivisionSeed>,
        countries: Option<engine::ccountry::AuthenticatedRoster<'_>>,
        reserved: Option<engine::ccountry::ReservedCountryObservation>,
        growing: bool,
    ) -> u64 {
        self.reset();
        self.carried = None;
        let rows = &sources.rows[..sources.outcome.len().min(sources.rows.len())];
        self.captured_current = true;
        let started = qpc();
        self.definitions.clear();
        if growing {
            self.air_defense.get_mut().reserve(rows.len());
        }
        let memory = &ModifierMemory {
            inner: memory,
            arena: &self.modifiers,
            definitions: &self.definitions,
            air_defense: &self.air_defense,
            countries: countries.map(|roster| (context.base, context.game_state, roster)),
            reserved: reserved
                .or_else(|| {
                    memory.authenticated_reserved_country_observation(
                        context.base,
                        context.game_state,
                    )
                })
                .map(|root| (context.base, context.game_state, root)),
        };
        memory.arena.borrow_mut().clear();
        if growing {
            memory.arena.borrow_mut().enable_worker_growth();
            self.geometry.enable_growth();
            self.packet.enable_growth();
            self.fresh.enable_growth();
        }
        if let Err(reason) = self
            .geometry
            .begin(memory, context.base, context.game_state)
            && growing
        {
            self.failure = Some(reason);
            self.fresh_failure = Some(reason);
            return qpc().saturating_sub(started).max(0) as u64;
        }
        carmy::capture_eta_inputs(
            memory,
            context.base,
            context.game_state,
            rows,
            0..rows.len(),
            sources.outcome,
            usize::MAX,
            usize::MAX,
            usize::MAX,
            &mut self.geometry,
            &mut self.statics,
            &mut self.packet,
        );
        self.counters = self.packet.counters();
        {
            let air_defense = self.air_defense.borrow();
            self.counters.air_defense_requests = air_defense.requests;
            self.counters.air_defense_hits = air_defense.hits;
        }
        // begin above rekeys statics exactly once. A warm complete roster
        // belongs to that same immutable map key, never to an earlier load or
        // to a failed root observation.
        let provinces = self.statics.province_count();
        if provinces.is_err() || self.statics.counters().cleared != 0 {
            self.land_len = None;
        }
        let geometry_provinces = self.geometry.province_count();
        self.required_provinces = geometry_provinces.unwrap_or(0).max(provinces.unwrap_or(0));
        // The worker retains both begun roots; no sizing read is repeated.
        let required_land = self.required_provinces.max(self.land_terrains.len());
        if growing
            && (self
                .statics
                .reserve_provinces(self.required_provinces)
                .is_err()
                || engine::allocation::resize(
                    &mut self.land_terrains,
                    required_land,
                    fresh::EMPTY_PROVINCE,
                )
                .is_err())
        {
            self.failure = Some("division ETA allocation failed");
            self.fresh_failure = self.failure;
            return qpc().saturating_sub(started).max(0) as u64;
        }
        let land = if let Outcome::Unavailable { reason } = self.packet.outcome().outcome {
            // A failed mandatory prefix cannot publish this suffix. Keep
            // known roster/count demand, but do not retry a failed province
            // read through cold terrain acquisition. The fresh reader counts
            // source-header demand and stops before all graph getters too.
            Err(reason)
        } else {
            let result = match (provinces, geometry_provinces) {
                (Err(reason), _) | (_, Err(reason)) => Err(reason),
                (Ok(statics), Ok(geometry)) if statics != geometry => {
                    Err("fresh counter terrain province census differs")
                }
                (Ok(_), Ok(_)) => match self.land_len {
                    Some(len) => Ok(len),
                    None => carmy::capture_counter_land_terrains(
                        memory,
                        context.base,
                        context.game_state,
                        &mut self.geometry,
                        &mut self.statics,
                        &mut self.land_terrains,
                    ),
                },
            };
            self.land_len = result.ok();
            result
        };
        let land = land.and_then(|len| {
            self.land_terrains
                .get(..len)
                .ok_or("fresh counter LAND roster span invalid")
        });
        carmy::capture_fresh_counter_eta(
            memory,
            context.base,
            context.game_state,
            context.hour,
            fresh_sources.rows,
            context.span.0..context.span.1,
            context.partition,
            fresh_sources.outcome,
            land,
            &mut self.geometry,
            &mut self.packet,
            &mut self.fresh,
        );
        self.fresh_hour = Some(context.hour);
        self.fresh_failure = self.fresh.sort_rows_by_division().err();
        match self.required_fresh_terrain() {
            Ok(demand) => self.fresh_terrain_demand = demand,
            Err(_) => {
                self.fresh_failure = self
                    .fresh_failure
                    .or(Some("fresh counter ETA terrain demand overflow"));
            }
        }
        self.required_modifiers = memory.arena.borrow().required_capacity();
        qpc().saturating_sub(started).max(0) as u64
    }

    fn required_fresh_terrain(&self) -> std::io::Result<usize> {
        let walk = self.fresh.outcome();
        let demand = walk.required_terrain.max(self.fresh_terrain_demand);
        if walk.required_rows > self.fresh.rows().len()
            && let Some(kinds) = self.land_len
        {
            // Skipped row slots cannot report their normal-domain demand.
            // Bound them by the completed terrain domain without rereading
            // their native graphs or copying their routes.
            Ok(demand.max(engine::allocation::mul(walk.required_rows, kinds)?))
        } else {
            Ok(demand)
        }
    }

    fn outcome(&self) -> Outcome {
        if let Some(reason) = self.failure {
            Outcome::Unavailable { reason }
        } else if !self.captured_current {
            Outcome::Unavailable {
                reason: "division ETA input range was not acquired",
            }
        } else if let Some(rows) = &self.carried {
            Outcome::Rows { len: rows.len() }
        } else {
            self.packet.outcome().outcome
        }
    }

    /// Rows this range admitted, bounded by its own outcome.
    fn rows(&self) -> &[carmy::EtaInputRow] {
        let rows = self.packet.rows();
        &rows[..self.outcome().len().min(rows.len())]
    }

    fn project(&self, sample_hour: i32) -> std::io::Result<Vec<DivisionEtaInputsRow>> {
        let mut rows = engine::allocation::vec(self.rows().len())?;
        for r in self.rows() {
            let terrain = self.packet.terrain();
            let legs = self.packet.legs();
            let terrain_speed = engine::allocation::collect_vec(
                terrain[r.terrain_start..r.terrain_start + r.terrain_len]
                    .iter()
                    .map(|&(terrain, modifier)| TerrainSpeed { terrain, modifier }),
            )?;
            let span = r.leg_start..r.leg_start + r.leg_len;
            let optional = |values: &[i64], present: bool| {
                if present {
                    copy_vec(&values[span.start..span.end])
                } else {
                    Ok(Vec::new())
                }
            };
            rows.push(DivisionEtaInputsRow {
                division: r.division,
                sample_hour,
                speed_modifier: r.speed_modifier,
                speed_base: r.speed_base,
                speed_org: r.speed_org,
                speed_fuel: r.speed_fuel,
                current_speed: r.current_speed,
                terrain_speed,
                fort_penalty: r.fort_penalty,
                snow_penalty: r.snow_penalty,
                river_adjustment: r.river_adjustment,
                river_leader: r.river_leader,
                class: r.class,
                rail_country_modifier: r.rail_country_modifier,
                rail_state_modifier: r.rail_state_modifier,
                rail_friendly_state_modifier: r.rail_friendly_state_modifier,
                leg_river_combined: optional(legs.river, r.any_river)?,
                leg_transport_speed: optional(legs.transport, r.any_transport)?,
                leg_rail_speed: optional(legs.rail, r.any_rail)?,
            });
        }
        Ok(rows)
    }

    /// True when every row's terrain and leg span lies inside this range's arenas.
    fn spans_valid(&self) -> bool {
        let terrain = self.packet.terrain().len();
        let legs = self.packet.leg_count();
        let arrays = self.packet.legs();
        if arrays.river.len() != legs || arrays.transport.len() != legs || arrays.rail.len() != legs
        {
            return false;
        }
        let span = |start: usize, len: usize, total: usize| {
            start.checked_add(len).is_some_and(|end| end <= total)
        };
        self.rows().iter().all(|r| {
            span(r.terrain_start, r.terrain_len, terrain) && span(r.leg_start, r.leg_len, legs)
        })
    }
}

pub struct Captured {
    ranges: Vec<RangeCaptured>,
    /// Census bounds, with one stable handle partition per dispatched worker.
    plan: [(usize, usize); MAX_RANGES],
    planned: usize,
}

impl Captured {
    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            ranges: (0..c.ranges.clamp(1, MAX_RANGES))
                .map(|_| RangeCaptured::with_capacities(c))
                .collect(),
            plan: [(0, 0); MAX_RANGES],
            planned: 0,
        }
    }

    pub fn reset(&mut self) {
        for range in &mut self.ranges {
            range.reset();
        }
        self.plan = [(0, 0); MAX_RANGES];
        self.planned = 0;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.ranges.iter().map(RangeCaptured::allocated_bytes).sum()
    }

    pub fn grow(&mut self, c: &mut Capacities) {
        for range in &self.ranges {
            range.grow(c);
        }
        c.ranges = c.ranges.clamp(1, MAX_RANGES);
        while self.ranges.len() < c.ranges {
            self.ranges.push(RangeCaptured::with_capacities(c));
        }
        self.ranges.truncate(c.ranges);
        for range in &mut self.ranges {
            range.reserve(c);
        }
    }

    /// Stable ownership keeps unrelated arrivals and reroutes from refreshing
    /// another division's cache. Workers filter the census by full handle.
    pub(crate) fn plan(&mut self, sources: &Sources<'_, DivisionSeed>) -> usize {
        let count = sources.outcome.len().min(sources.rows.len());
        self.plan = [(0, count); MAX_RANGES];
        self.planned = self.ranges.len();
        self.planned
    }

    /// Lend one planned range's buffers and its span to the pool.
    pub(crate) fn take_range(
        &mut self,
        range: usize,
    ) -> (RangeCaptured, (usize, usize), (usize, usize)) {
        (
            std::mem::replace(&mut self.ranges[range], RangeCaptured::vacant()),
            self.plan[range],
            (range, self.planned),
        )
    }

    pub(crate) fn restore_range(&mut self, range: usize, captured: RangeCaptured) {
        self.ranges[range] = captured;
    }

    pub(crate) fn fail_missing(&mut self) {
        self.reset();
        self.ranges[0].fail_owned_capture("division ETA input acquisition not supplied");
    }

    /// This hour's dispatched ranges, in stable worker order.
    fn active(&self) -> &[RangeCaptured] {
        &self.ranges[..self.planned.max(1)]
    }

    /// This hour's attribution, summed over every acquired range.
    pub(crate) fn counters(&self) -> Counters {
        let mut counters = Counters::default();
        for range in self.active() {
            counters.add(range.counters);
        }
        counters
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        let (mut len, mut unread) = (0_usize, 0_u32);
        for range in self.active() {
            match range.outcome() {
                Outcome::Unavailable { reason } => return [Outcome::Unavailable { reason }],
                Outcome::Rows { len: rows } => len += rows,
                Outcome::Truncated {
                    len: rows,
                    unread_rows,
                } => {
                    len += rows;
                    unread = unread.saturating_add(unread_rows);
                }
            }
        }
        [if unread == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: unread,
            }
        }]
    }

    pub fn tables(&self) -> [TableView; N] {
        // A range publishes rows only after a complete acquisition; any other
        // range state makes the outcome unavailable.
        [table_iter::<tables::DivisionEtaInputs, _>(
            self.outcomes()[0],
            self.active()
                .iter()
                .flat_map(|range| range.carried.iter().flatten().map(|row| row.as_ref())),
            copy_row,
        )]
    }
}

fn copy_row(row: &DivisionEtaInputsRow) -> DivisionEtaInputsRow {
    DivisionEtaInputsRow {
        division: row.division,
        sample_hour: row.sample_hour,
        speed_modifier: row.speed_modifier,
        speed_base: row.speed_base,
        speed_org: row.speed_org,
        speed_fuel: row.speed_fuel,
        current_speed: row.current_speed,
        terrain_speed: owned_vec(copy_vec(&row.terrain_speed)),
        fort_penalty: row.fort_penalty,
        snow_penalty: row.snow_penalty,
        river_adjustment: row.river_adjustment,
        river_leader: row.river_leader,
        class: row.class,
        rail_country_modifier: row.rail_country_modifier,
        rail_state_modifier: row.rail_state_modifier,
        rail_friendly_state_modifier: row.rail_friendly_state_modifier,
        leg_river_combined: owned_vec(copy_vec(&row.leg_river_combined)),
        leg_transport_speed: owned_vec(copy_vec(&row.leg_transport_speed)),
        leg_rail_speed: owned_vec(copy_vec(&row.leg_rail_speed)),
    }
}

pub(crate) struct FreshProfile<'a> {
    row: &'a carmy::FreshCounterEtaRow,
    terrain: &'a [(i32, i64)],
}

impl FreshProfile<'_> {
    pub(crate) fn project(&self) -> Recorded<Option<replay::rows::FreshCounterEta>> {
        fresh::project(self.row, self.terrain, self.row.sample_hour)
    }

    // Comparison does not build or copy a terrain Vec. It deliberately mirrors
    // the established project() refusal law for an invalid normal span.
    pub(crate) fn matches(
        &self,
        previous: &Recorded<Option<replay::rows::FreshCounterEta>>,
    ) -> bool {
        let Recorded::Value(Some(previous)) = previous else {
            return false;
        };
        if previous.sample_hour != self.row.sample_hour
            || previous.current_speed != self.row.current_speed.ok().flatten()
            || previous.rail != self.row.rail.ok().flatten()
        {
            return false;
        }
        let normal = self.row.normal.ok().flatten().and_then(|normal| {
            let end = normal.terrain_start.checked_add(normal.terrain_len)?;
            self.terrain
                .get(normal.terrain_start..end)
                .map(|terrain| (normal, terrain))
        });
        match (normal, &previous.normal) {
            (None, None) => true,
            (Some((normal, terrain)), Some(previous)) => {
                previous.speed_modifier == normal.prepared.modifier
                    && previous.speed_base == normal.prepared.base_speed
                    && previous.speed_org == normal.prepared.organisation
                    && previous.speed_fuel == normal.prepared.fuel
                    && previous.fort_penalty == normal.fort_penalty
                    && previous.snow_penalty == normal.snow_penalty
                    && previous.river_adjustment == normal.river_adjustment
                    && previous.river_leader == normal.river_leader
                    && previous.terrain_speed.len() == terrain.len()
                    && previous.terrain_speed.iter().zip(terrain).all(
                        |(previous, &(terrain, modifier))| {
                            previous.terrain == terrain && previous.modifier == modifier
                        },
                    )
            }
            _ => false,
        }
    }
}

impl Captured {
    pub(crate) fn fresh_profile(
        &self,
        division: u64,
        expected_hour: i32,
    ) -> Option<FreshProfile<'_>> {
        if self.planned == 0 || division == 0 {
            return None;
        }
        // Same full-handle partition rule as acquisition; no probe of failed
        // sibling partitions or retry through a later range.
        let partition = ((division ^ (division >> 32)) % self.planned as u64) as usize;
        let range = self.active().get(partition)?;
        let hour = range.fresh_hour?;
        if hour != expected_hour {
            return None;
        }
        if range.failure.is_some()
            || range.fresh_failure.is_some()
            || matches!(range.fresh.outcome().outcome, Outcome::Unavailable { .. })
        {
            return None;
        }
        let index = range
            .fresh
            .rows()
            .binary_search_by_key(&division, |row| row.division)
            .ok()?;
        let row = &range.fresh.rows()[index];
        (row.sample_hour == hour).then_some(FreshProfile {
            row,
            terrain: range.fresh.terrain(),
        })
    }
}

#[cfg(test)]
pub(crate) use tests::{allocation_failure_route_exercise, fresh_transfer_profile_fixture};

#[cfg(test)]
mod tests;
