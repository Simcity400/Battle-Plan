//! CFEXMember naval combat slot and convoy-terminal operands.
//! Sunk-ship and convoy event hooks are excluded. The copied battle-slot
//! information survives ship loss and remains separate from live ship fields.

use crate::capture_text::Value;
use crate::{Memory, cnaval_combatant, cpdx_array::Array, creference_registry, cship, read};

/// CFEXMember vtable.
pub const VTABLE_RVA: usize = 0x02a0_a228;
/// Packed ship u64 handle; zero is valid only for convoy members.
pub const SHIP_HANDLE: usize = 0x08;
/// CFEXMember convoy handle u64.
pub const CONVOY_HANDLE: usize = 0x10;
/// CFEXMember owner country id u32.
pub const OWNER: usize = 0x20;
/// Cached build cost IC i64.
pub const BUILD_COST_IC: usize = 0x30;
/// Cached strength ratio i64 fixed5.
pub const CACHED_STRENGTH: usize = 0x38;
/// Signed arrival hours.
pub const ARRIVAL_HOURS: usize = 0xe4;
/// Received critical count i32.
pub const RECEIVED_CRITICALS: usize = 0x124;
/// Carrier-plane evacuation state i32.
pub const EVACUATION_STATE: usize = 0x128;
/// Detection state i32.
pub const DETECTION_STATE: usize = 0x12c;
/// Escape progress i64 fixed5.
pub const ESCAPE_PROGRESS: usize = 0x130;
/// Six received-damage i64 lanes.
pub const RECEIVED_DAMAGE: usize = 0x198;

/// Caller-owned typed arenas; spans in DetailsRow index these arrays. Ship
/// destinations grow on the acquisition worker from the original native counts.
pub struct DetailBuffers<'a> {
    pub damage: &'a mut Vec<crate::cfex_damage_entry::Row>,
    pub air_hits: &'a mut Vec<crate::naval_combat_air_hit::Row>,
    pub naval_hits: &'a mut Vec<crate::naval_combat_hit::Row>,
    pub lengths: [usize; 3],
    pub required: [usize; 3],
    pub planes: crate::walk::Buffer<'a, crate::cfex_air::Row>,
    pub planes_len: usize,
    pub planes_required: usize,
    pub plane_air_hits: crate::walk::Buffer<'a, crate::naval_combat_air_hit::Row>,
    pub plane_naval_hits: crate::walk::Buffer<'a, crate::naval_combat_hit::Row>,
    pub plane_hit_lengths: [usize; 2],
    pub plane_hit_required: [usize; 2],
    pub plane_kills: crate::walk::Buffer<'a, crate::naval_air_efficiency_inputs::WingKill>,
    pub plane_kills_len: usize,
    pub plane_kills_required: usize,
    pub weapon_sources: &'a mut Vec<crate::naval_ship_modifier_inputs::SourceRow>,
    pub weapon_terms: &'a mut Vec<crate::naval_ship_modifier_inputs::ModifierTerm>,
    pub weapon_lengths: [usize; 2],
    pub weapon_required: [usize; 2],
    pub commander_inputs: crate::walk::Buffer<'a, crate::naval_commander_inputs::Row>,
    pub commander_inputs_len: usize,
    pub commander_inputs_required: usize,
    pub commander_timed_traits: &'a mut Vec<crate::cunit_leader::TimedTrait>,
    pub commander_timed_traits_len: usize,
    /// Each side's task-force handles (`CCombatant+0x20`).
    pub side_units: crate::walk::Buffer<'a, u64>,
    pub side_units_len: usize,
    pub side_units_required: usize,
    /// Side country tags, grown at admission on the acquisition worker.
    pub summary_countries: &'a mut Vec<u32>,
    pub summary_countries_len: usize,
    /// Signed custom-sea values for regions of the live combat walk.
    pub custom_sea: crate::walk::Buffer<'a, i16>,
    pub custom_sea_len: usize,
    pub custom_sea_required: usize,
    pub damage_sources: crate::walk::Buffer<'a, (u32, i64)>,
    pub damage_sources_len: usize,
    pub damage_sources_required: usize,
    pub wing_types: crate::walk::Buffer<'a, u32>,
    pub wing_types_len: usize,
    pub wing_types_required: usize,
    pub plane_wings: crate::walk::Buffer<'a, u64>,
    pub plane_wings_len: usize,
    pub plane_wings_required: usize,
}

