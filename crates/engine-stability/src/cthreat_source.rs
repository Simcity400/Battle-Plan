
use engine_native_memory::{Memory, msvc_string, read, read_slice, walk::Buffer};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029f_2ff8);
pub const COUNTRY: usize = 0x08;
pub const SECOND_COUNTRY: usize = 0x0c;
pub const CURRENT: usize = 0x10;
pub const FINAL: usize = 0x18;
pub const DAILY: usize = 0x20;
pub const DATE: usize = 0x30;
pub const REASON: usize = 0x40;
pub const SIZE: usize = 0x60;
pub const FIXED_POINT_SCALE: i64 = 100_000;
pub const DAILY_UPDATE_RVA: Rva = Rva(0x00f8_3c00);
pub const UPDATE_RVA: Rva = Rva(0x00f8_40a0);

const HEAD: usize = 0x34;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub ordinal: u32,
    pub country: u32,
    pub second_country: u32,
    pub current_raw: i64,
    pub final_raw: i64,
    pub daily_raw: i64,
    pub date: i32,
    pub reason_start: usize,
    pub reason_len: usize,
}

pub(crate) fn count_row(
    memory: &impl Memory,
    base: usize,
    ordinal: u32,
    pointer: usize,
) -> Result<(Option<Row>, usize, usize), &'static str> {
    read_row_buffer(memory, base, ordinal, pointer, &mut Buffer::Count, 0)
}

pub(crate) fn read_row_buffer(
    memory: &impl Memory,
    base: usize,
    ordinal: u32,
    pointer: usize,
    reasons: &mut Buffer<'_, u8>,
    used: usize,
) -> Result<(Option<Row>, usize, usize), &'static str> {
    let expected =
        engine_sites::at(base, VTABLE_RVA).ok_or("threat source module address overflow")?;
    let head = read::<[u8; HEAD]>(memory, pointer).ok_or("threat source fields unreadable")?;
    if u64::from_le_bytes(head[..8].try_into().unwrap()) != expected as u64 {
        return Err("threat source vtable mismatch");
    }
    let word = |offset: usize| u32::from_le_bytes(head[offset..offset + 4].try_into().unwrap());
    let quad = |offset: usize| i64::from_le_bytes(head[offset..offset + 8].try_into().unwrap());
    let reason = pointer
        .checked_add(REASON)
        .ok_or("threat source reason address overflow")?;
    let (source, reason_len) = literal(memory, reason)?;
    let end = used
        .checked_add(reason_len)
        .ok_or("threat source reason arena extent overflow")?;
    reasons.ensure(end)?;
    if reason_len > reasons.len().saturating_sub(used) {
        return Ok((None, used, reason_len));
    }
    if !read_slice(memory, source, &mut reasons[used..end]) {
        return Err("threat source reason bytes unreadable");
    }
    Ok((
        Some(Row {
            ordinal,
            country: word(COUNTRY),
            second_country: word(SECOND_COUNTRY),
            current_raw: quad(CURRENT),
            final_raw: quad(FINAL),
            daily_raw: quad(DAILY),
            date: word(DATE) as i32,
            reason_start: used,
            reason_len,
        }),
        end,
        reason_len,
    ))
}

