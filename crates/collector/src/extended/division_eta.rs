//! Hourly native ETA operands from the shared division census. Normal speed
//! composition and route duration consume owned inputs on the spool thread.
use super::production::Sources;
use crate::{
    sampling::qpc,
    spooling::unavailable,
    table_capture::{Arena, CaptureRows},
};
use engine::{Memory, carmy::DivisionSeed, cprovince, cunit, walk::Outcome};
use replay::{
    frame::TableInput,
    rows::{DivisionEtaRow, TableId, tables},
};
use std::{cell::RefCell, mem::size_of};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::DivisionEta];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// Callback-local counts, accumulated across the one or two acquisition passes.
/// Journal sizes are peak bounded army-scope demand, not allocated pool bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CacheDiagnostics {
    pub hits: u64,
    pub retained_misses: u64,
    pub direct_reads: u64,
    pub witness_skips: u64,
    pub witness_fallbacks: u64,
    pub witness_entries_peak: usize,
    pub witness_bytes_peak: usize,
    pub witness_entries_required: usize,
    pub witness_bytes_required: usize,
    pub stages: StageDiagnostics,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) struct StageCost {
    pub calls: u64,
    pub ticks: u64,
}
impl StageCost {
    fn record(&mut self, started: i64) {
        self.calls = self.calls.saturating_add(1);
        self.ticks = self
            .ticks
            .saturating_add(qpc().saturating_sub(started).max(0) as u64);
    }
    fn absorb(&mut self, next: Self) {
        self.calls = self.calls.saturating_add(next.calls);
        self.ticks = self.ticks.saturating_add(next.ticks);
    }
}

/// Nested acquisition service spans, plus disjoint pass-final verification.
/// No QPC call or trace is added to the scalar-memory read path. Capability
/// fallbacks execute in the engine outside the corresponding resolver span.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) struct StageDiagnostics {
    pub walk: StageCost,
    pub supply_acquire: StageCost,
    pub supply_verify: StageCost,
    pub transport_acquire: StageCost,
    pub transport_verify: StageCost,
    pub modifiers_acquire: StageCost,
    pub modifiers_verify: StageCost,
    pub primary_acquire: StageCost,
    pub primary_verify: StageCost,
    pub cache_verify: StageCost,
    pub scope_verify: StageCost,
    pub supply_builds: u64,
    pub transport_builds: u64,
    pub supply_fallbacks: u64,
    pub modifier_fallbacks: u64,
}
impl StageDiagnostics {
    pub(crate) fn absorb(&mut self, next: Self) {
        macro_rules! spans { ($($field:ident),+ $(,)?) => { $(self.$field.absorb(next.$field);)+ }; }
        spans!(
            walk,
            supply_acquire,
            supply_verify,
            transport_acquire,
            transport_verify,
            modifiers_acquire,
            modifiers_verify,
            primary_acquire,
            primary_verify,
            cache_verify,
            scope_verify
        );
        self.supply_builds = self.supply_builds.saturating_add(next.supply_builds);
        self.transport_builds = self.transport_builds.saturating_add(next.transport_builds);
        self.supply_fallbacks = self.supply_fallbacks.saturating_add(next.supply_fallbacks);
        self.modifier_fallbacks = self
            .modifier_fallbacks
            .saturating_add(next.modifier_fallbacks);
    }
}

fn measure_stage<T>(
    cache: &RefCell<SmallReadCache>,
    stage: impl FnOnce(&mut StageDiagnostics) -> &mut StageCost,
    work: impl FnOnce() -> T,
) -> T {
    let started = qpc();
    let result = work();
    stage(&mut cache.borrow_mut().diagnostics.stages).record(started);
    result
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    legs: usize,
    cache_entries: usize,
    witness_entries: usize,
    witness_bytes: usize,
    transport_entries: usize,
    modifier_accumulators: usize,
    modifier_bytes: usize,
    supply_entries: usize,
    air_defense_entries: usize,
    primary_roots: usize,
    primary_chunks: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 8192,
            legs: 32 * 1024,
            cache_entries: 128 * 1024,
            witness_entries: 1024,
            witness_bytes: 64 * 1024,
            transport_entries: 1024,
            modifier_accumulators: 1024,
            modifier_bytes: 32 * 1024 * 16,
            supply_entries: 1024,
            air_defense_entries: 1024,
            primary_roots: 1024,
            primary_chunks: 2048,
        }
    }
}

// The planner examines owned seeds only; it has no live memory capability.
pub(crate) const MAX_RANGES: usize = 4;
// copy_u32s copies at most 64 IDs per batch. Before the first speed call,
// geometry bypasses CacheRef and every other local value is an owned seed.
const LEADING_WITNESS_ENTRIES: usize = cunit::ETA_PATH_CAP.div_ceil(64);
const LEADING_WITNESS_BYTES: usize = cunit::ETA_PATH_CAP * size_of::<u32>();

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RangePlan {
    pub start: usize,
    pub end: usize,
    pub serial: bool,
}

fn plan_ranges(
    sources: &Sources<'_, DivisionSeed>,
    checked: bool,
    max_ranges: usize,
    storage: &[(usize, usize); MAX_RANGES],
    plans: &mut [RangePlan; MAX_RANGES],
) -> usize {
    fn route_count(seed: &DivisionSeed) -> Option<usize> {
        let route = engine::cpdx_array::Array::from_header(seed.ordinary_route?, 4)?;
        let count = route.count as usize;
        (count <= engine::cunit::ETA_PATH_CAP).then_some(count)
    }

    // A serial descriptor is only a dispatch marker. The caller must pass the
    // original Sources/outcomes to the existing whole-table acquisition, even
    // for malformed source lengths; it must not slice by this descriptor.
    plans.fill(RangePlan::default());
    plans[0] = RangePlan {
        start: 0,
        end: sources
            .first
            .len()
            .max(if checked { sources.second.len() } else { 0 }),
        serial: true,
    };

    let count = sources.first_outcome.len();
    if max_ranges < 2
        || matches!(sources.first_outcome, Outcome::Unavailable { .. })
        || count == 0
        || count > sources.first.len()
        || count > storage[0].0
        || (checked
            && (matches!(sources.second_outcome, Outcome::Unavailable { .. })
                || sources.second_outcome.len() != count
                || count > sources.second.len()))
    {
        return 1;
    }
    let ranges = max_ranges.min(MAX_RANGES).min(count);
    if ranges < 2 {
        return 1;
    }

    let counts = |index: usize| -> Option<(usize, usize)> {
        let first = &sources.first[index];
        let first_count = route_count(first)?;
        let second_count = if checked {
            let second = &sources.second[index];
            if first.pointer != second.pointer || first.division != second.division {
                return None;
            }
            route_count(second)?
        } else {
            first_count
        };
        Some((first_count, second_count))
    };

    // This deliberately uses complete native route counts. A self edge or
    // zero-speed stop cannot justify admission without reading native inputs.
    let mut first_total = 0_usize;
    let mut second_total = 0_usize;
    let mut total_weight = 0_usize;
    for index in 0..count {
        let Some((first, second)) = counts(index) else {
            return 1;
        };
        let Some(next_first) = first_total.checked_add(first) else {
            return 1;
        };
        let Some(next_second) = second_total.checked_add(second) else {
            return 1;
        };
        let Some(next_weight) = first
            .max(second)
            .checked_add(1)
            .and_then(|weight| total_weight.checked_add(weight))
        else {
            return 1;
        };
        first_total = next_first;
        second_total = next_second;
        total_weight = next_weight;
        if first_total > storage[0].1 || second_total > storage[0].1 {
            return 1;
        }
    }

    // Choose the same contiguous boundaries for both cold passes. The maximum
    // per-seed route demand balances their larger workload conservatively.
    // A cut is moved toward the remaining average without creating empty
    // ranges. Physical storage is checked independently afterwards: inability
    // to fit these cuts selects serial acquisition before any native read.
    let mut candidate = [RangePlan::default(); MAX_RANGES];
    let mut start = 0;
    let mut remaining_weight = total_weight;
    for (part, plan) in candidate.iter_mut().take(ranges).enumerate() {
        let remaining_parts = ranges - part;
        let limit = count - (remaining_parts - 1);
        let target = remaining_weight.div_ceil(remaining_parts);
        let mut end = start;
        let mut weight = 0_usize;
        while end < limit {
            let Some((first, second)) = counts(end) else {
                return 1;
            };
            let Some(next_weight) = first
                .max(second)
                .checked_add(1)
                .and_then(|next| weight.checked_add(next))
            else {
                return 1;
            };
            if remaining_parts != 1
                && end != start
                && (weight >= target
                    || (next_weight > target && next_weight - target > target - weight))
            {
                break;
            }
            weight = next_weight;
            end += 1;
        }
        if end == start {
            return 1;
        }
        *plan = RangePlan {
            start,
            end,
            serial: false,
        };
        let Some(next_remaining) = remaining_weight.checked_sub(weight) else {
            return 1;
        };
        remaining_weight = next_remaining;
        start = end;
    }
    if start != count || remaining_weight != 0 {
        return 1;
    }

    for (part, plan) in candidate.iter().take(ranges).enumerate() {
        if plan.end - plan.start > storage[part].0 {
            return 1;
        }
        let mut first_legs = 0_usize;
        let mut second_legs = 0_usize;
        for index in plan.start..plan.end {
            let Some((first, second)) = counts(index) else {
                return 1;
            };
            let Some(next_first) = first_legs.checked_add(first) else {
                return 1;
            };
            let Some(next_second) = second_legs.checked_add(second) else {
                return 1;
            };
            first_legs = next_first;
            second_legs = next_second;
        }
        if first_legs > storage[part].1 || second_legs > storage[part].1 {
            return 1;
        }
    }

    // A stationary seed never replaces NormalMovementContext. Preserve reuse
    // when the last moving army before a cut is the first moving army after
    // it: that uncommon census uses the complete serial path.
    for plan in candidate.iter().take(ranges).skip(1) {
        for pass in 0..=usize::from(checked) {
            let seeds = if pass == 0 {
                sources.first
            } else {
                sources.second
            };
            let before = seeds[..plan.start]
                .iter()
                .rev()
                .find(|seed| route_count(seed).is_some_and(|count| count != 0));
            let after = seeds[plan.start..count]
                .iter()
                .find(|seed| route_count(seed).is_some_and(|count| count != 0));
            if before
                .zip(after)
                .is_some_and(|(before, after)| before.pointer == after.pointer)
            {
                return 1;
            }
        }
    }
    *plans = candidate;
    ranges
}

#[cfg(test)]
mod range_planner_tests {
    use super::{MAX_RANGES, RangePlan, plan_ranges};
    use crate::extended::production::Sources;
    use engine::{carmy::DivisionSeed, walk::Outcome};

    fn seed(index: usize, route_count: u32) -> DivisionSeed {
        let mut header = [0_u8; 16];
        // These are deliberately opaque addresses: no Memory object is used.
        header[..8].copy_from_slice(&0x1000_u64.to_le_bytes());
        header[8..12].copy_from_slice(&route_count.to_le_bytes());
        header[12..].copy_from_slice(&route_count.to_le_bytes());
        DivisionSeed {
            pointer: index + 1,
            division: index as u64 + 10,
            ordinary_route: Some(header),
            ..Default::default()
        }
    }

    fn sources<'a>(
        first: &'a [DivisionSeed],
        second: &'a [DivisionSeed],
    ) -> Sources<'a, DivisionSeed> {
        Sources {
            first,
            first_outcome: Outcome::Rows { len: first.len() },
            second,
            second_outcome: Outcome::Rows { len: second.len() },
        }
    }

    fn serial(plans: &[RangePlan; MAX_RANGES], end: usize) {
        assert_eq!(
            plans[0],
            RangePlan {
                start: 0,
                end,
                serial: true
            }
        );
        assert_eq!(plans[1..], [RangePlan::default(); MAX_RANGES - 1]);
    }

    #[test]
    fn contiguous_balanced_ranges_use_only_owned_headers() {
        let first: [_; 8] = std::array::from_fn(|index| seed(index, 2));
        let input = sources(&first, &first);
        let mut plans = [RangePlan::default(); MAX_RANGES];
        let storage = [(8, 16), (2, 4), (2, 4), (2, 4)];
        assert_eq!(
            plan_ranges(&input, true, usize::MAX, &storage, &mut plans),
            4
        );
        assert_eq!(
            plans,
            std::array::from_fn(|part| RangePlan {
                start: part * 2,
                end: part * 2 + 2,
                serial: false,
            })
        );

        let skewed: [_; 8] =
            std::array::from_fn(|index| seed(index, if index == 0 { 10 } else { 0 }));
        let storage = [(8, 10), (3, 0), (2, 0), (2, 0)];
        assert_eq!(
            plan_ranges(&sources(&skewed, &[]), false, 4, &storage, &mut plans),
            4
        );
        assert_eq!(
            plans.map(|plan| (plan.start, plan.end)),
            [(0, 1), (1, 4), (4, 6), (6, 8)]
        );
    }

    #[test]
    fn empty_unavailable_invalid_lengths_and_disabled_parallelism_keep_serial_gate() {
        let first = [seed(0, 0), seed(1, 0)];
        let storage = [(2, 0); MAX_RANGES];
        let mut plans = [RangePlan {
            start: 8,
            end: 9,
            serial: false,
        }; MAX_RANGES];
        for max_ranges in [0, 1] {
            assert_eq!(
                plan_ranges(
                    &sources(&first, &[]),
                    false,
                    max_ranges,
                    &storage,
                    &mut plans
                ),
                1
            );
            serial(&plans, 2);
        }
        assert_eq!(
            plan_ranges(&sources(&[], &[]), false, 4, &storage, &mut plans),
            1
        );
        serial(&plans, 0);
        for outcome in [
            Outcome::Unavailable {
                reason: "source fault",
            },
            Outcome::Rows { len: 3 },
        ] {
            let mut input = sources(&first, &[]);
            input.first_outcome = outcome;
            assert_eq!(plan_ranges(&input, false, 4, &storage, &mut plans), 1);
            serial(&plans, 2);
        }
        let mut small = storage;
        small[0].0 = 1;
        assert_eq!(
            plan_ranges(&sources(&first, &[]), false, 4, &small, &mut plans),
            1
        );
        serial(&plans, 2);
    }

    #[test]
    fn cold_shape_and_identity_must_agree_but_unchecked_second_is_ignored() {
        let first = [seed(0, 1), seed(1, 1)];
        let storage = [(2, 8); MAX_RANGES];
        let mut plans = [RangePlan::default(); MAX_RANGES];
        for second in [[seed(1, 1), seed(0, 1)], [seed(0, 1), seed(2, 1)]] {
            assert_eq!(
                plan_ranges(&sources(&first, &second), true, 4, &storage, &mut plans),
                1
            );
            serial(&plans, 2);
        }
        assert_eq!(
            plan_ranges(&sources(&first, &first[..1]), true, 4, &storage, &mut plans),
            1
        );
        serial(&plans, 2);
        let mut unavailable = sources(&first, &first);
        unavailable.second_outcome = Outcome::Unavailable {
            reason: "cold source fault",
        };
        assert_eq!(plan_ranges(&unavailable, true, 4, &storage, &mut plans), 1);
        serial(&plans, 2);
        assert_eq!(plan_ranges(&unavailable, false, 4, &storage, &mut plans), 2);
        assert!(plans[..2].iter().all(|plan| !plan.serial));
    }

    #[test]
    fn full_native_route_limits_and_invalid_headers_fall_back_before_reads() {
        let allowed = [seed(0, 4096), seed(1, 0)];
        let storage = [(2, 4096), (1, 0), (0, 0), (0, 0)];
        let mut plans = [RangePlan::default(); MAX_RANGES];
        assert_eq!(
            plan_ranges(&sources(&allowed, &[]), false, 4, &storage, &mut plans),
            2
        );
        for invalid in [None, seed(0, 4097).ordinary_route, Some([0xff; 16])] {
            let mut first = allowed;
            first[0].ordinary_route = invalid;
            assert_eq!(
                plan_ranges(&sources(&first, &[]), false, 4, &storage, &mut plans),
                1
            );
            serial(&plans, 2);
        }
        let mut first = allowed;
        let mut overflowing = first[0].ordinary_route.unwrap();
        overflowing[..8].copy_from_slice(&(usize::MAX as u64).to_le_bytes());
        first[0].ordinary_route = Some(overflowing);
        assert_eq!(
            plan_ranges(&sources(&first, &[]), false, 4, &storage, &mut plans),
            1
        );
        serial(&plans, 2);
    }

    #[test]
    fn conservative_route_sum_never_assumes_self_edges_or_zero_speed() {
        let first = [seed(0, 4), seed(1, 4)];
        let mut plans = [RangePlan::default(); MAX_RANGES];
        let mut storage = [(2, 7), (1, 4), (0, 0), (0, 0)];
        // Serial execution could consume fewer than eight actual legs. The
        // pure planner cannot assume that without demanding route/speed reads.
        assert_eq!(
            plan_ranges(&sources(&first, &[]), false, 2, &storage, &mut plans),
            1
        );
        serial(&plans, 2);
        storage[0].1 = 8;
        assert_eq!(
            plan_ranges(&sources(&first, &[]), false, 2, &storage, &mut plans),
            2
        );
        let second = [seed(0, 5), seed(1, 5)];
        assert_eq!(
            plan_ranges(&sources(&first, &second), true, 2, &storage, &mut plans),
            1
        );
        serial(&plans, 2);
    }

    #[test]
    fn every_physical_range_must_fit_both_cold_passes() {
        let first = [seed(0, 1), seed(1, 1)];
        let second = [seed(0, 1), seed(1, 5)];
        let mut plans = [RangePlan::default(); MAX_RANGES];
        let storage = [(2, 6), (1, 1), (0, 0), (0, 0)];
        assert_eq!(
            plan_ranges(&sources(&first, &second), true, 2, &storage, &mut plans),
            1
        );
        serial(&plans, 2);
        let uniform: [_; 8] = std::array::from_fn(|index| seed(index, 0));
        let storage = [(8, 0), (1, 0), (2, 0), (2, 0)];
        assert_eq!(
            plan_ranges(&sources(&uniform, &[]), false, 4, &storage, &mut plans),
            1
        );
        serial(&plans, 8);
    }

    #[test]
    fn inherited_truncation_is_not_multiplied_or_used_as_a_native_bound() {
        let first: [_; 4] = std::array::from_fn(|index| seed(index, 1));
        let mut input = sources(&first, &first);
        input.first_outcome = Outcome::Truncated {
            len: 4,
            unread_rows: 11,
        };
        input.second_outcome = Outcome::Truncated {
            len: 4,
            unread_rows: 7,
        };
        let storage = [(4, 4), (1, 1), (1, 1), (1, 1)];
        let mut plans = [RangePlan::default(); MAX_RANGES];
        assert_eq!(plan_ranges(&input, true, 4, &storage, &mut plans), 4);
        assert_eq!(plans[0].start, 0);
        assert_eq!(plans[3].end, 4);
        assert!(plans.iter().all(|plan| plan.end - plan.start == 1));
        assert_eq!(
            input.first_outcome,
            Outcome::Truncated {
                len: 4,
                unread_rows: 11
            }
        );
        assert_eq!(
            input.second_outcome,
            Outcome::Truncated {
                len: 4,
                unread_rows: 7
            }
        );
    }

    #[test]
    fn unused_source_storage_never_participates_in_preflight() {
        let first = [seed(0, 1), seed(1, 1), DivisionSeed::default()];
        let second = [seed(0, 2), seed(1, 2), seed(99, 4097)];
        let mut input = sources(&first, &second);
        input.first_outcome = Outcome::Rows { len: 2 };
        input.second_outcome = Outcome::Rows { len: 2 };
        let storage = [(2, 4), (1, 2), (0, 0), (0, 0)];
        let mut plans = [RangePlan::default(); MAX_RANGES];
        assert_eq!(plan_ranges(&input, true, 4, &storage, &mut plans), 2);
        assert_eq!(
            plans[0],
            RangePlan {
                start: 0,
                end: 1,
                serial: false
            }
        );
        assert_eq!(
            plans[1],
            RangePlan {
                start: 1,
                end: 2,
                serial: false
            }
        );
        assert_eq!(plans[2..], [RangePlan::default(); MAX_RANGES - 2]);
    }

    #[test]
    fn same_moving_army_across_stationary_boundary_uses_serial_context() {
        let mut first = [seed(0, 1), seed(1, 0), seed(2, 0), seed(3, 1)];
        let mut plans = [RangePlan::default(); MAX_RANGES];
        let storage = [(4, 4); MAX_RANGES];
        assert_eq!(
            plan_ranges(&sources(&first, &[]), false, 2, &storage, &mut plans),
            2
        );
        first[3].pointer = first[0].pointer;
        first[3].division = first[0].division;
        assert_eq!(
            plan_ranges(&sources(&first, &[]), false, 2, &storage, &mut plans),
            1
        );
        serial(&plans, first.len());
    }
}

/// The flat acquisition pool retains one workspace for each active ETA range.
/// Frame payloads never own these caches. Every workspace stays leased until
/// all ranges have acquired and its terminal verification has completed.
pub(crate) struct Workspace {
    capacities: Capacities,
    cache: RefCell<SmallReadCache>,
    transport: TransportMemo,
    modifiers: RefCell<ModifierSnapshots>,
    supply: RefCell<SupplyMemo>,
    air_defense: RefCell<AirDefenseMemo>,
    primary: RefCell<engine::carmy_leader::PrimaryRootCache>,
    geometry: cprovince::GeometryPacket,
    geometry_second: Option<cprovince::GeometryPacket>,
}
impl Workspace {
    /// Check every leased workspace before planning any parallel native work.
    /// A pool with a smaller/disabled witness must request the serial plan.
    pub(crate) fn supports_ranges(&self) -> bool {
        let cache = self.cache.borrow();
        cache.witness.enabled()
            && cache.witness.entries.len() >= LEADING_WITNESS_ENTRIES
            && cache.witness.bytes.len() >= LEADING_WITNESS_BYTES
    }
    pub(crate) fn with_capacities(c: &Capacities, checked: bool) -> Self {
        let geometry = cprovince::GeometryCapacities {
            provinces: c.rows.saturating_mul(2),
            neighbour_entries: c.legs.saturating_mul(4),
        };
        Self {
            capacities: *c,
            cache: RefCell::new(SmallReadCache::with_witness(
                c.cache_entries,
                c.witness_entries,
                c.witness_bytes,
            )),
            transport: TransportMemo::new(c.transport_entries),
            modifiers: RefCell::new(ModifierSnapshots::new(
                c.modifier_accumulators,
                c.modifier_bytes,
            )),
            supply: RefCell::new(SupplyMemo::new(c.supply_entries)),
            air_defense: RefCell::new(AirDefenseMemo::new(c.air_defense_entries)),
            primary: RefCell::new(engine::carmy_leader::PrimaryRootCache::new(
                c.primary_roots,
                c.primary_chunks,
            )),
            geometry: cprovince::GeometryPacket::with_capacities(geometry),
            geometry_second: checked.then(|| cprovince::GeometryPacket::with_capacities(geometry)),
        }
    }

