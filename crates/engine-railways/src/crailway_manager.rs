
use engine_sites::Rva;
use crate::{
    Memory, cgame_state,
    cpdx_array::Array,
    cprovince, cprovince_railway_info as info,
    memory::{RecordField, RecordLayout, RecordSources},
    rail_neighbor as edge, read, read_slice,
    walk::{Buffer, Outcome},
};

pub mod fresh_inputs;
pub mod selected_inputs;

pub const VTABLE_RVA: Rva = Rva(0x029e_f450);

pub const PROVINCE_INFO_TABLE: usize = 0x08;
#[cfg(test)]
pub const PROVINCE_INFO_COUNT: usize = 0x14;
pub const COOLDOWN_PROVINCES: usize = 0x20;
pub const COOLDOWN_PROVINCE_COUNT: usize = 0x2c;
pub const PROVINCE_INFO_STRIDE: usize = 8;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00f0_51e0);
pub const DESTRUCTOR_RVA: Rva = Rva(0x00f0_54c0);
pub const LOAD_TOKEN_RVA: Rva = Rva(0x00f0_8180);
pub const SET_PROVINCE_ON_COOLDOWN_RVA: Rva = Rva(0x00f0_7810);
pub const UPDATE_COOLDOWNS_RVA: Rva = Rva(0x00f0_6fc0);

pub fn movement_speed(
    memory: &impl Memory,
    base: usize,
    state: usize,
    from: usize,
    to: usize,
) -> Result<i64, &'static str> {
    let provinces = eta_array(memory, state, cgame_state::PROVINCES, 8)?;
    let from_id = eta_province(memory, base, provinces, from)?;
    let to_id = eta_province(memory, base, provinces, to)?;
    let infrastructure = eta_infrastructure(memory, base, state, to)?;
    let road_base = eta_global::<i64>(memory, base, Rva(0x033e_0928))?;
    let road_scale = eta_global::<i64>(memory, base, Rva(0x033e_09b8))?;
    let road = road_base.wrapping_add(infrastructure.wrapping_mul(road_scale) / 100_000);
    let percentage =
        railway_percentage_from_ids(memory, base, state, provinces, (from, from_id), to_id)?;
    let rail_base = eta_global::<i64>(memory, base, Rva(0x033e_0a48))?;
    let rail_scale = eta_global::<i64>(memory, base, Rva(0x033e_0ae0))?;
    let rail = rail_base.wrapping_add(percentage.wrapping_mul(rail_scale) / 100_000);
    Ok(road.max(rail))
}

pub fn railway_percentage(
    memory: &impl Memory,
    base: usize,
    state: usize,
    from: usize,
    to: usize,
) -> Result<i64, &'static str> {
    let provinces = eta_array(memory, state, cgame_state::PROVINCES, 8)?;
    let from_id = eta_province(memory, base, provinces, from)?;
    let to_id = eta_province(memory, base, provinces, to)?;
    railway_percentage_from_ids(memory, base, state, provinces, (from, from_id), to_id)
}

fn railway_percentage_from_ids(
    memory: &impl Memory,
    base: usize,
    state: usize,
    provinces: Array,
    from: (usize, u32),
    to_id: u32,
) -> Result<i64, &'static str> {
    let manager = eta_pointer(memory, state, cgame_state::RAIL_MANAGER)?;
    eta_vtable(memory, base, manager, VTABLE_RVA)?;
    let slots = eta_array(memory, manager, PROVINCE_INFO_TABLE, 8)?;
    if slots.count != provinces.count {
        return Err("ETA railway province extent mismatch");
    }
    let slot = slots.element(from.1, 8).ok_or("ETA railway slot absent")?;
    let node = eta_read::<u64>(memory, slot, 0)? as usize;
    let mut weighted_level = 0_i64;
    if node != 0 {
        eta_vtable(memory, base, node, info::VTABLE_RVA)?;
        let definition = eta_pointer(memory, from.0, cprovince::DEFINITION)?;
        if eta_read::<u64>(memory, node, info::PROVINCE_RECORD)? != definition as u64
            || eta_read::<u64>(memory, node, info::BUILDING_STATUS)?
                != eta_address(from.0, cprovince::BUILDING_STATUS)? as u64
            || eta_read::<u32>(memory, definition, crate::province_definition::ID)? != from.1
        {
            return Err("ETA railway province backlink mismatch");
        }
        if eta_read::<i32>(memory, node, info::COOLDOWN_TICKS)? <= 0 {
            let pairs = eta_array(memory, node, info::NEIGHBOR_PAIRS, edge::STRIDE)?;
            for index in 0..pairs.count {
                let at = pairs
                    .element(index, edge::STRIDE)
                    .ok_or("ETA rail edge absent")?;
                let pair = eta_read::<[u8; 8]>(memory, at, 0)?;
                if u32::from_le_bytes(pair[..4].try_into().unwrap()) == to_id {
                    let level = i32::from_le_bytes(pair[4..].try_into().unwrap());
                    let building = eta_read::<u64>(memory, node, info::BUILDING)? as usize;
                    let efficiency = if building == 0 {
                        0
                    } else {
                        eta_vtable(memory, base, building, crate::cbuilding::VTABLE_RVA)?;
                        if eta_read::<u64>(memory, building, crate::cbuilding::BUILDING_STATUS)?
                            != eta_address(from.0, cprovince::BUILDING_STATUS)? as u64
                        {
                            return Err("ETA rail building backlink mismatch");
                        }
                        let level = eta_read::<i16>(memory, building, crate::cbuilding::LEVEL)?;
                        let healthy =
                            eta_read::<i16>(memory, building, crate::cbuilding::HEALTHY_LEVEL)?;
                        let partial = if level != 0 && level != healthy {
                            eta_read::<i64>(memory, building, crate::cbuilding::PARTIAL_HEALTH)?
                        } else {
                            0
                        };
                        eta_building_efficiency(level, healthy, partial)?
                    };
                    weighted_level = i64::from(level).wrapping_mul(efficiency);
                    break;
                }
            }
        }
    }
    let denominator = i64::from(eta_global::<i32>(memory, base, Rva(0x033d_fbe8))?) * 100_000;
    if denominator == 0 {
        Ok(0)
    } else {
        eta_ratio(weighted_level, denominator)
    }
}

