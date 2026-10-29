//! CEquipmentProductionLicense: one granted equipment licence.
//!
//! The 0x58-byte object saves lended and required CIC, owner and giver tags,
//! start date, parent variant and every produced variant in that order, without
//! conditional field gates. Loading restores the same fields.
//!
//! The i64 `UNSERIALIZED_OPERAND` defaults to 100,000 and receives the creation argument. It is not
//! serialized; its exact role remains unknown and capture preserves it raw.

use crate::{Memory, cequipment_variant, cpdx_array::Array, read};

/// CEquipmentProductionLicense primary vtable.
pub const VTABLE_RVA: usize = 0x02a3_ab38;
/// CEquipmentProductionLicense lended CIC, i32.
pub const LENDED_CIC: usize = 0x08;
/// CEquipmentProductionLicense required CIC, i32.
pub const REQUIRED_CIC: usize = 0x0c;
/// CEquipmentProductionLicense owning (licensee) raw country reference, u32.
pub const OWNER: usize = 0x10;
/// CEquipmentProductionLicense granting (licensor) raw country reference, u32.
pub const GIVER: usize = 0x14;
/// CEquipmentProductionLicense raw start-date hour, i32.
pub const START_DATE: usize = 0x20;
/// CEquipmentProductionLicense unserialized i64 operand.
pub const UNSERIALIZED_OPERAND: usize = 0x30;
/// CEquipmentProductionLicense produced-variant pointer array.
pub const EQUIPMENT: usize = 0x38;
/// Produced-variant array capacity, i32; the CPdxArray header at `EQUIPMENT` places it after the data pointer.
#[cfg(test)]
pub const EQUIPMENT_CAPACITY: usize = 0x40;
/// Produced-variant array count, i32.
#[cfg(test)]
pub const EQUIPMENT_COUNT: usize = 0x44;
/// Produced-variant pointer stride.
pub const EQUIPMENT_STRIDE: usize = 0x08;
/// CEquipmentProductionLicense parent CEquipmentVariant pointer.
pub const PARENT: usize = 0x50;
/// CEquipmentProductionLicense allocation size.
pub const SIZE: usize = 0x58;

/// One licence relation. `equipment` indexes the caller's handle arena.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Raw country reference of the licensee: the owner of this roster.
    pub owner: u32,
    /// Stored position in the owner's array; the save writes the same order.
    pub ordinal: u32,
    /// Raw country reference of the licensor.
    pub giver: u32,
    pub lended_cic: i32,
    pub required_cic: i32,
    /// Native engine date hour; no calendar conversion happens here.
    pub start_date: i32,
    /// Authenticated parent CEquipmentVariant handle, not a process pointer.
    pub parent: u64,
    /// Unserialized raw i64 (`UNSERIALIZED_OPERAND`).
    pub operand_raw: i64,
    pub equipment_start: usize,
    pub equipment_len: usize,
}

/// Reads one licence, then its ordered produced-variant handles into the
/// caller's arena. A list that does not fit yields `None`: the row is omitted
/// and counted, never published with a shortened produced list.
#[cfg(test)]
pub(crate) fn read_row(
    memory: &impl Memory,
    base: usize,
    owner: u32,
    ordinal: u32,
    pointer: usize,
    handles: &mut [u64],
    used: usize,
) -> Result<(Option<Row>, usize, usize), &'static str> {
    read_row_buffer(
        memory,
        base,
        owner,
        ordinal,
        pointer,
        &mut crate::walk::Buffer::Fixed(handles),
        used,
    )
}

