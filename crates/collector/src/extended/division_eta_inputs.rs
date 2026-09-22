//! Per-division operands a replayer folds into the full ETA. The moving census
//! is cut into contiguous seed ranges, one owned acquisition each with its own
//! packets and geometry memo: the composed speed graph once per division, the
//! edge constants once, and a thin route scan for the per-leg vectors.
use super::production::Sources;
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::grow_capacity,
};
use engine::{
    Memory, carmy,
    carmy::DivisionSeed,
    cprovince,
    memory::{RecordLayout, RecordSources},
    modifier_accumulator::{
        SNAPSHOT_ENTRY_CAPACITY, SNAPSHOT_NODE_CAPACITY, SnapshotArena, SnapshotCapacity, Value,
    },
    walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{DivisionEtaInputsRow, TableId, tables},
};
use std::cell::RefCell;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::DivisionEtaInputs];
// The shared division census is charged to Divisions with the other consumers.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// One range per pool worker at most.
pub(crate) const MAX_RANGES: usize = 8;
/// Below this the split costs more than it saves; one range takes everything.
const MIN_SPLIT_WEIGHT: usize = 256;

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
/// read and the rest are answered from those bytes. Only raw arrays are shared:
/// metadata, graph authentication and every other read stay with the source,
/// and the arena is cleared before each capture to acquire current values.
struct ModifierMemory<'a, M> {
    inner: &'a M,
    arena: &'a RefCell<SnapshotArena>,
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
    pub base: usize,
    pub game_state: usize,
    /// The contiguous census seeds this range admits.
    pub span: (usize, usize),
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CaptureReport {
    pub ticks: [u64; N],
}

pub(crate) type Counters = carmy::EtaInputCounters;

/// Per-range capacities: every range allocates the same, so a balanced plan
/// still fits what the whole census needed before the split.
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

