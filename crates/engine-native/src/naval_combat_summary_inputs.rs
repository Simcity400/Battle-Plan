pub use engine_native_parts::naval_side_layout::{COUNTRIES, COUNTRY_COUNT};
use crate::{
    Memory, ccountry, cnaval_combatant, cpdx_array::Array, modifier_accumulator as accumulator,
    modifier_definition as metadata,
};
use cnaval_combatant::field;
use engine_sites::Rva;

pub const SECONDARY_COUNTRIES: usize = 0x50;
const LIST_COUNT: usize = COUNTRY_COUNT - COUNTRIES;
pub const NODE_TAG: usize = 0;
pub const NODE_NEXT: usize = 0x10;
pub const RECENT_JOIN: usize = 0x160;
pub const ESCORT: usize = 0x168;
pub const MODIFIER_KEYS: [u32; 5] = [
    crate::modifier_keys::NAVAL_ENEMY_POSITIONING_IN_INITIAL_ATTACK,
    crate::modifier_keys::SCREENING_EFFICIENCY,
    crate::modifier_keys::POSITIONING,
    crate::modifier_keys::SCREENING_WITHOUT_SCREENS,
    crate::modifier_keys::NAVAL_ENEMY_FLEET_SIZE_RATIO_PENALTY_FACTOR,
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Modifier {
    pub raw: i64,
    pub present: u8,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub recent_join_penalty: i64,
    pub escort_bonus: i64,
    pub countries: [usize; 2],
    pub secondary_countries: [usize; 2],
    pub modifiers: [[Modifier; 5]; 5],
    pub modifier_metadata: [[u32; 2]; 5],
    pub feature_flags: u32,
    pub eligibility_callback: u8,
}
fn site(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("naval summary site withheld")
}

fn add(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("naval summary address overflow")
}
fn values(memory: &impl Memory, address: usize) -> Result<[Modifier; 5], &'static str> {
    let mut result = [Modifier::default(); 5];
    if address == 0 {
        return Ok(result);
    }
    let mut cached = true;
    for (out, key) in result.iter_mut().zip(MODIFIER_KEYS) {
        if let Some(value) = memory.modifier_accumulator_value(address, key) {
            let value = value?;
            *out = Modifier {
                raw: value.raw,
                present: u8::from(value.present),
            };
        } else {
            cached = false;
            break;
        }
    }
    if cached {
        return Ok(result);
    }
    Array::read(memory, address, 16).ok_or("naval summary modifier array invalid")?;
    let mut raw = [accumulator::Value::default(); 5];
    accumulator::lookup_many_at(memory, address, &MODIFIER_KEYS, &mut raw)?;
    for (out, value) in result.iter_mut().zip(raw) {
        *out = Modifier {
            raw: value.raw,
            present: u8::from(value.present),
        };
    }
    Ok(result)
}

pub fn countries(
    memory: &impl Memory,
    side: usize,
    slots: u32,
    arena: &mut Vec<u32>,
    len: &mut usize,
) -> Result<[usize; 2], &'static str> {
    list(memory, side, COUNTRIES, slots, arena, len)
}

fn list(
    memory: &impl Memory,
    side: usize,
    offset: usize,
    slots: u32,
    arena: &mut Vec<u32>,
    len: &mut usize,
) -> Result<[usize; 2], &'static str> {
    let head = field::<u64>(memory, side, offset)? as usize;
    let count = field::<i32>(memory, side, offset + LIST_COUNT)?;
    let count = u32::try_from(count).map_err(|_| "naval summary country count negative")?;
    if count > slots {
        return Err("naval summary country count exceeds the country table");
    }
    let start = *len;
    let required = start
        .checked_add(count as usize)
        .ok_or("naval summary country arena overflow")?;
    let additional = required.saturating_sub(arena.len());
    crate::allocation::reserve_push_batch(arena, additional)
        .map_err(|_| "collector allocation failed")?;
    let mut end = start;
    let mut node = head;
    for _ in 0..count {
        if node == 0 {
            return Err("naval summary country list ended early");
        }
        let tag = field::<u32>(memory, node, NODE_TAG)?;
        if end == arena.len() {
            arena.push(tag);
        } else {
            arena[end] = tag;
        }
        end = end
            .checked_add(1)
            .ok_or("naval summary country arena overflow")?;
        node = field::<u64>(memory, node, NODE_NEXT)? as usize;
    }
    if node != 0 {
        return Err("naval summary country list exceeds native count");
    }
    *len = end;
    Ok([start, end - start])
}

