use engine_native_memory::{
    Memory,
    capture_text::Value,
    cpdx_array::Array,
    memory::{FieldBatch, Raw},
};
use engine_native::{
    cfex_member::{
        ARRIVAL_HOURS,
        ATTRIBUTION_NAME,
        BATTLE_SLOT,
        BUILD_COST_IC,
        CACHED_STRENGTH,
        CONVOY_HANDLE,
        DAMAGE_ENTRIES,
        DETECTION_STATE,
        ESCAPE_PROGRESS,
        EVACUATION_STATE,
        GROUP,
        OWNER,
        RECEIVED_CRITICALS,
        RECEIVED_DAMAGE,
        SHIP_HANDLE,
        STATE,
        EntryOutput,
        append_entries,
    },
    cnaval_combatant,
    cship,
};
use engine_native_parts::creference_registry;

pub struct DetailBuffers<'a> {
    pub damage: &'a mut Vec<engine_native::cfex_damage_entry::Row>,
    pub air_hits: &'a mut Vec<crate::naval_combat_air_hit::Row>,
    pub naval_hits: &'a mut Vec<crate::naval_combat_hit::Row>,
    pub lengths: [usize; 3],
    pub required: [usize; 3],
    pub planes: engine_native_memory::walk::Buffer<'a, crate::naval_air_details::Row>,
    pub planes_len: usize,
    pub planes_required: usize,
    pub plane_air_hits: engine_native_memory::walk::Buffer<'a, crate::naval_combat_air_hit::Row>,
    pub plane_naval_hits: engine_native_memory::walk::Buffer<'a, crate::naval_combat_hit::Row>,
    pub plane_hit_lengths: [usize; 2],
    pub plane_hit_required: [usize; 2],
    pub plane_kills: engine_native_memory::walk::Buffer<'a, crate::naval_air_efficiency_inputs::WingKill>,
    pub plane_kills_len: usize,
    pub plane_kills_required: usize,
    pub weapon_sources: &'a mut Vec<crate::naval_ship_modifier_inputs::SourceRow>,
    pub weapon_terms: &'a mut Vec<crate::naval_ship_modifier_inputs::ModifierTerm>,
    pub weapon_lengths: [usize; 2],
    pub weapon_required: [usize; 2],
    pub doctrine_keys: engine_native::cship::modifier_sources::DoctrineKeys,
    pub term_metadata: crate::naval_ship_modifier_inputs::TermMetadata,
    pub source_memo: Option<crate::naval_ship_modifier_inputs::SourceMemo>,
    pub commander_inputs: engine_native_memory::walk::Buffer<'a, crate::naval_commander_inputs::Row>,
    pub commander_inputs_len: usize,
    pub commander_inputs_required: usize,
    pub commander_timed_traits: &'a mut Vec<engine_native::cunit_leader::TimedTrait>,
    pub commander_timed_traits_len: usize,
    pub side_units: engine_native_memory::walk::Buffer<'a, u64>,
    pub side_units_len: usize,
    pub side_units_required: usize,
    pub summary_countries: &'a mut Vec<u32>,
    pub summary_countries_len: usize,
    pub custom_sea: engine_native_memory::walk::Buffer<'a, i16>,
    pub custom_sea_len: usize,
    pub custom_sea_required: usize,
    pub damage_sources: engine_native_memory::walk::Buffer<'a, (u32, i64)>,
    pub damage_sources_len: usize,
    pub damage_sources_required: usize,
    pub wing_types: engine_native_memory::walk::Buffer<'a, u32>,
    pub wing_types_len: usize,
    pub wing_types_required: usize,
    pub plane_wings: engine_native_memory::walk::Buffer<'a, u64>,
    pub plane_wings_len: usize,
    pub plane_wings_required: usize,
}


