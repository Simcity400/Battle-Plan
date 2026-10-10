//! Current-hour simulation operands for a fresh counter route.
//!
//! A cache-miss duration needs actual first-edge speed only when subtracting
//! progress on a matching first node. Other route legs use the movement speed
//! tree. The stored route determines invasion and attacking selectors; it does
//! not bound the terrain domain. No selection, cache history or UI clock is read.
//! Readers use prepared caller storage and the parent's current-pass memos.

use super::*;

/// Full legacy route bytes or the only two IDs the speed graph consumes.
/// The native route header/count is retained separately in DivisionSeed.
#[derive(Clone, Copy)]
pub(crate) enum MovementRouteIds<'a> {
    Full(&'a [u32]),
    Endpoints {
        route: Array,
        first: Result<u32, &'static str>,
        last: &'a std::cell::Cell<Option<Result<u32, &'static str>>>,
    },
}

impl MovementRouteIds<'_> {
    pub(super) fn get(
        self,
        memory: &impl Memory,
        index: usize,
    ) -> Result<Option<u32>, &'static str> {
        match self {
            Self::Full(ids) => Ok(ids.get(index).copied()),
            Self::Endpoints { route, first, last } => {
                if route.count == 0 {
                    Ok(None)
                } else if index == 0 {
                    first.map(Some)
                } else if index == route.count as usize - 1 {
                    let result = last.get().unwrap_or_else(|| {
                        let result = route
                            .element(route.count - 1, 4)
                            .and_then(|at| read::<u32>(memory, at))
                            .ok_or("fresh counter ETA last route ID unreadable");
                        last.set(Some(result));
                        result
                    });
                    result.map(Some)
                } else {
                    Ok(None)
                }
            }
        }
    }
}

/// Only per-army getters shared by the actual edge and fresh route domain.
/// Parent edge acquisition records these at their original native demand sites.
#[derive(Default, Clone, Copy)]
pub(super) struct NormalEdgeObservations {
    pub(super) terrain: Option<TerrainObservation>,
    pub(super) fort_penalty: Option<Result<i64, &'static str>>,
    pub(super) snow_penalty: Option<Result<i64, &'static str>>,
    pub(super) river_adjustment: Option<Result<i64, &'static str>>,
    pub(super) river_leader: Option<Result<i64, &'static str>>,
}

#[derive(Clone, Copy)]
pub(super) struct TerrainObservation {
    pub(super) pointer: usize,
    pub(super) unit: Result<i64, &'static str>,
    /// Includes the demanded native index read; None means not yet queried.
    pub(super) leader: Option<Result<i64, &'static str>>,
}

#[derive(Clone, Copy)]
pub(super) struct CurrentSpeedObservation {
    pub(super) from: usize,
    pub(super) to: usize,
    pub(super) result: Result<MovementSpeedInputs, &'static str>,
}

/// Transient legacy-to-fresh handoff, never projected into replay rows.
pub(super) struct DivisionEtaObservation {
    pub(super) division: u64,
    pub(super) pointer: usize,
    pub(super) context: Option<NormalMovementContext>,
}

/// Raw normal operands; terrain values live in the packet's admitted span.
#[derive(Clone, Copy)]
pub struct FreshNormalEtaRow {
    pub prepared: PreparedNormalSpeed,
    pub fort_penalty: i64,
    pub snow_penalty: i64,
    pub river_adjustment: i64,
    pub river_leader: i64,
    pub terrain_start: usize,
    pub terrain_len: usize,
}

/// Each failure retains its own native demand; Ok(None) is an unqueried branch.
#[derive(Clone, Copy)]
pub struct FreshCounterEtaRow {
    pub division: u64,
    pub sample_hour: i32,
    pub current_speed: Result<Option<i64>, &'static str>,
    pub normal: Result<Option<FreshNormalEtaRow>, &'static str>,
    pub rail: Result<Option<StrategicRedeploymentInputs>, &'static str>,
}

#[derive(Clone, Copy)]
pub struct FreshCounterEtaWalk {
    pub outcome: Outcome,
    pub terrain_len: usize,
    pub required_rows: usize,
    /// Demand of admitted normal rows; a lower bound if row capacity skipped
    /// units. In that case required_rows * complete LAND roster length is a
    /// checked upper bound for the next off-hook terrain reservation.
    pub required_terrain: usize,
}

/// Storage is sized from the authenticated unit census and complete live LAND domain.
/// Fixed callback packets refuse shortage; worker packets grow from the original
/// census and complete LAND domain before publication.
pub struct FreshCounterEtaPacket {
    rows: Vec<FreshCounterEtaRow>,
    terrain: Vec<(i32, i64)>,
    result: FreshCounterEtaWalk,
    growing: bool,
}

const UNCAPTURED: FreshCounterEtaWalk = FreshCounterEtaWalk {
    outcome: Outcome::Unavailable {
        reason: "fresh counter ETA not captured",
    },
    terrain_len: 0,
    required_rows: 0,
    required_terrain: 0,
};

impl FreshCounterEtaPacket {
    pub fn enable_growth(&mut self) {
        self.growing = true;
    }
    pub fn try_new(rows: usize, terrain: usize) -> std::io::Result<Self> {
        Ok(Self {
            rows: crate::allocation::vec(rows)?,
            terrain: crate::allocation::vec(terrain)?,
            result: UNCAPTURED,
            growing: false,
        })
    }

    pub fn vacant() -> Self {
        Self {
            rows: Vec::new(),
            terrain: Vec::new(),
            result: UNCAPTURED,
            growing: false,
        }
    }

    pub fn reserve(&mut self, rows: usize, terrain: usize) -> std::io::Result<()> {
        eta_reserve(&mut self.rows, rows)?;
        eta_reserve(&mut self.terrain, terrain)
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.terrain.clear();
        self.result = UNCAPTURED;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * size_of::<FreshCounterEtaRow>()
            + self.terrain.capacity() * size_of::<(i32, i64)>()
    }

    pub fn rows(&self) -> &[FreshCounterEtaRow] {
        &self.rows
    }

    pub fn capacities(&self) -> (usize, usize) {
        (self.rows.capacity(), self.terrain.capacity())
    }

    pub fn terrain(&self) -> &[(i32, i64)] {
        &self.terrain
    }

    pub fn outcome(&self) -> FreshCounterEtaWalk {
        self.result
    }

