
use dashboard_query::Query;
use std::io;

pub const HOURS: i32 = 4 * 720;

pub fn hours(index: u32) -> Option<(i32, i32)> {
    let first = i32::try_from(index).ok()?.checked_mul(HOURS)?;
    Some((first, first.checked_add(HOURS - 1)?))
}

pub fn within(query: &Query<'_>, index: u32) -> io::Result<(i32, i32)> {
    let (first, last) = hours(index).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid window"))?;
    Ok((first.max(query.timeline.first), last.min(query.timeline.last)))
}

