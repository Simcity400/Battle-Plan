use crate::invalid;
use replay::{
    Hour,
    frame::{DecodedFrame, TableView},
    rows::{TableRows, TableSchema},
};
use std::io;

/// What one table of one decoded hour actually holds.
///
/// `Truncated` keeps the rows the collector's partial walk did reach: they are
/// real recorded rows, not a guess, and a pass may draw them. A partial walk
/// never asserts that the absent keys were deleted, so `missing` counts rows
/// the hour is silent about, not rows that are gone.
pub enum Available<'a, T: TableSchema> {
    /// Every row the hour recorded.
    Complete(&'a [T::Row]),
    /// The rows the walk reached, and how many rows it did not reach.
    Truncated { rows: &'a [T::Row], missing: u32 },
    /// The table failed for this hour, with the collector's own reason.
    Unavailable { reason: &'a str },
}

impl<'a, T: TableSchema> Available<'a, T> {
    /// The rows the hour holds. An unavailable table holds none.
    pub fn rows(&self) -> &'a [T::Row] {
        match self {
            Available::Complete(rows) | Available::Truncated { rows, .. } => rows,
            Available::Unavailable { .. } => &[],
        }
    }

    /// Whether the hour holds the whole table.
    pub fn is_complete(&self) -> bool {
        matches!(self, Available::Complete(_))
    }
}

/// The rows of `T` this hour holds, whatever state its table is in.
///
/// The only error is a decoded table holding another table's rows, which is a
/// corrupt file rather than a recorded gap.
pub fn rows<T: TableSchema>(frame: &DecodedFrame) -> io::Result<Available<'_, T>> {
    match &frame.tables[T::ID.index()] {
        TableView::Complete(rows) => Ok(Available::Complete(typed::<T>(rows, frame.hour)?)),
        TableView::Truncated { rows, missing } => Ok(Available::Truncated {
            rows: typed::<T>(rows, frame.hour)?,
            missing: *missing,
        }),
        TableView::Unavailable { reason } => Ok(Available::Unavailable { reason }),
    }
}

fn typed<T: TableSchema>(rows: &TableRows, hour: Hour) -> io::Result<&[T::Row]> {
    rows.rows::<T>().ok_or_else(|| {
        invalid(format!(
            "hour {hour} {:?} contains another table's rows",
            T::ID
        ))
    })
}