fn eta_address(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("ETA railway address overflow")
}

fn eta_read<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, eta_address(pointer, offset)?).ok_or("ETA railway input unreadable")
}

fn eta_pointer(memory: &impl Memory, pointer: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(eta_read::<u64>(memory, pointer, offset)?)
        .ok()
        .filter(|&p| p != 0)
        .ok_or("ETA railway pointer null or invalid")
}

fn eta_key(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("ETA railway address overflow")
}

fn eta_global<T: crate::memory::Raw>(
    memory: &impl Memory,
    base: usize,
    key: Rva,
) -> Result<T, &'static str> {
    eta_read(memory, eta_key(base, key)?, 0)
}

fn eta_global_pointer(memory: &impl Memory, base: usize, key: Rva) -> Result<usize, &'static str> {
    eta_pointer(memory, eta_key(base, key)?, 0)
}

fn eta_array(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    Array::read(memory, eta_address(pointer, offset)?, stride).ok_or("ETA railway array invalid")
}

fn eta_vtable(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    key: Rva,
) -> Result<(), &'static str> {
    if pointer == 0 || eta_read::<u64>(memory, pointer, 0)? != eta_key(base, key)? as u64 {
        return Err("ETA railway source vtable mismatch");
    }
    Ok(())
}

fn eta_province(
    memory: &impl Memory,
    base: usize,
    provinces: Array,
    pointer: usize,
) -> Result<u32, &'static str> {
    eta_vtable(memory, base, pointer, cprovince::VTABLE_RVA)?;
    let id = eta_read::<u32>(memory, pointer, cprovince::ID)?;
    if id == 0
        || provinces
            .element(id, 8)
            .and_then(|at| read::<u64>(memory, at))
            != Some(pointer as u64)
    {
        return Err("ETA railway province identity mismatch");
    }
    Ok(id)
}

fn eta_exceptional(value: i64) -> bool {
    (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd
}

fn eta_ratio(numerator: i64, denominator: i64) -> Result<i64, &'static str> {
    if denominator == 0 || eta_exceptional(numerator) {
        return Ok(0xffff_ffff);
    }
    numerator
        .wrapping_mul(100_000)
        .checked_div(denominator)
        .ok_or("ETA railway native division overflow")
}

fn eta_building_efficiency(level: i16, healthy: i16, partial: i64) -> Result<i64, &'static str> {
    if level == 0 {
        return Ok(0);
    }
    if level == healthy {
        return Ok(100_000);
    }
    let partial = if eta_exceptional(partial) {
        0xffff_ffff
    } else {
        partial.wrapping_mul(100_000) / 10_000_000
    };
    eta_ratio(
        partial.wrapping_add(i64::from(healthy) * 100_000),
        i64::from(level) * 100_000,
    )
}

fn eta_infrastructure(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    province: usize,
) -> Result<i64, &'static str> {
    let state = eta_read::<u64>(memory, province, cprovince::STATE)? as usize;
    if state == 0 {
        return Ok(0);
    }
    eta_state_infrastructure(memory, base, game_state, state)
}

fn eta_state_infrastructure(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    state: usize,
) -> Result<i64, &'static str> {
    use crate::{
        cbuilding as building, cbuilding_database as database, cbuilding_status as status,
        cbuilding_template as template, cstate,
    };
    eta_vtable(memory, base, state, cstate::VTABLE_RVA)?;
    let states = eta_array(memory, game_state, cgame_state::STATES, 8)?;
    let state_id = eta_read::<u32>(memory, state, cstate::ID)?;
    if state_id == 0
        || states
            .element(state_id, 8)
            .and_then(|at| read::<u64>(memory, at))
            != Some(state as u64)
    {
        return Err("ETA infrastructure state identity mismatch");
    }
    let database = eta_global_pointer(memory, base, database::SINGLETON_RVA)?;
    let definition = eta_pointer(memory, database, 0x3b0)?;
    if eta_read::<u8>(memory, definition, template::LOOKUP_ENABLED_CANDIDATE)? == 0 {
        return Ok(0);
    }
    let status = eta_address(state, cstate::BUILDING_STATUS)?;
    eta_vtable(memory, base, status, status::VTABLE_RVA)?;
    let indices = eta_array(memory, status, status::BUILDING_INDICES, 4)?;
    if indices.count == 0 {
        return Ok(0);
    }
    let ordinal = eta_read::<u32>(memory, definition, template::DATABASE_ORDINAL)?;
    let definitions = eta_array(memory, database, database::TEMPLATES, 8)?;
    if definitions
        .element(ordinal, 8)
        .and_then(|at| read::<u64>(memory, at))
        != Some(definition as u64)
    {
        return Err("ETA infrastructure template identity mismatch");
    }
    let index = indices
        .element(ordinal, 4)
        .and_then(|at| read::<i32>(memory, at))
        .ok_or("ETA infrastructure index absent")?;
    if index == -1 {
        return Ok(0);
    }
    let buildings = eta_array(memory, status, status::BUILDINGS, 8)?;
    let slot = buildings
        .element(
            u32::try_from(index).map_err(|_| "ETA infrastructure index negative")?,
            8,
        )
        .ok_or("ETA infrastructure building absent")?;
    let instance = eta_read::<u64>(memory, slot, 0)? as usize;
    if instance == 0 {
        return Ok(0);
    }
    eta_vtable(memory, base, instance, building::VTABLE_RVA)?;
    if eta_read::<u64>(memory, instance, building::BUILDING_STATUS)? != status as u64
        || eta_read::<u64>(memory, instance, building::TEMPLATE)? != definition as u64
        || eta_read::<u32>(memory, instance, building::TYPE_ID)?
            != eta_read::<u32>(memory, definition, template::TYPE_ID)?
    {
        return Err("ETA infrastructure building identity mismatch");
    }
    let healthy = eta_read::<i16>(memory, instance, building::HEALTHY_LEVEL)?;
    let mut maximum = eta_read::<i32>(memory, definition, 0x2bc)?;
    if maximum < 1 {
        maximum = eta_read::<i32>(memory, definition, 0x2b8)?;
        if maximum <= 0 {
            maximum = eta_global::<i32>(memory, base, Rva(0x033e_5274))?;
        }
    }
    eta_ratio(i64::from(healthy) * 100_000, i64::from(maximum) * 100_000)
}

