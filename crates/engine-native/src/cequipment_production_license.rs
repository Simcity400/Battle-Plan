
use crate::{Memory, cequipment_variant, cpdx_array::Array, read};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x02a3_b258);
pub const LENDED_CIC: usize = 0x08;
pub const REQUIRED_CIC: usize = 0x0c;
pub const OWNER: usize = 0x10;
pub const GIVER: usize = 0x14;
pub const START_DATE: usize = 0x20;
pub const UNSERIALIZED_OPERAND: usize = 0x30;
pub const EQUIPMENT: usize = 0x38;
#[cfg(test)]
pub const EQUIPMENT_CAPACITY: usize = 0x40;
#[cfg(test)]
pub const EQUIPMENT_COUNT: usize = 0x44;
pub const EQUIPMENT_STRIDE: usize = 0x08;
pub const PARENT: usize = 0x50;
pub const SIZE: usize = 0x58;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub owner: u32,
    pub ordinal: u32,
    pub giver: u32,
    pub lended_cic: i32,
    pub required_cic: i32,
    pub start_date: i32,
    pub parent: u64,
    pub operand_raw: i64,
    pub equipment_start: usize,
    pub equipment_len: usize,
}

#[cfg(test)]
pub(crate) fn read_row(
    memory: &impl Memory,
    base: usize,
    owner: u32,
    ordinal: u32,
    pointer: usize,
    handles: &mut Vec<u64>,
    used: usize,
) -> Result<(Option<Row>, usize, usize), &'static str> {
    read_row_buffer(
        memory,
        base,
        owner,
        ordinal,
        pointer,
        &mut crate::walk::Buffer::Growing(handles),
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
    let expected =
        engine_sites::at(base, VTABLE_RVA).ok_or("production licence module address overflow")?;
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
        put(&mut bytes, LICENSE, &(VTABLE_RVA.0 as u64).to_le_bytes());
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
            &(cequipment_variant::VTABLE_RVA.0 as u64).to_le_bytes(),
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
        let mut handles = vec![0; 4];
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
    fn broken_licence_fields_fail_explicitly() {
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
                    &mut vec![0; 4],
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
                &mut vec![0; 16],
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
                &mut vec![0; 4],
                0
            ),
            Err("production licence record fields unreadable")
        );
    }
}