fn literal(memory: &impl Memory, string: usize) -> Result<(usize, usize), &'static str> {
    let at = |offset: usize| {
        string
            .checked_add(offset)
            .ok_or("threat source reason address overflow")
    };
    let len = read::<u64>(memory, at(msvc_string::LENGTH)?)
        .ok_or("threat source reason length unreadable")?;
    let capacity = read::<u64>(memory, at(msvc_string::CAPACITY)?)
        .ok_or("threat source reason capacity unreadable")?;
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return Err("threat source reason header invalid");
    }
    let len = usize::try_from(len).map_err(|_| "threat source reason length overflow")?;
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        string
    } else {
        read::<u64>(memory, string)
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|pointer| *pointer != 0)
            .ok_or("threat source reason pointer unreadable or null")?
    };
    if read::<u8>(
        memory,
        source
            .checked_add(len)
            .ok_or("threat source reason address overflow")?,
    ) != Some(0)
    {
        return Err("threat source reason terminator unreadable or invalid");
    }
    Ok((source, len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    fn read_row(
        memory: &impl Memory,
        base: usize,
        ordinal: u32,
        pointer: usize,
        reasons: &mut Vec<u8>,
        used: usize,
    ) -> Result<(Option<Row>, usize, usize), &'static str> {
        read_row_buffer(memory, base, ordinal, pointer, &mut Buffer::Growing(reasons), used)
    }

    const BASE: usize = 0x1000;
    const SOURCE: usize = BASE + 0x80;

    fn fixture(reason: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; 0x200];
        let mut put = |offset: usize, value: &[u8]| {
            let at = SOURCE - BASE + offset;
            bytes[at..at + value.len()].copy_from_slice(value);
        };
        put(0, &(VTABLE_RVA.0 as u64).to_le_bytes());
        put(COUNTRY, &0x8000_0007_u32.to_le_bytes());
        put(SECOND_COUNTRY, &u32::MAX.to_le_bytes());
        put(CURRENT, &i64::MIN.to_le_bytes());
        put(FINAL, &i64::MAX.to_le_bytes());
        put(DAILY, &(-125_682_i64).to_le_bytes());
        put(DATE, &i32::MIN.to_le_bytes());
        put(REASON, reason);
        put(
            REASON + msvc_string::LENGTH,
            &(reason.len() as u64 - 1).to_le_bytes(),
        );
        put(REASON + msvc_string::CAPACITY, &15_u64.to_le_bytes());
        bytes
    }

    #[test]
    fn every_native_word_and_literal_reason_survives_with_negative_values() {
        let bytes = fixture(b"Peace\0");
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let mut arena = vec![0; 16];
        let (row, used, wanted) = read_row(&memory, 0, 3, SOURCE, &mut arena, 4).unwrap();
        assert_eq!(wanted, 5);
        assert_eq!(
            row.unwrap(),
            Row {
                ordinal: 3,
                country: 0x8000_0007,
                second_country: u32::MAX,
                current_raw: i64::MIN,
                final_raw: i64::MAX,
                daily_raw: -125_682,
                date: i32::MIN,
                reason_start: 4,
                reason_len: 5,
            }
        );
        assert_eq!(used, 9);
        assert_eq!(&arena[4..9], b"Peace");
        let bytes = fixture(b"\0");
        let (row, used, wanted) = read_row(
            &SliceMemory {
                base: BASE,
                bytes: &bytes,
            },
            0,
            0,
            SOURCE,
            &mut arena,
            0,
        )
        .unwrap();
        let row = row.unwrap();
        assert_eq!(
            (row.reason_start, row.reason_len, used, wanted),
            (0, 0, 0, 0)
        );
    }

    #[test]
    fn broken_headers_and_missing_terminators_fail_explicitly() {
        let bytes = fixture(b"Peace\0");
        for (offset, value, reason) in [
            (0_usize, 1_u8, "threat source vtable mismatch"),
            (
                REASON + msvc_string::CAPACITY,
                3,
                "threat source reason header invalid",
            ),
            (
                REASON + 5,
                b'!',
                "threat source reason terminator unreadable or invalid",
            ),
        ] {
            let mut broken = bytes.clone();
            broken[SOURCE - BASE + offset] = value;
            assert_eq!(
                read_row(
                    &SliceMemory {
                        base: BASE,
                        bytes: &broken
                    },
                    0,
                    0,
                    SOURCE,
                    &mut vec![0; 16],
                    0
                ),
                Err(reason)
            );
        }
        assert_eq!(
            read_row(
                &SliceMemory {
                    base: BASE,
                    bytes: &bytes[..SOURCE - BASE]
                },
                0,
                0,
                SOURCE,
                &mut vec![0; 16],
                0
            ),
            Err("threat source fields unreadable")
        );
    }
}
