//! Raw side operands for positioning141609460, screening14160A7B0 and
//! carrier crowding14160C430. Ship/plane counts and sums remain viewer work.
use crate::{
    Memory, ccountry, cnaval_combatant, cpdx_array::Array, modifier_accumulator as accumulator,
    modifier_definition as metadata,
};
use cnaval_combatant::field;

/// Ordered country linked list;1417DA860/1406D7F90 iterate node+10.
/// CCombatant::AddUnit1413CC2B0 ->1413CC0D0 appends a 0x20-byte node only
/// when CountryTagListContainsEquivalent1406D7F90 finds no equivalent tag, so
/// the list holds at most one entry per country object. There is no native
/// size limit; the live country table header bounds the walk.
pub const COUNTRIES: usize = 0x38;
/// Signed i32 node count, incremented by1413CC0D0.
pub const COUNTRY_COUNT: usize = 0x48;
/// The second ordered list1413CC0D0 fills: canonically distinct roster owners of the side's
/// units. `UpdateCombatSharedVisionFlag@1413CEBE0` admits the viewer through it.
pub const SECONDARY_COUNTRIES: usize = 0x50;
/// Each list's signed count follows its head by 0x10.
const LIST_COUNT: usize = COUNTRY_COUNT - COUNTRIES;
/// Node layout written by1413CC0D0: raw u32 tag +0, next pointer +0x10.
pub const NODE_TAG: usize = 0;
pub const NODE_NEXT: usize = 0x10;
///141609460 subtracts recent joins and adds convoy escort raw values.
pub const RECENT_JOIN: usize = 0x140;
pub const ESCORT: usize = 0x148;
/// Raw fixed5 definitions, in positioning/screening/carrier/AA binder order.
pub const FIXED_DEFINES: [usize; 18] = [
    0x03318ca0, 0x03318d30, 0x03318e00, 0x03318ee0, 0x03318fb8, 0x03319140, 0x03319228, 0x03319930,
    0x03319a28, 0x03319b08, 0x03319bc8, 0x0331ada8, 0x0331aea0, 0x0331af88, 0x0331b070, 0x03318f68,
    0x0331a468, 0x0331a778,
];
/// Minimum ships, initial positioning duration, free carrier threshold.
pub const INTEGER_DEFINES: [usize; 3] = [0x03319080, 0x0331d1ac, 0x03318ec0];
/// Sorted raw lookup keys:initial positioning, screening, positioning,
/// convoy-only screening, opponent-admiral ship-ratio penalty.
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
    pub is_attacker: u8,
    /// Start and length in the caller-owned country arena, in native order.
    pub countries: [usize; 2],
    /// The [`SECONDARY_COUNTRIES`] list, in the same arena.
    pub secondary_countries: [usize; 2],
    /// Country, regional weather, sea terrain, own admiral, opposing admiral.
    pub modifiers: [[Modifier; 5]; 5],
    /// Per key: raw flags and feature selector.
    pub modifier_metadata: [[u32; 2]; 5],
    pub feature_flags: u32,
    pub eligibility_callback: u8,
    pub fixed_defines: [i64; 18],
    pub integer_defines: [i32; 3],
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
    let array = Array::read(memory, address, 16).ok_or("naval summary modifier array invalid")?;
    if array.count > 8192 {
        return Err("naval summary modifier bound exceeded");
    }
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

/// Appends a side's native country list to the caller-owned arena from
/// `*len`, returning its `[start, len]` span. The walk is bounded by the
/// native signed count, which may not exceed `slots`, the live country-table
/// header: the list holds one node per distinct country object. The arena
/// grows at this admission point on the acquisition worker; a failed walk
/// leaves `*len` unchanged.
pub fn countries(
    memory: &impl Memory,
    side: usize,
    slots: u32,
    arena: &mut Vec<u32>,
    len: &mut usize,
) -> Result<[usize; 2], &'static str> {
    list(memory, side, COUNTRIES, slots, arena, len)
}

