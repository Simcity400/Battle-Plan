//! Per-division operands a replayer folds into the full ETA. The moving census
//! is assigned to stable handle partitions, one acquisition each with its own
//! packets and geometry memo: the composed speed graph once per division, the
//! edge constants once, and a thin route scan for the per-leg vectors.
use super::production::Sources;
use crate::{sampling::qpc, spooling::table, table_capture::grow_capacity};
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
    rows::{DivisionEtaInputsRow, TableId, TerrainSpeed, tables},
};
use std::{cell::RefCell, sync::Arc};
mod cadence;
pub(crate) use cadence::Store as CadenceStore;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::DivisionEtaInputs];
// The shared division census is charged to Divisions with the other consumers.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// One range per pool worker at most.
pub(crate) const MAX_RANGES: usize = 8;

/// A snapshot pays for itself from the second lookup on the same accumulator,
/// so the ceilings only keep one range's arena bounded. Above them the
/// unchanged direct reader answers every lookup.
const SNAPSHOT_LIMITS: SnapshotCapacity = SnapshotCapacity {
    nodes: 1 << 16,
    entries: 1 << 21,
};

fn bounded_snapshot_capacity(capacity: SnapshotCapacity) -> SnapshotCapacity {
    SnapshotCapacity {
        nodes: capacity.nodes.min(SNAPSHOT_LIMITS.nodes),
        entries: capacity.entries.min(SNAPSHOT_LIMITS.entries),
    }
}

fn grow_snapshot_capacity(capacity: &mut SnapshotCapacity, required: SnapshotCapacity) {
    let requested = bounded_snapshot_capacity(required);
    grow_capacity(&mut capacity.nodes, requested.nodes);
    grow_capacity(&mut capacity.entries, requested.entries);
    *capacity = bounded_snapshot_capacity(*capacity);
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
            slots: vec![None; rows],
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
        let share = |total: usize, floor: usize| (total / ranges).max(floor);
        let whole = cprovince::GeometryCapacities::default();
        Self {
            rows: share(8192, 512),
            terrain: share(65_536, 4096),
            legs: share(128 * 1024, 8192),
            geometry: cprovince::GeometryCapacities {
                provinces: share(whole.provinces, 1024),
                neighbour_entries: share(whole.neighbour_entries, 8192),
            },
            modifiers: SnapshotCapacity {
                nodes: share(8192, 1024),
                entries: share(262_144, 32_768),
            },
            ranges,
        }
    }
}

fn packet(c: &Capacities) -> carmy::EtaInputPacket {
    carmy::EtaInputPacket::new(c.rows, c.terrain, c.legs)
}

/// One stable partition of the census, acquired by one worker with its own
/// buffers and geometry memo.
pub struct RangeCaptured {
    pub(crate) cadence: Option<Box<CadenceStore>>,
    carried: Option<Vec<Arc<DivisionEtaInputsRow>>>,
    /// The latest full-census demand, so a forced full read fits.
    census: cadence::Census,
    packet: carmy::EtaInputPacket,
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
        Self {
            cadence: None,
            carried: None,
            census: cadence::Census::default(),
            packet: packet(c),
            geometry: cprovince::GeometryPacket::with_capacities(c.geometry),
            statics: cprovince::StaticGeometry::default(),
            modifiers: RefCell::new(SnapshotArena::new(bounded_snapshot_capacity(c.modifiers))),
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
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        self.carried.as_ref().map_or(0, |rows| {
            rows.capacity() * size_of::<Arc<DivisionEtaInputsRow>>()
        }) + self.packet.allocated_bytes()
            + self.geometry.allocated_bytes()
            + self.statics.allocated_bytes()
            + self.modifiers.borrow().allocated_bytes()
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
        self.geometry.grow(c.geometry);
        self.packet.reserve(c.rows, c.terrain, c.legs);
        self.modifiers
            .get_mut()
            .grow(bounded_snapshot_capacity(c.modifiers));
        self.air_defense.get_mut().reserve(c.rows);
    }

