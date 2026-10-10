//! CRailwayManager's persistent rail province and directed edge state.

use std::collections::HashMap;

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

/// CRailwayManager primary vtable.
pub const VTABLE_RVA: usize = 0x0295_c0b0;

/// CRailwayManager province-indexed CProvinceRailwayInfo pointer table.
pub const PROVINCE_INFO_TABLE: usize = 0x08;
/// Province-info table signed i32 count.
pub const PROVINCE_INFO_COUNT: usize = 0x14;
/// Cooldown province list data pointer.
pub const COOLDOWN_PROVINCES: usize = 0x20;
/// Cooldown province-list signed count, traversed backwards from count-1.
pub const COOLDOWN_PROVINCE_COUNT: usize = 0x2c;
/// CRailwayManager table pointer stride.
pub const PROVINCE_INFO_STRIDE: usize = 8;
/// Manager constructor sizing and zero-filling the province table.
pub const CONSTRUCTOR_RVA: usize = 0x00e7_d380;
/// Province-info lookup/creation stores the selected table slot.
pub const GET_OR_CREATE_PROVINCE_INFO_RVA: usize = 0x00e7_d6d0;
/// Railway token loader can allocate and replace info slots.
pub const LOAD_TOKEN_RVA: usize = 0x00e8_0320;
/// Province-cooldown setter writes stored cooldown and notifies supply.
pub const SET_PROVINCE_ON_COOLDOWN_RVA: usize = 0x00e7_f9b0;
/// Cooldown updater decrements values and removes expired IDs.
pub const UPDATE_COOLDOWNS_RVA: usize = 0x00e7_f160;

/// Successful directed-edge speeds shared within one hourly acquisition.
/// The query has no division operand. Clear storage before every capture to
/// reread dynamic map inputs. Worker caches grow from reached dependencies;
/// vacant callback caches use the complete direct reader without allocation.
pub(crate) struct MovementSpeeds {
    key: Option<(usize, usize)>,
    entries: HashMap<(usize, usize), i64>,
    worker_growth: bool,
    infrastructure: HashMap<usize, i64>,
    inputs: MovementInputs,
}

/// Lazy current-pass dependencies shared by different directed edges. A hit
/// retains the complete successful authentication that admitted that value.
struct MovementInputs {
    provinces: Option<Array>,
    identities: HashMap<usize, u32>,
    slots: Option<Array>,
    defines: [Option<i64>; 5],
}

impl MovementInputs {
    fn clear(&mut self) {
        self.provinces = None;
        self.identities.clear();
        self.slots = None;
        self.defines.fill(None);
    }
    fn province(
        &mut self,
        memory: &impl Memory,
        base: usize,
        provinces: Array,
        pointer: usize,
    ) -> Result<u32, &'static str> {
        if let Some(&id) = self.identities.get(&pointer) {
            return Ok(id);
        }
        let id = eta_province(memory, base, provinces, pointer)?;
        crate::allocation::reserve_map(&mut self.identities, 1)
            .map_err(|_| "ETA province cache allocation failed")?;
        self.identities.insert(pointer, id);
        Ok(id)
    }
}

fn current<T: Copy>(
    slot: Option<&mut Option<T>>,
    read: impl FnOnce() -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    if let Some(value) = slot.as_deref().copied().flatten() {
        return Ok(value);
    }
    let value = read()?;
    if let Some(slot) = slot {
        *slot = Some(value);
    }
    Ok(value)
}

impl MovementSpeeds {
    pub fn try_new(capacity: usize) -> std::io::Result<Self> {
        let mut memo = Self::vacant();
        memo.worker_growth = true;
        // Known directed-edge query demand prepares the edge cache. State and
        // province dependencies grow only when reached by the corresponding queries.
        crate::allocation::reserve_map(&mut memo.entries, capacity)?;
        Ok(memo)
    }

    #[cfg(test)]
    pub fn new(capacity: usize) -> Self {
        Self::try_new(capacity).unwrap()
    }

    pub fn vacant() -> Self {
        Self {
            key: None,
            inputs: MovementInputs {
                provinces: None,
                identities: HashMap::new(),
                slots: None,
                defines: [None; 5],
            },
            entries: HashMap::new(),
            worker_growth: false,
            infrastructure: HashMap::new(),
        }
    }

