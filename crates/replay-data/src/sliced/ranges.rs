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
    let removed: usize = ranges.iter().map(|range| span(old, range).len()).sum();
    let mut retired = allocation::vec(removed)?;
    allocation::reserve(old, fresh.len().saturating_sub(removed))?;
    // Reverse ranges leave the indices before each replacement unchanged. Moving
    // suffixes happens only on the spool thread; unchanged rows keep ownership.
    for range in ranges.iter().rev() {
        let old_span = span(old, range);
        let new_span = span(&fresh, range);
        retired.extend(old.splice(old_span, fresh.drain(new_span)));
    }
    Ok(retired)
}
