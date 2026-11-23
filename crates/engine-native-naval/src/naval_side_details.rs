use engine_native_memory::{Memory, cpdx_array::Array};
use engine_native::cnaval_combatant::{
    addresses,
    field,
    Side,
    COMMANDER,
    ENGAGED,
    SCREENING,
    POSITIONING,
    ANTI_AIR,
    UNITS,
};
use engine_sites::Rva;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DetailsRow {
    pub damage_history: [[i64; 6]; 6],
    pub original_health: i64,
    pub total_damage: i64,
    pub damage_by_type: [usize; 2],
    pub commander: u64,
    pub engaged: u8,
    pub retreating: u8,
    pub capital_screening: i64,
    pub carrier_screening: i64,
    pub positioning: i64,
    pub anti_air: i64,
    pub planes: [usize; 2],
    pub summary: crate::naval_combat_summary_inputs::Row,
    pub commander_inputs: [usize; 2],
    pub units: [usize; 2],
}

fn damage_by_type(
    memory: &impl Memory,
    side: usize,
    output: engine_native_memory::walk::Buffer<'_, (u32, i64)>,
    used: &mut usize,
    required: &mut usize,
) -> Result<[usize; 2], &'static str> {
    walk_damage_by_type(memory, side, Some(output), used, required)
}

pub fn damage_source_demand(memory: &impl Memory, side: usize) -> Result<usize, &'static str> {
    let (mut used, mut required) = (0, 0);
    walk_damage_by_type(memory, side, None, &mut used, &mut required)?;
    Ok(required)
}

fn walk_damage_by_type(
    memory: &impl Memory,
    side: usize,
    mut output: Option<engine_native_memory::walk::Buffer<'_, (u32, i64)>>,
    used: &mut usize,
    required: &mut usize,
) -> Result<[usize; 2], &'static str> {
    let sentinel = field::<u64>(memory, side, 0x2b0)? as usize;
    let count = field::<u64>(memory, side, 0x2b8)?;
    let count = usize::try_from(count).map_err(|_| "naval damage source count overflow")?;
    let start = *used;
    let end = start
        .checked_add(count)
        .ok_or("naval damage source arena length overflow")?;
    *required = (*required).max(end);
    if let Some(output) = output.as_mut()
        && !output.ensure(end)?
    {
        return Err("naval damage source arena capacity exceeded");
    }
    if sentinel == 0 {
        return Err("naval damage source sentinel null");
    }
    let mut next = field::<u64>(memory, sentinel, 0)? as usize;
    let mut previous = sentinel;
    for index in start..end {
        if next == sentinel || next == 0 {
            return Err("naval damage source list ended early");
        }
        if let Some(output) = output.as_mut() {
            output[index] = (field(memory, next, 0x10)?, field(memory, next, 0x18)?);
        } else if field::<u64>(memory, next, 8)? as usize != previous {
            return Err("naval damage source predecessor mismatch");
        }
        previous = next;
        next = field::<u64>(memory, next, 0)? as usize;
    }
    if next != sentinel {
        return Err("naval damage source list exceeds native count");
    }
    if output.is_none()
        && (field::<u64>(memory, side, 0x2b0)? as usize != sentinel
            || field::<u64>(memory, side, 0x2b8)? != count as u64)
    {
        return Err("naval damage source header changed");
    }
    *used = end;
    Ok([start, count])
}

pub(crate) struct SideArenas<'a> {
    pub countries: &'a mut Vec<u32>,
    pub countries_len: &'a mut usize,
    pub units: engine_native_memory::walk::Buffer<'a, u64>,
    pub units_len: &'a mut usize,
    pub units_required: &'a mut usize,
    pub damage_sources: engine_native_memory::walk::Buffer<'a, (u32, i64)>,
    pub damage_sources_len: &'a mut usize,
    pub damage_sources_required: &'a mut usize,
}