    /// Off-thread before projection; sorting does not move terrain arenas.
    pub fn sort_rows_by_division(&mut self) -> Result<(), &'static str> {
        self.rows.sort_unstable_by_key(|row| row.division);
        if self
            .rows
            .windows(2)
            .any(|pair| pair[0].division == pair[1].division)
        {
            return Err("fresh counter ETA division listed twice");
        }
        Ok(())
    }
}

/// Off-hook capacity preparation only. The complete province count bounds the
/// distinct LAND roster; per-division terrain storage uses its actual length.
pub fn counter_land_terrain_capacity(
    memory: &impl Memory,
    state: usize,
) -> Result<usize, &'static str> {
    let root = presentation_array(memory, state, cgame_state::PROVINCES, 8)?;
    if root.count < 2 {
        return Err("fresh counter terrain province census empty");
    }
    Ok(root.count as usize)
}

/// Off-thread initial sizing only; capture still authenticates its own pass.
/// Failure leaves the caller's existing capacity floors unchanged.
pub fn fresh_counter_eta_map_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<(usize, usize), &'static str> {
    fresh_counter_eta_map_demand_keyed(memory, base, state)
        .map(|(_, provinces, terrains)| (provinces, terrains))
}

/// A completed advisory count is reusable only under this exact native key.
pub fn fresh_counter_eta_map_demand_keyed(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<(crate::cprovince::StaticGeometryKey, usize, usize), &'static str> {
    let count = counter_land_terrain_capacity(memory, state)?;
    let mut geometry =
        crate::cprovince::GeometryPacket::try_advisory(crate::cprovince::GeometryCapacities {
            provinces: count,
            neighbour_entries: 0,
        })
        .map_err(|_| "fresh counter map hint geometry allocation failed")?;
    let mut statics = crate::cprovince::StaticGeometry::default();
    statics
        .reserve_provinces_advisory(count)
        .map_err(|_| "fresh counter map hint static allocation failed")?;
    let mut terrains = Vec::new();
    terrains
        .try_reserve_exact(count)
        .map_err(|_| "fresh counter map hint terrain allocation failed")?;
    terrains.resize(
        count,
        crate::cprovince::StaticProvince {
            pointer: 0,
            definition: 0,
            flags: 0,
            terrain: 0,
            terrain_index: 0,
        },
    );
    geometry.begin(memory, base, state)?;
    if !statics.begin(memory, base, state) {
        return Err("fresh counter map hint static root unavailable");
    }
    let key = statics.demand_key()?;
    let len = capture_counter_land_terrains(
        memory,
        base,
        state,
        &mut geometry,
        &mut statics,
        &mut terrains,
    )?;
    if crate::cprovince::static_geometry_demand_key(memory, base, state)? != key {
        return Err("fresh counter map hint key changed during census");
    }
    Ok((key, geometry.province_count()?, len))
}

/// Complete distinct terrain domain of live LAND provinces. Sea/null terrain
/// slots are not consumed by the normal getter and do not enter this domain.
/// The caller prepares geometry and StaticGeometry for the live province count
/// and provides an arena large enough for the distinct terrain demand. A failed
/// walk never certifies its partially written prefix as a complete roster.
pub fn capture_counter_land_terrains(
    memory: &impl Memory,
    base: usize,
    _state: usize,
    geometry: &mut crate::cprovince::GeometryPacket,
    statics: &mut crate::cprovince::StaticGeometry,
    output: &mut [crate::cprovince::StaticProvince],
) -> Result<usize, &'static str> {
    geometry
        .ready()
        .map_err(|_| "fresh counter terrain geometry unavailable")?;
    // Both packets were begun once for this pass. Reuse failed root outcomes
    // rather than calling begin again or clearing the legacy edge observations.
    let count = statics.province_count()?;
    if statics.province_root()? != geometry.province_root()? {
        return Err("fresh counter terrain province census differs");
    }
    let expected = base
        .checked_add(crate::cterrain_type::VTABLE_RVA)
        .ok_or("fresh counter terrain vtable overflow")? as u64;
    let mut len = 0;
    for id in 1..count {
        let id = u32::try_from(id).map_err(|_| "fresh counter terrain province ID overflow")?;
        if geometry.acquire_route_pointer(memory, id)? == 0
            || geometry.acquire_flags(memory, id)? & 1 == 0
        {
            continue;
        }
        // Unlike province(), this accessor refuses an unprepared cold slot
        // rather than growing its persistent Vec in the frozen acquisition.
        let province = statics.province_prepared(memory, base, id)?;
        if province.flags & 1 == 0 || province.terrain == 0 || province.terrain_index <= 0 {
            return Err("fresh counter LAND terrain identity invalid");
        }
        if let Some(seen) = output[..len].iter().find(|seen| {
            seen.terrain == province.terrain || seen.terrain_index == province.terrain_index
        }) {
            if seen.terrain != province.terrain || seen.terrain_index != province.terrain_index {
                return Err("fresh counter LAND terrain pointer/index inconsistent");
            }
            continue;
        }
        let fields = presentation_batch(
            memory,
            [
                (province.terrain, 0, 8),
                (province.terrain, crate::cterrain_type::DATABASE_INDEX, 4),
            ],
        );
        if presentation_batched::<u64, 2>(memory, &fields, 0)? != expected
            || presentation_batched::<i32, 2>(memory, &fields, 1)? != province.terrain_index
        {
            return Err("fresh counter LAND terrain identity mismatch");
        }
        let slot = output
            .get_mut(len)
            .ok_or("fresh counter LAND terrain arena exhausted")?;
        *slot = province;
        len += 1;
    }
    Ok(len)
}

/// Pure validation is shared by this capture's rows, but its first failure is
/// reported only when a row reaches that ordinal, before any value reuse/read.
/// Legacy terrain matching and native getter searches retain their costs.
#[derive(Clone, Copy)]
struct LandTerrainDomain<'a> {
    source: Result<&'a [crate::cprovince::StaticProvince], &'static str>,
    first_invalid: Option<(usize, &'static str)>,
}

