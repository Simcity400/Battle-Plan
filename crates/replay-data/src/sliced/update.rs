//! One hour's read of an owner-sliced table as an update of its complete
//! baseline: every row of the listed owners is replaced by the read's rows of
//! those owners (possibly none), and every other owner's rows are kept.
//!
//! The update describes exactly [`merge_slice`](super::merge_slice)'s merged
//! table without materialising it: the listed owners are those the hour read
//! and those no longer live. The frame encoder writes the merged table's
//! change against its baseline by visiting only the listed owners' rows, which
//! are contiguous because the owner is the most significant part of every row
//! key, and its commit replaces those rows in the baseline in place.

use super::SlicedRow;
use crate::{
    codec::{invalid_static as invalid, put_varint},
    rows::TableRows,
};
use engine::allocation;
use std::{io, ops::Range};

/// The listed owners' replacement rows for an owner-sliced table's baseline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SliceUpdate {
    /// Ascending, distinct.
    #[doc(hidden)]
    pub owners: Vec<u32>,
    /// Rows of listed owners only, in key order.
    #[doc(hidden)]
    pub rows: TableRows,
}

/// Two tables of the same owner-sliced type, with `$wrap` naming that type's
/// `TableRows` variant; the keyed TechnologyDefinitions table has no owners
/// and merges by key instead.
macro_rules! sliced_pair {
    ($left:expr, $right:expr, |$a:ident, $b:ident, $wrap:ident| $body:expr, $other:expr) => {
        sliced_pair!(@arms ($left, $right), $a, $b, $wrap, $body, $other;
            ResearchTechnologies, ResearchInputs, ResearchAppliedBonuses, ResearchModifiers,
            AirCountryRelations, AirNavalPatrolHistories,
            RaidAirSites, StateResourceInputs, StateResources,
            StateCores, StateDynamicModifiers, CountryBuildingResources, RelationOpinions,
            RelationTuples, CountryStabilityInputs, CountryStabilityLines, CombatLogEquipment,
            NavalMissionCountries, NavalMissionCommanders, SupplyNodes, TrainInventory,
            AvailableAdvisors, Focus, CompletedFocuses, Projects, ProjectHistory)
    };
    (@arms $pair:expr, $a:ident, $b:ident, $wrap:ident, $body:expr, $other:expr; $($variant:ident),+) => {
        match $pair {
            $((TableRows::$variant($a), TableRows::$variant($b)) => {
                #[allow(unused_variables)]
                let $wrap = TableRows::$variant;
                $body
            })+
            _ => $other,
        }
    };
}

/// The rows of `owner` in `rows` (in key order).
fn owner_range<R: SlicedRow>(rows: &[R], owner: u32) -> Range<usize> {
    let start = rows.partition_point(|row| row.owner() < owner);
    start..start + rows[start..].partition_point(|row| row.owner() <= owner)
}

/// The distinct owners of `rows` (in key order), one binary search per owner.
fn each_owner<R: SlicedRow>(rows: &[R], mut visit: impl FnMut(u32, Range<usize>)) {
    let mut start = 0;
    while let Some(first) = rows.get(start) {
        let owner = first.owner();
        let end = start + rows[start..].partition_point(|row| row.owner() <= owner);
        visit(owner, start..end);
        start = end;
    }
}

impl SliceUpdate {
    /// The update [`merge_slice`](super::merge_slice) applies: `fresh` holds
    /// only owners `read` selects, in key order; the listed owners are every
    /// owner of `previous` or `fresh` that `read` selects or that is no longer
    /// `live`. Refuses what `merge_slice` refuses, with its errors.
    pub fn new(
        previous: &TableRows,
        fresh: TableRows,
        read: impl Fn(u32) -> bool,
        live: impl Fn(u32) -> bool,
    ) -> io::Result<Self> {
        fn listed<R: SlicedRow>(
            previous: &[R],
            fresh: &[R],
            read: impl Fn(u32) -> bool,
            live: impl Fn(u32) -> bool,
        ) -> io::Result<Vec<u32>> {
            if fresh.iter().any(|row| !read(row.owner()))
                || fresh.windows(2).any(|pair| pair[0].key() >= pair[1].key())
            {
                return Err(invalid(
                    "sliced read holds an unread owner's row or is not in key order",
                ));
            }
            let mut owners = Vec::new();
            let mut count = 0_usize;
            each_owner(previous, |_, _| count += 1);
            each_owner(fresh, |_, _| count += 1);
            allocation::reserve(&mut owners, count)?;
            each_owner(previous, |owner, _| {
                if read(owner) || !live(owner) {
                    owners.push(owner);
                }
            });
            each_owner(fresh, |owner, _| owners.push(owner));
            owners.sort_unstable();
            owners.dedup();
            Ok(owners)
        }
        let owners = sliced_pair!(
            previous,
            &fresh,
            |old, new, _wrap| listed(old, new, read, live)?,
            return Err(invalid("cannot merge different sliced table types"))
        );
        Ok(Self {
            owners,
            rows: fresh,
        })
    }