pub fn read(
    memory: &impl Memory,
    base: usize,
    state: usize,
    combat: usize,
    [side, opponent]: [usize; 2],
    arena: &mut Vec<u32>,
    len: &mut usize,
) -> Result<Row, &'static str> {
    let count = field::<i32>(memory, side, COUNTRY_COUNT)?;
    let secondary = field::<i32>(memory, side, SECONDARY_COUNTRIES + LIST_COUNT)?;
    let reader = if count > 0 || secondary > 0 {
        Some(ccountry::Reader::new(memory, base, state)?)
    } else {
        None
    };
    let mut row = Row {
        recent_join_penalty: field(memory, side, RECENT_JOIN)?,
        escort_bonus: field(memory, side, ESCORT)?,
        ..Default::default()
    };
    let slots = reader.as_ref().map_or(0, |reader| reader.count());
    row.countries = countries(memory, side, slots, arena, len)?;
    row.secondary_countries = list(memory, side, SECONDARY_COUNTRIES, slots, arena, len)?;
    let table = field::<u64>(memory, site(base, metadata::TABLE_RVA)?, 0)? as usize;
    for (out, key) in row.modifier_metadata.iter_mut().zip(MODIFIER_KEYS) {
        let record = add(table, key as usize * metadata::STRIDE)?;
        *out = [
            field(memory, record, metadata::FLAGS)?,
            field(memory, record, metadata::FEATURE_SELECTOR)?,
        ];
    }
    let callback = field::<u64>(memory, site(base, metadata::ELIGIBILITY_CALLBACK_RVA)?, 0)?;
    row.eligibility_callback = if callback == 0 {
        0
    } else if callback == site(base, metadata::ELIGIBILITY_FLAGS_GETTER_RVA)? as u64 {
        1
    } else {
        return Err("naval summary modifier callback unsupported");
    };
    row.feature_flags = field(memory, site(base, metadata::TRADE_FEATURE_FLAGS_RVA)?, 0)?;
    let own = field::<u64>(memory, side, cnaval_combatant::COMMANDER)? as usize;
    let enemy = field::<u64>(memory, opponent, cnaval_combatant::COMMANDER)? as usize;
    for (scope, leader) in [(3, own), (4, enemy)] {
        if leader != 0 {
            if field::<u64>(memory, leader, 0)?
                != site(base, crate::cnavy_leader::VTABLE_RVA)? as u64
            {
                return Err("naval summary commander identity mismatch");
            }
            row.modifiers[scope] = values(memory, add(leader, 0x418)?)?;
        }
    }
    if let Some(reader) = reader.filter(|_| count > 0) {
        let country = reader.resolve(arena[row.countries[0]])?;
        row.modifiers[0] = values(
            memory,
            add(country.pointer, ccountry::MODIFIER_ACCUMULATOR)?,
        )?;
        let province = field::<u64>(memory, combat, crate::cnaval_combat::PROVINCE)? as usize;
        let (_, regional) =
            crate::cweather_manager::movement_modifiers(memory, base, state, province)?;
        row.modifiers[1] = values(memory, regional)?;
        let region = field::<u64>(memory, province, crate::cprovince::STRATEGIC_REGION)? as usize;
        let terrain = crate::naval_ship_modifier_inputs::terrain(memory, base, region)?;
        row.modifiers[2] = values(memory, add(terrain, 0xb8)?)?;
    }
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side_list(count: usize) -> Vec<u8> {
        const NODES: usize = 0x100;
        let mut data = vec![0u8; NODES + count * 0x20];
        let head = if count == 0 { 0 } else { BASE + NODES };
        data[COUNTRIES..COUNTRIES + 8].copy_from_slice(&(head as u64).to_le_bytes());
        data[COUNTRY_COUNT..COUNTRY_COUNT + 4].copy_from_slice(&(count as i32).to_le_bytes());
        for index in 0..count {
            let node = NODES + index * 0x20;
            data[node..node + 4].copy_from_slice(&(1000 + index as u32).to_le_bytes());
            let next = if index + 1 == count {
                0
            } else {
                BASE + node + 0x20
            };
            data[node + NODE_NEXT..node + NODE_NEXT + 8]
                .copy_from_slice(&(next as u64).to_le_bytes());
        }
        data
    }
    const BASE: usize = 0x10_0000;

    #[test]
    fn the_secondary_list_follows_its_own_head_and_count() {
        let mut data = side_list(2);
        let (head, count) = (
            data[COUNTRIES..COUNTRIES + 8].to_vec(),
            data[COUNTRY_COUNT..COUNTRY_COUNT + 4].to_vec(),
        );
        data[COUNTRIES..COUNTRY_COUNT + 4].fill(0);
        data[SECONDARY_COUNTRIES..SECONDARY_COUNTRIES + 8].copy_from_slice(&head);
        data[SECONDARY_COUNTRIES + LIST_COUNT..SECONDARY_COUNTRIES + LIST_COUNT + 4]
            .copy_from_slice(&count);
        let memory = crate::memory::SliceMemory {
            base: BASE,
            bytes: &data,
        };
        let (mut arena, mut len) = (Vec::new(), 0);
        assert_eq!(
            countries(&memory, BASE, 4, &mut arena, &mut len),
            Ok([0, 0])
        );
        assert_eq!(
            list(&memory, BASE, SECONDARY_COUNTRIES, 4, &mut arena, &mut len),
            Ok([0, 2])
        );
        assert_eq!(arena[..len], [1000, 1001]);
    }

    #[test]
    fn country_list_beyond_the_old_cap_fills_the_caller_arena_in_order() {
        let count = 15_000;
        let data = side_list(count);
        let memory = crate::memory::SliceMemory {
            base: BASE,
            bytes: &data,
        };
        let mut arena = vec![7, 8, 9];
        let mut len = 2;
        let span = countries(&memory, BASE, count as u32 + 1, &mut arena, &mut len).unwrap();
        assert_eq!(span, [2, count]);
        assert_eq!(len, count + 2);
        assert_eq!(arena[..2], [7, 8]);
        assert!(
            arena[2..len]
                .iter()
                .enumerate()
                .all(|(index, &tag)| tag == 1000 + index as u32)
        );
        let mut len = 0;
        assert_eq!(
            countries(&memory, BASE, count as u32 - 1, &mut arena, &mut len),
            Err("naval summary country count exceeds the country table")
        );
        assert_eq!(len, 0);
    }

    #[test]
    fn country_list_count_mismatches_fail_without_publishing_a_span() {
        let mut data = side_list(3);
        data[COUNTRY_COUNT..COUNTRY_COUNT + 4].copy_from_slice(&2i32.to_le_bytes());
        let memory = crate::memory::SliceMemory {
            base: BASE,
            bytes: &data,
        };
        let (mut arena, mut len) = (Vec::new(), 0);
        assert_eq!(
            countries(&memory, BASE, 10, &mut arena, &mut len),
            Err("naval summary country list exceeds native count")
        );
        assert_eq!(len, 0);
        data[COUNTRY_COUNT..COUNTRY_COUNT + 4].copy_from_slice(&4i32.to_le_bytes());
        let memory = crate::memory::SliceMemory {
            base: BASE,
            bytes: &data,
        };
        assert_eq!(
            countries(&memory, BASE, 10, &mut arena, &mut len),
            Err("naval summary country list ended early")
        );
        data[COUNTRY_COUNT..COUNTRY_COUNT + 4].copy_from_slice(&(-1i32).to_le_bytes());
        let memory = crate::memory::SliceMemory {
            base: BASE,
            bytes: &data,
        };
        assert_eq!(
            countries(&memory, BASE, 10, &mut arena, &mut len),
            Err("naval summary country count negative")
        );
        assert_eq!(len, 0);
    }

    #[test]
    fn raw_modifier_lookup_preserves_absence_extremes_and_rejects_short_arrays() {
        let mut data = vec![0; 64];
        data[..8].copy_from_slice(&0x1020u64.to_le_bytes());
        data[8..12].copy_from_slice(&2u32.to_le_bytes());
        data[12..16].copy_from_slice(&2u32.to_le_bytes());
        data[32..36].copy_from_slice(&0x5au32.to_le_bytes());
        data[40..48].copy_from_slice(&i64::MIN.to_le_bytes());
        data[48..52].copy_from_slice(&0x1d0u32.to_le_bytes());
        data[56..64].copy_from_slice(&i64::MAX.to_le_bytes());
        let memory = crate::memory::SliceMemory {
            base: 0x1000,
            bytes: &data,
        };
        let row = values(&memory, 0x1000).unwrap();
        assert_eq!(
            row[0],
            Modifier {
                raw: i64::MIN,
                present: 1
            }
        );
        assert_eq!(row[1], Modifier::default());
        assert_eq!(
            row[4],
            Modifier {
                raw: i64::MAX,
                present: 1
            }
        );
        data[8..12].copy_from_slice(&8193u32.to_le_bytes());
        data[12..16].copy_from_slice(&8193u32.to_le_bytes());
        assert_eq!(
            values(
                &crate::memory::SliceMemory {
                    base: 0x1000,
                    bytes: &data
                },
                0x1000
            ),
            Err("modifier accumulator entry unreadable")
        );
    }

    #[test]
    fn modifier_arrays_past_8192_are_read_whole() {
        const COUNT: usize = 8193;
        let mut data = vec![0; 16 + COUNT * 16];
        data[..8].copy_from_slice(&0x1010u64.to_le_bytes());
        data[8..12].copy_from_slice(&(COUNT as u32).to_le_bytes());
        data[12..16].copy_from_slice(&(COUNT as u32).to_le_bytes());
        for index in 0..COUNT {
            let at = 16 + index * 16;
            data[at..at + 4].copy_from_slice(&(index as u32 + 1).to_le_bytes());
            data[at + 8..at + 16].copy_from_slice(&i64::MIN.to_le_bytes());
        }
        let row = values(
            &crate::memory::SliceMemory {
                base: 0x1000,
                bytes: &data,
            },
            0x1000,
        )
        .unwrap();
        assert!(
            row.iter()
                .all(|value| value.present == 1 && value.raw == i64::MIN)
        );
    }
}
