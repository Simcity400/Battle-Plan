//! Passive regional mine operands and the writers that certify their carry.
use crate::{Memory, cgame_state, cpdx_array::Array, read, read_slice, walk::Buffer};
use std::ops::Range;

/// Strategic navy's raw fixed5 regional mine vector, indexed by region id.
pub const REGIONAL_MINES: usize = 0xc8;
/// The simulation spreads peace-time decay over this many clock hours.
pub const DECAY_SLICES: usize = 24;
/// Native clock origin used before the signed remainder selecting a slice.
pub const DECAY_CLOCK_ORIGIN: i32 = 43_800_000;
/// Persistent navy interface receiver's displacement from its primary object.
pub const PERSISTENT_RECEIVER: usize = 8;

/// Sole point writers, native sliced decay, and complete lifecycle writers.
pub const WRITERS: [(&str, usize, u64); 7] = [
    ("mine region addition", 0x00e8_a000, 0x6e94_eeb3_2e37_fce3),
    ("mine index addition", 0x00e8_a040, 0xfb65_e768_acf6_996e),
    ("mine peace decay", 0x00e8_e4f0, 0x4922_ae21_0537_1896),
    ("navy construction", 0x00e8_8190, 0x60d3_e0d8_0296_fbf8),
    ("navy regional growth", 0x00e9_2ae0, 0xb4b9_d7f9_399a_125d),
    ("navy reset", 0x00e9_6510, 0x60d3_efd8_0297_1575),
    ("navy persistent load", 0x00e9_7960, 0xe688_884a_0a9a_e2ff),
];

/// The caller supplies a navy authenticated by the current physical census.
/// The header is acquired first/on-lifecycle-change, under its certificate.
pub fn header(memory: &impl Memory, navy: usize) -> Result<Array, &'static str> {
    navy.checked_add(REGIONAL_MINES)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or("regional mine header unreadable or invalid")
}

/// A native decay slice; the last slices may be empty for short vectors.
pub fn slice(count: u32, part: usize) -> Option<Range<u32>> {
    if part >= DECAY_SLICES {
        return None;
    }
    let width = count.div_ceil(DECAY_SLICES as u32);
    let start = width.checked_mul(part as u32)?.min(count);
    Some(start..start.saturating_add(width).min(count))
}

/// Which native slice owns an individual valid region index.
pub fn region_slice(count: u32, region: u32) -> Option<usize> {
    (region < count).then(|| (region / count.div_ceil(DECAY_SLICES as u32)) as usize)
}

/// Event-only bounded metadata. It never reads a mine cell or walks a vector.
pub fn event_count(memory: &impl Memory, navy: usize) -> Option<u32> {
    let count: i32 = read(
        memory,
        navy.checked_add(REGIONAL_MINES + crate::cpdx_array::COUNT)?,
    )?;
    u32::try_from(count).ok()
}

/// Event-only decay slice selection. A negative native remainder is refused.
pub fn event_decay_slice(memory: &impl Memory, base: usize) -> Option<usize> {
    let state = usize::try_from(read::<u64>(
        memory,
        base.checked_add(cgame_state::SINGLETON_RVA)?,
    )?)
    .ok()?;
    let hour: i32 = read(memory, state.checked_add(cgame_state::CURRENT_HOUR)?)?;
    usize::try_from(hour.wrapping_sub(DECAY_CLOCK_ORIGIN) % DECAY_SLICES as i32).ok()
}

/// First/changed native slice only. Zeros remain represented by its complete
/// slice certificate; nonzero signed values retain their region indices.
pub fn capture_slice(
    memory: &impl Memory,
    source: Array,
    part: usize,
    output: &mut Vec<(u32, i64)>,
) -> Result<usize, &'static str> {
    let range = slice(source.count, part).ok_or("regional mine slice invalid")?;
    let mut len = 0;
    let mut bytes = [0_u8; 64 * 8];
    let mut first = range.start;
    while first < range.end {
        let count = (range.end - first).min((bytes.len() / 8) as u32) as usize;
        let at = source
            .element(first, 8)
            .ok_or("regional mine slice address overflow")?;
        if !read_slice(memory, at, &mut bytes[..count * 8]) {
            return Err("regional mine slice unreadable");
        }
        for (offset, word) in bytes[..count * 8].chunks_exact(8).enumerate() {
            let amount = i64::from_le_bytes(word.try_into().unwrap());
            if amount != 0 {
                Buffer::Growing(output).ensure(len + 1)?;
                output[len] = (first + offset as u32, amount);
                len += 1;
            }
        }
        first += count as u32;
    }
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;
    #[test]
    fn native_slice_partition_is_complete_disjoint_and_has_no_chosen_cap() {
        for count in [0, 1, 23, 24, 25, 300, 5000, i32::MAX as u32] {
            let mut end = 0;
            for part in 0..DECAY_SLICES {
                let range = slice(count, part).unwrap();
                assert_eq!(range.start, end);
                if !range.is_empty() {
                    assert_eq!(region_slice(count, range.start), Some(part));
                    assert_eq!(region_slice(count, range.end - 1), Some(part));
                }
                end = range.end;
            }
            assert_eq!(end, count);
            assert_eq!(region_slice(count, count), None);
        }
        assert!(slice(3, DECAY_SLICES).is_none());
    }
    #[test]
    fn slice_reads_keep_signed_values_and_skip_only_observed_zeros() {
        let mut bytes = vec![0; 0x100 + 25 * 8];
        bytes[0x100..0x108].copy_from_slice(&(-7_i64).to_le_bytes());
        bytes[0x108..0x110].copy_from_slice(&19_i64.to_le_bytes());
        let source = Array {
            data: 0x100,
            count: 25,
        };
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut out = Vec::new();
        assert_eq!(capture_slice(&memory, source, 0, &mut out), Ok(2));
        assert_eq!(out[..2], [(0, -7), (1, 19)]);
        assert_eq!(capture_slice(&memory, source, 1, &mut out), Ok(0));
        assert!(
            capture_slice(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes[..0x108]
                },
                source,
                0,
                &mut out
            )
            .is_err()
        );
    }
}
