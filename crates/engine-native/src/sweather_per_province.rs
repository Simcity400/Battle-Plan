//! SWeatherPerProvince snow, temperature words and mud at the completed hourly boundary, with
//! the manager's per-province display bytes and stored day/night clock.
//!
//! Normal staging jobs finish before publication jobs, then display bytes
//! are written serially. Provinces update in rotating subsets; each row
//! preserves its stored state.

use crate::{
    Memory,
    cpdx_array::Array,
    cweather_manager::walk_dense_records,
    memory::{RecordField, RecordLayout, RecordSources},
    read_slice,
    walk::Outcome,
};

/// Province-weather vtable.
pub const VTABLE_RVA: usize = 0x0296_0b40;
/// Province-weather array stride.
pub const STRIDE: usize = 0x180;
/// Province ID, i32, authenticated against its array slot.
pub const ID: usize = 0x08;
/// Permanent snow, signed i64 fixed5, written during load setup and
/// retained at hourly boundaries.
pub const PERMANENT_SNOW: usize = 0x30;
/// Snow, i64 fixed5, clamped and stored at full width.
pub const SNOW: usize = 0x40;
/// Embedded modifier collection at +0x58.
pub const MODIFIERS: usize = 0x58;
/// Temperature, signed i64 fixed5, used for truncated numeric text and
/// temperature classification. Serialization retains all eight bytes,
/// including zero.
pub const TEMPERATURE: usize = 0x118;
/// Committed drift, signed i64 fixed5: weather smoothing averages the active
/// neighbours' words and weather modifier tasks commit the working drift
/// +0x170 into it with the staged temperature.
pub const DRIFT: usize = 0x128;
/// Raw mud byte tested by the province tooltip.
/// Display updating writes display mud from this OR custom mud +0x138, so it is
/// independent.
pub const RAW_MUD: usize = 0x130;
/// Staged temperature, signed i64 fixed5: province weather calculation stores
/// +0x118 plus the clamped working drift and weather smoothing then smooths it;
/// weather modifier tasks commit it into +0x118 when +0x168 is set.
pub const STAGED_TEMPERATURE: usize = 0x160;
/// Working drift, signed i64 fixed5: province weather calculation adds the
/// hashed step, clamps it and resets it outside the period's range.
pub const WORKING_DRIFT: usize = 0x170;
/// Separate day/night row stride; day and night getters index the
/// CWeatherManager+0x370 array by this physical province slot.
pub const CLOCK_STRIDE: usize = 0x330;
/// Stored night factor, signed i64 fixed5 at +0x240, updated by day/night
/// processing and read at full width.
pub const CLOCK_NIGHT_FACTOR: usize = 0x240;
/// Stored local hour, signed i32 at +0x260. Suspension may retain zero
/// after reset.
pub const CLOCK_LOCAL_HOUR: usize = 0x260;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u32,
    pub snow: i64,
    pub permanent_snow: i64,
    /// `CWeatherManager+0x310[id]`, filled by [`fill_display`].
    pub display_snow: u8,
    /// `CWeatherManager+0x328[id]`, filled by [`fill_display`].
    pub display_mud: u8,
    pub raw_mud: u8,
    pub temperature: i64,
    pub drift: i64,
    pub staged_temperature: i64,
    pub working_drift: i64,
    pub local_hour: i32,
    pub night_factor: i64,
}

/// Identity and stored weather values, gathered per batch without padding.
pub const FIELDS: [RecordField; 8] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 12,
    },
    RecordField {
        source_offset: SNOW,
        destination_offset: 12,
        len: 8,
    },
    RecordField {
        source_offset: PERMANENT_SNOW,
        destination_offset: 20,
        len: 8,
    },
    RecordField {
        source_offset: TEMPERATURE,
        destination_offset: 28,
        len: 8,
    },
    RecordField {
        source_offset: RAW_MUD,
        destination_offset: 36,
        len: 1,
    },
    RecordField {
        source_offset: DRIFT,
        destination_offset: 37,
        len: 8,
    },
    RecordField {
        source_offset: STAGED_TEMPERATURE,
        destination_offset: 45,
        len: 8,
    },
    RecordField {
        source_offset: WORKING_DRIFT,
        destination_offset: 53,
        len: 8,
    },
];
pub const RECORD_BYTES: usize = 61;

/// Only the two stored clock words: no row identity or dense padding exists
/// in this capture. The authenticated manager and equal live counts own the join.
pub const CLOCK_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: CLOCK_NIGHT_FACTOR,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: CLOCK_LOCAL_HOUR,
        destination_offset: 8,
        len: 4,
    },
];
pub const CLOCK_RECORD_BYTES: usize = 12;

