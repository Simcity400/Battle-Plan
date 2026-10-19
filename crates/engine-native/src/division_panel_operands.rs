//! Read-only operands demanded by deployed fresh attrition.
use crate::{
    Memory, carmy, carmy::DivisionSeed, cequipment_variant, cequipment_variant_pool,
    cpdx_array::Array, cprovince, cstate, equipment_entry, read,
};
pub const VARIANT_TERRAIN_ATTRITION: usize = 0x3a0;
pub fn held_variants(
    memory: &impl Memory,
    base: usize,
    seed: &DivisionSeed,
    mut visit: impl FnMut(equipment_entry::Row) -> Result<(), &'static str>,
) -> Result<u32, &'static str> {
    let pool = seed
        .pointer
        .checked_add(carmy::EQUIPMENT_POOL)
        .ok_or("attrition membership pool overflow")?;
    let entries = cequipment_variant_pool::entries(memory, base, pool)?;
    for ordinal in 0..entries.count {
        let address = entries
            .element(ordinal, equipment_entry::STRIDE)
            .ok_or("attrition membership slot overflow")?;
        let raw =
            read::<[u8; 16]>(memory, address).ok_or("attrition membership entry unreadable")?;
        let amount_raw = i64::from_le_bytes(raw[8..].try_into().unwrap());
        if amount_raw != 0 {
            let variant_pointer = u64::from_le_bytes(raw[..8].try_into().unwrap()) as usize;
            let variant_handle = cequipment_variant::identity(memory, base, variant_pointer)?;
            visit(equipment_entry::Row {
                source_ordinal: ordinal,
                variant_handle,
                variant_pointer,
                amount_raw,
            })?;
        }
    }
    Ok(entries.count)
}
pub fn terrain(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    handle: u64,
    mut visit: impl FnMut(u32, i64) -> Result<(), &'static str>,
) -> Result<u32, &'static str> {
    if cequipment_variant::identity(memory, base, pointer)? != handle {
        return Err("attrition variant identity changed");
    }
    let array = Array::read(
        memory,
        pointer
            .checked_add(VARIANT_TERRAIN_ATTRITION)
            .ok_or("attrition variant terrain overflow")?,
        8,
    )
    .ok_or("attrition variant terrain vector unreadable")?;
    for ordinal in 0..array.count {
        visit(
            ordinal,
            read(
                memory,
                array
                    .element(ordinal, 8)
                    .ok_or("attrition variant terrain slot overflow")?,
            )
            .ok_or("attrition variant terrain value unreadable")?,
        )?;
    }
    Ok(array.count)
}
/// The first nonnull ordinary route province, otherwise the held location.
pub fn selected_province(memory: &impl Memory, seed: &DivisionSeed) -> Result<usize, &'static str> {
    let route = seed
        .ordinary_route
        .ok_or("attrition ordinary route unavailable")?;
    let array = Array::from_header(route, 8).ok_or("attrition ordinary route invalid")?;
    let first = if array.count > 0 {
        read::<u64>(memory, array.data).ok_or("attrition route province unreadable")? as usize
    } else {
        0
    };
    let province = if first != 0 {
        first
    } else {
        seed.location.ok_or("attrition location unavailable")?
    };
    Ok(province)
}
pub fn province_state(
    memory: &impl Memory,
    base: usize,
    province: usize,
) -> Result<Option<(u32, usize)>, &'static str> {
    if province == 0 {
        return Ok(None);
    }
    if read::<u64>(memory, province) != Some((base + cprovince::VTABLE_RVA) as u64) {
        return Err("attrition province identity mismatch");
    }
    let state = read::<u64>(memory, province + cprovince::STATE)
        .ok_or("attrition state pointer unreadable")? as usize;
    if state == 0 {
        return Ok(None);
    }
    if read::<u64>(memory, state) != Some((base + cstate::VTABLE_RVA) as u64) {
        return Err("attrition state identity mismatch");
    }
    let id = read(memory, state + cstate::ID).ok_or("attrition state identity unreadable")?;
    Ok(Some((id, state)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;
    #[test]
    fn zero_quantity_does_not_touch_a_variant_and_negative_quantity_is_held() {
        let mut bytes = vec![0u8; 0x2000];
        let army = 0x100;
        let pool = army + carmy::EQUIPMENT_POOL;
        let entries = 0x900;
        let variant = 0x1000;
        let put = |bytes: &mut [u8], at: usize, value: u64| {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes())
        };
        put(&mut bytes, pool, cequipment_variant_pool::VTABLE_RVA as u64);
        put(
            &mut bytes,
            pool + cequipment_variant_pool::ITEMS,
            entries as u64,
        );
        bytes[pool + cequipment_variant_pool::COUNT..pool + cequipment_variant_pool::COUNT + 4]
            .copy_from_slice(&2u32.to_le_bytes());
        bytes[pool + cequipment_variant_pool::CAPACITY
            ..pool + cequipment_variant_pool::CAPACITY + 4]
            .copy_from_slice(&2u32.to_le_bytes());
        put(&mut bytes, entries, u64::MAX);
        put(&mut bytes, entries + 16, variant as u64);
        put(&mut bytes, entries + 24, (-2i64) as u64);
        put(&mut bytes, variant, cequipment_variant::VTABLE_RVA as u64);
        put(
            &mut bytes,
            variant + 8,
            (19u64 << 32) | cequipment_variant::REFERENCE_TYPE as u64,
        );
        let seed = DivisionSeed {
            pointer: army,
            ..Default::default()
        };
        let mut seen = Vec::new();
        assert_eq!(
            held_variants(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                &seed,
                |row| {
                    seen.push(row);
                    Ok(())
                }
            ),
            Ok(2)
        );
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].source_ordinal, 1);
        assert_eq!(seen[0].amount_raw, -2);
    }
}
