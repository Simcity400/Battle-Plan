
use crate::{
    Memory,
    allocation_equipment_row,
    cequipment_type,
    cequipment_variant::{self, pool as cequipment_variant_pool},
    cpdx_array::Array,
    craid_instance,
    raid_speed_inputs::{self, Raid},
    read,
    walk::Outcome,
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub reference_type: u32,
    pub id: u32,
    pub phase: i32,
    pub end_hour: Option<i32>,
    pub items_start: usize,
    pub items_len: usize,
    pub archetypes_start: usize,
    pub archetypes_len: usize,
    pub nukes: i32,
}

impl Row {
    pub fn key(self) -> u64 {
        (u64::from(self.id) << 32) | u64::from(self.reference_type)
    }
}

pub struct Arenas<'a> {
    pub variants: &'a mut Vec<u64>,
    pub amounts: &'a mut Vec<i64>,
    pub archetype_tokens: &'a mut Vec<u32>,
    pub archetype_amounts: &'a mut Vec<i64>,
}

pub struct ArenasBuffer<'a> {
    pub variants: crate::walk::Buffer<'a, u64>,
    pub amounts: crate::walk::Buffer<'a, i64>,
    pub archetype_tokens: crate::walk::Buffer<'a, u32>,
    pub archetype_amounts: crate::walk::Buffer<'a, i64>,
}

impl<'a> From<Arenas<'a>> for ArenasBuffer<'a> {
    fn from(value: Arenas<'a>) -> Self {
        Self {
            variants: crate::walk::Buffer::Growing(value.variants),
            amounts: crate::walk::Buffer::Growing(value.amounts),
            archetype_tokens: crate::walk::Buffer::Growing(value.archetype_tokens),
            archetype_amounts: crate::walk::Buffer::Growing(value.archetype_amounts),
        }
    }
}

pub struct Walk {
    pub outcome: Outcome,
    pub items_len: usize,
    pub required_items: usize,
    pub archetypes_len: usize,
    pub required_archetypes: usize,
}

pub fn recorded(raid: &Raid, hour: i32) -> bool {
    (craid_instance::PHASE_ASSEMBLING..=craid_instance::PHASE_PREPARED).contains(&raid.phase)
        || raid_speed_inputs::live(raid.phase, raid.outcome, raid.end_hour, hour)
}

