//! Inline non-polymorphic raid source. Ship identity is a full type/ID
//! reference, distinct from source location.

/// Four-byte source kind, used only when its presence byte is set.
pub const KIND: usize = 0x00;
/// Source-kind presence byte.
pub const KIND_PRESENT: usize = 0x04;
/// Province pointer initialized to null and serialized by province ID.
pub const PROVINCE: usize = 0x08;
/// Ship reference type, u32, initialized and serialized with the ID word.
#[cfg(test)]
pub const SHIP_REFERENCE_TYPE: usize = 0x10;
/// Ship reference ID, u32.
#[cfg(test)]
pub const SHIP_REFERENCE_ID: usize = 0x14;
/// Country, i32, initialized independently of kind and saved only if positive.
pub const COUNTRY: usize = 0x18;
/// Embedded building reference, serialized only when set.
pub const BUILDING: usize = 0x20;
/// Building location, i32: province/state by building type, -1 for none.
/// The reference is set when location is present or country is positive.
pub const BUILDING_LOCATION: usize = 0x28;
/// Building reference type token inside the `BUILDING` reference, 0x4c17 for none; saved as `building.template`.
pub const BUILDING_TYPE: usize = 0x2c;
/// Building reference i32 country inside the `BUILDING` reference, positive for a country-scoped building.
pub const BUILDING_COUNTRY: usize = 0x30;
/// CRaidSource kind 2 is naval_base.
pub const KIND_NAVAL_BASE: u32 = 2;
/// CRaidSource kind 3 is carrier.
pub const KIND_CARRIER: u32 = 3;
/// CRaidSource kind 4 is submarine.
pub const KIND_SUBMARINE: u32 = 4;
/// CRaidSource kind 5 is building.
pub const KIND_BUILDING: u32 = 5;

use crate::{Memory, ccountry, ccountry_raid_status, cprovince, read};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Read only when the native presence byte is set.
    pub kind: Option<u32>,
    pub kind_present: u8,
    /// Zero means the native province pointer was null.
    pub province: u32,
    pub ship_reference_type: u32,
    pub ship_reference_id: u32,
    /// Native signed handle, including nonpositive absence values.
    pub country: i32,
    /// The building reference ([`BUILDING_TYPE`], [`BUILDING_LOCATION`],
    /// [`BUILDING_COUNTRY`]), as stored whatever the kind.
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
    // Province pointer through the building reference's country.
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
        let expected = base
            .checked_add(cprovince::VTABLE_RVA)
            .ok_or("module address overflow")?;
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