pub fn cooldown_list(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    ids: &mut Vec<u32>,
) -> Result<(), &'static str> {
    ids.clear();
    let manager = game_state
        .checked_add(cgame_state::RAIL_MANAGER)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("rail manager unreadable or null")?;
    let vtable = engine_sites::at(base, VTABLE_RVA)
        .ok_or("module address overflow")?;
    if read::<u64>(memory, manager) != Some(vtable as u64) {
        return Err("rail manager vtable mismatch");
    }
    let list = manager
        .checked_add(COOLDOWN_PROVINCES)
        .and_then(|p| Array::read(memory, p, 4))
        .ok_or("rail cooldown list unreadable or invalid")?;
    crate::allocation::resize_default(ids, list.count as usize)
        .map_err(|_| "rail cooldown list allocation failed")?;
    crate::cpdx_array::copy_u32s(memory, list, ids, "rail cooldown list unreadable")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcomes {
    pub provinces: Outcome,
    pub edges: Outcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildingOutcomes {
    pub rows: Outcomes,
    pub buildings: Outcome,
    pub required: usize,
}

impl Outcomes {
    fn unavailable(reason: &'static str) -> Self {
        Self {
            provinces: Outcome::Unavailable { reason },
            edges: Outcome::Unavailable { reason },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RailLocation {
    pub province: u32,
    pub info: usize,
    pub building_status: usize,
    pub identity: [u8; INFO_BYTES],
    pub provinces: Array,
    pub cooldown: Option<i32>,
    pub pairs: Option<[u8; 16]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RailHeader {
    pub manager: usize,
    pub rail: Array,
    pub provinces: Array,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InfoOutputs<'a> {
    pub cooldown: i32,
    pub edges: &'a [edge::Row],
    pub building: fresh_inputs::RailwayBuildingRow,
    pub levels: &'a [i32],
}

#[derive(Debug, Clone, Copy)]
pub struct HeldBatch<'a> {
    pub cooldowns: &'a [info::Row],
    pub edges: &'a [edge::Row],
    pub buildings: &'a [fresh_inputs::RailwayBuildingRow],
    pub levels: &'a [i32],
}

pub trait RailCarry {
    fn tables(&mut self, _header: &RailHeader) {}
    fn quiet_batch(&mut self, _first: u32, _count: u32) -> Option<HeldBatch<'_>> {
        None
    }
    fn carried_batch(&mut self) {}
    fn plan(&mut self, location: &RailLocation) -> (usize, bool);
    fn held(&self, token: usize) -> Option<InfoOutputs<'_>>;
    fn carried(&mut self, token: usize);
    fn read(&mut self, token: usize, outputs: Option<InfoOutputs<'_>>);
    fn read_batch(&mut self) {}
}

pub struct Fresh;
impl RailCarry for Fresh {
    fn plan(&mut self, _: &RailLocation) -> (usize, bool) {
        (0, false)
    }
    fn held(&self, _: usize) -> Option<InfoOutputs<'_>> {
        None
    }
    fn carried(&mut self, _: usize) {}
    fn read(&mut self, _: usize, _: Option<InfoOutputs<'_>>) {}
}

fn whole(row: &fresh_inputs::RailwayBuildingRow) -> bool {
    row.building.is_ok()
        && row.selection.is_some_and(|selection| {
            selection.levels.is_ok()
                && selection.pair_count.is_ok()
                && selection.building_level.is_ok()
        })
}

fn rebased(
    mut row: fresh_inputs::RailwayBuildingRow,
    first: usize,
) -> fresh_inputs::RailwayBuildingRow {
    if let Some(selection) = row.selection.as_mut()
        && let Ok(span) = selection.levels.as_mut()
    {
        span.first = first;
    }
    row
}

fn append_edges(
    output: &mut Buffer<'_, edge::Row>,
    progress: &mut Progress,
    rows: &[edge::Row],
) -> Result<(), &'static str> {
    let required = progress
        .len
        .checked_add(rows.len())
        .ok_or("rail edge demand overflow")?;
    output.ensure(required)?;
    let take = rows.len().min(output.len() - progress.len);
    progress.omit((rows.len() - take) as u32);
    output[progress.len..progress.len + take].copy_from_slice(&rows[..take]);
    progress.len += take;
    Ok(())
}

fn fits(len: usize, more: usize, ensure: impl FnOnce(usize) -> Result<bool, &'static str>) -> bool {
    len.checked_add(more)
        .is_some_and(|required| ensure(required) == Ok(true))
}

#[allow(clippy::too_many_arguments)]
fn publish_held(
    held: HeldBatch<'_>,
    province_output: &mut Buffer<'_, info::Row>,
    province_progress: &mut Progress,
    edge_output: &mut Buffer<'_, edge::Row>,
    edge_progress: &mut Progress,
    building_output: Option<&mut Buffer<'_, fresh_inputs::RailwayBuildingRow>>,
    building_progress: &mut Progress,
    levels: Option<&mut selected_inputs::LevelBuffer<'_>>,
    building_required: &mut usize,
) -> bool {
    let (Some(building_output), Some(levels)) = (building_output, levels) else {
        return false;
    };
    let infos = held.cooldowns.len();
    if held.buildings.len() != infos {
        return false;
    }
    let (Some(level_end), Some(required)) = (
        levels.len.checked_add(held.levels.len()),
        levels.required.checked_add(held.levels.len()),
    ) else {
        return false;
    };
    if !fits(building_progress.len, infos, |n| building_output.ensure(n))
        || !fits(levels.len, held.levels.len(), |n| levels.values.ensure(n))
        || (province_progress.error.is_none()
            && !fits(province_progress.len, infos, |n| province_output.ensure(n)))
        || (edge_progress.error.is_none()
            && !fits(edge_progress.len, held.edges.len(), |n| {
                edge_output.ensure(n)
            }))
    {
        return false;
    }
    if province_progress.error.is_none() {
        province_output[province_progress.len..province_progress.len + infos]
            .copy_from_slice(held.cooldowns);
        province_progress.len += infos;
    }
    if edge_progress.error.is_none() {
        let end = edge_progress.len + held.edges.len();
        edge_output[edge_progress.len..end].copy_from_slice(held.edges);
        edge_progress.len = end;
    }
    let base = levels.len;
    levels.values[base..level_end].copy_from_slice(held.levels);
    levels.len = level_end;
    levels.required = required;
    for row in held.buildings {
        let mut row = *row;
        if let Some(selection) = row.selection.as_mut()
            && let Ok(span) = selection.levels.as_mut()
        {
            span.first += base;
        }
        building_output[building_progress.len] = row;
        building_progress.len += 1;
    }
    *building_required += infos;
    true
}

#[derive(Default)]
struct Progress {
    len: usize,
    unread: u32,
    error: Option<&'static str>,
}

impl Progress {
    fn omit(&mut self, count: u32) {
        match self.unread.checked_add(count) {
            Some(value) => self.unread = value,
            None => self.error = Some("rail omitted row count overflow"),
        }
    }

    fn outcome(self) -> Outcome {
        if let Some(reason) = self.error {
            Outcome::Unavailable { reason }
        } else if self.unread != 0 {
            Outcome::Truncated {
                len: self.len,
                unread_rows: self.unread,
            }
        } else {
            Outcome::Rows { len: self.len }
        }
    }
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    province_output: &mut Vec<info::Row>,
    edge_output: &mut Vec<edge::Row>,
) -> Outcomes {
    walk_inner(
        memory,
        base,
        game_state,
        Buffer::Growing(province_output),
        Buffer::Growing(edge_output),
        None,
        None,
        &mut Fresh,
    )
    .rows
}

#[cfg(test)]
pub fn walk_with_buildings(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    province_output: &mut Vec<info::Row>,
    edge_output: &mut Vec<edge::Row>,
    building_output: &mut Vec<fresh_inputs::RailwayBuildingRow>,
) -> BuildingOutcomes {
    building_output.fill(fresh_inputs::RailwayBuildingRow::default());
    walk_inner(
        memory,
        base,
        game_state,
        Buffer::Growing(province_output),
        Buffer::Growing(edge_output),
        Some(Buffer::Growing(building_output)),
        None,
        &mut Fresh,
    )
}

pub fn walk_with_selection(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    province_output: &mut Vec<info::Row>,
    edge_output: &mut Vec<edge::Row>,
    building_output: &mut Vec<fresh_inputs::RailwayBuildingRow>,
    levels: &mut selected_inputs::Levels<'_>,
) -> BuildingOutcomes {
    walk_with_selection_carried(
        memory,
        base,
        game_state,
        province_output,
        edge_output,
        building_output,
        levels,
        &mut Fresh,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn walk_with_selection_carried(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    province_output: &mut Vec<info::Row>,
    edge_output: &mut Vec<edge::Row>,
    building_output: &mut Vec<fresh_inputs::RailwayBuildingRow>,
    levels: &mut selected_inputs::Levels<'_>,
    carry: &mut impl RailCarry,
) -> BuildingOutcomes {
    building_output.fill(fresh_inputs::RailwayBuildingRow::default());
    let mut buffer = selected_inputs::LevelBuffer {
        values: Buffer::Growing(levels.values),
        len: 0,
        required: 0,
    };
    let outcomes = walk_inner(
        memory,
        base,
        game_state,
        Buffer::Growing(province_output),
        Buffer::Growing(edge_output),
        Some(Buffer::Growing(building_output)),
        Some(&mut buffer),
        carry,
    );
    levels.len = buffer.len;
    levels.required = buffer.required;
    outcomes
}

pub fn read_header(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<RailHeader, &'static str> {
    let manager = game_state
        .checked_add(cgame_state::RAIL_MANAGER)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("rail manager unreadable or null")?;
    let manager_vtable =
        engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    if read::<u64>(memory, manager) != Some(manager_vtable as u64) {
        return Err("rail manager vtable mismatch");
    }
    let rail = manager
        .checked_add(PROVINCE_INFO_TABLE)
        .and_then(|p| Array::read(memory, p, PROVINCE_INFO_STRIDE))
        .ok_or("rail province array unreadable or invalid")?;
    let provinces = game_state
        .checked_add(cgame_state::PROVINCES)
        .and_then(|p| Array::read(memory, p, 8))
        .ok_or("rail source province array unreadable or invalid")?;
    if provinces.count < 2 || rail.count != provinces.count {
        return Err("rail table extent differs from province table");
    }
    Ok(RailHeader {
        manager,
        rail,
        provinces,
    })
}

#[allow(clippy::too_many_arguments)]
fn walk_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut province_output: Buffer<'_, info::Row>,
    mut edge_output: Buffer<'_, edge::Row>,
    mut building_output: Option<Buffer<'_, fresh_inputs::RailwayBuildingRow>>,
    mut levels: Option<&mut selected_inputs::LevelBuffer<'_>>,
    carry: &mut impl RailCarry,
) -> BuildingOutcomes {
    let mut province_progress = Progress::default();
    let mut edge_progress = Progress::default();
    let mut building_progress = Progress::default();
    let mut building_required = 0;
    let result = (|| {
        let RailHeader {
            manager,
            rail,
            provinces,
        } = read_header(memory, base, game_state)?;
        province_output.ensure(rail.count as usize)?;
        if let Some(output) = building_output.as_mut() {
            output.ensure(rail.count as usize)?;
        }
        let info_vtable = engine_sites::at(base, info::VTABLE_RVA)
            .ok_or("module address overflow")?;
        let province_vtable = engine_sites::at(base, cprovince::VTABLE_RVA)
            .ok_or("module address overflow")?;
        let mut slots = [0_u8; PROVINCE_BATCH * PROVINCE_INFO_STRIDE];
        let mut ids = [0_u32; PROVINCE_BATCH];
        let mut infos = [0_usize; PROVINCE_BATCH];
        let mut identity = [0_u8; PROVINCE_BATCH * INFO_BYTES];
        let mut pairs_headers = [0_u8; PROVINCE_BATCH * 16];
        let mut pair_counts = [0_u8; PROVINCE_BATCH * 4];
        let mut cooldowns = [0_u8; PROVINCE_BATCH * 4];
        let mut definition_ids = [0_u8; PROVINCE_BATCH * 4];
        let mut definition_pointers = [0_usize; PROVINCE_BATCH];
        let mut batch = ProvinceBatch::new();
        let mut neighbor_batch = ProvinceBatch::new();
        let mut edge_scratch = EdgeScratch::new();
        let mut building_sources = [fresh_inputs::RailwaySource::default(); PROVINCE_BATCH];
        let carrying = building_output.is_some() && levels.is_some();
        if carrying {
            carry.tables(&RailHeader {
                manager,
                rail,
                provinces,
            });
        }
        let mut plans = [Plan::Unplanned; PROVINCE_BATCH];
        for first in (0..rail.count).step_by(PROVINCE_BATCH) {
            let count = (rail.count - first).min(PROVINCE_BATCH as u32);
            let published = carrying
                && match carry.quiet_batch(first, count) {
                    Some(held) => publish_held(
                        held,
                        &mut province_output,
                        &mut province_progress,
                        &mut edge_output,
                        &mut edge_progress,
                        building_output.as_mut(),
                        &mut building_progress,
                        levels.as_deref_mut(),
                        &mut building_required,
                    ),
                    None => false,
                };
            if published {
                carry.carried_batch();
                continue;
            }
            let address = rail
                .element(first, PROVINCE_INFO_STRIDE)
                .ok_or("rail slot address overflow")?;
            if !read_slice(
                memory,
                address,
                &mut slots[..count as usize * PROVINCE_INFO_STRIDE],
            ) {
                return Err("rail province slots unreadable");
            }
            let mut live = 0;
            let mut overflow = false;
            for (offset, bytes) in slots[..count as usize * PROVINCE_INFO_STRIDE]
                .chunks_exact(PROVINCE_INFO_STRIDE)
                .enumerate()
            {
                let Ok(pointer) = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                else {
                    overflow = true;
                    break;
                };
                if pointer == 0 {
                    continue;
                }
                infos[live] = pointer;
                ids[live] = first + offset as u32;
                live += 1;
            }
            batch.gather(memory, provinces, &ids[..live], &PROVINCE_LAYOUT);
            for (entry, definition) in definition_pointers[..live].iter_mut().enumerate() {
                *definition = usize::try_from(batch.definition_word(entry)).unwrap_or(0);
            }
            let id_copied = memory.copy_records(
                RecordSources::Pointers(&definition_pointers[..live]),
                &DEFINITION_ID_LAYOUT,
                &mut definition_ids[..live * 4],
            );
            let mut combined = [0_u8; PROVINCE_BATCH * COMBINED_INFO_BYTES];
            let combined_copied = memory.copy_records(
                RecordSources::Pointers(&infos[..live]),
                &COMBINED_INFO_LAYOUT,
                &mut combined[..live * COMBINED_INFO_BYTES],
            );
            let (info_copied, counts_copied, pairs_copied, cooldown_copied) =
                if combined_copied == live * COMBINED_INFO_FIELDS.len() {
                    for entry in 0..live {
                        let rec = &combined[entry * COMBINED_INFO_BYTES..(entry + 1) * COMBINED_INFO_BYTES];
                        identity[entry * INFO_BYTES..(entry + 1) * INFO_BYTES]
                            .copy_from_slice(&rec[..INFO_BYTES]);
                        pairs_headers[entry * 16..entry * 16 + 12]
                            .copy_from_slice(&rec[INFO_BYTES..INFO_BYTES + 12]);
                        pair_counts[entry * 4..entry * 4 + 4]
                            .copy_from_slice(&rec[INFO_BYTES + 12..INFO_BYTES + 16]);
                        cooldowns[entry * 4..entry * 4 + 4]
                            .copy_from_slice(&rec[INFO_BYTES + 16..INFO_BYTES + 20]);
                    }
                    (live, live, live, live)
                } else {
                    (
                        memory.copy_records(
                            RecordSources::Pointers(&infos[..live]),
                            &INFO_LAYOUT,
                            &mut identity[..live * INFO_BYTES],
                        ),
                        memory.copy_records(
                            RecordSources::Pointers(&infos[..live]),
                            &PAIR_COUNT_LAYOUT,
                            &mut pair_counts[..live * 4],
                        ),
                        memory.copy_records(
                            RecordSources::Pointers(&infos[..live]),
                            &PAIRS_LAYOUT,
                            &mut pairs_headers[..live * 16],
                        ),
                        memory.copy_records(
                            RecordSources::Pointers(&infos[..live]),
                            &COOLDOWN_LAYOUT,
                            &mut cooldowns[..live * 4],
                        ),
                    )
                };
            for entry in 0..live {
                pairs_headers[entry * 16 + 12..entry * 16 + 16]
                    .copy_from_slice(&pair_counts[entry * 4..entry * 4 + 4]);
                let province = ids[entry];
                let province_pointer = batch.authenticate(entry, province, province_vtable)?;
                if info_copied <= entry {
                    return Err("rail info identity unreadable");
                }
                let header = &identity[entry * INFO_BYTES..(entry + 1) * INFO_BYTES];
                if u64::from_le_bytes(header[..8].try_into().unwrap()) != info_vtable as u64 {
                    return Err("rail info vtable mismatch");
                }
                let definition = batch.definition(entry)?;
                let status = province_pointer
                    .checked_add(cprovince::BUILDING_STATUS)
                    .ok_or("rail province building status address overflow")?;
                if id_copied <= entry {
                    return Err("rail province definition id unreadable");
                }
                let definition_id = u32::from_le_bytes(
                    definition_ids[entry * 4..entry * 4 + 4].try_into().unwrap(),
                );
                if definition_id != province {
                    return Err("rail province definition id differs from slot");
                }
                if u64::from_le_bytes(
                    header[info::PROVINCE_RECORD..info::BUILDING_STATUS]
                        .try_into()
                        .unwrap(),
                ) != definition
                    || u64::from_le_bytes(header[info::BUILDING_STATUS..info::BUILDING].try_into().unwrap())
                        != status as u64
                {
                    return Err("rail info province backlink mismatch");
                }
                plans[entry] = Plan::Unplanned;
                let held = if carrying {
                    let (token, carried) = carry.plan(&RailLocation {
                        province,
                        info: infos[entry],
                        building_status: status,
                        identity: header.try_into().unwrap(),
                        provinces,
                        cooldown: (cooldown_copied > entry).then(|| {
                            i32::from_le_bytes(
                                cooldowns[entry * 4..entry * 4 + 4].try_into().unwrap(),
                            )
                        }),
                        pairs: (pairs_copied > entry && counts_copied > entry).then(|| {
                            pairs_headers[entry * 16..entry * 16 + 16]
                                .try_into()
                                .unwrap()
                        }),
                    });
                    let held = if carried { carry.held(token) } else { None };
                    plans[entry] = if held.is_some() {
                        Plan::Carried { token }
                    } else {
                        Plan::Read { token, edges: None }
                    };
                    held
                } else {
                    None
                };
                if building_output.is_some() {
                    building_required += 1;
                    building_sources[entry] = fresh_inputs::RailwaySource {
                        province,
                        info: infos[entry],
                        building_status: status,
                        cooldown: if cooldown_copied > entry {
                            Ok(i32::from_le_bytes(
                                cooldowns[entry * 4..entry * 4 + 4].try_into().unwrap(),
                            ))
                        } else {
                            Err("rail cooldown unreadable")
                        },
                        pairs: if pairs_copied > entry && counts_copied > entry {
                            Array::from_header(
                                pairs_headers[entry * 16..entry * 16 + 16]
                                    .try_into()
                                    .unwrap(),
                                edge::STRIDE,
                            )
                            .ok_or("rail neighbor array unreadable or invalid")
                        } else {
                            Err("rail neighbor array unreadable or invalid")
                        },
                        selection: levels.as_deref_mut().map(|levels| {
                            selected_inputs::SelectionRow {
                                levels: match held {
                                    Some(held) => levels.append(held.levels),
                                    None => levels.read(
                                        memory,
                                        infos[entry],
                                        definition_pointers[entry],
                                    ),
                                },
                                pair_count: if counts_copied > entry {
                                    Ok(i32::from_le_bytes(
                                        pair_counts[entry * 4..entry * 4 + 4].try_into().unwrap(),
                                    ))
                                } else {
                                    Err("selected railway raw pair count unreadable")
                                },
                                ..Default::default()
                            }
                        }),
                        carried: false,
                    };
                    let source = &mut building_sources[entry];
                    source.carried = held.is_some()
                        && source
                            .selection
                            .is_some_and(|selection| selection.levels.is_ok());
                }
                if province_progress.error.is_none() {
                    if province_progress.len == province_output.len() {
                        province_progress.omit(1);
                    } else if cooldown_copied > entry {
                        province_output[province_progress.len] = info::Row {
                            province,
                            cooldown_ticks: i32::from_le_bytes(
                                cooldowns[entry * 4..entry * 4 + 4].try_into().unwrap(),
                            ),
                            building_bound: info::building_bound(header),
                        };
                        province_progress.len += 1;
                    } else {
                        province_progress.error = Some("rail cooldown unreadable");
                    }
                }
                let edge_start = (edge_progress.len, edge_progress.unread);
                if let (Some(held), None) = (held, edge_progress.error) {
                    if let Err(reason) =
                        append_edges(&mut edge_output, &mut edge_progress, held.edges)
                    {
                        edge_progress.error = Some(reason);
                    }
                } else if edge_progress.error.is_none() {
                    let pairs = (pairs_copied > entry && counts_copied > entry)
                        .then(|| {
                            Array::from_header(
                                pairs_headers[entry * 16..entry * 16 + 16]
                                    .try_into()
                                    .unwrap(),
                                edge::STRIDE,
                            )
                        })
                        .flatten();
                    let result = match pairs {
                        None => Err("rail neighbor array unreadable or invalid"),
                        Some(pairs) => read_edges(
                            memory,
                            provinces,
                            province_vtable,
                            &mut neighbor_batch,
                            &mut edge_scratch,
                            pairs,
                            province,
                            &mut edge_output,
                            &mut edge_progress,
                        ),
                    };
                    if let Err(reason) = result {
                        edge_progress.error = Some(reason);
                    }
                }
                if let Plan::Read { edges, .. } = &mut plans[entry] {
                    *edges = (edge_progress.error.is_none()
                        && edge_progress.unread == edge_start.1)
                        .then_some((edge_start.0, edge_progress.len));
                }
            }
            if let Some(output) = building_output.as_deref_mut() {
                let take = live.min(output.len() - building_progress.len);
                let first_row = building_progress.len;
                let outcome = fresh_inputs::railway_buildings(
                    memory,
                    base,
                    &building_sources[..live],
                    &mut output[first_row..first_row + take],
                );
                building_progress.len += outcome.len();
                match outcome {
                    Outcome::Truncated { unread_rows, .. } => building_progress.omit(unread_rows),
                    Outcome::Unavailable { reason } => building_progress.error = Some(reason),
                    Outcome::Rows { .. } => {}
                }
                if carrying {
                    settle(
                        carry,
                        &plans[..live],
                        &building_sources[..live],
                        (output, first_row, outcome.len()),
                        &edge_output,
                        levels.as_deref(),
                    );
                    carry.read_batch();
                }
                building_sources[..live].fill(fresh_inputs::RailwaySource::default());
            }
            if overflow {
                return Err("rail info pointer overflow");
            }
        }
        Ok(())
    })();
    match result {
        Err(reason) => BuildingOutcomes {
            rows: Outcomes::unavailable(reason),
            buildings: Outcome::Unavailable { reason },
            required: building_required,
        },
        Ok(()) => BuildingOutcomes {
            rows: Outcomes {
                provinces: province_progress.outcome(),
                edges: edge_progress.outcome(),
            },
            buildings: building_progress.outcome(),
            required: building_required,
        },
    }
}
#[derive(Clone, Copy)]
enum Plan {
    Unplanned,
    Read {
        token: usize,
        edges: Option<(usize, usize)>,
    },
    Carried {
        token: usize,
    },
}

fn settle(
    carry: &mut impl RailCarry,
    plans: &[Plan],
    sources: &[fresh_inputs::RailwaySource],
    (output, first_row, written): (&mut [fresh_inputs::RailwayBuildingRow], usize, usize),
    edges: &[edge::Row],
    levels: Option<&selected_inputs::LevelBuffer<'_>>,
) {
    let values = levels.map_or(&[][..], |levels| &levels.values[..levels.len]);
    for (entry, (plan, source)) in plans.iter().zip(sources).enumerate() {
        let row = (entry < written).then_some(first_row + entry);
        match *plan {
            Plan::Unplanned => {}
            Plan::Carried { token } => {
                let span = source.selection.and_then(|selection| selection.levels.ok());
                if let (Some(row), Some(span), Some(held), true) =
                    (row, span, carry.held(token), source.carried)
                {
                    output[row] = rebased(held.building, span.first);
                }
                carry.carried(token);
            }
            Plan::Read { token, edges: span } => {
                let outputs = (|| {
                    let (start, end) = span?;
                    let building = output[row?];
                    if !whole(&building) {
                        return None;
                    }
                    let levels = building.selection?.levels.ok()?;
                    Some(InfoOutputs {
                        cooldown: source.cooldown.ok()?,
                        edges: &edges[start..end],
                        building: rebased(building, 0),
                        levels: values.get(levels.first..levels.first.checked_add(levels.len)?)?,
                    })
                })();
                carry.read(token, outputs);
            }
        }
    }
}

pub const PROVINCE_BATCH: usize = 64;
const PROVINCE_RECORD_BYTES: usize = 20;
const SLOT_FIELD: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 8,
}];
const PROVINCE_FIELDS: [RecordField; 3] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: cprovince::ID,
        destination_offset: 8,
        len: 4,
    },
    RecordField {
        source_offset: cprovince::DEFINITION,
        destination_offset: 12,
        len: 8,
    },
];
const NEIGHBOR_FIELDS: [RecordField; 2] = [PROVINCE_FIELDS[0], PROVINCE_FIELDS[1]];
pub const INFO_BYTES: usize = info::BUILDING + 8;
const INFO_FIELD: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: INFO_BYTES,
}];
const PAIRS_FIELD: [RecordField; 1] = [RecordField {
    source_offset: info::NEIGHBOR_PAIRS,
    destination_offset: 0,
    len: 12,
}];
const PAIR_COUNT_FIELD: [RecordField; 1] = [RecordField {
    source_offset: info::NEIGHBOR_PAIR_COUNT,
    destination_offset: 0,
    len: 4,
}];
const COOLDOWN_FIELD: [RecordField; 1] = [RecordField {
    source_offset: info::COOLDOWN_TICKS,
    destination_offset: 0,
    len: 4,
}];
const DEFINITION_ID_FIELD: [RecordField; 1] = [RecordField {
    source_offset: crate::province_definition::ID,
    destination_offset: 0,
    len: 4,
}];

