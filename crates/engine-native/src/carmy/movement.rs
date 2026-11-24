use super::*;

pub struct MovementPacket {
    pub(super) units: Vec<MovementRow>,
    pub(super) paths: Vec<u32>,
    pub(super) combats: MovementCombats,
    combat_references: Vec<cunit::MapCombatReference>,
    combat_identities: cunit::MapCombatIdentityMemo,
    sample_hour: Option<i32>,
    required_combat_references: usize,
    route_bytes: Vec<u8>,
    defer_routes: bool,
    raw_paths_len: usize,
    raw_paths_prefix: usize,
    pub(super) result: MovementWalk,
}
impl MovementPacket {
    pub fn try_new(unit_capacity: usize, path_capacity: usize) -> std::io::Result<Self> {
        let paths = engine_allocation::vec(path_capacity)?;
        let bytes = Self::route_byte_capacity(path_capacity)?;
        Ok(Self {
            units: engine_allocation::vec(unit_capacity)?,
            paths,
            combats: MovementCombats::try_new(unit_capacity)?,
            combat_references: Vec::new(),
            combat_identities: cunit::MapCombatIdentityMemo::default(),
            sample_hour: None,
            required_combat_references: 0,
            route_bytes: engine_allocation::vec(bytes)?,
            defer_routes: false,
            raw_paths_len: 0,
            raw_paths_prefix: 0,
            result: Self::vacant().result,
        })
    }

    #[cfg(test)]
    pub fn new(unit_capacity: usize, path_capacity: usize) -> Self {
        Self::try_new(unit_capacity, path_capacity).unwrap()
    }

