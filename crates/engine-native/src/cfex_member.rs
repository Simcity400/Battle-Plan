
use engine_sites::Rva;
use crate::capture_text::Value;
use crate::{
    Memory, cnaval_combatant,
    cpdx_array::Array,
    creference_registry, cship,
    memory::{FieldBatch, Raw},
    read,
};

pub const VTABLE_RVA: Rva = Rva(0x02a9_a258);
pub const SHIP_HANDLE: usize = 0x08;
pub const CONVOY_HANDLE: usize = 0x10;
pub const OWNER: usize = 0x20;
pub const BUILD_COST_IC: usize = 0x30;
pub const CACHED_STRENGTH: usize = 0x38;
pub const ARRIVAL_HOURS: usize = 0xe4;
pub const RECEIVED_CRITICALS: usize = 0x124;
pub const EVACUATION_STATE: usize = 0x128;
pub const DETECTION_STATE: usize = 0x12c;
pub const ESCAPE_PROGRESS: usize = 0x130;
pub const RECEIVED_DAMAGE: usize = 0x198;

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
    pub doctrine_keys: crate::naval_ship_modifier_inputs::DoctrineKeys,
    pub term_metadata: crate::naval_ship_modifier_inputs::TermMetadata,
    pub source_memo: Option<crate::naval_ship_modifier_inputs::SourceMemo>,
    pub commander_inputs: crate::walk::Buffer<'a, crate::naval_commander_inputs::Row>,
    pub commander_inputs_len: usize,
    pub commander_inputs_required: usize,
    pub commander_timed_traits: &'a mut Vec<crate::cunit_leader::TimedTrait>,
    pub commander_timed_traits_len: usize,
    pub side_units: crate::walk::Buffer<'a, u64>,
    pub side_units_len: usize,
    pub side_units_required: usize,
    pub summary_countries: &'a mut Vec<u32>,
    pub summary_countries_len: usize,
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
        cship::EQUIPMENT_POOL + crate::cequipment_variant_pool::ITEMS,
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

#[allow(
    clippy::too_many_arguments,
    reason = "naval combatant reader threads combat context and shared detail buffers"
)]
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
    let started = crate::clock::qpc();
    let row = details_timed(
        memory, base, pointer, group, combat, side, game_state, buffers,
    );
    MEMBER_PHASES[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    MEMBER_PHASES[3].fetch_add(
        crate::clock::since(started),
        std::sync::atomic::Ordering::Relaxed,
    );
    row
}

#[allow(
    clippy::too_many_arguments,
    reason = "naval combatant reader threads combat context and shared detail buffers"
)]
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
        crate::batch_fallback::record(crate::batch_fallback::Batch::NavalMemberRecords);
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
        |memory, p| crate::cfex_damage_entry::read(memory, base, p),
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
        let fields = crate::cship::GatheredObject::gather(memory, ship, COMBAT_SHIP_FIELDS);
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
        let side = field::<u64>(memory, group, crate::cfex_group::SIDE)? as usize;
        let started = crate::clock::qpc();
        let context =
            crate::naval_ship_modifier_inputs::read(memory, base, game_state, ship, side, buffers);
        MEMBER_PHASES[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        MEMBER_PHASES[2].fetch_add(
            crate::clock::since(started),
            std::sync::atomic::Ordering::Relaxed,
        );
        row.weapon_context = context?;
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

pub enum EntryOutput<'a, T> {
    Owned(&'a mut Vec<T>),
    Slice(&'a mut [T]),
}

const ENTRY_RUN: usize = 64;
const ENTRY_STRIDE: usize = 0x20;
const ENTRY_RECORD: usize = 64;

pub struct EntryRun<'a, M> {
    source: &'a M,
    start: usize,
    bytes: &'a [u8],
}

impl<M: Memory> Memory for EntryRun<'_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if let Some(offset) = address.checked_sub(self.start)
            && offset
                .checked_add(destination.len())
                .is_some_and(|end| end <= self.bytes.len())
        {
            destination.copy_from_slice(&self.bytes[offset..offset + destination.len()]);
            return true;
        }
        self.source.copy(address, destination)
    }

    fn prefetch(&self, address: usize, len: usize) {
        self.source.prefetch(address, len);
    }

    fn authenticated_countries(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<crate::ccountry::AuthenticatedRoster<'_>> {
        self.source.authenticated_countries(base, game_state)
    }

    fn modifier_definitions(&self) -> Option<&crate::modifier_definition::PassDefinitions> {
        self.source.modifier_definitions()
    }
}

