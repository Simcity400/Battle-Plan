
use engine_native_memory::{Memory, cpdx_array::Array, memory::RecordField, walk::Outcome};
use crate::cweather_manager::walk_dense_records;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029f_40e8);
pub const STRIDE: usize = 0x160;
pub const ID: usize = 0x08;
pub const CUSTOM_SEA: usize = 0x30;
pub const MODIFIERS: usize = 0x58;
pub const FLAGS: usize = 0x108;
pub const SNOW: usize = 0x118;
pub const BLIZZARD: usize = 0x120;
pub const UPDATE_RVA: Rva = Rva(0x00fa_3de0);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u32,
    pub max_snow: i64,
    pub light_rain: u8,
    pub heavy_rain: u8,
    pub snow: u8,
    pub blizzard: u8,
    pub sandstorm: u8,
    pub arctic_water: u8,
}

pub const FIELDS: [RecordField; 8] = {
    let mut fields = [RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 12,
    }; 8];
    let mut flag = 0;
    while flag < 6 {
        fields[flag + 1] = RecordField {
            source_offset: FLAGS + flag * 8,
            destination_offset: 12 + flag,
            len: 1,
        };
        flag += 1;
    }
    fields[7] = RecordField {
        source_offset: CUSTOM_SEA,
        destination_offset: CUSTOM_SEA_HEADER,
        len: 16,
    };
    fields
};
const CUSTOM_SEA_HEADER: usize = 18;
pub const RECORD_BYTES: usize = 34;

pub fn custom_sea(record: &[u8]) -> Option<Array> {
    Array::from_header(
        record
            .get(CUSTOM_SEA_HEADER..CUSTOM_SEA_HEADER + 16)?
            .try_into()
            .ok()?,
        2,
    )
}

pub(crate) fn walk(memory: &impl Memory, base: usize, array: Array, output: &mut [Row]) -> Outcome {
    let Some(vtable) = engine_sites::at(base, VTABLE_RVA) else {
        return Outcome::Unavailable {
            reason: "module address overflow",
        };
    };
    walk_dense_records(
        memory,
        array,
        STRIDE,
        &FIELDS,
        RECORD_BYTES,
        output,
        |index, record, copied| decode_record(index, record, copied, vtable),
    )
}

pub fn decode_record(
    index: u32,
    record: &[u8],
    copied: usize,
    vtable: usize,
) -> Result<Row, &'static str> {
    if record.len() < RECORD_BYTES {
        return Err("weather region fields unreadable");
    }
    if copied == 0 {
        return Err("weather region identity unreadable");
    }
    if u64::from_le_bytes(record[..8].try_into().unwrap()) != vtable as u64 {
        return Err("weather region vtable mismatch");
    }
    let id = u32::from_le_bytes(record[8..12].try_into().unwrap());
    if id != index {
        return Err("weather region id differs from slot");
    }
    if copied < FIELDS.len() {
        return Err("weather region fields unreadable");
    }
    Ok(Row {
        id,
        max_snow: 0,
        light_rain: record[12],
        heavy_rain: record[13],
        snow: record[14],
        blizzard: record[15],
        sandstorm: record[16],
        arctic_water: record[17],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    #[test]
    fn independent_flags_include_slot_zero() {
        let mut bytes = [0; STRIDE];
        bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        for (flag, value) in [1, 0, 255, 1, 2, 3].into_iter().enumerate() {
            bytes[FLAGS + flag * 8] = value;
        }
        let mut rows = [Row::default(); 1];
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                Array {
                    data: 0x1000,
                    count: 1
                },
                &mut rows
            ),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(
            rows[0],
            Row {
                id: 0,
                max_snow: 0,
                light_rain: 1,
                heavy_rain: 0,
                snow: 255,
                blizzard: 1,
                sandstorm: 2,
                arctic_water: 3
            }
        );
    }

    #[test]
    fn the_custom_sea_header_rides_the_record_gather() {
        let mut bytes = [0; STRIDE];
        bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[CUSTOM_SEA..CUSTOM_SEA + 8].copy_from_slice(&0x9000_u64.to_le_bytes());
        bytes[CUSTOM_SEA + 8..CUSTOM_SEA + 12].copy_from_slice(&4_u32.to_le_bytes());
        bytes[CUSTOM_SEA + 12..CUSTOM_SEA + 16].copy_from_slice(&3_u32.to_le_bytes());
        let mut record = [0; RECORD_BYTES];
        let layout = engine_native_memory::memory::RecordLayout::new(&FIELDS, RECORD_BYTES).unwrap();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let copied = memory.copy_records(
            engine_native_memory::memory::RecordSources::Strided {
                base: 0x1000,
                stride: STRIDE,
                count: 1,
            },
            &layout,
            &mut record,
        );
        assert_eq!(copied, FIELDS.len());
        assert!(decode_record(0, &record, copied, VTABLE_RVA.0 as usize).is_ok());
        assert_eq!(
            custom_sea(&record),
            Some(Array {
                data: 0x9000,
                count: 3
            })
        );
        record[CUSTOM_SEA_HEADER + 12..CUSTOM_SEA_HEADER + 16].copy_from_slice(&5_u32.to_le_bytes());
        assert_eq!(custom_sea(&record), None);
        assert_eq!(
            decode_record(0, &record, FIELDS.len() - 1, VTABLE_RVA.0 as usize),
            Err("weather region fields unreadable")
        );
    }

    #[test]
    fn row_fault_and_wrong_identity_never_publish_a_complete_prefix() {
        let mut bytes = [0; STRIDE * 2];
        for offset in [0, STRIDE] {
            bytes[offset..offset + 8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        }
        let array = Array {
            data: 0x1000,
            count: 2,
        };
        let mut rows = [Row::default(); 2];
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                array,
                &mut rows
            ),
            Outcome::Unavailable {
                reason: "weather region id differs from slot"
            }
        );
        bytes[STRIDE + ID..STRIDE + ID + 4].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes[..STRIDE + FLAGS + 5 * 8]
                },
                0,
                array,
                &mut rows
            ),
            Outcome::Unavailable {
                reason: "weather region fields unreadable"
            }
        );
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes[..STRIDE]
                },
                0,
                array,
                &mut rows[..1]
            ),
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
    }
}