/// [`countries`] for the list whose head is at `offset`.
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

/// Caller has authenticated combat and both side back-links. Retain source
/// words without calling engine code. Country tags are appended to the
/// caller-owned `arena` from `*len`.
pub fn read(
    memory: &impl Memory,
    base: usize,
    state: usize,
    combat: usize,
    side: usize,
    arena: &mut Vec<u32>,
    len: &mut usize,
) -> Result<Row, &'static str> {
    let count = field::<i32>(memory, side, COUNTRY_COUNT)?;
    let secondary = field::<i32>(memory, side, SECONDARY_COUNTRIES + LIST_COUNT)?;
    // Native1415B4BB0 only resolves countries for nonempty membership; the
    // country-table header then also bounds the list walks.
    let reader = if count > 0 || secondary > 0 {
        Some(ccountry::Reader::new(memory, base, state)?)
    } else {
        None
    };
    let mut row = Row {
        recent_join_penalty: field(memory, side, RECENT_JOIN)?,
        escort_bonus: field(memory, side, ESCORT)?,
        is_attacker: field(memory, side, 0xb8)?,
        ..Default::default()
    };
    let slots = reader.as_ref().map_or(0, |reader| reader.count());
    row.countries = countries(memory, side, slots, arena, len)?;
    row.secondary_countries = list(memory, side, SECONDARY_COUNTRIES, slots, arena, len)?;
    for (value, rva) in row.fixed_defines.iter_mut().zip(FIXED_DEFINES) {
        *value = field(memory, base, rva)?;
    }
    for (value, rva) in row.integer_defines.iter_mut().zip(INTEGER_DEFINES) {
        *value = field(memory, base, rva)?;
    }
    let table = field::<u64>(memory, base, metadata::TABLE_RVA)? as usize;
    for (out, key) in row.modifier_metadata.iter_mut().zip(MODIFIER_KEYS) {
        let record = add(table, key as usize * metadata::STRIDE)?;
        *out = [
            field(memory, record, metadata::FLAGS)?,
            field(memory, record, metadata::FEATURE_SELECTOR)?,
        ];
    }
    let callback = field::<u64>(memory, base, metadata::ELIGIBILITY_CALLBACK_RVA)?;
    row.eligibility_callback = if callback == 0 {
        0
    } else if callback == add(base, metadata::ELIGIBILITY_FLAGS_GETTER_RVA)? as u64 {
        1
    } else {
        return Err("naval summary modifier callback unsupported");
    };
    row.feature_flags = field(memory, base, metadata::TRADE_FEATURE_FLAGS_RVA)?;
    let own = field::<u64>(memory, side, cnaval_combatant::COMMANDER)? as usize;
    let opponent = field::<u64>(
        memory,
        combat,
        if row.is_attacker != 0 { 0x30 } else { 0x28 },
    )? as usize;
    let enemy = field::<u64>(memory, opponent, cnaval_combatant::COMMANDER)? as usize;
    for (scope, leader) in [(3, own), (4, enemy)] {
        if leader != 0 {
            if field::<u64>(memory, leader, 0)?
                != add(base, crate::cnavy_leader::VTABLE_RVA)? as u64
            {
                return Err("naval summary commander identity mismatch");
            }
            row.modifiers[scope] = values(memory, add(leader, 0x418)?)?;
        }
    }
    // Native1415B4BB0 only runs country/region/terrain branches for nonempty
    // side-country membership. Its country is the first linked-list node.
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

    /// A side at `BASE` with `count` 0x20-byte nodes after it, tag = 1000 + i.
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
        // Move the list to the secondary head; the primary list is then empty.
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
        // A previous side's countries stay in place before this span.
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
        // The live country-table header, not a chosen constant, bounds the walk.
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
        // Native count one below the linked nodes.
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
    fn raw_modifier_lookup_preserves_absence_extremes_and_rejects_unbounded_arrays() {
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
            Err("naval summary modifier bound exceeded")
        );
    }
}
