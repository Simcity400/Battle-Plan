use crate::{
    codec::{invalid_static as invalid, put_varint},
    rows::{ChangedRows, RowCodec},
};
use engine_allocation as allocation;
use std::{io, ops::Range};
fn span<R: RowCodec>(rows: &[R], range: &Range<u128>) -> Range<usize> {
    rows.partition_point(|row| row.key() < range.start)
        ..rows.partition_point(|row| row.key() < range.end)
}
pub fn fits<R: RowCodec>(_old: &[R], fresh: &[R], ranges: &[Range<u128>]) -> bool {
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
pub fn encode<R: RowCodec>(
    old: &[R],
    fresh: &[R],
    ranges: &[Range<u128>],
    output: &mut Vec<u8>,
    deleted: &mut Vec<u8>,
    mut changed: Option<&mut ChangedRows>,
) -> io::Result<bool> {
    if !fits(old, fresh, ranges) {
        return Err(invalid("invalid certified key ranges"));
    }
    u32::try_from(fresh.len()).map_err(|_| invalid("too many rows"))?;
    u32::try_from(old.len()).map_err(|_| invalid("too many rows"))?;
    allocation::reserve(output, 4)?;
    let count_at = output.len();
    output.extend_from_slice(&0u32.to_le_bytes());
    let (mut count, mut gone_count, mut last, mut gone_last) = (0u32, 0u32, 0u128, 0u128);
    deleted.clear();
    for range in ranges {
        let (old_span, fresh_span) = (span(old, range), span(fresh, range));
        let mut previous = old[old_span.clone()].iter().enumerate().peekable();
        for (at, row) in fresh[fresh_span.clone()].iter().enumerate() {
            while let Some((_, gone)) = previous.next_if(|(_, p)| p.key() < row.key()) {
                allocation::reserve(deleted, 19)?;
                put_varint(deleted, gone.key() - gone_last);
                gone_last = gone.key();
                gone_count += 1;
                if let Some(changed) = changed.as_deref_mut() {
                    changed.remove(gone.key());
                }
            }
            let baseline = previous.next_if(|(_, p)| p.key() == row.key());
            if baseline.map(|(_, baseline)| baseline) != Some(row)
                && row.encode_change(baseline.map(|(_, baseline)| baseline), last, output)?
            {
                count += 1;
                last = row.key();
                if let Some(changed) = changed.as_deref_mut() {
                    changed.pair(
                        (fresh_span.start + at) as u32,
                        baseline.map(|(before, _)| (old_span.start + before) as u32),
                    );
                }
            }
        }
        for (_, gone) in previous {
            allocation::reserve(deleted, 19)?;
            put_varint(deleted, gone.key() - gone_last);
            gone_last = gone.key();
            gone_count += 1;
            if let Some(changed) = changed.as_deref_mut() {
                changed.remove(gone.key());
            }
        }
    }
    output[count_at..count_at + 4].copy_from_slice(&count.to_le_bytes());
    allocation::reserve(output, allocation::add(4, deleted.len())?)?;
    output.extend_from_slice(&gone_count.to_le_bytes());
    output.extend_from_slice(deleted);
    Ok(count != 0 || gone_count != 0)
}
pub fn apply<R: RowCodec>(
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
    for range in ranges.iter().rev() {
        let old_span = span(old, range);
        let new_span = span(&fresh, range);
        retired.extend(old.splice(old_span, fresh.drain(new_span)));
    }
    Ok(retired)
}

#[doc(hidden)]
pub fn growth<R: RowCodec>(old: &[R], fresh: &[R], ranges: &[Range<u128>]) -> (usize, usize) {
    let (mut removed, mut inserted, mut peak) = (0, 0, 0);
    for range in ranges.iter().rev() {
        removed += span(old, range).len();
        inserted += span(fresh, range).len();
        peak = usize::max(peak, inserted - removed.min(inserted));
    }
    (removed, peak)
}
