//! Lossless byte patches between two encodings of one field.
//!
//! A patch is `varint hunks` followed by that many hunks of
//! `varint skip, varint removed, varint inserted, inserted bytes`: copy `skip`
//! unchanged base bytes, drop `removed` base bytes, then write the inserted
//! bytes. Base bytes after the last hunk are copied. Every hunk changes
//! something, and hunks never touch or overlap, so each patch has one reading.
//! A patch with differences stores the bytes of a hunk that inserts as many
//! bytes as it removes as each new byte's wrapping difference from the byte
//! it replaces.

use crate::codec::{Decoder, invalid_static as invalid, put_varint};
use engine_allocation as allocation;
use std::io;

/// Equal-length values merge differing runs closer than this many equal
/// bytes: a separate hunk costs at least three varints.
const MERGE_GAP: usize = 3;
/// Copies found by search are at least this long.
const GRAM: usize = 8;
/// In-place edits resynchronize within this distance before a search.
const DIAGONAL: usize = 64;

/// `(base start, removed, target start, inserted)`, in ascending order.
type Hunk = (usize, usize, usize, usize);

/// Append the patch from `base` to `target`. The caller keeps the whole value
/// when the patch is not smaller. Equal-length values patch their differing
/// runs; otherwise, or when that leaves most bytes inserted, copies of
/// shifted base bytes are searched (a window that drops its oldest entries
/// and appends new ones copies everything it keeps).
#[cfg(test)]
pub(crate) fn diff(base: &[u8], target: &[u8], out: &mut Vec<u8>) -> io::Result<()> {
    diff_with(base, target, out, false)
}

/// [`diff`]; with `differences`, a hunk that replaces as many bytes as it
/// removes stores each new byte's wrapping difference from the byte it
/// replaces.
pub fn diff_with(
    base: &[u8],
    target: &[u8],
    out: &mut Vec<u8>,
    differences: bool,
) -> io::Result<()> {
    if base == target {
        return Err(invalid("patch between equal values"));
    }
    let prefix = base.iter().zip(target).take_while(|(a, b)| a == b).count();
    let limit = base.len().min(target.len()) - prefix;
    let suffix = base
        .iter()
        .rev()
        .zip(target.iter().rev())
        .take(limit)
        .take_while(|(a, b)| a == b)
        .count();
    let (base_end, target_end) = (base.len() - suffix, target.len() - suffix);
    let mut hunks = Vec::new();
    if base.len() == target.len() {
        in_place(base, target, prefix, base_end, &mut hunks)?;
    } else {
        allocation::reserve(&mut hunks, 1)?;
        hunks.push((prefix, base_end - prefix, prefix, target_end - prefix));
    }
    let inserted: usize = hunks.iter().map(|hunk| hunk.3).sum();
    if base.len() != target.len() || inserted * 4 > target.len() {
        let mut aligned = Vec::new();
        align(base, target, prefix, base_end, target_end, &mut aligned)?;
        if encoded_bytes(&aligned) < encoded_bytes(&hunks) {
            hunks = aligned;
        }
    }
    allocation::reserve(out, encoded_bytes(&hunks))?;
    put_varint(out, hunks.len() as u128);
    let mut position = 0;
    for (base_start, removed, target_start, inserted) in hunks {
        put_varint(out, (base_start - position) as u128);
        put_varint(out, removed as u128);
        put_varint(out, inserted as u128);
        let bytes = &target[target_start..target_start + inserted];
        if differences && removed == inserted {
            let replaced = &base[base_start..base_start + removed];
            out.extend(
                bytes
                    .iter()
                    .zip(replaced)
                    .map(|(new, old)| new.wrapping_sub(*old)),
            );
        } else {
            out.extend_from_slice(bytes);
        }
        position = base_start + removed;
    }
    Ok(())
}

fn varint_bytes(value: usize) -> usize {
    crate::rows::varint_bytes(value as u128)
}

fn encoded_bytes(hunks: &[Hunk]) -> usize {
    let mut position = 0;
    let mut size = varint_bytes(hunks.len());
    for &(base_start, removed, _, inserted) in hunks {
        size += varint_bytes(base_start - position)
            + varint_bytes(removed)
            + varint_bytes(inserted)
            + inserted;
        position = base_start + removed;
    }
    size
}

