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
    rows::{ChangedRows, TableId, TableRows, count_header},
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
        sliced_pair!(@list ($left, $right), $a, $b, $wrap, $body, $other;)
    };
    (@list $pair:expr, $a:ident, $b:ident, $wrap:ident, $body:expr, $other:expr; $($extra:ident),*) => {
        sliced_pair!(@arms $pair, $a, $b, $wrap, $body, $other;
            CountryMarket, MarketContractTerms, LendLeaseAgreements, ResearchTechnologies, ResearchInputs, ResearchAppliedBonuses, ResearchModifiers,
            AirCountryRelations, AirNavalPatrolHistories,
            RaidAirSites, StateResourceInputs, StateResources,
            StateCores, StateDynamicModifiers, CountryBuildingResources, RelationOpinions,
            RelationTuples, CountryStabilityInputs, CountryStabilityLines, CombatLogEquipment,
            NavalMissionCountries, NavalMissionCommanders, SupplyNodes, TradeRoutes, TrainInventory,
            AvailableAdvisors, Focus, CompletedFocuses, Projects, ProjectHistory $(, $extra)*)
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

/// Two tables of a type that takes keyed holder updates: every owner-sliced type, and the
/// tables of [`takes_keyed_updates`].
macro_rules! keyed_pair {
    ($left:expr, $right:expr, |$a:ident, $b:ident, $wrap:ident| $body:expr, $other:expr) => {
        sliced_pair!(@list ($left, $right), $a, $b, $wrap, $body, $other;
            Divisions, DivisionStatus, DivisionMovement, DivisionPanelStats, DivisionLandInputs,
            DivisionRetreatBans, MovementStateTerms, CoreSupplyAccess, CountryUnitSpeedEntries,
            RiverRelations, DivisionTransportSpeeds, WeatherProvinces, DivisionTemplateIcons,
            AirBaseRegistrations, CharacterRoleTraits, ScriptVariables)
    };
}

/// Whether the producer of `table`, a table whose rows change by key each hour and that no
/// owner slicing reads, publishes only its changed, new and removed rows against a shared
/// baseline ([`SliceUpdate::keyed`]). [`keyed_pair!`] takes every one of them.
pub fn takes_keyed_updates(table: TableId) -> bool {
    let empty = TableRows::empty(table);
    sliced_pair!(@arms (&empty, &empty), _old, _new, _wrap, true, false;
        Divisions, DivisionStatus, DivisionMovement, DivisionPanelStats, DivisionLandInputs,
        DivisionRetreatBans, MovementStateTerms, CoreSupplyAccess, CountryUnitSpeedEntries,
        RiverRelations, DivisionTransportSpeeds, WeatherProvinces, DivisionTemplateIcons,
        AirBaseRegistrations, CharacterRoleTraits, ScriptVariables)
}

/// `rows.partition_point(holds)`, probing outward from the front so short prefixes stay local.
fn gallop<R>(rows: &[R], holds: impl Fn(&R) -> bool) -> usize {
    let mut low = 0;
    let mut high = 1;
    while high <= rows.len() && holds(&rows[high - 1]) {
        low = high;
        high = high.saturating_mul(2);
    }
    low + rows[low..high.min(rows.len())].partition_point(holds)
}

/// The rows of `owner` in `rows` (in key order), searched onward from an earlier owner's end.
fn owner_range_from<R: SlicedRow>(rows: &[R], from: usize, owner: u32) -> Range<usize> {
    let from = if from > 0 && rows[from - 1].owner() >= owner {
        0
    } else {
        from
    };
    let start = from + gallop(&rows[from..], |row| row.owner() < owner);
    start..start + gallop(&rows[start..], |row| row.owner() <= owner)
}

/// The distinct owners of `rows` (in key order), each found next to the last.
fn owner_ranges<R: SlicedRow>(rows: &[R]) -> impl Iterator<Item = (u32, Range<usize>)> + '_ {
    let mut start = 0;
    std::iter::from_fn(move || {
        let first = rows.get(start)?;
        let owner = first.owner();
        let end = start + gallop(&rows[start..], |row| row.owner() <= owner);
        let range = start..end;
        start = end;
        Some((owner, range))
    })
}