/// Decode the same exact-field record on the typed path and owned-copy worker.
/// Commit both words only after the whole record succeeded; preserve raw values.
pub fn fill_clock_record(record: &[u8], copied: usize, row: &mut Row) -> Result<(), &'static str> {
    if record.len() < CLOCK_RECORD_BYTES || copied < CLOCK_FIELDS.len() {
        return Err("weather clock fields unreadable");
    }
    row.night_factor = i64::from_le_bytes(record[..8].try_into().unwrap());
    row.local_hour = i32::from_le_bytes(record[8..12].try_into().unwrap());
    Ok(())
}

/// Join the admitted weather prefix to the equally sized native clock array.
/// Exact fields are copied in bounded batches without allocating at the hook.
pub fn fill_clocks(
    memory: &impl Memory,
    array: Array,
    provinces: u32,
    rows: &mut [Row],
) -> Result<(), &'static str> {
    if array.count != provinces || rows.len() > provinces as usize {
        return Err("weather clock array differs from the province array");
    }
    const BATCH: usize = 64;
    const LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&CLOCK_FIELDS, CLOCK_RECORD_BYTES).expect("valid clock layout");
    let mut bytes = [0; BATCH * CLOCK_RECORD_BYTES];
    for (batch, destination) in rows.chunks_mut(BATCH).enumerate() {
        let sources = RecordSources::Strided {
            base: array
                .element((batch * BATCH) as u32, CLOCK_STRIDE)
                .unwrap_or(0),
            stride: CLOCK_STRIDE,
            count: destination.len(),
        };
        let copied = memory.copy_records(
            sources,
            &LAYOUT,
            &mut bytes[..destination.len() * CLOCK_RECORD_BYTES],
        );
        for (index, row) in destination.iter_mut().enumerate() {
            fill_clock_record(
                &bytes[index * CLOCK_RECORD_BYTES..(index + 1) * CLOCK_RECORD_BYTES],
                copied
                    .saturating_sub(index * CLOCK_FIELDS.len())
                    .min(CLOCK_FIELDS.len()),
                row,
            )?;
        }
    }
    Ok(())
}

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
        return Err("weather province fields unreadable");
    }
    if copied == 0 {
        return Err("weather province identity unreadable");
    }
    if u64::from_le_bytes(record[..8].try_into().unwrap()) != vtable as u64 {
        return Err("weather province vtable mismatch");
    }
    let id = u32::from_le_bytes(record[8..12].try_into().unwrap());
    if id != index {
        return Err("weather province id differs from slot");
    }
    if copied < FIELDS.len() {
        return Err("weather province fields unreadable");
    }
    Ok(Row {
        id,
        snow: i64::from_le_bytes(record[12..20].try_into().unwrap()),
        permanent_snow: i64::from_le_bytes(record[20..28].try_into().unwrap()),
        temperature: i64::from_le_bytes(record[28..36].try_into().unwrap()),
        raw_mud: record[36],
        drift: i64::from_le_bytes(record[37..45].try_into().unwrap()),
        staged_temperature: i64::from_le_bytes(record[45..53].try_into().unwrap()),
        working_drift: i64::from_le_bytes(record[53..61].try_into().unwrap()),
        ..Row::default()
    })
}

/// Copies the two display byte arrays into the rows, which hold slots
/// `0..rows.len()` of an array of `provinces` rows. Both arrays are sized to
/// the province count, so any other count refuses the table.
/// Whole arrays are read in fixed chunks, without allocating.
pub fn fill_display(
    memory: &impl Memory,
    [snow, mud]: [Array; 2],
    provinces: u32,
    rows: &mut [Row],
) -> Result<(), &'static str> {
    if snow.count != provinces || mud.count != provinces {
        return Err("weather display arrays differ from the province array");
    }
    let mut chunk = [0_u8; 1024];
    for (array, is_snow) in [(snow, true), (mud, false)] {
        for start in (0..provinces as usize).step_by(chunk.len()) {
            let bytes = &mut chunk[..(provinces as usize - start).min(1024)];
            if !read_slice(memory, array.data + start, bytes) {
                return Err("weather display bytes unreadable");
            }
            for (row, &byte) in rows.iter_mut().skip(start).zip(bytes.iter()) {
                if is_snow {
                    row.display_snow = byte;
                } else {
                    row.display_mud = byte;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