    pub fn clear(&mut self) {
        self.key = None;
        self.inputs.clear();
        self.entries.clear();
        self.infrastructure.clear();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<(usize, usize, i64)>()
            + self.infrastructure.capacity() * size_of::<(usize, i64)>()
            + self.inputs.identities.capacity() * size_of::<(usize, u32)>()
    }

    pub fn get(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        from: usize,
        to: usize,
    ) -> Result<i64, &'static str> {
        if self.key.is_some_and(|key| key != (base, state)) {
            self.clear();
        }
        self.key = Some((base, state));
        if !self.worker_growth {
            return movement_speed(memory, base, state, from, to);
        }
        if let Some(&speed) = self.entries.get(&(from, to)) {
            return Ok(speed);
        }
        let infrastructure = &mut self.infrastructure;
        let result = movement_speed_with_infrastructure(
            memory,
            base,
            state,
            from,
            to,
            Some(&mut self.inputs),
            |province| Self::infrastructure(infrastructure, memory, base, state, province),
        );
        let speed = match result {
            Ok(speed) => speed,
            Err(reason) => {
                self.clear();
                return Err(reason);
            }
        };
        crate::allocation::reserve_map(&mut self.entries, 1)
            .map_err(|_| "ETA edge cache allocation failed")?;
        self.entries.insert((from, to), speed);
        Ok(speed)
    }

    /// Different destination provinces share the same current state fraction.
    /// Keep the province-to-state read, then reuse that state's complete,
    /// authenticated lookup. No entry survives the hourly clear.
    fn infrastructure(
        entries: &mut HashMap<usize, i64>,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        province: usize,
    ) -> Result<i64, &'static str> {
        let state = eta_read::<u64>(memory, province, cprovince::STATE)? as usize;
        if state == 0 {
            return Ok(0);
        }
        if let Some(&value) = entries.get(&state) {
            return Ok(value);
        }
        let value = eta_state_infrastructure(memory, base, game_state, state)?;
        crate::allocation::reserve_map(entries, 1)
            .map_err(|_| "ETA state cache allocation failed")?;
        entries.insert(state, value);
        Ok(value)
    }
}

/// Passive railway movement speed before the army's outer scale. Road and
/// directed rail speeds compete by signed maximum. Rail damage and cooldown
/// come from departure province. No calls, retained pointers, allocation or
/// replacement census.
pub fn movement_speed(
    memory: &impl Memory,
    base: usize,
    state: usize,
    from: usize,
    to: usize,
) -> Result<i64, &'static str> {
    movement_speed_with_infrastructure(memory, base, state, from, to, None, |province| {
        eta_infrastructure(memory, base, state, province)
    })
}

fn movement_speed_with_infrastructure(
    memory: &impl Memory,
    base: usize,
    state: usize,
    from: usize,
    to: usize,
    mut inputs: Option<&mut MovementInputs>,
    infrastructure: impl FnOnce(usize) -> Result<i64, &'static str>,
) -> Result<i64, &'static str> {
    let provinces = current(
        inputs.as_deref_mut().map(|inputs| &mut inputs.provinces),
        || eta_array(memory, state, cgame_state::PROVINCES, 8),
    )?;
    let mut province = |pointer| match inputs.as_deref_mut() {
        Some(inputs) => inputs.province(memory, base, provinces, pointer),
        None => eta_province(memory, base, provinces, pointer),
    };
    let from_id = province(from)?;
    let to_id = province(to)?;
    let infrastructure = infrastructure(to)?;
    // All four speed defines are signed fixed5 qwords.
    let road_base = current(inputs.as_deref_mut().map(|v| &mut v.defines[0]), || {
        eta_read::<i64>(memory, base, 0x0331_cf40)
    })?;
    let road_scale = current(inputs.as_deref_mut().map(|v| &mut v.defines[1]), || {
        eta_read::<i64>(memory, base, 0x0331_cfd0)
    })?;
    let road = road_base.wrapping_add(infrastructure.wrapping_mul(road_scale) / 100_000);
    let percentage = railway_percentage_from_ids(
        memory,
        base,
        state,
        provinces,
        (from, from_id),
        to_id,
        inputs.as_deref_mut(),
    )?;
    let rail_base = current(inputs.as_deref_mut().map(|v| &mut v.defines[3]), || {
        eta_read::<i64>(memory, base, 0x0331_d068)
    })?;
    let rail_scale = current(inputs.map(|v| &mut v.defines[4]), || {
        eta_read::<i64>(memory, base, 0x0331_d0e8)
    })?;
    let rail = rail_base.wrapping_add(percentage.wrapping_mul(rail_scale) / 100_000);
    Ok(road.max(rail))
}