    fn reset(&mut self) {
        self.cache.get_mut().clear();
        self.transport.clear();
        self.modifiers.get_mut().clear();
        self.supply.get_mut().clear();
        self.air_defense.get_mut().clear();
        self.primary.get_mut().clear();
        self.geometry.clear();
        if let Some(geometry) = &mut self.geometry_second {
            geometry.clear();
        }
    }

    /// Pool maintenance only; no growth is permitted inside an acquisition job.
    fn grow(&mut self, c: &Capacities) {
        let first = self.geometry.required_capacities();
        let second = self
            .geometry_second
            .as_ref()
            .map_or(first, |p| p.required_capacities());
        let geometry = cprovince::GeometryCapacities {
            provinces: first
                .provinces
                .max(second.provinces)
                .max(c.rows.saturating_mul(2)),
            neighbour_entries: first
                .neighbour_entries
                .max(second.neighbour_entries)
                .max(c.legs.saturating_mul(4)),
        };
        self.cache.get_mut().grow(c.cache_entries);
        self.cache
            .get_mut()
            .witness
            .grow(c.witness_entries, c.witness_bytes);
        self.transport.grow(c.transport_entries);
        self.modifiers
            .get_mut()
            .grow(c.modifier_accumulators, c.modifier_bytes);
        self.supply.get_mut().grow(c.supply_entries);
        self.air_defense.get_mut().grow(c.air_defense_entries);
        self.primary
            .get_mut()
            .grow(c.primary_roots, c.primary_chunks);
        self.geometry.grow(geometry);
        // Pooled workspaces keep independent cold storage available even
        // after their most recent callback did not request a check.
        if let Some(second) = &mut self.geometry_second {
            second.grow(geometry);
        } else {
            self.geometry_second = Some(cprovince::GeometryPacket::with_capacities(geometry));
        }
        self.capacities = *c;
    }

    /// Decode-worker maintenance after a callback returns this workspace.
    /// Reusing an ungrown workspace remains bounded and preserves capacity
    /// failures/fallbacks; it never permits callback allocation.
    pub(crate) fn prepare_for_reuse(&mut self) {
        let mut capacities = self.capacities;
        let diagnostics = self.cache.borrow().diagnostics;
        crate::table_capture::grow_capacity(
            &mut capacities.witness_entries,
            diagnostics.witness_entries_required,
        );
        crate::table_capture::grow_capacity(
            &mut capacities.witness_bytes,
            diagnostics.witness_bytes_required,
        );
        self.grow(&capacities);
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        self.cache.borrow().allocated_bytes()
            + self.transport.allocated_bytes()
            + self.modifiers.borrow().allocated_bytes()
            + self.supply.borrow().allocated_bytes()
            + self.air_defense.borrow().allocated_bytes()
            + self.primary.borrow().allocated_bytes()
            + self.geometry.allocated_bytes()
            + self
                .geometry_second
                .as_ref()
                .map_or(0, cprovince::GeometryPacket::allocated_bytes)
    }
}

#[derive(Clone, Copy)]
struct PassReport {
    walk: cunit::EtaWalk,
    outcome: Outcome,
    geometry_active: bool,
    verified: bool,
    terminal_stage: u8,
}
impl Default for PassReport {
    fn default() -> Self {
        let outcome = Outcome::Unavailable {
            reason: "ETA range was not acquired",
        };
        Self {
            walk: cunit::EtaWalk {
                outcome,
                legs_len: 0,
                required_legs: 0,
                visited_sources: 0,
                acquisition_failed: true,
                boundary_failure: None,
            },
            outcome,
            geometry_active: false,
            verified: false,
            terminal_stage: 0,
        }
    }
}

/// Owned row/leg storage moves with its job and returns to the same frame slot.
/// Native readers and mutable dependency caches live only in Workspace.
pub(crate) struct RangeCaptured {
    plan: RangePlan,
    rows: CaptureRows<cunit::EtaInputs>,
    legs: Arena<cunit::EtaLeg>,
    seed_masks: [Vec<u64>; 2],
    speed_frontiers: [Vec<cunit::EtaSpeedFrontier>; 2],
    reports: [PassReport; 2],
    geometry_same: bool,
    diagnostics: CacheDiagnostics,
    ticks: u64,
}
impl RangeCaptured {
    fn new(rows: usize, legs: usize, sources: usize, successful: u16) -> Self {
        Self {
            plan: RangePlan::default(),
            rows: CaptureRows::new(rows, successful),
            legs: Arena::new(legs, successful),
            seed_masks: [
                vec![0; sources],
                crate::table_capture::comparison_rows(sources, successful),
            ],
            speed_frontiers: [
                vec![None; sources],
                crate::table_capture::comparison_rows(sources, successful),
            ],
            reports: [PassReport::default(); 2],
            geometry_same: false,
            diagnostics: CacheDiagnostics::default(),
            ticks: 0,
        }
    }

    fn reset(&mut self, plan: RangePlan, checked: bool) {
        self.plan = plan;
        self.rows.reset();
        self.rows.checked = checked;
        self.legs.reset();
        self.seed_masks.iter_mut().for_each(|m| m.fill(0));
        self.speed_frontiers.iter_mut().for_each(|f| f.fill(None));
        self.reports = [PassReport::default(); 2];
        self.geometry_same = false;
        self.diagnostics = CacheDiagnostics::default();
        self.ticks = 0;
    }

    /// A failed accepted job is terminal. Verification failure retains the
    /// complete acquisition frontier; an interrupted acquisition cannot credit
    /// its unfinished native walk. Neither phase may silently retry.
    pub(crate) fn fail(&mut self, reason: &'static str, pass: usize, acquisition: bool) {
        let outcome = Outcome::Unavailable { reason };
        self.reports[pass].outcome = outcome;
        if acquisition {
            self.reports[pass].walk.outcome = outcome;
            self.reports[pass].walk.acquisition_failed = true;
        } else {
            self.reports[pass].terminal_stage = u8::MAX;
        }
        self.reports[pass].verified = true;
    }

    /// Each pass receives its independently captured full census. A serial
    /// descriptor retains the original outcome and the original capacity law.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn acquire_pass(
        &mut self,
        workspace: &mut Workspace,
        memory: &impl Memory,
        base: usize,
        state: usize,
        sources: &[DivisionSeed],
        source_outcome: Outcome,
        pass: usize,
    ) -> u64 {
        let started = qpc();
        if pass == 0 {
            workspace.reset();
        }
        self.reports[pass] = PassReport::default();
        if !self.plan.serial && !workspace.supports_ranges() {
            self.fail("ETA range witness storage unavailable", pass, true);
            let cost = qpc().saturating_sub(started).max(0) as u64;
            self.ticks = self.ticks.saturating_add(cost);
            return cost;
        }
        let leading_context = !self.plan.serial
            && sources.get(..self.plan.start).is_some_and(|prefix| {
                prefix.iter().any(|seed| {
                    seed.ordinary_route
                        .and_then(|header| engine::cpdx_array::Array::from_header(header, 4))
                        .is_some_and(|route| route.count != 0)
                })
            });
        let (sources, outcome) = if self.plan.serial {
            (sources, source_outcome)
        } else {
            match sources.get(self.plan.start..self.plan.end) {
                Some(sources) => (sources, Outcome::Rows { len: sources.len() }),
                None => (
                    &[][..],
                    Outcome::Unavailable {
                        reason: "ETA range source span invalid",
                    },
                ),
            }
        };
        let geometry = if pass == 0 {
            Some(&mut workspace.geometry)
        } else {
            workspace.geometry_second.as_mut()
        };
        let Some(geometry) = geometry else {
            self.fail("ETA cold geometry storage unavailable", pass, true);
            let cost = qpc().saturating_sub(started).max(0) as u64;
            self.ticks = self.ticks.saturating_add(cost);
            return cost;
        };
        let report = &mut self.reports[pass];
        report.geometry_active = !matches!(outcome, Outcome::Unavailable { .. })
            && outcome.len() <= sources.len()
            && outcome.len() <= self.seed_masks[pass].len();
        let ready = if report.geometry_active {
            geometry.begin(memory, base, state)
        } else {
            match outcome {
                Outcome::Unavailable { reason } => Err(reason),
                _ => Err("ETA division seed storage invalid"),
            }
        };
        let cached = CacheRef {
            source: memory,
            cache: &workspace.cache,
            modifiers: Some(&workspace.modifiers),
            supply: Some(&workspace.supply),
            air_defense: Some(&workspace.air_defense),
            primary: Some(&workspace.primary),
        };
        let output = if pass == 0 {
            &mut self.rows.rows
        } else {
            &mut self.rows.second
        };
        let legs = if pass == 0 {
            &mut self.legs.first
        } else {
            &mut self.legs.second
        };
        report.walk = measure_stage(
            &workspace.cache,
            |s| &mut s.walk,
            || {
                ready.map_or_else(
                    |reason| cunit::EtaWalk {
                        outcome: Outcome::Unavailable { reason },
                        legs_len: 0,
                        required_legs: 0,
                        visited_sources: 0,
                        acquisition_failed: true,
                        boundary_failure: None,
                    },
                    |()| {
                        cunit::walk_eta_with_transport(
                            &cached,
                            memory,
                            base,
                            state,
                            sources,
                            outcome,
                            output,
                            legs,
                            &mut self.seed_masks[pass],
                            &mut self.speed_frontiers[pass],
                            leading_context,
                            geometry,
                            |owner, region| {
                                measure_stage(
                                    &workspace.cache,
                                    |s| &mut s.transport_acquire,
                                    || {
                                        workspace.transport.resolve(owner, region, || {
                                            let mut cache = workspace.cache.borrow_mut();
                                            cache.diagnostics.stages.transport_builds = cache
                                                .diagnostics
                                                .stages
                                                .transport_builds
                                                .saturating_add(1);
                                            drop(cache);
                                            engine::cnaval_unit_transfer::movement_speed(
                                                &cached, base, state, owner, region,
                                            )
                                        })
                                    },
                                )
                            },
                        )
                    },
                )
            },
        );
        report.outcome = report.walk.outcome;
        let cost = qpc().saturating_sub(started).max(0) as u64;
        self.ticks = self.ticks.saturating_add(cost);
        cost
    }

    /// The callback schedules this phase only after every range acquisition
    /// has completed. All retained dependencies are checked even after faults.
    pub(crate) fn verify_pass(
        &mut self,
        workspace: &mut Workspace,
        memory: &impl Memory,
        base: usize,
        state: usize,
        pass: usize,
    ) -> u64 {
        let started = qpc();
        let report = &mut self.reports[pass];
        let geometry = if pass == 0 {
            Some(&workspace.geometry)
        } else {
            workspace.geometry_second.as_ref()
        };
        if report.geometry_active
            && let Some(geometry) = geometry
            && let Err(reason) = geometry.verify(memory)
        {
            report.outcome = Outcome::Unavailable { reason };
            report.terminal_stage = 1;
        }
        if !measure_stage(
            &workspace.cache,
            |s| &mut s.transport_verify,
            || {
                workspace.transport.verify_and_clear(|owner, region| {
                    engine::cnaval_unit_transfer::movement_speed(memory, base, state, owner, region)
                })
            },
        ) {
            report.outcome = Outcome::Unavailable {
                reason: "division ETA transport speed changed or unreadable",
            };
            report.terminal_stage = 2;
        }
        if !measure_stage(
            &workspace.cache,
            |s| &mut s.supply_verify,
            || {
                workspace
                    .supply
                    .borrow_mut()
                    .verify_and_clear(|base, state, owner, area| {
                        engine::ccountry::has_core_supply_access_direct(
                            memory, base, state, owner, area,
                        )
                    })
            },
        ) {
            report.outcome = Outcome::Unavailable {
                reason: "division ETA core supply access changed or unreadable",
            };
            report.terminal_stage = 3;
        }
        if !measure_stage(
            &workspace.cache,
            |s| &mut s.modifiers_verify,
            || workspace.modifiers.borrow_mut().verify_and_clear(memory),
        ) {
            report.outcome = Outcome::Unavailable {
                reason: "division ETA modifier snapshot changed or unreadable",
            };
            report.terminal_stage = 4;
        }
        workspace.air_defense.borrow_mut().clear();
        if !measure_stage(
            &workspace.cache,
            |s| &mut s.primary_verify,
            || workspace.primary.borrow_mut().verify_and_clear(memory),
        ) {
            report.outcome = Outcome::Unavailable {
                reason: "division ETA primary order dependencies changed or unreadable",
            };
            report.terminal_stage = 5;
        }
        if !measure_stage(
            &workspace.cache,
            |s| &mut s.cache_verify,
            || workspace.cache.borrow_mut().verify_and_clear(memory),
        ) {
            report.outcome = Outcome::Unavailable {
                reason: "division ETA cached operands changed or unreadable",
            };
            report.terminal_stage = 6;
        }
        report.verified = true;
        self.diagnostics = workspace.cache.borrow().diagnostics;
        if pass == 0 {
            self.rows.outcome = report.outcome;
            self.legs.len = report.walk.legs_len;
            self.legs.required = report.walk.required_legs;
        } else {
            self.geometry_same = workspace
                .geometry_second
                .as_ref()
                .is_some_and(|second| workspace.geometry.same_observation(second));
        }
        let cost = qpc().saturating_sub(started).max(0) as u64;
        self.ticks = self.ticks.saturating_add(cost);
        cost
    }

    fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.legs.allocated_bytes()
            + self
                .seed_masks
                .iter()
                .map(|m| m.capacity() * size_of::<u64>())
                .sum::<usize>()
            + self
                .speed_frontiers
                .iter()
                .map(|f| f.capacity() * size_of::<cunit::EtaSpeedFrontier>())
                .sum::<usize>()
    }
}

impl CacheDiagnostics {
    fn absorb(&mut self, next: Self) {
        self.hits = self.hits.saturating_add(next.hits);
        self.retained_misses = self.retained_misses.saturating_add(next.retained_misses);
        self.direct_reads = self.direct_reads.saturating_add(next.direct_reads);
        self.witness_skips = self.witness_skips.saturating_add(next.witness_skips);
        self.witness_fallbacks = self
            .witness_fallbacks
            .saturating_add(next.witness_fallbacks);
        self.witness_entries_peak = self.witness_entries_peak.max(next.witness_entries_peak);
        self.witness_bytes_peak = self.witness_bytes_peak.max(next.witness_bytes_peak);
        self.witness_entries_required = self
            .witness_entries_required
            .max(next.witness_entries_required);
        self.witness_bytes_required = self.witness_bytes_required.max(next.witness_bytes_required);
        self.stages.absorb(next.stages);
    }
}

pub struct Captured {
    ranges: [Option<RangeCaptured>; MAX_RANGES],
    active: usize,
    outcome: Outcome,
    checked: bool,
    mismatches: usize,
    source_outcomes: [Outcome; 2],
    seed_masks: [Vec<u64>; 2],
    seed_required: usize,
    required_legs: usize,
    retained_ranges: [usize; 2],
    used_legs: [usize; 2],
    pass_required_legs: [usize; 2],
    leg_limits: [[usize; MAX_RANGES]; 2],
    completed: RefCell<Vec<DivisionEtaRow>>,
    diagnostics: CacheDiagnostics,
    capacities: Capacities,
    overhead_ticks: u64,
}
impl Captured {
    #[cfg(test)]
    pub(crate) fn workspace_capacities(&self) -> Capacities {
        self.capacities
    }