/// Merge owner groups, not rows. Both inputs are already ordered, so neither
/// sorting nor a membership check for every row is necessary.
fn merge_owners<R: SlicedRow>(
    previous: &[R],
    fresh: &[R],
    keep: impl Fn(u32) -> bool,
) -> io::Result<Vec<u32>> {
    let count = allocation::add(owner_ranges(previous).count(), owner_ranges(fresh).count())?;
    let mut owners = allocation::vec(count)?;
    let mut old = owner_ranges(previous)
        .map(|(owner, _)| owner)
        .filter(|&owner| keep(owner))
        .peekable();
    for (owner, _) in owner_ranges(fresh) {
        while let Some(kept) = old.next_if(|&kept| kept < owner) {
            owners.push(kept);
        }
        if old.peek() == Some(&owner) {
            old.next();
        }
        owners.push(owner);
    }
    owners.extend(old);
    Ok(owners)
}

/// Consume one fresh owner's group. Unlisted owners remain unconsumed, so the
/// caller's final length check still refuses them.
fn take_owner<R: SlicedRow>(rows: &[R], next: &mut usize, owner: u32) -> Range<usize> {
    let start = *next;
    if rows.get(start).is_some_and(|row| row.owner() == owner) {
        *next += gallop(&rows[start..], |row| row.owner() <= owner);
    }
    start..*next
}