const SLOT_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&SLOT_FIELD, 8).expect("valid record layout");
const PROVINCE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PROVINCE_FIELDS, PROVINCE_RECORD_BYTES).expect("valid record layout");
const NEIGHBOR_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&NEIGHBOR_FIELDS, PROVINCE_RECORD_BYTES).expect("valid record layout");
const INFO_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&INFO_FIELD, INFO_BYTES).expect("valid record layout");
const PAIRS_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PAIRS_FIELD, 16).expect("valid record layout");
const PAIR_COUNT_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PAIR_COUNT_FIELD, 4).expect("valid record layout");
const COOLDOWN_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&COOLDOWN_FIELD, 4).expect("valid record layout");
const COMBINED_INFO_BYTES: usize = INFO_BYTES + 20;
const COMBINED_INFO_FIELDS: [RecordField; 4] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: INFO_BYTES,
    },
    RecordField {
        source_offset: info::NEIGHBOR_PAIRS,
        destination_offset: INFO_BYTES,
        len: 12,
    },
    RecordField {
        source_offset: info::NEIGHBOR_PAIR_COUNT,
        destination_offset: INFO_BYTES + 12,
        len: 4,
    },
    RecordField {
        source_offset: info::COOLDOWN_TICKS,
        destination_offset: INFO_BYTES + 16,
        len: 4,
    },
];
const COMBINED_INFO_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&COMBINED_INFO_FIELDS, COMBINED_INFO_BYTES).expect("valid record layout");
const DEFINITION_ID_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&DEFINITION_ID_FIELD, 4).expect("valid record layout");