pub(crate) fn read_row_buffer(
    memory: &impl Memory,
    base: usize,
    owner: u32,
    ordinal: u32,
    pointer: usize,
    handles: &mut crate::walk::Buffer<'_, u64>,
    used: usize,
) -> Result<(Option<Row>, usize, usize), &'static str> {
    let head =
        read::<[u8; 0x58]>(memory, pointer).ok_or("production licence record fields unreadable")?;
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("production licence module address overflow")?;
    if u64::from_le_bytes(head[..8].try_into().unwrap()) != expected as u64 {
        return Err("production licence vtable mismatch");
    }
    if u32::from_le_bytes(head[OWNER..OWNER + 4].try_into().unwrap()) != owner {
        return Err("production licence owner differs from its roster country");
    }
    let word = |offset: usize| u32::from_le_bytes(head[offset..offset + 4].try_into().unwrap());
    let quad = |offset: usize| u64::from_le_bytes(head[offset..offset + 8].try_into().unwrap());
    let parent = usize::try_from(quad(PARENT))
        .ok()
        .filter(|pointer| *pointer != 0)
        .ok_or("production licence parent pointer unreadable or null")?;
    let parent = cequipment_variant::identity(memory, base, parent)?;
    let array = Array::from_header(
        head[EQUIPMENT..EQUIPMENT + 16].try_into().unwrap(),
        EQUIPMENT_STRIDE,
    )
    .ok_or("production licence equipment array invalid")?;
    let wanted = array.count as usize;
    handles.ensure(
        used.checked_add(wanted)
            .ok_or("production licence handles count overflow")?,
    )?;
    if wanted > handles.len().saturating_sub(used) {
        return Ok((None, used, wanted));
    }
    for index in 0..array.count {
        let variant = array
            .element(index, EQUIPMENT_STRIDE)
            .and_then(|address| read::<u64>(memory, address))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|pointer| *pointer != 0)
            .ok_or("production licence equipment pointer unreadable or null")?;
        handles[used + index as usize] = cequipment_variant::identity(memory, base, variant)?;
    }
    Ok((
        Some(Row {
            owner,
            ordinal,
            giver: word(GIVER),
            lended_cic: word(LENDED_CIC) as i32,
            required_cic: word(REQUIRED_CIC) as i32,
            start_date: word(START_DATE) as i32,
            parent,
            operand_raw: i64::from_le_bytes(
                head[UNSERIALIZED_OPERAND..UNSERIALIZED_OPERAND + 8]
                    .try_into()
                    .unwrap(),
            ),
            equipment_start: used,
            equipment_len: wanted,
        }),
        used + wanted,
        wanted,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    const BASE: usize = 0x1000;
    const LICENSE: usize = 0x1100;
    const LIST: usize = 0x1200;
    const VARIANT: usize = 0x1300;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address - BASE..address - BASE + value.len()].copy_from_slice(value);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x400];
        put(&mut bytes, LICENSE, &(VTABLE_RVA as u64).to_le_bytes());
        put(
            &mut bytes,
            LICENSE + UNSERIALIZED_OPERAND,
            &100_000_i64.to_le_bytes(),
        );
        put(&mut bytes, LICENSE + LENDED_CIC, &i32::MIN.to_le_bytes());
        put(&mut bytes, LICENSE + REQUIRED_CIC, &i32::MAX.to_le_bytes());
        put(&mut bytes, LICENSE + OWNER, &7_u32.to_le_bytes());
        put(&mut bytes, LICENSE + GIVER, &u32::MAX.to_le_bytes());
        put(&mut bytes, LICENSE + START_DATE, &i32::MIN.to_le_bytes());
        put(
            &mut bytes,
            LICENSE + PARENT,
            &(VARIANT as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            LICENSE + EQUIPMENT,
            &(LIST as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            LICENSE + EQUIPMENT_CAPACITY,
            &2_u32.to_le_bytes(),
        );
        put(&mut bytes, LICENSE + EQUIPMENT_COUNT, &2_u32.to_le_bytes());
        put(&mut bytes, LIST, &(VARIANT as u64).to_le_bytes());
        put(&mut bytes, LIST + 8, &(VARIANT as u64).to_le_bytes());
        put(
            &mut bytes,
            VARIANT,
            &(cequipment_variant::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            VARIANT + cequipment_variant::TYPE,
            &0xfedc_ba98_0000_0046_u64.to_le_bytes(),
        );
        bytes
    }

    #[test]
    fn both_countries_signed_cic_words_and_the_ordered_list_survive() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let mut handles = [0; 4];
        let (row, used, wanted) = read_row(&memory, 0, 7, 3, LICENSE, &mut handles, 1).unwrap();
        assert_eq!(
            row.unwrap(),
            Row {
                owner: 7,
                ordinal: 3,
                giver: u32::MAX,
                lended_cic: i32::MIN,
                required_cic: i32::MAX,
                start_date: i32::MIN,
                parent: 0xfedc_ba98_0000_0046,
                operand_raw: 100_000,
                equipment_start: 1,
                equipment_len: 2,
            }
        );
        assert_eq!((used, wanted), (3, 2));
        assert_eq!(&handles[1..3], &[0xfedc_ba98_0000_0046; 2]);
    }

    #[test]
    fn a_full_arena_omits_the_row_and_broken_variants_fail_explicitly() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        assert_eq!(
            read_row(&memory, 0, 7, 0, LICENSE, &mut [0; 1], 0),
            Ok((None, 0, 2))
        );
        for (address, reason) in [
            (LICENSE, "production licence vtable mismatch"),
            (
                LICENSE + OWNER,
                "production licence owner differs from its roster country",
            ),
            (
                LICENSE + PARENT,
                "production licence parent pointer unreadable or null",
            ),
            (
                LIST + 8,
                "production licence equipment pointer unreadable or null",
            ),
        ] {
            let mut broken = fixture();
            put(&mut broken, address, &0_u64.to_le_bytes());
            assert_eq!(
                read_row(
                    &SliceMemory {
                        base: BASE,
                        bytes: &broken
                    },
                    0,
                    7,
                    0,
                    LICENSE,
                    &mut [0; 4],
                    0
                ),
                Err(reason),
                "{address:#x}"
            );
        }
        let mut broken = fixture();
        put(&mut broken, LICENSE + EQUIPMENT_COUNT, &9_u32.to_le_bytes());
        assert_eq!(
            read_row(
                &SliceMemory {
                    base: BASE,
                    bytes: &broken
                },
                0,
                7,
                0,
                LICENSE,
                &mut [0; 16],
                0
            ),
            Err("production licence equipment array invalid")
        );
        assert_eq!(
            read_row(
                &SliceMemory {
                    base: BASE,
                    bytes: &fixture()[..LICENSE - BASE]
                },
                0,
                7,
                0,
                LICENSE,
                &mut [0; 4],
                0
            ),
            Err("production licence record fields unreadable")
        );
    }
}