impl SliceUpdate {
    /// Whether this update replaces the held row keyed `key` of `table`: a row of a listed
    /// owner (the top 32 of the table's key bits), in a listed range, or a listed key.
    pub fn replaces(&self, table: TableId, key: u128) -> bool {
        let owner = (key >> table.key_bits().saturating_sub(32)) as u32;
        self.owners.binary_search(&owner).is_ok()
            || self.ranges.iter().any(|range| range.contains(&key))
            || self.keys.binary_search(&key).is_ok()
    }

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
            if fresh.windows(2).any(|pair| pair[0].key() >= pair[1].key())
                || owner_ranges(fresh).any(|(owner, _)| !read(owner))
            {
                return Err(invalid(
                    "sliced read holds an unread owner's row or is not in key order",
                ));
            }
            merge_owners(previous, fresh, |owner| read(owner) || !live(owner))
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
                && keyed_pair!(
                    previous,
                    &self.rows,
                    |_old, new, _wrap| key_fits(new, &self.keys),
                    false
                );
        }
        fn fits<R: SlicedRow>(fresh: &[R], listed: &[u32]) -> bool {
            fresh.windows(2).all(|pair| pair[0].key() < pair[1].key())
                && owner_ranges(fresh).all(|(owner, _)| listed.binary_search(&owner).is_ok())
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
        self.encode_changes_with(previous, output, deleted, None)
    }

    /// [`Self::encode_changes`], recording in `changed` each row of an owner or key update it
    /// writes (its index in the update's rows beside its baseline row's) and each key it
    /// deletes; an owner update writes nothing when no row changed.
    pub fn encode_changes_recorded(
        &self,
        previous: &TableRows,
        output: &mut Vec<u8>,
        deleted: &mut Vec<u8>,
        changed: &mut ChangedRows,
    ) -> io::Result<bool> {
        self.encode_changes_with(previous, output, deleted, Some(changed))
    }

    fn encode_changes_with(
        &self,
        previous: &TableRows,
        output: &mut Vec<u8>,
        deleted: &mut Vec<u8>,
        changed: Option<&mut ChangedRows>,
    ) -> io::Result<bool> {
        if !self.ranges.is_empty() {
            return self
                .rows
                .encode_range_update(previous, &self.ranges, output, deleted, changed);
        }
        if !self.keys.is_empty() {
            if !self.fits(previous) {
                return Err(invalid("invalid key update"));
            }
            return keyed_pair!(
                previous,
                &self.rows,
                |old, new, _wrap| encode_keys(old, new, &self.keys, output, deleted, changed),
                Err(invalid("key update rows differ from baseline type"))
            );
        }
        fn encode<R: SlicedRow>(
            previous: &[R],
            fresh: &[R],
            listed: &[u32],
            output: &mut Vec<u8>,
            deleted: &mut Vec<u8>,
            mut changed: Option<&mut ChangedRows>,
        ) -> io::Result<bool> {
            u32::try_from(fresh.len()).map_err(|_| invalid("too many rows"))?;
            u32::try_from(previous.len()).map_err(|_| invalid("too many rows"))?;
            // A recorded merge writes its header only once a row differs.
            let start = output.len();
            let mut count_at = None;
            if changed.is_none() {
                count_at = Some(count_header(output)?);
            }
            let mut changed_count = 0_u32;
            let mut changed_key = 0_u128;
            deleted.clear();
            let mut deleted_count = 0_u32;
            let mut deleted_key = 0_u128;
            let mut next_fresh = 0;
            let mut next_old = 0;
            for &owner in listed {
                let old = owner_range_from(previous, next_old, owner);
                next_old = old.end;
                let old_start = old.start;
                let old = &previous[old];
                let new = take_owner(fresh, &mut next_fresh, owner);
                let new_start = new.start;
                let new = &fresh[new];
                if old == new {
                    continue;
                }
                allocation::reserve(
                    deleted,
                    allocation::mul(old.len(), (R::KEY_BITS as usize).div_ceil(7).max(1))?,
                )?;
                let mut old = old.iter().enumerate().peekable();
                for (at, row) in new.iter().enumerate() {
                    let key = row.key();
                    while let Some((_, gone)) = old.next_if(|(_, previous)| previous.key() < key) {
                        put_varint(deleted, gone.key() - deleted_key);
                        deleted_key = gone.key();
                        deleted_count += 1;
                        if let Some(changed) = &mut changed {
                            changed.remove(gone.key());
                        }
                    }
                    let baseline = old.next_if(|(_, previous)| previous.key() == key);
                    if baseline.is_some_and(|(_, baseline)| baseline == row) {
                        continue;
                    }
                    if count_at.is_none() {
                        count_at = Some(count_header(output)?);
                    }
                    if row.encode_change(baseline.map(|(_, baseline)| baseline), changed_key, output)? {
                        changed_count += 1;
                        changed_key = key;
                        if let Some(changed) = &mut changed {
                            // Both lengths were checked to fit 32 bits above.
                            let before = baseline.map(|(before, _)| (old_start + before) as u32);
                            changed.pair((new_start + at) as u32, before);
                        }
                    }
                }
                for (_, gone) in old {
                    put_varint(deleted, gone.key() - deleted_key);
                    deleted_key = gone.key();
                    deleted_count += 1;
                    if let Some(changed) = &mut changed {
                        changed.remove(gone.key());
                    }
                }
            }
            if next_fresh != fresh.len() {
                return Err(invalid("slice update holds an unlisted owner's row"));
            }
            if changed.is_some() && changed_count == 0 && deleted_count == 0 {
                output.truncate(start);
                return Ok(false);
            }
            let count_at = match count_at {
                Some(at) => at,
                None => count_header(output)?,
            };
            output[count_at..count_at + 4].copy_from_slice(&changed_count.to_le_bytes());
            allocation::reserve(output, allocation::add(4, deleted.len())?)?;
            output.extend_from_slice(&deleted_count.to_le_bytes());
            output.extend_from_slice(deleted);
            Ok(changed_count != 0 || deleted_count != 0)
        }
        sliced_pair!(
            previous,
            &self.rows,
            |old, new, _wrap| encode(old, new, &self.owners, output, deleted, changed),
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
            return keyed_pair!(
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
            let mut next_old = 0;
            let mut removed = 0_usize;
            for &owner in listed {
                let old = owner_range_from(previous, next_old, owner);
                next_old = old.end;
                let new = take_owner(&fresh, &mut next_fresh, owner);
                removed += old.len();
                same_counts &= old.len() == new.len();
            }
            if next_fresh != fresh.len() {
                return Err(invalid("slice update holds an unlisted owner's row"));
            }
            if same_counts {
                // Each listed owner's fresh rows take its old rows' places;
                // `fresh` then holds the replaced rows.
                let mut next_fresh = 0;
                let mut next_old = 0;
                for &owner in listed {
                    let old = owner_range_from(previous, next_old, owner);
                    next_old = old.end;
                    for index in old {
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
            let mut listed = listed.iter().copied().peekable();
            for row in std::mem::take(previous) {
                while listed.peek().is_some_and(|&owner| owner < row.owner()) {
                    listed.next();
                }
                if listed.peek() == Some(&row.owner()) {
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
    mut recorded: Option<&mut ChangedRows>,
) -> io::Result<bool> {
    if !key_fits(fresh, keys) {
        return Err(invalid("invalid key update rows"));
    }
    u32::try_from(fresh.len()).map_err(|_| invalid("too many rows"))?;
    u32::try_from(old.len()).map_err(|_| invalid("too many rows"))?;
    allocation::reserve(output, 4)?;
    let count_at = output.len();
    output.extend_from_slice(&0_u32.to_le_bytes());
    let mut changed = 0_u32;
    let mut changed_key = 0_u128;
    let mut gone = 0_u32;
    let mut gone_key = 0_u128;
    deleted.clear();
    for &key in keys {
        let before = old.binary_search_by_key(&key, |row| row.key()).ok();
        let previous = before.map(|i| &old[i]);
        let at = fresh.binary_search_by_key(&key, |row| row.key()).ok();
        if let Some(at) = at {
            if fresh[at].encode_change(previous, changed_key, output)? {
                changed = changed
                    .checked_add(1)
                    .ok_or(invalid("too many changed rows"))?;
                changed_key = key;
                if let Some(recorded) = recorded.as_deref_mut() {
                    // Both lengths were checked to fit 32 bits above.
                    recorded.pair(at as u32, before.map(|before| before as u32));
                }
            }
        } else if previous.is_some() {
            allocation::reserve(deleted, (R::KEY_BITS as usize).div_ceil(7).max(1))?;
            put_varint(deleted, key - gone_key);
            gone_key = key;
            gone = gone
                .checked_add(1)
                .ok_or(invalid("too many removed rows"))?;
            if let Some(recorded) = recorded.as_deref_mut() {
                recorded.remove(key);
            }
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
    use crate::rows::{LendLeaseAgreementRow, MarketEquipment, MarketPool, ResearchInputRow};

    /// [`SliceUpdate::replaces`] finds a held row's owner in the top 32 of its key bits, for
    /// every fixture row of every owner-sliced table.
    #[test]
    fn every_sliced_table_keeps_its_owner_in_the_top_32_key_bits() {
        let mut sliced = 0;
        for table in TableId::ALL {
            let empty = TableRows::empty(table);
            if !sliced_pair!(&empty, &empty, |_rows, _other, _wrap| true, false) {
                continue;
            }
            sliced += 1;
            let rows = crate::rows::sample_extended_table(table, 1)
                .unwrap_or_else(|| panic!("{table:?} has no fixture rows"));
            let bits = table.key_bits();
            sliced_pair!(
                &rows,
                &rows,
                |rows, _other, _wrap| {
                    assert!(!rows.is_empty(), "{table:?} has no fixture rows");
                    for row in rows.iter() {
                        let key = row.key();
                        assert_eq!((key >> bits.saturating_sub(32)) as u32, row.owner(), "{table:?}");
                        let update = SliceUpdate {
                            ranges: Vec::new(),
                            owners: vec![row.owner()],
                            keys: Vec::new(),
                            rows: TableRows::empty(table),
                        };
                        assert!(update.replaces(table, key), "{table:?}");
                        let other = SliceUpdate {
                            owners: vec![row.owner().wrapping_add(1)],
                            ..update
                        };
                        assert!(!other.replaces(table, key), "{table:?}");
                    }
                },
                unreachable!()
            );
        }
        assert_eq!(sliced, 30);
    }

    /// Every keyed producer table takes keyed updates, and none is owner-sliced.
    #[test]
    fn keyed_producer_tables_take_keyed_updates_and_are_not_sliced() {
        let mut keyed = 0;
        for table in TableId::ALL {
            let empty = TableRows::empty(table);
            if !takes_keyed_updates(table) {
                continue;
            }
            keyed += 1;
            assert!(keyed_pair!(&empty, &empty, |_old, _new, _wrap| true, false), "{table:?}");
            assert!(!sliced_pair!(&empty, &empty, |_old, _new, _wrap| true, false), "{table:?}");
        }
        assert_eq!(keyed, 16);
    }

    fn input(country: u32, index: u32, progress: i64) -> ResearchInputRow {
        ResearchInputRow {
            country,
            index,
            progress_raw: progress,
            ..Default::default()
        }
    }

    /// Odd owners 1..=79, with group sizes crossing every galloping step.
    fn grouped(progress: i64) -> Vec<ResearchInputRow> {
        const SIZES: [u32; 14] = [1, 2, 3, 4, 5, 7, 8, 9, 16, 17, 63, 64, 65, 130];
        (0..40)
            .flat_map(|slot: u32| {
                let size = SIZES[slot as usize % SIZES.len()];
                (0..size).map(move |index| input(slot * 2 + 1, index, progress))
            })
            .collect()
    }

    #[test]
    fn galloping_owner_searches_equal_whole_table_binary_searches() {
        for len in 0..70 {
            let values: Vec<usize> = (0..len).collect();
            for split in 0..=len {
                assert_eq!(gallop(&values, |&value| value < split), split);
            }
        }
        let rows = grouped(0);
        let naive = |owner: u32| {
            let start = rows.partition_point(|row| row.country < owner);
            start..start + rows[start..].partition_point(|row| row.country <= owner)
        };
        let expected: Vec<(u32, Range<usize>)> = (1..80)
            .step_by(2)
            .map(|owner| (owner, naive(owner)))
            .collect();
        assert_eq!(owner_ranges(&rows).collect::<Vec<_>>(), expected);
        // Every cursor, including one past the owner's rows, finds them.
        for owner in 0..82 {
            for from in 0..=rows.len() {
                assert_eq!(
                    owner_range_from(&rows, from, owner),
                    naive(owner),
                    "{owner} {from}"
                );
            }
        }
    }

    /// A recorded owner update whose listed owners read their rows again unchanged writes
    /// nothing, needs no room and records nothing.
    #[test]
    fn a_recorded_reread_of_unchanged_owners_grows_nothing() {
        let previous = TableRows::ResearchInputs(grouped(0));
        let fresh = TableRows::ResearchInputs(
            grouped(0)
                .into_iter()
                .filter(|row| row.country % 7 == 3)
                .collect(),
        );
        let update = SliceUpdate::new(&previous, fresh, |owner| owner % 7 == 3, |_| true).unwrap();
        assert!(!update.owners().is_empty());
        let mut output = vec![9_u8; 5];
        output.shrink_to_fit();
        let capacity = output.capacity();
        let mut changed = ChangedRows::default();
        assert!(
            !update
                .encode_changes_recorded(&previous, &mut output, &mut Vec::new(), &mut changed)
                .unwrap()
        );
        assert_eq!(output.capacity(), capacity);
        assert_eq!(output, vec![9_u8; 5]);
        assert!(changed.pairs.is_empty() && changed.removed.is_empty() && !changed.lost);
    }

    #[test]
    fn updates_of_large_owner_groups_encode_and_apply_the_merged_table() {
        let previous = TableRows::ResearchInputs(grouped(0));
        type Read = fn(u32) -> bool;
        let cases: [(Vec<ResearchInputRow>, Read, Read); 4] = [
            // Same counts: rows swap in place.
            (
                grouped(1)
                    .into_iter()
                    .filter(|row| row.country % 7 == 3)
                    .collect(),
                |owner| owner % 7 == 3,
                |_| true,
            ),
            // Groups shrink, owner 40 is new, 9 returns nothing, owners ending in 5 are gone.
            (
                {
                    let mut rows: Vec<_> = grouped(2)
                        .into_iter()
                        .filter(|row| row.country % 6 == 1 && row.index % 3 != 1)
                        .chain([input(40, 0, 2), input(53, 200, 2)])
                        .collect();
                    rows.sort_by_key(ResearchInputRow::key);
                    rows
                },
                |owner| owner % 6 == 1 || owner == 40 || owner == 53 || owner == 9,
                |owner| owner % 10 != 5,
            ),
            // Nothing read, nothing gone.
            (Vec::new(), |_| false, |_| true),
            // Everything read and changed.
            (grouped(3), |_| true, |_| true),
        ];
        for (step, (fresh, read, live)) in cases.into_iter().enumerate() {
            let fresh = TableRows::ResearchInputs(fresh);
            let merged = crate::sliced::merge_slice(&previous, fresh.clone(), read, live).unwrap();
            let update = SliceUpdate::new(&previous, fresh, read, live).unwrap();
            assert!(update.fits(&previous), "{step}");
            assert_eq!(update.merged(&previous).unwrap(), merged, "{step}");
            let (mut expected, mut expected_deleted) = (Vec::new(), Vec::new());
            let changed = merged
                .encode_changes(&previous, &mut expected, &mut expected_deleted)
                .unwrap();
            let (mut actual, mut deleted) = (Vec::new(), Vec::new());
            assert_eq!(
                update
                    .encode_changes(&previous, &mut actual, &mut deleted)
                    .unwrap(),
                changed,
                "{step}"
            );
            assert_eq!(actual, expected, "{step}");
            // Recorded: the same bytes when a row changed and none otherwise, and the rows it
            // wrote and the keys it deleted, by key, those the merged table's diff records.
            let (mut recorded, mut rows) = (Vec::new(), ChangedRows::default());
            let wrote = update
                .encode_changes_recorded(&previous, &mut recorded, &mut deleted, &mut rows)
                .unwrap();
            assert_eq!(wrote, changed, "{step}");
            assert_eq!(recorded, if changed { expected } else { Vec::new() }, "{step}");
            let mut whole = ChangedRows::default();
            merged
                .encode_changes_recorded(&previous, &mut Vec::new(), &mut Vec::new(), &mut whole)
                .unwrap();
            let keys = |table: &TableRows, at: u32| table.iter().nth(at as usize).unwrap().key();
            let by_key = |table: &TableRows, written: &ChangedRows| {
                written
                    .pairs
                    .iter()
                    .map(|&(at, before)| (keys(table, at), before.map(|before| keys(&previous, before))))
                    .collect::<Vec<_>>()
            };
            assert!(!rows.lost, "{step}");
            assert_eq!(by_key(&update.rows, &rows), by_key(&merged, &whole), "{step}");
            assert_eq!(rows.removed, whole.removed, "{step}");
            let mut applied = previous.clone();
            update.apply(&mut applied).unwrap();
            assert_eq!(applied, merged, "{step}");
        }
    }

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