/// Passive railway percentage used by railway-gun speed. Only directed rail
/// connection and departure health participate; army road speed is unreached.
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
    railway_percentage_from_ids(memory, base, state, provinces, (from, from_id), to_id, None)
}

fn railway_percentage_from_ids(
    memory: &impl Memory,
    base: usize,
    state: usize,
    provinces: Array,
    from: (usize, u32),
    to_id: u32,
    mut inputs: Option<&mut MovementInputs>,
) -> Result<i64, &'static str> {
    let slots = current(inputs.as_deref_mut().map(|v| &mut v.slots), || {
        let manager = eta_pointer(memory, state, cgame_state::RAIL_MANAGER)?;
        eta_vtable(memory, base, manager, VTABLE_RVA)?;
        let slots = eta_array(memory, manager, PROVINCE_INFO_TABLE, 8)?;
        if slots.count != provinces.count {
            return Err("ETA railway province extent mismatch");
        }
        Ok(slots)
    })?;
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
        // Railway percentage accepts zero and negative cooldowns.
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
    // The denominator define is signed i32, unlike speed defines.
    // Zero returns zero rather than the fixed-point sentinel.
    let denominator = current(inputs.map(|v| &mut v.defines[2]), || {
        eta_read::<i32>(memory, base, 0x0331_bfe0).map(i64::from)
    })? * 100_000;
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
    rva: usize,
) -> Result<(), &'static str> {
    if pointer == 0 || eta_read::<u64>(memory, pointer, 0)? != eta_address(base, rva)? as u64 {
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

/// Signed shorts and wrapping fixed5 operations. Partial health divides
/// by 10,000,000 rather than the healthy-level scale.
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
    let database = eta_pointer(memory, base, database::SINGLETON_RVA)?;
    // Select the infrastructure template from its database field.
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
    // Prefer the first positive local value, then the second, then the signed
    // global define, without a lower clamp.
    let mut maximum = eta_read::<i32>(memory, definition, 0x2bc)?;
    if maximum < 1 {
        maximum = eta_read::<i32>(memory, definition, 0x2b8)?;
        if maximum <= 0 {
            maximum = eta_read::<i32>(memory, base, 0x0331_b5b4)?;
        }
    }
    eta_ratio(i64::from(healthy) * 100_000, i64::from(maximum) * 100_000)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcomes {
    pub provinces: Outcome,
    pub edges: Outcome,
}

/// Optional building operands have their own status and retained demand.
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

/// One shared census, with independent cooldown and edge outcomes. Null info
/// slots are absent; an allocated info with zero cooldown remains a row.
/// Directed pairs are retained independently, without reciprocal-edge guesses.
/// Fixed-size stack windows amortize guarded copies; no allocation or retry.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    province_output: &mut [info::Row],
    edge_output: &mut [edge::Row],
) -> Outcomes {
    walk_inner(
        memory,
        base,
        game_state,
        Buffer::Fixed(province_output),
        Buffer::Fixed(edge_output),
        None,
        None,
    )
    .rows
}

/// Reuse the authenticated info windows; no second province census.
pub fn walk_with_buildings(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    province_output: &mut [info::Row],
    edge_output: &mut [edge::Row],
    building_output: &mut [fresh_inputs::RailwayBuildingRow],
) -> BuildingOutcomes {
    building_output.fill(fresh_inputs::RailwayBuildingRow::default());
    walk_inner(
        memory,
        base,
        game_state,
        Buffer::Fixed(province_output),
        Buffer::Fixed(edge_output),
        Some(Buffer::Fixed(building_output)),
        None,
    )
}