struct ProvinceBatch {
    slots: [usize; PROVINCE_BATCH],
    slot_bytes: [u8; PROVINCE_BATCH * 8],
    records: [u8; PROVINCE_BATCH * PROVINCE_RECORD_BYTES],
    slot_copied: usize,
    copied: usize,
    fields: usize,
}

impl ProvinceBatch {
    fn new() -> Self {
        Self {
            slots: [0; PROVINCE_BATCH],
            slot_bytes: [0; PROVINCE_BATCH * 8],
            records: [0; PROVINCE_BATCH * PROVINCE_RECORD_BYTES],
            slot_copied: 0,
            copied: 0,
            fields: 0,
        }
    }

    fn gather(
        &mut self,
        memory: &impl Memory,
        provinces: Array,
        ids: &[u32],
        layout: &RecordLayout<'_>,
    ) {
        let len = ids.len().min(PROVINCE_BATCH);
        self.fields = layout.field_count();
        for (slot, &id) in self.slots[..len].iter_mut().zip(ids) {
            *slot = provinces.element(id, 8).unwrap_or(0);
        }
        self.slot_copied = memory.copy_records(
            RecordSources::Pointers(&self.slots[..len]),
            &SLOT_LAYOUT,
            &mut self.slot_bytes[..len * 8],
        );
        let mut objects = 0;
        for index in 0..self.slot_copied.min(len) {
            let word = u64::from_le_bytes(
                self.slot_bytes[index * 8..index * 8 + 8]
                    .try_into()
                    .unwrap(),
            );
            match usize::try_from(word) {
                Ok(pointer) if pointer != 0 => self.slots[index] = pointer,
                _ => break,
            }
            objects = index + 1;
        }
        self.copied = memory.copy_records(
            RecordSources::Pointers(&self.slots[..objects]),
            layout,
            &mut self.records[..objects * PROVINCE_RECORD_BYTES],
        );
    }