    /// One worker's whole job: acquire this range and report it.
    pub(crate) fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        sources: Sources<'_, DivisionSeed>,
        countries: Option<engine::ccountry::AuthenticatedRoster<'_>>,
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
            self.capture(
                memory,
                CaptureContext {
                    span: (0, selected.rows.len()),
                    ..context
                },
                selected,
                countries,
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
            let rows = self.project(context.hour).into_iter().zip(terrain);
            self.carried = Some(store.finish(context.hour, rows)?);
            self.census = store.census();
            Ok(())
        }));
        let result = match result {
            Ok(result) => result,
            Err(payload) => {
                store.invalidate();
                self.cadence = Some(store);
                std::panic::resume_unwind(payload);
            }
        };
        if let Err(reason) = result {
            store.invalidate();
            self.failure = Some(reason);
        }
        self.cadence = Some(store);
        CaptureReport {
            ticks: [qpc().saturating_sub(started).max(0) as u64],
        }
    }

    /// Finish an accepted worker failure without retrying the read. A denied
    /// range denies the table, exactly as a denied single job did.
    pub(crate) fn fail_owned_capture(&mut self, reason: &'static str) -> CaptureReport {
        self.reset();
        if let Some(store) = &mut self.cadence {
            store.invalidate();
        }
        self.failure = Some(reason);
        CaptureReport::default()
    }

    fn capture(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        sources: Sources<'_, DivisionSeed>,
        countries: Option<engine::ccountry::AuthenticatedRoster<'_>>,
    ) -> u64 {
        self.reset();
        self.carried = None;
        let rows = &sources.rows[..sources.outcome.len().min(sources.rows.len())];
        self.captured_current = true;
        let started = qpc();
        let span = context.span.0.min(rows.len())..context.span.1.min(rows.len());
        self.definitions.clear();
        let memory = &ModifierMemory {
            inner: memory,
            arena: &self.modifiers,
            definitions: &self.definitions,
            air_defense: &self.air_defense,
            countries: countries.map(|roster| (context.base, context.game_state, roster)),
        };
        memory.arena.borrow_mut().clear();
        let _ = self
            .geometry
            .begin(memory, context.base, context.game_state);
        carmy::capture_eta_inputs(
            memory,
            context.base,
            context.game_state,
            rows,
            span,
            sources.outcome,
            usize::MAX,
            usize::MAX,
            usize::MAX,
            &mut self.geometry,
            &mut self.statics,
            &mut self.packet,
        );
        self.required_modifiers = memory.arena.borrow().required_capacity();
        self.counters = self.packet.counters();
        let air_defense = self.air_defense.borrow();
        self.counters.air_defense_requests = air_defense.requests;
        self.counters.air_defense_hits = air_defense.hits;
        qpc().saturating_sub(started).max(0) as u64
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

    fn project(&self, sample_hour: i32) -> Vec<DivisionEtaInputsRow> {
        self.rows()
            .iter()
            .map(|r| {
                let terrain = self.packet.terrain();
                let legs = self.packet.legs();
                let terrain_speed = terrain[r.terrain_start..r.terrain_start + r.terrain_len]
                    .iter()
                    .map(|&(terrain, modifier)| TerrainSpeed { terrain, modifier })
                    .collect();
                let span = r.leg_start..r.leg_start + r.leg_len;
                let optional = |values: &[i64], present: bool| {
                    if present {
                        values[span.clone()].to_vec()
                    } else {
                        Vec::new()
                    }
                };
                DivisionEtaInputsRow {
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
                    leg_river_combined: optional(legs.river, r.any_river),
                    leg_transport_speed: optional(legs.transport, r.any_transport),
                    leg_rail_speed: optional(legs.rail, r.any_rail),
                }
            })
            .collect()
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
        let rows: Vec<_> = self
            .active()
            .iter()
            .flat_map(|range| range.carried.iter().flatten().map(Arc::as_ref))
            .collect();
        [table::<tables::DivisionEtaInputs, _>(
            self.outcomes()[0],
            &rows,
            |row| (*row).clone(),
        )]
    }
}

#[cfg(test)]
mod tests;