/// Selected operands share this same info identity, cooldown, pair-count and
/// building pass. No current unit census or attach-time baseline is required.
pub fn walk_with_selection(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    province_output: &mut [info::Row],
    edge_output: &mut [edge::Row],
    building_output: &mut [fresh_inputs::RailwayBuildingRow],
    levels: &mut selected_inputs::Levels<'_>,
) -> BuildingOutcomes {
    building_output.fill(fresh_inputs::RailwayBuildingRow::default());
    let mut buffer = selected_inputs::LevelBuffer {
        values: Buffer::Fixed(levels.values),
        len: 0,
        required: 0,
    };
    let outcomes = walk_inner(
        memory,
        base,
        game_state,
        Buffer::Fixed(province_output),
        Buffer::Fixed(edge_output),
        Some(Buffer::Fixed(building_output)),
        Some(&mut buffer),
    );
    levels.len = buffer.len;
    levels.required = buffer.required;
    outcomes
}

/// Worker province rows follow live province extent. Edges and indexed
/// levels grow from original headers before dependent rows are published.
pub fn walk_with_selection_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    province_output: &mut Vec<info::Row>,
    edge_output: &mut Vec<edge::Row>,
    building_output: &mut Vec<fresh_inputs::RailwayBuildingRow>,
    levels: &mut selected_inputs::GrowingLevels<'_>,
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
    );
    levels.len = buffer.len;
    levels.required = buffer.required;
    outcomes
}

