//! The windowed shape a reply of a family over time takes instead of the campaign: window `index`
//! holds the hours `index * HOURS` on, on one fixed grid, so a widget asks the windows it shows and
//! the next ahead, each reply a few blocks of the recording whatever the campaign's length.

use dashboard_query::Query;
use std::io;

/// Hours one window holds: four blocks.
pub const HOURS: i32 = 4 * 720;

/// The hours of window `index`, both included; none past an `i32`.
pub fn hours(index: u32) -> Option<(i32, i32)> {
    let first = i32::try_from(index).ok()?.checked_mul(HOURS)?;
    Some((first, first.checked_add(HOURS - 1)?))
}

/// The hours of window `index` the timeline holds, first past last where it holds none.
pub fn within(query: &Query<'_>, index: u32) -> io::Result<(i32, i32)> {
    let (first, last) = hours(index).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid window"))?;
    Ok((first.max(query.timeline.first), last.min(query.timeline.last)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_holds_its_own_hours_on_one_grid() {
        assert_eq!(hours(0), Some((0, HOURS - 1)));
        assert_eq!(hours(15_211), Some((15_211 * HOURS, 15_212 * HOURS - 1)));
        assert_eq!(hours(u32::MAX), None);
        assert_eq!(HOURS % 24, 0, "a window holds whole days");
    }
}