pub const DETAIL_ARENA_CAPACITY: &str = "naval detail nested arena capacity exceeded";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DetailsRow {
    /// Same-pass group pointer for the convoy source join; never replayed.
    pub native_group: usize,
    pub member: cnaval_combatant::Row,
    /// Ordered group source: 0 active +0x8, 1 retained +0x70.
    pub member_source: u8,
    pub group_index: u32,
    pub member_index: u32,
    pub convoy: u64,
    pub subunit_index: i32,
    pub formation_type: i32,
    pub build_cost_ic: i64,
    pub cached_strength_ratio: i64,
    pub pride: u8,
    pub is_convoy: u8,
    // Stored std::strings retain their exact copied lengths.
    pub name: crate::capture_text::Owned,
    pub type_name: crate::capture_text::Owned,
    pub sprite: crate::capture_text::Owned,
    pub sunk_by: crate::capture_text::Owned,
    pub variant: u64,
    pub arrival_hours: i32,
    pub received_criticals: i32,
    pub evacuation_state: i32,
    pub detection_state: i32,
    pub escape_progress: i64,
    /// State 8 only: the live ship +0x228 or live convoy variant +0x208
    /// provides the stored effective-speed operand for on-demand escape calculation.
    pub escape_speed_base: i64,
    pub last_target: crate::clast_target_info::Row,
    pub received_damage: [i64; 6],
    /// Live ship can have disappeared while its copied battle slot survives.
    pub live_ship: u8,
    pub live_convoy: u8,
    pub strength: i64,
    pub organisation: i64,
    /// Raw CShip +300/+308, before the window's organisation modifier composition.
    pub base_organisation: i64,
    pub maximum_hit_points: i64,
    pub formation_flags: u64,
    pub weapon_context: crate::naval_ship_modifier_inputs::ContextRow,
    /// Start/count pairs in the corresponding caller arena.
    pub damage_entries: [usize; 2],
    /// Distinct member+150 and+168 arrays, retained in native order.
    pub air_hits: [[usize; 2]; 2],
    pub naval_hits: [usize; 2],
}
impl DetailsRow {
    pub fn allocated_bytes(&self) -> usize {
        self.name.allocated_bytes()
            + self.type_name.allocated_bytes()
            + self.sprite.allocated_bytes()
            + self.sunk_by.allocated_bytes()
            + self.weapon_context.terrain_name.allocated_bytes()
    }
}

