//! Certified key-range replacements, including insertions and deletions.
use crate::{
    codec::{invalid_static as invalid, put_varint},
    rows::RowCodec,
};
use engine_allocation as allocation;
use std::{io, ops::Range};
fn span<R: RowCodec>(rows: &[R], range: &Range<u128>) -> Range<usize> {
    rows.partition_point(|row| row.key() < range.start)
        ..rows.partition_point(|row| row.key() < range.end)
}
pub(crate) fn fits<R: RowCodec>(_old: &[R], fresh: &[R], ranges: &[Range<u128>]) -> bool {
    ranges.iter().all(|range| range.start < range.end)
        && ranges.windows(2).all(|pair| pair[0].end <= pair[1].start)
        && fresh.windows(2).all(|pair| pair[0].key() < pair[1].key())
        && fresh.iter().all(|row| {
            let at = ranges.partition_point(|range| range.end <= row.key());
            ranges
                .get(at)
                .is_some_and(|range| range.contains(&row.key()))
        })
}
pub(crate) fn encode<R: RowCodec>(
    old: &[R],
    fresh: &[R],
    ranges: &[Range<u128>],
    output: &mut Vec<u8>,
    deleted: &mut Vec<u8>,
) -> io::Result<bool> {
    if !fits(old, fresh, ranges) {
        return Err(invalid("invalid certified key ranges"));
    }
    allocation::reserve(output, 4)?;
    let count_at = output.len();
    output.extend_from_slice(&0u32.to_le_bytes());
    let (mut count, mut gone_count, mut last, mut gone_last) = (0u32, 0u32, 0u128, 0u128);
    deleted.clear();
    for range in ranges {
        let mut previous = old[span(old, range)].iter().peekable();
        for row in &fresh[span(fresh, range)] {
            while let Some(gone) = previous.next_if(|p| p.key() < row.key()) {
                allocation::reserve(deleted, 19)?;
                put_varint(deleted, gone.key() - gone_last);
                gone_last = gone.key();
                gone_count += 1;
            }
            let baseline = previous.next_if(|p| p.key() == row.key());
            if baseline != Some(row) && row.encode_change(baseline, last, output)? {
                count += 1;
                last = row.key();
            }
        }
        for gone in previous {
            allocation::reserve(deleted, 19)?;
            put_varint(deleted, gone.key() - gone_last);
            gone_last = gone.key();
            gone_count += 1;
        }
    }
    output[count_at..count_at + 4].copy_from_slice(&count.to_le_bytes());
    allocation::reserve(output, allocation::add(4, deleted.len())?)?;
    output.extend_from_slice(&gone_count.to_le_bytes());
    output.extend_from_slice(deleted);
    Ok(count != 0 || gone_count != 0)
}
pub(crate) fn apply<R: RowCodec>(
    old: &mut Vec<R>,
    mut fresh: Vec<R>,
    ranges: &[Range<u128>],
) -> io::Result<Vec<R>> {
    if !fits(old, &fresh, ranges) {
        return Err(invalid("invalid certified key ranges"));
    }
    let (removed, peak) = growth(old, &fresh, ranges);
    let mut retired = allocation::vec(removed)?;
    allocation::reserve(old, peak)?;
    // Reverse ranges leave the indices before each replacement unchanged. Moving
    // suffixes happens only on the spool thread; unchanged rows keep ownership.
    for range in ranges.iter().rev() {
        let old_span = span(old, range);
        let new_span = span(&fresh, range);
        retired.extend(old.splice(old_span, fresh.drain(new_span)));
    }
    Ok(retired)
}

/// Returns removed rows and peak growth while reverse splices precede earlier deletions.
fn growth<R: RowCodec>(old: &[R], fresh: &[R], ranges: &[Range<u128>]) -> (usize, usize) {
    let (mut removed, mut inserted, mut peak) = (0, 0, 0);
    for range in ranges.iter().rev() {
        removed += span(old, range).len();
        inserted += span(fresh, range).len();
        peak = usize::max(peak, inserted - removed.min(inserted));
    }
    (removed, peak)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::ConstructionLineOutputRow;

    fn rows(lines: &[u64]) -> Vec<ConstructionLineOutputRow> {
        lines
            .iter()
            .map(|&line| ConstructionLineOutputRow {
                line,
                ..Default::default()
            })
            .collect()
    }

    /// Reverse splicing inserts a later range before an earlier range frees space.
    #[test]
    fn replacement_reserves_the_largest_intermediate_length() {
        let ranges = [1..3, 10..12];
        assert_eq!(growth(&rows(&[1, 2]), &rows(&[10, 11]), &ranges), (2, 2));
        assert_eq!(growth(&rows(&[1, 2]), &rows(&[2, 10]), &ranges), (2, 1));
        assert_eq!(growth(&rows(&[10, 11]), &rows(&[1, 2]), &ranges), (2, 0));
        let mut old = rows(&[1, 2]);
        let retired = apply(&mut old, rows(&[10, 11]), &ranges).unwrap();
        assert_eq!((old, retired), (rows(&[10, 11]), rows(&[1, 2])));
    }
}
