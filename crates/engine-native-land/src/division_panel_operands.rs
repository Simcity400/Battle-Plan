use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native::{
    carmy::{self, DivisionSeed},
    cequipment_variant::{self, pool as cequipment_variant_pool},
    cgame_state,
    cprovince,
    cstate,
};
use engine_native_parts::equipment_entry;
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
pub fn selected_province(
    memory: &impl Memory,
    game_state: usize,
    seed: &DivisionSeed,
) -> Result<usize, &'static str> {
    selected_province_in(memory, &mut None, game_state, seed)
}

pub fn selected_province_in(
    memory: &impl Memory,
    roster: &mut Option<Array>,
    game_state: usize,
    seed: &DivisionSeed,
) -> Result<usize, &'static str> {
    let route = seed
        .ordinary_route
        .ok_or("attrition ordinary route unavailable")?;
    let array = Array::from_header(route, 4).ok_or("attrition ordinary route invalid")?;
    let first = if array.count > 0 {
        let id = match seed.route_head {
            Some(head) => head,
            None => read::<u32>(memory, array.data),
        }
        .ok_or("attrition first route id unreadable")?;
        let provinces = match *roster {
            Some(provinces) => provinces,
            None => {
                let provinces = Array::read(
                    memory,
                    game_state
                        .checked_add(cgame_state::PROVINCES)
                        .ok_or("attrition province roster overflow")?,
                    8,
                )
                .ok_or("attrition province roster invalid")?;
                *roster = Some(provinces);
                provinces
            }
        };
        let province = read::<u64>(
            memory,
            provinces
                .element(id, 8)
                .ok_or("attrition route province outside roster")?,
        )
        .ok_or("attrition route province unreadable")? as usize;
        if province != 0 && read::<u32>(memory, province + cprovince::ID) != Some(id) {
            return Err("attrition route province identity mismatch");
        }
        province
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
    if engine_sites::at(base, cprovince::VTABLE_RVA)
        .is_none_or(|vtable| read::<u64>(memory, province) != Some(vtable as u64))
    {
        return Err("attrition province identity mismatch");
    }
    let state = read::<u64>(memory, province + cprovince::STATE)
        .ok_or("attrition state pointer unreadable")? as usize;
    if state == 0 {
        return Ok(None);
    }
    if engine_sites::at(base, cstate::VTABLE_RVA)
        .is_none_or(|vtable| read::<u64>(memory, state) != Some(vtable as u64))
    {
        return Err("attrition state identity mismatch");
    }
    let id = read(memory, state + cstate::ID).ok_or("attrition state identity unreadable")?;
    Ok(Some((id, state)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
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
        put(
            &mut bytes,
            pool,
            cequipment_variant_pool::VTABLE_RVA.0 as u64,
        );
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
        put(&mut bytes, variant, cequipment_variant::VTABLE_RVA.0 as u64);
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

    #[test]
    fn route_ids_resolve_through_the_province_roster() {
        let mut bytes = vec![0u8; 0x2000];
        let (state, roster, ids, province) = (0x100, 0x800, 0x900, 0x1000);
        let header = |data: usize, count: u32| {
            let mut header = [0u8; 16];
            header[..8].copy_from_slice(&(data as u64).to_le_bytes());
            header[8..12].copy_from_slice(&count.to_le_bytes());
            header[12..].copy_from_slice(&count.to_le_bytes());
            header
        };
        let at = state + cgame_state::PROVINCES;
        bytes[at..at + 16].copy_from_slice(&header(roster, 8));
        bytes[roster + 7 * 8..roster + 8 * 8].copy_from_slice(&(province as u64).to_le_bytes());
        bytes[ids..ids + 8].copy_from_slice(&(7u64 | (3 << 32)).to_le_bytes());
        bytes[province + cprovince::ID..province + cprovince::ID + 4]
            .copy_from_slice(&7u32.to_le_bytes());
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut seed = DivisionSeed {
            ordinary_route: Some(header(ids, 2)),
            location: Some(0x1800),
            ..Default::default()
        };
        assert_eq!(selected_province(&memory, state, &seed), Ok(province));
        bytes[province + cprovince::ID] = 8;
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            selected_province(&memory, state, &seed),
            Err("attrition route province identity mismatch")
        );
        seed.ordinary_route = Some(header(ids, 0));
        assert_eq!(selected_province(&memory, state, &seed), Ok(0x1800));
    }

    #[test]
    fn a_gathered_route_head_and_one_roster_header_serve_every_mover() {
        struct Reads<'a>(SliceMemory<'a>, std::cell::RefCell<Vec<usize>>);
        impl Memory for Reads<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.1.borrow_mut().push(address);
                self.0.copy(address, destination)
            }
        }
        let mut bytes = vec![0u8; 0x2000];
        let (state, roster, province) = (0x100, 0x800, 0x1000);
        let at = state + cgame_state::PROVINCES;
        bytes[at..at + 8].copy_from_slice(&(roster as u64).to_le_bytes());
        bytes[at + 8..at + 12].copy_from_slice(&8u32.to_le_bytes());
        bytes[at + 12..at + 16].copy_from_slice(&8u32.to_le_bytes());
        bytes[roster + 7 * 8..roster + 8 * 8].copy_from_slice(&(province as u64).to_le_bytes());
        bytes[province + cprovince::ID..province + cprovince::ID + 4]
            .copy_from_slice(&7u32.to_le_bytes());
        let memory = Reads(
            SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            Default::default(),
        );
        let mut route = [0u8; 16];
        route[..8].copy_from_slice(&0x9000u64.to_le_bytes());
        route[8..12].copy_from_slice(&2u32.to_le_bytes());
        route[12..].copy_from_slice(&2u32.to_le_bytes());
        let mut seed = DivisionSeed {
            ordinary_route: Some(route),
            route_head: Some(Some(7)),
            location: Some(0x1800),
            ..Default::default()
        };
        let mut held = None;
        for _ in 0..3 {
            assert_eq!(
                selected_province_in(&memory, &mut held, state, &seed),
                Ok(province)
            );
        }
        let reads = memory.1.borrow();
        assert_eq!(reads.iter().filter(|&&address| address == at).count(), 1);
        assert!(!reads.contains(&0x9000));
        drop(reads);
        seed.route_head = Some(None);
        assert_eq!(
            selected_province_in(&memory, &mut held, state, &seed),
            Err("attrition first route id unreadable")
        );
        assert!(!memory.1.borrow().contains(&0x9000));
    }
}