const MEMBER_FIELDS: [(usize, usize); 17] = [
    (DAMAGE_ENTRIES, 16),
    (0x150, 16),
    (0x168, 16),
    (0x180, 16),
    (CONVOY_HANDLE, 8),
    (0x24, 4),
    (0x28, 4),
    (BUILD_COST_IC, 8),
    (CACHED_STRENGTH, 8),
    (0x40, 1),
    (0x41, 1),
    (0xc8, 8),
    (ARRIVAL_HOURS, 4),
    (RECEIVED_CRITICALS, 4),
    (EVACUATION_STATE, 4),
    (DETECTION_STATE, 4),
    (ESCAPE_PROGRESS, 8),
];
const MEMBER_RECORD: [(usize, usize); 31] = [
    (0, 8),
    (SHIP_HANDLE, 8),
    (CONVOY_HANDLE, 8),
    (OWNER, 4),
    (0x24, 4),
    (0x28, 4),
    (BUILD_COST_IC, 8),
    (CACHED_STRENGTH, 8),
    (0x40, 1),
    (0x41, 1),
    (0x48, 32),
    (0x68, 32),
    (0x88, 32),
    (ATTRIBUTION_NAME, 32),
    (0xc8, 8),
    (STATE, 4),
    (ARRIVAL_HOURS, 4),
    (GROUP, 8),
    (DAMAGE_ENTRIES, 16),
    (BATTLE_SLOT, 4),
    (RECEIVED_CRITICALS, 4),
    (EVACUATION_STATE, 4),
    (DETECTION_STATE, 4),
    (ESCAPE_PROGRESS, 8),
    (0x138, 8),
    (0x140, 1),
    (0x144, 4),
    (0x148, 1),
    (0x150, 16),
    (0x168, 16),
    (0x180, 16),
];

const MEMBER_FIELDS_END: usize = 0x190;

