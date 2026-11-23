
pub const KIND: usize = 0x00;
pub const KIND_PRESENT: usize = 0x04;
pub const PROVINCE: usize = 0x08;
#[cfg(test)]
pub const SHIP_REFERENCE_TYPE: usize = 0x10;
#[cfg(test)]
pub const SHIP_REFERENCE_ID: usize = 0x14;
pub const COUNTRY: usize = 0x18;
pub const BUILDING: usize = 0x20;
pub const BUILDING_LOCATION: usize = 0x28;
pub const BUILDING_TYPE: usize = 0x2c;
pub const BUILDING_COUNTRY: usize = 0x30;
pub const KIND_NAVAL_BASE: u32 = 2;
pub const KIND_CARRIER: u32 = 3;
pub const KIND_SUBMARINE: u32 = 4;
pub const KIND_BUILDING: u32 = 5;

use engine_native_memory::{Memory, read};
use engine_native::{ccountry, cprovince};
use crate::ccountry_raid_status;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub kind: Option<u32>,
    pub kind_present: u8,
    pub province: u32,
    pub ship_reference_type: u32,
    pub ship_reference_id: u32,
    pub country: i32,
    pub building_type: u32,
    pub building_location: i32,
    pub building_country: i32,
}

pub(crate) fn read_row(
    memory: &impl Memory,
    base: usize,
    address: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
) -> Result<Row, &'static str> {
    let mut presence = [0_u8; 1];
    let mut location = [0_u8; BUILDING_COUNTRY + 4 - PROVINCE];
    if memory.copy_scattered(
        [
            address
                .checked_add(KIND_PRESENT)
                .ok_or("raid source address overflow")?,
            address
                .checked_add(PROVINCE)
                .ok_or("raid source address overflow")?,
        ],
        &mut [&mut presence, &mut location],
    ) != 2
    {
        return Err("raid source unreadable");
    }
    let word = |offset: usize| {
        i32::from_le_bytes(
            location[offset - PROVINCE..offset - PROVINCE + 4]
                .try_into()
                .unwrap(),
        )
    };
    let kind_present = presence[0];
    if kind_present > 1 {
        return Err("raid source presence is invalid");
    }
    let kind = if kind_present == 0 {
        None
    } else {
        Some(
            read::<u32>(memory, address)
                .filter(|&kind| kind <= KIND_BUILDING)
                .ok_or("raid source kind unreadable or invalid")?,
        )
    };
    let country = i32::from_le_bytes(location[16..20].try_into().unwrap());
    ccountry_raid_status::authenticate_country(countries, country)?;
    let province_pointer = usize::try_from(u64::from_le_bytes(location[..8].try_into().unwrap()))
        .map_err(|_| "raid source province address overflow")?;
    let province = if province_pointer == 0 {
        0
    } else {
        let expected =
            engine_sites::at(base, cprovince::VTABLE_RVA).ok_or("module address overflow")?;
        if read::<u64>(memory, province_pointer) != Some(expected as u64) {
            return Err("raid source province vtable mismatch");
        }
        province_pointer
            .checked_add(cprovince::ID)
            .and_then(|p| read::<u32>(memory, p))
            .filter(|&id| id != 0)
            .ok_or("raid source province id unreadable or zero")?
    };
    Ok(Row {
        kind,
        kind_present,
        province,
        ship_reference_type: u32::from_le_bytes(location[8..12].try_into().unwrap()),
        ship_reference_id: u32::from_le_bytes(location[12..16].try_into().unwrap()),
        country,
        building_type: word(BUILDING_TYPE) as u32,
        building_location: word(BUILDING_LOCATION),
        building_country: word(BUILDING_COUNTRY),
    })
}