impl<'a> LandTerrainDomain<'a> {
    fn new(source: Result<&'a [crate::cprovince::StaticProvince], &'static str>) -> Self {
        let first_invalid = source.ok().and_then(|terrains| {
            terrains.iter().enumerate().find_map(|(ordinal, terrain)| {
                if terrain.flags & 1 == 0 || terrain.terrain == 0 || terrain.terrain_index <= 0 {
                    Some((ordinal, "fresh counter ETA LAND terrain domain invalid"))
                } else if terrains[..ordinal]
                    .iter()
                    .any(|earlier| earlier.terrain_index == terrain.terrain_index)
                {
                    Some((ordinal, "fresh counter ETA LAND terrain listed twice"))
                } else {
                    None
                }
            })
        });
        Self {
            source,
            first_invalid,
        }
    }
}

/// The source census, map domain and shared packet all belong to this hour.
/// The caller clears/prepares shared memos once before this frozen acquisition;
/// this function never clears legacy rows or carries an earlier fresh payload.
#[allow(clippy::too_many_arguments)]
pub fn capture_fresh_counter_eta(
    memory: &impl Memory,
    base: usize,
    state: usize,
    hour: i32,
    seeds: &[DivisionSeed],
    span: std::ops::Range<usize>,
    partition: (usize, usize),
    source_outcome: Outcome,
    land_terrains: Result<&[crate::cprovince::StaticProvince], &'static str>,
    geometry: &mut crate::cprovince::GeometryPacket,
    shared: &mut EtaInputPacket,
    packet: &mut FreshCounterEtaPacket,
) -> FreshCounterEtaWalk {
    packet.clear();
    shared
        .fresh_observations
        .sort_unstable_by_key(|row| (row.division, row.pointer));
    let mut required_rows = 0_usize;
    let mut required_terrain = 0_usize;
    let result = (|| {
        if shared
            .fresh_observations
            .windows(2)
            .any(|pair| (pair[0].division, pair[0].pointer) == (pair[1].division, pair[1].pointer))
        {
            return Err("fresh counter ETA observation listed twice");
        }
        if let Outcome::Unavailable { reason } = source_outcome {
            return Err(reason);
        }
        if source_outcome.len() != seeds.len() || span.start > span.end || span.end > seeds.len() {
            return Err("fresh counter ETA source range invalid");
        }
        if partition.1 == 0 || partition.0 >= partition.1 {
            return Err("fresh counter ETA partition invalid");
        }
        if shared.counters.walks != 0
            && let Outcome::Unavailable { reason } = shared.result.outcome
        {
            // A failed legacy pass retains its exact native failure frontier.
            // Count owned all-unit profile demand without trying
            // any source getter again.
            required_rows = seeds[span.clone()]
                .iter()
                .filter(|seed| {
                    (seed.division ^ (seed.division >> 32)) % partition.1 as u64
                        == partition.0 as u64
                        && seed
                            .ordinary_route
                            .and_then(|header| Array::from_header(header, 4))
                            .is_some()
                })
                .count();
            if let Ok(terrains) = land_terrains {
                required_terrain = required_rows
                    .checked_mul(terrains.len())
                    .ok_or("fresh counter ETA terrain demand overflow")?;
            }
            return Err(reason);
        }
        geometry
            .ready()
            .map_err(|_| "fresh counter ETA geometry unavailable")?;
        let expected = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let mut missing = match source_outcome {
            Outcome::Truncated { unread_rows, .. } if span.start == 0 && partition.0 == 0 => {
                unread_rows
            }
            _ => 0,
        };
        let land_terrains = LandTerrainDomain::new(land_terrains);
        for seed in &seeds[span] {
            if (seed.division ^ (seed.division >> 32)) % partition.1 as u64 != partition.0 as u64 {
                continue;
            }
            if seed.pointer == 0
                || seed.division == 0
                || seed.primary_vtable != Some(expected)
                || seed.handle != Some(seed.division)
            {
                return Err("fresh counter ETA division identity invalid");
            }
            let route = seed
                .ordinary_route
                .and_then(|header| Array::from_header(header, 4))
                .ok_or("fresh counter ETA route header unavailable")?;
            required_rows = required_rows
                .checked_add(1)
                .ok_or("fresh counter ETA row demand overflow")?;
            if packet.growing {
                eta_reserve(&mut packet.rows, required_rows)
                    .map_err(|_| "fresh counter ETA allocation failed")?;
            }
            if packet.rows.len() == packet.rows.capacity() {
                missing = missing
                    .checked_add(1)
                    .ok_or("fresh counter ETA missing count overflow")?;
                continue;
            }
            let mut context = shared
                .take_fresh_context(seed.division, seed.pointer)
                .unwrap_or_default();
            let last = std::cell::Cell::new(context.route_endpoints.map(|(_, last)| Ok(last)));
            let endpoints = if route.count == 0 {
                MovementRouteIds::Full(&[])
            } else {
                match context.route_endpoints {
                    Some((first, _)) => MovementRouteIds::Endpoints {
                        route,
                        first: Ok(first),
                        last: &last,
                    },
                    None => route_endpoints(memory, route, &last),
                }
            };
            let row = capture_row(
                memory,
                base,
                state,
                hour,
                seed,
                route.count,
                endpoints,
                land_terrains,
                geometry,
                shared,
                packet,
                &mut context,
                &mut required_terrain,
            );
            packet.rows.push(row);
        }
        Ok(if missing == 0 {
            Outcome::Rows {
                len: packet.rows.len(),
            }
        } else {
            Outcome::Truncated {
                len: packet.rows.len(),
                unread_rows: missing,
            }
        })
    })();
    let walk = FreshCounterEtaWalk {
        outcome: result.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        terrain_len: packet.terrain.len(),
        required_rows,
        required_terrain,
    };
    packet.result = walk;
    shared.clear_fresh_observations();
    walk
}

fn route_endpoints<'a>(
    memory: &impl Memory,
    route: Array,
    last: &'a std::cell::Cell<Option<Result<u32, &'static str>>>,
) -> MovementRouteIds<'a> {
    let first = route
        .element(0, 4)
        .and_then(|at| read::<u32>(memory, at))
        .ok_or("fresh counter ETA route endpoint unreadable");
    MovementRouteIds::Endpoints { route, first, last }
}