/// Read a copied battle slot using its authenticated group and stable native key.
/// Storage grows only on the acquisition worker. Every full handle and byte
/// survives; no engine function is called and no observation is retried.
#[allow(clippy::too_many_arguments)]
pub(crate) fn details(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    group: usize,
    combat: u64,
    side: u8,
    game_state: usize,
    buffers: &mut DetailBuffers<'_>,
) -> Result<DetailsRow, &'static str> {
    use cnaval_combatant::field;
    let member = cnaval_combatant::member(memory, base, pointer, group, combat, side)?;
    let mut row_counts = [0usize; 3];
    // Each array uses its stored count. Header validation checks capacity
    // and extent; guarded entry reads validate payloads.
    let mut arrays = [Array { data: 0, count: 0 }; 4];
    for (header, (offset, stride, lane)) in arrays.iter_mut().zip([
        (DAMAGE_ENTRIES, 0x20, 0),
        (0x150, 8, 1),
        (0x168, 8, 1),
        (0x180, 8, 2),
    ]) {
        let array = Array::read(
            memory,
            pointer
                .checked_add(offset)
                .ok_or("naval detail nested header overflow")?,
            stride,
        )
        .ok_or("naval detail nested array invalid")?;
        row_counts[lane] += array.count as usize;
        *header = array;
    }
    for (lane, count) in row_counts.into_iter().enumerate() {
        buffers.required[lane] = buffers.required[lane]
            .checked_add(count)
            .ok_or("naval detail required capacity overflow")?;
    }
    let text = |offset| {
        crate::capture_text::Owned::read(
            memory,
            pointer
                .checked_add(offset)
                .ok_or("naval detail text overflow")?,
            false,
        )
        .map_err(|reason| {
            if reason == "string allocation failed" {
                reason
            } else {
                "naval detail text invalid or unreadable"
            }
        })
    };
    let mut row = DetailsRow {
        native_group: group,
        member,
        convoy: field(memory, pointer, CONVOY_HANDLE)?,
        subunit_index: field(memory, pointer, 0x24)?,
        formation_type: field(memory, pointer, 0x28)?,
        build_cost_ic: field(memory, pointer, BUILD_COST_IC)?,
        cached_strength_ratio: field(memory, pointer, CACHED_STRENGTH)?,
        pride: field(memory, pointer, 0x40)?,
        is_convoy: field(memory, pointer, 0x41)?,
        name: text(0x48)?,
        type_name: text(0x68)?,
        sprite: text(0x88)?,
        sunk_by: text(ATTRIBUTION_NAME)?,
        variant: field(memory, pointer, 0xc8)?,
        arrival_hours: field(memory, pointer, ARRIVAL_HOURS)?,
        received_criticals: field(memory, pointer, RECEIVED_CRITICALS)?,
        evacuation_state: field(memory, pointer, EVACUATION_STATE)?,
        detection_state: field(memory, pointer, DETECTION_STATE)?,
        escape_progress: field(memory, pointer, ESCAPE_PROGRESS)?,
        last_target: crate::clast_target_info::read(
            memory,
            base,
            pointer
                .checked_add(0x138)
                .ok_or("naval last-target overflow")?,
        )?,
        ..DetailsRow::default()
    };
    let damage: [u8; 48] = field(memory, pointer, RECEIVED_DAMAGE)?;
    for (value, bytes) in row.received_damage.iter_mut().zip(damage.chunks_exact(8)) {
        *value = i64::from_le_bytes(bytes.try_into().unwrap());
    }
    row.damage_entries = append_entries(
        memory,
        arrays[0],
        0x20,
        EntryOutput::Owned(buffers.damage),
        &mut buffers.lengths[0],
        |p| crate::cfex_damage_entry::read(memory, base, p),
        false,
    )?;
    for (index, array) in arrays[1..3].iter().copied().enumerate() {
        row.air_hits[index] = append_entries(
            memory,
            array,
            8,
            EntryOutput::Owned(buffers.air_hits),
            &mut buffers.lengths[1],
            |p| crate::naval_combat_air_hit::read(memory, base, p),
            true,
        )?;
    }
    row.naval_hits = append_entries(
        memory,
        arrays[3],
        8,
        EntryOutput::Owned(buffers.naval_hits),
        &mut buffers.lengths[2],
        |p| crate::naval_combat_hit::read(memory, base, p),
        true,
    )?;
    if let Some(ship) = creference_registry::resolve(memory, base, member.ship)? {
        if field::<u64>(memory, ship, 0)? != (base + cship::VTABLE_RVA) as u64
            || field::<u64>(memory, ship, cship::HANDLE)? != member.ship
        {
            return Err("naval detail ship identity mismatch");
        }
        row.live_ship = 1;
        if row.member.state == 8 {
            row.escape_speed_base = field(memory, ship, 0x228)?;
        }
        row.strength = field(memory, ship, cship::STRENGTH)?;
        row.organisation = field(memory, ship, cship::ORGANISATION)?;
        row.base_organisation = field(memory, ship, 0x300)?;
        row.maximum_hit_points = field(memory, ship, 0x308)?;
        row.formation_flags = field(memory, ship, cship::FORMATION_FLAGS)?;
        row.weapon_context = crate::naval_ship_modifier_inputs::read(
            memory,
            base,
            game_state,
            ship,
            field::<u64>(memory, group, crate::cfex_group::SIDE)? as usize,
            buffers,
        )?;
    }
    if row.live_ship == 0
        && let Some(convoy) = creference_registry::resolve(memory, base, row.convoy)?
    {
        let values =
            crate::cconvoy_ship::read(memory, base, convoy, row.convoy, row.member.state == 8)?;
        row.live_convoy = 1;
        row.escape_speed_base = values.speed;
        row.strength = values.strength;
        row.organisation = values.organisation;
        row.maximum_hit_points = values.maximum_strength;
        row.base_organisation = values.maximum_organisation;
    }
    Ok(row)
}