/// Differing runs of two equal-length values between `start` and `end`.
fn in_place(
    base: &[u8],
    target: &[u8],
    start: usize,
    end: usize,
    hunks: &mut Vec<Hunk>,
) -> io::Result<()> {
    let mut index = start;
    while index < end {
        if base[index] == target[index] {
            index += 1;
            continue;
        }
        let first = index;
        let mut last = index;
        while index < end {
            if base[index] != target[index] {
                last = index;
            } else if index - last > MERGE_GAP {
                break;
            }
            index += 1;
        }
        allocation::reserve(hunks, 1)?;
        hunks.push((first, last + 1 - first, first, last + 1 - first));
    }
    Ok(())
}

/// A monotonic copy/insert alignment of `base[start..base_end]` and
/// `target[start..target_end]`: equal runs are copied, an in-place edit
/// resynchronizes on its diagonal, and otherwise the next base copy of at
/// least [`GRAM`] bytes is searched.
fn align(
    base: &[u8],
    target: &[u8],
    start: usize,
    base_end: usize,
    target_end: usize,
    hunks: &mut Vec<Hunk>,
) -> io::Result<()> {
    let gram =
        |bytes: &[u8], at: usize| u64::from_le_bytes(bytes[at..at + GRAM].try_into().unwrap());
    let mut grams: Option<Vec<(u64, u32)>> = None;
    let (mut p, mut q) = (start, start);
    loop {
        while p < target_end && q < base_end && target[p] == base[q] {
            p += 1;
            q += 1;
        }
        if p == target_end || q == base_end {
            if p < target_end || q < base_end {
                allocation::reserve(hunks, 1)?;
                hunks.push((q, base_end - q, p, target_end - p));
            }
            return Ok(());
        }
        let mut sync = (1..=DIAGONAL)
            .take_while(|d| p + d + GRAM <= target_end && q + d + GRAM <= base_end)
            .find(|d| target[p + d..p + d + GRAM] == base[q + d..q + d + GRAM])
            .map(|d| (p + d, q + d));
        if sync.is_none() && base_end - q >= GRAM {
            let index = match &mut grams {
                Some(index) => index,
                None => {
                    let mut index = allocation::vec(base_end + 1 - GRAM - start)?;
                    for at in start..=base_end - GRAM {
                        index.push((gram(base, at), at as u32));
                    }
                    index.sort_unstable();
                    grams.insert(index)
                }
            };
            sync = (p..).take_while(|i| i + GRAM <= target_end).find_map(|i| {
                let key = gram(target, i);
                let first = index.partition_point(|&(g, at)| (g, at as usize) < (key, q));
                index
                    .get(first)
                    .filter(|&&(g, at)| g == key && at as usize + GRAM <= base_end)
                    .map(|&(_, at)| (i, at as usize))
            });
        }
        let (i, j) = sync.unwrap_or((target_end, base_end));
        allocation::reserve(hunks, 1)?;
        hunks.push((q, j - q, p, i - p));
        (p, q) = (i, j);
        if sync.is_none() {
            return Ok(());
        }
    }
}

/// Apply one patch read from `input` to `base`, appending the value to `out`.
#[cfg(test)]
pub(crate) fn apply(base: &[u8], input: &mut Decoder<'_>, out: &mut Vec<u8>) -> io::Result<()> {
    apply_with(base, input, out, false)
}

/// [`apply`] a patch written by [`diff_with`].
pub fn apply_with(
    base: &[u8],
    input: &mut Decoder<'_>,
    out: &mut Vec<u8>,
    differences: bool,
) -> io::Result<()> {
    let hunks = varint_usize(input)?;
    if hunks == 0 || hunks > input.remaining() / 3 {
        return Err(invalid("invalid patch hunk count"));
    }
    let mut position = 0_usize;
    for index in 0..hunks {
        let skip = varint_usize(input)?;
        let removed = varint_usize(input)?;
        let inserted = varint_usize(input)?;
        if (index != 0 && skip == 0) || (removed == 0 && inserted == 0) {
            return Err(invalid("noncanonical patch hunk"));
        }
        let start = position
            .checked_add(skip)
            .filter(|start| *start <= base.len())
            .ok_or_else(|| invalid("patch hunk outside its base"))?;
        let end = start
            .checked_add(removed)
            .filter(|end| *end <= base.len())
            .ok_or_else(|| invalid("patch hunk outside its base"))?;
        let bytes = input.take(inserted)?;
        allocation::reserve(out, allocation::add(skip, inserted)?)?;
        out.extend_from_slice(&base[position..start]);
        if differences && removed == inserted {
            out.extend(
                bytes
                    .iter()
                    .zip(&base[start..end])
                    .map(|(new, old)| new.wrapping_add(*old)),
            );
        } else {
            out.extend_from_slice(bytes);
        }
        position = end;
    }
    allocation::reserve(out, base.len() - position)?;
    out.extend_from_slice(&base[position..]);
    Ok(())
}