pub struct EntryRecord<'a> {
    pointer: usize,
    fields: &'a [crate::memory::RecordField],
    record: &'a [u8],
}

impl cship::Served for EntryRecord<'_> {
    fn lookup(&self, address: usize, len: usize) -> cship::Lookup<'_> {
        if self.pointer == 0 || len == 0 || self.record.is_empty() {
            return cship::Lookup::Source;
        }
        let Some(offset) = address.checked_sub(self.pointer) else {
            return cship::Lookup::Source;
        };
        for field in self.fields {
            if offset >= field.source_offset
                && offset
                    .checked_add(len)
                    .is_some_and(|end| end <= field.source_offset + field.len)
            {
                let at = field.destination_offset + offset - field.source_offset;
                return cship::Lookup::Served(&self.record[at..at + len]);
            }
        }
        cship::Lookup::Source
    }
}

pub(crate) type EntryMemory<'a, M> = cship::GatheredMemory<'a, EntryRun<'a, M>, EntryRecord<'a>>;

pub fn append_entries<M: Memory, T>(
    memory: &M,
    array: Array,
    stride: usize,
    mut output: EntryOutput<'_, T>,
    length: &mut usize,
    mut decode: impl FnMut(&EntryMemory<'_, M>, usize) -> Result<T, &'static str>,
    object: Option<&'static [crate::memory::RecordField]>,
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
    let fields = object.unwrap_or(&[]);
    let row_bytes = fields
        .iter()
        .map(|field| field.destination_offset + field.len)
        .max()
        .unwrap_or(0);
    let layout = (row_bytes != 0 && row_bytes <= ENTRY_RECORD)
        .then(|| crate::memory::RecordLayout::new(fields, row_bytes))
        .flatten();
    let mut elements = [0_u8; ENTRY_RUN * ENTRY_STRIDE];
    let mut records = [0_u8; ENTRY_RUN * ENTRY_RECORD];
    let mut pointers = [0_usize; ENTRY_RUN];
    let mut first = 0_u32;
    while first < array.count {
        let run = ((array.count - first) as usize).min(ENTRY_RUN);
        let start = array
            .element(first, stride)
            .ok_or("naval detail nested element overflow")?;
        let copied = stride <= ENTRY_STRIDE && memory.copy(start, &mut elements[..run * stride]);
        let elements = EntryRun {
            source: memory,
            start,
            bytes: if copied {
                &elements[..run * stride]
            } else {
                &[]
            },
        };
        let mut complete = 0;
        if copied
            && object.is_some()
            && let Some(layout) = &layout
        {
            for (index, pointer) in pointers[..run].iter_mut().enumerate() {
                let at = index * stride;
                *pointer = u64::from_le_bytes(
                    elements.bytes[at..at + 8]
                        .try_into()
                        .expect("pointer element"),
                ) as usize;
            }
            complete = memory.copy_records(
                crate::memory::RecordSources::Pointers(&pointers[..run]),
                layout,
                &mut records[..run * row_bytes],
            ) / fields.len().max(1);
        }
        if !copied || (object.is_some() && complete < run) {
            crate::batch_fallback::record(crate::batch_fallback::Batch::NavalEntryRuns);
        }
        for offset in 0..run {
            let index = first + offset as u32;
            let entry = array
                .element(index, stride)
                .ok_or("naval detail nested element overflow")?;
            let entry = if object.is_some() {
                read::<u64>(&elements, entry).ok_or("naval detail nested pointer unreadable")?
                    as usize
            } else {
                entry
            };
            let record = EntryRecord {
                pointer: entry,
                fields,
                record: if offset < complete {
                    &records[offset * row_bytes..(offset + 1) * row_bytes]
                } else {
                    &[]
                },
            };
            let row = decode(&cship::GatheredMemory::new(&elements, &record), entry)?;
            match &mut output {
                EntryOutput::Owned(rows) => rows.push(row),
                EntryOutput::Slice(rows) => rows[*length + index as usize] = row,
            }
        }
        first += run as u32;
    }
    *length = end;
    Ok(span)
}

pub const STATE: usize = 0xe0;
pub const GROUP: usize = 0xe8;
pub const ATTRIBUTION_NAME: usize = 0xa8;
pub const DAMAGE_ENTRIES: usize = 0x108;
pub const BATTLE_SLOT: usize = 0x120;

#[cfg(test)]
mod serial_reference;

#[cfg(test)]
mod entry_run_tests;
