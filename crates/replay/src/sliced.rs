//! Rows of [`Cadence::Sliced`](crate::cadence::Cadence::Sliced) tables.
//!
//! A sliced table is read, at each hour, for the owners of that hour's slice
//! and for every owner whose change fingerprint differs or cannot be read,
//! so a steady hour does not read the whole table; an hour whose fingerprint
//! pass is incomplete, or whose native writers are not observed, reads every
//! owner. Each row belongs to one owner, whose read hour SliceReads records.
//! Owners are native country IDs, canonical country slots or native state
//! IDs, one kind per table. A recorder merges each hour's read
//! into the previous complete rows and records each owner's read hour in
//! SliceReads ([`read_hour`]); consumers use that hour, never the frame's hour.
//! Rows carry no hour, so an owner's unchanged rows stay unchanged when it is
//! read again. Register a new sliced table here: implement [`SlicedRow`] for
//! its row and add its variant to the dispatch below.
//!
//! TechnologyDefinitions is the one keyed table: it has no owner and no
//! sample hour. Its rows are the templates the hour's rosters reached, so a
//! read replaces the rows whose keys it returned and keeps the others; a full
//! hour reads every roster and replaces the table.

use crate::rows::{
    AceRangeInputRow, AirCountryRelationRow, AirNavalPatrolHistoryRow, CountryBuildingResourceRow,
    CountryStabilityInputRow, CountryStabilityLineRow, DivisionRetreatBanRow, RaidAirSiteRow,
    RaidCountryInputRow, RelationOpinionRow, RelationTupleRow, ResearchAppliedBonusRow,
    ResearchInputRow, ResearchModifierRow, ResearchTechnologyRow, SliceReadRow, StateCoreRow,
    StateDynamicModifierRow, StateResourceInputRow, StateResourceRow, StockpileArchetypeRow,
    TableId, TableRows,
};
use std::io;

pub trait SlicedRow: Clone {
    /// The slice owner: a native country ID, canonical country slot or native
    /// state ID. It must be
    /// the most significant part of the row key, so one owner's rows are
    /// contiguous in key order.
    fn owner(&self) -> u32;
}

macro_rules! sliced_by {
    ($owner:ident: $($row:ty),+) => {$(
        impl SlicedRow for $row {
            fn owner(&self) -> u32 { self.$owner }
        }
    )+};
}
// Native country IDs.
sliced_by!(
    country: ResearchInputRow,
    ResearchAppliedBonusRow,
    ResearchModifierRow,
    ResearchTechnologyRow,
    CountryBuildingResourceRow,
    CountryStabilityInputRow,
    CountryStabilityLineRow
);
// Canonical country slots: the raid launch family keys its rows by slot.
sliced_by!(
    canonical_slot: RaidCountryInputRow,
    StockpileArchetypeRow,
    RaidAirSiteRow,
    AceRangeInputRow
);
// Canonical country slots, held in the row's `country` field.
sliced_by!(country: AirCountryRelationRow, AirNavalPatrolHistoryRow);
// Native state IDs.
sliced_by!(state: StateResourceInputRow, StateResourceRow, StateDynamicModifierRow, StateCoreRow);

/// A relation row belongs to the country whose relation status holds it.
macro_rules! sliced_by_owner_country {
    ($($row:ty),+) => {$(
        impl SlicedRow for $row {
            fn owner(&self) -> u32 { self.owner_country }
        }
    )+};
}
sliced_by_owner_country!(RelationOpinionRow, RelationTupleRow);
// The canonical slot of a division's roster owner.
impl SlicedRow for DivisionRetreatBanRow {
    fn owner(&self) -> u32 {
        self.owner
    }
}