fn units(
    memory: &impl Memory,
    base: usize,
    side: &Side,
    mut arena: engine_native_memory::walk::Buffer<'_, u64>,
    len: &mut usize,
    required: &mut usize,
) -> Result<[usize; 2], &'static str> {
    const CHUNK: u32 = 64;
    let array = Array::read(
        memory,
        side.pointer
            .checked_add(UNITS)
            .ok_or("naval side address overflow")?,
        8,
    )
    .ok_or("naval side unit array unreadable or invalid")?;
    let module = |key: Rva| {
        engine_sites::at(base, key)
            .map(|address| address as u64)
            .ok_or("naval side unit vtable overflow")
    };
    let (primary, secondary) = (
        module(engine_native::ctask_force::VTABLE_RVA)?,
        module(engine_native::ctask_force::SECONDARY_VTABLE_RVA)?,
    );
    let start = *len;
    let count = array.count as usize;
    let end = start
        .checked_add(count)
        .ok_or("naval side unit length overflow")?;
    let fits = arena.ensure(end)?;
    let mut slots = [0u8; CHUNK as usize * 8];
    let mut first = 0;
    while first < array.count {
        let n = (array.count - first).min(CHUNK) as usize;
        let address = array
            .element(first, 8)
            .ok_or("naval side unit slot unreadable")?;
        if !engine_native_memory::read_slice(memory, address, &mut slots[..n * 8]) {
            return Err("naval side unit slot unreadable");
        }
        for (offset, word) in slots[..n * 8].chunks_exact(8).enumerate() {
            let pointer = u64::from_le_bytes(word.try_into().expect("eight bytes")) as usize;
            let (mut vtable, mut reference) = ([0u8; 8], [0u8; 16]);
            if memory.copy_scattered(
                addresses(pointer, [0, engine_native::ctask_force::SECONDARY_BASE])?,
                &mut [&mut vtable, &mut reference],
            ) != 2
            {
                return Err("naval side unit unreadable");
            }
            let handle = u64::from_le_bytes(reference[8..].try_into().expect("eight bytes"));
            if u64::from_le_bytes(vtable) != primary
                || u64::from_le_bytes(reference[..8].try_into().expect("eight bytes")) != secondary
                || handle == 0
            {
                return Err("naval side unit is not a task force");
            }
            if fits {
                arena[start + first as usize + offset] = handle;
            }
        }
        first += n as u32;
    }
    *required = required
        .checked_add(count)
        .ok_or("naval side unit demand overflow")?;
    if !fits {
        return Err(crate::naval_member_details::DETAIL_ARENA_CAPACITY);
    }
    *len += count;
    Ok([start, count])
}

pub(crate) fn details(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    combat: usize,
    [side, opponent]: [&Side; 2],
    arenas: &mut SideArenas<'_>,
) -> Result<DetailsRow, &'static str> {
    let leader = field::<u64>(memory, side.pointer, COMMANDER)? as usize;
    let commander = if leader == 0 {
        0
    } else {
        if field::<u64>(memory, leader, 0)?
            != engine_sites::at(base, engine_native_parts::cnavy_leader::VTABLE_RVA)
                .ok_or("naval leader vtable overflow")? as u64
        {
            return Err("naval commander vtable mismatch");
        }
        let handle = field::<u64>(memory, leader, engine_native::cunit_leader::TYPE)?;
        if handle == 0 || engine_native_parts::creference_registry::resolve(memory, base, handle)? != Some(leader)
        {
            return Err("naval commander reference mismatch");
        }
        handle
    };
    let history: [u8; 36 * 8] = field(memory, side.pointer, 0x188)?;
    let mut damage_history = [[0; 6]; 6];
    for (value, bytes) in damage_history
        .iter_mut()
        .flatten()
        .zip(history.chunks_exact(8))
    {
        *value = i64::from_le_bytes(bytes.try_into().expect("eight bytes"));
    }
    Ok(DetailsRow {
        damage_history,
        original_health: field(memory, side.pointer, 0x178)?,
        total_damage: field(memory, side.pointer, 0x180)?,
        damage_by_type: damage_by_type(
            memory,
            side.pointer,
            arenas.damage_sources.suffix(0),
            arenas.damage_sources_len,
            arenas.damage_sources_required,
        )?,
        commander,
        engaged: field(memory, side.pointer, ENGAGED)?,
        retreating: field(memory, side.pointer, 0x140)?,
        capital_screening: field(memory, side.pointer, SCREENING)?,
        carrier_screening: field(memory, side.pointer, SCREENING + 8)?,
        positioning: field(memory, side.pointer, POSITIONING)?,
        anti_air: field(memory, side.pointer, ANTI_AIR)?,
        planes: [0; 2],
        commander_inputs: [0; 2],
        summary: crate::naval_combat_summary_inputs::read(
            memory,
            base,
            game_state,
            combat,
            [side.pointer, opponent.pointer],
            arenas.countries,
            arenas.countries_len,
        )?,
        units: units(
            memory,
            base,
            side,
            arenas.units.suffix(0),
            arenas.units_len,
            arenas.units_required,
        )?,
    })
}

