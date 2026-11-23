use engine_native_memory::{Memory, cpdx_array::Array};
use engine_native::cnaval_combatant::field;

use engine_native::cfex_air::VTABLE_RVA;


#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Row {
    pub wing_types: [usize; 2],
    pub source: u8,
    pub group_index: u32,
    pub index: u32,
    pub carrier_slot: i32,
    pub carrier_member_present: u8,
    pub carrier_state: u32,
    pub air_base: u64,
    pub wings: [[usize; 2]; 2],
    pub initial_planes: i32,
    pub current_planes: i32,
    pub casualties: i32,
    pub time_duration: i32,
    pub country: u32,
    pub date: i32,
    pub flags: [u8; 2],
    pub variant: u64,
    pub naval_hits: [usize; 2],
    pub air_hits: [[usize; 2]; 2],
    pub kills: [usize; 2],
    pub efficiency: crate::naval_air_efficiency_inputs::Row,
}

pub(crate) fn read_hits(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    row: &mut Row,
    buffers: &mut crate::naval_member_details::DetailBuffers<'_>,
) -> Result<(), &'static str> {
    use engine_native::cfex_member::{EntryOutput, append_entries};
    use engine_native::cfex_member::DETAIL_ARENA_CAPACITY;
    let mut counts = [0usize; 2];
    let mut arrays = [Array::default(); 3];
    for (header, (offset, lane)) in arrays.iter_mut().zip([(0x100, 1), (0x118, 0), (0x130, 0)]) {
        let array = Array::read(
            memory,
            pointer
                .checked_add(offset)
                .ok_or("naval plane hit array overflow")?,
            8,
        )
        .ok_or("naval plane hit array invalid")?;
        counts[lane] += array.count as usize;
        *header = array;
    }
    buffers.plane_air_hits.ensure(
        buffers.plane_hit_lengths[0]
            .checked_add(counts[0])
            .ok_or("naval plane hit length overflow")?,
    )?;
    buffers.plane_naval_hits.ensure(
        buffers.plane_hit_lengths[1]
            .checked_add(counts[1])
            .ok_or("naval plane hit length overflow")?,
    )?;
    let capacity = [buffers.plane_air_hits.len(), buffers.plane_naval_hits.len()];
    let mut fits = true;
    for lane in 0..2 {
        buffers.plane_hit_required[lane] = buffers.plane_hit_required[lane]
            .checked_add(counts[lane])
            .ok_or("naval plane hit required overflow")?;
        fits &= buffers.plane_hit_lengths[lane]
            .checked_add(counts[lane])
            .is_some_and(|n| n <= capacity[lane]);
    }
    if !fits {
        return Err(DETAIL_ARENA_CAPACITY);
    }
    row.naval_hits = append_entries(
        memory,
        arrays[0],
        8,
        EntryOutput::Slice(&mut buffers.plane_naval_hits),
        &mut buffers.plane_hit_lengths[1],
        |memory, p| crate::naval_combat_hit::read(memory, base, p),
        Some(&crate::naval_combat_hit::RECORD),
    )?;
    for (index, array) in arrays[1..].iter().copied().enumerate() {
        row.air_hits[index] = append_entries(
            memory,
            array,
            8,
            EntryOutput::Slice(&mut buffers.plane_air_hits),
            &mut buffers.plane_hit_lengths[0],
            |memory, p| crate::naval_combat_air_hit::read(memory, base, p),
            Some(&crate::naval_combat_air_hit::RECORD),
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn read(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    group: usize,
    wing_types: &mut Vec<u32>,
    wing_types_len: &mut usize,
    wing_types_required: &mut usize,
    plane_wings: &mut Vec<u64>,
    plane_wings_len: &mut usize,
    plane_wings_required: &mut usize,
) -> Result<Row, &'static str> {
    read_buffer(
        memory,
        base,
        pointer,
        group,
        engine_native_memory::walk::Buffer::Growing(wing_types),
        wing_types_len,
        wing_types_required,
        engine_native_memory::walk::Buffer::Growing(plane_wings),
        plane_wings_len,
        plane_wings_required,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn read_buffer(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    group: usize,
    mut wing_types: engine_native_memory::walk::Buffer<'_, u32>,
    wing_types_len: &mut usize,
    wing_types_required: &mut usize,
    mut plane_wings: engine_native_memory::walk::Buffer<'_, u64>,
    plane_wings_len: &mut usize,
    plane_wings_required: &mut usize,
) -> Result<Row, &'static str> {
    if field::<u64>(memory, pointer, 0)?
        != engine_sites::at(base, VTABLE_RVA)
            .ok_or("naval plane module overflow")? as u64
        || field::<u64>(memory, pointer, 8)? != group as u64
    {
        return Err("naval plane identity or group mismatch");
    }
    let carrier = field::<u64>(memory, pointer, 0x10)? as usize;
    let carrier_slot = field(memory, pointer, 0x18)?;
    let carrier_state = if carrier == 0 {
        0
    } else {
        if field::<u64>(memory, carrier, 0)?
            != engine_sites::at(base, engine_native::cfex_member::VTABLE_RVA)
                .ok_or("naval plane carrier module overflow")? as u64
            || field::<i32>(memory, carrier, engine_native::cfex_member::BATTLE_SLOT)? != carrier_slot
            || field::<u64>(memory, carrier, engine_native::cfex_member::GROUP)? != group as u64
        {
            return Err("naval plane carrier identity mismatch");
        }
        field(memory, carrier, engine_native::cfex_member::STATE)?
    };
    let mut row = Row {
        carrier_slot,
        carrier_member_present: u8::from(carrier != 0),
        carrier_state,
        air_base: field(memory, pointer, 0x1c)?,
        initial_planes: field(memory, pointer, 0x58)?,
        current_planes: field(memory, pointer, 0x5c)?,
        casualties: field(memory, pointer, 0x60)?,
        time_duration: field(memory, pointer, 0xf8)?,
        country: field(memory, pointer, 0x64)?,
        date: field(memory, pointer, 0x70)?,
        flags: [field(memory, pointer, 0x80)?, field(memory, pointer, 0x81)?],
        ..Row::default()
    };
    let variant = field::<u64>(memory, pointer, 0xf0)? as usize;
    if variant != 0 {
        if field::<u64>(memory, variant, 0)?
            != engine_sites::at(base, engine_native_parts::equipment_variant_identity::VTABLE_RVA)
                .ok_or("naval plane variant module overflow")? as u64
        {
            return Err("naval plane variant identity mismatch");
        }
        row.variant = field(memory, variant, 8)?;
    }
    for (index, offset) in [0x28, 0x40].into_iter().enumerate() {
        let address = pointer
            .checked_add(offset)
            .ok_or("naval plane wing array overflow")?;
        let array = Array::read(memory, address, 8).ok_or("naval plane wing array invalid")?;
        let start = *plane_wings_len;
        let end = start
            .checked_add(array.count as usize)
            .ok_or("naval plane wing arena length overflow")?;
        *plane_wings_required = (*plane_wings_required).max(end);
        plane_wings.ensure(end)?;
        let output = plane_wings
            .get_mut(start..end)
            .ok_or("naval plane wing arena capacity exceeded")?;
        for (i, slot) in output.iter_mut().enumerate() {
            let reference = engine_native_memory::read(
                memory,
                array
                    .element(i as u32, 8)
                    .ok_or("naval plane wing element overflow")?,
            )
            .ok_or("naval plane wing unreadable")?;
            *slot = reference;
        }
        row.wings[index] = [start, array.count as usize];
        *plane_wings_len = end;
    }
    let start = *wing_types_len;
    let count = row.wings[1][1];
    let end = start
        .checked_add(count)
        .ok_or("naval air wing type arena length overflow")?;
    *wing_types_required = (*wing_types_required).max(end);
    wing_types.ensure(end)?;
    let output = wing_types
        .get_mut(start..end)
        .ok_or("naval air wing type arena capacity exceeded")?;
    for (&handle, slot) in plane_wings[row.wings[1][0]..row.wings[1][0] + count]
        .iter()
        .zip(output)
    {
        *slot = match engine_native_parts::creference_registry::resolve(memory, base, handle)? {
            None => u32::MAX,
            Some(secondary) => {
                let wing = secondary
                    .checked_sub(engine_native_parts::cair_wing_layout::SECONDARY)
                    .ok_or("naval air wing primary address underflow")?;
                if field::<u64>(memory, wing, 0)?
                    != engine_sites::at(base, engine_native_parts::cair_wing_layout::VTABLE_RVA)
                        .ok_or("naval air wing module overflow")? as u64
                    || field::<u64>(memory, secondary, 0)?
                        != engine_sites::at(base, engine_native_parts::cair_wing_layout::SECONDARY_VTABLE_RVA)
                            .ok_or("naval air wing secondary module overflow")?
                            as u64
                    || field::<u64>(memory, wing, engine_native_parts::cair_wing_layout::REFERENCE_TYPE)? != handle
                {
                    return Err("naval air wing identity mismatch");
                }
                let pool = field::<u64>(memory, wing, engine_native_parts::cair_wing_layout::POOL)? as usize;
                if field::<u64>(memory, pool, 0)?
                    != engine_sites::at(base, engine_native_parts::cair_wing_pool::VTABLE_RVA)
                        .ok_or("naval air wing pool module overflow")? as u64
                {
                    return Err("naval air wing pool identity mismatch");
                }
                let definition =
                    field::<u64>(memory, pool, engine_native_parts::cair_wing_pool::DEFINITION)? as usize;
                field(
                    memory,
                    definition,
                    engine_native_parts::csub_unit_definition::DATABASE_INDEX,
                )?
            }
        };
    }
    *wing_types_len = end;
    row.wing_types = [start, count];
    Ok(row)
}

#[cfg(test)]
mod presentation_cache_tests {
    use super::*;

    #[test]
    fn active_air_skips_derived_hover_text_and_grows_its_hits() {
        const ENTRY: usize = 0x100;
        struct Guard(Vec<u8>);
        impl Memory for Guard {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                assert!(address >= ENTRY + 0xf0 || address + output.len() <= ENTRY + 0x88);
                engine_native_memory::memory::SliceMemory {
                    base: 0,
                    bytes: &self.0,
                }
                .copy(address, output)
            }
        }
        let mut bytes = vec![0; 0x400];
        bytes[ENTRY..ENTRY + 8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[ENTRY + 0x88..ENTRY + 0xc0].fill(0xff);
        bytes[ENTRY + 0xc0..ENTRY + 0xf0].fill(0xff);
        let mut memory = Guard(bytes);
        let mut buffers = crate::naval_member_details::DetailBuffers {
            damage: &mut Vec::new(),
            air_hits: &mut Vec::new(),
            naval_hits: &mut Vec::new(),
            lengths: [0; 3],
            required: [0; 3],
            planes: engine_native_memory::walk::Buffer::Count,
            planes_len: 0,
            planes_required: 0,
            plane_air_hits: engine_native_memory::walk::Buffer::Count,
            plane_naval_hits: engine_native_memory::walk::Buffer::Count,
            plane_hit_lengths: [0; 2],
            plane_hit_required: [0; 2],
            plane_kills: engine_native_memory::walk::Buffer::Count,
            plane_kills_len: 0,
            plane_kills_required: 0,
            weapon_sources: &mut Vec::new(),
            weapon_terms: &mut Vec::new(),
            weapon_lengths: [0; 2],
            weapon_required: [0; 2],
            doctrine_keys: Default::default(),
            term_metadata: Default::default(),
            source_memo: Default::default(),
            commander_inputs: engine_native_memory::walk::Buffer::Count,
            commander_inputs_len: 0,
            commander_inputs_required: 0,
            commander_timed_traits: &mut Vec::new(),
            commander_timed_traits_len: 0,
            side_units: engine_native_memory::walk::Buffer::Count,
            side_units_len: 0,
            side_units_required: 0,
            summary_countries: &mut Vec::new(),
            summary_countries_len: 0,
            custom_sea: engine_native_memory::walk::Buffer::Count,
            custom_sea_len: 0,
            custom_sea_required: 0,
            damage_sources: engine_native_memory::walk::Buffer::Count,
            damage_sources_len: 0,
            damage_sources_required: 0,
            wing_types: engine_native_memory::walk::Buffer::Count,
            wing_types_len: 0,
            wing_types_required: 0,
            plane_wings: engine_native_memory::walk::Buffer::Count,
            plane_wings_len: 0,
            plane_wings_required: 0,
        };
        let mut row = read_buffer(
            &memory,
            0,
            ENTRY,
            0,
            buffers.wing_types.suffix(0),
            &mut buffers.wing_types_len,
            &mut buffers.wing_types_required,
            buffers.plane_wings.suffix(0),
            &mut buffers.plane_wings_len,
            &mut buffers.plane_wings_required,
        )
        .unwrap();
        assert_eq!(row.wing_types, [0, 0]);
        read_hits(&memory, 0, ENTRY, &mut row, &mut buffers).unwrap();
        assert!(row.wings.iter().all(|span| span[1] == 0));

        const COUNT: usize = 65;
        const DATA: usize = 0x1000;
        const HIT: usize = 0x900;
        memory.0.resize(DATA + COUNT * 8, 0);
        for (at, value) in [
            (ENTRY + 0x100, DATA as u64),
            (HIT, crate::naval_combat_hit::VTABLE_RVA.0 as u64),
            (HIT + 0x28, engine_native_memory::msvc_string::INLINE_CAPACITY),
            (HIT + 0x38, i64::MIN as u64),
        ] {
            memory.0[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        for offset in [0x108, 0x10c] {
            memory.0[ENTRY + offset..ENTRY + offset + 4]
                .copy_from_slice(&(COUNT as u32).to_le_bytes());
        }
        for entry in memory.0[DATA..].chunks_exact_mut(8) {
            entry.copy_from_slice(&(HIT as u64).to_le_bytes());
        }
        let mut hits = Vec::new();
        buffers.plane_naval_hits = engine_native_memory::walk::Buffer::Growing(&mut hits);
        read_hits(&memory, 0, ENTRY, &mut row, &mut buffers).unwrap();
        assert_eq!(row.naval_hits, [0, COUNT]);
        assert_eq!(buffers.plane_naval_hits[COUNT - 1].damage, i64::MIN);
    }
    #[test]
    fn original_wing_headers_grow_both_arenas() {
        const PLANE: usize = 0x100;
        const DATA: usize = 0x1000;
        let (mut types, mut wings) = (Vec::new(), Vec::new());
        for count in [15_usize, 50, 82] {
            let mut bytes = vec![0; DATA + count * 16];
            bytes[PLANE..PLANE + 8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
            for (offset, first) in [(0x28, DATA), (0x40, DATA + count * 8)] {
                bytes[PLANE + offset..PLANE + offset + 8]
                    .copy_from_slice(&(first as u64).to_le_bytes());
                bytes[PLANE + offset + 8..PLANE + offset + 16]
                    .copy_from_slice(&(((count as u64) << 32) | count as u64).to_le_bytes());
            }
            for index in 0..count {
                bytes[DATA + index * 8..DATA + index * 8 + 8]
                    .copy_from_slice(&(index as u64 + 1).to_le_bytes());
            }
            let memory = engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let row = read_buffer(
                &memory,
                0,
                PLANE,
                0,
                engine_native_memory::walk::Buffer::Growing(&mut types),
                &mut 0,
                &mut 0,
                engine_native_memory::walk::Buffer::Growing(&mut wings),
                &mut 0,
                &mut 0,
            )
            .unwrap();
            assert_eq!(row.wings, [[0, count], [count, count]]);
            assert_eq!(row.wing_types, [0, count]);
            assert_eq!(&types[..count], vec![u32::MAX; count]);
            assert!(
                wings[..count]
                    .iter()
                    .enumerate()
                    .all(|(index, &value)| value == index as u64 + 1)
            );
            assert!(wings[count..2 * count].iter().all(|&value| value == 0));
        }
    }
}