    pub fn with_capacities(c: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        let unavailable = Outcome::Unavailable {
            reason: "clock unavailable",
        };
        Self {
            // Slot zero retains full-table capacity for the original serial
            // admission path. Other slots contain output storage only.
            ranges: std::array::from_fn(|index| {
                Some(RangeCaptured::new(
                    if index == 0 {
                        c.rows
                    } else {
                        c.rows.div_ceil(MAX_RANGES)
                    },
                    if index == 0 {
                        c.legs
                    } else {
                        c.legs.div_ceil(MAX_RANGES)
                    },
                    if index == 0 {
                        c.rows
                    } else {
                        c.rows.div_ceil(MAX_RANGES)
                    },
                    successful[0],
                ))
            }),
            active: 1,
            outcome: unavailable,
            checked: false,
            mismatches: 0,
            source_outcomes: [unavailable; 2],
            seed_masks: [
                vec![0; c.rows],
                crate::table_capture::comparison_rows(c.rows, successful[0]),
            ],
            seed_required: 0,
            required_legs: 0,
            retained_ranges: [0; 2],
            used_legs: [0; 2],
            pass_required_legs: [0; 2],
            leg_limits: [[0; MAX_RANGES]; 2],
            completed: RefCell::new(Vec::with_capacity(c.rows)),
            diagnostics: CacheDiagnostics::default(),
            capacities: *c,
            overhead_ticks: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self::with_capacities(
            &Capacities {
                rows: 0,
                legs: 0,
                cache_entries: 0,
                witness_entries: 0,
                witness_bytes: 0,
                transport_entries: 0,
                modifier_accumulators: 0,
                modifier_bytes: 0,
                supply_entries: 0,
                air_defense_entries: 0,
                primary_roots: 0,
                primary_chunks: 0,
            },
            &[0],
        )
    }

    pub fn reset(&mut self) {
        self.outcome = Outcome::Unavailable {
            reason: "clock unavailable",
        };
        self.checked = false;
        self.mismatches = 0;
        self.active = 1;
        self.seed_required = 0;
        self.required_legs = 0;
        self.retained_ranges = [0; 2];
        self.used_legs = [0; 2];
        self.pass_required_legs = [0; 2];
        self.leg_limits = [[0; MAX_RANGES]; 2];
        self.completed.get_mut().clear();
        self.seed_masks.iter_mut().for_each(|m| m.fill(0));
        self.diagnostics = CacheDiagnostics::default();
        self.overhead_ticks = 0;
        for range in self.ranges.iter_mut().flatten() {
            range.reset(RangePlan::default(), false);
        }
    }

    pub(crate) fn reserve_sources(&mut self, capacity: usize) {
        crate::table_capture::grow_vec(&mut self.seed_masks[0], capacity);
        if !self.ranges[0]
            .as_ref()
            .expect("ETA range restored")
            .rows
            .second
            .is_empty()
            || self.capacities.rows == 0
        {
            crate::table_capture::grow_vec(&mut self.seed_masks[1], capacity);
        }
        for (index, range) in self.ranges.iter_mut().enumerate() {
            let range = range.as_mut().expect("ETA range restored");
            let target = if index == 0 {
                capacity
            } else {
                capacity.div_ceil(MAX_RANGES)
            };
            crate::table_capture::grow_vec(&mut range.seed_masks[0], target);
            crate::table_capture::grow_vec(&mut range.speed_frontiers[0], target);
            if !range.rows.second.is_empty() || self.capacities.rows == 0 {
                crate::table_capture::grow_vec(&mut range.seed_masks[1], target);
                crate::table_capture::grow_vec(&mut range.speed_frontiers[1], target);
            }
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.ranges
            .iter()
            .flatten()
            .map(RangeCaptured::allocated_bytes)
            .sum::<usize>()
            + self
                .seed_masks
                .iter()
                .map(|m| m.capacity() * size_of::<u64>())
                .sum::<usize>()
            + self.completed.borrow().capacity() * size_of::<DivisionEtaRow>()
    }

    /// Called only after all range payloads have returned to this frame.
    pub fn grow(&mut self, c: &mut Capacities, successful: &[u16]) {
        match self.outcome {
            Outcome::Rows { len } if len > 0 && len >= c.rows.saturating_sub(c.rows / 4) => {
                let target = c.rows.saturating_add(1);
                crate::table_capture::grow_capacity(&mut c.rows, target);
            }
            Outcome::Truncated { len, unread_rows } => crate::table_capture::grow_capacity(
                &mut c.rows,
                len.saturating_add(unread_rows as usize),
            ),
            _ => {}
        }
        crate::table_capture::grow_capacity(&mut c.legs, self.required_legs);
        let sources = self.seed_required.max(c.rows);
        for (index, range) in self.ranges.iter_mut().enumerate() {
            let range = range.as_mut().expect("ETA range restored before growth");
            let rows = if index == 0 {
                c.rows
            } else {
                c.rows.div_ceil(MAX_RANGES)
            };
            let legs = if index == 0 {
                c.legs
            } else {
                c.legs.div_ceil(MAX_RANGES)
            };
            let masks = if index == 0 {
                sources
            } else {
                sources.div_ceil(MAX_RANGES)
            };
            crate::table_capture::grow_vec(&mut range.rows.rows, rows);
            crate::table_capture::comparison(&mut range.rows.second, rows, successful[0]);
            crate::table_capture::grow_vec(&mut range.legs.first, legs);
            crate::table_capture::comparison(&mut range.legs.second, legs, successful[0]);
            crate::table_capture::grow_vec(&mut range.seed_masks[0], masks);
            crate::table_capture::comparison(&mut range.seed_masks[1], masks, successful[0]);
            crate::table_capture::grow_vec(&mut range.speed_frontiers[0], masks);
            crate::table_capture::comparison(&mut range.speed_frontiers[1], masks, successful[0]);
        }
        crate::table_capture::grow_vec(&mut self.seed_masks[0], sources);
        crate::table_capture::comparison(&mut self.seed_masks[1], sources, successful[0]);
        let completed = self.completed.get_mut();
        completed.clear();
        completed.reserve(c.rows);
        self.capacities = *c;
    }

    /// Pure admission planning over the already owned census. Returning one
    /// serial descriptor is decided before any ETA geometry or speed read.
    pub(crate) fn prepare_ranges(
        &mut self,
        sources: &Sources<'_, DivisionSeed>,
        checked: bool,
        max_ranges: usize,
    ) -> usize {
        let started = qpc();
        self.reset();
        self.checked = checked;
        self.source_outcomes = [sources.first_outcome, sources.second_outcome];
        self.seed_required = sources.first_outcome.len().max(if checked {
            sources.second_outcome.len()
        } else {
            0
        });
        let storage = std::array::from_fn(|index| {
            let part = self.ranges[index]
                .as_ref()
                .expect("ETA range restored before planning");
            let mut rows = part.rows.rows.len().min(part.seed_masks[0].len());
            let mut legs = part.legs.first.len();
            if checked {
                rows = rows
                    .min(part.rows.second.len())
                    .min(part.seed_masks[1].len());
                legs = legs.min(part.legs.second.len());
            }
            (rows, legs)
        });
        let mut plans = [RangePlan::default(); MAX_RANGES];
        let max_ranges = if self.capacities.witness_entries >= LEADING_WITNESS_ENTRIES
            && self.capacities.witness_bytes >= LEADING_WITNESS_BYTES
        {
            max_ranges
        } else {
            1
        };
        self.active = plan_ranges(sources, checked, max_ranges, &storage, &mut plans);
        for (index, plan) in plans.into_iter().enumerate().take(self.active) {
            self.ranges[index]
                .as_mut()
                .expect("ETA range restored")
                .reset(plan, checked);
        }
        self.overhead_ticks = qpc().saturating_sub(started).max(0) as u64;
        self.active
    }

    pub(crate) fn take_range(&mut self, index: usize) -> Option<RangeCaptured> {
        if index < self.active {
            self.ranges[index].take()
        } else {
            None
        }
    }
    pub(crate) fn restore_range(&mut self, index: usize, range: RangeCaptured) {
        assert!(index < self.active && self.ranges[index].is_none());
        self.ranges[index] = Some(range);
    }

    /// Reduce only the source prefix the serial walker demanded. Terminal
    /// faults occur after acquisition and never shorten this mask frontier.
    fn reduce_pass(&mut self, pass: usize) -> Outcome {
        self.seed_masks[pass].fill(0);
        self.retained_ranges[pass] = 0;
        self.used_legs[pass] = 0;
        self.pass_required_legs[pass] = 0;
        self.leg_limits[pass].fill(0);
        let mut outcome = Outcome::Rows { len: 0 };
        let mut len = 0_usize;
        let mut required = 0_usize;
        let mut terminal_stage = 0;
        let mut terminal = None;
        let mut serial = false;
        let mut pending_boundary: Option<(usize, &'static str)> = None;
        for (index, range) in self.ranges[..self.active].iter().flatten().enumerate() {
            let report = range.reports[pass];
            self.retained_ranges[pass] += 1;
            if !report.verified {
                return Outcome::Unavailable {
                    reason: "ETA range verification incomplete",
                };
            }
            serial = range.plan.serial;
            let start = if serial { 0 } else { range.plan.start };
            let mut walk = report.walk;
            let mut mask_override = None;
            if !serial && let Some((army, reason)) = pending_boundary {
                let Some(frontiers) = range.speed_frontiers[pass].get(..walk.visited_sources)
                else {
                    return Outcome::Unavailable {
                        reason: "ETA speed frontier span invalid",
                    };
                };
                let next = frontiers.iter().enumerate().find_map(|(slot, frontier)| {
                    frontier
                        .filter(|frontier| frontier.0 != army)
                        .map(|frontier| (slot, frontier))
                });
                if let Some((slot, (_, mask, legs_required, legs_used, _rows))) = next {
                    // The serial walker checks the preceding army immediately
                    // before this call, after this seed's route prelude.
                    walk.visited_sources = slot + 1;
                    walk.required_legs = legs_required;
                    walk.legs_len = legs_used;
                    walk.outcome = Outcome::Unavailable { reason };
                    walk.acquisition_failed = true;
                    mask_override = Some(mask);
                    pending_boundary = None;
                } else if walk.acquisition_failed {
                    // A pre-speed source/geometry failure wins: the original
                    // wrapper ignores final context failure after an abort.
                    pending_boundary = None;
                }
            }
            let visit = walk.visited_sources;
            let masks = &range.seed_masks[pass];
            if visit > masks.len()
                || start
                    .checked_add(visit)
                    .is_none_or(|end| end > self.seed_masks[pass].len())
            {
                return Outcome::Unavailable {
                    reason: "ETA range seed mask span invalid",
                };
            }
            self.seed_masks[pass][start..start + visit].copy_from_slice(&masks[..visit]);
            if let Some(mask) = mask_override {
                self.seed_masks[pass][start + visit - 1] = mask;
            }
            required = match required.checked_add(walk.required_legs) {
                Some(value) => value,
                None => {
                    return Outcome::Unavailable {
                        reason: "ETA leg count overflow",
                    };
                }
            };
            self.used_legs[pass] = match self.used_legs[pass].checked_add(walk.legs_len) {
                Some(value) => value,
                None => {
                    return Outcome::Unavailable {
                        reason: "ETA leg count overflow",
                    };
                }
            };
            self.leg_limits[pass][index] = walk.legs_len;
            len = match len.checked_add(walk.outcome.len()) {
                Some(value) => value,
                None => {
                    return Outcome::Unavailable {
                        reason: "ETA row count overflow",
                    };
                }
            };
            let boundary_only =
                !serial && report.walk.boundary_failure.is_some() && report.terminal_stage == 0;
            if matches!(report.outcome, Outcome::Unavailable { .. })
                && !boundary_only
                && (terminal.is_none() || report.terminal_stage > terminal_stage)
            {
                terminal = Some(report.outcome);
                terminal_stage = report.terminal_stage;
            }
            outcome = walk.outcome;
            if walk.acquisition_failed {
                // A reconstructed boundary abort precedes this range's own
                // acquisition error, but independent terminal checks retain
                // their original later-stage priority.
                if terminal_stage == 0 {
                    terminal = Some(walk.outcome);
                }
                break;
            }
            if !serial && pending_boundary.is_none() {
                pending_boundary = report.walk.boundary_failure;
            }
        }
        self.required_legs = self.required_legs.max(required);
        self.pass_required_legs[pass] = required;
        if let Some(outcome) = terminal {
            return outcome;
        }
        if let Some((_, reason)) = pending_boundary {
            return Outcome::Unavailable { reason };
        }
        if serial {
            return outcome;
        }
        match self.source_outcomes[pass] {
            Outcome::Truncated { unread_rows, .. } => Outcome::Truncated { len, unread_rows },
            Outcome::Unavailable { reason } => Outcome::Unavailable { reason },
            _ => Outcome::Rows { len },
        }
    }

    fn next_owned_leg(
        &self,
        pass: usize,
        range_index: &mut usize,
        leg_index: &mut usize,
    ) -> Option<cunit::EtaLeg> {
        while *range_index < self.retained_ranges[pass] {
            let range = self.ranges[*range_index].as_ref()?;
            if *leg_index < self.leg_limits[pass][*range_index] {
                let legs = if pass == 0 {
                    &range.legs.first
                } else {
                    &range.legs.second
                };
                let value = legs.get(*leg_index).copied();
                *leg_index += 1;
                return value;
            }
            *range_index += 1;
            *leg_index = 0;
        }
        None
    }

    /// Compare the acquired prefixes without flattening or inspecting any
    /// speculative legs after either acquisition abort.
    fn same_owned_legs(&self) -> bool {
        if self.used_legs[0] != self.used_legs[1]
            || self.pass_required_legs[0] != self.pass_required_legs[1]
        {
            return false;
        }
        let (mut first_range, mut first_leg, mut second_range, mut second_leg) = (0, 0, 0, 0);
        for _ in 0..self.used_legs[0] {
            let first = self.next_owned_leg(0, &mut first_range, &mut first_leg);
            if first.is_none()
                || first != self.next_owned_leg(1, &mut second_range, &mut second_leg)
            {
                return false;
            }
        }
        true
    }

    pub(crate) fn finish_ranges(&mut self) -> AcquisitionReport {
        let started = qpc();
        if self.ranges[..self.active].iter().any(Option::is_none) {
            self.fail("ETA range payload missing", self.checked);
            return AcquisitionReport::default();
        }
        self.outcome = self.reduce_pass(0);
        let second = if self.checked {
            self.reduce_pass(1)
        } else {
            self.outcome
        };
        self.mismatches = usize::from(self.checked && self.outcome != second);
        self.diagnostics = CacheDiagnostics::default();
        let mut ticks = self.overhead_ticks;
        let common = self.retained_ranges[0].min(self.retained_ranges[1]);
        for (index, range) in self.ranges[..self.active].iter_mut().flatten().enumerate() {
            ticks = ticks.saturating_add(range.ticks);
            self.diagnostics.absorb(range.diagnostics);
            if self.checked
                && !matches!(self.outcome, Outcome::Unavailable { .. })
                && self.outcome == second
            {
                range.rows.outcome = range.reports[0].outcome;
                range.rows.compare(range.reports[1].outcome);
                self.mismatches = self.mismatches.saturating_add(range.rows.mismatches);
            }
            if self.checked && index < common {
                self.mismatches = self
                    .mismatches
                    .saturating_add(usize::from(!range.geometry_same));
            }
        }
        if self.checked {
            self.mismatches = self
                .mismatches
                .saturating_add(usize::from(!self.same_owned_legs()))
                .saturating_add(usize::from(
                    self.retained_ranges[0] != self.retained_ranges[1],
                ));
        }
        let matched =
            self.checked && crate::sampling::full_match(self.outcome, second, self.mismatches);
        ticks = ticks.saturating_add(qpc().saturating_sub(started).max(0) as u64);
        AcquisitionReport { matched, ticks }
    }

    #[cfg(test)]
    fn capture_with_workspace(
        &mut self,
        workspace: &mut Workspace,
        memory: &impl Memory,
        base: usize,
        state: usize,
        checked: bool,
        sources: Sources<'_, DivisionSeed>,
    ) -> AcquisitionReport {
        self.prepare_ranges(&sources, checked, 1);
        let mut range = self.take_range(0).expect("serial ETA range");
        for pass in 0..=usize::from(checked) {
            let (seeds, outcome) = if pass == 0 {
                (sources.first, sources.first_outcome)
            } else {
                (sources.second, sources.second_outcome)
            };
            range.acquire_pass(workspace, memory, base, state, seeds, outcome, pass);
            range.verify_pass(workspace, memory, base, state, pass);
        }
        self.restore_range(0, range);
        self.finish_ranges()
    }

    #[cfg(test)]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
        sources: Sources<'_, DivisionSeed>,
    ) -> ([bool; N], [u64; N]) {
        let mut workspace = Workspace::with_capacities(&self.capacities, needed[0]);
        let report =
            self.capture_with_workspace(&mut workspace, memory, base, state, needed[0], sources);
        ([report.matched], [report.ticks])
    }

    pub(crate) fn fail(&mut self, reason: &'static str, checked: bool) {
        self.reset();
        self.outcome = Outcome::Unavailable { reason };
        self.checked = checked;
    }
    pub(crate) fn reject_source_census(&mut self) -> bool {
        if !self.checked {
            return true;
        }
        self.outcome = Outcome::Unavailable {
            reason: "division ETA source census changed during double read",
        };
        self.mismatches = self.mismatches.saturating_add(1);
        false
    }
    pub(crate) fn consumed_seed_masks(&self, pass: usize) -> &[u64] {
        &self.seed_masks[pass]
    }
    pub(crate) fn reject_changed_seeds(&mut self, pass: usize, changed: &[u64]) -> bool {
        let valid = self.seed_masks[pass]
            .iter()
            .enumerate()
            .all(|(index, &mask)| {
                mask == 0
                    || changed
                        .get(index)
                        .is_some_and(|changed| changed & mask == 0)
            });
        if !valid {
            self.outcome = Outcome::Unavailable {
                reason: "division ETA shared seed changed or unreadable",
            };
            if self.checked {
                self.mismatches = self.mismatches.saturating_add(1);
            }
        }
        valid
    }
    pub(crate) fn cache_diagnostics(&self) -> CacheDiagnostics {
        self.diagnostics
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [if self.checked && self.mismatches != 0 {
            Outcome::Unavailable {
                reason: "division ETA operands changed during double read",
            }
        } else {
            self.outcome
        }]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.mismatches]
    }

    pub(crate) fn table_inputs(
        &self,
        projection: Option<&mut crate::projection::Projection<'_>>,
    ) -> [TableInput; N] {
        let outcome = self.outcomes()[0];
        if let Outcome::Unavailable { reason } = outcome {
            return [unavailable(reason).into()];
        }
        let mut rows = self.completed.borrow_mut();
        rows.clear();
        for range in self.ranges[..self.active].iter().flatten() {
            let count = if self.active == 1 {
                outcome.len()
            } else {
                range.reports[0].walk.outcome.len()
            };
            let Some(inputs) = range.rows.rows.get(..count) else {
                return [unavailable("division ETA row span invalid").into()];
            };
            let Some(legs) = range.legs.first.get(..range.legs.len) else {
                return [unavailable("division ETA leg arena invalid").into()];
            };
            for input in inputs {
                let (next_remaining, destination_remaining) =
                    match cunit::eta_remaining(input, legs) {
                        Ok(value) => value,
                        Err(reason) => return [unavailable(reason).into()],
                    };
                rows.push(DivisionEtaRow {
                    division: input.division,
                    next: input.next,
                    destination: input.destination,
                    path_total: input.path_total,
                    next_remaining,
                    destination_remaining,
                });
            }
        }
        if rows.len() != outcome.len() {
            return [unavailable("division ETA assembled row span invalid").into()];
        }
        [crate::projection::scalar_table::<tables::DivisionEta, _>(
            projection,
            outcome,
            &rows,
            |r| u128::from(r.division),
            |r| r.clone(),
        )]
    }
    #[cfg(test)]
    pub fn tables(&self) -> [replay::frame::TableView; N] {
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("unprojected ETA always returns a sample"),
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AcquisitionReport {
    pub matched: bool,
    pub ticks: u64,
}

#[cfg(test)]
mod range_reduction_tests {
    use super::*;

    fn fixture() -> Captured {
        let mut captured = Captured::with_capacities(
            &Capacities {
                rows: 8,
                legs: 8,
                cache_entries: 0,
                witness_entries: 0,
                witness_bytes: 0,
                transport_entries: 0,
                modifier_accumulators: 0,
                modifier_bytes: 0,
                supply_entries: 0,
                air_defense_entries: 0,
                primary_roots: 0,
                primary_chunks: 0,
            },
            &[0],
        );
        captured.active = 3;
        captured.source_outcomes = [Outcome::Rows { len: 6 }; 2];
        for index in 0..3 {
            let part = captured.ranges[index].as_mut().unwrap();
            part.reset(
                RangePlan {
                    start: index * 2,
                    end: index * 2 + 2,
                    serial: false,
                },
                true,
            );
            for pass in 0..2 {
                part.seed_masks[pass][..2].fill(DivisionSeed::CLASS);
                part.reports[pass] = PassReport {
                    walk: cunit::EtaWalk {
                        outcome: Outcome::Rows { len: 2 },
                        legs_len: 0,
                        required_legs: 2,
                        visited_sources: 2,
                        acquisition_failed: false,
                        boundary_failure: None,
                    },
                    outcome: Outcome::Rows { len: 2 },
                    geometry_active: false,
                    verified: true,
                    terminal_stage: 0,
                };
            }
            part.geometry_same = true;
        }
        captured
    }

    #[test]
    fn acquisition_abort_keeps_failing_seed_mask_and_discards_speculative_suffix() {
        let mut captured = fixture();
        let part = captured.ranges[1].as_mut().unwrap();
        part.reports[0].walk = cunit::EtaWalk {
            outcome: Outcome::Unavailable {
                reason: "failed source",
            },
            legs_len: 0,
            required_legs: 1,
            visited_sources: 1,
            acquisition_failed: true,
            boundary_failure: None,
        };
        part.reports[0].outcome = part.reports[0].walk.outcome;
        part.seed_masks[0][0] = DivisionSeed::PRIMARY_VTABLE;
        assert_eq!(
            captured.reduce_pass(0),
            Outcome::Unavailable {
                reason: "failed source"
            }
        );
        assert_eq!(
            &captured.seed_masks[0][..6],
            &[
                DivisionSeed::CLASS,
                DivisionSeed::CLASS,
                DivisionSeed::PRIMARY_VTABLE,
                0,
                0,
                0,
            ]
        );
        assert_eq!(captured.required_legs, 3);
    }

    #[test]
    fn terminal_fault_preserves_the_complete_acquisition_mask_union() {
        let mut captured = fixture();
        let part = captured.ranges[0].as_mut().unwrap();
        part.reports[0].outcome = Outcome::Unavailable {
            reason: "changed dependency",
        };
        part.reports[0].terminal_stage = 5;
        assert_eq!(
            captured.reduce_pass(0),
            Outcome::Unavailable {
                reason: "changed dependency"
            }
        );
        assert_eq!(&captured.seed_masks[0][..6], &[DivisionSeed::CLASS; 6]);
        assert_eq!(captured.required_legs, 6);
    }

    #[test]
    fn inherited_missing_sources_are_counted_once_after_range_reassembly() {
        let mut captured = fixture();
        captured.source_outcomes[0] = Outcome::Truncated {
            len: 6,
            unread_rows: 17,
        };
        assert_eq!(
            captured.reduce_pass(0),
            Outcome::Truncated {
                len: 6,
                unread_rows: 17
            }
        );
    }

    #[test]
    fn terminal_stage_priority_is_independent_of_range_completion_order() {
        let mut captured = fixture();
        captured.ranges[0].as_mut().unwrap().reports[0].outcome = Outcome::Unavailable {
            reason: "cache verification",
        };
        captured.ranges[0].as_mut().unwrap().reports[0].terminal_stage = 6;
        captured.ranges[2].as_mut().unwrap().reports[0].outcome = Outcome::Unavailable {
            reason: "transport verification",
        };
        captured.ranges[2].as_mut().unwrap().reports[0].terminal_stage = 2;
        assert_eq!(
            captured.reduce_pass(0),
            Outcome::Unavailable {
                reason: "cache verification"
            }
        );
        assert_eq!(&captured.seed_masks[0][..6], &[DivisionSeed::CLASS; 6]);
    }

    #[test]
    fn failed_verify_job_retains_masks_while_failed_acquire_stops_the_frontier() {
        let mut captured = fixture();
        captured.ranges[0]
            .as_mut()
            .unwrap()
            .fail("verify panic", 0, false);
        captured.reduce_pass(0);
        assert_eq!(&captured.seed_masks[0][..6], &[DivisionSeed::CLASS; 6]);
        let mut captured = fixture();
        let range = captured.ranges[1].as_mut().unwrap();
        range.reports[0].walk.visited_sources = 0;
        range.reports[0].walk.required_legs = 0;
        range.fail("acquire panic", 0, true);
        captured.reduce_pass(0);
        assert_eq!(
            &captured.seed_masks[0][..6],
            &[DivisionSeed::CLASS, DivisionSeed::CLASS, 0, 0, 0, 0]
        );
    }

    #[test]
    fn boundary_failure_stops_at_next_speed_prelude_after_stationary_sources() {
        let mut captured = fixture();
        let first = captured.ranges[0].as_mut().unwrap();
        first.reports[0].walk.boundary_failure = Some((100, "preceding context changed"));
        first.reports[0].walk.outcome = Outcome::Unavailable {
            reason: "preceding context changed",
        };
        first.reports[0].outcome = first.reports[0].walk.outcome;
        let next = captured.ranges[1].as_mut().unwrap();
        let before_speed = DivisionSeed::CLASS | DivisionSeed::LOCATION;
        next.speed_frontiers[0][1] = Some((200, before_speed, 2, 0, 1));
        next.seed_masks[0][1] = before_speed | DivisionSeed::ORGANISATION;
        assert_eq!(
            captured.reduce_pass(0),
            Outcome::Unavailable {
                reason: "preceding context changed"
            }
        );
        assert_eq!(
            &captured.seed_masks[0][..6],
            &[
                DivisionSeed::CLASS,
                DivisionSeed::CLASS,
                DivisionSeed::CLASS,
                before_speed,
                0,
                0,
            ]
        );
        assert_eq!(captured.required_legs, 4);
        assert_eq!(captured.retained_ranges[0], 2);
    }

    #[test]
    fn pre_speed_source_fault_precedes_pending_boundary_verification_failure() {
        let mut captured = fixture();
        let first = captured.ranges[0].as_mut().unwrap();
        first.reports[0].walk.boundary_failure = Some((100, "preceding context changed"));
        first.reports[0].walk.outcome = Outcome::Unavailable {
            reason: "preceding context changed",
        };
        first.reports[0].outcome = first.reports[0].walk.outcome;
        let next = captured.ranges[1].as_mut().unwrap();
        next.reports[0].walk.acquisition_failed = true;
        next.reports[0].walk.visited_sources = 1;
        next.reports[0].walk.required_legs = 1;
        next.reports[0].walk.outcome = Outcome::Unavailable {
            reason: "route unreadable",
        };
        next.reports[0].outcome = next.reports[0].walk.outcome;
        assert_eq!(
            captured.reduce_pass(0),
            Outcome::Unavailable {
                reason: "route unreadable"
            }
        );
        assert_eq!(
            &captured.seed_masks[0][..6],
            &[
                DivisionSeed::CLASS,
                DivisionSeed::CLASS,
                DivisionSeed::CLASS,
                0,
                0,
                0
            ]
        );
        assert_eq!(captured.required_legs, 3);
    }

    #[test]
    fn cold_comparison_excludes_speculative_suffix_after_matching_aborts() {
        let mut captured = fixture();
        captured.checked = true;
        let first = captured.ranges[0].as_mut().unwrap();
        for pass in 0..2 {
            first.reports[pass].walk.acquisition_failed = true;
            first.reports[pass].walk.visited_sources = 1;
            first.reports[pass].walk.required_legs = 1;
            first.reports[pass].walk.outcome = Outcome::Unavailable {
                reason: "same source fault",
            };
            first.reports[pass].outcome = first.reports[pass].walk.outcome;
        }
        for range in captured.ranges[1..3].iter_mut().flatten() {
            range.geometry_same = false;
            range.rows.rows[0].division = 1;
            range.rows.second[0].division = 2;
            range.reports[1].walk.legs_len = 999;
        }
        assert!(!captured.finish_ranges().matched);
        assert_eq!(captured.checked_mismatches(), [0]);
        assert_eq!(
            captured.outcomes(),
            [Outcome::Unavailable {
                reason: "same source fault"
            }]
        );
    }

    #[test]
    fn pooled_workspace_maintenance_retains_independent_cold_geometry() {
        let captured = fixture();
        let mut workspace = Workspace::with_capacities(&captured.capacities, true);
        let allocated = workspace.allocated_bytes();
        workspace.reset();
        workspace.prepare_for_reuse();
        assert!(workspace.geometry_second.is_some());
        assert_eq!(workspace.allocated_bytes(), allocated);
    }
}

const SMALL_READ_BYTES: usize = 32;

#[derive(Clone, Copy, Default)]
struct ModifierSnapshot {
    address: usize,
    header: [u8; 16],
    start: usize,
    len: usize,
    occupied: bool,
}

/// Full raw sorted accumulators, owned only until this pass's direct final
/// verification. A collision or exhausted arena uses the original live reader;
/// no eviction, allocation, partial snapshot, or cross-pass reuse is allowed.
struct ModifierSnapshots {
    entries: Vec<ModifierSnapshot>,
    used: Vec<usize>,
    used_len: usize,
    bytes: Vec<u8>,
    bytes_len: usize,
}

impl ModifierSnapshots {
    fn new(capacity: usize, bytes: usize) -> Self {
        Self {
            entries: vec![ModifierSnapshot::default(); capacity],
            used: vec![0; capacity],
            used_len: 0,
            bytes: vec![0; bytes],
            bytes_len: 0,
        }
    }

    fn lookup(
        &mut self,
        source: &impl Memory,
        address: usize,
        key: u32,
    ) -> Option<Result<engine::modifier_accumulator::Value, &'static str>> {
        if self.entries.is_empty() {
            return None;
        }
        let slot = ((address >> 4) ^ (address >> 17)) % self.entries.len();
        let entry = self.entries[slot];
        if entry.occupied {
            return if entry.address == address {
                Some(engine::modifier_accumulator::lookup_snapshot(
                    &self.bytes[entry.start..entry.start + entry.len],
                    key,
                ))
            } else {
                None
            };
        }
        let Some(header) = engine::read::<[u8; 16]>(source, address) else {
            return Some(Err("modifier accumulator header unreadable or invalid"));
        };
        let Some(array) =
            engine::cpdx_array::Array::from_header(header, engine::modifier_entry::STRIDE)
        else {
            return Some(Err("modifier accumulator header unreadable or invalid"));
        };
        let Some(len) = (array.count as usize).checked_mul(engine::modifier_entry::STRIDE) else {
            return Some(Err("modifier snapshot byte length overflow"));
        };
        let end = self
            .bytes_len
            .checked_add(len)
            .filter(|&end| end <= self.bytes.len())?;
        let bytes = &mut self.bytes[self.bytes_len..end];
        if len != 0 && !engine::read_slice(source, array.data, bytes) {
            return Some(Err("modifier snapshot entries unreadable"));
        }
        if let Err(reason) = engine::modifier_accumulator::validate_snapshot(bytes) {
            return Some(Err(reason));
        }
        if engine::read::<[u8; 16]>(source, address) != Some(header) {
            return Some(Err("modifier accumulator header changed"));
        }
        let result = engine::modifier_accumulator::lookup_snapshot(bytes, key);
        self.entries[slot] = ModifierSnapshot {
            address,
            header,
            start: self.bytes_len,
            len,
            occupied: true,
        };
        self.used[self.used_len] = slot;
        self.used_len += 1;
        self.bytes_len = end;
        Some(result)
    }

    fn clear(&mut self) {
        for &slot in &self.used[..self.used_len] {
            self.entries[slot] = ModifierSnapshot::default();
        }
        self.used_len = 0;
        self.bytes_len = 0;
    }

    fn verify_and_clear(&mut self, source: &impl Memory) -> bool {
        let mut matched = true;
        let mut fresh = [0_u8; 64 * 16];
        for &slot in &self.used[..self.used_len] {
            let entry = self.entries[slot];
            matched &= engine::read::<[u8; 16]>(source, entry.address) == Some(entry.header);
            // The stored header was fully bounded before insertion. Continue
            // checking all retained bytes even if a prior header/read failed.
            let data = u64::from_le_bytes(entry.header[..8].try_into().unwrap()) as usize;
            let mut offset = 0;
            while offset < entry.len {
                let count = (entry.len - offset).min(fresh.len());
                let copied = engine::read_slice(source, data + offset, &mut fresh[..count]);
                matched &= copied
                    && fresh[..count]
                        == self.bytes[entry.start + offset..entry.start + offset + count];
                offset += count;
            }
            matched &= engine::read::<[u8; 16]>(source, entry.address) == Some(entry.header);
            self.entries[slot] = ModifierSnapshot::default();
        }
        self.used_len = 0;
        self.bytes_len = 0;
        matched
    }

    fn grow(&mut self, capacity: usize, bytes: usize) {
        self.clear();
        if capacity > self.entries.len() {
            self.entries.resize(capacity, ModifierSnapshot::default());
            self.used.resize(capacity, 0);
        }
        if bytes > self.bytes.len() {
            self.bytes.resize(bytes, 0);
        }
    }

    fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<ModifierSnapshot>()
            + self.used.capacity() * size_of::<usize>()
            + self.bytes.capacity()
    }
}

#[derive(Clone, Copy, Default)]
struct AirDefenseEntry {
    arguments: [i64; 3],
    value: i64,
    occupied: bool,
}

/// Pure calculation sharing, not an input snapshot. Every lookup follows the
/// original guarded reads; only the deterministic integer series is omitted on
/// a hit. No final recomputation is needed for equal complete scalar arguments.
struct AirDefenseMemo {
    entries: Vec<AirDefenseEntry>,
    used: Vec<usize>,
    used_len: usize,
}
impl AirDefenseMemo {
    fn new(capacity: usize) -> Self {
        Self {
            entries: vec![AirDefenseEntry::default(); capacity],
            used: vec![0; capacity],
            used_len: 0,
        }
    }
    fn resolve(
        &mut self,
        arguments: [i64; 3],
        calculate: impl FnOnce() -> Result<i64, &'static str>,
    ) -> Option<Result<i64, &'static str>> {
        if self.entries.is_empty() {
            return None;
        }
        let mut hash = 0_u64;
        for argument in arguments {
            hash ^= argument as u64;
            hash = hash.wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(23);
        }
        let slot = hash as usize % self.entries.len();
        let entry = self.entries[slot];
        if entry.occupied {
            return if entry.arguments == arguments {
                Some(Ok(entry.value))
            } else {
                None
            };
        }
        let value = match calculate() {
            Ok(value) => value,
            Err(reason) => return Some(Err(reason)),
        };
        self.entries[slot] = AirDefenseEntry {
            arguments,
            value,
            occupied: true,
        };
        self.used[self.used_len] = slot;
        self.used_len += 1;
        Some(Ok(value))
    }
    fn clear(&mut self) {
        for &slot in &self.used[..self.used_len] {
            self.entries[slot] = AirDefenseEntry::default();
        }
        self.used_len = 0;
    }
    fn grow(&mut self, capacity: usize) {
        self.clear();
        if capacity > self.entries.len() {
            self.entries.resize(capacity, AirDefenseEntry::default());
            self.used.resize(capacity, 0);
        }
    }
    fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<AirDefenseEntry>()
            + self.used.capacity() * size_of::<usize>()
    }
}

