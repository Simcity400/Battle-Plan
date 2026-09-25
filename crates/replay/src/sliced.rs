//! Rows of [`Cadence::Sliced`](crate::cadence::Cadence::Sliced) tables.
//!
//! A sliced table is refreshed one owner slice per hour. Each row belongs to
//! one owner and records the hour it was read. Owners are native country IDs
//! or native state IDs, one kind per table. A recorder merges each hour's read
//! into the previous complete rows; consumers read [`SlicedRow::sample_hour`],
//! never the frame's hour. Register a new sliced table here: implement
//! [`SlicedRow`] for its row and add its variant to the dispatch below.

use crate::rows::{ResearchAppliedBonusRow, ResearchInputRow, ResearchModifierRow, TableRows};
use std::io;

pub trait SlicedRow: Clone {
    /// The slice owner: a native country ID or native state ID. It must be
    /// the most significant part of the row key, so one owner's rows are
    /// contiguous in key order.
    fn owner(&self) -> u32;
    /// The hour whose boundary this row was read at.
    fn sample_hour(&self) -> i32;
}

macro_rules! sliced_by_country {
    ($($row:ty),+) => {$(
        impl SlicedRow for $row {
            fn owner(&self) -> u32 { self.country }
            fn sample_hour(&self) -> i32 { self.sample_hour }
        }
    )+};
}
sliced_by_country!(
    ResearchInputRow,
    ResearchAppliedBonusRow,
    ResearchModifierRow
);

macro_rules! dispatch {
    ($previous:expr, $fresh:expr, |$old:ident, $new:ident| $body:expr) => {
        match ($previous, $fresh) {
            (TableRows::ResearchInputs($old), TableRows::ResearchInputs($new)) => {
                Ok(TableRows::ResearchInputs($body?))
            }
            (TableRows::ResearchAppliedBonuses($old), TableRows::ResearchAppliedBonuses($new)) => {
                Ok(TableRows::ResearchAppliedBonuses($body?))
            }
            (TableRows::ResearchModifiers($old), TableRows::ResearchModifiers($new)) => {
                Ok(TableRows::ResearchModifiers($body?))
            }
            (previous, fresh) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "cannot merge {:?} rows into {:?}",
                    fresh.table(),
                    previous.table()
                ),
            )),
        }
    };
}

/// Replace the rows of every owner `read` selects in `previous` (a complete
/// table in key order) with `fresh` (in key order), in one linear pass.
/// `fresh` must hold only selected owners' rows. Rows of owners that are no
/// longer `live` are dropped. Other owners keep their rows and their own
/// sample hours.
pub fn merge_slice(
    previous: &TableRows,
    fresh: TableRows,
    read: impl Fn(u32) -> bool,
    live: impl Fn(u32) -> bool,
) -> io::Result<TableRows> {
    fn merge<R: SlicedRow>(
        old: &[R],
        new: Vec<R>,
        key: impl Fn(&R) -> u128,
        read: impl Fn(u32) -> bool,
        live: impl Fn(u32) -> bool,
    ) -> io::Result<Vec<R>> {
        if new.iter().any(|row| !read(row.owner()))
            || new.windows(2).any(|pair| key(&pair[0]) >= key(&pair[1]))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "sliced read holds an unread owner's row or is not in key order",
            ));
        }
        let mut rows = Vec::with_capacity(old.len() + new.len());
        let mut old = old
            .iter()
            .filter(|row| !read(row.owner()) && live(row.owner()))
            .peekable();
        let mut new = new.into_iter().peekable();
        loop {
            match (old.peek(), new.peek()) {
                (Some(a), Some(b)) if key(a) < key(b) => rows.push(old.next().unwrap().clone()),
                (Some(a), Some(b)) if key(a) == key(b) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "sliced read repeats a kept row's key",
                    ));
                }
                (_, Some(_)) => rows.push(new.next().unwrap()),
                (Some(_), None) => rows.push(old.next().unwrap().clone()),
                (None, None) => return Ok(rows),
            }
        }
    }
    dispatch!(previous, fresh, |old, new| merge(
        old,
        new,
        |row| row.key(),
        &read,
        &live
    ))
}

/// The distinct owners of a sliced table's rows, ascending.
pub fn owners(rows: &TableRows) -> Vec<u32> {
    fn collect<R: SlicedRow>(rows: &[R]) -> Vec<u32> {
        let mut owners: Vec<u32> = rows.iter().map(SlicedRow::owner).collect();
        owners.dedup();
        owners
    }
    match rows {
        TableRows::ResearchInputs(rows) => collect(rows),
        TableRows::ResearchAppliedBonuses(rows) => collect(rows),
        TableRows::ResearchModifiers(rows) => collect(rows),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::TableId;

    #[test]
    fn every_sliced_cadence_table_is_registered_for_merging() {
        for table in TableId::ALL {
            let merged = merge_slice(
                &TableRows::empty(table),
                TableRows::empty(table),
                |_| true,
                |_| true,
            );
            assert_eq!(
                crate::cadence::is_sliced(table),
                merged.is_ok(),
                "{table:?}"
            );
        }
    }

    fn row(country: u32, index: u32, hour: i32) -> ResearchInputRow {
        ResearchInputRow {
            country,
            index,
            sample_hour: hour,
            ..Default::default()
        }
    }

    #[test]
    fn merging_replaces_only_read_owners_and_keeps_key_order() {
        let previous = TableRows::ResearchInputs(vec![
            row(1, 0, 0),
            row(1, 1, 0),
            row(2, 0, 0),
            row(3, 0, 0),
            row(25, 0, 0),
        ]);
        let fresh = TableRows::ResearchInputs(vec![row(1, 0, 1), row(3, 0, 1)]);
        let merged = merge_slice(
            &previous,
            fresh,
            |owner| owner % 24 == 1 || owner == 3,
            |_| true,
        )
        .unwrap();
        assert_eq!(
            merged,
            TableRows::ResearchInputs(vec![row(1, 0, 1), row(2, 0, 0), row(3, 0, 1)])
        );
        assert_eq!(owners(&merged), [1, 2, 3]);
        // A fresh row of an unread owner, or out of key order, is a producer error.
        let only_one = TableRows::ResearchInputs(vec![row(2, 0, 1)]);
        assert!(merge_slice(&previous, only_one, |owner| owner == 1, |_| true).is_err());
        let unordered = TableRows::ResearchInputs(vec![row(3, 0, 1), row(1, 0, 1)]);
        assert!(merge_slice(&previous, unordered, |_| true, |_| true).is_err());
        // Mismatched tables are refused, not mixed.
        assert!(
            merge_slice(
                &previous,
                TableRows::ResearchModifiers(vec![]),
                |_| true,
                |_| true
            )
            .is_err()
        );
        // Owners that are gone lose their held rows even outside their slice.
        let gone = merge_slice(
            &merged,
            TableRows::ResearchInputs(vec![]),
            |_| false,
            |owner| owner != 2,
        )
        .unwrap();
        assert_eq!(
            gone,
            TableRows::ResearchInputs(vec![row(1, 0, 1), row(3, 0, 1)])
        );
    }
}
