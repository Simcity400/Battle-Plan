use super::*;

/// Pooled owned movement inputs. Workers grow from retained seed headers before
/// the payload; fixed readers refuse a complete unit on a route shortage.
pub struct MovementPacket {
    growing: bool,
    pub(super) units: Vec<MovementRow>,
    pub(super) paths: Vec<u32>,
    pub(super) combats: MovementCombats,
    combat_references: Vec<cunit::MapCombatReference>,
    sample_hour: Option<i32>,
    required_combat_references: usize,
    /// Pooled staging for one division's route bytes; holds no row data.
    route_bytes: Vec<u8>,
    pub(super) result: MovementWalk,
}
impl MovementPacket {
    pub fn try_new(unit_capacity: usize, path_capacity: usize) -> std::io::Result<Self> {
        let paths = crate::allocation::vec(path_capacity)?;
        let bytes = Self::route_byte_capacity(path_capacity)?;
        Ok(Self {
            growing: false,
            units: crate::allocation::vec(unit_capacity)?,
            paths,
            combats: MovementCombats::try_new(unit_capacity)?,
            combat_references: Vec::new(),
            sample_hour: None,
            required_combat_references: 0,
            route_bytes: crate::allocation::vec(bytes)?,
            result: Self::vacant().result,
        })
    }

    #[cfg(test)]
    pub fn new(unit_capacity: usize, path_capacity: usize) -> Self {
        Self::try_new(unit_capacity, path_capacity).unwrap()
    }

    pub fn vacant() -> Self {
        Self {
            growing: false,
            units: Vec::new(),
            paths: Vec::new(),
            combats: MovementCombats::default(),
            combat_references: Vec::new(),
            sample_hour: None,
            required_combat_references: 0,
            route_bytes: Vec::new(),
            result: MovementWalk {
                outcome: Outcome::Unavailable {
                    reason: "division movement not captured",
                },
                paths_len: 0,
                required_paths: 0,
            },
        }
    }

    fn route_byte_capacity(path_capacity: usize) -> std::io::Result<usize> {
        path_capacity
            .checked_mul(size_of::<u32>())
            .ok_or_else(crate::allocation::oom)
    }

    pub fn enable_growth(&mut self) {
        self.growing = true;
    }

