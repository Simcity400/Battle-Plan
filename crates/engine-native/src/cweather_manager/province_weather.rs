
use engine_sites::Rva;
use crate::{
    Memory,
    cpdx_array::Array,
    cweather_manager::walk_dense_records,
    memory::{RecordField, RecordLayout, RecordSources},
    read_slice,
    walk::Outcome,
};

pub const VTABLE_RVA: Rva = Rva(0x029f_4098);
pub const STRIDE: usize = 0x180;
pub const ID: usize = 0x08;
pub const PERMANENT_SNOW: usize = 0x30;
pub const SNOW: usize = 0x40;
pub const MODIFIERS: usize = 0x58;
pub const TEMPERATURE: usize = 0x118;
pub const DRIFT: usize = 0x128;
pub const RAW_MUD: usize = 0x130;
pub const STATIC_MODIFIERS: usize = 0x148;
pub const STATIC_MODIFIER_STRIDE: usize = 0x10;
pub const STATIC_MODIFIER_DAYS: usize = 0x08;
pub const STAGED_TEMPERATURE: usize = 0x160;
pub const WORKING_DRIFT: usize = 0x170;
pub const CLOCK_STRIDE: usize = 0x330;
pub const CLOCK_NIGHT_FACTOR: usize = 0x240;
pub const CLOCK_LOCAL_HOUR: usize = 0x260;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u32,
    pub snow: i64,
    pub permanent_snow: i64,
    pub display_snow: u8,
    pub display_mud: u8,
    pub raw_mud: u8,
    pub temperature: i64,
    pub drift: i64,
    pub staged_temperature: i64,
    pub working_drift: i64,
    pub local_hour: i32,
    pub night_factor: i64,
}

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

pub const STATIC_MODIFIER_HEADER: [RecordField; 1] = [RecordField {
    source_offset: STATIC_MODIFIERS,
    destination_offset: 0,
    len: 16,
}];

pub fn static_modifier_list(header: [u8; 16]) -> Option<Array> {
    Array::from_header(header, STATIC_MODIFIER_STRIDE)
}

pub fn read_static_modifiers(
    memory: &impl Memory,
    list: Array,
    definition_vtable: usize,
    out: &mut Vec<u8>,
) -> Result<(), &'static str> {
    const BATCH: usize = 16;
    let mut entries = [0_u8; BATCH * STATIC_MODIFIER_STRIDE];
    let mut first = 0;
    while first < list.count {
        let count = (list.count - first).min(BATCH as u32);
        let bytes = &mut entries[..count as usize * STATIC_MODIFIER_STRIDE];
        let source = list
            .element(first, STATIC_MODIFIER_STRIDE)
            .ok_or("static modifier list address overflow")?;
        if !read_slice(memory, source, bytes) {
            return Err("static modifier list unreadable");
        }
        for entry in bytes.chunks_exact(STATIC_MODIFIER_STRIDE) {
            let definition = usize::try_from(u64::from_le_bytes(entry[..8].try_into().unwrap()))
                .map_err(|_| "static modifier definition invalid")?;
            let days = &entry[STATIC_MODIFIER_DAYS..STATIC_MODIFIER_DAYS + 4];
            if crate::read::<u64>(memory, definition) != Some(definition_vtable as u64) {
                return Err("static modifier definition vtable mismatch");
            }
            let header = out.len();
            crate::allocation::reserve(out, 8).map_err(|_| "static modifier allocation failed")?;
            out.extend_from_slice(days);
            out.extend_from_slice(&[0; 4]);
            let mut used = out.len();
            let name = definition
                .checked_add(crate::cstatic_modifier::NAME)
                .ok_or("static modifier name address overflow")?;
            let (_, len) = crate::msvc_string::append(memory, name, out, &mut used)?;
            out.truncate(used);
            let len = u32::try_from(len).map_err(|_| "static modifier name too long")?;
            out[header + 4..header + 8].copy_from_slice(&len.to_le_bytes());
        }
        first += count;
    }
    Ok(())
}

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

pub fn fill_clock_record(record: &[u8], copied: usize, row: &mut Row) -> Result<(), &'static str> {
    if record.len() < CLOCK_RECORD_BYTES || copied < CLOCK_FIELDS.len() {
        return Err("weather clock fields unreadable");
    }
    row.night_factor = i64::from_le_bytes(record[..8].try_into().unwrap());
    row.local_hour = i32::from_le_bytes(record[8..12].try_into().unwrap());
    Ok(())
}

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
