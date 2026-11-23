
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    msvc_string,
    read,
    read_slice,
    walk::Outcome,
};
use engine_native::ccountry_supply_system::{self, world as csupply_system};

pub const STRIDE: usize = 0x70;
pub const PROVINCE: usize = 0x00;
pub const SUPPLY: usize = 0x08;
pub const START: usize = 0x10;
pub const PENALTY: usize = 0x18;
pub const ADDED_PENALTY: usize = 0x20;
pub const SOURCE: usize = 0x28;
pub const DURATION: usize = 0x48;
pub const PROVINCE_COPY: usize = 0x50;
pub const HOURS: usize = 0x58;
pub const DECAY: usize = 0x60;
pub const BASE: usize = 0x68;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub ordinal: u32,
    pub province: u32,
    pub supply: i64,
    pub start: i64,
    pub penalty: i64,
    pub added_penalty: i64,
    pub duration: i64,
    pub hours: i64,
    pub decay: i64,
    pub base: u8,
    pub source_start: usize,
    pub source_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceWalk {
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    sources: &mut Vec<u8>,
) -> SourceWalk {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(sources),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut sources: engine_native_memory::walk::Buffer<'_, u8>,
) -> SourceWalk {
    let result = (|| {
        let systems = csupply_system::Reader::new(memory, base, game_state)?;
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        let mut bytes_len = 0_usize;
        let mut required_bytes = 0_usize;
        for slot in 1..systems.count() {
            let Some(system) = systems.at(slot)? else {
                continue;
            };
            let country = read_country_buffer(
                memory,
                system.pointer,
                system.country.id,
                output.suffix(len),
                sources.suffix(bytes_len),
            );
            let added = match country.outcome {
                Outcome::Rows { len } => len,
                Outcome::Truncated {
                    len,
                    unread_rows: omitted,
                } => {
                    unread_rows = unread_rows
                        .checked_add(omitted)
                        .ok_or("foreign homebase row count overflow")?;
                    len
                }
                Outcome::Unavailable { reason } => return Err(reason),
            };
            for row in &mut output[len..len + added] {
                row.source_start += bytes_len;
            }
            len += added;
            bytes_len += country.bytes_len;
            required_bytes = required_bytes
                .checked_add(country.required_bytes)
                .ok_or("foreign homebase source byte count overflow")?;
        }
        Ok(SourceWalk {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            bytes_len,
            required_bytes,
        })
    })();
    result.unwrap_or_else(unavailable)
}

#[cfg(test)]
pub(crate) fn read_country(
    memory: &impl Memory,
    pointer: usize,
    country: u32,
    output: &mut Vec<Row>,
    sources: &mut Vec<u8>,
) -> SourceWalk {
    read_country_buffer(
        memory,
        pointer,
        country,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(sources),
    )
}