fn walk_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut province_output: Buffer<'_, info::Row>,
    mut edge_output: Buffer<'_, edge::Row>,
    mut building_output: Option<Buffer<'_, fresh_inputs::RailwayBuildingRow>>,
    mut levels: Option<&mut selected_inputs::LevelBuffer<'_>>,
) -> BuildingOutcomes {
    let mut province_progress = Progress::default();
    let mut edge_progress = Progress::default();
    let mut building_progress = Progress::default();
    let mut building_required = 0;
    let result = (|| {
        let manager = game_state
            .checked_add(cgame_state::RAIL_MANAGER)
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("rail manager unreadable or null")?;
        let manager_vtable = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")?;
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
        province_output.ensure(rail.count as usize)?;
        if let Some(output) = building_output.as_mut() {
            output.ensure(rail.count as usize)?;
        }
        let info_vtable = base
            .checked_add(info::VTABLE_RVA)
            .ok_or("module address overflow")?;
        let province_vtable = base
            .checked_add(cprovince::VTABLE_RVA)
            .ok_or("module address overflow")?;
        // A rail slot's own province, its definition, the info identity and
        // backlinks, the cooldown and the neighbor header are all gathered for
        // the whole batch. Every check below still runs in slot order against
        // that entry's copied prefix, so each reason keeps its position. The
        // fatal identity, the cooldown and the neighbor header take three
        // separate gathers, because a fault scoped to one table must not stop
        // another table's independently readable field for any later record.
        let mut slots = [0_u8; PROVINCE_BATCH * PROVINCE_INFO_STRIDE];
        let mut ids = [0_u32; PROVINCE_BATCH];
        let mut infos = [0_usize; PROVINCE_BATCH];
        let mut identity = [0_u8; PROVINCE_BATCH * 24];
        let mut pairs_headers = [0_u8; PROVINCE_BATCH * 16];
        let mut pair_counts = [0_u8; PROVINCE_BATCH * 4];
        let mut cooldowns = [0_u8; PROVINCE_BATCH * 4];
        let mut definition_ids = [0_u8; PROVINCE_BATCH * 4];
        let mut definition_pointers = [0_usize; PROVINCE_BATCH];
        let mut batch = ProvinceBatch::new();
        let mut neighbor_batch = ProvinceBatch::new();
        let mut edge_scratch = EdgeScratch::new();
        let mut building_sources = [fresh_inputs::RailwaySource::default(); PROVINCE_BATCH];
        for first in (0..rail.count).step_by(PROVINCE_BATCH) {
            let count = (rail.count - first).min(PROVINCE_BATCH as u32);
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
            // Saved province key comes from the definition. Authenticate it against
            // the already checked live slot rather than a proxy.
            let id_copied = memory.copy_records(
                RecordSources::Pointers(&definition_pointers[..live]),
                &DEFINITION_ID_LAYOUT,
                &mut definition_ids[..live * 4],
            );
            let info_copied = memory.copy_records(
                RecordSources::Pointers(&infos[..live]),
                &INFO_LAYOUT,
                &mut identity[..live * 24],
            );
            // The signed pair count is consumed independently of pointer/capacity.
            // Read it once and reuse its raw bytes in the edge-array header.
            let counts_copied = memory.copy_records(
                RecordSources::Pointers(&infos[..live]),
                &PAIR_COUNT_LAYOUT,
                &mut pair_counts[..live * 4],
            );
            let pairs_copied = memory.copy_records(
                RecordSources::Pointers(&infos[..live]),
                &PAIRS_LAYOUT,
                &mut pairs_headers[..live * 16],
            );
            let cooldown_copied = memory.copy_records(
                RecordSources::Pointers(&infos[..live]),
                &COOLDOWN_LAYOUT,
                &mut cooldowns[..live * 4],
            );
            for entry in 0..live {
                pairs_headers[entry * 16 + 12..entry * 16 + 16]
                    .copy_from_slice(&pair_counts[entry * 4..entry * 4 + 4]);
                let province = ids[entry];
                let province_pointer = batch.authenticate(entry, province, province_vtable)?;
                if info_copied <= entry {
                    return Err("rail info identity unreadable");
                }
                let header = &identity[entry * 24..entry * 24 + 24];
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
                    || u64::from_le_bytes(header[info::BUILDING_STATUS..24].try_into().unwrap())
                        != status as u64
                {
                    return Err("rail info province backlink mismatch");
                }
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
                                levels: levels.read(
                                    memory,
                                    infos[entry],
                                    definition_pointers[entry],
                                ),
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
                    };
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
                        };
                        province_progress.len += 1;
                    } else {
                        province_progress.error = Some("rail cooldown unreadable");
                    }
                }
                if edge_progress.error.is_none() {
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
            }
            if let Some(output) = building_output.as_deref_mut() {
                let take = live.min(output.len() - building_progress.len);
                let outcome = fresh_inputs::railway_buildings(
                    memory,
                    base,
                    &building_sources[..live],
                    &mut output[building_progress.len..building_progress.len + take],
                );
                building_progress.len += outcome.len();
                match outcome {
                    Outcome::Truncated { unread_rows, .. } => building_progress.omit(unread_rows),
                    Outcome::Unavailable { reason } => building_progress.error = Some(reason),
                    Outcome::Rows { .. } => {}
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
/// Provinces authenticated per guarded round, and the exact record each needs.
const PROVINCE_BATCH: usize = 64;
const PROVINCE_RECORD_BYTES: usize = 20;
const SLOT_FIELD: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 8,
}];
/// A rail slot's province: identity, then the definition its key comes from.
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
/// A directed edge's far endpoint is only authenticated, never read further.
const NEIGHBOR_FIELDS: [RecordField; 2] = [PROVINCE_FIELDS[0], PROVINCE_FIELDS[1]];
/// The info identity and both backlinks. An unreadable identity is fatal to
/// both tables, so this span must not share a gather with a field whose fault
/// is scoped to one table: `copy_records` stops at the first faulting field,
/// which would turn a later record's readable identity into a fatal one.
const INFO_FIELD: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 24,
}];
/// The neighbor vector header. Its fault is scoped to the edge table only.
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
/// The cooldown. Its fault is scoped to the cooldown table only.
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
    RecordLayout::new(&INFO_FIELD, 24).expect("valid record layout");
const PAIRS_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PAIRS_FIELD, 16).expect("valid record layout");
const PAIR_COUNT_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PAIR_COUNT_FIELD, 4).expect("valid record layout");
const COOLDOWN_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&COOLDOWN_FIELD, 4).expect("valid record layout");
const DEFINITION_ID_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&DEFINITION_ID_FIELD, 4).expect("valid record layout");

/// One batch of province-table authentications. Two guarded rounds replace the
/// two round trips each province used to make: the table slots, then each
/// object's identity. Reasons and their order per province are unchanged.
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
        // A null or unreadable slot fails at its own province, so no later
        // object is gathered; that province reports the original reason.
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

    /// The raw definition word, or zero when its exact field did not copy.
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

// Only freshly filled prefixes are consumed. Reuse these initialized buffers
// across infos without clearing bytes or ids outside the current prefix.
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
        // The sorted, non-self neighbor rule is a pure decode, so it selects
        // the prefix whose far endpoints are authenticated in one batch. The
        // first invalid pair still fails before any later pair is stored.
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