#[derive(Clone, Copy, Default)]
struct SupplyEntry {
    base: usize,
    state: usize,
    area: usize,
    owner: u32,
    accessible: bool,
    occupied: bool,
}

/// Native country/area predicates shared only during one independently checked
/// pass. Occupied slots are never evicted; every retained result is recomputed
/// through the unshared engine reader before publication, including on errors.
struct SupplyMemo {
    entries: Vec<SupplyEntry>,
    used: Vec<usize>,
    used_len: usize,
}
impl SupplyMemo {
    fn new(capacity: usize) -> Self {
        Self {
            entries: vec![SupplyEntry::default(); capacity],
            used: vec![0; capacity],
            used_len: 0,
        }
    }
    fn resolve(
        &mut self,
        base: usize,
        state: usize,
        owner: u32,
        area: usize,
        calculate: impl FnOnce() -> Result<bool, &'static str>,
    ) -> Option<Result<bool, &'static str>> {
        if self.entries.is_empty() {
            return None;
        }
        let hash = (owner as usize).wrapping_mul(0x9e37_79b9)
            ^ (area >> 3)
            ^ (area >> 19)
            ^ (base >> 16)
            ^ (state >> 3);
        let slot = hash % self.entries.len();
        let entry = self.entries[slot];
        if entry.occupied {
            return if (entry.base, entry.state, entry.owner, entry.area)
                == (base, state, owner, area)
            {
                Some(Ok(entry.accessible))
            } else {
                None
            };
        }
        let accessible = match calculate() {
            Ok(value) => value,
            Err(reason) => return Some(Err(reason)),
        };
        self.entries[slot] = SupplyEntry {
            base,
            state,
            area,
            owner,
            accessible,
            occupied: true,
        };
        self.used[self.used_len] = slot;
        self.used_len += 1;
        Some(Ok(accessible))
    }
    fn clear(&mut self) {
        for &slot in &self.used[..self.used_len] {
            self.entries[slot] = SupplyEntry::default();
        }
        self.used_len = 0;
    }
    fn verify_and_clear(
        &mut self,
        mut calculate: impl FnMut(usize, usize, u32, usize) -> Result<bool, &'static str>,
    ) -> bool {
        let mut matched = true;
        for &slot in &self.used[..self.used_len] {
            let entry = self.entries[slot];
            matched &=
                calculate(entry.base, entry.state, entry.owner, entry.area) == Ok(entry.accessible);
            self.entries[slot] = SupplyEntry::default();
        }
        self.used_len = 0;
        matched
    }
    fn grow(&mut self, capacity: usize) {
        self.clear();
        if capacity > self.entries.len() {
            self.entries.resize(capacity, SupplyEntry::default());
            self.used.resize(capacity, 0);
        }
    }
    fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<SupplyEntry>()
            + self.used.capacity() * size_of::<usize>()
    }
}

#[derive(Clone, Copy, Default)]
struct TransportEntry {
    owner: u32,
    region: usize,
    speed: i64,
    occupied: bool,
}

/// Derived results are local to one base/game-state/pass. Even reads which
/// collided in SmallReadCache participate in the direct final recomputation of
/// every retained result. No eviction or inferred success on failed misses.
struct TransportMemo {
    entries: Vec<TransportEntry>,
    used: Vec<usize>,
    used_len: usize,
}
impl TransportMemo {
    fn new(capacity: usize) -> Self {
        Self {
            entries: vec![TransportEntry::default(); capacity],
            used: vec![0; capacity],
            used_len: 0,
        }
    }
    fn resolve(
        &mut self,
        owner: u32,
        region: usize,
        calculate: impl FnOnce() -> Result<i64, &'static str>,
    ) -> Result<i64, &'static str> {
        if self.entries.is_empty() {
            return calculate();
        }
        let hash = (owner as usize).wrapping_mul(0x9e37_79b9) ^ (region >> 3) ^ (region >> 19);
        let slot = hash % self.entries.len();
        let entry = self.entries[slot];
        if entry.occupied {
            return if entry.owner == owner && entry.region == region {
                Ok(entry.speed)
            } else {
                calculate()
            };
        }
        let speed = calculate()?;
        self.entries[slot] = TransportEntry {
            owner,
            region,
            speed,
            occupied: true,
        };
        self.used[self.used_len] = slot;
        self.used_len += 1;
        Ok(speed)
    }
    fn clear(&mut self) {
        for &slot in &self.used[..self.used_len] {
            self.entries[slot] = TransportEntry::default();
        }
        self.used_len = 0;
    }
    fn verify_and_clear(
        &mut self,
        mut calculate: impl FnMut(u32, usize) -> Result<i64, &'static str>,
    ) -> bool {
        let mut matched = true;
        for &slot in &self.used[..self.used_len] {
            let entry = &mut self.entries[slot];
            let result = calculate(entry.owner, entry.region);
            matched &= result == Ok(entry.speed);
            *entry = TransportEntry::default();
        }
        self.used_len = 0;
        matched
    }
    fn grow(&mut self, capacity: usize) {
        self.clear();
        if self.entries.len() < capacity {
            self.entries.resize(capacity, TransportEntry::default());
            self.used.resize(capacity, 0);
        }
    }
    fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<TransportEntry>()
            + self.used.capacity() * size_of::<usize>()
    }
}

// The largest bulk read in the normal-common graph is a 64-member pointer
// chunk. Larger future requests preserve the original verifier via fallback.
const WITNESS_MAX_READ: usize = 64 * 8;

// Keep terminal scratch bounded: 8 KiB for the widest witness batch, and
// 512 bytes for small-cache payloads. No source gaps or duplicate reads merge.
const WITNESS_BATCH_READS: usize = 16;

fn verify_read_batch<const WIDTH: usize>(
    source: &impl Memory,
    count: usize,
    addresses: [usize; WITNESS_BATCH_READS],
    expected: [&[u8]; WITNESS_BATCH_READS],
) -> bool {
    let mut fresh = [[0; WIDTH]; WITNESS_BATCH_READS];
    let mut matched = true;
    let mut next = 0;
    while next < count {
        let mut fields = fresh.iter_mut().enumerate();
        let copied = source.copy_scattered(
            addresses,
            &mut std::array::from_fn(|_| {
                let (index, bytes) = fields.next().unwrap();
                // Empty destinations preserve descriptor indices without
                // retrying any completed or failed observation.
                let len = if index < next {
                    0
                } else {
                    expected[index].len()
                };
                &mut bytes[..len]
            }),
        );
        // A failed field may contain partial (even apparently equal) bytes.
        // Compare only complete observations, including after earlier failure.
        for index in next..copied.min(count) {
            matched &= &fresh[index][..expected[index].len()] == expected[index];
        }
        if copied >= count {
            break;
        }
        matched = false;
        // Memory leaves the suffix untouched. Skip exactly the failed field
        // and finish that suffix before moving on to the next bounded batch.
        next = copied + 1;
    }
    matched
}

#[derive(Clone, Copy, Default)]
struct WitnessRead {
    address: usize,
    start: usize,
    len: usize,
}

/// Verification-only observations omitted by the exact small-read cache.
/// Never serves acquired data: collisions and bulk reads remain fresh, in
/// their original order. A bounded shortfall disables graph-verifier elision
/// for every affected scope, without changing acquisition outcomes.
struct ReadWitnessJournal {
    entries: Vec<WitnessRead>,
    bytes: Vec<u8>,
    len: usize,
    bytes_len: usize,
    required_entries: usize,
    required_bytes: usize,
    generation: u64,
    uncovered: u64,
    exhausted_counter: bool,
}
impl ReadWitnessJournal {
    fn new(entries: usize, bytes: usize) -> Self {
        Self {
            entries: vec![WitnessRead::default(); entries],
            bytes: vec![0; bytes],
            len: 0,
            bytes_len: 0,
            required_entries: 0,
            required_bytes: 0,
            generation: 1,
            uncovered: 0,
            exhausted_counter: false,
        }
    }
    fn enabled(&self) -> bool {
        !self.entries.is_empty() && !self.bytes.is_empty() && !self.exhausted_counter
    }
    fn miss(&mut self) {
        if let Some(next) = self.uncovered.checked_add(1) {
            self.uncovered = next;
        } else {
            self.exhausted_counter = true;
        }
    }
    fn record(&mut self, address: usize, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.required_entries = self.required_entries.saturating_add(1);
        self.required_bytes = self.required_bytes.saturating_add(bytes.len());
        let end = self.bytes_len.checked_add(bytes.len());
        if bytes.len() > WITNESS_MAX_READ
            || self.len == self.entries.len()
            || end.is_none_or(|end| end > self.bytes.len())
        {
            self.miss();
            return;
        }
        let end = end.unwrap();
        self.bytes[self.bytes_len..end].copy_from_slice(bytes);
        self.entries[self.len] = WitnessRead {
            address,
            start: self.bytes_len,
            len: bytes.len(),
        };
        self.len += 1;
        self.bytes_len = end;
    }
    fn clear(&mut self) {
        self.entries[..self.len].fill(WitnessRead::default());
        self.bytes[..self.bytes_len].fill(0);
        self.len = 0;
        self.bytes_len = 0;
        self.required_entries = 0;
        self.required_bytes = 0;
        self.uncovered = 0;
        if let Some(next) = self.generation.checked_add(1) {
            self.generation = next;
        } else {
            // A generation must never alias a checkpoint from a prior pass.
            self.exhausted_counter = true;
        }
    }
    fn verify_and_clear(&mut self, source: &impl Memory) -> bool {
        let mut matched = true;
        for entries in self.entries[..self.len].chunks(WITNESS_BATCH_READS) {
            let addresses =
                std::array::from_fn(|index| entries.get(index).map_or(0, |entry| entry.address));
            let expected = std::array::from_fn(|index| {
                entries.get(index).map_or(&[][..], |entry| {
                    &self.bytes[entry.start..entry.start + entry.len]
                })
            });
            matched &=
                verify_read_batch::<WITNESS_MAX_READ>(source, entries.len(), addresses, expected);
        }
        self.clear();
        matched
    }
    fn grow(&mut self, entries: usize, bytes: usize) {
        self.clear();
        if entries > self.entries.len() {
            self.entries.resize(entries, WitnessRead::default());
        }
        if bytes > self.bytes.len() {
            self.bytes.resize(bytes, 0);
        }
    }
    fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<WitnessRead>() + self.bytes.capacity()
    }
}

#[derive(Clone, Copy, Default)]
struct CacheKey {
    address: usize,
    len: usize,
}

/// Callback-local exact reads only. Occupied slots are never evicted: a full
/// four-slot neighborhood is acquired normally, leaving retained observations
/// available for mandatory direct verification at the end of each pass.
struct SmallReadCache {
    // Dense metadata avoids fetching the 32-byte payload on misses/collisions.
    keys: Vec<CacheKey>,
    bytes: Vec<[u8; SMALL_READ_BYTES]>,
    used: Vec<usize>,
    used_len: usize,
    mask: Option<usize>,
    witness: ReadWitnessJournal,
    diagnostics: CacheDiagnostics,
}
impl SmallReadCache {
    fn new(capacity: usize) -> Self {
        Self {
            keys: vec![CacheKey::default(); capacity],
            bytes: vec![[0; SMALL_READ_BYTES]; capacity],
            used: vec![0; capacity],
            used_len: 0,
            mask: capacity
                .is_power_of_two()
                .then_some(capacity.wrapping_sub(1)),
            witness: ReadWitnessJournal::new(0, 0),
            diagnostics: CacheDiagnostics::default(),
        }
    }
    fn with_witness(capacity: usize, entries: usize, bytes: usize) -> Self {
        Self {
            witness: ReadWitnessJournal::new(entries, bytes),
            ..Self::new(capacity)
        }
    }
    #[inline]
    fn slot(&self, address: usize, len: usize) -> Option<usize> {
        if self.keys.is_empty() || len == 0 || len > SMALL_READ_BYTES {
            None
        } else {
            let hash = (address >> 3) ^ (address >> 19) ^ len.wrapping_mul(0x9e37_79b9);
            Some(match self.mask {
                Some(mask) => hash & mask,
                None => hash % self.keys.len(),
            })
        }
    }
    fn clear(&mut self) {
        self.diagnostics = CacheDiagnostics::default();
        self.witness.clear();
        for &slot in &self.used[..self.used_len] {
            self.keys[slot] = CacheKey::default();
            self.bytes[slot] = [0; SMALL_READ_BYTES];
        }
        self.used_len = 0;
    }
    fn grow(&mut self, capacity: usize) {
        self.clear();
        if self.keys.len() < capacity {
            self.keys.resize(capacity, CacheKey::default());
            self.bytes.resize(capacity, [0; SMALL_READ_BYTES]);
            self.used.resize(capacity, 0);
            self.mask = capacity
                .is_power_of_two()
                .then_some(capacity.wrapping_sub(1));
        }
    }
    fn allocated_bytes(&self) -> usize {
        self.keys.capacity() * size_of::<CacheKey>()
            + self.bytes.capacity() * size_of::<[u8; SMALL_READ_BYTES]>()
            + self.used.capacity() * size_of::<usize>()
            + self.witness.allocated_bytes()
    }
    fn observe_witness(&mut self) {
        self.diagnostics.witness_entries_peak =
            self.diagnostics.witness_entries_peak.max(self.witness.len);
        self.diagnostics.witness_bytes_peak = self
            .diagnostics
            .witness_bytes_peak
            .max(self.witness.bytes_len);
        self.diagnostics.witness_entries_required = self
            .diagnostics
            .witness_entries_required
            .max(self.witness.required_entries);
        self.diagnostics.witness_bytes_required = self
            .diagnostics
            .witness_bytes_required
            .max(self.witness.required_bytes);
    }
    fn verify_and_clear(&mut self, memory: &impl Memory) -> bool {
        self.observe_witness();
        let mut matched = self.witness.verify_and_clear(memory);
        for slots in self.used[..self.used_len].chunks(WITNESS_BATCH_READS) {
            let addresses = std::array::from_fn(|index| {
                slots.get(index).map_or(0, |&slot| self.keys[slot].address)
            });
            let expected = std::array::from_fn(|index| {
                slots
                    .get(index)
                    .map_or(&[][..], |&slot| &self.bytes[slot][..self.keys[slot].len])
            });
            // Every retained read remains independent, even after a witness
            // failure or a mismatch in an earlier batch.
            matched &=
                verify_read_batch::<SMALL_READ_BYTES>(memory, slots.len(), addresses, expected);
            for &slot in slots {
                self.keys[slot] = CacheKey::default();
                self.bytes[slot] = [0; SMALL_READ_BYTES];
            }
        }
        self.used_len = 0;
        matched
    }
}

