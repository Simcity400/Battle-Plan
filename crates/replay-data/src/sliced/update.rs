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
use engine_allocation as allocation;
use std::{io, ops::Range};

/// Replacement rows for listed owners, or listed exact holder keys, against
/// a shared complete baseline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SliceUpdate {
    /// Certified replacements of arbitrary key ranges; empty for owner slices.
    pub ranges: Vec<Range<u128>>,
    /// Ascending, distinct.
    #[doc(hidden)]
    pub owners: Vec<u32>,
    /// Exact row keys for certificate updates of individual holders.
    /// Empty for owner slices; keyed updates never also list owners.
    pub keys: Vec<u128>,
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
            CountryMarket, MarketContractTerms, LendLeaseAgreements, ResearchTechnologies, ResearchInputs, ResearchAppliedBonuses, ResearchModifiers,
            AirCountryRelations, AirNavalPatrolHistories,
            RaidAirSites, StateResourceInputs, StateResources,
            StateCores, StateDynamicModifiers, CountryBuildingResources, RelationOpinions,
            RelationTuples, CountryStabilityInputs, CountryStabilityLines, CombatLogEquipment,
            NavalMissionCountries, NavalMissionCommanders, SupplyNodes, TradeRoutes, TrainInventory,
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
    /// Replace only the listed complete row keys, including removed holders.
    pub fn keyed(keys: Vec<u128>, rows: TableRows) -> Self {
        Self {
            ranges: Vec::new(),
            owners: Vec::new(),
            keys,
            rows,
        }
    }

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
            ranges: Vec::new(),
            keys: Vec::new(),
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
        if !self.keys.is_empty() {
            return super::try_owners(&self.merged(previous)?);
        }
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
        if !self.ranges.is_empty() {
            return self.owners.is_empty() && self.rows.range_update_fits(previous, &self.ranges);
        }
        if !self.keys.is_empty() {
            return self.owners.is_empty()
                && self.keys.windows(2).all(|pair| pair[0] < pair[1])
                && sliced_pair!(
                    previous,
                    &self.rows,
                    |_old, new, _wrap| key_fits(new, &self.keys),
                    false
                );
        }
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
        if !self.ranges.is_empty() {
            return self
                .rows
                .encode_range_update(previous, &self.ranges, output, deleted);
        }
        if !self.keys.is_empty() {
            if !self.fits(previous) {
                return Err(invalid("invalid key update"));
            }
            return sliced_pair!(
                previous,
                &self.rows,
                |old, new, _wrap| encode_keys(old, new, &self.keys, output, deleted),
                Err(invalid("key update rows differ from baseline type"))
            );
        }
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
        if !self.ranges.is_empty() {
            return self.rows.apply_range_update(previous, &self.ranges);
        }
        if !self.keys.is_empty() {
            if !self.fits(previous) {
                return Err(invalid("invalid key update"));
            }
            return sliced_pair!(
                &mut *previous,
                self.rows,
                |old, new, wrap| apply_keys(old, new, &self.keys).map(wrap),
                Err(invalid("key update rows differ from baseline type"))
            );
        }
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
        let Self { owners, rows, .. } = self;
        sliced_pair!(
            &mut *previous,
            rows,
            |old, new, wrap| apply(old, new, &owners).map(wrap),
            Err(invalid("slice update rows differ from the baseline type"))
        )
    }
}