pub(crate) fn read_country_buffer(
    memory: &impl Memory,
    pointer: usize,
    country: u32,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut sources: engine_native_memory::walk::Buffer<'_, u8>,
) -> SourceWalk {
    let result = (|| {
        if country == 0 {
            return Err("foreign homebase country is zero");
        }
        let array = pointer
            .checked_add(ccountry_supply_system::FOREIGN_HOMEBASE_DATA)
            .and_then(|p| Array::read(memory, p, STRIDE))
            .ok_or("foreign homebase array unreadable or invalid")?;
        let mut len = 0_usize;
        let mut bytes_len = 0_usize;
        let mut required_bytes = 0_usize;
        output.ensure(array.count as usize)?;
        for ordinal in 0..array.count {
            let address = array
                .element(ordinal, STRIDE)
                .ok_or("foreign homebase address overflow")?;
            let string = address + SOURCE;
            let (source, source_len) = literal(memory, string)?;
            required_bytes = required_bytes
                .checked_add(source_len)
                .ok_or("foreign homebase source byte count overflow")?;
            sources.ensure(required_bytes)?;
            if len == output.len() || source_len > sources.len() - bytes_len {
                continue;
            }
            let mut prefix = [0_u8; SOURCE];
            let mut suffix = [0_u8; BASE + 1 - DURATION];
            if memory.copy_scattered(
                [address, address + DURATION],
                &mut [&mut prefix, &mut suffix],
            ) != 2
            {
                return Err("foreign homebase row unreadable");
            }
            let province = u32::from_le_bytes(prefix[PROVINCE..PROVINCE + 4].try_into().unwrap());
            let copy = u32::from_le_bytes(
                suffix[PROVINCE_COPY - DURATION..PROVINCE_COPY + 4 - DURATION]
                    .try_into()
                    .unwrap(),
            );
            if province == 0 || province != copy {
                return Err("foreign homebase province identity differs");
            }
            if suffix[BASE - DURATION] > 1 {
                return Err("foreign homebase base flag invalid");
            }
            if !read_slice(
                memory,
                source,
                &mut sources[bytes_len..bytes_len + source_len],
            ) {
                return Err("foreign homebase source bytes unreadable");
            }
            let scalar = |offset: usize| {
                let bytes = if offset < SOURCE {
                    &prefix[offset..offset + 8]
                } else {
                    &suffix[offset - DURATION..offset + 8 - DURATION]
                };
                i64::from_le_bytes(bytes.try_into().unwrap())
            };
            output[len] = Row {
                country,
                ordinal,
                province,
                supply: scalar(SUPPLY),
                start: scalar(START),
                penalty: scalar(PENALTY),
                added_penalty: scalar(ADDED_PENALTY),
                duration: scalar(DURATION),
                hours: scalar(HOURS),
                decay: scalar(DECAY),
                base: suffix[BASE - DURATION],
                source_start: bytes_len,
                source_len,
            };
            bytes_len += source_len;
            len += 1;
        }
        let unread_rows = array.count - len as u32;
        Ok(SourceWalk {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            bytes_len,
            required_bytes,
        })
    })();
    result.unwrap_or_else(unavailable)
}

fn literal(memory: &impl Memory, pointer: usize) -> Result<(usize, usize), &'static str> {
    let header = read::<[u8; msvc_string::STRIDE]>(memory, pointer)
        .ok_or("foreign homebase source header unreadable")?;
    let len = u64::from_le_bytes(
        header[msvc_string::LENGTH..msvc_string::CAPACITY]
            .try_into()
            .unwrap(),
    );
    let capacity = u64::from_le_bytes(header[msvc_string::CAPACITY..].try_into().unwrap());
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return Err("foreign homebase source header invalid");
    }
    let len = usize::try_from(len).map_err(|_| "foreign homebase source length overflow")?;
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        pointer
    } else {
        usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .ok()
            .filter(|&p| p != 0)
            .ok_or("foreign homebase source pointer invalid")?
    };
    let terminator = source
        .checked_add(len)
        .ok_or("foreign homebase source address overflow")?;
    if read::<u8>(memory, terminator) != Some(0) {
        return Err("foreign homebase source terminator invalid");
    }
    Ok((source, len))
}