    fn copied(&self, index: usize) -> usize {
        self.copied
            .saturating_sub(index * self.fields)
            .min(self.fields)
    }

    fn authenticate(&self, index: usize, id: u32, vtable: usize) -> Result<usize, &'static str> {
        if id == 0 {
            return Err("rail references reserved province zero");
        }
        let pointer = (index < self.slot_copied)
            .then(|| {
                u64::from_le_bytes(
                    self.slot_bytes[index * 8..index * 8 + 8]
                        .try_into()
                        .unwrap(),
                )
            })
            .and_then(|word| usize::try_from(word).ok())
            .filter(|&pointer| pointer != 0)
            .ok_or("rail province reference missing or unreadable")?;
        pointer
            .checked_add(cprovince::ID)
            .ok_or("rail province id address overflow")?;
        if self.copied(index) < 2 {
            return Err("rail province identity unreadable");
        }
        let record = &self.records[index * PROVINCE_RECORD_BYTES..];
        if u64::from_le_bytes(record[..8].try_into().unwrap()) != vtable as u64 {
            return Err("rail province vtable mismatch");
        }
        if u32::from_le_bytes(record[8..12].try_into().unwrap()) != id {
            return Err("rail province id differs from slot");
        }
        Ok(pointer)
    }

    fn definition_word(&self, index: usize) -> u64 {
        if self.copied(index) < 3 {
            return 0;
        }
        let start = index * PROVINCE_RECORD_BYTES + 12;
        u64::from_le_bytes(self.records[start..start + 8].try_into().unwrap())
    }

    fn definition(&self, index: usize) -> Result<u64, &'static str> {
        match self.definition_word(index) {
            0 => Err("rail province definition unreadable or null"),
            word => Ok(word),
        }
    }
}

