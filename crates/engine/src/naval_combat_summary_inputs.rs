//! Raw side operands for positioning141609460, screening14160A7B0 and
//! carrier crowding14160C430. Ship/plane counts and sums remain viewer work.
use crate::{
    Memory, ccountry, cnaval_combatant, cpdx_array::Array, modifier_accumulator as accumulator,
    modifier_definition as metadata,
};
use cnaval_combatant::field;

pub const COUNTRY_LIMIT: usize = 64;
/// Ordered country linked list;1417DA860/1406D7F90 iterate node+10.
pub const COUNTRIES: usize = 0x38;
pub const COUNTRY_COUNT: usize = 0x48;
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
pub const MODIFIER_KEYS: [u32; 5] = [0x5a, 0x1b1, 0x1b2, 0x1b3, 0x1d0];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Modifier {
    pub raw: i64,
    pub present: u8,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub recent_join_penalty: i64,
    pub escort_bonus: i64,
    pub is_attacker: u8,
    pub countries: [u32; COUNTRY_LIMIT],
    pub country_count: usize,
    /// Country, regional weather, sea terrain, own admiral, opposing admiral.
    pub modifiers: [[Modifier; 5]; 5],
    /// Per key: raw flags and feature selector.
    pub modifier_metadata: [[u32; 2]; 5],
    pub feature_flags: u32,
    pub eligibility_callback: u8,
    pub fixed_defines: [i64; 18],
    pub integer_defines: [i32; 3],
}
impl Default for Row {
    fn default() -> Self {
        Self {
            recent_join_penalty: 0,
            escort_bonus: 0,
            is_attacker: 0,
            countries: [0; COUNTRY_LIMIT],
            country_count: 0,
            modifiers: [[Modifier::default(); 5]; 5],
            modifier_metadata: [[0; 2]; 5],
            feature_flags: 0,
            eligibility_callback: 0,
            fixed_defines: [0; 18],
            integer_defines: [0; 3],
        }
    }
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

/// Caller has authenticated combat and both side back-links. Retain source
/// words without calling engine code.
pub fn read(
    memory: &impl Memory,
    base: usize,
    state: usize,
    combat: usize,
    side: usize,
) -> Result<Row, &'static str> {
    let head = field::<u64>(memory, side, COUNTRIES)? as usize;
    let count = field::<i32>(memory, side, COUNTRY_COUNT)?;
    if count < 0 || count as usize > COUNTRY_LIMIT {
        return Err("naval summary country bound exceeded");
    }
    let mut row = Row {
        recent_join_penalty: field(memory, side, RECENT_JOIN)?,
        escort_bonus: field(memory, side, ESCORT)?,
        is_attacker: field(memory, side, 0xb8)?,
        country_count: count as usize,
        ..Default::default()
    };
    let mut node = head;
    for out in &mut row.countries[..row.country_count] {
        if node == 0 {
            return Err("naval summary country list ended early");
        }
        *out = field(memory, node, 0)?;
        node = field::<u64>(memory, node, 0x10)? as usize;
    }
    if node != 0 {
        return Err("naval summary country list exceeds native count");
    }
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
    if count != 0 {
        let country = ccountry::Reader::new(memory, base, state)?.resolve(row.countries[0])?;
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