    pub fn vacant() -> Self {
        Self {
            units: Vec::new(),
            paths: Vec::new(),
            combats: MovementCombats::default(),
            combat_references: Vec::new(),
            combat_identities: cunit::MapCombatIdentityMemo::default(),
            sample_hour: None,
            required_combat_references: 0,
            route_bytes: Vec::new(),
            defer_routes: false,
            raw_paths_len: 0,
            raw_paths_prefix: 0,
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
            .ok_or_else(engine_allocation::oom)
    }

    pub fn defer_route_decode(&mut self) {
        self.defer_routes = true;
    }

    pub fn clear(&mut self) {
        self.units.clear();
        self.paths.clear();
        self.combats.clear();
        self.combat_references.clear();
        self.combat_identities.clear();
        self.sample_hour = None;
        self.required_combat_references = 0;
        self.raw_paths_len = 0;
        self.raw_paths_prefix = 0;
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
        engine_allocation::reserve_exact(&mut self.units, units)?;
        engine_allocation::reserve_exact(&mut self.paths, paths)?;
        self.combats.reserve(unit_capacity)?;
        let bytes = Self::route_byte_capacity(path_capacity)?;
        let additional = bytes.saturating_sub(self.route_bytes.len());
        engine_allocation::reserve_exact(&mut self.route_bytes, additional)
    }
    pub fn allocated_bytes(&self) -> usize {
        self.units.capacity() * std::mem::size_of::<MovementRow>()
            + self.paths.capacity() * std::mem::size_of::<u32>()
            + self.combats.allocated_bytes()
            + self.combat_references.capacity() * size_of::<cunit::MapCombatReference>()
            + self.combat_identities.allocated_bytes()
            + self.route_bytes.capacity()
    }
    pub fn reserve_combat_references(&mut self, capacity: usize) -> std::io::Result<()> {
        let additional = capacity.saturating_sub(self.combat_references.len());
        engine_allocation::reserve_exact(&mut self.combat_references, additional)?;
        self.combat_identities.reserve(capacity)
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
    pub fn outcome(&self) -> MovementWalk {
        self.result
    }

    fn capture_raw_routes(
        &mut self,
        memory: &impl Memory,
        route: Array,
        alternate: Array,
        start: usize,
    ) -> Result<(), &'static str> {
        let first_len = route.count as usize;
        let len = first_len + alternate.count as usize;
        self.raw_paths_len = start + len;
        let bytes = self
            .raw_paths_len
            .checked_mul(4)
            .ok_or("movement route byte count overflow")?;
        if bytes > self.route_bytes.capacity() {
            return Err("movement route staging capacity exhausted");
        }
        self.route_bytes.resize(bytes, 0);
        if len != 0 {
            let (first, second) = self.route_bytes[start * 4..].split_at_mut(first_len * 4);
            let copied = memory
                .copy_scattered_prefetched([route.data, alternate.data], &mut [first, second]);
            if copied != 0 {
                self.raw_paths_prefix = start + first_len;
            }
            if copied != 2 {
                return Err("movement route unreadable");
            }
            self.raw_paths_prefix = self.raw_paths_len;
        }
        if first_len != 0 && self.route_bytes[start * 4..start * 4 + 4] == [0; 4] {
            return Err("nonempty movement route has null next province");
        }
        Ok(())
    }
    pub fn base_speed_refusals(&self) -> usize {
        self.units[..self.result.outcome.len()]
            .iter()
            .filter(|row| (row.path_total as i32) > 0 && row.base_speed.is_none())
            .count()
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
        for (slot, word) in output.iter_mut().zip(bytes.chunks_exact(4)) {
            *slot = u32::from_le_bytes(word.try_into().unwrap());
        }
    }
    Ok(())
}

const MOVEMENT_AHEAD: usize = 4;

fn prefetch_movement_seed(memory: &impl Memory, seed: &DivisionSeed) {
    for header in [seed.ordinary_route, seed.alternate_route] {
        if let Some(route) = header.and_then(|header| Array::from_header(header, 4))
            && route.count != 0
        {
            memory.prefetch(route.data, route.count as usize * 4);
        }
    }
    if let Some(combat) = seed.combat.and_then(|header| Array::from_header(header, 8))
        && combat.count != 0
    {
        memory.prefetch(combat.data, combat.count as usize * 8);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn capture_movement_inputs(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    seeds: &[DivisionSeed],
    source_outcome: Outcome,
    geometry: &mut crate::cprovince::GeometryPacket,
    packet: &mut MovementPacket,
) -> MovementWalk {
    capture_movement_inputs_with_combat_references(
        memory,
        base,
        game_state,
        seeds,
        source_outcome,
        geometry,
        packet,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn capture_movement_inputs_with_combat_references(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    seeds: &[DivisionSeed],
    source_outcome: Outcome,
    geometry: &mut crate::cprovince::GeometryPacket,
    packet: &mut MovementPacket,
    sample_hour: Option<i32>,
) -> MovementWalk {
    capture_movement_inputs_with_extra(
        memory,
        base,
        game_state,
        seeds,
        source_outcome,
        geometry,
        packet,
        sample_hour,
        |_, _| {},
    )
}

#[allow(clippy::too_many_arguments)]
pub fn capture_movement_inputs_with_extra(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    seeds: &[DivisionSeed],
    source_outcome: Outcome,
    geometry: &mut crate::cprovince::GeometryPacket,
    packet: &mut MovementPacket,
    sample_hour: Option<i32>,
    mut observe: impl FnMut(&DivisionSeed, &mut MovementRow),
) -> MovementWalk {
    packet.clear();
    packet.sample_hour = sample_hour;
    let mut paths_len = 0;
    let mut required_paths = 0usize;
    let result = (|| {
        if let Outcome::Unavailable { reason } = source_outcome {
            return Err(reason);
        }
        if source_outcome.len() != seeds.len() {
            return Err("division movement source length invalid");
        }
        packet
            .reserve(seeds.len(), 0)
            .map_err(|_| "movement seed allocation failed")?;
        for (offset, stride) in [(cgame_state::COUNTRIES, 8), (cgame_state::COUNTRY_LINKS, 4)] {
            game_state
                .checked_add(offset)
                .and_then(|at| Array::read(memory, at, stride))
                .ok_or("division movement root invalid")?;
        }
        geometry
            .ready()
            .map_err(|_| "division movement root invalid")?;
        for seed in seeds.iter().take(MOVEMENT_AHEAD) {
            prefetch_movement_seed(memory, seed);
        }
        let mut len = 0usize;
        let missing = match source_outcome {
            Outcome::Truncated { unread_rows, .. } => unread_rows,
            _ => 0,
        };
        let mut province_batch = crate::cprovince::IdentityBatch::default();
        for (ordinal, seed) in seeds.iter().enumerate() {
            if ordinal.is_multiple_of(32) {
                let mut pointers = [0_usize; 64];
                let mut count = 0;
                for pending in &seeds[ordinal..(ordinal + 32).min(seeds.len())] {
                    for pointer in [pending.movement_target, pending.movement_source]
                        .into_iter()
                        .flatten()
                    {
                        if pointer != 0 {
                            pointers[count] = pointer;
                            count += 1;
                        }
                    }
                }
                geometry.stage_province_ids(memory, &pointers[..count], &mut province_batch);
            }
            if let Some(pending) = seeds.get(ordinal + MOVEMENT_AHEAD) {
                prefetch_movement_seed(memory, pending);
            }
            let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
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
            let path_len = route.count as usize;
            let alternate_len = alternate.count as usize;
            let needed = path_len + alternate_len;
            required_paths = required_paths
                .checked_add(needed)
                .ok_or("route arena overflow")?;
            packet
                .reserve(seeds.len(), required_paths)
                .map_err(|_| "movement route allocation failed")?;
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
            let numerator = movement_seed_value(seed.movement_numerator)?;
            let path_start = paths_len;
            let alternate_start = paths_len + path_len;
            if packet.defer_routes {
                packet.capture_raw_routes(memory, route, alternate, paths_len)?;
            } else {
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
            let target =
                geometry.acquire_province_id_batched(memory, target_pointer, &province_batch)?;
            let transfer = movement_seed_value(seed.naval_transfer_membership)? != 0;
            let source_pointer = movement_seed_value(seed.movement_source)?;
            let source =
                geometry.acquire_province_id_batched(memory, source_pointer, &province_batch)?;
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
                    match cunit::read_unit_combat_references(
                        memory,
                        base,
                        seed.pointer,
                        seed.combat.expect("validated same source header"),
                        first_combat,
                        &mut packet.combat_references,
                        start,
                        &mut packet.combat_identities,
                    ) {
                        Ok(count) => {
                            packet.combat_references.truncate(start + count);
                            combat_references = Some((start, count));
                        }
                        Err(reason) => {
                            packet.combat_references.truncate(start);
                            combat_references_failure = Some(reason);
                        }
                    }
                } else {
                    combat_references_failure = Some("unit combat-reference demand overflow");
                }
            }
            packet.units.push(MovementRow {
                combat_references,
                combat_references_failure,
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
                base_speed: None,
                fuel: None,
                transport_speed: None,
                queued_actions: None,
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
    let mut speeds = SpeedBatch::new();
    let mut index = 0;
    for (ordinal, seed) in seeds.iter().enumerate() {
        let Some(row) = packet.units[..outcome.len()].get(index) else {
            break;
        };
        if row.division != seed.division {
            continue;
        }
        if index.is_multiple_of(64) {
            speeds.capture(
                memory,
                &seeds[ordinal..],
                &packet.units[index..(index + 64).min(outcome.len())],
            );
        }
        let row = &mut packet.units[index];
        row.withdrawal = seed.withdrawal;
        row.movement_priority = seed.movement_priority;
        row.fuel = seed.fuel;
        row.queued_actions = seed.queued_actions;
        if (row.path_total as i32) > 0 {
            row.base_speed = speeds.read(memory, base, seed).ok();
        }
        observe(seed, row);
        index += 1;
    }
    packet.result = MovementWalk {
        outcome,
        paths_len,
        required_paths,
    };
    packet.result
}

const SPEED_FIELDS: [engine_native_memory::memory::RecordField; 2] = [
    engine_native_memory::memory::RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    engine_native_memory::memory::RecordField {
        source_offset: engine_native_parts::csub_unit_definition::MAXIMUM_SPEED,
        destination_offset: 8,
        len: 8,
    },
];
const SPEED_LAYOUT: engine_native_memory::memory::RecordLayout<'static> =
    engine_native_memory::memory::RecordLayout::new(&SPEED_FIELDS, 16).expect("valid movement speed layout");

struct SpeedBatch {
    pointers: [usize; 64],
    bytes: [u8; 64 * 16],
    complete: [bool; 64],
    len: usize,
}

impl SpeedBatch {
    fn new() -> Self {
        Self {
            pointers: [0; 64],
            bytes: [0; 64 * 16],
            complete: [false; 64],
            len: 0,
        }
    }

    fn capture(&mut self, memory: &impl Memory, seeds: &[DivisionSeed], rows: &[MovementRow]) {
        self.len = 0;
        let mut row = 0;
        for seed in seeds {
            if row == rows.len() {
                break;
            }
            if seed.division != rows[row].division {
                continue;
            }
            if rows[row].path_total as i32 > 0
                && let Some(pointer) = seed.stat_block.filter(|&at| at != 0)
                && !self.pointers[..self.len].contains(&pointer)
            {
                self.pointers[self.len] = pointer;
                self.len += 1;
            }
            row += 1;
        }
        engine_native_memory::memory::gather_records_once(
            memory,
            &self.pointers[..self.len],
            &SPEED_LAYOUT,
            &mut self.bytes[..self.len * 16],
            &mut self.complete,
            engine_native_memory::batch_fallback::Batch::DivisionMovementSpeed,
        );
    }

    fn read(
        &self,
        memory: &impl Memory,
        base: usize,
        seed: &DivisionSeed,
    ) -> Result<i64, &'static str> {
        let pointer = movement_seed_value(seed.stat_block)?;
        if let Some(row) = self.pointers[..self.len]
            .iter()
            .position(|&at| at == pointer)
            && self.complete[row]
        {
            let bytes = &self.bytes[row * 16..][..16];
            let expected = engine_sites::at(base, engine_native_parts::csub_unit_definition::VTABLE_RVA)
                .ok_or("module address overflow")? as u64;
            if u64::from_le_bytes(bytes[..8].try_into().unwrap()) != expected {
                return Err("division stat block identity mismatch");
            }
            return Ok(i64::from_le_bytes(bytes[8..].try_into().unwrap()));
        }
        movement_base_speed(memory, base, seed)
    }
}

fn movement_base_speed(
    memory: &impl Memory,
    base: usize,
    seed: &DivisionSeed,
) -> Result<i64, &'static str> {
    let stat = movement_seed_value(seed.stat_block)?;
    if stat == 0 {
        return Err("division stat block null");
    }
    let speed_at = stat
        .checked_add(engine_native_parts::csub_unit_definition::MAXIMUM_SPEED)
        .ok_or("division stat block address overflow")?;
    let expected = engine_sites::at(base, engine_native_parts::csub_unit_definition::VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let mut vtable = [0; 8];
    let mut speed = [0; 8];
    if memory.copy_scattered([stat, speed_at], &mut [&mut vtable, &mut speed]) != 2 {
        return Err("division stat block unreadable");
    }
    if u64::from_le_bytes(vtable) != expected {
        return Err("division stat block identity mismatch");
    }
    Ok(i64::from_le_bytes(speed))
}

pub fn project_movement(
    packet: &MovementPacket,
    output: &mut [MovementRow],
    paths: &mut [u32],
) -> MovementWalk {
    let path_len = if packet.defer_routes {
        packet.raw_paths_len
    } else {
        packet.paths.len()
    };
    if output.len() < packet.units.len() || paths.len() < path_len {
        return MovementWalk {
            outcome: Outcome::Unavailable {
                reason: "division movement projection capacity exhausted",
            },
            ..packet.result
        };
    }
    output[..packet.units.len()].copy_from_slice(&packet.units);
    if packet.defer_routes {
        paths[..path_len].fill(0);
        for (output, bytes) in paths[..packet.raw_paths_prefix]
            .iter_mut()
            .zip(packet.route_bytes.chunks_exact(4))
        {
            *output = u32::from_le_bytes(bytes.try_into().unwrap());
        }
    } else {
        paths[..packet.paths.len()].copy_from_slice(&packet.paths);
    }
    packet.result
}

#[cfg(test)]
pub fn walk_movement(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    sources: &[crate::cequipment_variant::pool::DivisionSource],
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

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub fn walk_movement_with_inputs(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    sources: &[crate::cequipment_variant::pool::DivisionSource],
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
    sources: &[crate::cequipment_variant::pool::DivisionSource],
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
                engine_native_memory::cpdx_array::copy_u32_prefix(
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
                if engine_sites::which(
                    base,
                    vtable as usize,
                    &[
                        combat_layout::VTABLE_RVA,
                        engine_native_parts::cland_border_war_combat::VTABLE_RVA,
                    ],
                )
                .is_none()
                {
                    return Err("division first combat identity mismatch");
                }
                let side = presentation_field::<u64>(memory, p, combat_layout::ATTACKER)? as usize;
                let vtable = read::<u64>(memory, side).ok_or("first combat side unreadable")?;
                if engine_sites::which(
                    base,
                    vtable as usize,
                    &[
                        side_layout::VTABLE_RVA,
                        engine_native_parts::cland_border_war_combatant::VTABLE_RVA,
                    ],
                )
                .is_none()
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
                base_speed: None,
                fuel: None,
                transport_speed: None,
                queued_actions: None,
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
        row.fuel = source
            .pointer
            .checked_add(FUEL_CANDIDATE)
            .and_then(|at| read(memory, at));
        row.queued_actions = source
            .pointer
            .checked_add(cunit::QUEUED_ACTION_COUNT)
            .and_then(|at| read(memory, at));
        if (row.path_total as i32) > 0 {
            let seed = DivisionSeed {
                stat_block: source
                    .pointer
                    .checked_add(cunit::STAT_BLOCK)
                    .and_then(|at| read::<u64>(memory, at))
                    .map(|pointer| pointer as usize),
                ..Default::default()
            };
            row.base_speed = movement_base_speed(memory, base, &seed).ok();
        }
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
    fn deferred_routes_match_serial_bytes_and_completed_prefixes_after_partial_faults() {
        use std::cell::RefCell;
        struct Routes {
            bytes: Vec<u8>,
            fault: Option<usize>,
            reads: RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for Routes {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                self.reads.borrow_mut().push((at, output.len()));
                let Some(bytes) = self.bytes.get(at..at + output.len()) else {
                    return false;
                };
                output.copy_from_slice(bytes);
                self.fault != Some(at)
            }
        }
        for (ordinary, alternate) in [(0, 0), (0, 3), (1, 0), (3, 2), (5_000, 7)] {
            for fault in [None, Some(4), Some((ordinary + 1) * 4)] {
                let len = ordinary + alternate;
                let memory = Routes {
                    bytes: (0..=len as u32)
                        .flat_map(|value| value.to_le_bytes())
                        .collect(),
                    fault,
                    reads: RefCell::default(),
                };
                let route = Array {
                    data: 4,
                    count: ordinary as u32,
                };
                let other = Array {
                    data: (ordinary + 1) * 4,
                    count: alternate as u32,
                };
                let mut expected = vec![0; len];
                let (first, second) = expected.split_at_mut(ordinary);
                let serial = movement_route_prefixes(
                    &memory,
                    route,
                    other,
                    &mut vec![0; len * 4],
                    first,
                    second,
                );
                let reads = memory.reads.take();
                let mut packet = MovementPacket::new(0, len);
                packet.defer_route_decode();
                let deferred = packet.capture_raw_routes(&memory, route, other, 0);
                assert_eq!(deferred, serial);
                assert_eq!(memory.reads.take(), reads);
                drop(memory);
                let mut actual = vec![u32::MAX; len];
                project_movement(&packet, &mut [], &mut actual);
                assert_eq!(actual, expected, "{ordinary}/{alternate}, {fault:?}");
            }
        }
    }
}