struct CacheRef<'a, M> {
    source: &'a M,
    cache: &'a RefCell<SmallReadCache>,
    modifiers: Option<&'a RefCell<ModifierSnapshots>>,
    supply: Option<&'a RefCell<SupplyMemo>>,
    air_defense: Option<&'a RefCell<AirDefenseMemo>>,
    primary: Option<&'a RefCell<engine::carmy_leader::PrimaryRootCache>>,
}
impl<M: Memory> CacheRef<'_, M> {
    // The lookup borrow is released before entry. Carry its first empty slot
    // through the source read without another hash, probe or lookup borrow.
    #[inline(never)]
    fn copy_miss(&self, address: usize, destination: &mut [u8], vacant: Option<usize>) -> bool {
        if !self.source.copy(address, destination) {
            self.cache.borrow_mut().witness.miss();
            return false;
        }
        let mut cache = self.cache.borrow_mut();
        if let Some(slot) = vacant {
            let key = CacheKey {
                address,
                len: destination.len(),
            };
            let mut bytes = [0; SMALL_READ_BYTES];
            bytes[..destination.len()].copy_from_slice(destination);
            cache.bytes[slot] = bytes;
            cache.keys[slot] = key;
            let next = cache.used_len;
            cache.used[next] = slot;
            cache.used_len += 1;
            cache.diagnostics.retained_misses = cache.diagnostics.retained_misses.saturating_add(1);
        } else {
            cache.witness.record(address, destination);
        }
        true
    }
}
impl<M: Memory> Memory for CacheRef<'_, M> {
    fn finish_read_witness(
        &self,
        witness: engine::memory::ReadWitness,
    ) -> Option<Result<(), &'static str>> {
        let mut cache = self.cache.borrow_mut();
        if !cache.witness.enabled()
            || witness.owner != self.cache as *const RefCell<SmallReadCache> as usize
            || witness.generation != cache.witness.generation
        {
            return None;
        }
        let complete = witness.uncovered == cache.witness.uncovered;
        cache.observe_witness();
        // Only uncached collision/bulk observations are scoped. Exact scalar
        // cache entries and typed shared dependencies retain their independent
        // terminal checks; this never evicts or restages them for every army.
        let started = qpc();
        let matched = cache.witness.verify_and_clear(self.source);
        cache.diagnostics.stages.scope_verify.record(started);
        if !matched {
            return Some(Err("division ETA owned dependencies changed or unreadable"));
        }
        if complete {
            cache.diagnostics.witness_skips = cache.diagnostics.witness_skips.saturating_add(1);
            Some(Ok(()))
        } else {
            // The original complete-check path records this fallback once.
            None
        }
    }
    fn movement_primary_root(
        &self,
        base: usize,
        member: usize,
    ) -> Option<Result<usize, &'static str>> {
        let primary = self.primary?;
        Some(measure_stage(
            self.cache,
            |stage| &mut stage.primary_acquire,
            || primary.borrow_mut().resolve(self, base, member),
        ))
    }
    fn read_witness(&self) -> Option<engine::memory::ReadWitness> {
        let cache = self.cache.borrow();
        cache
            .witness
            .enabled()
            .then_some(engine::memory::ReadWitness {
                owner: self.cache as *const RefCell<SmallReadCache> as usize,
                generation: cache.witness.generation,
                uncovered: cache.witness.uncovered,
            })
    }
    fn read_witness_complete(&self, witness: engine::memory::ReadWitness) -> bool {
        let mut cache = self.cache.borrow_mut();
        let complete = cache.witness.enabled()
            && witness.owner == self.cache as *const RefCell<SmallReadCache> as usize
            && witness.generation == cache.witness.generation
            && witness.uncovered == cache.witness.uncovered;
        if complete {
            cache.diagnostics.witness_skips = cache.diagnostics.witness_skips.saturating_add(1);
        } else {
            cache.diagnostics.witness_fallbacks =
                cache.diagnostics.witness_fallbacks.saturating_add(1);
        }
        complete
    }
    fn movement_air_defense_value(
        &self,
        defense: i64,
        denominator: i64,
        reduction: i64,
    ) -> Option<Result<i64, &'static str>> {
        self.air_defense?
            .borrow_mut()
            .resolve([defense, denominator, reduction], || {
                engine::csub_unit_definition::movement_air_defense_value(
                    defense,
                    denominator,
                    reduction,
                )
            })
    }
    fn core_supply_access(
        &self,
        base: usize,
        state: usize,
        owner: u32,
        area: usize,
    ) -> Option<Result<bool, &'static str>> {
        let supply = self.supply?;
        let result = measure_stage(
            self.cache,
            |stage| &mut stage.supply_acquire,
            || {
                supply.borrow_mut().resolve(base, state, owner, area, || {
                    let mut cache = self.cache.borrow_mut();
                    cache.diagnostics.stages.supply_builds =
                        cache.diagnostics.stages.supply_builds.saturating_add(1);
                    drop(cache);
                    engine::ccountry::has_core_supply_access_direct(self, base, state, owner, area)
                })
            },
        );
        if result.is_none() {
            let mut cache = self.cache.borrow_mut();
            cache.diagnostics.stages.supply_fallbacks =
                cache.diagnostics.stages.supply_fallbacks.saturating_add(1);
        }
        result
    }
    fn modifier_accumulator_value(
        &self,
        address: usize,
        key: u32,
    ) -> Option<Result<engine::modifier_accumulator::Value, &'static str>> {
        let modifiers = self.modifiers?;
        let result = measure_stage(
            self.cache,
            |stage| &mut stage.modifiers_acquire,
            || modifiers.borrow_mut().lookup(self.source, address, key),
        );
        if result.is_none() {
            let mut cache = self.cache.borrow_mut();
            cache.diagnostics.stages.modifier_fallbacks = cache
                .diagnostics
                .stages
                .modifier_fallbacks
                .saturating_add(1);
        }
        result
    }
    #[inline(always)]
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let mut cache = self.cache.borrow_mut();
        let mut vacant = None;
        if let Some(mut slot) = cache.slot(address, destination.len()) {
            // The home hash is unchanged. Probe only adjacent dense metadata,
            // without eviction or another division for non-power-of-two sizes.
            for _ in 0..cache.keys.len().min(4) {
                let key = &cache.keys[slot];
                if key.len == destination.len() && key.address == address {
                    destination.copy_from_slice(&cache.bytes[slot][..key.len]);
                    cache.diagnostics.hits = cache.diagnostics.hits.saturating_add(1);
                    return true;
                }
                if key.len == 0 {
                    // First-empty insertion and no in-pass deletion mean an
                    // exact key cannot occur later in this probe sequence.
                    vacant = Some(slot);
                    break;
                }
                slot += 1;
                if slot == cache.keys.len() {
                    slot = 0;
                }
            }
        }
        if vacant.is_none() && !destination.is_empty() {
            cache.diagnostics.direct_reads = cache.diagnostics.direct_reads.saturating_add(1);
        }
        drop(cache);
        self.copy_miss(address, destination, vacant)
    }
    fn authenticated_countries(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<engine::ccountry::AuthenticatedRoster<'_>> {
        self.source.authenticated_countries(base, game_state)
    }
    // No immutable_view: callers must not bypass acquisition or final checks.
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::frame::TableView;
    use replay::rows::TableRows;

    struct TerminalMemory {
        reads: RefCell<Vec<(usize, usize)>>,
        scalar_calls: Cell<usize>,
        batches: Cell<usize>,
        faults: Cell<u64>,
        partial: Cell<usize>,
        changed: Cell<Option<usize>>,
        epoch: Cell<u8>,
        ordinal_bytes: bool,
    }
    impl TerminalMemory {
        fn new(ordinal_bytes: bool) -> Self {
            Self {
                reads: RefCell::new(Vec::new()),
                scalar_calls: Cell::new(0),
                batches: Cell::new(0),
                faults: Cell::new(0),
                partial: Cell::new(0),
                changed: Cell::new(None),
                epoch: Cell::new(0),
                ordinal_bytes,
            }
        }
        fn reset_calls(&self) {
            self.reads.borrow_mut().clear();
            self.scalar_calls.set(0);
            self.batches.set(0);
        }
        fn observe(&self, address: usize, destination: &mut [u8]) -> bool {
            let ordinal = self.reads.borrow().len();
            self.reads.borrow_mut().push((address, destination.len()));
            let failed = ordinal < 64 && self.faults.get() & (1_u64 << ordinal) != 0;
            let copied = if failed {
                self.partial.get().min(destination.len())
            } else {
                destination.len()
            };
            for (index, byte) in destination.iter_mut().take(copied).enumerate() {
                *byte = (address as u8)
                    .wrapping_add(index as u8)
                    .wrapping_add(self.epoch.get())
                    .wrapping_add(if self.ordinal_bytes { ordinal as u8 } else { 0 });
            }
            if !failed
                && self.changed.get() == Some(ordinal)
                && let Some(last) = destination.last_mut()
            {
                *last ^= 1;
            }
            !failed
        }
    }
    impl Memory for TerminalMemory {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.scalar_calls.set(self.scalar_calls.get() + 1);
            self.observe(address, destination)
        }
        fn copy_scattered<const K: usize>(
            &self,
            addresses: [usize; K],
            destinations: &mut [&mut [u8]; K],
        ) -> usize {
            self.batches.set(self.batches.get() + 1);
            for (completed, (address, destination)) in addresses
                .into_iter()
                .zip(destinations.iter_mut())
                .enumerate()
            {
                if destination.is_empty() {
                    continue;
                }
                if address == 0
                    || address.checked_add(destination.len()).is_none()
                    || !self.observe(address, destination)
                {
                    return completed;
                }
            }
            K
        }
    }
    fn acquire_terminal_journal(
        memory: &impl Memory,
        spans: &[(usize, usize)],
    ) -> ReadWitnessJournal {
        let mut journal =
            ReadWitnessJournal::new(spans.len(), spans.iter().map(|&(_, len)| len).sum());
        let mut bytes = [0; WITNESS_MAX_READ];
        for &(address, len) in spans {
            assert!(memory.copy(address, &mut bytes[..len]));
            journal.record(address, &bytes[..len]);
        }
        journal
    }
    fn acquire_terminal_cache(memory: &impl Memory, spans: &[(usize, usize)]) -> SmallReadCache {
        let cache = RefCell::new(SmallReadCache::new(128));
        {
            let cached = CacheRef {
                source: memory,
                cache: &cache,
                modifiers: None,
                supply: None,
                air_defense: None,
                primary: None,
            };
            let mut bytes = [0; SMALL_READ_BYTES];
            for &(address, len) in spans {
                assert!(cached.copy(address, &mut bytes[..len]));
            }
        }
        assert_eq!(cache.borrow().used_len, spans.len());
        cache.into_inner()
    }

    #[test]
    fn witness_terminal_batches_keep_duplicates_overlaps_widths_and_differences() {
        let count = 2 * WITNESS_BATCH_READS + 3;
        let widths = [1, 2, 4, 8, 12, 16, 24, 32, 33, 64, 511, 512];
        let spans: Vec<_> = (0..count)
            .map(|index| (0x1000 + 2 * (index % 3), widths[index % widths.len()]))
            .collect();
        for changed in [None, Some(0), Some(15), Some(16), Some(count - 1)] {
            // Repeated equal address/width pairs have different observed bytes.
            let source = TerminalMemory::new(true);
            let mut journal = acquire_terminal_journal(&source, &spans);
            let storage = (journal.entries.as_ptr(), journal.bytes.as_ptr());
            let allocated = journal.allocated_bytes();
            let generation = journal.generation;
            source.reset_calls();
            source.changed.set(changed);
            assert_eq!(journal.verify_and_clear(&source), changed.is_none());
            assert_eq!(*source.reads.borrow(), spans);
            assert_eq!(source.scalar_calls.get(), 0);
            assert_eq!(source.batches.get(), count.div_ceil(WITNESS_BATCH_READS));
            assert_eq!((journal.len, journal.bytes_len), (0, 0));
            assert_eq!((journal.required_entries, journal.required_bytes), (0, 0));
            assert_eq!(journal.generation, generation + 1);
            assert!(
                journal
                    .entries
                    .iter()
                    .all(|entry| { entry.address == 0 && entry.start == 0 && entry.len == 0 })
            );
            assert!(journal.bytes.iter().all(|&byte| byte == 0));
            assert_eq!(storage, (journal.entries.as_ptr(), journal.bytes.as_ptr()));
            assert_eq!(journal.allocated_bytes(), allocated);
        }
    }

    #[test]
    fn small_cache_terminal_batches_preserve_used_order_and_reduce_calls() {
        let count = 2 * WITNESS_BATCH_READS + 3;
        let spans: Vec<_> = (0..count)
            .map(|index| (0x2000 + 8 * ((index * 11) % count), 8))
            .collect();
        for changed in [None, Some(0), Some(16), Some(count - 1)] {
            let source = TerminalMemory::new(true);
            let mut cache = acquire_terminal_cache(&source, &spans);
            let allocated = cache.allocated_bytes();
            let storage = (
                cache.keys.as_ptr(),
                cache.bytes.as_ptr(),
                cache.used.as_ptr(),
            );
            source.reset_calls();
            source.changed.set(changed);
            assert_eq!(cache.verify_and_clear(&source), changed.is_none());
            assert_eq!(*source.reads.borrow(), spans);
            assert_eq!(source.scalar_calls.get(), 0);
            assert_eq!(source.batches.get(), count.div_ceil(WITNESS_BATCH_READS));
            assert_eq!(cache.used_len, 0);
            assert!(
                cache
                    .keys
                    .iter()
                    .all(|key| key.address == 0 && key.len == 0)
            );
            assert!(cache.bytes.iter().flatten().all(|&byte| byte == 0));
            assert_eq!(cache.diagnostics.retained_misses, count as u64);
            assert_eq!(cache.diagnostics.direct_reads, 0);
            assert_eq!(cache.diagnostics.hits, 0);
            assert_eq!(cache.allocated_bytes(), allocated);
            assert_eq!(
                storage,
                (
                    cache.keys.as_ptr(),
                    cache.bytes.as_ptr(),
                    cache.used.as_ptr()
                )
            );
        }
    }

    #[test]
    fn terminal_faults_skip_failed_fields_finish_suffixes_and_preserve_byte_counts() {
        let count = WITNESS_BATCH_READS + 3;
        let spans: Vec<_> = (0..count).map(|index| (0x2000 + index * 8, 8)).collect();
        let faults = [
            1_u64,
            1 << (WITNESS_BATCH_READS / 2),
            1 << (WITNESS_BATCH_READS - 1),
            1 << (count - 1),
            (1 << 1) | (1 << 2) | (1 << (WITNESS_BATCH_READS + 1)),
            (1 << count) - 1,
        ];
        for as_cache in [false, true] {
            for faults in faults {
                // A failed copy can leave zero, partial, or all expected bytes.
                for partial in [0, 3, 8] {
                    let source = TerminalMemory::new(false);
                    let counted = crate::division_detail::CountedMemory::new(&source);
                    let mut journal = ReadWitnessJournal::new(0, 0);
                    let mut cache = SmallReadCache::new(0);
                    if as_cache {
                        cache = acquire_terminal_cache(&counted, &spans);
                    } else {
                        journal = acquire_terminal_journal(&counted, &spans);
                    }
                    let before = counted.bytes();
                    source.reset_calls();
                    source.faults.set(faults);
                    source.partial.set(partial);
                    let matched = if as_cache {
                        cache.verify_and_clear(&counted)
                    } else {
                        journal.verify_and_clear(&counted)
                    };
                    assert!(!matched);
                    assert_eq!(*source.reads.borrow(), spans);
                    assert_eq!(source.scalar_calls.get(), 0);
                    assert_eq!(
                        counted.bytes() - before,
                        (count as u64 - u64::from(faults.count_ones())) * 8
                    );
                    let continuations = (0..count)
                        .filter(|&index| {
                            faults & (1 << index) != 0
                                && !(index + 1).is_multiple_of(WITNESS_BATCH_READS)
                                && index + 1 < count
                        })
                        .count();
                    assert_eq!(
                        source.batches.get(),
                        count.div_ceil(WITNESS_BATCH_READS) + continuations
                    );
                    assert_eq!((journal.len, journal.bytes_len, cache.used_len), (0, 0, 0));
                    assert!(journal.bytes.iter().all(|&byte| byte == 0));
                    assert!(
                        cache
                            .keys
                            .iter()
                            .all(|key| key.address == 0 && key.len == 0)
                    );
                    assert!(cache.bytes.iter().flatten().all(|&byte| byte == 0));
                }
            }
        }
    }

    #[test]
    fn terminal_batches_use_the_existing_memory_default_for_scalar_sources() {
        let source = Observed::new();
        let spans = [(16, 8), (24, 8), (32, 8), (40, 8)];
        let mut journal = acquire_terminal_journal(&source, &spans);
        source.calls.borrow_mut().clear();
        source.fault.set(Some(24));
        assert!(!journal.verify_and_clear(&source));
        assert_eq!(*source.calls.borrow(), spans);
        source.fault.set(None);
        let mut cache = acquire_terminal_cache(&source, &spans);
        source.calls.borrow_mut().clear();
        source.fault.set(Some(24));
        assert!(!cache.verify_and_clear(&source));
        assert_eq!(*source.calls.borrow(), spans);
    }

    #[test]
    fn empty_terminal_batches_clear_generations_without_source_calls() {
        let source = TerminalMemory::new(false);
        let mut journal = ReadWitnessJournal::new(2, 64);
        journal.record(usize::MAX, &[]);
        let generation = journal.generation;
        assert!(journal.verify_and_clear(&source));
        assert_eq!(journal.generation, generation + 1);
        let mut cache = SmallReadCache::with_witness(0, 2, 64);
        assert!(cache.verify_and_clear(&source));
        assert!(source.reads.borrow().is_empty());
        assert_eq!((source.scalar_calls.get(), source.batches.get()), (0, 0));
    }

    #[test]
    fn scoped_terminal_batches_keep_witness_order_and_independent_fresh_passes() {
        let source = TerminalMemory::new(false);
        let cache = RefCell::new(SmallReadCache::with_witness(1, 4, 64));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let stale = memory.read_witness().unwrap();
        for epoch in [0_u8, 10] {
            source.epoch.set(epoch);
            let mark = memory.read_witness().unwrap();
            let mut bytes = [0; 8];
            assert!(memory.copy(0x2010, &mut bytes));
            assert_eq!(bytes[0], 0x10 + epoch);
            assert!(memory.copy(0x2020, &mut bytes));
            assert!(memory.copy(0x2020, &mut bytes));
            source.reset_calls();
            assert_eq!(memory.finish_read_witness(mark), Some(Ok(())));
            assert_eq!(*source.reads.borrow(), [(0x2020, 8), (0x2020, 8)]);
            assert_eq!(source.batches.get(), 1);
            assert_eq!(cache.borrow().used_len, 1);
            assert_eq!(cache.borrow().witness.len, 0);
            assert_eq!(cache.borrow().diagnostics.witness_entries_peak, 2);
            assert_eq!(cache.borrow().diagnostics.witness_bytes_peak, 16);
            source.reset_calls();
            assert!(cache.borrow_mut().verify_and_clear(&source));
            assert_eq!(*source.reads.borrow(), [(0x2010, 8)]);
            assert_eq!((source.scalar_calls.get(), source.batches.get()), (0, 1));
        }
        assert!(!memory.read_witness_complete(stale));
        assert!(memory.copy(0x2010, &mut [0; 8]));
        source.epoch.set(11);
        assert!(!cache.borrow_mut().verify_and_clear(&source));
        assert!(memory.copy(0x2010, &mut [0; 8]));
        assert!(cache.borrow_mut().verify_and_clear(&source));
    }

    #[test]
    fn conversion_sorts_census_order_and_rejects_duplicate_handles() {
        let mut capture = Captured::with_capacities(
            &Capacities {
                rows: 3,
                legs: 0,
                cache_entries: 0,
                witness_entries: 0,
                witness_bytes: 0,
                transport_entries: 0,
                modifier_accumulators: 0,
                modifier_bytes: 0,
                supply_entries: 0,
                air_defense_entries: 0,
                primary_roots: 0,
                primary_chunks: 0,
            },
            &[0],
        );
        for (row, division) in capture.ranges[0]
            .as_mut()
            .unwrap()
            .rows
            .rows
            .iter_mut()
            .zip([9, 2, 7])
        {
            row.division = division;
        }
        for outcome in [
            Outcome::Rows { len: 3 },
            Outcome::Truncated {
                len: 3,
                unread_rows: 4,
            },
        ] {
            capture.outcome = outcome;
            let [view] = capture.tables();
            let rows = match view {
                TableView::Complete(TableRows::DivisionEta(rows)) => rows,
                TableView::Truncated {
                    rows: TableRows::DivisionEta(rows),
                    missing: 4,
                } => rows,
                other => panic!("unexpected ETA view {other:?}"),
            };
            assert_eq!(
                rows.iter().map(|r| r.division).collect::<Vec<_>>(),
                [2, 7, 9]
            );
        }
        capture.ranges[0].as_mut().unwrap().rows.rows[2].division = 2;
        assert!(matches!(capture.tables(), [TableView::Unavailable { .. }]));
        capture.outcome = Outcome::Truncated {
            len: 0,
            unread_rows: 0,
        };
        assert!(matches!(capture.tables(), [TableView::Unavailable { .. }]));
    }
    use crate::sampling::CONSISTENCY_TARGET;
    use std::cell::Cell;

    #[test]
    fn shared_seed_terminal_changes_reject_only_eta_consumed_fields() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0]);
        captured.outcome = Outcome::Rows { len: 1 };
        captured.checked = true;
        captured.seed_masks[0][0] = DivisionSeed::CLASS | DivisionSeed::ORDINARY_ROUTE;
        assert!(captured.reject_changed_seeds(0, &[DivisionSeed::ORGANISATION]));
        assert_eq!(captured.outcome, Outcome::Rows { len: 1 });
        assert!(!captured.reject_changed_seeds(0, &[DivisionSeed::CLASS]));
        assert!(matches!(captured.outcome, Outcome::Unavailable { .. }));
        assert_eq!(captured.mismatches, 1);
    }

    #[test]
    fn eta_source_masks_are_independent_of_private_row_capacity() {
        let mut captured = Captured::empty();
        captured.reserve_sources(3);
        assert_eq!(captured.ranges[0].as_mut().unwrap().rows.rows.len(), 0);
        assert_eq!(captured.ranges[0].as_mut().unwrap().rows.second.len(), 0);
        assert_eq!(captured.consumed_seed_masks(0).len(), 3);
        assert_eq!(captured.consumed_seed_masks(1).len(), 3);
    }

    #[test]
    fn missing_eta_seed_source_does_not_acquire_geometry() {
        let source = Observed::new();
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0]);
        let missing = Outcome::Unavailable {
            reason: "missing authenticated seeds",
        };
        captured.capture(
            &source,
            0,
            usize::MAX,
            &[true],
            Sources {
                first: &[],
                first_outcome: missing,
                second: &[],
                second_outcome: missing,
            },
        );
        assert_eq!(captured.outcome, missing);
        assert!(source.calls.borrow().is_empty());
    }

    struct Observed {
        bytes: RefCell<[u8; 256]>,
        calls: RefCell<Vec<(usize, usize)>>,
        fault: Cell<Option<usize>>,
    }
    impl Observed {
        fn new() -> Self {
            Self {
                bytes: RefCell::new(std::array::from_fn(|i| i as u8)),
                calls: RefCell::new(Vec::new()),
                fault: Cell::new(None),
            }
        }
        fn accumulator(&self, address: usize, data: usize, values: &[(u32, i64)]) {
            let mut bytes = self.bytes.borrow_mut();
            bytes[address..address + 16].fill(0);
            bytes[address..address + 8].copy_from_slice(&(data as u64).to_le_bytes());
            bytes[address + 8..address + 12].copy_from_slice(&(values.len() as u32).to_le_bytes());
            bytes[address + 12..address + 16].copy_from_slice(&(values.len() as u32).to_le_bytes());
            for (index, (key, raw)) in values.iter().enumerate() {
                let at = data + index * 16;
                bytes[at..at + 16].fill(0);
                bytes[at..at + 4].copy_from_slice(&key.to_le_bytes());
                bytes[at + 8..at + 16].copy_from_slice(&raw.to_le_bytes());
            }
        }
    }
    impl Memory for Observed {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, destination.len()));
            if self.fault.get() == Some(address) {
                destination.fill(255);
                return false;
            }
            let Some(end) = address.checked_add(destination.len()) else {
                return false;
            };
            let bytes = self.bytes.borrow();
            let Some(source) = bytes.get(address..end) else {
                return false;
            };
            destination.copy_from_slice(source);
            true
        }
    }

    #[test]
    fn scoped_dependency_witness_reuses_bounded_storage_without_replaying_graphs() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::with_witness(0, 1, 8));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let allocated = cache.borrow().allocated_bytes();
        let mut previous = None;
        for address in [8, 16, 24, 32] {
            let mark = memory.read_witness().unwrap();
            assert!(memory.copy(address, &mut [0; 8]));
            if let Some(stale) = previous {
                assert_eq!(memory.finish_read_witness(stale), None);
                assert_eq!(cache.borrow().witness.len, 1);
            }
            assert_eq!(memory.finish_read_witness(mark), Some(Ok(())));
            assert_eq!(cache.borrow().witness.len, 0);
            previous = Some(mark);
        }
        assert_eq!(
            &*source.calls.borrow(),
            &[
                (8, 8),
                (8, 8),
                (16, 8),
                (16, 8),
                (24, 8),
                (24, 8),
                (32, 8),
                (32, 8)
            ]
        );
        assert_eq!(cache.borrow().allocated_bytes(), allocated);
        assert_eq!(cache.borrow().diagnostics.witness_skips, 4);
        assert_eq!(cache.borrow().diagnostics.witness_entries_peak, 1);
        assert!(cache.borrow_mut().verify_and_clear(&source));
    }

    #[test]
    fn scoped_dependency_witness_preserves_incomplete_and_failed_observations() {
        for fault in [false, true] {
            let source = Observed::new();
            let cache = RefCell::new(SmallReadCache::with_witness(0, 1, 8));
            let memory = CacheRef {
                source: &source,
                cache: &cache,
                modifiers: None,
                supply: None,
                air_defense: None,
                primary: None,
            };
            let mark = memory.read_witness().unwrap();
            assert!(memory.copy(8, &mut [0; 8]));
            assert!(memory.copy(16, &mut [0; 8]));
            let mut foreign = mark;
            foreign.owner ^= 1;
            assert_eq!(memory.finish_read_witness(foreign), None);
            assert_eq!(cache.borrow().witness.len, 1);
            if fault {
                source.fault.set(Some(8));
            }
            assert_eq!(
                memory.finish_read_witness(mark),
                if fault {
                    Some(Err("division ETA owned dependencies changed or unreadable"))
                } else {
                    None
                }
            );
            assert_eq!(cache.borrow().witness.len, 0);
            let mark = memory.read_witness().unwrap();
            assert!(memory.copy(24, &mut [0; 8]));
            source.bytes.borrow_mut()[24] ^= 1;
            assert_eq!(
                memory.finish_read_witness(mark),
                Some(Err("division ETA owned dependencies changed or unreadable"))
            );
            assert_eq!(cache.borrow().witness.len, 0);
        }
    }

    #[test]
    fn primary_root_hook_uses_cache_ref_and_independent_terminal_checks_without_reentry() {
        use engine::{corder_instance as order, corders_group as group};
        const MEMBER: usize = 0x100;
        const GROUP: usize = 0x1000;
        const ROOT: usize = 0x2000;
        const ROOTS: usize = 0x3000;
        const MEMBERS: usize = 0x4000;
        struct Source {
            bytes: RefCell<Vec<u8>>,
            calls: RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for Source {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.calls.borrow_mut().push((address, destination.len()));
                engine::memory::SliceMemory {
                    base: 0,
                    bytes: &self.bytes.borrow(),
                }
                .copy(address, destination)
            }
        }
        let mut bytes = vec![0; 0x5000];
        for (address, value) in [
            (MEMBER + engine::corders_group_member::GROUP, GROUP as u64),
            (GROUP, group::VTABLE_RVA as u64),
            (GROUP + group::HANDLE, group::REFERENCE_TYPE as u64),
            (GROUP + group::PRIMARY_ORDER_DATA, ROOTS as u64),
            (GROUP + group::PRIMARY_ORDER_DATA + 8, (1_u64 << 32) | 1),
            (ROOTS, ROOT as u64),
            (ROOT, order::VTABLE_RVA as u64),
            (ROOT + order::OWNER, GROUP as u64),
            (ROOT + order::ASSIGNED_MEMBER_DATA, MEMBERS as u64),
            (ROOT + order::ASSIGNED_MEMBER_DATA + 8, (1_u64 << 32) | 1),
            (MEMBERS, MEMBER as u64),
        ] {
            bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes[MEMBER + 0x28] = 1;
        let source = Source {
            bytes: RefCell::new(bytes),
            calls: RefCell::new(Vec::new()),
        };
        let cache = RefCell::new(SmallReadCache::with_witness(128, 8, 512));
        let primary = RefCell::new(engine::carmy_leader::PrimaryRootCache::new(8, 4));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: Some(&primary),
        };
        let allocated = primary.borrow().allocated_bytes() + cache.borrow().allocated_bytes();
        let witness = memory.read_witness().unwrap();
        assert_eq!(memory.movement_primary_root(0, MEMBER), Some(Ok(ROOT)));
        assert_eq!(memory.movement_primary_root(0, MEMBER), Some(Ok(ROOT)));
        assert!(memory.read_witness_complete(witness));
        assert_eq!(
            source
                .calls
                .borrow()
                .iter()
                .filter(|(address, _)| *address == MEMBERS)
                .count(),
            1
        );
        source.bytes.borrow_mut()[MEMBERS..MEMBERS + 8].fill(0);
        assert!(!primary.borrow_mut().verify_and_clear(&source));
        let before = source.calls.borrow().len();
        // A failed typed verifier cannot prevent the scalar verifier from
        // checking and releasing its overlapping retained observations.
        assert!(!cache.borrow_mut().verify_and_clear(&source));
        assert!(source.calls.borrow().len() > before);
        assert_eq!(cache.borrow().used_len, 0);
        assert_eq!(memory.movement_primary_root(0, MEMBER), Some(Ok(0)));
        assert!(primary.borrow_mut().verify_and_clear(&source));
        assert!(cache.borrow_mut().verify_and_clear(&source));
        assert_eq!(
            primary.borrow().allocated_bytes() + cache.borrow().allocated_bytes(),
            allocated
        );
    }

    #[test]
    fn witness_keeps_collision_and_bulk_reads_fresh_and_checks_every_observation() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::with_witness(1, 8, 256));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let mark = memory.read_witness().unwrap();
        assert!(memory.copy(8, &mut [0; 8]));
        assert!(memory.copy(8, &mut [0; 8]));
        let mut bytes = [0; 8];
        assert!(memory.copy(16, &mut bytes));
        assert_eq!(bytes[0], 16);
        source.bytes.borrow_mut()[16] = 99;
        assert!(memory.copy(16, &mut bytes));
        assert_eq!(
            bytes[0], 99,
            "journal must never serve cached collision data"
        );
        assert!(memory.copy(64, &mut [0; 64]));
        assert_eq!(
            &*source.calls.borrow(),
            &[(8, 8), (16, 8), (16, 8), (64, 64)]
        );
        assert!(memory.read_witness_complete(mark));
        assert!(!cache.borrow_mut().verify_and_clear(&source));
        let diagnostics = cache.borrow().diagnostics;
        assert_eq!(diagnostics.hits, 1);
        assert_eq!(diagnostics.retained_misses, 1);
        assert_eq!(diagnostics.direct_reads, 3);
        assert_eq!(diagnostics.witness_skips, 1);
        assert_eq!(diagnostics.witness_entries_peak, 3);
        assert_eq!(diagnostics.witness_bytes_peak, 80);
        assert_eq!(diagnostics.witness_entries_required, 3);
        assert_eq!(diagnostics.witness_bytes_required, 80);
        assert!(!memory.read_witness_complete(mark));
        assert_eq!(cache.borrow().witness.len, 0);
        assert!(cache.borrow().witness.bytes.iter().all(|&byte| byte == 0));
    }

    #[test]
    fn witness_terminal_fault_does_not_skip_later_journal_or_small_cache_reads() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::with_witness(1, 8, 256));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let mark = memory.read_witness().unwrap();
        for address in [8, 16, 24, 32] {
            assert!(memory.copy(address, &mut [0; 8]));
        }
        assert!(memory.read_witness_complete(mark));
        source.calls.borrow_mut().clear();
        source.fault.set(Some(16));
        source.bytes.borrow_mut()[24] ^= 1;
        assert!(!cache.borrow_mut().verify_and_clear(&source));
        assert_eq!(
            &*source.calls.borrow(),
            &[(16, 8), (24, 8), (32, 8), (8, 8)]
        );
        assert_eq!(cache.borrow().witness.len, 0);
        assert_eq!(cache.borrow().used_len, 0);
    }

    #[test]
    fn witness_capacity_and_failed_initial_reads_force_original_verifier_fallback() {
        for (entries, bytes) in [(1, 128), (8, 8)] {
            let source = Observed::new();
            let cache = RefCell::new(SmallReadCache::with_witness(0, entries, bytes));
            let memory = CacheRef {
                source: &source,
                cache: &cache,
                modifiers: None,
                supply: None,
                air_defense: None,
                primary: None,
            };
            let mark = memory.read_witness().unwrap();
            assert!(memory.copy(16, &mut [0; 8]));
            assert!(memory.read_witness_complete(mark));
            assert!(memory.copy(32, &mut [0; 8]));
            assert!(!memory.read_witness_complete(mark));
            assert_eq!(cache.borrow().witness.len, 1);
            assert!(cache.borrow_mut().verify_and_clear(&source));
            let diagnostics = cache.borrow().diagnostics;
            assert_eq!(diagnostics.witness_entries_required, 2);
            assert_eq!(diagnostics.witness_bytes_required, 16);
            assert_eq!(diagnostics.witness_fallbacks, 1);
            assert!(!memory.read_witness_complete(mark));
            let next = memory.read_witness().unwrap();
            source.fault.set(Some(64));
            assert!(!memory.copy(64, &mut [0; 8]));
            assert!(!memory.read_witness_complete(next));
            assert_eq!(
                cache.borrow().witness.len,
                0,
                "failed partial bytes must not be retained"
            );
            assert!(cache.borrow_mut().verify_and_clear(&source));
        }
    }

    #[test]
    fn witness_maximum_span_and_disabled_or_foreign_checkpoints_fail_closed() {
        let bytes = [7; WITNESS_MAX_READ + 1];
        let source = engine::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let cache = RefCell::new(SmallReadCache::with_witness(0, 4, 2 * bytes.len()));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let mark = memory.read_witness().unwrap();
        assert!(memory.copy(0x1000, &mut [0; WITNESS_MAX_READ]));
        assert!(memory.read_witness_complete(mark));
        assert!(memory.copy(0x1000, &mut [0; WITNESS_MAX_READ + 1]));
        assert!(!memory.read_witness_complete(mark));
        assert_eq!(cache.borrow().witness.len, 1);
        assert!(cache.borrow_mut().verify_and_clear(&source));
        let other_cache = RefCell::new(SmallReadCache::with_witness(0, 4, 2 * bytes.len()));
        let other = CacheRef {
            source: &source,
            cache: &other_cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        assert!(!other.read_witness_complete(memory.read_witness().unwrap()));
        let disabled_cache = RefCell::new(SmallReadCache::new(4));
        let disabled = CacheRef {
            source: &source,
            cache: &disabled_cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        assert!(disabled.read_witness().is_none());
        assert!(!disabled.read_witness_complete(mark));
        cache.borrow_mut().witness.generation = u64::MAX;
        cache.borrow_mut().clear();
        assert!(memory.read_witness().is_none());
    }

    #[test]
    fn witness_does_not_replace_independent_modifier_snapshot_verification() {
        let source = Observed::new();
        source.accumulator(16, 64, &[(2, 10), (5, 20)]);
        let cache = RefCell::new(SmallReadCache::with_witness(1, 8, 256));
        let modifiers = RefCell::new(ModifierSnapshots::new(8, 128));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: Some(&modifiers),
            supply: None,
            air_defense: None,
            primary: None,
        };
        let mark = memory.read_witness().unwrap();
        assert_eq!(
            engine::modifier_accumulator::lookup_at(&memory, 16, 2)
                .unwrap()
                .raw,
            10
        );
        assert!(memory.read_witness_complete(mark));
        source.bytes.borrow_mut()[88] ^= 1; // Unselected key5 value still belongs to the snapshot.
        assert!(!modifiers.borrow_mut().verify_and_clear(&source));
        assert!(cache.borrow_mut().verify_and_clear(&source));
    }

    #[test]
    fn witness_growth_accounting_and_independent_pass_diagnostics_are_bounded() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::with_witness(1, 1, 8));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let allocated = cache.borrow().allocated_bytes();
        for _ in 0..2 {
            assert!(memory.copy(8, &mut [0; 8]));
            assert!(memory.copy(16, &mut [0; 8]));
            assert!(cache.borrow_mut().verify_and_clear(&source));
        }
        assert_eq!(cache.borrow().allocated_bytes(), allocated);
        assert_eq!(cache.borrow().diagnostics.retained_misses, 2);
        assert_eq!(cache.borrow().diagnostics.direct_reads, 2);
        assert_eq!(cache.borrow().diagnostics.witness_entries_peak, 1);
        assert_eq!(cache.borrow().diagnostics.witness_bytes_peak, 8);
        cache.borrow_mut().clear();
        assert_eq!(cache.borrow().diagnostics, CacheDiagnostics::default());
        cache.borrow_mut().witness.grow(4, 128);
        assert!(cache.borrow().allocated_bytes() >= allocated + 3 * size_of::<WitnessRead>() + 120);
        let mark = memory.read_witness().unwrap();
        assert!(memory.copy(32, &mut [0; 64]));
        assert!(memory.read_witness_complete(mark));
        assert!(cache.borrow_mut().verify_and_clear(&source));
    }

    #[test]
    fn modifier_snapshot_batches_all_entries_and_reuses_raw_presence_only() {
        use engine::modifier_accumulator::{Value, lookup_at};
        let source = Observed::new();
        source.accumulator(16, 64, &[(2, i64::MIN), (5, 0), (9, i64::MAX)]);
        let cache = RefCell::new(SmallReadCache::new(1));
        let modifiers = RefCell::new(ModifierSnapshots::new(8, 128));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: Some(&modifiers),
            supply: None,
            air_defense: None,
            primary: None,
        };
        for _ in 0..3 {
            assert_eq!(
                lookup_at(&memory, 16, 2),
                Ok(Value {
                    raw: i64::MIN,
                    present: true
                })
            );
            assert_eq!(
                lookup_at(&memory, 16, 5),
                Ok(Value {
                    raw: 0,
                    present: true
                })
            );
            assert_eq!(
                lookup_at(&memory, 16, 9),
                Ok(Value {
                    raw: i64::MAX,
                    present: true
                })
            );
            assert_eq!(lookup_at(&memory, 16, 6), Ok(Value::default()));
        }
        assert_eq!(&*source.calls.borrow(), &[(16, 16), (64, 48), (16, 16)]);
        assert_eq!(cache.borrow().used_len, 0);
        assert!(modifiers.borrow_mut().verify_and_clear(&source));
        assert_eq!(&source.calls.borrow()[3..], &[(16, 16), (64, 48), (16, 16)]);
        assert_eq!(modifiers.borrow().used_len, 0);
        assert_eq!(modifiers.borrow().bytes_len, 0);
        assert!(
            modifiers
                .borrow()
                .entries
                .iter()
                .all(|entry| !entry.occupied && entry.address == 0 && entry.header == [0; 16])
        );
    }

    #[test]
    fn modifier_snapshot_verifies_unused_keys_values_padding_and_all_header_fields() {
        for mutation in [16, 24, 28, 96, 100, 104] {
            let source = Observed::new();
            source.accumulator(16, 64, &[(2, 7), (5, 0), (9, 13)]);
            let mut snapshots = ModifierSnapshots::new(8, 128);
            let first = snapshots.lookup(&source, 16, 2).unwrap().unwrap();
            source.bytes.borrow_mut()[mutation] ^= 1;
            assert_eq!(snapshots.lookup(&source, 16, 2), Some(Ok(first)));
            assert!(
                !snapshots.verify_and_clear(&source),
                "missed mutation at {mutation}"
            );
            assert_eq!(snapshots.used_len, 0);
            assert_eq!(snapshots.bytes_len, 0);
        }
        let source = Observed::new();
        source.accumulator(16, 64, &[(2, 7)]);
        let mut snapshots = ModifierSnapshots::new(8, 128);
        assert_eq!(snapshots.lookup(&source, 16, 2).unwrap().unwrap().raw, 7);
        source.bytes.borrow_mut()[72..80].copy_from_slice(&11_i64.to_le_bytes());
        assert!(!snapshots.verify_and_clear(&source));
        assert_eq!(snapshots.lookup(&source, 16, 2).unwrap().unwrap().raw, 11);
        assert!(snapshots.verify_and_clear(&source));
    }

    #[test]
    fn modifier_snapshot_faults_check_every_retained_array_and_clear_failed_passes() {
        for fault in [16, 64, 32, 128] {
            let source = Observed::new();
            source.accumulator(16, 64, &[(2, 7), (5, 11)]);
            source.accumulator(32, 128, &[(3, 17)]);
            let mut snapshots = ModifierSnapshots::new(8, 128);
            assert!(snapshots.lookup(&source, 16, 2).unwrap().is_ok());
            assert!(snapshots.lookup(&source, 32, 3).unwrap().is_ok());
            source.calls.borrow_mut().clear();
            source.fault.set(Some(fault));
            assert!(!snapshots.verify_and_clear(&source));
            assert_eq!(
                &*source.calls.borrow(),
                &[(16, 16), (64, 32), (16, 16), (32, 16), (128, 16), (32, 16)]
            );
            assert_eq!(snapshots.used_len, 0);
            source.fault.set(None);
            assert!(snapshots.lookup(&source, 16, 2).unwrap().is_ok());
            // A later failed lookup must not discard prior retained inputs:
            // capture finalizes them even when the walk is already unavailable.
            source.fault.set(Some(32));
            assert!(snapshots.lookup(&source, 32, 3).unwrap().is_err());
            source.bytes.borrow_mut()[72] ^= 1;
            assert!(!snapshots.verify_and_clear(&source));
            assert_eq!(snapshots.used_len, 0);
        }
    }

    #[test]
    fn modifier_snapshot_rejects_unsorted_suffix_and_failed_initial_reads() {
        let source = Observed::new();
        let mut snapshots = ModifierSnapshots::new(8, 128);
        for suffix in [0, 1, 5] {
            source.accumulator(16, 64, &[(2, 7), (5, 11), (suffix, 13)]);
            assert_eq!(
                snapshots.lookup(&source, 16, 2),
                Some(Err("modifier accumulator keys not positive and increasing"))
            );
            assert_eq!(snapshots.used_len, 0);
            assert_eq!(snapshots.bytes_len, 0);
        }
        source.accumulator(16, 64, &[(2, 7)]);
        for fault in [16, 64] {
            source.fault.set(Some(fault));
            assert!(snapshots.lookup(&source, 16, 2).unwrap().is_err());
            assert_eq!(snapshots.used_len, 0);
        }
        source.fault.set(None);
        assert!(snapshots.lookup(&source, 16, 2).unwrap().is_ok());
        assert!(snapshots.verify_and_clear(&source));
    }

    #[test]
    fn modifier_snapshot_collisions_and_capacity_fallback_keep_full_original_validation() {
        use engine::modifier_accumulator::lookup_at;
        for (slots, arena) in [(0, 0), (1, 128), (8, 32)] {
            let source = Observed::new();
            source.accumulator(16, 64, &[(2, 7), (5, 11)]);
            source.accumulator(32, 128, &[(3, 17), (7, 19)]);
            let cache = RefCell::new(SmallReadCache::new(0));
            let modifiers = RefCell::new(ModifierSnapshots::new(slots, arena));
            let memory = CacheRef {
                source: &source,
                cache: &cache,
                modifiers: Some(&modifiers),
                supply: None,
                air_defense: None,
                primary: None,
            };
            assert_eq!(lookup_at(&memory, 16, 2).unwrap().raw, 7);
            assert_eq!(lookup_at(&memory, 32, 3).unwrap().raw, 17);
            source.bytes.borrow_mut()[136..144].copy_from_slice(&23_i64.to_le_bytes());
            assert_eq!(lookup_at(&memory, 32, 3).unwrap().raw, 23);
            source.bytes.borrow_mut()[144..148].copy_from_slice(&3_u32.to_le_bytes());
            assert!(
                lookup_at(&memory, 32, 3).is_err(),
                "fallback skipped malformed suffix"
            );
            assert_eq!(modifiers.borrow().used_len, usize::from(slots != 0));
            assert!(modifiers.borrow_mut().verify_and_clear(&source));
        }
    }

    #[test]
    fn modifier_snapshot_empty_arrays_reset_growth_and_accounting_are_bounded() {
        let source = Observed::new();
        source.accumulator(16, 64, &[]);
        let mut snapshots = ModifierSnapshots::new(0, 0);
        assert!(snapshots.lookup(&source, 16, 1).is_none());
        assert_eq!(snapshots.allocated_bytes(), 0);
        snapshots.grow(8, 128);
        let bytes = snapshots.allocated_bytes();
        let pointers = (
            snapshots.entries.as_ptr(),
            snapshots.used.as_ptr(),
            snapshots.bytes.as_ptr(),
        );
        assert!(bytes >= 8 * (size_of::<ModifierSnapshot>() + size_of::<usize>()) + 128);
        for _ in 0..3 {
            assert_eq!(
                snapshots.lookup(&source, 16, 1),
                Some(Ok(engine::modifier_accumulator::Value::default()))
            );
            assert!(snapshots.verify_and_clear(&source));
            assert_eq!(
                (
                    snapshots.entries.as_ptr(),
                    snapshots.used.as_ptr(),
                    snapshots.bytes.as_ptr()
                ),
                pointers
            );
            assert_eq!(snapshots.allocated_bytes(), bytes);
        }
        assert!(
            source
                .calls
                .borrow()
                .iter()
                .all(|&(address, len)| (address, len) == (16, 16))
        );
        assert!(snapshots.lookup(&source, 16, 1).unwrap().is_ok());
        snapshots.clear();
        assert_eq!(snapshots.used_len, 0);
        assert_eq!(snapshots.bytes_len, 0);
        assert!(
            snapshots
                .entries
                .iter()
                .all(|entry| !entry.occupied && entry.address == 0)
        );
    }

    #[test]
    fn modifier_snapshot_initial_header_reread_rejects_changes_including_empty_arrays() {
        struct Changed {
            source: Observed,
            reads: Cell<usize>,
            changed_byte: usize,
        }
        impl Memory for Changed {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if !self.source.copy(address, destination) {
                    return false;
                }
                if address == 16 {
                    self.reads.set(self.reads.get() + 1);
                    if self.reads.get() > 1 {
                        destination[self.changed_byte] ^= 1;
                    }
                }
                true
            }
        }
        for count in [0, 1] {
            for changed_byte in [0, 8, 12] {
                let source = Changed {
                    source: Observed::new(),
                    reads: Cell::new(0),
                    changed_byte,
                };
                source.source.accumulator(16, 64, &[(2, 7)][..count]);
                let mut snapshots = ModifierSnapshots::new(8, 128);
                assert_eq!(
                    snapshots.lookup(&source, 16, 2),
                    Some(Err("modifier accumulator header changed"))
                );
                assert_eq!(snapshots.used_len, 0);
                assert_eq!(snapshots.bytes_len, 0);
            }
        }
    }

    #[test]
    fn modifier_snapshot_direct_verification_reaches_the_final_partial_batch() {
        struct Wide {
            bytes: RefCell<Vec<u8>>,
            calls: RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for Wide {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.calls.borrow_mut().push((address, destination.len()));
                engine::memory::SliceMemory {
                    base: 0,
                    bytes: &self.bytes.borrow(),
                }
                .copy(address, destination)
            }
        }
        let source = Wide {
            bytes: RefCell::new(vec![0; 64 + 65 * 16]),
            calls: RefCell::new(Vec::new()),
        };
        {
            let mut bytes = source.bytes.borrow_mut();
            bytes[16..24].copy_from_slice(&64_u64.to_le_bytes());
            bytes[24..28].copy_from_slice(&65_u32.to_le_bytes());
            bytes[28..32].copy_from_slice(&65_u32.to_le_bytes());
            for index in 0..65 {
                let at = 64 + index * 16;
                bytes[at..at + 4].copy_from_slice(&(index as u32 + 1).to_le_bytes());
            }
        }
        let mut snapshots = ModifierSnapshots::new(8, 65 * 16);
        assert!(snapshots.lookup(&source, 16, 1).unwrap().is_ok());
        source.bytes.borrow_mut()[64 + 64 * 16 + 8] = 1;
        source.calls.borrow_mut().clear();
        assert!(!snapshots.verify_and_clear(&source));
        assert_eq!(
            &*source.calls.borrow(),
            &[(16, 16), (64, 1024), (1088, 16), (16, 16)]
        );
        assert_eq!(snapshots.used_len, 0);
    }

    #[test]
    fn transport_memo_hits_are_exact_and_end_at_each_independent_pass() {
        let mut memo = TransportMemo::new(8);
        let calls = Cell::new(0);
        for _ in 0..4 {
            assert_eq!(
                memo.resolve(u32::MAX, 0x1000, || {
                    calls.set(calls.get() + 1);
                    Ok(i64::MIN)
                }),
                Ok(i64::MIN)
            );
        }
        assert_eq!(calls.get(), 1);
        assert!(memo.verify_and_clear(|owner, region| {
            assert_eq!((owner, region), (u32::MAX, 0x1000));
            calls.set(calls.get() + 1);
            Ok(i64::MIN)
        }));
        assert_eq!(calls.get(), 2);
        assert_eq!(memo.used_len, 0);
        assert!(
            memo.entries
                .iter()
                .all(|e| !e.occupied && e.region == 0 && e.owner == 0 && e.speed == 0)
        );
        assert_eq!(memo.resolve(u32::MAX, 0x1000, || Ok(17)), Ok(17));
        assert!(memo.verify_and_clear(|_, _| Ok(17)));
    }

    #[test]
    fn supply_memo_shares_true_and_false_only_until_independent_pass_verification() {
        for value in [false, true] {
            let mut memo = SupplyMemo::new(8);
            let calls = Cell::new(0);
            for _ in 0..4 {
                assert_eq!(
                    memo.resolve(0, 16, u32::MAX, 0, || {
                        calls.set(calls.get() + 1);
                        Ok(value)
                    }),
                    Some(Ok(value))
                );
            }
            assert_eq!(calls.get(), 1);
            assert!(memo.verify_and_clear(|base, state, owner, area| {
                assert_eq!((base, state, owner, area), (0, 16, u32::MAX, 0));
                calls.set(calls.get() + 1);
                Ok(value)
            }));
            assert_eq!(calls.get(), 2);
            assert_eq!(memo.used_len, 0);
            assert!(memo.entries.iter().all(|e| !e.occupied
                && e.base == 0
                && e.state == 0
                && e.area == 0
                && e.owner == 0
                && !e.accessible));
            assert_eq!(
                memo.resolve(0, 16, u32::MAX, 0, || Ok(!value)),
                Some(Ok(!value))
            );
            assert!(memo.verify_and_clear(|_, _, _, _| Ok(!value)));
        }
    }

    #[test]
    fn pure_air_memo_matches_native_function_for_signed_and_exceptional_arguments() {
        let mut memo = AirDefenseMemo::new(128);
        for arguments in [
            [0, 100_000, 50_000],
            [1, 100_000, 50_000],
            [50_000, 100_000, 50_000],
            [-50_000, 100_000, 50_000],
            [100_000, 100_000, 50_000],
            [150_000, 100_000, 50_000],
            [1_000_000, 100_000, 50_000],
            [100_000, -100_000, 50_000],
            [100_000, 0, 50_000],
            [100_000, 100_000, -50_000],
            [100_000, i64::MIN, i64::MAX],
            [i64::MIN, 100_000, 50_000],
            [i64::MAX, 100_000, 50_000],
        ] {
            let calculate = || {
                engine::csub_unit_definition::movement_air_defense_value(
                    arguments[0],
                    arguments[1],
                    arguments[2],
                )
            };
            let expected = calculate();
            for _ in 0..2 {
                assert_eq!(
                    memo.resolve(arguments, calculate).unwrap_or_else(calculate),
                    expected
                );
            }
        }
    }

    #[test]
    fn pure_air_memo_preserves_all_three_raw_keys_without_eviction_or_error_success() {
        let mut memo = AirDefenseMemo::new(1);
        let key = [i64::MIN, -1, i64::MAX];
        assert_eq!(
            memo.resolve(key, || Err("native calculation traps")),
            Some(Err("native calculation traps"))
        );
        assert_eq!(memo.used_len, 0);
        let calls = Cell::new(0);
        for _ in 0..3 {
            assert_eq!(
                memo.resolve(key, || {
                    calls.set(calls.get() + 1);
                    Ok(0)
                }),
                Some(Ok(0))
            );
        }
        assert_eq!(calls.get(), 1);
        for changed in [
            [i64::MIN + 1, -1, i64::MAX],
            [i64::MIN, 1, i64::MAX],
            [i64::MIN, -1, i64::MIN],
        ] {
            assert_eq!(
                memo.resolve(changed, || panic!("collision uses caller's exact fallback")),
                None
            );
        }
        assert_eq!(
            memo.resolve(key, || panic!("retained key must survive collisions")),
            Some(Ok(0))
        );
        assert_eq!(memo.used_len, 1);
        let mut empty = AirDefenseMemo::new(0);
        assert_eq!(
            empty.resolve(key, || panic!("empty cache falls back")),
            None
        );
        assert_eq!(empty.allocated_bytes(), 0);
    }

    #[test]
    fn pure_air_capability_fallback_errors_and_changed_arguments_are_exact() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(0));
        let air = RefCell::new(AirDefenseMemo::new(1));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: Some(&air),
            primary: None,
        };
        assert_eq!(
            memory.movement_air_defense_value(100_000, 100_000, 50_000),
            Some(Ok(25_000))
        );
        assert_eq!(
            memory.movement_air_defense_value(100_000, 100_000, 50_000),
            Some(Ok(25_000))
        );
        assert_eq!(
            memory.movement_air_defense_value(100_000, 100_000, -50_000),
            None
        );
        let expected =
            engine::csub_unit_definition::movement_air_defense_value(100_000, 100_000, -50_000);
        assert_eq!(expected, Ok(-25_000));
        // The hook is pure and must never access source bytes itself.
        assert!(source.calls.borrow().is_empty());
        air.borrow_mut().clear();
        assert_eq!(
            memory.movement_air_defense_value(100_000, 100_000, -50_000),
            Some(expected)
        );
    }

    #[test]
    fn pure_air_memo_reset_growth_and_pool_accounting_are_bounded() {
        let mut memo = AirDefenseMemo::new(0);
        memo.grow(8);
        let bytes = memo.allocated_bytes();
        let pointers = (memo.entries.as_ptr(), memo.used.as_ptr());
        assert!(bytes >= 8 * (size_of::<AirDefenseEntry>() + size_of::<usize>()));
        for _ in 0..3 {
            assert_eq!(memo.resolve([1, 2, 3], || Ok(4)), Some(Ok(4)));
            memo.clear();
            assert_eq!(memo.used_len, 0);
            assert!(
                memo.entries
                    .iter()
                    .all(|e| !e.occupied && e.arguments == [0; 3] && e.value == 0)
            );
            assert_eq!((memo.entries.as_ptr(), memo.used.as_ptr()), pointers);
            assert_eq!(memo.allocated_bytes(), bytes);
        }
    }

    #[test]
    fn supply_memo_full_keys_collisions_capacity_and_faults_never_alias_or_evict() {
        let mut memo = SupplyMemo::new(1);
        assert_eq!(
            memo.resolve(8, 16, 1, 32, || Err("first fault")),
            Some(Err("first fault"))
        );
        assert_eq!(memo.used_len, 0);
        assert_eq!(memo.resolve(8, 16, 1, 32, || Ok(true)), Some(Ok(true)));
        for (base, state, owner, area) in [
            (9, 16, 1, 32),
            (8, 17, 1, 32),
            (8, 16, 2, 32),
            (8, 16, 1, 33),
        ] {
            assert_eq!(
                memo.resolve(base, state, owner, area, || panic!(
                    "collision must use ordinary reader"
                )),
                None
            );
        }
        assert_eq!(
            memo.resolve(8, 16, 1, 32, || panic!("expected retained hit")),
            Some(Ok(true))
        );
        assert_eq!(memo.used_len, 1);
        assert!(memo.verify_and_clear(|base, state, owner, area| {
            assert_eq!((base, state, owner, area), (8, 16, 1, 32));
            Ok(true)
        }));
        let mut empty = SupplyMemo::new(0);
        assert_eq!(
            empty.resolve(0, 0, 0, 0, || panic!("zero capacity must fall back")),
            None
        );
        assert_eq!(empty.used_len, 0);
    }

    #[test]
    fn supply_memo_verifies_all_retained_keys_after_mutation_fault_or_failed_miss() {
        for fault in [false, true] {
            let mut memo = SupplyMemo::new(8);
            assert_eq!(memo.resolve(0, 0, 1, 8, || Ok(true)), Some(Ok(true)));
            assert_eq!(memo.resolve(0, 0, 1, 16, || Ok(false)), Some(Ok(false)));
            assert_eq!(
                memo.resolve(0, 0, 1, 24, || Err("walk failed")),
                Some(Err("walk failed"))
            );
            let mut calls = 0;
            assert!(!memo.verify_and_clear(|_, _, _, area| {
                calls += 1;
                if area == 8 {
                    if fault {
                        Err("verify fault")
                    } else {
                        Ok(false)
                    }
                } else {
                    assert_eq!(area, 16);
                    Ok(false)
                }
            }));
            assert_eq!(calls, 2);
            assert_eq!(memo.used_len, 0);
            assert!(
                memo.entries
                    .iter()
                    .all(|entry| !entry.occupied && entry.area == 0)
            );
        }
    }

    #[test]
    fn supply_memo_direct_verification_covers_small_read_collisions() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(1));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let mut memo = SupplyMemo::new(1);
        assert_eq!(
            memo.resolve(0, 0, 1, 8, || {
                let a = engine::read::<u8>(&memory, 32).ok_or("a")?;
                let b = engine::read::<u8>(&memory, 64).ok_or("b")?;
                Ok(a < b)
            }),
            Some(Ok(true))
        );
        source.bytes.borrow_mut()[64] = 0;
        assert_eq!(
            memo.resolve(0, 0, 1, 8, || panic!("memo hit")),
            Some(Ok(true))
        );
        assert!(!memo.verify_and_clear(|_, _, _, _| {
            let a = engine::read::<u8>(&source, 32).ok_or("a")?;
            let b = engine::read::<u8>(&source, 64).ok_or("b")?;
            Ok(a < b)
        }));
        assert!(cache.borrow_mut().verify_and_clear(&source));
    }

    #[test]
    fn supply_memo_growth_reset_and_accounting_remain_off_callback_and_bounded() {
        let mut memo = SupplyMemo::new(0);
        assert_eq!(memo.allocated_bytes(), 0);
        memo.grow(8);
        let bytes = memo.allocated_bytes();
        let pointers = (memo.entries.as_ptr(), memo.used.as_ptr());
        assert!(bytes >= 8 * (size_of::<SupplyEntry>() + size_of::<usize>()));
        for _ in 0..3 {
            assert_eq!(memo.resolve(0, 0, 1, 8, || Ok(false)), Some(Ok(false)));
            assert!(memo.verify_and_clear(|_, _, _, _| Ok(false)));
            assert_eq!((memo.entries.as_ptr(), memo.used.as_ptr()), pointers);
            assert_eq!(memo.allocated_bytes(), bytes);
        }
        assert_eq!(memo.resolve(0, 0, 1, 8, || Ok(true)), Some(Ok(true)));
        memo.clear();
        assert_eq!(memo.used_len, 0);
        assert!(memo.entries.iter().all(|e| !e.occupied && e.area == 0));
        assert_eq!(memo.allocated_bytes(), bytes);
    }

    #[test]
    fn supply_capability_runs_real_native_reader_and_final_check_bypasses_its_hit() {
        struct Source {
            bytes: RefCell<Vec<u8>>,
            calls: Cell<usize>,
        }
        impl Memory for Source {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.calls.set(self.calls.get() + 1);
                engine::memory::SliceMemory {
                    base: 0,
                    bytes: &self.bytes.borrow(),
                }
                .copy(address, destination)
            }
        }
        let source = Source {
            bytes: RefCell::new(vec![0; 0x6000]),
            calls: Cell::new(0),
        };
        {
            let mut bytes = source.bytes.borrow_mut();
            for (header, data) in [
                (engine::cgame_state::COUNTRIES, 0x2000_u64),
                (engine::cgame_state::COUNTRY_LINKS, 0x2100),
            ] {
                let at = 0x1000 + header;
                bytes[at..at + 8].copy_from_slice(&data.to_le_bytes());
                bytes[at + 8..at + 12].copy_from_slice(&2_u32.to_le_bytes());
                bytes[at + 12..at + 16].copy_from_slice(&2_u32.to_le_bytes());
            }
            bytes[0x2008..0x2010].copy_from_slice(&0x3000_u64.to_le_bytes());
            bytes[0x2104..0x2108].copy_from_slice(&1_u32.to_le_bytes());
            bytes[0x3000..0x3008]
                .copy_from_slice(&(engine::ccountry::VTABLE_RVA as u64).to_le_bytes());
            let id = 0x3000 + engine::ccountry::ID;
            bytes[id..id + 4].copy_from_slice(&1_u32.to_le_bytes());
        }
        let cache = RefCell::new(SmallReadCache::new(0));
        let supply = RefCell::new(SupplyMemo::new(8));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: Some(&supply),
            air_defense: None,
            primary: None,
        };
        assert_eq!(
            engine::ccountry::has_core_supply_access(&memory, 0, 0x1000, 1, 0),
            Ok(true)
        );
        let calls = source.calls.get();
        assert!(calls > 0);
        assert_eq!(
            engine::ccountry::has_core_supply_access(&memory, 0, 0x1000, 1, 0),
            Ok(true)
        );
        assert_eq!(source.calls.get(), calls);
        let capital = 0x3000 + engine::ccountry::CAPITAL;
        source.bytes.borrow_mut()[capital..capital + 4].copy_from_slice(&1_i32.to_le_bytes());
        assert_eq!(
            engine::ccountry::has_core_supply_access(&memory, 0, 0x1000, 1, 0),
            Ok(true)
        );
        assert!(
            !supply
                .borrow_mut()
                .verify_and_clear(|base, state, owner, area| {
                    engine::ccountry::has_core_supply_access_direct(
                        &source, base, state, owner, area,
                    )
                })
        );
        assert_eq!(supply.borrow().used_len, 0);
        assert!(source.calls.get() > calls);
        source.bytes.borrow_mut()[capital..capital + 4].fill(0);
        assert_eq!(
            engine::ccountry::has_core_supply_access(&memory, 0, 0x1000, 1, 0),
            Ok(true)
        );
        assert!(
            supply
                .borrow_mut()
                .verify_and_clear(|base, state, owner, area| {
                    engine::ccountry::has_core_supply_access_direct(
                        &source, base, state, owner, area,
                    )
                })
        );
        for _ in 0..2 {
            assert!(engine::ccountry::has_core_supply_access(&memory, 0, 0x1000, 0, 0).is_err());
            assert_eq!(supply.borrow().used_len, 0);
        }
        // Capacity exhaustion preserves the original guarded reader too.
        let empty = RefCell::new(SupplyMemo::new(0));
        let fallback = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: Some(&empty),
            air_defense: None,
            primary: None,
        };
        assert_eq!(
            engine::ccountry::has_core_supply_access(&fallback, 0, 0x1000, 1, 0),
            Ok(true)
        );
        assert_eq!(empty.borrow().used_len, 0);
    }

    #[test]
    fn transport_collisions_and_failed_misses_never_replace_a_retained_key() {
        let mut memo = TransportMemo::new(1);
        assert_eq!(
            memo.resolve(1, 8, || Err("initial fault")),
            Err("initial fault")
        );
        assert_eq!(memo.used_len, 0);
        assert_eq!(memo.resolve(1, 8, || Ok(11)), Ok(11));
        assert_eq!(memo.resolve(2, 8, || Ok(22)), Ok(22));
        assert_eq!(
            memo.resolve(1, 16, || Err("collision fault")),
            Err("collision fault")
        );
        assert_eq!(memo.resolve(2, 8, || Ok(23)), Ok(23));
        assert_eq!(
            memo.resolve(1, 8, || panic!("retained key recalculated early")),
            Ok(11)
        );
        assert_eq!(memo.used_len, 1);
        assert!(memo.verify_and_clear(|owner, region| {
            assert_eq!((owner, region), (1, 8));
            Ok(11)
        }));
    }

    #[test]
    fn transport_verification_checks_all_results_after_change_or_fault_and_clears_keys() {
        for fault in [false, true] {
            let mut memo = TransportMemo::new(8);
            assert_eq!(memo.resolve(1, 8, || Ok(11)), Ok(11));
            assert_eq!(memo.resolve(1, 16, || Ok(22)), Ok(22));
            let mut calls = 0;
            assert!(!memo.verify_and_clear(|_, region| {
                calls += 1;
                if region == 8 {
                    if fault { Err("verify fault") } else { Ok(12) }
                } else {
                    Ok(22)
                }
            }));
            assert_eq!(calls, 2);
            assert_eq!(memo.used_len, 0);
            assert!(memo.entries.iter().all(|e| !e.occupied && e.region == 0));
        }
    }

    #[test]
    fn transport_direct_verification_covers_operands_not_retained_by_small_read_cache() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(1));
        let cached = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        // The first operand occupies the sole read-cache slot. The derived
        // speed's second operand collides, so small-read verification alone
        // cannot discover its later change.
        let mut memo = TransportMemo::new(1);
        assert_eq!(
            memo.resolve(7, 8, || {
                let a = engine::read::<u8>(&cached, 32).ok_or("a")?;
                let b = engine::read::<u8>(&cached, 64).ok_or("b")?;
                Ok(i64::from(a) + i64::from(b))
            }),
            Ok(96)
        );
        source.bytes.borrow_mut()[64] = 65;
        assert_eq!(memo.resolve(7, 8, || panic!("expected memo hit")), Ok(96));
        assert!(!memo.verify_and_clear(|_, _| {
            let a = engine::read::<u8>(&source, 32).ok_or("a")?;
            let b = engine::read::<u8>(&source, 64).ok_or("b")?;
            Ok(i64::from(a) + i64::from(b))
        }));
        assert!(cache.borrow_mut().verify_and_clear(&source));
    }

    #[test]
    fn transport_capacity_reset_and_accounting_do_not_allocate_during_use() {
        let mut memo = TransportMemo::new(0);
        assert_eq!(memo.resolve(0, 0, || Ok(9)), Ok(9));
        assert_eq!(memo.allocated_bytes(), 0);
        memo.grow(2);
        let bytes = memo.allocated_bytes();
        let entry_pointer = memo.entries.as_ptr();
        let used_pointer = memo.used.as_ptr();
        assert!(bytes >= 2 * (size_of::<TransportEntry>() + size_of::<usize>()));
        for _ in 0..3 {
            assert_eq!(memo.resolve(0, 0, || Ok(9)), Ok(9));
            assert!(memo.verify_and_clear(|_, _| Ok(9)));
            assert_eq!(memo.entries.as_ptr(), entry_pointer);
            assert_eq!(memo.used.as_ptr(), used_pointer);
        }
        assert_eq!(memo.resolve(1, 8, || Ok(10)), Ok(10));
        memo.clear();
        assert_eq!(memo.used_len, 0);
        assert!(memo.entries.iter().all(|e| !e.occupied && e.region == 0));
        assert_eq!(memo.allocated_bytes(), bytes);
    }

    #[test]
    fn exact_small_reads_share_only_within_one_verified_pass() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(8));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let mut bytes = [0; 8];
        for _ in 0..4 {
            assert!(memory.copy(64, &mut bytes));
        }
        assert_eq!(bytes, [64, 65, 66, 67, 68, 69, 70, 71]);
        assert_eq!(source.calls.borrow().as_slice(), &[(64, 8)]);
        assert!(memory.immutable_view(64, 8).is_none());
        assert!(cache.borrow_mut().verify_and_clear(&source));
        assert_eq!(source.calls.borrow().as_slice(), &[(64, 8), (64, 8)]);
        assert_eq!(cache.borrow().used_len, 0);
        assert!(
            cache
                .borrow()
                .keys
                .iter()
                .all(|e| e.address == 0 && e.len == 0)
        );
        assert!(
            cache
                .borrow()
                .bytes
                .iter()
                .all(|bytes| *bytes == [0; SMALL_READ_BYTES])
        );
        source.bytes.borrow_mut()[64] = 17;
        assert!(memory.copy(64, &mut bytes));
        assert_eq!(bytes[0], 17);
        assert_eq!(source.calls.borrow().len(), 3);
        assert!(cache.borrow_mut().verify_and_clear(&source));
    }

    #[test]
    fn small_read_mask_keeps_original_modulo_slots_for_arbitrary_capacities() {
        for capacity in [0, 1, 2, 3, 7, 8, 16, 31, 32, 1023, 1024, 32 * 1024] {
            let cache = SmallReadCache::new(capacity);
            assert_eq!(
                cache.mask,
                capacity
                    .is_power_of_two()
                    .then_some(capacity.wrapping_sub(1))
            );
            for address in [
                0,
                1,
                7,
                8,
                31,
                32,
                63,
                64,
                0x1234_5678,
                usize::MAX - 32,
                usize::MAX,
            ] {
                for len in [0, 1, 2, 4, 8, 16, 17, 24, 31, 32, 33, usize::MAX] {
                    let expected = if capacity == 0 || len == 0 || len > SMALL_READ_BYTES {
                        None
                    } else {
                        Some(
                            ((address >> 3) ^ (address >> 19) ^ len.wrapping_mul(0x9e37_79b9))
                                % capacity,
                        )
                    };
                    assert_eq!(
                        cache.slot(address, len),
                        expected,
                        "capacity={capacity}, address={address}, len={len}"
                    );
                }
            }
        }
    }

    #[test]
    fn small_read_growth_updates_mask_from_actual_length_and_clears_both_arrays() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(0));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let mut length = 0;
        for requested in [3, 8, 9, 16, 7, 0, 17, 32] {
            cache.borrow_mut().grow(requested);
            length = length.max(requested);
            {
                let cache = cache.borrow();
                assert_eq!(
                    (cache.keys.len(), cache.bytes.len(), cache.used.len()),
                    (length, length, length)
                );
                assert_eq!(
                    cache.mask,
                    length.is_power_of_two().then_some(length.wrapping_sub(1))
                );
                assert_eq!(cache.used_len, 0);
                assert!(
                    cache
                        .keys
                        .iter()
                        .all(|key| key.len == 0 && key.address == 0)
                );
                assert!(
                    cache
                        .bytes
                        .iter()
                        .all(|bytes| *bytes == [0; SMALL_READ_BYTES])
                );
                for address in [32_usize, 64, 128] {
                    let expected =
                        ((address >> 3) ^ (address >> 19) ^ 8_usize.wrapping_mul(0x9e37_79b9))
                            % length;
                    assert_eq!(cache.slot(address, 8), Some(expected));
                }
            }
            let pointers = {
                let cache = cache.borrow();
                (
                    cache.keys.as_ptr(),
                    cache.bytes.as_ptr(),
                    cache.used.as_ptr(),
                )
            };
            let allocated = cache.borrow().allocated_bytes();
            assert!(memory.copy(64, &mut [0; 8]));
            assert!(memory.copy(64, &mut [0; 8]));
            assert!(cache.borrow_mut().verify_and_clear(&source));
            let cache = cache.borrow();
            assert_eq!(
                (
                    cache.keys.as_ptr(),
                    cache.bytes.as_ptr(),
                    cache.used.as_ptr()
                ),
                pointers
            );
            assert_eq!(cache.allocated_bytes(), allocated);
            assert_eq!(
                allocated,
                cache.keys.capacity() * size_of::<CacheKey>()
                    + cache.bytes.capacity() * SMALL_READ_BYTES
                    + cache.used.capacity() * size_of::<usize>()
            );
            drop(cache);
            // Leave an occupied slot for the next grow, including no-growth
            // requests: growth must clear both metadata and owned payload.
            assert!(memory.copy(64, &mut [0; 8]));
        }
        cache.borrow_mut().clear();
    }

    #[test]
    fn separated_small_read_metadata_preserves_collision_fault_and_change_semantics() {
        for capacity in [3, 7, 8, 16] {
            let source = Observed::new();
            let cache = RefCell::new(SmallReadCache::new(capacity));
            let memory = CacheRef {
                source: &source,
                cache: &cache,
                modifiers: None,
                supply: None,
                air_defense: None,
                primary: None,
            };
            let slot = cache.borrow().slot(32, 8).unwrap();
            let mut first = [0; 8];
            assert!(memory.copy(32, &mut first));
            let retained = capacity.min(4);
            for address in 33..32 + retained {
                assert_eq!(cache.borrow().slot(address, 8), Some(slot));
                assert!(memory.copy(address, &mut [0; 8]));
            }
            let collision = 32 + retained;
            assert_eq!(cache.borrow().slot(collision, 8), Some(slot));
            let mut second = [0; 8];
            assert!(memory.copy(collision, &mut second));
            source.bytes.borrow_mut()[collision] ^= 1;
            assert!(memory.copy(collision, &mut second));
            assert_eq!(second[0], source.bytes.borrow()[collision]);
            source.fault.set(Some(collision));
            assert!(!memory.copy(collision, &mut second));
            assert_eq!(cache.borrow().used_len, retained);
            assert_eq!(cache.borrow().keys[slot].address, 32);
            assert_eq!(&cache.borrow().bytes[slot][..8], &first);
            source.bytes.borrow_mut()[32] ^= 1;
            let mut hit = [0; 8];
            assert!(memory.copy(32, &mut hit));
            assert_eq!(hit, first);
            assert!(!cache.borrow_mut().verify_and_clear(&source));
            assert_eq!(cache.borrow().used_len, 0);
            assert!(cache.borrow().keys.iter().all(|key| key.len == 0));
            assert!(
                cache
                    .borrow()
                    .bytes
                    .iter()
                    .all(|bytes| *bytes == [0; SMALL_READ_BYTES])
            );
        }
    }

    #[test]
    fn four_probe_neighborhood_wraps_and_bounds_retention_for_all_small_capacities() {
        for capacity in [0, 1, 2, 3, 4, 5, 7, 8, 16] {
            let source = Observed::new();
            let cache = RefCell::new(SmallReadCache::with_witness(capacity, 16, 128));
            let memory = CacheRef {
                source: &source,
                cache: &cache,
                modifiers: None,
                supply: None,
                air_defense: None,
                primary: None,
            };
            let address = if capacity == 0 {
                32
            } else {
                (8..192)
                    .step_by(8)
                    .find(|&address| cache.borrow().slot(address, 8) == Some(capacity - 1))
                    .unwrap()
            };
            let retained = capacity.min(4);
            let allocated = cache.borrow().allocated_bytes();
            let witness = memory.read_witness().unwrap();
            for offset in 0..5 {
                assert!(memory.copy(address + offset, &mut [0; 8]));
            }
            for offset in 0..5 {
                assert!(memory.copy(address + offset, &mut [0; 8]));
            }
            assert_eq!(source.calls.borrow().len(), 10 - retained);
            {
                let cache = cache.borrow();
                assert_eq!(cache.used_len, retained);
                assert_eq!(cache.diagnostics.hits, retained as u64);
                assert_eq!(cache.diagnostics.retained_misses, retained as u64);
                assert_eq!(cache.diagnostics.direct_reads, (2 * (5 - retained)) as u64);
                assert_eq!(cache.witness.len, 2 * (5 - retained));
                for offset in 0..retained {
                    let slot = (capacity - 1 + offset) % capacity;
                    assert_eq!(cache.used[offset], slot);
                    assert_eq!(cache.keys[slot].address, address + offset);
                    assert_eq!(cache.keys[slot].len, 8);
                }
            }
            assert!(memory.read_witness_complete(witness));
            assert!(cache.borrow_mut().verify_and_clear(&source));
            assert_eq!(source.calls.borrow().len(), 2 * (10 - retained));
            assert_eq!(cache.borrow().allocated_bytes(), allocated);
            assert!(!memory.read_witness_complete(witness));
            source.bytes.borrow_mut()[address] ^= 1;
            let mut bytes = [0; 8];
            assert!(memory.copy(address, &mut bytes));
            assert_eq!(bytes[0], source.bytes.borrow()[address]);
            assert!(cache.borrow_mut().verify_and_clear(&source));
        }
    }

    #[test]
    fn probed_failed_insertion_stays_empty_and_final_fault_checks_every_retained_key() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::with_witness(8, 8, 64));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let witness = memory.read_witness().unwrap();
        assert!(memory.copy(32, &mut [0; 8]));
        source.fault.set(Some(33));
        assert!(!memory.copy(33, &mut [0; 8]));
        assert_eq!(cache.borrow().used_len, 1);
        source.fault.set(None);
        for address in [34, 35, 33] {
            assert!(memory.copy(address, &mut [0; 8]));
        }
        assert_eq!(cache.borrow().used_len, 4);
        // Failed acquisition cannot become a complete witness after recovery.
        assert!(!memory.read_witness_complete(witness));
        source.fault.set(Some(32));
        let before = source.calls.borrow().len();
        assert!(!cache.borrow_mut().verify_and_clear(&source));
        assert_eq!(
            &source.calls.borrow()[before..],
            &[(32, 8), (34, 8), (35, 8), (33, 8)]
        );
        assert_eq!(cache.borrow().used_len, 0);
        assert!(cache.borrow().keys.iter().all(|key| key.len == 0));
    }

    #[test]
    fn probed_overlap_lengths_stay_distinct_and_full_neighborhood_exhausts_witness() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::with_witness(32, 1, 8));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        for len in [1, 2, 4, 8, 16, 24, 32] {
            assert!(memory.copy(64, &mut [0; 32][..len]));
        }
        for len in [1, 2, 4, 8, 16, 24, 32] {
            let mut bytes = [0; 32];
            assert!(memory.copy(64, &mut bytes[..len]));
            assert_eq!(&bytes[..len], &source.bytes.borrow()[64..64 + len]);
        }
        assert_eq!(source.calls.borrow().len(), 7);
        assert!(cache.borrow_mut().verify_and_clear(&source));
        let witness = memory.read_witness().unwrap();
        for address in 32..36 {
            assert!(memory.copy(address, &mut [0; 8]));
        }
        assert!(memory.copy(36, &mut [0; 8]));
        assert!(memory.read_witness_complete(witness));
        source.bytes.borrow_mut()[43] ^= 1;
        let mut direct = [0; 8];
        assert!(memory.copy(36, &mut direct));
        assert_eq!(direct[7], source.bytes.borrow()[43]);
        assert!(!memory.read_witness_complete(witness));
        assert_eq!(cache.borrow().witness.required_entries, 2);
        let before = source.calls.borrow().len();
        assert!(!cache.borrow_mut().verify_and_clear(&source));
        assert_eq!(
            &source.calls.borrow()[before..],
            &[(36, 8), (32, 8), (33, 8), (34, 8), (35, 8)]
        );
    }

    #[test]
    fn collisions_different_lengths_large_and_empty_reads_never_evict_or_alias() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(1));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        assert!(memory.copy(64, &mut [0; 8]));
        assert!(memory.copy(72, &mut [0; 8]));
        assert!(memory.copy(72, &mut [0; 8]));
        assert!(memory.copy(64, &mut [0; 4]));
        assert!(memory.copy(64, &mut [0; 4]));
        assert!(memory.copy(64, &mut [0; 8]));
        assert!(memory.copy(96, &mut [0; 33]));
        assert!(memory.copy(96, &mut [0; 33]));
        assert!(memory.copy(96, &mut []));
        assert_eq!(source.calls.borrow().len(), 8);
        assert_eq!(cache.borrow().used_len, 1);
        assert_eq!(cache.borrow().keys[0].address, 64);
        assert_eq!(cache.borrow().keys[0].len, 8);
        assert!(cache.borrow_mut().verify_and_clear(&source));
        assert_eq!(source.calls.borrow().last(), Some(&(64, 8)));
    }

    #[test]
    fn failed_initial_reads_are_not_cached_and_verification_faults_fail_the_pass() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(8));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        source.fault.set(Some(32));
        assert!(!memory.copy(32, &mut [0; 8]));
        assert!(!memory.copy(32, &mut [0; 8]));
        assert_eq!(cache.borrow().used_len, 0);
        source.fault.set(None);
        assert!(memory.copy(32, &mut [0; 8]));
        assert!(memory.copy(40, &mut [0; 8]));
        source.fault.set(Some(32));
        assert!(!cache.borrow_mut().verify_and_clear(&source));
        // Verification must still inspect the second retained span after the
        // first faults, and must clear both before another independent pass.
        assert_eq!(&source.calls.borrow()[4..], &[(32, 8), (40, 8)]);
        assert_eq!(cache.borrow().used_len, 0);
        assert!(cache.borrow().keys.iter().all(|e| e.len == 0));
        assert!(
            cache
                .borrow()
                .bytes
                .iter()
                .all(|bytes| *bytes == [0; SMALL_READ_BYTES])
        );
    }

    #[test]
    fn final_direct_reread_exposes_a_mutation_hidden_by_an_intermediate_cache_hit() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(8));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let mut bytes = [0; 8];
        assert!(memory.copy(32, &mut bytes));
        source.bytes.borrow_mut()[32] ^= 1;
        assert!(memory.copy(32, &mut bytes));
        assert_eq!(bytes[0], 32);
        assert!(!cache.borrow_mut().verify_and_clear(&source));
        assert!(memory.copy(32, &mut bytes));
        assert_eq!(bytes[0], 33);
        assert!(cache.borrow_mut().verify_and_clear(&source));
    }

    #[test]
    fn cache_reset_growth_and_accounting_are_off_callback_and_bounded() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(0));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        assert!(memory.copy(32, &mut [0; 8]));
        assert!(memory.copy(32, &mut [0; 8]));
        assert_eq!(cache.borrow().allocated_bytes(), 0);
        cache.borrow_mut().grow(2);
        let allocated = cache.borrow().allocated_bytes();
        assert!(allocated >= 2 * (size_of::<CacheKey>() + SMALL_READ_BYTES + size_of::<usize>()));
        assert!(memory.copy(32, &mut [0; 8]));
        cache.borrow_mut().clear();
        assert_eq!(cache.borrow().used_len, 0);
        assert!(memory.copy(32, &mut [0; 8]));
        assert!(cache.borrow_mut().verify_and_clear(&source));
        assert_eq!(cache.borrow().allocated_bytes(), allocated);
    }

    #[test]
    fn conversion_uses_owned_edges_and_keeps_separate_leg_rounding() {
        let mut capture = Captured::with_capacities(
            &Capacities {
                rows: 1,
                legs: 2,
                cache_entries: 2,
                witness_entries: 0,
                witness_bytes: 0,
                transport_entries: 2,
                modifier_accumulators: 0,
                modifier_bytes: 0,
                supply_entries: 0,
                air_defense_entries: 0,
                primary_roots: 0,
                primary_chunks: 0,
            },
            &[0],
        );
        capture.ranges[0].as_mut().unwrap().rows.rows[0] = cunit::EtaInputs {
            division: 7,
            next: 10,
            destination: 11,
            path_total: 2,
            leg_start: 0,
            leg_len: 2,
            next_leg_count: 1,
            numerator: 1,
            current_speed: (3_i64).into(),
            normal: None,
        };
        capture.ranges[0].as_mut().unwrap().legs.first[..2].copy_from_slice(&[
            cunit::EtaLeg {
                distance: 2,
                speed: (3_i64).into(),
            },
            cunit::EtaLeg {
                distance: 2,
                speed: (3_i64).into(),
            },
        ]);
        capture.ranges[0].as_mut().unwrap().legs.len = 2;
        capture.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(TableRows::DivisionEta(rows))] = capture.tables() else {
            panic!("expected exact ETA rows");
        };
        assert_eq!(rows[0].next_remaining, 33_333);
        assert_eq!(rows[0].destination_remaining, 99_999);
        assert_eq!(rows[0].path_total, 2);
        capture.ranges[0].as_mut().unwrap().legs.len = 1;
        assert!(matches!(capture.tables(), [TableView::Unavailable { .. }]));
    }

    #[test]
    fn failures_and_truncated_empty_prefix_are_not_standstill() {
        let mut capture = Captured::with_capacities(
            &Capacities {
                rows: 1,
                legs: 1,
                cache_entries: 2,
                witness_entries: 0,
                witness_bytes: 0,
                transport_entries: 2,
                modifier_accumulators: 0,
                modifier_bytes: 0,
                supply_entries: 0,
                air_defense_entries: 0,
                primary_roots: 0,
                primary_chunks: 0,
            },
            &[0],
        );
        capture.outcome = Outcome::Truncated {
            len: 0,
            unread_rows: 8,
        };
        assert!(matches!(
            capture.tables(),
            [TableView::Truncated { missing: 8, .. }]
        ));
        capture.checked = true;
        capture.mismatches = 1;
        assert!(matches!(capture.tables(), [TableView::Unavailable { .. }]));
    }

    #[test]
    fn independent_eta_window_retires_only_comparison_storage() {
        let mut caps = Capacities {
            rows: 1,
            legs: 1,
            cache_entries: 2,
            witness_entries: 0,
            witness_bytes: 0,
            transport_entries: 2,
            modifier_accumulators: 0,
            modifier_bytes: 0,
            supply_entries: 0,
            air_defense_entries: 0,
            primary_roots: 0,
            primary_chunks: 0,
        };
        let mut capture = Captured::with_capacities(&caps, &[0]);
        capture.required_legs = 3;
        capture.grow(&mut caps, &[CONSISTENCY_TARGET]);
        assert!(capture.ranges[0].as_mut().unwrap().legs.first.len() >= 3);
        assert!(capture.ranges[0].as_mut().unwrap().rows.second.is_empty());
        assert!(capture.ranges[0].as_mut().unwrap().legs.second.is_empty());
        assert!(!capture.ranges[0].as_mut().unwrap().rows.rows.is_empty());
    }
}