fn member_field<T: Raw>(
    memory: &impl Memory,
    pointer: usize,
    fields: &Option<FieldBatch<17>>,
    index: usize,
) -> Result<T, &'static str> {
    match fields {
        Some(fields) => fields.get(memory, index).ok_or("naval field unreadable"),
        None => cnaval_combatant::field(memory, pointer, MEMBER_FIELDS[index].0),
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DetailsRow {
    pub native_group: usize,
    pub member: cnaval_combatant::Row,
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
    pub name: engine_native_memory::capture_text::Owned,
    pub type_name: engine_native_memory::capture_text::Owned,
    pub sprite: engine_native_memory::capture_text::Owned,
    pub sunk_by: engine_native_memory::capture_text::Owned,
    pub variant: u64,
    pub arrival_hours: i32,
    pub received_criticals: i32,
    pub evacuation_state: i32,
    pub detection_state: i32,
    pub escape_progress: i64,
    pub escape_speed_base: i64,
    pub last_target: crate::clast_target_info::Row,
    pub received_damage: [i64; 6],
    pub live_ship: u8,
    pub live_convoy: u8,
    pub strength: i64,
    pub organisation: i64,
    pub base_organisation: i64,
    pub maximum_hit_points: i64,
    pub formation_flags: u64,
    pub weapon_context: crate::naval_ship_modifier_inputs::ContextRow,
    pub damage_entries: [usize; 2],
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

const COMBAT_SHIP_FIELDS: [(usize, usize); 18] = [
    (0, 16),
    (cship::EQUIPMENT_POOL, 8),
    (
        cship::EQUIPMENT_POOL + engine_native::cequipment_variant::pool::ITEMS,
        16,
    ),
    (0x88, 4),
    (0x1b8, 8),
    (0x1e0, 16),
    (0x1f8, 32),
    (0x228, 8),
    (0x270, 8),
    (0x300, 16),
    (0x318, 8),
    (0x60c, 4),
    (cship::FORMATION_FLAGS, 8),
    (cship::STRENGTH, 16),
    (cship::TASK_FORCE, 16),
    (0x750, 16),
    (0x8f8, 8),
    (0x918, 16),
];

pub static MEMBER_PHASES: [std::sync::atomic::AtomicU64; 4] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 4];

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
    let started = engine_native_memory::clock::qpc();
    let row = details_timed(
        memory, base, pointer, group, combat, side, game_state, buffers,
    );
    MEMBER_PHASES[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    MEMBER_PHASES[3].fetch_add(
        engine_native_memory::clock::since(started),
        std::sync::atomic::Ordering::Relaxed,
    );
    row
}

#[allow(clippy::too_many_arguments)]
fn details_timed(
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
    let record = cship::GatheredObject::gather(memory, pointer, MEMBER_RECORD);
    if !record.complete() {
        engine_native_memory::batch_fallback::record(engine_native_memory::batch_fallback::Batch::NavalMemberRecords);
    }
    let source = memory;
    let memory = &record.memory(source);
    let member = cnaval_combatant::member(memory, base, pointer, group, combat, side)?;
    let fields = pointer
        .checked_add(MEMBER_FIELDS_END)
        .map(|_| FieldBatch::gather(memory, MEMBER_FIELDS.map(|(at, len)| (pointer + at, len))));
    let mut row_counts = [0usize; 3];
    let mut arrays = [Array { data: 0, count: 0 }; 4];
    for (index, (header, (stride, lane))) in arrays
        .iter_mut()
        .zip([(0x20, 0), (8, 1), (8, 1), (8, 2)])
        .enumerate()
    {
        let array = match &fields {
            Some(fields) => fields
                .get::<[u8; 16]>(memory, index)
                .and_then(|header| Array::from_header(header, stride)),
            None => Array::read(
                memory,
                pointer
                    .checked_add(MEMBER_FIELDS[index].0)
                    .ok_or("naval detail nested header overflow")?,
                stride,
            ),
        }
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
        engine_native_memory::capture_text::Owned::read(
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
        convoy: member_field(memory, pointer, &fields, 4)?,
        subunit_index: member_field(memory, pointer, &fields, 5)?,
        formation_type: member_field(memory, pointer, &fields, 6)?,
        build_cost_ic: member_field(memory, pointer, &fields, 7)?,
        cached_strength_ratio: member_field(memory, pointer, &fields, 8)?,
        pride: member_field(memory, pointer, &fields, 9)?,
        is_convoy: member_field(memory, pointer, &fields, 10)?,
        name: text(0x48)?,
        type_name: text(0x68)?,
        sprite: text(0x88)?,
        sunk_by: text(ATTRIBUTION_NAME)?,
        variant: member_field(memory, pointer, &fields, 11)?,
        arrival_hours: member_field(memory, pointer, &fields, 12)?,
        received_criticals: member_field(memory, pointer, &fields, 13)?,
        evacuation_state: member_field(memory, pointer, &fields, 14)?,
        detection_state: member_field(memory, pointer, &fields, 15)?,
        escape_progress: member_field(memory, pointer, &fields, 16)?,
        last_target: crate::clast_target_info::read(
            memory,
            base,
            pointer
                .checked_add(0x138)
                .ok_or("naval last-target overflow")?,
        )?,
        ..DetailsRow::default()
    };
    let memory = source;
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
        |memory, p| engine_native::cfex_damage_entry::read(memory, base, p),
        None,
    )?;
    for (index, array) in arrays[1..3].iter().copied().enumerate() {
        row.air_hits[index] = append_entries(
            memory,
            array,
            8,
            EntryOutput::Owned(buffers.air_hits),
            &mut buffers.lengths[1],
            |memory, p| crate::naval_combat_air_hit::read(memory, base, p),
            Some(&crate::naval_combat_air_hit::RECORD),
        )?;
    }
    row.naval_hits = append_entries(
        memory,
        arrays[3],
        8,
        EntryOutput::Owned(buffers.naval_hits),
        &mut buffers.lengths[2],
        |memory, p| crate::naval_combat_hit::read(memory, base, p),
        Some(&crate::naval_combat_hit::RECORD),
    )?;
    if let Some(ship) = creference_registry::resolve(memory, base, member.ship)? {
        let fields = engine_native::cship::GatheredObject::gather(memory, ship, COMBAT_SHIP_FIELDS);
        let memory = &fields.memory(memory);
        if Some(field::<u64>(memory, ship, 0)?)
            != engine_sites::at(base, cship::VTABLE_RVA).map(|address| address as u64)
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
        let side = field::<u64>(memory, group, engine_native_parts::cfex_group::SIDE)? as usize;
        let started = engine_native_memory::clock::qpc();
        let context =
            crate::naval_ship_modifier_inputs::read(memory, base, game_state, ship, side, buffers);
        MEMBER_PHASES[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        MEMBER_PHASES[2].fetch_add(
            engine_native_memory::clock::since(started),
            std::sync::atomic::Ordering::Relaxed,
        );
        row.weapon_context = context?;
    }
    if row.live_ship == 0
        && let Some(convoy) = creference_registry::resolve(memory, base, row.convoy)?
    {
        let values =
            engine_native::cconvoy_ship::read(memory, base, convoy, row.convoy, row.member.state == 8)?;
        row.live_convoy = 1;
        row.escape_speed_base = values.speed;
        row.strength = values.strength;
        row.organisation = values.organisation;
        row.maximum_hit_points = values.maximum_strength;
        row.base_organisation = values.maximum_organisation;
    }
    Ok(row)
}