macro_rules! dispatch {
    ($previous:expr, $fresh:expr, |$old:ident, $new:ident| $body:expr) => {
        match ($previous, $fresh) {
            (TableRows::ResearchTechnologies($old), TableRows::ResearchTechnologies($new)) => {
                Ok(TableRows::ResearchTechnologies($body?))
            }
            (TableRows::ResearchInputs($old), TableRows::ResearchInputs($new)) => {
                Ok(TableRows::ResearchInputs($body?))
            }
            (TableRows::ResearchAppliedBonuses($old), TableRows::ResearchAppliedBonuses($new)) => {
                Ok(TableRows::ResearchAppliedBonuses($body?))
            }
            (TableRows::ResearchModifiers($old), TableRows::ResearchModifiers($new)) => {
                Ok(TableRows::ResearchModifiers($body?))
            }
            (TableRows::AirCountryRelations($old), TableRows::AirCountryRelations($new)) => {
                Ok(TableRows::AirCountryRelations($body?))
            }
            (
                TableRows::AirNavalPatrolHistories($old),
                TableRows::AirNavalPatrolHistories($new),
            ) => Ok(TableRows::AirNavalPatrolHistories($body?)),
            (TableRows::RaidCountryInputs($old), TableRows::RaidCountryInputs($new)) => {
                Ok(TableRows::RaidCountryInputs($body?))
            }
            (TableRows::DivisionRetreatBans($old), TableRows::DivisionRetreatBans($new)) => {
                Ok(TableRows::DivisionRetreatBans($body?))
            }
            (TableRows::StockpileArchetypes($old), TableRows::StockpileArchetypes($new)) => {
                Ok(TableRows::StockpileArchetypes($body?))
            }
            (TableRows::RaidAirSites($old), TableRows::RaidAirSites($new)) => {
                Ok(TableRows::RaidAirSites($body?))
            }
            (TableRows::AceRangeInputs($old), TableRows::AceRangeInputs($new)) => {
                Ok(TableRows::AceRangeInputs($body?))
            }
            (TableRows::StateResourceInputs($old), TableRows::StateResourceInputs($new)) => {
                Ok(TableRows::StateResourceInputs($body?))
            }
            (TableRows::StateResources($old), TableRows::StateResources($new)) => {
                Ok(TableRows::StateResources($body?))
            }
            (TableRows::StateCores($old), TableRows::StateCores($new)) => {
                Ok(TableRows::StateCores($body?))
            }
            (TableRows::StateDynamicModifiers($old), TableRows::StateDynamicModifiers($new)) => {
                Ok(TableRows::StateDynamicModifiers($body?))
            }
            (
                TableRows::CountryBuildingResources($old),
                TableRows::CountryBuildingResources($new),
            ) => Ok(TableRows::CountryBuildingResources($body?)),
            (TableRows::RelationOpinions($old), TableRows::RelationOpinions($new)) => {
                Ok(TableRows::RelationOpinions($body?))
            }
            (TableRows::RelationTuples($old), TableRows::RelationTuples($new)) => {
                Ok(TableRows::RelationTuples($body?))
            }
            (TableRows::CountryStabilityInputs($old), TableRows::CountryStabilityInputs($new)) => {
                Ok(TableRows::CountryStabilityInputs($body?))
            }
            (TableRows::CountryStabilityLines($old), TableRows::CountryStabilityLines($new)) => {
                Ok(TableRows::CountryStabilityLines($body?))
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
/// sample hours. For the keyed table, `read` and `live` are ignored: fresh
/// rows replace the rows with their keys and every other row is kept.
pub fn merge_slice(
    previous: &TableRows,
    fresh: TableRows,
    read: impl Fn(u32) -> bool,
    live: impl Fn(u32) -> bool,
) -> io::Result<TableRows> {
    if let (TableRows::TechnologyDefinitions(old), TableRows::TechnologyDefinitions(new)) =
        (previous, &fresh)
    {
        if new.windows(2).any(|pair| pair[0].key() >= pair[1].key()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "keyed sliced read is not in key order",
            ));
        }
        let mut rows = Vec::with_capacity(old.len().max(new.len()));
        let mut old = old.iter().peekable();
        let mut new = new.iter().peekable();
        loop {
            match (old.peek(), new.peek()) {
                (Some(a), Some(b)) if a.key() < b.key() => rows.push(old.next().unwrap().clone()),
                (Some(a), Some(b)) if a.key() == b.key() => {
                    old.next();
                    rows.push(new.next().unwrap().clone());
                }
                (_, Some(_)) => rows.push(new.next().unwrap().clone()),
                (Some(_), None) => rows.push(old.next().unwrap().clone()),
                (None, None) => return Ok(TableRows::TechnologyDefinitions(rows)),
            }
        }
    }
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

/// The hour `owner`'s rows of `table` were read at, from SliceReads rows
/// (in key order).
pub fn read_hour(reads: &[SliceReadRow], table: TableId, owner: u32) -> Option<i32> {
    let key = (table.index() as u32, owner);
    reads
        .binary_search_by_key(&key, |row| (row.table, row.owner))
        .ok()
        .map(|index| reads[index].sample_hour)
}

/// Whether a sliced table has no owners: its read replaces rows by key
/// ([`merge_slice`]) and SliceReads has no rows for it.
pub fn is_keyed(table: TableId) -> bool {
    table == TableId::TechnologyDefinitions
}

/// Whether a complete read of this sliced table replaces the held table.
/// A technology roster covers the whole template database (ECE9E0 creates one
/// instance per template), so any complete, non-empty definitions read is the
/// whole table and templates removed by `reloadtechnologies` leave with it.
pub fn replaces_whole(fresh: &TableRows) -> bool {
    matches!(fresh, TableRows::TechnologyDefinitions(rows) if !rows.is_empty())
}

/// The distinct owners of a sliced table's rows, ascending.
pub fn owners(rows: &TableRows) -> Vec<u32> {
    fn collect<R: SlicedRow>(rows: &[R]) -> Vec<u32> {
        let mut owners: Vec<u32> = rows.iter().map(SlicedRow::owner).collect();
        owners.dedup();
        owners
    }
    match rows {
        TableRows::ResearchTechnologies(rows) => collect(rows),
        TableRows::ResearchInputs(rows) => collect(rows),
        TableRows::ResearchAppliedBonuses(rows) => collect(rows),
        TableRows::ResearchModifiers(rows) => collect(rows),
        TableRows::AirCountryRelations(rows) => collect(rows),
        TableRows::AirNavalPatrolHistories(rows) => collect(rows),
        TableRows::RaidCountryInputs(rows) => collect(rows),
        TableRows::StockpileArchetypes(rows) => collect(rows),
        TableRows::DivisionRetreatBans(rows) => collect(rows),
        TableRows::RaidAirSites(rows) => collect(rows),
        TableRows::AceRangeInputs(rows) => collect(rows),
        TableRows::StateResourceInputs(rows) => collect(rows),
        TableRows::StateResources(rows) => collect(rows),
        TableRows::StateDynamicModifiers(rows) => collect(rows),
        TableRows::StateCores(rows) => collect(rows),
        TableRows::CountryBuildingResources(rows) => collect(rows),
        TableRows::RelationOpinions(rows) => collect(rows),
        TableRows::RelationTuples(rows) => collect(rows),
        TableRows::CountryStabilityInputs(rows) => collect(rows),
        TableRows::CountryStabilityLines(rows) => collect(rows),
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
            progress_raw: i64::from(hour),
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

    #[test]
    fn read_hours_are_looked_up_by_table_and_owner() {
        let read = |table: TableId, owner, sample_hour| SliceReadRow {
            table: table.index() as u32,
            owner,
            sample_hour,
        };
        let reads = [
            read(TableId::ResearchTechnologies, 1, 5),
            read(TableId::ResearchTechnologies, 25, 6),
            read(TableId::ResearchInputs, 1, 7),
        ];
        let mut reads = reads.to_vec();
        reads.sort_by_key(|row| (row.table, row.owner));
        assert_eq!(
            read_hour(&reads, TableId::ResearchTechnologies, 25),
            Some(6)
        );
        assert_eq!(read_hour(&reads, TableId::ResearchInputs, 1), Some(7));
        assert_eq!(read_hour(&reads, TableId::ResearchInputs, 25), None);
        assert!(is_keyed(TableId::TechnologyDefinitions) && !is_keyed(TableId::ResearchInputs));
    }

    fn definition(handle: u32, year: i32) -> crate::rows::TechnologyDefinitionRow {
        crate::rows::TechnologyDefinitionRow {
            technology_handle: handle,
            year,
            ..Default::default()
        }
    }

    #[test]
    fn keyed_definitions_replace_returned_handles_and_keep_the_rest() {
        let previous = TableRows::TechnologyDefinitions(vec![
            definition(1, 1936),
            definition(3, 1936),
            definition(5, 1936),
        ]);
        // A read that reached handles 3 and 4 only; the owner filter is ignored.
        let fresh =
            TableRows::TechnologyDefinitions(vec![definition(3, 1940), definition(4, 1941)]);
        let merged = merge_slice(&previous, fresh, |_| false, |_| false).unwrap();
        assert_eq!(
            merged,
            TableRows::TechnologyDefinitions(vec![
                definition(1, 1936),
                definition(3, 1940),
                definition(4, 1941),
                definition(5, 1936),
            ])
        );
        // An empty read (no live country in the slice) keeps every row.
        let empty = merge_slice(
            &merged,
            TableRows::TechnologyDefinitions(vec![]),
            |_| true,
            |_| true,
        )
        .unwrap();
        assert_eq!(empty, merged);
        assert!(owners(&merged).is_empty());
        // A complete non-empty definitions read is the whole table; an empty one is not.
        assert!(replaces_whole(&TableRows::TechnologyDefinitions(vec![
            definition(1, 0)
        ])));
        assert!(!replaces_whole(&TableRows::TechnologyDefinitions(vec![])));
        assert!(!replaces_whole(&TableRows::ResearchInputs(vec![row(
            1, 0, 0
        )])));
        let unordered = TableRows::TechnologyDefinitions(vec![definition(4, 0), definition(4, 0)]);
        assert!(merge_slice(&merged, unordered, |_| true, |_| true).is_err());
    }

    /// Merges hour 25's slice (owner 1, plus owner 3 early) into hour 24's
    /// complete rows, `parts` rows per owner, marking a row's hour in `mark`,
    /// and returns each surviving row's (owner, mark).
    fn merged_marks<R: SlicedRow>(
        wrap: fn(Vec<R>) -> TableRows,
        unwrap: fn(&TableRows) -> &[R],
        row: fn(u32, u32, i32) -> R,
        mark: fn(&R) -> i32,
        parts: u32,
    ) -> Vec<(u32, i32)> {
        let rows = |owners: &[u32], hour| {
            let mut rows = Vec::new();
            for &owner in owners {
                rows.extend((0..parts).map(|part| row(owner, part, hour)));
            }
            wrap(rows)
        };
        let merged = merge_slice(
            &rows(&[1, 2, 3], 24),
            rows(&[1, 3], 25),
            |owner| owner == 1 || owner == 3,
            |_| true,
        )
        .unwrap();
        assert_eq!(owners(&merged), [1, 2, 3]);
        let mut marks: Vec<(u32, i32)> = unwrap(&merged)
            .iter()
            .map(|row| (row.owner(), mark(row)))
            .collect();
        marks.dedup();
        marks
    }

    #[test]
    fn every_raid_family_merges_by_its_owner() {
        let expected = [(1, 25), (2, 24), (3, 25)];
        macro_rules! family {
            ($variant:ident, $parts:literal, $field:ident, |$owner:ident, $part:ident, $hour:ident| $row:expr) => {
                assert_eq!(
                    merged_marks(
                        TableRows::$variant,
                        |rows| match rows {
                            TableRows::$variant(rows) => rows,
                            _ => unreachable!(),
                        },
                        |$owner, $part, $hour| {
                            let _ = $part;
                            $row
                        },
                        |row| row.$field as i32,
                        $parts,
                    ),
                    expected,
                    stringify!($variant)
                );
            };
        }
        // The launch family's hour source, marked by its reservation list.
        assert_eq!(
            merged_marks(
                TableRows::RaidCountryInputs,
                |rows| match rows {
                    TableRows::RaidCountryInputs(rows) => rows,
                    _ => unreachable!(),
                },
                |owner, _, hour| RaidCountryInputRow {
                    canonical_slot: owner,
                    convoy_reservations: vec![hour as u32],
                    ..Default::default()
                },
                |row| row.convoy_reservations[0] as i32,
                1,
            ),
            expected,
            "RaidCountryInputs"
        );
        family!(RaidAirSites, 2, base, |owner, part, hour| RaidAirSiteRow {
            canonical_slot: owner,
            ordinal: part,
            base: hour as u64,
            ..Default::default()
        });
        family!(AceRangeInputs, 2, range, |owner, part, hour| {
            AceRangeInputRow {
                canonical_slot: owner,
                ace: u64::from(part) << 32 | 0x77,
                range: i64::from(hour),
                ..Default::default()
            }
        });
        family!(StateResourceInputs, 1, gate, |owner, part, hour| {
            StateResourceInputRow {
                state: owner,
                gate: hour as u8,
                ..Default::default()
            }
        });
        family!(StateResources, 2, pool, |owner, part, hour| {
            StateResourceRow {
                state: owner,
                resource: 0x3000 + part,
                pool: i64::from(hour),
                ..Default::default()
            }
        });
        family!(
            StateDynamicModifiers,
            2,
            scope_state,
            |owner, part, hour| {
                StateDynamicModifierRow {
                    state: owner,
                    ordinal: part,
                    scope_state: hour,
                    ..Default::default()
                }
            }
        );
        family!(CountryBuildingResources, 2, amount, |owner, part, hour| {
            CountryBuildingResourceRow {
                country: owner,
                ordinal: part,
                amount: i64::from(hour),
                ..Default::default()
            }
        });
    }
}