    /// The listed owners, ascending.
    pub fn owners(&self) -> &[u32] {
        &self.owners
    }

    /// The listed owners' rows, in key order.
    pub fn rows(&self) -> &TableRows {
        &self.rows
    }

    /// The distinct owners of the merged table, ascending.
    pub fn owners_after(&self, previous: &TableRows) -> io::Result<Vec<u32>> {
        fn merged<R: SlicedRow>(
            previous: &[R],
            fresh: &[R],
            listed: &[u32],
        ) -> io::Result<Vec<u32>> {
            let mut count = 0_usize;
            each_owner(previous, |_, _| count += 1);
            each_owner(fresh, |_, _| count += 1);
            let mut owners = allocation::vec(count)?;
            each_owner(previous, |owner, _| {
                if listed.binary_search(&owner).is_err() {
                    owners.push(owner);
                }
            });
            each_owner(fresh, |owner, _| owners.push(owner));
            owners.sort_unstable();
            owners.dedup();
            Ok(owners)
        }
        sliced_pair!(
            previous,
            &self.rows,
            |old, new, _wrap| merged(old, new, &self.owners),
            Err(invalid("cannot merge different sliced table types"))
        )
    }

    /// The merged table, copying every kept row: what
    /// [`merge_slice`](super::merge_slice) returns for the same read.
    pub fn merged(&self, previous: &TableRows) -> io::Result<TableRows> {
        let mut rows = previous.try_clone()?;
        self.clone().apply(&mut rows)?;
        Ok(rows)
    }

    /// Whether the rows are of `previous`'s type, every row's owner is
    /// listed, and the rows keep key order: what encoding and applying need.
    pub fn fits(&self, previous: &TableRows) -> bool {
        fn fits<R: SlicedRow>(fresh: &[R], listed: &[u32]) -> bool {
            fresh.windows(2).all(|pair| pair[0].key() < pair[1].key())
                && fresh
                    .iter()
                    .all(|row| listed.binary_search(&row.owner()).is_ok())
        }
        self.owners.windows(2).all(|pair| pair[0] < pair[1])
            && sliced_pair!(
                previous,
                &self.rows,
                |_old, new, _wrap| fits(new, &self.owners),
                false
            )
    }