    pub fn clear(&mut self) {
        self.units.clear();
        self.paths.clear();
        self.combats.clear();
        self.combat_references.clear();
        self.sample_hour = None;
        self.required_combat_references = 0;
        self.result = MovementWalk {
            outcome: Outcome::Unavailable {
                reason: "division movement not captured",
            },
            paths_len: 0,
            required_paths: 0,
        };
    }
    pub fn reserve(&mut self, unit_capacity: usize, path_capacity: usize) -> std::io::Result<()> {
        let units = unit_capacity.saturating_sub(self.units.len());
        let paths = path_capacity.saturating_sub(self.paths.len());
        crate::allocation::reserve_exact(&mut self.units, units)?;
        crate::allocation::reserve_exact(&mut self.paths, paths)?;
        self.combats.reserve(unit_capacity)?;
        let bytes = Self::route_byte_capacity(path_capacity)?;
        let additional = bytes.saturating_sub(self.route_bytes.len());
        crate::allocation::reserve_exact(&mut self.route_bytes, additional)
    }
    pub fn allocated_bytes(&self) -> usize {
        self.units.capacity() * std::mem::size_of::<MovementRow>()
            + self.paths.capacity() * std::mem::size_of::<u32>()
            + self.combats.allocated_bytes()
            + self.combat_references.capacity() * size_of::<cunit::MapCombatReference>()
            + self.route_bytes.capacity()
    }
    pub fn reserve_combat_references(&mut self, capacity: usize) -> std::io::Result<()> {
        let additional = capacity.saturating_sub(self.combat_references.len());
        crate::allocation::reserve_exact(&mut self.combat_references, additional)
    }
    pub fn combat_references_required(&self) -> usize {
        self.required_combat_references
    }
    pub fn combat_references_capacity(&self) -> usize {
        self.combat_references.capacity()
    }
    pub fn combat_references_hour(&self) -> Option<i32> {
        self.sample_hour
    }
    pub fn combat_reference_failures(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.units[..self.result.outcome.len()]
            .iter()
            .filter_map(|row| row.combat_references_failure)
    }
    pub fn combat_references(&self, row: &MovementRow) -> Option<&[cunit::MapCombatReference]> {
        let (start, count) = row.combat_references?;
        if count != row.combat_count as usize {
            return None;
        }
        self.combat_references.get(start..start.checked_add(count)?)
    }
    pub fn refuse_selected_retreat(&mut self) -> [u64; 2] {
        let mut refused = [0; 2];
        for row in &mut self.units {
            if row.selected_commander == Some(true) {
                for (index, value) in row.selected_retreat.iter_mut().enumerate() {
                    refused[index] += u64::from(value.take().is_some());
                }
            }
        }
        refused
    }
    pub fn units_for_selected(&self) -> &[MovementRow] {
        &self.units[..self.result.outcome.len()]
    }
    pub fn outcome(&self) -> MovementWalk {
        self.result
    }
    /// Independent current-pass suffix refusals; established row coverage is
    /// reported by outcome(), so unavailable prefixes add no false refusals.
    pub fn movement_style_refusals(&self) -> (usize, usize) {
        self.units[..self.result.outcome.len()].iter().fold(
            (0, 0),
            |(withdrawal, priority), row| {
                (
                    withdrawal + usize::from(row.withdrawal.is_none()),
                    priority + usize::from(row.movement_priority.is_none()),
                )
            },
        )
    }
}

pub(super) fn movement_route_prefixes(
    memory: &impl Memory,
    route: Array,
    alternate: Array,
    staging: &mut Vec<u8>,
    path: &mut [u32],
    alternate_path: &mut [u32],
) -> Result<(), &'static str> {
    if path.is_empty() && alternate_path.is_empty() {
        return Ok(());
    }
    // Sized to the routes read: zeroing the whole staging buffer per division
    // would cost more than the copy. Stale bytes are
    // harmless because only a fully copied span is decoded.
    let bytes = path
        .len()
        .checked_add(alternate_path.len())
        .and_then(|len| len.checked_mul(4))
        .ok_or("movement route byte count overflow")?;
    if bytes > staging.capacity() {
        return Err("movement route staging capacity exhausted");
    }
    if staging.len() < bytes {
        staging.resize(bytes, 0);
    }
    let (first, second) = staging[..bytes].split_at_mut(path.len() * 4);
    let copied = memory.copy_scattered_prefetched(
        [route.data, alternate.data],
        &mut [&mut *first, &mut *second],
    );
    for (index, (bytes, output)) in [(first, path), (second, alternate_path)]
        .into_iter()
        .enumerate()
    {
        if index >= copied {
            return Err("movement route unreadable");
        }
        // Retain a completed ordinary prefix even if the alternate copy failed.
        // A failed span may be partial and must never enter the owned arena.
        for (slot, word) in output.iter_mut().zip(bytes.chunks_exact(4)) {
            *slot = u32::from_le_bytes(word.try_into().unwrap());
        }
    }
    Ok(())
}

/// Acquire exact admitted routes and dependent inputs from the shared census.
/// `geometry` is initialized for this current pass and belongs to this worker;
/// only the immutable fixed division seeds are shared between workers.
#[allow(clippy::too_many_arguments)]
pub fn capture_movement_inputs(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    seeds: &[DivisionSeed],
    source_outcome: Outcome,
    row_capacity: usize,
    path_capacity: usize,
    geometry: &mut crate::cprovince::GeometryPacket,
    packet: &mut MovementPacket,
) -> MovementWalk {
    capture_movement_inputs_with_combat_references(
        memory,
        base,
        game_state,
        seeds,
        source_outcome,
        row_capacity,
        path_capacity,
        geometry,
        packet,
        None,
    )
}