#[cfg(test)]
mod damage_source_tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    #[test]
    fn ordered_source_map_keeps_native_occurrences_and_refuses_short_list() {
        let mut bytes = vec![0; 0x700];
        let put64 = |bytes: &mut Vec<u8>, at: usize, value: u64| {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        };
        let side = 0x100;
        let sentinel = 0x500;
        let first = 0x540;
        let second = 0x580;
        put64(&mut bytes, side + 0x2b0, sentinel as u64);
        put64(&mut bytes, side + 0x2b8, 2);
        put64(&mut bytes, sentinel, first as u64);
        put64(&mut bytes, first, second as u64);
        put64(&mut bytes, second, sentinel as u64);
        put64(&mut bytes, first + 8, sentinel as u64);
        put64(&mut bytes, second + 8, first as u64);
        bytes[first + 0x10..first + 0x14].copy_from_slice(&17_u32.to_le_bytes());
        bytes[second + 0x10..second + 0x14].copy_from_slice(&13_u32.to_le_bytes());
        put64(&mut bytes, first + 0x18, 3000);
        put64(&mut bytes, second + 0x18, 2000);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut output = vec![(0, 0); 2];
        let (mut used, mut required) = (0, 0);
        assert_eq!(
            damage_by_type(
                &memory,
                side,
                engine_native_memory::walk::Buffer::Growing(&mut output),
                &mut used,
                &mut required
            ),
            Ok([0, 2])
        );
        assert_eq!(output, [(17, 3000), (13, 2000)]);
        assert_eq!(damage_source_demand(&memory, side), Ok(2));
        put64(&mut bytes, first, sentinel as u64);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        (used, required) = (0, 0);
        assert_eq!(
            damage_by_type(
                &memory,
                side,
                engine_native_memory::walk::Buffer::Growing(&mut output),
                &mut used,
                &mut required
            ),
            Err("naval damage source list ended early")
        );
        assert_eq!(
            damage_source_demand(&memory, side),
            Err("naval damage source list ended early")
        );
    }

    #[test]
    fn damage_source_hint_drops_a_changed_map_size() {
        struct Torn(std::cell::Cell<usize>);
        impl Memory for Torn {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let value = match address {
                    0x3b0 | 0x500 => 0x500_u64,
                    0x3b8 => {
                        let count = self.0.get();
                        self.0.set(count + 1);
                        u64::from(count != 0)
                    }
                    _ => return false,
                };
                output.copy_from_slice(&value.to_le_bytes());
                true
            }
        }
        assert_eq!(
            damage_source_demand(&Torn(std::cell::Cell::new(0)), 0x100),
            Err("naval damage source header changed")
        );
    }

    #[test]
    fn damage_source_hint_stops_a_cycle_with_an_untrusted_large_count() {
        struct Cycle;
        impl Memory for Cycle {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let value = match address {
                    0x3b0 | 0x548 => 0x500_u64,
                    0x3b8 => u64::MAX,
                    0x500 | 0x540 => 0x540,
                    _ => return false,
                };
                output.copy_from_slice(&value.to_le_bytes());
                true
            }
        }
        assert_eq!(
            damage_source_demand(&Cycle, 0x100),
            Err("naval damage source predecessor mismatch")
        );
    }
    #[test]
    fn original_side_headers_grow_country_and_mod_sized_unit_and_damage_arenas() {
        const SIDE: usize = 0x100;
        const SENTINEL: usize = 0x500;
        const FORCE: usize = 0x600;
        const NODES: usize = 0x1000;
        const POINTERS: usize = 0x10_0000;
        let (mut damage, mut handles) = (Vec::new(), Vec::new());
        for count in [15_usize, 50, 82] {
            let mut bytes = vec![0; POINTERS + count * 8];
            let put = |bytes: &mut [u8], at: usize, value: u64| {
                bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
            };
            put(&mut bytes, SIDE + 0x2b0, SENTINEL as u64);
            put(&mut bytes, SIDE + 0x2b8, count as u64);
            put(&mut bytes, SENTINEL, NODES as u64);
            put(&mut bytes, SIDE + UNITS, POINTERS as u64);
            put(
                &mut bytes,
                SIDE + UNITS + 8,
                ((count as u64) << 32) | count as u64,
            );
            put(&mut bytes, FORCE, engine_native::ctask_force::VTABLE_RVA.0 as u64);
            put(
                &mut bytes,
                FORCE + engine_native::ctask_force::SECONDARY_BASE,
                engine_native::ctask_force::SECONDARY_VTABLE_RVA.0 as u64,
            );
            put(
                &mut bytes,
                FORCE + engine_native::ctask_force::SECONDARY_BASE + 8,
                0xfeed_beef_0000_0046,
            );
            for index in 0..count {
                let node = NODES + index * 0x20;
                put(
                    &mut bytes,
                    node,
                    if index + 1 < count {
                        (node + 0x20) as u64
                    } else {
                        SENTINEL as u64
                    },
                );
                put(
                    &mut bytes,
                    node + 8,
                    if index == 0 {
                        SENTINEL as u64
                    } else {
                        (node - 0x20) as u64
                    },
                );
                bytes[node + 0x10..node + 0x14].copy_from_slice(&(index as u32).to_le_bytes());
                put(&mut bytes, node + 0x18, -(index as i64) as u64);
                put(&mut bytes, POINTERS + index * 8, FORCE as u64);
            }
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            assert_eq!(
                damage_by_type(
                    &memory,
                    SIDE,
                    engine_native_memory::walk::Buffer::Growing(&mut damage),
                    &mut 0,
                    &mut 0
                ),
                Ok([0, count])
            );
            assert!(
                damage[..count]
                    .iter()
                    .enumerate()
                    .all(|(index, &row)| row == (index as u32, -(index as i64)))
            );
            let side = Side {
                pointer: SIDE,
                scalars: Ok((0, 0)),
            };
            assert_eq!(
                units(
                    &memory,
                    0,
                    &side,
                    engine_native_memory::walk::Buffer::Growing(&mut handles),
                    &mut 0,
                    &mut 0
                ),
                Ok([0, count])
            );
            assert_eq!(&handles[..count], vec![0xfeed_beef_0000_0046; count]);
            assert!(
                damage_by_type(
                    &memory,
                    SIDE,
                    engine_native_memory::walk::Buffer::Count,
                    &mut 0,
                    &mut 0
                )
                .is_err()
            );
            assert!(
                units(
                    &memory,
                    0,
                    &side,
                    engine_native_memory::walk::Buffer::Count,
                    &mut 0,
                    &mut 0
                )
                .is_err()
            );
        }
    }
}
