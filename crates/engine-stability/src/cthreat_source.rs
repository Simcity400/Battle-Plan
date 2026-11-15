//! CThreatSource objects reached through CWorldThreat's pointer array.
//!
//! Each source stores two raw country references, a date, a reason and three
//! signed fixed-point values with denominator 100,000. Negative contributions
//! are ordinary state and are never clamped here.

use crate::{Memory, msvc_string, read, read_slice, walk::Buffer};
use engine_sites::Rva;

/// CThreatSource primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x029f_2ff8);
/// CThreatSource raw country reference.
pub const COUNTRY: usize = 0x08;
/// CThreatSource second raw country reference.
pub const SECOND_COUNTRY: usize = 0x0c;
/// CThreatSource current contribution i64.
pub const CURRENT: usize = 0x10;
/// CThreatSource final contribution i64.
pub const FINAL: usize = 0x18;
/// CThreatSource daily change i64.
pub const DAILY: usize = 0x20;
/// CThreatSource raw date hour i32.
pub const DATE: usize = 0x30;
/// CThreatSource reason std::string.
pub const REASON: usize = 0x40;
/// CThreatSource allocation size.
pub const SIZE: usize = 0x60;
/// CThreatSource native fixed-point denominator.
pub const FIXED_POINT_SCALE: i64 = 100_000;
/// CThreatSource daily ramp/decay update.
pub const DAILY_UPDATE_RVA: Rva = Rva(0x00f8_3c00);
/// Matching-source update; replaces final, current and daily contribution.
pub const UPDATE_RVA: Rva = Rva(0x00f8_40a0);

/// Bytes copied in one guarded batch: every recorded scalar lies in the first 0x34 bytes.
const HEAD: usize = 0x34;

/// One native contribution in stored order. `reason` indexes the caller's arena.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Stored vector position; compaction can change the order.
    pub ordinal: u32,
    /// Raw country reference; resolve through CountryLinks, never normalized here.
    pub country: u32,
    /// Second raw country reference; its engine role is not established.
    pub second_country: u32,
    pub current_raw: i64,
    pub final_raw: i64,
    pub daily_raw: i64,
    /// Native engine date hour; no calendar conversion happens on the game thread.
    pub date: i32,
    pub reason_start: usize,
    pub reason_len: usize,
}

/// Copies one source's recorded fields in a single guarded batch, then its
/// literal reason bytes. Zero and negative values are state, not absence.
///
/// A reason that does not fit the caller's remaining arena yields `None`: the
/// row is omitted and counted, exactly as a full row buffer omits one. The
/// returned `wanted` is the reason's full length either way, so the spool thread
/// can grow the arena without a second engine read.
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
    // Serialization preserves the complete stored reason string.
    // Preserve the original descriptor and terminator observation while sizing.
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

/// Resolves an MSVC std::string's byte span without copying or converting it.
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
    use crate::memory::SliceMemory;

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
        // An empty literal reason is valid and consumes no arena bytes.
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