fn unavailable(reason: &'static str) -> SourceWalk {
    SourceWalk {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    const COUNTRY: usize = 0x1000;
    const RECORDS: usize = 0x1400;

    fn memory() -> Vec<u8> {
        let mut bytes = vec![0; 0x800];
        let header = ccountry_supply_system::FOREIGN_HOMEBASE_DATA;
        bytes[header..header + 8].copy_from_slice(&(RECORDS as u64).to_le_bytes());
        bytes[header + 8..header + 12].copy_from_slice(&3_u32.to_le_bytes());
        bytes[header + 12..header + 16].copy_from_slice(&3_u32.to_le_bytes());
        for (ordinal, source) in [b"abcdefg".as_slice(), &[0xff, 0], &[]]
            .into_iter()
            .enumerate()
        {
            let offset = RECORDS - COUNTRY + ordinal * STRIDE;
            bytes[offset + PROVINCE..offset + PROVINCE + 4].copy_from_slice(&90_u32.to_le_bytes());
            bytes[offset + PROVINCE_COPY..offset + PROVINCE_COPY + 4]
                .copy_from_slice(&90_u32.to_le_bytes());
            for (field, value) in [
                (SUPPLY, i64::MAX),
                (START, i64::MIN),
                (PENALTY, -123456),
                (ADDED_PENALTY, 9876543210),
                (DURATION, 1100000),
                (HOURS, 400000),
                (DECAY, 100001),
            ] {
                bytes[offset + field..offset + field + 8].copy_from_slice(&value.to_le_bytes());
            }
            bytes[offset + SOURCE..offset + SOURCE + source.len()].copy_from_slice(source);
            bytes[offset + SOURCE + msvc_string::LENGTH..offset + SOURCE + msvc_string::CAPACITY]
                .copy_from_slice(&(source.len() as u64).to_le_bytes());
            bytes[offset + SOURCE + msvc_string::CAPACITY..offset + SOURCE + msvc_string::STRIDE]
                .copy_from_slice(&msvc_string::INLINE_CAPACITY.to_le_bytes());
            bytes[offset + BASE] = 1;
        }
        bytes
    }

    fn capture(bytes: &[u8], output: &mut Vec<Row>, sources: &mut Vec<u8>) -> SourceWalk {
        read_country(
            &SliceMemory {
                base: COUNTRY,
                bytes,
            },
            COUNTRY,
            23,
            output,
            sources,
        )
    }

    #[test]
    fn native_rows_preserve_signed_precision_duplicate_provinces_and_source_bytes() {
        let bytes = memory();
        let mut rows = vec![Row::default(); 3];
        let mut source = vec![0; 9];
        assert_eq!(
            capture(&bytes, &mut rows, &mut source),
            SourceWalk {
                outcome: Outcome::Rows { len: 3 },
                bytes_len: 9,
                required_bytes: 9,
            }
        );
        assert_eq!(&source, b"abcdefg\xff\0");
        assert_eq!(
            (rows[0].country, rows[0].supply, rows[0].start),
            (23, i64::MAX, i64::MIN)
        );
        assert_eq!(rows[0].added_penalty, 9876543210);
        assert_eq!(
            (
                rows[1].ordinal,
                rows[1].province,
                rows[1].source_start,
                rows[1].source_len
            ),
            (1, 90, 7, 2)
        );
        assert_eq!(
            (rows[2].ordinal, rows[2].province, rows[2].source_len),
            (2, 90, 0)
        );
    }

    #[test]
    fn the_count_probe_reports_every_row_and_required_byte() {
        let bytes = memory();
        assert_eq!(
            read_country_buffer(
                &SliceMemory {
                    base: COUNTRY,
                    bytes: &bytes,
                },
                COUNTRY,
                23,
                engine_native_memory::walk::Buffer::Count,
                engine_native_memory::walk::Buffer::Count,
            ),
            SourceWalk {
                outcome: Outcome::Truncated {
                    len: 0,
                    unread_rows: 3
                },
                bytes_len: 0,
                required_bytes: 9,
            }
        );
    }

    #[test]
    fn bad_native_identity_string_and_header_never_become_successful_rows() {
        let mut bytes = memory();
        let mut rows = vec![Row::default(); 3];
        let mut source = vec![0; 9];
        bytes[RECORDS - COUNTRY + PROVINCE_COPY] = 91;
        assert_eq!(
            capture(&bytes, &mut rows, &mut source).outcome,
            Outcome::Unavailable {
                reason: "foreign homebase province identity differs"
            }
        );
        bytes = memory();
        bytes[RECORDS - COUNTRY + BASE] = 2;
        assert_eq!(
            capture(&bytes, &mut rows, &mut source).outcome,
            Outcome::Unavailable {
                reason: "foreign homebase base flag invalid"
            }
        );
        bytes = memory();
        bytes[RECORDS - COUNTRY + SOURCE + 7] = 1;
        assert_eq!(
            capture(&bytes, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Unavailable {
                reason: "foreign homebase source terminator invalid"
            }
        );
        bytes = memory();
        bytes[ccountry_supply_system::FOREIGN_HOMEBASE_DATA + 8] = 2;
        assert_eq!(
            capture(&bytes, &mut rows, &mut source).outcome,
            Outcome::Unavailable {
                reason: "foreign homebase array unreadable or invalid"
            }
        );
    }

    #[test]
    fn empty_foreign_array_needs_no_record_or_string_storage() {
        let mut bytes = memory();
        let header = ccountry_supply_system::FOREIGN_HOMEBASE_DATA;
        bytes[header..header + 16].fill(0);
        assert_eq!(
            capture(&bytes[..header + 16], &mut Vec::new(), &mut Vec::new()),
            SourceWalk {
                outcome: Outcome::Rows { len: 0 },
                bytes_len: 0,
                required_bytes: 0,
            }
        );
    }
}