    /// Write the merged table's change against `previous` exactly as the
    /// encoder's full comparison of the two complete tables writes it: the
    /// changed rows in key order, then the deleted keys. Only the listed
    /// owners' rows can differ. Returns whether anything changed.
    pub fn encode_changes(
        &self,
        previous: &TableRows,
        output: &mut Vec<u8>,
        deleted: &mut Vec<u8>,
    ) -> io::Result<bool> {
        fn encode<R: SlicedRow>(
            previous: &[R],
            fresh: &[R],
            listed: &[u32],
            output: &mut Vec<u8>,
            deleted: &mut Vec<u8>,
        ) -> io::Result<bool> {
            u32::try_from(fresh.len()).map_err(|_| invalid("too many rows"))?;
            allocation::reserve(output, 8)?;
            let count_at = output.len();
            output.extend_from_slice(&0_u32.to_le_bytes());
            let mut changed_count = 0_u32;
            let mut changed_key = 0_u128;
            deleted.clear();
            let mut deleted_count = 0_u32;
            let mut deleted_key = 0_u128;
            let mut next_fresh = 0;
            for &owner in listed {
                let old = &previous[owner_range(previous, owner)];
                let new_start = next_fresh;
                while fresh
                    .get(next_fresh)
                    .is_some_and(|row| row.owner() == owner)
                {
                    next_fresh += 1;
                }
                let new = &fresh[new_start..next_fresh];
                allocation::reserve(
                    deleted,
                    allocation::mul(old.len(), (R::KEY_BITS as usize).div_ceil(7).max(1))?,
                )?;
                let mut old = old.iter().peekable();
                for row in new {
                    let key = row.key();
                    while let Some(gone) = old.next_if(|previous| previous.key() < key) {
                        put_varint(deleted, gone.key() - deleted_key);
                        deleted_key = gone.key();
                        deleted_count += 1;
                    }
                    let baseline = old.next_if(|previous| previous.key() == key);
                    if baseline == Some(row) {
                        continue;
                    }
                    if row.encode_change(baseline, changed_key, output)? {
                        changed_count += 1;
                        changed_key = key;
                    }
                }
                for gone in old {
                    put_varint(deleted, gone.key() - deleted_key);
                    deleted_key = gone.key();
                    deleted_count += 1;
                }
            }
            if next_fresh != fresh.len() {
                return Err(invalid("slice update holds an unlisted owner's row"));
            }
            output[count_at..count_at + 4].copy_from_slice(&changed_count.to_le_bytes());
            allocation::reserve(output, allocation::add(4, deleted.len())?)?;
            output.extend_from_slice(&deleted_count.to_le_bytes());
            output.extend_from_slice(deleted);
            Ok(changed_count != 0 || deleted_count != 0)
        }
        sliced_pair!(
            previous,
            &self.rows,
            |old, new, _wrap| encode(old, new, &self.owners, output, deleted),
            Err(invalid("slice update rows differ from the baseline type"))
        )
    }

    /// Replace the listed owners' rows of `previous` with this update's rows
    /// and return the replaced rows. Owners whose row count is unchanged swap
    /// in place; otherwise the table is rebuilt once, moving every row, with
    /// its storage reserved before anything moves. On error `previous` is
    /// unchanged.
    pub fn apply(self, previous: &mut TableRows) -> io::Result<TableRows> {
        fn apply<R: SlicedRow>(
            previous: &mut Vec<R>,
            mut fresh: Vec<R>,
            listed: &[u32],
        ) -> io::Result<Vec<R>> {
            let mut same_counts = true;
            let mut next_fresh = 0;
            let mut removed = 0_usize;
            for &owner in listed {
                let old = owner_range(previous, owner);
                let start = next_fresh;
                while fresh
                    .get(next_fresh)
                    .is_some_and(|row| row.owner() == owner)
                {
                    next_fresh += 1;
                }
                removed += old.len();
                same_counts &= old.len() == next_fresh - start;
            }
            if next_fresh != fresh.len() {
                return Err(invalid("slice update holds an unlisted owner's row"));
            }
            if same_counts {
                // Each listed owner's fresh rows take its old rows' places;
                // `fresh` then holds the replaced rows.
                let mut next_fresh = 0;
                for &owner in listed {
                    for index in owner_range(previous, owner) {
                        std::mem::swap(&mut previous[index], &mut fresh[next_fresh]);
                        next_fresh += 1;
                    }
                }
                return Ok(fresh);
            }
            let total = allocation::add(previous.len() - removed, fresh.len())?;
            let mut rebuilt = allocation::vec(total)?;
            let mut replaced = allocation::vec(removed)?;
            let mut fresh = fresh.into_iter().peekable();
            for row in std::mem::take(previous) {
                if listed.binary_search(&row.owner()).is_ok() {
                    replaced.push(row);
                    continue;
                }
                while let Some(new) = fresh.next_if(|new| new.key() < row.key()) {
                    rebuilt.push(new);
                }
                rebuilt.push(row);
            }
            rebuilt.extend(fresh);
            *previous = rebuilt;
            Ok(replaced)
        }
        let Self { owners, rows } = self;
        sliced_pair!(
            &mut *previous,
            rows,
            |old, new, wrap| apply(old, new, &owners).map(wrap),
            Err(invalid("slice update rows differ from the baseline type"))
        )
    }
}