/// Existing callback census, with caller-associated attempted hour. This
/// enables only optional field acquisition; legacy readers pass no hour.
#[allow(clippy::too_many_arguments)]
pub fn capture_movement_inputs_with_combat_references(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    seeds: &[DivisionSeed],
    source_outcome: Outcome,
    row_capacity: usize,
    path_capacity: usize,
    geometry: &mut crate::cprovince::GeometryPacket,
    packet: &mut MovementPacket,
    sample_hour: Option<i32>,
) -> MovementWalk {
    packet.clear();
    packet.sample_hour = sample_hour;
    let mut registry = crate::creference_registry::HeaderResolver::default();
    let mut paths_len = 0;
    let mut required_paths = 0usize;
    let result = (|| {
        if let Outcome::Unavailable { reason } = source_outcome {
            return Err(reason);
        }
        if source_outcome.len() != seeds.len() {
            return Err("division movement source length invalid");
        }
        if packet.growing {
            packet
                .reserve(seeds.len(), 0)
                .map_err(|_| "movement seed allocation failed")?;
        }
        // Preserve independent root availability even with a shared country
        // census. Geometry admits its own current province roster in begin().
        for (offset, stride) in [(cgame_state::COUNTRIES, 8), (cgame_state::COUNTRY_LINKS, 4)] {
            game_state
                .checked_add(offset)
                .and_then(|at| Array::read(memory, at, stride))
                .ok_or("division movement root invalid")?;
        }
        geometry
            .ready()
            .map_err(|_| "division movement root invalid")?;
        let mut len = 0usize;
        let mut missing = match source_outcome {
            Outcome::Truncated { unread_rows, .. } => unread_rows,
            _ => 0,
        };
        for seed in seeds {
            let expected = base
                .checked_add(VTABLE_RVA)
                .ok_or("module address overflow")?;
            if seed.primary_vtable != Some(expected as u64) {
                return Err("movement division vtable mismatch");
            }
            if movement_seed_value(seed.handle)? != seed.division || seed.division == 0 {
                return Err("movement division identity changed");
            }
            seed.pointer
                .checked_add(cunit::ORDINARY_ROUTE_DATA)
                .ok_or("route overflow")?;

            let route = seed
                .ordinary_route
                .and_then(|header| Array::from_header(header, 4))
                .ok_or("ordinary route invalid")?;
            seed.pointer
                .checked_add(cunit::ALTERNATE_ROUTE_DATA)
                .ok_or("route overflow")?;

            let alternate = seed
                .alternate_route
                .and_then(|header| Array::from_header(header, 4))
                .ok_or("alternate route invalid")?;
            // Whole routes, sized from the original live count before payload.
            let path_len = route.count as usize;
            let alternate_len = alternate.count as usize;
            let needed = path_len + alternate_len;
            required_paths = required_paths
                .checked_add(needed)
                .ok_or("route arena overflow")?;
            if packet.growing {
                packet
                    .reserve(seeds.len(), required_paths)
                    .map_err(|_| "movement route allocation failed")?;
            }
            // Pressure reuses the already-owned seed header, even if an old
            // row/route capacity gap omits this unit. An invalid optional hint
            // supplies no demand; the established reader retains its own error.
            let mut reference_demand_overflow = false;
            if sample_hour.is_some()
                && let Some(combat) = seed.combat.and_then(|header| Array::from_header(header, 8))
            {
                match packet
                    .required_combat_references
                    .checked_add(combat.count as usize)
                {
                    Some(required) => packet.required_combat_references = required,
                    None => {
                        packet.required_combat_references = usize::MAX;
                        reference_demand_overflow = true;
                    }
                }
            }
            if len
                == (if packet.growing {
                    seeds.len()
                } else {
                    row_capacity
                })
                .min(packet.units.capacity())
                || (if packet.growing {
                    packet.paths.capacity()
                } else {
                    path_capacity
                })
                .min(packet.paths.capacity())
                .saturating_sub(paths_len)
                    < needed
            {
                missing = missing
                    .checked_add(1)
                    .ok_or("movement missing count overflow")?;
                continue;
            }
            let numerator = movement_seed_value(seed.movement_numerator)?;
            let path_start = paths_len;
            let alternate_start = paths_len + path_len;
            packet.paths.resize(paths_len + needed, 0);
            let (path, alternate_path) = packet.paths[paths_len..].split_at_mut(path_len);
            movement_route_prefixes(
                memory,
                route,
                alternate,
                &mut packet.route_bytes,
                path,
                alternate_path,
            )?;
            if path.first() == Some(&0) {
                return Err("nonempty movement route has null next province");
            }

            let combat = seed
                .combat
                .and_then(|header| Array::from_header(header, 8))
                .ok_or("division presentation array invalid")?;
            let mut combat_role = 0;
            let mut combat_province = 0;
            let mut first_combat = 0;
            if combat.count != 0 {
                let pointer = read::<u64>(memory, combat.data)
                    .filter(|&p| p != 0)
                    .ok_or("first combat unreadable or null")?
                    as usize;
                first_combat = pointer;
                (combat_role, combat_province) = packet.combats.capture(
                    memory,
                    base,
                    game_state,
                    pointer,
                    seed.logical_owner,
                    |province| geometry.acquire_province_id(memory, province),
                )?;
            }
            let target_pointer = movement_seed_value(seed.movement_target)?;
            let target = geometry.acquire_province_id(memory, target_pointer)?;
            let transfer = movement_seed_value(seed.naval_transfer_membership)? != 0;
            let source_pointer = movement_seed_value(seed.movement_source)?;
            let source = geometry.acquire_province_id(memory, source_pointer)?;
            let retreat = movement_seed_value(seed.retreat)?;
            let strategic_redeployment = movement_seed_value(seed.strategic_redeployment)?;
            let entity = movement_seed_value(seed.map_entity)?;
            let strategic_action = strategic_action::capture(
                memory,
                base,
                game_state,
                seed.pointer,
                route.count,
                target_pointer,
                retreat,
                strategic_redeployment,
                Some(seed),
            );
            let mut combat_references = None;
            let mut combat_references_failure = None;
            if sample_hour.is_some() {
                if !reference_demand_overflow {
                    let start = packet.combat_references.len();
                    let count = combat.count as usize;
                    if packet.growing {
                        packet
                            .reserve_combat_references(
                                start
                                    .checked_add(count)
                                    .ok_or("combat-reference demand overflow")?,
                            )
                            .map_err(|_| "combat-reference allocation failed")?;
                    }
                    let available = packet.combat_references.capacity() - start;
                    if count <= available {
                        // Capacity was prepared off-thread; resize cannot grow.
                        packet
                            .combat_references
                            .resize(start + count, Default::default());
                        match cunit::read_unit_combat_references(
                            memory,
                            base,
                            seed.pointer,
                            seed.combat.expect("validated same source header"),
                            first_combat,
                            &mut packet.combat_references[start..],
                            &mut registry,
                        ) {
                            Ok(count) => combat_references = Some((start, count)),
                            Err(reason) => {
                                packet.combat_references.truncate(start);
                                combat_references_failure = Some(reason);
                            }
                        }
                    } else {
                        combat_references_failure =
                            Some("unit combat-reference arena capacity exceeded");
                    }
                } else {
                    combat_references_failure = Some("unit combat-reference demand overflow");
                }
            }
            packet.units.push(MovementRow {
                combat_references,
                combat_references_failure,
                selected_exiled: None,
                selected_commander: None,
                selected_retreat: [None; 2],
                selected_retreat_failures: [None; 2],
                division: seed.division,
                source,
                target,
                numerator,
                retreat,
                strategic_redeployment,
                strategic_action,
                naval_transfer: u8::from(transfer),
                combat_count: combat.count,
                combat_role,
                combat_province,
                map_entity: u8::from(entity != 0),
                path_start,
                path_len,
                path_total: route.count,
                alternate_start,
                alternate_len,
                alternate_total: alternate.count,
                withdrawal: None,
                movement_priority: None,
            });
            paths_len += needed;
            len += 1;
        }
        Ok(if missing == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: missing,
            }
        })
    })();
    let outcome = result.unwrap_or_else(|reason| Outcome::Unavailable { reason });
    // Every established row and route finishes before the optional style
    // suffix. The admitted rows retain census order with capacity gaps, so
    // stream the same authenticated seeds without retaining another pointer
    // arena or allocating. A refusal neither erases a row nor skips its peer.
    let mut index = 0;
    for seed in seeds {
        let Some(row) = packet.units[..outcome.len()].get_mut(index) else {
            break;
        };
        if row.division != seed.division {
            continue;
        }
        if sample_hour.is_some() {
            row.selected_exiled = seed.exiled;
            let (context, values) = if row.combat_count as i32 > 0 {
                crate::carmy_leader::current_retreat_inputs(memory, base, seed)
            } else {
                (None, [Err("selected retreat not demanded"); 2])
            };
            row.selected_commander = context;
            row.selected_retreat = values.map(Result::ok);
            row.selected_retreat_failures = values.map(Result::err);
        }
        row.withdrawal = cunit::read_withdrawal(memory, seed.pointer);
        row.movement_priority = seed
            .pointer
            .checked_add(cunit::MOVEMENT_PRIORITY)
            .and_then(|at| read(memory, at));
        index += 1;
    }
    packet.result = MovementWalk {
        outcome,
        paths_len,
        required_paths,
    };
    packet.result
}