pub(crate) enum EntryOutput<'a, T> {
    Owned(&'a mut Vec<T>),
    Slice(&'a mut [T]),
}

/// Owned storage reserves the validated header count once; rows are appended
/// only after each entry's guarded reads complete.
pub(crate) fn append_entries<T>(
    memory: &impl Memory,
    array: Array,
    stride: usize,
    mut output: EntryOutput<'_, T>,
    length: &mut usize,
    mut decode: impl FnMut(usize) -> Result<T, &'static str>,
    indirect: bool,
) -> Result<[usize; 2], &'static str> {
    let count = array.count as usize;
    let end = length
        .checked_add(count)
        .ok_or("naval detail required capacity overflow")?;
    let span = [*length, count];
    match &mut output {
        EntryOutput::Owned(rows) => {
            rows.truncate(*length);
            crate::allocation::reserve_push_batch(rows, count)
                .map_err(|_| "naval detail allocation failed")?;
        }
        EntryOutput::Slice(rows) if end > rows.len() => return Err(DETAIL_ARENA_CAPACITY),
        EntryOutput::Slice(_) => (),
    }
    for index in 0..array.count {
        let entry = array
            .element(index, stride)
            .ok_or("naval detail nested element overflow")?;
        let entry = if indirect {
            read::<u64>(memory, entry).ok_or("naval detail nested pointer unreadable")? as usize
        } else {
            entry
        };
        let row = decode(entry)?;
        match &mut output {
            EntryOutput::Owned(rows) => rows.push(row),
            EntryOutput::Slice(rows) => rows[*length + index as usize] = row,
        }
    }
    *length = end;
    Ok(span)
}

/// CFEXMember convoy client packed handle u64.
pub const CLIENT_HANDLE: usize = 0xd0;
/// CFEXMember convoy index u32.
pub const CONVOY_INDEX: usize = 0xd8;
/// CFEXMember state u32.
pub const STATE: usize = 0xe0;
/// CFEXMember CFEXGroup pointer.
pub const GROUP: usize = 0xe8;
/// CFEXMember attribution std::string.
pub const ATTRIBUTION_NAME: usize = 0xa8;
/// Engagement weight read as u32; its actual width remains unproven.
pub const ENGAGEMENT_WEIGHT: usize = 0xf8;
/// Damage-entry array pointer with 0x20-byte entries.
pub const DAMAGE_ENTRIES: usize = 0x108;
/// CFEXMember damage-entry count i32.
pub const DAMAGE_ENTRIES_COUNT: usize = 0x114;
/// CFEXMember battle-slot index u32.
pub const BATTLE_SLOT: usize = 0x120;
/// CFEXMember empty state mask.
pub const STATE_UNINITIALIZED: u32 = 0;
/// CFEXMember waiting bit.
pub const STATE_WAITING: u32 = 1;
/// CFEXMember fighting bit.
pub const STATE_FIGHTING: u32 = 2;
/// CFEXMember on-the-way bit.
pub const STATE_ON_THE_WAY: u32 = 4;
/// CFEXMember escaping bit.
pub const STATE_ESCAPING: u32 = 8;
/// CFEXMember run-away bit.
pub const STATE_RUN_AWAY: u32 = 0x10;
/// CFEXMember sunk bit.
pub const STATE_SUNK: u32 = 0x20;
/// CFEXMember sink writer entry.
pub const SINK_WRITER_RVA: usize = 0x0196_1dd0;
/// CFEXMember sink writer bytes.
pub const SINK_WRITER_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48,
];
/// CFEXMember convoy terminal call site.
pub const CONVOY_TERMINAL_CALL_RVA: usize = 0x0196_2378;
/// Instruction after the five-byte convoy-terminal call.
pub const CONVOY_TERMINAL_RESUME_RVA: usize = 0x0196_237d;
/// CFEXMember terminal call bytes.
pub const CONVOY_TERMINAL_CALL_SENTINEL: [u8; 5] = [0xe8, 0x13, 0x8b, 0x86, 0xfe];
/// CFEXMember terminal argument setup start.
pub const CONVOY_TERMINAL_CONTEXT_RVA: usize = 0x0196_2364;
/// CFEXMember terminal argument setup and call.
pub const CONVOY_TERMINAL_CONTEXT_SENTINEL: [u8; 25] = [
    0x4d, 0x8d, 0x4c, 0x24, 0x20, 0x4d, 0x8b, 0xc7, 0xba, 0x01, 0x00, 0x00, 0x00, 0x48, 0x8b, 0x0d,
    0x18, 0x4b, 0x9b, 0x01, 0xe8, 0x13, 0x8b, 0x86, 0xfe,
];