const HEADERS: usize = cequipment_variant_pool::COUNT + 4 - cequipment_variant_pool::ALLOCATIONS;

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut Vec<Row>,
    arenas: Arenas<'_>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(rows),
        arenas.into(),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut rows: crate::walk::Buffer<'_, Row>,
    arenas: ArenasBuffer<'_>,
) -> Walk {
    let mut walk = Walk {
        outcome: Outcome::Rows { len: 0 },
        items_len: 0,
        required_items: 0,
        archetypes_len: 0,
        required_archetypes: 0,
    };
    let ArenasBuffer {
        mut variants,
        mut amounts,
        mut archetype_tokens,
        mut archetype_amounts,
    } = arenas;
    let archetype_vtable =
        engine_sites::at(base, cequipment_type::VTABLE_RVA).map(|address| address as u64);
    let (mut len, mut unread) = (0_usize, 0_u32);
    let result = raid_speed_inputs::each_raid(memory, base, game_state, |hour, raid| {
        if !recorded(&raid, hour) {
            return Ok(());
        }
        let archetype_vtable = archetype_vtable.ok_or("module address overflow")?;
        let pool = raid
            .pointer
            .checked_add(craid_instance::EQUIPMENT)
            .ok_or("raid pool address overflow")?;
        let headers = pool
            .checked_add(cequipment_variant_pool::ALLOCATIONS)
            .and_then(|at| read::<[u8; HEADERS]>(memory, at))
            .ok_or("raid pool headers unreadable")?;
        let header = |field: usize| -> [u8; 16] {
            let at = field - cequipment_variant_pool::ALLOCATIONS;
            headers[at..at + 16].try_into().unwrap()
        };
        let items = Array::from_header(header(cequipment_variant_pool::ITEMS), 16)
            .ok_or("raid pool header invalid")?;
        let archetypes = Array::from_header(
            header(cequipment_variant_pool::ALLOCATIONS),
            allocation_equipment_row::STRIDE,
        )
        .ok_or("raid pool archetype header invalid")?;
        let nukes = raid
            .pointer
            .checked_add(craid_instance::NUKES)
            .and_then(|at| read::<i32>(memory, at))
            .ok_or("raid nuke count unreadable")?;
        let count = items.count as usize;
        let archetype_count = archetypes.count as usize;
        walk.required_items += count;
        walk.required_archetypes += archetype_count;
        let start = walk.items_len;
        let archetype_start = walk.archetypes_len;
        variants.ensure(walk.required_items)?;
        amounts.ensure(walk.required_items)?;
        archetype_tokens.ensure(walk.required_archetypes)?;
        archetype_amounts.ensure(walk.required_archetypes)?;
        rows.ensure(
            len.checked_add(1)
                .ok_or("raid equipment row count overflow")?,
        )?;
        let capacity = variants.len().min(amounts.len());
        let archetype_capacity = archetype_tokens.len().min(archetype_amounts.len());
        let fits = capacity - start >= count
            && archetype_capacity - archetype_start >= archetype_count
            && len < rows.len();
        if fits {
            for index in 0..items.count {
                let entry = items
                    .element(index, 16)
                    .and_then(|at| read::<[u8; 16]>(memory, at))
                    .ok_or("raid pool entry unreadable")?;
                let pointer = u64::from_le_bytes(entry[..8].try_into().unwrap()) as usize;
                let variant = if pointer == 0 {
                    0
                } else {
                    cequipment_variant::identity(memory, base, pointer)?
                };
                let at = start + index as usize;
                variants[at] = variant;
                amounts[at] = i64::from_le_bytes(entry[8..].try_into().unwrap());
            }
            for index in 0..archetypes.count {
                let entry = archetypes
                    .element(index, allocation_equipment_row::STRIDE)
                    .and_then(|at| read::<[u8; allocation_equipment_row::STRIDE]>(memory, at))
                    .ok_or("raid pool archetype total unreadable")?;
                let archetype = allocation_equipment_row::ARCHETYPE;
                let pointer =
                    u64::from_le_bytes(entry[archetype..archetype + 8].try_into().unwrap());
                let identity = usize::try_from(pointer)
                    .ok()
                    .filter(|&pointer| pointer != 0)
                    .and_then(|pointer| read::<[u8; 12]>(memory, pointer))
                    .ok_or("raid pool archetype null or unreadable")?;
                if u64::from_le_bytes(identity[..8].try_into().unwrap()) != archetype_vtable {
                    return Err("raid pool archetype vtable mismatch");
                }
                let at = archetype_start + index as usize;
                archetype_tokens[at] = u32::from_le_bytes(
                    identity[cequipment_type::TOKEN..cequipment_type::TOKEN + 4]
                        .try_into()
                        .unwrap(),
                );
                archetype_amounts[at] = i64::from_le_bytes(
                    entry[allocation_equipment_row::AMOUNT..allocation_equipment_row::STRIDE]
                        .try_into()
                        .unwrap(),
                );
            }
            rows[len] = Row {
                reference_type: raid.reference_type,
                id: raid.id,
                phase: raid.phase,
                end_hour: raid.end_hour,
                items_start: start,
                items_len: count,
                archetypes_start: archetype_start,
                archetypes_len: archetype_count,
                nukes,
            };
            walk.items_len += count;
            walk.archetypes_len += archetype_count;
            len += 1;
        } else {
            unread += 1;
        }
        Ok(())
    });
    walk.outcome = match result {
        Err(reason) => Outcome::Unavailable { reason },
        Ok(()) if unread == 0 => Outcome::Rows { len },
        Ok(()) => Outcome::Truncated {
            len,
            unread_rows: unread,
        },
    };
    walk
}