/// Pure projection from owned inputs. This function has no native-memory,
/// module-base or game-state parameter and cannot follow retained pointers.
pub fn project_movement(
    packet: &MovementPacket,
    output: &mut [MovementRow],
    paths: &mut [u32],
) -> MovementWalk {
    if output.len() < packet.units.len() || paths.len() < packet.paths.len() {
        return MovementWalk {
            outcome: Outcome::Unavailable {
                reason: "division movement projection capacity exhausted",
            },
            ..packet.result
        };
    }
    output[..packet.units.len()].copy_from_slice(&packet.units);
    paths[..packet.paths.len()].copy_from_slice(&packet.paths);
    packet.result
}

/// Reads only sources from this callback's independently authenticated census.
/// No engine call, retained live pointer, allocation, or pathfinding request.
#[cfg(test)]
pub fn walk_movement(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    sources: &[crate::cequipment_variant_pool::DivisionSource],
    source_outcome: Outcome,
    output: &mut [MovementRow],
    paths: &mut [u32],
) -> MovementWalk {
    walk_movement_internal(
        memory,
        base,
        game_state,
        sources,
        source_outcome,
        output,
        paths,
        None,
    )
}

/// Acquire shared geography lazily within the current call. Failed unit, route,
/// or render inputs retain their original outcome.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub fn walk_movement_with_inputs(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    sources: &[crate::cequipment_variant_pool::DivisionSource],
    source_outcome: Outcome,
    output: &mut [MovementRow],
    paths: &mut [u32],
    inputs: &mut MovementInputs,
) -> MovementWalk {
    inputs.shared.get_mut().clear();
    let result = walk_movement_internal(
        memory,
        base,
        game_state,
        sources,
        source_outcome,
        output,
        paths,
        Some(inputs),
    );
    inputs.shared.get_mut().clear();
    result
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
fn walk_movement_internal(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    sources: &[crate::cequipment_variant_pool::DivisionSource],
    source_outcome: Outcome,
    output: &mut [MovementRow],
    paths: &mut [u32],
    inputs: Option<&MovementInputs>,
) -> MovementWalk {
    let mut paths_len = 0;
    let mut required_paths = 0_usize;
    let result = (|| {
        if let Outcome::Unavailable { reason } = source_outcome {
            return Err(reason);
        }
        if source_outcome.len() != sources.len() {
            return Err("division movement source length invalid");
        }
        let array = |offset, stride| {
            game_state
                .checked_add(offset)
                .and_then(|p| Array::read(memory, p, stride))
                .ok_or("division movement root invalid")
        };
        let reader = Reader {
            memory,
            base,
            countries: array(cgame_state::COUNTRIES, 8)?,
            links: array(cgame_state::COUNTRY_LINKS, 4)?,
            provinces: game_state
                .checked_add(cgame_state::PROVINCES)
                .and_then(|address| shared_presentation_read(memory, inputs, address))
                .and_then(|header| Array::from_header(header, 8))
                .ok_or("division movement root invalid")?,
        };
        let province_id = |pointer| reader.province_cached(pointer, inputs);
        let mut len = 0;
        let mut missing = match source_outcome {
            Outcome::Truncated { unread_rows, .. } => unread_rows,
            _ => 0,
        };
        for source in sources {
            reader.vtable(
                source.pointer,
                VTABLE_RVA,
                "movement division vtable mismatch",
            )?;
            if presentation_field::<u64>(memory, source.pointer, cunit::HANDLE)? != source.division
                || source.division == 0
            {
                return Err("movement division identity changed");
            }
            let at = source.pointer;
            let route = Array::read(
                memory,
                at.checked_add(cunit::ORDINARY_ROUTE_DATA)
                    .ok_or("route overflow")?,
                4,
            )
            .ok_or("ordinary route invalid")?;
            let alternate = Array::read(
                memory,
                at.checked_add(cunit::ALTERNATE_ROUTE_DATA)
                    .ok_or("route overflow")?,
                4,
            )
            .ok_or("alternate route invalid")?;
            let path_len = route.count as usize;
            let alternate_len = alternate.count as usize;
            required_paths = required_paths
                .checked_add(path_len + alternate_len)
                .ok_or("route arena overflow")?;
            if len == output.len()
                || paths.len().saturating_sub(paths_len) < path_len + alternate_len
            {
                missing = missing
                    .checked_add(1)
                    .ok_or("movement missing count overflow")?;
                continue;
            }
            let numerator = presentation_field::<i64>(memory, at, MOVEMENT_NUMERATOR)?;
            let path_start = paths_len;
            let alternate_start = paths_len + path_len;
            for (array, start, count) in [
                (route, path_start, path_len),
                (alternate, alternate_start, alternate_len),
            ] {
                crate::cpdx_array::copy_u32_prefix(
                    memory,
                    array,
                    &mut paths[start..start + count],
                    "movement route unreadable",
                )?;
            }
            if path_len != 0 && paths[path_start] == 0 {
                return Err("nonempty movement route has null next province");
            }
            let combat = presentation_array(memory, at, cunit::COMBAT_REFERENCE_DATA, 8)?;
            let mut combat_role = 0;
            let mut combat_province = 0;
            if combat.count != 0 {
                use crate::{cland_combat as combat_layout, cland_combatant as side_layout};
                let p = read::<u64>(memory, combat.data)
                    .filter(|&p| p != 0)
                    .ok_or("first combat unreadable or null")? as usize;
                let vtable = read::<u64>(memory, p).ok_or("first combat identity unreadable")?;
                if ![
                    combat_layout::VTABLE_RVA,
                    crate::cland_border_war_combat::VTABLE_RVA,
                ]
                .iter()
                .any(|&rva| base.checked_add(rva).map(|p| p as u64) == Some(vtable))
                {
                    return Err("division first combat identity mismatch");
                }
                let side = presentation_field::<u64>(memory, p, combat_layout::ATTACKER)? as usize;
                let vtable = read::<u64>(memory, side).ok_or("first combat side unreadable")?;
                if ![
                    side_layout::VTABLE_RVA,
                    crate::cland_border_war_combatant::VTABLE_RVA,
                ]
                .iter()
                .any(|&rva| base.checked_add(rva).map(|p| p as u64) == Some(vtable))
                    || presentation_field::<u64>(memory, side, side_layout::COMBAT)? != p as u64
                {
                    return Err("first combat side identity mismatch");
                }
                combat_role = if side_layout::contains_country(
                    memory,
                    base,
                    game_state,
                    side,
                    presentation_field::<u32>(memory, at, cunit::LOGICAL_OWNER)?,
                )? {
                    1
                } else {
                    2
                };
                combat_province =
                    province_id(
                        presentation_field::<u64>(memory, p, combat_layout::PROVINCE)? as usize,
                    )?;
            }
            let target_pointer = presentation_field::<u64>(memory, at, MOVEMENT_TARGET)? as usize;
            let target = province_id(target_pointer)?;
            let transfer = presentation_field::<u64>(memory, at, NAVAL_TRANSFER_MEMBERSHIP)? != 0;
            let source_province =
                province_id(presentation_field::<u64>(memory, at, MOVEMENT_SOURCE)? as usize)?;
            let (retreat, strategic_redeployment) = movement_scalar_tail(memory, at)?;
            let entity = presentation_field::<u64>(memory, at, cunit::MAP_ENTITY)?;
            let strategic_action = strategic_action::capture(
                memory,
                base,
                game_state,
                at,
                route.count,
                target_pointer,
                retreat,
                strategic_redeployment,
                None,
            );
            output[len] = MovementRow {
                combat_references: None,
                combat_references_failure: None,
                selected_exiled: None,
                selected_commander: None,
                selected_retreat: [None; 2],
                selected_retreat_failures: [None; 2],
                division: source.division,
                source: source_province,
                target,
                numerator,
                retreat,
                strategic_redeployment,
                strategic_action,
                naval_transfer: u8::from(transfer),
                combat_count: combat.count,
                combat_role,
                combat_province,
                map_entity: u8::from(entity != 0),
                path_start,
                path_len,
                path_total: route.count,
                alternate_start,
                alternate_len,
                alternate_total: alternate.count,
                withdrawal: None,
                movement_priority: None,
            };
            paths_len += path_len + alternate_len;
            len += 1;
        }
        Ok(if missing == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: missing,
            }
        })
    })();
    let outcome = result.unwrap_or_else(|reason| Outcome::Unavailable { reason });
    // Keep this test-only reference's optional suffix behind the same complete
    // movement-table prefix as capture_movement_inputs. A later first-combat
    // failure must not add style reads to already withheld prefix rows.
    let mut index = 0;
    for source in sources {
        let Some(row) = output[..outcome.len()].get_mut(index) else {
            break;
        };
        if row.division != source.division {
            continue;
        }
        row.withdrawal = cunit::read_withdrawal(memory, source.pointer);
        row.movement_priority = source
            .pointer
            .checked_add(cunit::MOVEMENT_PRIORITY)
            .and_then(|at| read(memory, at));
        index += 1;
    }
    MovementWalk {
        outcome,
        paths_len,
        required_paths,
    }
}

#[cfg(test)]
mod capacity_tests {
    use super::*;
    #[test]
    fn route_byte_demand_excludes_path_allocator_spare_capacity() {
        let mut packet = MovementPacket::new(1, 1);
        packet.paths.reserve(30_000);
        for _ in 0..20 {
            packet.reserve(1, 5_001).unwrap();
            assert_eq!(packet.route_bytes.capacity(), 5_001 * size_of::<u32>());
        }
    }
}