#[allow(clippy::too_many_arguments)]
fn capture_row(
    memory: &impl Memory,
    base: usize,
    state: usize,
    hour: i32,
    seed: &DivisionSeed,
    ordinary_count: u32,
    ids: MovementRouteIds<'_>,
    land_terrains: LandTerrainDomain<'_>,
    geometry: &mut crate::cprovince::GeometryPacket,
    shared: &mut EtaInputPacket,
    packet: &mut FreshCounterEtaPacket,
    context: &mut NormalMovementContext,
    required_terrain: &mut usize,
) -> FreshCounterEtaRow {
    let mut row = FreshCounterEtaRow {
        division: seed.division,
        sample_hour: hour,
        current_speed: Err("fresh counter ETA current speed unavailable"),
        normal: Err("fresh counter ETA current speed unavailable"),
        rail: Err("fresh counter ETA current speed unavailable"),
    };
    let legacy_terrains = match context.legacy_terrain {
        Some((start, len)) => start
            .checked_add(len)
            .and_then(|end| shared.terrain.get(start..end))
            .ok_or("fresh counter ETA legacy terrain span invalid"),
        None => Ok(&[][..]),
    };
    let mut reader = EtaGeometry {
        packet: geometry,
        memory,
    };
    let mut inputs = MovementSharedInputs {
        seed,
        route_ids: ids,
        geometry: &mut reader,
        rail_speeds: Some(&mut shared.rail),
        state_modifiers: Some(&shared.state_modifiers),
        category_contributions: Some(&shared.category_contributions),
        supply_templates: Some(&shared.supply_templates),
        air_prefixes: Some(&shared.air_prefixes),
    };
    let current = if ordinary_count == 0 {
        Ok(None)
    } else {
        (|| {
            let location = seed
                .location
                .ok_or("fresh counter ETA location unavailable")?;
            if location == 0 {
                return Err("fresh counter ETA moving division location null");
            }
            let first = ids
                .get(memory, 0)?
                .ok_or("fresh counter ETA first route ID unavailable")?;
            let next = inputs.geometry.route_pointer(first)?;
            movement_speed_inputs_seeded_in_pass(
                memory,
                base,
                state,
                &mut inputs,
                location,
                next,
                context,
                |owner, region| {
                    eta_transport_speed(memory, base, state, &mut shared.transport, owner, region)
                },
            )
            .map(MovementSpeedInputs::evaluate)
        })()
        .map(Some)
    };
    row.current_speed = current;
    // The duration calculation consumes current_speed only for its progress-subtraction branch.
    // Different-first routes may still use the independently acquired domains.
    // Context stages retain both success and failure, so this does not retry a
    // failed common/root/rail getter through the fresh domain.
    let domains = land_domains(memory, base, state, seed, &mut inputs, context);
    match domains {
        Err(reason) => {
            row.normal = Err(reason);
            row.rail = Err(reason);
        }
        Ok((normal, rail)) => {
            row.rail = Ok(rail);
            row.normal = match normal {
                None => Ok(None),
                Some(roots) => capture_normal(
                    memory,
                    base,
                    state,
                    seed,
                    roots,
                    land_terrains,
                    legacy_terrains,
                    &mut inputs,
                    context,
                    packet,
                    required_terrain,
                )
                .map(Some),
            };
        }
    }
    row
}

/// Admission only: every graph operand still comes from the existing reader.
#[allow(clippy::too_many_arguments)]
fn land_domains(
    memory: &impl Memory,
    base: usize,
    state: usize,
    seed: &DivisionSeed,
    shared: &mut MovementSharedInputs<'_>,
    context: &mut NormalMovementContext,
) -> Result<
    (
        Option<MovementOwnerInputs>,
        Option<StrategicRedeploymentInputs>,
    ),
    &'static str,
> {
    if seed
        .naval_transfer_membership
        .ok_or("fresh counter ETA transfer input unavailable")?
        != 0
    {
        return Ok((None, None));
    }
    let land = movement_cached(Some(&mut context.land), || {
        movement_land_inputs(memory, base, state, seed.pointer, Some(&mut *shared))
    })?;
    if land.invasion() {
        return Ok((None, None));
    }
    let roots = movement_cached(Some(&mut context.roots), || {
        movement_owner_inputs(
            memory,
            base,
            state,
            seed.pointer,
            land.location,
            Some(&mut *shared),
        )
    })?;
    if roots.strategic == 0 {
        return Ok((Some(roots), None));
    }
    let rail = movement_cached(Some(&mut context.rail), || {
        movement_rail_inputs(memory, base, state, roots, Some(&*shared))
    })?;
    let operands = StrategicRedeploymentInputs {
        country_modifier: rail.country,
        state_modifier: rail.state,
        friendly_state_modifier: rail.friendly_state,
    };
    Ok((rail.blocked().then_some(roots), Some(operands)))
}