fn key_fits<R: crate::rows::RowCodec>(fresh: &[R], keys: &[u128]) -> bool {
    fresh.windows(2).all(|pair| pair[0].key() < pair[1].key())
        && fresh
            .iter()
            .all(|row| keys.binary_search(&row.key()).is_ok())
}
fn encode_keys<R: crate::rows::RowCodec>(
    old: &[R],
    fresh: &[R],
    keys: &[u128],
    output: &mut Vec<u8>,
    deleted: &mut Vec<u8>,
) -> io::Result<bool> {
    if !key_fits(fresh, keys) {
        return Err(invalid("invalid key update rows"));
    }
    allocation::reserve(output, 4)?;
    let count_at = output.len();
    output.extend_from_slice(&0_u32.to_le_bytes());
    let mut changed = 0_u32;
    let mut changed_key = 0_u128;
    let mut gone = 0_u32;
    let mut gone_key = 0_u128;
    deleted.clear();
    for &key in keys {
        let previous = old
            .binary_search_by_key(&key, |row| row.key())
            .ok()
            .map(|i| &old[i]);
        let current = fresh
            .binary_search_by_key(&key, |row| row.key())
            .ok()
            .map(|i| &fresh[i]);
        if let Some(row) = current {
            if row.encode_change(previous, changed_key, output)? {
                changed = changed
                    .checked_add(1)
                    .ok_or(invalid("too many changed rows"))?;
                changed_key = key;
            }
        } else if previous.is_some() {
            allocation::reserve(deleted, (R::KEY_BITS as usize).div_ceil(7).max(1))?;
            put_varint(deleted, key - gone_key);
            gone_key = key;
            gone = gone
                .checked_add(1)
                .ok_or(invalid("too many removed rows"))?;
        }
    }
    output[count_at..count_at + 4].copy_from_slice(&changed.to_le_bytes());
    allocation::reserve(output, allocation::add(4, deleted.len())?)?;
    output.extend_from_slice(&gone.to_le_bytes());
    output.extend_from_slice(deleted);
    Ok(changed != 0 || gone != 0)
}
fn apply_keys<R: crate::rows::RowCodec>(
    old: &mut Vec<R>,
    mut fresh: Vec<R>,
    keys: &[u128],
) -> io::Result<Vec<R>> {
    let mut removed = 0_usize;
    let mut same_keys = true;
    for &key in keys {
        let present = old.binary_search_by_key(&key, |row| row.key()).is_ok();
        removed += usize::from(present);
        same_keys &= present == fresh.binary_search_by_key(&key, |row| row.key()).is_ok();
    }
    if same_keys {
        for row in &mut fresh {
            let index = old
                .binary_search_by_key(&row.key(), |old| old.key())
                .map_err(|_| invalid("key update lost its baseline"))?;
            std::mem::swap(row, &mut old[index]);
        }
        return Ok(fresh);
    }
    // Membership changes move retained rows once; immutable pools stay shared.
    let mut rebuilt = allocation::vec(allocation::add(old.len() - removed, fresh.len())?)?;
    let mut replaced = allocation::vec(removed)?;
    let mut fresh = fresh.into_iter().peekable();
    for row in std::mem::take(old) {
        if keys.binary_search(&row.key()).is_ok() {
            replaced.push(row);
            continue;
        }
        while let Some(new) = fresh.next_if(|new| new.key() < row.key()) {
            rebuilt.push(new);
        }
        rebuilt.push(row);
    }
    rebuilt.extend(fresh);
    *old = rebuilt;
    Ok(replaced)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::{LendLeaseAgreementRow, MarketEquipment, MarketPool};
    #[test]
    fn keyed_contract_tick_and_removal_keep_sibling_terms_and_route_allocations() {
        use crate::rows::{MarketContractRuntime, MarketContractTermsRow, SharedMarketPool};
        let pool: SharedMarketPool = MarketPool {
            items: vec![
                MarketEquipment {
                    variant: 31,
                    amount_raw: 99
                };
                5000
            ],
            ..Default::default()
        }
        .into();
        let allocation = pool.items.as_ptr();
        let mut rows: Vec<_> = (0..5000)
            .map(|id| MarketContractTermsRow {
                country: 5000,
                id,
                kind: 1,
                equipment: pool.clone(),
                prices: pool.clone(),
                runtime: MarketContractRuntime {
                    route_equipment: pool.clone(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .collect();
        let mut changed = rows[17].clone();
        changed.runtime.period_days = 8;
        let removed_key = rows[18].key();
        let storage = rows.as_ptr();
        let mut baseline = TableRows::MarketContractTerms(std::mem::take(&mut rows));
        SliceUpdate::keyed(
            vec![changed.key()],
            TableRows::MarketContractTerms(vec![changed]),
        )
        .apply(&mut baseline)
        .unwrap();
        let TableRows::MarketContractTerms(after) = &baseline else {
            unreachable!()
        };
        assert_eq!(after.as_ptr(), storage);
        assert_eq!(after[17].runtime.period_days, 8);
        assert!(
            after
                .iter()
                .all(|row| row.equipment.items.as_ptr() == allocation
                    && row.prices.items.as_ptr() == allocation
                    && row.runtime.route_equipment.items.as_ptr() == allocation)
        );
        SliceUpdate::keyed(
            vec![removed_key],
            TableRows::MarketContractTerms(Vec::new()),
        )
        .apply(&mut baseline)
        .unwrap();
        let TableRows::MarketContractTerms(after) = baseline else {
            unreachable!()
        };
        assert_eq!(after.len(), 4999);
        assert!(
            after
                .iter()
                .all(|row| row.runtime.route_equipment.items.as_ptr() == allocation)
        );
    }
    #[test]
    fn keyed_scalar_commit_keeps_all_unchanged_pool_allocations() {
        let pool: crate::rows::SharedMarketPool = MarketPool {
            items: vec![
                MarketEquipment {
                    variant: 31,
                    amount_raw: 99
                };
                5000
            ],
            ..Default::default()
        }
        .into();
        let allocation = pool.items.as_ptr();
        let mut baseline = TableRows::LendLeaseAgreements(
            (0..5000)
                .map(|id| LendLeaseAgreementRow {
                    country: 5000,
                    id,
                    monthly: pool.clone(),
                    ..Default::default()
                })
                .collect(),
        );
        let row = LendLeaseAgreementRow {
            country: 5000,
            id: 17,
            monthly: pool.clone(),
            fuel_sent_raw: 77,
            ..Default::default()
        };
        let update = SliceUpdate::keyed(vec![row.key()], TableRows::LendLeaseAgreements(vec![row]));
        let TableRows::LendLeaseAgreements(before) = &baseline else {
            unreachable!()
        };
        let storage = before.as_ptr();
        update.apply(&mut baseline).unwrap();
        let TableRows::LendLeaseAgreements(after) = &baseline else {
            unreachable!()
        };
        assert_eq!(storage, after.as_ptr());
        assert_eq!(after[17].fuel_sent_raw, 77);
        assert!(
            after
                .iter()
                .all(|row| row.monthly.items.as_ptr() == allocation)
        );
        let key = after[18].key();
        SliceUpdate::keyed(vec![key], TableRows::LendLeaseAgreements(Vec::new()))
            .apply(&mut baseline)
            .unwrap();
        let TableRows::LendLeaseAgreements(after) = &baseline else {
            unreachable!()
        };
        assert_eq!(after.len(), 4999);
        assert!(
            after
                .iter()
                .all(|row| row.monthly.items.as_ptr() == allocation)
        );
    }
}
