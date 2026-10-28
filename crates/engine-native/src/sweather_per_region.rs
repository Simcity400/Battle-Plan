//! SWeatherPerRegion weather flags at the completed hourly boundary.
//!
//! Province publication jobs finish before region jobs are dispatched and
//! joined. Only flags displayed by the map and naval combat window are read.

use crate::{
    Memory, cpdx_array::Array, cweather_manager::walk_dense_records, memory::RecordField,
    walk::Outcome,
};

/// Region-weather vtable.
pub const VTABLE_RVA: usize = 0x029f_3f88;
/// Region-weather array stride (`cweather_manager::REGIONS`).
pub const STRIDE: usize = 0x160;
/// Region ID, i32, authenticated against its array slot.
pub const ID: usize = 0x08;
/// Embedded modifier collection.
pub const MODIFIERS: usize = 0x58;
/// Independent weather flag bytes, eight bytes apart: light rain, heavy rain, snow, blizzard,
/// sandstorm and arctic water, in that order from `FLAGS`. These flags select modifiers.
pub const FLAGS: usize = 0x108;
/// Snow flag used to accumulate province snow.
pub const SNOW: usize = 0x118;
/// Blizzard flag used to accumulate province snow.
pub const BLIZZARD: usize = 0x120;
/// Region-weather update invoked from a parallel region job.
pub const UPDATE_RVA: usize = 0x00fa_3d70;

/// The named native weather flags are independent bytes. They are retained
/// separately, without deriving a weather-kind enum or quantized intensity.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u32,
    /// The manager's snow ceiling, stamped by `cweather_manager::Reader`.
    pub max_snow: i64,
    pub light_rain: u8,
    pub heavy_rain: u8,
    pub snow: u8,
    pub blizzard: u8,
    pub sandstorm: u8,
    pub arctic_water: u8,
}

/// Identity and the six flag bytes, gathered per batch.
pub const FIELDS: [RecordField; 7] = {
    let mut fields = [RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 12,
    }; 7];
    let mut flag = 0;
    while flag < 6 {
        fields[flag + 1] = RecordField {
            source_offset: FLAGS + flag * 8,
            destination_offset: 12 + flag,
            len: 1,
        };
        flag += 1;
    }
    fields
};
pub const RECORD_BYTES: usize = 18;

pub(crate) fn walk(memory: &impl Memory, base: usize, array: Array, output: &mut [Row]) -> Outcome {
    let Some(vtable) = base.checked_add(VTABLE_RVA) else {
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

/// Decode one owned exact-field gather, retaining native field-failure order.
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
    use crate::memory::SliceMemory;

    #[test]
    fn independent_flags_include_slot_zero() {
        let mut bytes = [0; STRIDE];
        bytes[..8].copy_from_slice(&(VTABLE_RVA as u64).to_le_bytes());
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
    fn row_fault_and_wrong_identity_never_publish_a_complete_prefix() {
        let mut bytes = [0; STRIDE * 2];
        for offset in [0, STRIDE] {
            bytes[offset..offset + 8].copy_from_slice(&(VTABLE_RVA as u64).to_le_bytes());
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