#[allow(clippy::too_many_arguments)]
fn capture_normal(
    memory: &impl Memory,
    base: usize,
    state: usize,
    seed: &DivisionSeed,
    roots: MovementOwnerInputs,
    land_terrains: LandTerrainDomain<'_>,
    legacy_terrains: Result<&[(i32, i64)], &'static str>,
    shared: &mut MovementSharedInputs<'_>,
    context: &mut NormalMovementContext,
    packet: &mut FreshCounterEtaPacket,
    required_terrain: &mut usize,
) -> Result<FreshNormalEtaRow, &'static str> {
    let terrains = land_terrains.source?;
    let legacy_terrains = legacy_terrains?;
    *required_terrain = required_terrain
        .checked_add(terrains.len())
        .ok_or("fresh counter ETA terrain demand overflow")?;
    if packet.growing {
        let required = packet
            .terrain
            .len()
            .checked_add(terrains.len())
            .ok_or("fresh counter ETA terrain demand overflow")?;
        eta_reserve(&mut packet.terrain, required)
            .map_err(|_| "fresh counter ETA allocation failed")?;
    }
    if terrains.len()
        > packet
            .terrain
            .capacity()
            .saturating_sub(packet.terrain.len())
    {
        return Err("fresh counter ETA terrain arena exhausted");
    }
    let common = movement_cached(Some(&mut context.normal), || {
        normal_movement_inputs(
            memory,
            base,
            state,
            seed.pointer,
            roots.owner_raw,
            roots.owner.pointer,
            roots.actual_state,
            Some(&mut *shared),
        )
    })?;
    let (fort_penalty, snow_penalty, river_adjustment) = normal_eta_penalties(
        memory,
        common.modifier.stats,
        &mut context.edge_observations,
    )?;
    let mut row = FreshNormalEtaRow {
        prepared: common.speed.prepare(),
        fort_penalty,
        snow_penalty,
        river_adjustment,
        river_leader: movement_cached(Some(&mut context.edge_observations.river_leader), || {
            movement_modifier(
                memory,
                base,
                common.modifier.leader_accumulator,
                keys::RIVER_CROSSING_FACTOR,
            )
        })?,
        terrain_start: packet.terrain.len(),
        terrain_len: 0,
    };
    let result = (|| {
        for (ordinal, terrain) in terrains.iter().enumerate() {
            if let Some((frontier, reason)) = land_terrains.first_invalid
                && ordinal == frontier
            {
                return Err(reason);
            }
            if let Some((_, value)) = legacy_terrains
                .iter()
                .find(|(index, _)| *index == terrain.terrain_index)
            {
                packet.terrain.push((terrain.terrain_index, *value));
                continue;
            }
            let seen = context
                .edge_observations
                .terrain
                .filter(|seen| seen.pointer == terrain.terrain);
            let mut value = match seen {
                Some(seen) => seen.unit?,
                None => crate::csub_unit_definition::movement_terrain(
                    memory,
                    base,
                    common.modifier.stats,
                    terrain.terrain,
                )?,
            };
            if common.modifier.leader != 0 {
                let leader = match seen.and_then(|seen| seen.leader) {
                    Some(result) => result?,
                    None => crate::carmy_leader::movement_terrain(
                        memory,
                        base,
                        common.modifier.leader,
                        terrain.terrain_index,
                    )?,
                };
                value = value.wrapping_add(leader);
            }
            packet.terrain.push((terrain.terrain_index, value));
        }
        Ok(())
    })();
    if let Err(reason) = result {
        packet.terrain.truncate(row.terrain_start);
        return Err(reason);
    }
    row.terrain_len = packet.terrain.len() - row.terrain_start;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const NORMAL_STATS: usize = 0x100;
    const NORMAL_TERRAINS: [usize; 2] = [0x600, 0x1000];
    const NORMAL_ADJUSTERS: usize = 0x1500;

    struct NormalMemory {
        bytes: [u8; 0x1600],
        fail_at: Option<usize>,
        reads: RefCell<Vec<(usize, usize)>>,
    }

    impl NormalMemory {
        fn new() -> Self {
            let mut bytes = [0_u8; 0x1600];
            let put = |bytes: &mut [u8], at: usize, value: &[u8]| {
                bytes[at..at + value.len()].copy_from_slice(value);
            };
            put(
                &mut bytes,
                NORMAL_STATS,
                &(crate::csub_unit_definition::VTABLE_RVA as u64).to_le_bytes(),
            );
            let array = NORMAL_STATS + crate::csub_unit_definition::MOVEMENT_TERRAIN_ADJUSTERS;
            put(&mut bytes, array, &(NORMAL_ADJUSTERS as u64).to_le_bytes());
            put(&mut bytes, array + 8, &2_u32.to_le_bytes());
            put(&mut bytes, array + 12, &2_u32.to_le_bytes());
            for (ordinal, terrain) in NORMAL_TERRAINS.into_iter().enumerate() {
                let entry = NORMAL_ADJUSTERS + ordinal * 0x28;
                let token = ordinal as u32 + 123;
                put(
                    &mut bytes,
                    terrain,
                    &(crate::cterrain_type::VTABLE_RVA as u64).to_le_bytes(),
                );
                put(&mut bytes, terrain + 8, &token.to_le_bytes());
                put(
                    &mut bytes,
                    terrain + 0x88,
                    &(ordinal as i32 + 1).to_le_bytes(),
                );
                for at in [entry, terrain + 0x168] {
                    put(&mut bytes, at, &0x0277_3a70_u64.to_le_bytes());
                }
                put(&mut bytes, entry + 8, &token.to_le_bytes());
                put(
                    &mut bytes,
                    entry + 0x20,
                    &[-7_i64, i64::MAX][ordinal].to_le_bytes(),
                );
                put(
                    &mut bytes,
                    terrain + 0x188,
                    &[4_i64, 1][ordinal].to_le_bytes(),
                );
            }
            Self {
                bytes,
                fail_at: None,
                reads: RefCell::new(Vec::new()),
            }
        }
    }

    impl Memory for NormalMemory {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, output.len()));
            self.fail_at != Some(address)
                && crate::memory::SliceMemory {
                    base: 0,
                    bytes: &self.bytes,
                }
                .copy(address, output)
        }
    }

    fn normal_domain() -> [crate::cprovince::StaticProvince; 2] {
        NORMAL_TERRAINS.map(|terrain| crate::cprovince::StaticProvince {
            pointer: 0,
            definition: 0,
            flags: 1,
            terrain,
            terrain_index: if terrain == NORMAL_TERRAINS[0] { 1 } else { 2 },
        })
    }

    fn normal_context() -> NormalMovementContext {
        NormalMovementContext {
            normal: Some(Ok(NormalMovementInputs {
                modifier: NormalModifierCommon {
                    value: NormalModifierOperands::default(),
                    stats: NORMAL_STATS,
                    owner: 0,
                    leader: 0,
                    leader_accumulator: 0,
                },
                speed: NormalSpeedOperands::default(),
            })),
            edge_observations: NormalEdgeObservations {
                fort_penalty: Some(Ok(0)),
                snow_penalty: Some(Ok(0)),
                river_adjustment: Some(Ok(0)),
                river_leader: Some(Ok(0)),
                terrain: None,
            },
            ..Default::default()
        }
    }

    fn normal_capture(
        memory: &impl Memory,
        domain: LandTerrainDomain<'_>,
        legacy: Result<&[(i32, i64)], &'static str>,
        context: &mut NormalMovementContext,
        packet: &mut FreshCounterEtaPacket,
        required: &mut usize,
    ) -> Result<FreshNormalEtaRow, &'static str> {
        let seed = DivisionSeed::default();
        let mut geometry_packet = crate::cprovince::GeometryPacket::vacant();
        let mut geometry = EtaGeometry {
            packet: &mut geometry_packet,
            memory,
        };
        let mut inputs = MovementSharedInputs {
            seed: &seed,
            route_ids: MovementRouteIds::Full(&[]),
            geometry: &mut geometry,
            rail_speeds: None,
            state_modifiers: None,
            category_contributions: None,
            supply_templates: None,
            air_prefixes: None,
        };
        let roots = MovementOwnerInputs {
            roster: 0,
            owner_raw: 0,
            owner: ccountry::Country {
                pointer: 0,
                canonical_slot: 0,
                id: 0,
            },
            actual_state: 0,
            strategic: 0,
        };
        capture_normal(
            memory,
            0,
            0,
            &seed,
            roots,
            domain,
            legacy,
            &mut inputs,
            context,
            packet,
            required,
        )
    }

    #[test]
    fn normal_frontier_preserves_an_earlier_native_failure_and_its_read_trace() {
        let mut memory = NormalMemory::new();
        memory.fail_at = Some(NORMAL_TERRAINS[0]);
        let mut terrains = normal_domain();
        terrains[1].flags = 0;
        let mut packet = FreshCounterEtaPacket::try_new(1, 3).unwrap();
        packet.terrain.push((99, -9));
        let mut required = 0;
        assert_eq!(
            normal_capture(
                &memory,
                LandTerrainDomain::new(Ok(&terrains)),
                Ok(&[]),
                &mut normal_context(),
                &mut packet,
                &mut required
            )
            .err(),
            Some("movement stat operand unreadable")
        );
        assert_eq!(
            *memory.reads.borrow(),
            [(NORMAL_STATS, 8), (NORMAL_TERRAINS[0], 8)]
        );
        assert_eq!(packet.terrain(), [(99, -9)]);
        assert_eq!(required, 2);
    }

    #[test]
    fn normal_frontier_precedes_legacy_and_cached_reuse_and_rolls_back_only_its_row() {
        let memory = NormalMemory::new();
        let terrains = normal_domain();
        let mut packet = FreshCounterEtaPacket::try_new(3, 5).unwrap();
        let row = normal_capture(
            &memory,
            LandTerrainDomain::new(Ok(&terrains)),
            Ok(&[]),
            &mut normal_context(),
            &mut packet,
            &mut 0,
        )
        .unwrap();
        assert_eq!((row.terrain_start, row.terrain_len), (0, 2));
        assert_eq!(packet.terrain(), [(1, -3), (2, i64::MIN)]);
        assert_eq!(memory.reads.borrow().len(), 20);
        let first_trace = memory.reads.borrow()[..10].to_vec();
        memory.reads.borrow_mut().clear();
        for (flags, index, reason) in [
            (1, 1, "fresh counter ETA LAND terrain listed twice"),
            (0, 1, "fresh counter ETA LAND terrain domain invalid"),
        ] {
            let mut invalid = terrains;
            invalid[1].flags = flags;
            invalid[1].terrain_index = index;
            let domain = LandTerrainDomain::new(Ok(&invalid));
            let mut context = normal_context();
            context.edge_observations.terrain = Some(TerrainObservation {
                pointer: invalid[0].terrain,
                unit: Err("earlier cached terrain failure"),
                leader: None,
            });
            assert_eq!(
                normal_capture(&memory, domain, Ok(&[]), &mut context, &mut packet, &mut 0).err(),
                Some("earlier cached terrain failure")
            );
            assert!(memory.reads.borrow().is_empty());
            context = normal_context();
            assert_eq!(
                normal_capture(&memory, domain, Ok(&[]), &mut context, &mut packet, &mut 0).err(),
                Some(reason)
            );
            assert_eq!(*memory.reads.borrow(), first_trace);
            memory.reads.borrow_mut().clear();
            context = normal_context();
            context.edge_observations.terrain = Some(TerrainObservation {
                pointer: invalid[1].terrain,
                unit: Ok(123),
                leader: None,
            });
            // Both ordinals match this legacy index. The reached frontier must
            // refuse before reuse; first ordinal's successful reuse is retained
            // only until this failed row rolls back.
            assert_eq!(
                normal_capture(
                    &memory,
                    domain,
                    Ok(&[(1, -17)]),
                    &mut context,
                    &mut packet,
                    &mut 0
                )
                .err(),
                Some(reason)
            );
            assert!(memory.reads.borrow().is_empty());
            assert_eq!(packet.terrain(), [(1, -3), (2, i64::MIN)]);
        }
    }

    #[test]
    fn normal_frontier_keeps_domain_legacy_capacity_and_common_operand_precedence() {
        let memory = NormalMemory::new();
        let mut terrains = normal_domain();
        terrains[0].flags = 0;
        let domain = LandTerrainDomain::new(Ok(&terrains));
        let mut packet = FreshCounterEtaPacket::try_new(1, 2).unwrap();
        let mut context = normal_context();
        context.normal = Some(Err("common failure"));
        let mut required = 0;
        assert_eq!(
            normal_capture(
                &memory,
                LandTerrainDomain::new(Err("LAND failure")),
                Err("legacy failure"),
                &mut context,
                &mut packet,
                &mut required
            )
            .err(),
            Some("LAND failure")
        );
        assert_eq!(
            normal_capture(
                &memory,
                domain,
                Err("legacy failure"),
                &mut context,
                &mut packet,
                &mut required
            )
            .err(),
            Some("legacy failure")
        );
        assert_eq!(required, 0);
        let mut short = FreshCounterEtaPacket::try_new(1, 0).unwrap();
        assert_eq!(
            normal_capture(
                &memory,
                domain,
                Ok(&[]),
                &mut context,
                &mut short,
                &mut required
            )
            .err(),
            Some("fresh counter ETA terrain arena exhausted")
        );
        assert_eq!(required, 2);
        assert_eq!(
            normal_capture(
                &memory,
                domain,
                Ok(&[]),
                &mut context,
                &mut packet,
                &mut required
            )
            .err(),
            Some("common failure")
        );
        context = normal_context();
        context.edge_observations.fort_penalty = Some(Err("penalty failure"));
        assert_eq!(
            normal_capture(
                &memory,
                domain,
                Ok(&[]),
                &mut context,
                &mut packet,
                &mut required
            )
            .err(),
            Some("penalty failure")
        );
        context = normal_context();
        context.edge_observations.river_leader = Some(Err("river failure"));
        assert_eq!(
            normal_capture(
                &memory,
                domain,
                Ok(&[]),
                &mut context,
                &mut packet,
                &mut required
            )
            .err(),
            Some("river failure")
        );
        assert!(memory.reads.borrow().is_empty());
    }

    struct EndpointsOnly<'a> {
        values: &'a [(usize, u32)],
        reads: RefCell<Vec<usize>>,
    }

    impl Memory for EndpointsOnly<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push(address);
            let Some((_, value)) = self.values.iter().find(|(at, _)| *at == address) else {
                return false;
            };
            if output.len() != 4 {
                return false;
            }
            output.copy_from_slice(&value.to_le_bytes());
            true
        }
    }

    struct RefusedField<'a> {
        source: crate::memory::SliceMemory<'a>,
        address: usize,
        attempts: std::cell::Cell<usize>,
    }

    impl Memory for RefusedField<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == self.address {
                self.attempts.set(self.attempts.get() + 1);
                return false;
            }
            self.source.copy(address, output)
        }
    }

    fn capture_fixture(
        memory: &impl Memory,
        world: &super::super::eta_tests::World,
        seed: DivisionSeed,
    ) -> FreshCounterEtaRow {
        capture_fixture_with_legacy(memory, world, seed, false).unwrap()
    }

    fn capture_fixture_with_legacy(
        memory: &impl Memory,
        world: &super::super::eta_tests::World,
        seed: DivisionSeed,
        legacy: bool,
    ) -> Result<FreshCounterEtaRow, &'static str> {
        let count = counter_land_terrain_capacity(memory, world.state).unwrap();
        let mut geometry = crate::cprovince::GeometryPacket::with_capacities(
            crate::cprovince::GeometryCapacities {
                provinces: count,
                neighbour_entries: count * 6,
            },
        );
        let mut statics = crate::cprovince::StaticGeometry::default();
        statics.reserve_provinces(count).unwrap();
        geometry.begin(memory, world.base, world.state).unwrap();
        let mut shared = EtaInputPacket::new(1, count, 16).unwrap();
        if legacy {
            let walk = capture_eta_inputs(
                memory,
                world.base,
                world.state,
                &[seed],
                0..1,
                Outcome::Rows { len: 1 },
                1,
                count,
                16,
                &mut geometry,
                &mut statics,
                &mut shared,
            );
            assert!(matches!(walk.outcome, Outcome::Unavailable { .. }));
            assert_eq!(shared.fresh_observations.len(), 1);
        } else {
            assert!(statics.begin(memory, world.base, world.state));
        }
        let mut terrains = vec![
            crate::cprovince::StaticProvince {
                pointer: 0,
                definition: 0,
                flags: 0,
                terrain: 0,
                terrain_index: 0,
            };
            count
        ];
        let len = if legacy {
            0
        } else {
            capture_counter_land_terrains(
                memory,
                world.base,
                world.state,
                &mut geometry,
                &mut statics,
                &mut terrains,
            )
            .unwrap()
        };
        let mut packet = FreshCounterEtaPacket::try_new(1, len).unwrap();
        let walk = capture_fresh_counter_eta(
            memory,
            world.base,
            world.state,
            19,
            &[seed],
            0..1,
            (0, 1),
            Outcome::Rows { len: 1 },
            Ok(&terrains[..len]),
            &mut geometry,
            &mut shared,
            &mut packet,
        );
        assert!(shared.fresh_observations.is_empty());
        match walk.outcome {
            Outcome::Rows { len: 1 } => Ok(packet.rows()[0]),
            Outcome::Unavailable { reason } => {
                assert_eq!(walk.required_rows, 1);
                assert!(packet.rows().is_empty());
                Err(reason)
            }
            _ => panic!("unexpected fresh fixture outcome"),
        }
    }

    #[test]
    fn actual_edge_failure_preserves_fresh_normal_domain() {
        let world =
            super::super::eta_tests::World::synthetic(super::super::eta_tests::Shape::small());
        let mut seed = world.seeds[0];
        seed.naval_transfer_membership = Some(0);
        seed.strategic_redeployment = Some(0);
        let source = world.memory();
        let route = Array::from_header(seed.ordinary_route.unwrap(), 4).unwrap();
        let first = read::<u32>(&source, route.element(0, 4).unwrap()).unwrap();
        let provinces =
            presentation_array(&source, world.state, cgame_state::PROVINCES, 8).unwrap();
        let next = read::<u64>(&source, provinces.element(first, 8).unwrap()).unwrap() as usize;
        let memory = RefusedField {
            source,
            address: next + crate::cprovince::MOVEMENT_BUILDINGS,
            attempts: std::cell::Cell::new(0),
        };
        let row = capture_fixture(&memory, &world, seed);
        assert!(row.current_speed.is_err());
        assert!(row.normal.is_ok_and(|normal| normal.is_some()));
        assert!(row.rail.is_ok_and(|rail| rail.is_none()));
        assert_eq!(memory.attempts.get(), 1);
    }

    #[test]
    fn shared_normal_failure_is_retained_without_retry() {
        let world =
            super::super::eta_tests::World::synthetic(super::super::eta_tests::Shape::small());
        let mut seed = world.seeds[0];
        seed.naval_transfer_membership = Some(0);
        seed.strategic_redeployment = Some(0);
        let memory = RefusedField {
            source: world.memory(),
            address: seed.stat_block.unwrap() + 0x290,
            attempts: std::cell::Cell::new(0),
        };
        let row = capture_fixture(&memory, &world, seed);
        assert!(row.current_speed.is_err());
        assert!(row.normal.is_err());
        assert!(row.rail.is_ok_and(|rail| rail.is_none()));
        assert_eq!(memory.attempts.get(), 1);
    }

    #[test]
    fn failed_first_id_keeps_transfer_domains_unqueried() {
        let world =
            super::super::eta_tests::World::synthetic(super::super::eta_tests::Shape::small());
        let mut seed = world.seeds[0];
        seed.naval_transfer_membership = Some(1);
        let route = Array::from_header(seed.ordinary_route.unwrap(), 4).unwrap();
        let memory = RefusedField {
            source: world.memory(),
            address: route.element(0, 4).unwrap(),
            attempts: std::cell::Cell::new(0),
        };
        let row = capture_fixture(&memory, &world, seed);
        assert!(row.current_speed.is_err());
        assert!(row.normal.is_ok_and(|normal| normal.is_none()));
        assert!(row.rail.is_ok_and(|rail| rail.is_none()));
        assert_eq!(memory.attempts.get(), 1);
    }

    #[test]
    fn refused_legacy_prefix_skips_fresh_getters_and_clears_handoff() {
        let world =
            super::super::eta_tests::World::synthetic(super::super::eta_tests::Shape::small());
        let mut seed = world.seeds[0];
        seed.naval_transfer_membership = Some(0);
        seed.strategic_redeployment = Some(0);
        let source = world.memory();
        let route = Array::from_header(seed.ordinary_route.unwrap(), 4).unwrap();
        let first = read::<u32>(&source, route.element(0, 4).unwrap()).unwrap();
        let provinces =
            presentation_array(&source, world.state, cgame_state::PROVINCES, 8).unwrap();
        let next = read::<u64>(&source, provinces.element(first, 8).unwrap()).unwrap() as usize;
        let memory = RefusedField {
            source,
            address: next + crate::cprovince::MOVEMENT_BUILDINGS,
            attempts: std::cell::Cell::new(0),
        };
        assert!(capture_fixture_with_legacy(&memory, &world, seed, true).is_err());
        assert_eq!(memory.attempts.get(), 1);
    }

    #[test]
    fn null_destination_common_failure_is_not_retried_by_normal_domain() {
        let mut world =
            super::super::eta_tests::World::synthetic(super::super::eta_tests::Shape::small());
        let mut seed = world.seeds[0];
        seed.naval_transfer_membership = Some(0);
        seed.strategic_redeployment = Some(0);
        let route = Array::from_header(seed.ordinary_route.unwrap(), 4).unwrap();
        let first = read::<u32>(&world.memory(), route.element(0, 4).unwrap()).unwrap();
        let provinces =
            presentation_array(&world.memory(), world.state, cgame_state::PROVINCES, 8).unwrap();
        let slot = provinces.element(first, 8).unwrap() - world.base;
        world.bytes[slot..slot + 8].copy_from_slice(&0_u64.to_le_bytes());
        let memory = RefusedField {
            source: world.memory(),
            address: seed.stat_block.unwrap() + 0x290,
            attempts: std::cell::Cell::new(0),
        };
        let row = capture_fixture(&memory, &world, seed);
        assert!(row.current_speed.is_err());
        assert!(row.normal.is_err());
        assert!(row.rail.is_ok_and(|rail| rail.is_none()));
        assert_eq!(memory.attempts.get(), 1);
    }

    #[test]
    fn legacy_refusal_counts_demand_without_any_source_read() {
        struct NoReads;
        impl Memory for NoReads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("legacy refusal must not perform a fresh source read")
            }
        }
        let mut header = [0_u8; 16];
        header[..8].copy_from_slice(&0x1000_u64.to_le_bytes());
        header[8..12].copy_from_slice(&1_u32.to_le_bytes());
        header[12..].copy_from_slice(&1_u32.to_le_bytes());
        let seed = DivisionSeed {
            pointer: 0x100,
            division: 7,
            ordinary_route: Some(header),
            ..Default::default()
        };
        let mut shared = EtaInputPacket::new(1, 0, 0).unwrap();
        shared.result.outcome = Outcome::Unavailable {
            reason: "legacy route unreadable",
        };
        shared.counters.walks = 1;
        shared.fresh_observations.push(DivisionEtaObservation {
            division: seed.division,
            pointer: seed.pointer,
            context: Some(NormalMovementContext {
                army: seed.pointer,
                ..Default::default()
            }),
        });
        let mut geometry = crate::cprovince::GeometryPacket::vacant();
        let mut packet = FreshCounterEtaPacket::try_new(1, 0).unwrap();
        let walk = capture_fresh_counter_eta(
            &NoReads,
            0,
            0,
            19,
            &[seed],
            0..1,
            (0, 1),
            Outcome::Rows { len: 1 },
            Err("LAND roster not captured"),
            &mut geometry,
            &mut shared,
            &mut packet,
        );
        assert_eq!(
            walk.outcome,
            Outcome::Unavailable {
                reason: "legacy route unreadable"
            }
        );
        assert_eq!(walk.required_rows, 1);
        assert!(packet.rows().is_empty());
        assert!(shared.fresh_observations.is_empty());
    }

    #[test]
    fn endpoint_scan_leaves_unreadable_middle_unqueried() {
        let memory = EndpointsOnly {
            values: &[(0x1000, 17), (0x1010, 93)],
            reads: RefCell::new(Vec::new()),
        };
        let last = std::cell::Cell::new(None);
        let ids = route_endpoints(
            &memory,
            Array {
                data: 0x1000,
                count: 5,
            },
            &last,
        );
        assert_eq!(ids.get(&memory, 0), Ok(Some(17)));
        assert_eq!(ids.get(&memory, 4), Ok(Some(93)));
        assert_eq!(ids.get(&memory, 2), Ok(None));
        assert_eq!(*memory.reads.borrow(), [0x1000, 0x1010]);
    }

    #[test]
    fn one_entry_route_reads_once_and_keeps_same_endpoint() {
        let memory = EndpointsOnly {
            values: &[(0x1000, 17)],
            reads: RefCell::new(Vec::new()),
        };
        let last = std::cell::Cell::new(None);
        let ids = route_endpoints(
            &memory,
            Array {
                data: 0x1000,
                count: 1,
            },
            &last,
        );
        assert_eq!(ids.get(&memory, 0), Ok(Some(17)));
        assert_eq!(ids.get(&memory, 1), Ok(None));
        assert_eq!(*memory.reads.borrow(), [0x1000]);
    }

    #[test]
    fn failed_last_endpoint_is_not_queried_until_the_normal_branch() {
        let memory = EndpointsOnly {
            values: &[(0x1000, 17)],
            reads: RefCell::new(Vec::new()),
        };
        let last = std::cell::Cell::new(None);
        let ids = route_endpoints(
            &memory,
            Array {
                data: 0x1000,
                count: 5,
            },
            &last,
        );
        assert_eq!(ids.get(&memory, 0), Ok(Some(17)));
        assert_eq!(*memory.reads.borrow(), [0x1000]);
        assert!(ids.get(&memory, 4).is_err());
        assert!(ids.get(&memory, 4).is_err());
        assert_eq!(*memory.reads.borrow(), [0x1000, 0x1010]);
    }
}