struct EdgeScratch {
    bytes: [u8; PROVINCE_BATCH * edge::STRIDE],
    neighbors: [u32; PROVINCE_BATCH],
}

impl EdgeScratch {
    fn new() -> Self {
        Self {
            bytes: [0; PROVINCE_BATCH * edge::STRIDE],
            neighbors: [0; PROVINCE_BATCH],
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn read_edges(
    memory: &impl Memory,
    provinces: Array,
    province_vtable: usize,
    batch: &mut ProvinceBatch,
    scratch: &mut EdgeScratch,
    pairs: Array,
    province: u32,
    output: &mut Buffer<'_, edge::Row>,
    progress: &mut Progress,
) -> Result<(), &'static str> {
    let required = progress
        .len
        .checked_add(pairs.count as usize)
        .ok_or("rail edge demand overflow")?;
    output.ensure(required)?;
    let take = (pairs.count as usize).min(output.len() - progress.len);
    progress.omit(pairs.count - take as u32);
    let EdgeScratch {
        bytes: buffer,
        neighbors,
    } = scratch;
    let mut previous = 0;
    for first in (0..take).step_by(PROVINCE_BATCH) {
        let count = (take - first).min(PROVINCE_BATCH);
        let address = pairs
            .element(first as u32, edge::STRIDE)
            .ok_or("rail pair address overflow")?;
        if !read_slice(memory, address, &mut buffer[..count * edge::STRIDE]) {
            return Err("rail neighbor rows unreadable");
        }
        let mut staged = 0;
        let mut invalid = false;
        let mut ordered = previous;
        for bytes in buffer[..count * edge::STRIDE].chunks_exact(edge::STRIDE) {
            let neighbor = i32::from_le_bytes(bytes[..edge::LEVEL].try_into().unwrap());
            if neighbor <= 0 || neighbor as u32 <= ordered || neighbor as u32 == province {
                invalid = true;
                break;
            }
            ordered = neighbor as u32;
            neighbors[staged] = ordered;
            staged += 1;
        }
        batch.gather(memory, provinces, &neighbors[..staged], &NEIGHBOR_LAYOUT);
        for (index, bytes) in buffer[..count * edge::STRIDE]
            .chunks_exact(edge::STRIDE)
            .enumerate()
        {
            if index == staged {
                if invalid {
                    return Err("rail neighbors duplicate, unsorted or invalid");
                }
                break;
            }
            let to_province = neighbors[index];
            batch.authenticate(index, to_province, province_vtable)?;
            output[progress.len] = edge::Row {
                from_province: province,
                to_province,
                level: i32::from_le_bytes(bytes[edge::LEVEL..].try_into().unwrap()),
            };
            progress.len += 1;
            previous = to_province;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