/// One contiguous range of the census, acquired by one worker with its own
/// buffers and geometry memo.
pub struct RangeCaptured {
    packet: carmy::EtaInputPacket,
    geometry: cprovince::GeometryPacket,
    /// Session-static map data, re-keyed from current roots before each capture.
    statics: cprovince::StaticGeometry,
    /// Walk-local raw modifier accumulators, cleared before each capture.
    modifiers: RefCell<SnapshotArena>,
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
            packet: packet(c),
            geometry: cprovince::GeometryPacket::with_capacities(c.geometry),
            statics: cprovince::StaticGeometry::default(),
            modifiers: RefCell::new(SnapshotArena::new(bounded_snapshot_capacity(c.modifiers))),
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
        self.air_defense.get_mut().reset_counts();
        self.required_modifiers = SnapshotCapacity::default();
        self.captured_current = false;
        self.counters = Counters::default();
        self.failure = None;
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        self.packet.allocated_bytes()
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
        CaptureReport {
            ticks: [self.capture(memory, context, sources, countries)],
        }
    }

    /// Finish an accepted worker failure without retrying the read. A denied
    /// range denies the table, exactly as a denied single job did.
    pub(crate) fn fail_owned_capture(&mut self, reason: &'static str) -> CaptureReport {
        self.reset();
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
        let rows = &sources.rows[..sources.outcome.len().min(sources.rows.len())];
        self.captured_current = true;
        let started = qpc();
        let span = context.span.0.min(rows.len())..context.span.1.min(rows.len());
        let memory = &ModifierMemory {
            inner: memory,
            arena: &self.modifiers,
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
        } else {
            self.packet.outcome().outcome
        }
    }

    /// Rows this range admitted, bounded by its own outcome.
    fn rows(&self) -> &[carmy::EtaInputRow] {
        let rows = self.packet.rows();
        &rows[..self.outcome().len().min(rows.len())]
    }

    /// True when every row's terrain and leg span lies inside this range's arenas.
    fn spans_valid(&self) -> bool {
        let terrain = self.packet.terrain().len();
        let legs = self.packet.legs().distance.len();
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
    /// This hour's contiguous spans, one per dispatched range.
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

    /// Contiguous spans balanced by native route length, so every worker scans
    /// a similar number of legs. Owned seed headers only; no native reads.
    pub(crate) fn plan(&mut self, sources: &Sources<'_, DivisionSeed>) -> usize {
        let count = sources.outcome.len().min(sources.rows.len());
        self.plan = [(count, count); MAX_RANGES];
        self.plan[0] = (0, count);
        self.planned = 1;
        let ranges = self.ranges.len().min(MAX_RANGES);
        if ranges < 2 || count < 2 * ranges {
            return 1;
        }
        let seeds = &sources.rows[..count];
        let weight = |seed: &DivisionSeed| -> usize {
            seed.ordinary_route
                .and_then(|header| engine::cpdx_array::Array::from_header(header, 4))
                .map_or(1, |route| {
                    (route.count as usize).min(engine::cunit::ETA_PATH_CAP) + 1
                })
        };
        let mut remaining: usize = seeds.iter().map(weight).sum();
        if remaining < MIN_SPLIT_WEIGHT {
            return 1;
        }
        let mut start = 0;
        for part in 0..ranges {
            let parts = ranges - part;
            if parts == 1 {
                self.plan[part] = (start, count);
                break;
            }
            // Leave at least one seed for every range still to come.
            let limit = count - (parts - 1);
            let target = remaining.div_ceil(parts);
            let (mut end, mut taken) = (start, 0);
            while end < limit {
                let next = taken + weight(&seeds[end]);
                if end != start
                    && (taken >= target || (next > target && next - target > target - taken))
                {
                    break;
                }
                taken = next;
                end += 1;
            }
            if end == start {
                taken = weight(&seeds[start]);
                end = start + 1;
            }
            self.plan[part] = (start, end);
            remaining = remaining.saturating_sub(taken);
            start = end;
        }
        self.planned = ranges;
        ranges
    }

    /// Lend one planned range's buffers and its span to the pool.
    pub(crate) fn take_range(&mut self, range: usize) -> (RangeCaptured, (usize, usize)) {
        (
            std::mem::replace(&mut self.ranges[range], RangeCaptured::vacant()),
            self.plan[range],
        )
    }

    pub(crate) fn restore_range(&mut self, range: usize, captured: RangeCaptured) {
        self.ranges[range] = captured;
    }

    /// Whole-census fallback for a callback that never dispatched the ranges.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        sources: Sources<'_, DivisionSeed>,
    ) -> [u64; N] {
        self.reset();
        let count = sources.outcome.len().min(sources.rows.len());
        self.plan[0] = (0, count);
        self.planned = 1;
        let context = CaptureContext {
            base,
            game_state: state,
            span: self.plan[0],
        };
        [self.ranges[0].capture(memory, context, sources, None)]
    }

    /// This hour's dispatched ranges, in census order.
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
        let outcome = self.outcomes()[0];
        if self.active().iter().any(|range| !range.spans_valid()) {
            return [unavailable("division ETA input spans invalid")];
        }
        // Ranges are contiguous in census order, so concatenating them keeps the
        // row order one walk over the whole census produced.
        let rows: Vec<(&RangeCaptured, &carmy::EtaInputRow)> = self
            .active()
            .iter()
            .flat_map(|range| range.rows().iter().map(move |row| (range, row)))
            .collect();
        [table::<tables::DivisionEtaInputs, _>(
            outcome,
            &rows,
            |(range, r)| {
                let terrain = range.packet.terrain();
                let legs = range.packet.legs();
                let pairs = &terrain[r.terrain_start..r.terrain_start + r.terrain_len];
                let mut terrain_speed = vec![
                    0;
                    pairs
                        .iter()
                        .map(|(index, _)| usize::from(*index))
                        .max()
                        .unwrap_or(0)
                ];
                for &(index, value) in pairs {
                    terrain_speed[usize::from(index) - 1] = value;
                }
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
                    speed_modifier: r.speed_modifier,
                    speed_base: r.speed_base,
                    speed_org: r.speed_org,
                    speed_fuel: r.speed_fuel,
                    speed_outer_scale: r.speed_outer_scale,
                    current_speed: r.current_speed,
                    terrain_speed,
                    fort_penalty: r.fort_penalty,
                    snow_penalty: r.snow_penalty,
                    river_adjustment: r.river_adjustment,
                    river_leader: r.river_leader,
                    has_leader: r.has_leader,
                    class: r.class,
                    invasion: r.invasion,
                    rail_blocked: r.rail_blocked,
                    adjacency_complete: r.adjacency_complete,
                    leg_distance: legs.distance[span.clone()].to_vec(),
                    leg_crossing: legs.crossing[span.clone()].to_vec(),
                    leg_river_combined: optional(legs.river, r.any_river),
                    leg_transport_speed: optional(legs.transport, r.any_transport),
                    leg_rail_speed: optional(legs.rail, r.any_rail),
                }
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;

    #[test]
    fn air_defense_memo_keeps_exact_signed_arguments_through_collisions_and_resets() {
        // Every input collides in the sole slot, including sign changes whose
        // pure results happen to be equal. Only identical triples can hit.
        let mut memo = AirDefenseMemo::with_capacity(1);
        for (index, arguments) in [
            [100_000, 100_000, 50_000],
            [-100_000, 100_000, 50_000],
            [100_000, -100_000, 50_000],
            [100_000, 100_000, -50_000],
            [100_000, 0, 50_000],
            [100_000, i64::MIN, i64::MAX],
            [100_000, i64::MAX, i64::MIN],
            [0, i64::MIN, i64::MAX],
            [100_000, 100_000, 50_000],
        ]
        .into_iter()
        .enumerate()
        {
            let [defense, denominator, reduction] = arguments;
            let expected = engine::csub_unit_definition::movement_air_defense_value(
                defense,
                denominator,
                reduction,
            );
            assert!(expected.is_ok());
            assert_eq!(memo.value(defense, denominator, reduction), expected);
            assert_eq!(memo.requests, 2 * index as u64 + 1);
            assert_eq!(memo.hits, index as u64);
            assert_eq!(memo.value(defense, denominator, reduction), expected);
            assert_eq!(memo.requests, 2 * index as u64 + 2);
            assert_eq!(memo.hits, index as u64 + 1);
        }
        memo.reset_counts();
        assert_eq!((memo.requests, memo.hits), (0, 0));
        assert_eq!(memo.value(100_000, 100_000, 50_000), Ok(25_000));
        assert_eq!((memo.requests, memo.hits), (1, 1));
    }

    #[test]
    fn air_defense_hits_preserve_fresh_operands_and_first_native_failure() {
        use engine::csub_unit_definition as stats;
        use std::cell::Cell;
        const BASE: usize = 0x140_000_000;
        const STATS: usize = 0x1000;
        struct Source {
            arguments: Cell<[i64; 3]>,
            fault: Cell<Option<usize>>,
            identity: Cell<bool>,
            reads: RefCell<Vec<usize>>,
        }
        impl Memory for Source {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.reads.borrow_mut().push(address);
                if self.fault.get() == Some(address) {
                    return false;
                }
                let arguments = self.arguments.get();
                let value = if address == STATS {
                    if self.identity.get() {
                        (BASE + stats::VTABLE_RVA) as i64
                    } else {
                        0
                    }
                } else if address == STATS + stats::MOVEMENT_AIR_DEFENSE {
                    arguments[0]
                } else if address == BASE + stats::AIR_DEFENSE_DENOMINATOR_RVA {
                    arguments[1]
                } else if address == BASE + stats::AIR_DEFENSE_REDUCTION_RVA {
                    arguments[2]
                } else {
                    return false;
                };
                output.copy_from_slice(&value.to_le_bytes());
                true
            }
        }
        let source = Source {
            arguments: Cell::new([100_000, 100_000, 50_000]),
            fault: Cell::new(None),
            identity: Cell::new(true),
            reads: RefCell::new(Vec::new()),
        };
        let arena = RefCell::new(SnapshotArena::new(SnapshotCapacity::default()));
        let air_defense = RefCell::new(AirDefenseMemo::with_capacity(1));
        let memory = ModifierMemory {
            inner: &source,
            arena: &arena,
            air_defense: &air_defense,
            countries: None,
        };
        let addresses = [
            STATS,
            STATS + stats::MOVEMENT_AIR_DEFENSE,
            BASE + stats::AIR_DEFENSE_DENOMINATOR_RVA,
            BASE + stats::AIR_DEFENSE_REDUCTION_RVA,
        ];
        for arguments in [
            [100_000, 100_000, 50_000],
            [50_000, 100_000, 50_000],
            [50_000, 200_000, 50_000],
            [50_000, 200_000, -50_000],
            [100_000, 100_000, 50_000],
        ] {
            source.arguments.set(arguments);
            let expected = stats::movement_air_defense(&source, BASE, STATS);
            air_defense.borrow_mut().reset_counts();
            for hits in [0, 1] {
                source.reads.borrow_mut().clear();
                assert_eq!(stats::movement_air_defense(&memory, BASE, STATS), expected);
                assert_eq!(&*source.reads.borrow(), &addresses);
                let memo = air_defense.borrow();
                assert_eq!((memo.requests, memo.hits), (hits + 1, hits));
            }
        }
        // The matching successful entry stays present while each fresh read
        // fails. The pure hook must not run until every operand is available.
        for (index, address) in addresses.into_iter().enumerate() {
            source.fault.set(Some(address));
            let expected = stats::movement_air_defense(&source, BASE, STATS);
            assert!(expected.is_err());
            source.reads.borrow_mut().clear();
            assert_eq!(stats::movement_air_defense(&memory, BASE, STATS), expected);
            assert_eq!(&*source.reads.borrow(), &addresses[..=index]);
            let memo = air_defense.borrow();
            assert_eq!((memo.requests, memo.hits), (2, 1));
        }
        source.fault.set(None);
        source.identity.set(false);
        let expected = stats::movement_air_defense(&source, BASE, STATS);
        assert!(expected.is_err());
        source.reads.borrow_mut().clear();
        assert_eq!(stats::movement_air_defense(&memory, BASE, STATS), expected);
        assert_eq!(&*source.reads.borrow(), &addresses[..1]);
        source.identity.set(true);
        source.arguments.set([0, i64::MIN, i64::MAX]);
        source.fault.set(Some(addresses[2]));
        source.reads.borrow_mut().clear();
        assert_eq!(stats::movement_air_defense(&memory, BASE, STATS), Ok(0));
        assert_eq!(&*source.reads.borrow(), &addresses[..2]);
        let memo = air_defense.borrow();
        assert_eq!((memo.requests, memo.hits), (2, 1));
    }

    #[test]
    fn air_defense_storage_grows_with_rows_off_capture_and_accounts_for_capacity() {
        let mut range = RangeCaptured::vacant();
        assert_eq!(range.allocated_bytes(), 0);
        let memo = range.air_defense.get_mut();
        for _ in 0..2 {
            assert_eq!(memo.value(100_000, 100_000, 50_000), Ok(25_000));
        }
        assert_eq!((memo.requests, memo.hits), (2, 0));
        assert_eq!(memo.allocated_bytes(), 0);
        let mut capacities = Capacities {
            rows: 1,
            terrain: 0,
            legs: 0,
            geometry: cprovince::GeometryCapacities {
                provinces: 0,
                neighbour_entries: 0,
            },
            modifiers: SnapshotCapacity::default(),
            ranges: 1,
        };
        for rows in [1, 3] {
            capacities.rows = rows;
            range.reserve(&capacities);
            range.reset();
            let memo = range.air_defense.get_mut();
            assert_eq!(memo.slots.len(), rows);
            assert_eq!(
                memo.allocated_bytes(),
                memo.slots.capacity() * std::mem::size_of::<Option<AirDefenseEntry>>()
            );
            assert_eq!(memo.value(100_000, 100_000, 50_000), Ok(25_000));
            assert_eq!(memo.value(100_000, 100_000, 50_000), Ok(25_000));
            assert_eq!((memo.requests, memo.hits), (2, 1));
            assert_eq!(
                range.allocated_bytes(),
                range.packet.allocated_bytes() + range.air_defense.borrow().allocated_bytes()
            );
        }
        range.reset();
        assert_eq!(
            range.air_defense.get_mut().value(100_000, 100_000, 50_000),
            Ok(25_000)
        );
        assert_eq!(range.air_defense.borrow().hits, 1);
        range.fail_owned_capture("worker stopped");
        let memo = range.air_defense.borrow();
        assert_eq!((memo.requests, memo.hits), (0, 0));
        assert_eq!(range.counters.air_defense_requests, 0);
        assert_eq!(range.counters.air_defense_hits, 0);
    }

    #[test]
    fn country_roster_reuse_keeps_ordinary_eta_reads_fresh_and_scoped() {
        use engine::{ccountry, cgame_state, country_memory::Cache};
        use std::cell::Cell;
        let mut bytes = vec![0; 0x4000];
        let mut put = |at: usize, value: &[u8]| bytes[at..at + value.len()].copy_from_slice(value);
        for (offset, pointer, count) in [
            (cgame_state::COUNTRIES, 0x1500_u64, 4_u32),
            (cgame_state::COUNTRY_LINKS, 0x1700, 8),
        ] {
            put(0x1000 + offset, &pointer.to_le_bytes());
            put(0x1000 + offset + 8, &count.to_le_bytes());
            put(0x1000 + offset + 12, &count.to_le_bytes());
        }
        for slot in [1, 2] {
            put(0x1500 + slot * 8, &0x2000_u64.to_le_bytes());
        }
        for link in [1, 2, 7] {
            put(0x1700 + link * 4, &1_u32.to_le_bytes());
        }
        put(0x2000, &(ccountry::VTABLE_RVA as u64).to_le_bytes());
        put(0x2000 + ccountry::ID, &7_u32.to_le_bytes());
        put(0x2000 + ccountry::COMMAND_POWER, &11_i64.to_le_bytes());
        struct Source {
            bytes: Vec<u8>,
            reads: Cell<usize>,
            scalars: Cell<usize>,
            spans: RefCell<Vec<(usize, usize)>>,
            fault: Cell<Option<usize>>,
        }
        impl Memory for Source {
            fn read_scalar<T: engine::memory::Raw>(&self, address: usize) -> Option<T> {
                self.scalars.set(self.scalars.get() + 1);
                self.reads.set(self.reads.get() + 1);
                self.spans
                    .borrow_mut()
                    .push((address, std::mem::size_of::<T>()));
                if self.fault.get() == Some(address) {
                    return None;
                }
                SliceMemory {
                    base: 0,
                    bytes: &self.bytes,
                }
                .read_scalar::<T>(address)
            }

            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.reads.set(self.reads.get() + 1);
                self.spans.borrow_mut().push((address, output.len()));
                self.fault.get() != Some(address)
                    && SliceMemory {
                        base: 0,
                        bytes: &self.bytes,
                    }
                    .copy(address, output)
            }
        }
        let source = Source {
            bytes,
            reads: Cell::new(0),
            scalars: Cell::new(0),
            spans: RefCell::new(Vec::new()),
            fault: Cell::new(None),
        };
        let mut cache = Cache::with_capacity(64);
        let cached = cache.prepare(&source, 0, 0x1000);
        let roster = cached.authenticated_countries(0, 0x1000).unwrap();
        let expected = ccountry::Reader::new(&cached, 0, 0x1000)
            .unwrap()
            .resolve(7)
            .unwrap();
        let arena = RefCell::new(SnapshotArena::new(SnapshotCapacity::default()));
        let air_defense = RefCell::new(AirDefenseMemo::with_capacity(0));
        let memory = ModifierMemory {
            inner: &source,
            arena: &arena,
            air_defense: &air_defense,
            countries: Some((0, 0x1000, roster)),
        };
        source.reads.set(0);
        source.scalars.set(0);
        source.spans.borrow_mut().clear();
        let reader = ccountry::Reader::new(&memory, 0, 0x1000).unwrap();
        assert_eq!(reader.resolve(7).unwrap(), expected);
        assert!(reader.at(2).unwrap().is_none());
        // The complete callback census owns both headers and the authenticated
        // objects. The requested link still reads its exact four native bytes.
        assert_eq!(source.reads.get(), 1);
        assert_eq!(source.scalars.get(), 1);
        assert_eq!(*source.spans.borrow(), [(0x1700 + 7 * 4, 4)]);
        assert!(memory.authenticated_countries(1, 0x1000).is_none());
        assert!(memory.authenticated_countries(0, 0x1001).is_none());
        assert_eq!(engine::read::<u32>(&memory, 0x2000 + ccountry::ID), Some(7));
        assert_eq!(source.reads.get(), 2);
        assert_eq!(source.scalars.get(), 2);
        assert_eq!(
            engine::read::<i64>(&memory, 0x2000 + ccountry::COMMAND_POWER),
            Some(11)
        );
        source.fault.set(Some(0x2000 + ccountry::COMMAND_POWER));
        assert_eq!(
            engine::read::<i64>(&memory, 0x2000 + ccountry::COMMAND_POWER),
            None
        );
        assert_eq!(source.scalars.get(), 2);
        assert_eq!(
            &source.spans.borrow()[1..],
            [
                (0x2000 + ccountry::ID, 4),
                (0x2000 + ccountry::COMMAND_POWER, 8),
                (0x2000 + ccountry::COMMAND_POWER, 8),
            ]
        );
        source.fault.set(Some(0x1700 + 7 * 4));
        source.spans.borrow_mut().clear();
        assert!(reader.resolve(7).is_err());
        assert_eq!(*source.spans.borrow(), [(0x1700 + 7 * 4, 4)]);
        let direct = ModifierMemory {
            countries: None,
            ..memory
        };
        source.fault.set(None);
        source.spans.borrow_mut().clear();
        assert_eq!(
            ccountry::Reader::new(&direct, 0, 0x1000)
                .unwrap()
                .resolve(7)
                .unwrap(),
            expected
        );
        assert_eq!(
            *source.spans.borrow(),
            [
                (0x1000 + cgame_state::COUNTRIES, 16),
                (0x1000 + cgame_state::COUNTRY_LINKS, 16),
                (0x1700 + 7 * 4, 4),
                (0x1500 + 8, 8),
                (0x2000, 8),
                (0x2000 + ccountry::ID, 4),
                (0x1700 + 7 * 4, 4),
                (0x1700 + 4, 4),
                (0x1700 + 4, 4),
                (0x1500 + 8, 8),
            ]
        );
        // Without the scoped census, each header and its original failure
        // remain mandatory. The second header cannot conceal the first fault.
        let headers = [
            (0x1000 + cgame_state::COUNTRIES, 16),
            (0x1000 + cgame_state::COUNTRY_LINKS, 16),
        ];
        for (index, &(address, _)) in headers.iter().enumerate() {
            source.fault.set(Some(address));
            source.spans.borrow_mut().clear();
            assert!(ccountry::Reader::new(&direct, 0, 0x1000).is_err());
            assert_eq!(&*source.spans.borrow(), &headers[..=index]);
        }
    }

    /// Every division of a country asks its accumulator for the same movement
    /// keys, and each lookup scans the whole array. The snapshot reads it once.
    #[test]
    fn the_modifier_snapshot_reads_each_accumulator_once_a_walk() {
        const ACCUMULATOR: usize = 0x1000;
        const DATA: usize = 0x2000;
        const ENTRIES: u32 = 200;
        let mut bytes = vec![0_u8; 0x4000];
        bytes[ACCUMULATOR..ACCUMULATOR + 8].copy_from_slice(&(DATA as u64).to_le_bytes());
        bytes[ACCUMULATOR + 8..ACCUMULATOR + 12].copy_from_slice(&ENTRIES.to_le_bytes());
        bytes[ACCUMULATOR + 12..ACCUMULATOR + 16].copy_from_slice(&ENTRIES.to_le_bytes());
        for index in 0..ENTRIES {
            let at = DATA + index as usize * 16;
            bytes[at..at + 4].copy_from_slice(&(index + 1).to_le_bytes());
            bytes[at + 8..at + 16].copy_from_slice(&i64::from(index).to_le_bytes());
        }
        struct Counted<'a> {
            bytes: &'a [u8],
            reads: std::cell::Cell<usize>,
            fault: Option<usize>,
        }
        impl Memory for Counted<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.reads.set(self.reads.get() + 1);
                if self.fault == Some(address) {
                    return false;
                }
                SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(address, destination)
            }
        }
        let keys = [0x2b_u32, 0x5e, 0x17f, 0xc8];
        let source = Counted {
            bytes: &bytes,
            reads: std::cell::Cell::new(0),
            fault: None,
        };
        let expected = keys
            .map(|key| engine::modifier_accumulator::lookup_at(&source, ACCUMULATOR, key).unwrap());
        assert_eq!(
            expected,
            [
                Value {
                    raw: 42,
                    present: true
                },
                Value {
                    raw: 93,
                    present: true
                },
                Value::default(),
                Value {
                    raw: 199,
                    present: true
                },
            ]
        );
        let direct = source.reads.replace(0);
        let arena = RefCell::new(SnapshotArena::new(SnapshotCapacity {
            nodes: 4,
            entries: ENTRIES as usize,
        }));
        let air_defense = RefCell::new(AirDefenseMemo::with_capacity(0));
        let memory = ModifierMemory {
            inner: &source,
            arena: &arena,
            air_defense: &air_defense,
            countries: None,
        };
        let values = keys
            .map(|key| engine::modifier_accumulator::lookup_at(&memory, ACCUMULATOR, key).unwrap());
        assert_eq!(values, expected);
        // One scan for the four keys, and the scan is the direct reader's own.
        assert_eq!(source.reads.get() * keys.len(), direct);
        // A new capture clears the arena and acquires current bytes again.
        arena.borrow_mut().clear();
        source.reads.set(0);
        let values = keys
            .map(|key| engine::modifier_accumulator::lookup_at(&memory, ACCUMULATOR, key).unwrap());
        assert_eq!(values, expected);
        assert_eq!(source.reads.get() * keys.len(), direct);
        // Too small an arena refuses the snapshot after its header read and
        // the direct reader answers every key.
        let small = RefCell::new(SnapshotArena::new(SnapshotCapacity {
            nodes: 4,
            entries: 8,
        }));
        let memory = ModifierMemory {
            inner: &source,
            arena: &small,
            air_defense: &air_defense,
            countries: None,
        };
        source.reads.set(0);
        assert_eq!(
            memory.modifier_accumulator_value(ACCUMULATOR, keys[0]),
            None
        );
        assert_eq!(source.reads.get(), 1);
        let values = keys
            .map(|key| engine::modifier_accumulator::lookup_at(&memory, ACCUMULATOR, key).unwrap());
        assert_eq!(values, expected);
        assert_eq!(source.reads.get(), direct + 1);
        assert!(small.borrow().required_capacity().entries >= ENTRIES as usize);

        let empty = RefCell::new(SnapshotArena::new(SnapshotCapacity::default()));
        let memory = ModifierMemory {
            inner: &source,
            arena: &empty,
            air_defense: &air_defense,
            countries: None,
        };
        source.reads.set(0);
        assert_eq!(
            memory.modifier_accumulator_value(ACCUMULATOR, keys[0]),
            None
        );
        assert_eq!(source.reads.get(), 0);
        let values = keys
            .map(|key| engine::modifier_accumulator::lookup_at(&memory, ACCUMULATOR, key).unwrap());
        assert_eq!(values, expected);
        assert_eq!(source.reads.get(), direct);
        assert_eq!(empty.borrow().required_capacity().nodes, 1);
        assert_eq!(empty.borrow().allocated_bytes(), 0);

        // Source failures remain owned errors; lookup must not retry them through
        // the direct-reader fallback, even when another movement key is requested.
        for (fault, reads, reason) in [
            (
                ACCUMULATOR,
                1,
                "modifier accumulator header unreadable or invalid",
            ),
            (DATA + 64 * 16, 66, "modifier accumulator entry unreadable"),
        ] {
            let source = Counted {
                bytes: &bytes,
                reads: std::cell::Cell::new(0),
                fault: Some(fault),
            };
            let arena = RefCell::new(SnapshotArena::new(SnapshotCapacity {
                nodes: 4,
                entries: ENTRIES as usize,
            }));
            let memory = ModifierMemory {
                inner: &source,
                arena: &arena,
                air_defense: &air_defense,
                countries: None,
            };
            assert_eq!(
                memory.modifier_accumulator_value(ACCUMULATOR, keys[0]),
                Some(Err(reason))
            );
            assert_eq!(source.reads.get(), reads);
            for key in keys {
                assert_eq!(
                    engine::modifier_accumulator::lookup_at(&memory, ACCUMULATOR, key),
                    Err(reason)
                );
            }
            assert_eq!(source.reads.get(), reads);
        }
    }

    #[test]
    fn a_failed_census_never_becomes_an_empty_eta_input_table() {
        let mut capture = Captured::with_capacities(&Capacities::default());
        let unavailable = Outcome::Unavailable {
            reason: "census failed",
        };
        capture.capture(
            &SliceMemory {
                base: 0,
                bytes: &[],
            },
            0,
            0,
            Sources {
                rows: &[],
                outcome: unavailable,
            },
        );
        assert!(matches!(capture.outcomes()[0], Outcome::Unavailable { .. }));
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        assert_eq!(capture.counters().walks, 1);
    }

    #[test]
    fn contiguous_ranges_cover_the_census_and_a_failed_range_denies_the_table() {
        let mut capture = Captured::with_capacities(&Capacities::with_ranges(3));
        let mut route = [0; 16];
        route[..8].copy_from_slice(&0x2000_u64.to_le_bytes());
        route[8..12].copy_from_slice(&64_u32.to_le_bytes());
        route[12..].copy_from_slice(&64_u32.to_le_bytes());
        let seeds: Vec<_> = (0..12)
            .map(|index| DivisionSeed {
                pointer: 0x1000 + index,
                division: index as u64 + 1,
                ordinary_route: Some(route),
                ..Default::default()
            })
            .collect();
        let outcome = Outcome::Rows { len: seeds.len() };
        let sources = Sources {
            rows: &seeds,
            outcome,
        };
        assert_eq!(capture.plan(&sources), 3);
        // The spans are contiguous, nonempty and cover the whole census.
        let mut next = 0;
        for range in 0..3 {
            let (captured, span) = capture.take_range(range);
            assert_eq!(span.0, next);
            assert!(span.1 > span.0);
            next = span.1;
            capture.restore_range(range, captured);
        }
        assert_eq!(next, seeds.len());
        // A census too small to be worth splitting stays serial.
        let stationary: Vec<_> = seeds
            .iter()
            .map(|seed| DivisionSeed {
                ordinary_route: Some([0; 16]),
                ..*seed
            })
            .collect();
        assert_eq!(
            capture.plan(&Sources {
                rows: &stationary,
                outcome,
            }),
            1
        );
        // A worker failure on one range denies the table with its own reason.
        assert_eq!(capture.plan(&sources), 3);
        let (mut range, _) = capture.take_range(0);
        assert_eq!(range.fail_owned_capture("worker stopped").ticks, [0]);
        capture.restore_range(0, range);
        assert!(matches!(
            capture.outcomes()[0],
            Outcome::Unavailable {
                reason: "worker stopped"
            }
        ));
    }
}