fn varint_usize(input: &mut Decoder<'_>) -> io::Result<usize> {
    usize::try_from(input.varint()?).map_err(|_| invalid("patch length exceeds address space"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(base: &[u8], target: &[u8]) -> Vec<u8> {
        let mut patch = Vec::new();
        diff(base, target, &mut patch).unwrap();
        let mut input = Decoder::new(&patch);
        let mut out = Vec::new();
        apply(base, &mut input, &mut out).unwrap();
        input.finish().unwrap();
        assert_eq!(out, target);
        patch
    }

    #[test]
    fn one_changed_field_of_a_large_value_patches_in_a_few_bytes() {
        let base: Vec<u8> = (0..4096).map(|i| (i * 7) as u8).collect();
        let mut target = base.clone();
        target[2000..2004].copy_from_slice(&[1, 2, 3, 4]);
        assert!(round_trip(&base, &target).len() <= 10);
        // Scattered same-length changes become separate hunks.
        target[10] ^= 0xff;
        target[4000] ^= 0xff;
        assert!(round_trip(&base, &target).len() <= 24);
        // Insertions and removals keep the common prefix and suffix.
        let mut inserted = base.clone();
        inserted.splice(100..100, [9, 9, 9, 9]);
        assert!(round_trip(&base, &inserted).len() <= 10);
        assert!(round_trip(&inserted, &base).len() <= 10);
        // A window that drops its oldest entry and appends one copies the rest.
        let mut window = base[8..].to_vec();
        window.extend_from_slice(&[5; 8]);
        assert!(round_trip(&base, &window).len() <= 16);
        // Two separated edits that also change the length.
        let mut both = inserted.clone();
        both[3000] ^= 0xff;
        assert!(round_trip(&base, &both).len() <= 20);
        round_trip(&[], &[1, 2, 3]);
        round_trip(&[1, 2, 3], &[]);
        round_trip(&[1, 2, 3], &[1, 2, 3, 3]);
    }

    #[test]
    fn in_place_hunks_store_differences_when_asked() {
        let mut base = vec![7_u8; 64];
        base[8..16].copy_from_slice(&0x01ff_u64.to_le_bytes());
        base[40..48].copy_from_slice(&(-5_i64).to_le_bytes());
        let mut target = base.clone();
        // A counter crosses a byte boundary and another falls by two.
        target[8..16].copy_from_slice(&0x0201_u64.to_le_bytes());
        target[40..48].copy_from_slice(&(-7_i64).to_le_bytes());
        let mut plain = Vec::new();
        diff_with(&base, &target, &mut plain, false).unwrap();
        let mut differences = Vec::new();
        diff_with(&base, &target, &mut differences, true).unwrap();
        // The same hunks; the second patch's new bytes are 2, 1 and 254.
        assert_eq!(plain.len(), differences.len());
        assert!(differences.ends_with(&[254]) && differences.windows(2).any(|w| w == [2, 1]));
        for (patch, with) in [(&plain, false), (&differences, true)] {
            let mut input = Decoder::new(patch);
            let mut out = Vec::new();
            apply_with(&base, &mut input, &mut out, with).unwrap();
            input.finish().unwrap();
            assert_eq!(out, target);
        }
        // A shifted value is inserted as it is, with or without differences.
        let mut longer = target.clone();
        longer.splice(20..20, [1, 2, 3]);
        let mut patch = Vec::new();
        diff_with(&base, &longer, &mut patch, true).unwrap();
        let mut input = Decoder::new(&patch);
        let mut out = Vec::new();
        apply_with(&base, &mut input, &mut out, true).unwrap();
        assert_eq!(out, longer);
    }

    #[test]
    fn malformed_patches_are_refused() {
        let base = [1, 2, 3, 4];
        for patch in [
            vec![0],                   // no hunks
            vec![1, 0, 0, 0],          // an empty hunk
            vec![1, 5, 0, 1, 7],       // skips past the base
            vec![1, 0, 5, 0],          // removes past the base
            vec![1, 0, 1, 2, 7],       // inserted bytes truncated
            vec![2, 0, 1, 0, 0, 1, 0], // a second hunk touching the first
        ] {
            let mut out = Vec::new();
            assert!(
                apply(&base, &mut Decoder::new(&patch), &mut out).is_err(),
                "accepted {patch:?}"
            );
        }
        assert!(diff(&base, &base, &mut Vec::new()).is_err());
    }
}
