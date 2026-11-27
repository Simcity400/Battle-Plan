
use super::SlicedRow;
use crate::{codec::invalid_static as invalid, rows::{TableId, TableRows}};
#[cfg(any(test, feature = "capture", feature = "test-support"))]
use crate::{codec::put_varint, rows::{ChangedRows, count_header}};
use engine_allocation as allocation;
use std::{io, ops::Range};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SliceUpdate {
    pub ranges: Vec<Range<u128>>,
    #[doc(hidden)]
    pub owners: Vec<u32>,
    pub keys: Vec<u128>,
    #[doc(hidden)]
    pub kept_masks: Vec<u64>,
    #[doc(hidden)]
    pub rows: TableRows,
}

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
                let $wrap = TableRows::$variant;
                $body
            })+
            _ => $other,
        }
    };
}

macro_rules! keyed_pair {
    ($left:expr, $right:expr, |$a:ident, $b:ident, $wrap:ident| $body:expr, $other:expr) => {
        sliced_pair!(@list ($left, $right), $a, $b, $wrap, $body, $other;
            Divisions, DivisionStatus, DivisionMovement, DivisionPanelStats, DivisionLandInputs,
            DivisionRetreatBans, MovementStateTerms, CoreSupplyAccess, CountryUnitSpeedEntries,
            RiverRelations, DivisionTransportSpeeds, WeatherProvinces, DivisionTemplateIcons,
            AirBaseRegistrations, CharacterRoleTraits, ScriptVariables, ScriptVariableNames, ScriptVariableValues,
            RailCooldowns, IntelContributions, IntelSources, ShipModels, CountryCounterInputs,
            ArmyOrderPlans, OrdersGroupPlans, Fronts, InvasionPreparations,
            LandCombatDivisions, LandCombatSides, LandCombatModifiers, ReserveModifierTrees,
            ProvinceUnitOrder, Ships, OrdersGroupMembers, ArmyUpgradeRequests,
            AirBases, WingEquipment, AirWings, ProductionLines, Stockpiles, DivisionEquipment,
            Fleets, TaskForces, UnitLeaders, PoliticalParties, ActiveIdeas, Politics,
            ResearchSlots, DecisionCounts, Decisions, DynamicModifiers, PreferredTactics,
            NavalCombatShips, OrdersGroupCounterInputs, CountryAppearance,
            TaskForceCounterInputs, ProvinceBuildings, StateBuildings, BuildingConstruction,
            StateResistance, CharacterPortraits)
    };
}

pub fn takes_keyed_updates(table: TableId) -> bool {
    matches!(
        table,
        TableId::Divisions
            | TableId::DivisionStatus
            | TableId::DivisionMovement
            | TableId::DivisionPanelStats
            | TableId::DivisionLandInputs
            | TableId::DivisionRetreatBans
            | TableId::MovementStateTerms
            | TableId::CoreSupplyAccess
            | TableId::CountryUnitSpeedEntries
            | TableId::RiverRelations
            | TableId::DivisionTransportSpeeds
            | TableId::WeatherProvinces
            | TableId::DivisionTemplateIcons
            | TableId::AirBaseRegistrations
            | TableId::CharacterRoleTraits
            | TableId::ScriptVariables
            | TableId::ScriptVariableNames
            | TableId::ScriptVariableValues
            | TableId::RailCooldowns
            | TableId::IntelContributions
            | TableId::IntelSources
            | TableId::ShipModels
            | TableId::CountryCounterInputs
            | TableId::ArmyOrderPlans
            | TableId::OrdersGroupPlans
            | TableId::Fronts
            | TableId::InvasionPreparations
            | TableId::LandCombatDivisions
            | TableId::LandCombatSides
            | TableId::LandCombatModifiers
            | TableId::ReserveModifierTrees
            | TableId::ProvinceUnitOrder
            | TableId::Ships
            | TableId::OrdersGroupMembers
            | TableId::ArmyUpgradeRequests
            | TableId::AirBases
            | TableId::WingEquipment
            | TableId::AirWings
            | TableId::ProductionLines
            | TableId::Stockpiles
            | TableId::DivisionEquipment
            | TableId::Fleets
            | TableId::TaskForces
            | TableId::UnitLeaders
            | TableId::PoliticalParties
            | TableId::ActiveIdeas
            | TableId::Politics
            | TableId::ResearchSlots
            | TableId::DecisionCounts
            | TableId::Decisions
            | TableId::DynamicModifiers
            | TableId::PreferredTactics
            | TableId::NavalCombatShips
            | TableId::OrdersGroupCounterInputs
            | TableId::CountryAppearance
            | TableId::TaskForceCounterInputs
            | TableId::ProvinceBuildings
            | TableId::StateBuildings
            | TableId::BuildingConstruction
            | TableId::StateResistance
            | TableId::CharacterPortraits
    )
}

fn gallop<R>(rows: &[R], holds: impl Fn(&R) -> bool) -> usize {
    let mut low = 0;
    let mut high = 1;
    while high <= rows.len() && holds(&rows[high - 1]) {
        low = high;
        high = high.saturating_mul(2);
    }
    low + rows[low..high.min(rows.len())].partition_point(holds)
}

fn owner_range_from<R: SlicedRow>(rows: &[R], from: usize, owner: u32) -> Range<usize> {
    let from = if from > 0 && rows[from - 1].owner() >= owner {
        0
    } else {
        from
    };
    let start = from + gallop(&rows[from..], |row| row.owner() < owner);
    start..start + gallop(&rows[start..], |row| row.owner() <= owner)
}

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

fn merge_owners<R: SlicedRow>(
    previous: &[R],
    fresh: &[R],
    fresh_owners: usize,
    keep: impl Fn(u32) -> bool,
) -> io::Result<Vec<u32>> {
    let mut owners = allocation::vec(fresh_owners)?;
    let mut old = owner_ranges(previous)
        .map(|(owner, _)| owner)
        .filter(|&owner| keep(owner))
        .peekable();
    for (owner, _) in owner_ranges(fresh) {
        while let Some(kept) = old.next_if(|&kept| kept < owner) {
            allocation::reserve_push_batch(&mut owners, 1)?;
            owners.push(kept);
        }
        if old.peek() == Some(&owner) {
            old.next();
        }
        allocation::reserve_push_batch(&mut owners, 1)?;
        owners.push(owner);
    }
    for kept in old {
        allocation::reserve_push_batch(&mut owners, 1)?;
        owners.push(kept);
    }
    Ok(owners)
}

fn merge_owner_lists(
    previous: &[u32],
    fresh: &[u32],
    keep: impl Fn(u32) -> bool,
) -> io::Result<Vec<u32>> {
    let mut owners = allocation::vec(fresh.len())?;
    let mut old = previous.iter().copied().filter(|&owner| keep(owner)).peekable();
    for &owner in fresh {
        while let Some(kept) = old.next_if(|&kept| kept < owner) {
            allocation::reserve_push_batch(&mut owners, 1)?;
            owners.push(kept);
        }
        if old.peek() == Some(&owner) {
            old.next();
        }
        allocation::reserve_push_batch(&mut owners, 1)?;
        owners.push(owner);
    }
    for kept in old {
        allocation::reserve_push_batch(&mut owners, 1)?;
        owners.push(kept);
    }
    Ok(owners)
}

fn take_owner<R: SlicedRow>(rows: &[R], next: &mut usize, owner: u32) -> Range<usize> {
    let start = *next;
    if rows.get(start).is_some_and(|row| row.owner() == owner) {
        *next += gallop(&rows[start..], |row| row.owner() <= owner);
    }
    start..*next
}

impl SliceUpdate {
    pub fn replaces(&self, table: TableId, key: u128) -> bool {
        let owner = (key >> table.key_bits().saturating_sub(32)) as u32;
        self.owners.binary_search(&owner).is_ok()
            || self.ranges.iter().any(|range| range.contains(&key))
            || self.keys.binary_search(&key).is_ok()
    }

    pub fn keyed(keys: Vec<u128>, rows: TableRows) -> Self {
        Self {
            ranges: Vec::new(),
            owners: Vec::new(),
            keys,
            kept_masks: Vec::new(),
            rows,
        }
    }

    pub fn keyed_masked(keys: Vec<u128>, rows: TableRows, kept_masks: Vec<u64>) -> Self {
        Self {
            ranges: Vec::new(),
            owners: Vec::new(),
            keys,
            kept_masks,
            rows,
        }
    }

    pub fn ranged(ranges: Vec<Range<u128>>, rows: TableRows) -> Self {
        Self {
            ranges,
            owners: Vec::new(),
            keys: Vec::new(),
            kept_masks: Vec::new(),
            rows,
        }
    }

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
            let mut prev_key: Option<u128> = None;
            let mut last_owner: Option<u32> = None;
            let mut fresh_owners = 0_usize;
            for row in fresh {
                let key = row.key();
                if prev_key.is_some_and(|prev| prev >= key) {
                    return Err(invalid(
                        "sliced read holds an unread owner's row or is not in key order",
                    ));
                }
                prev_key = Some(key);
                let owner = row.owner();
                if last_owner != Some(owner) {
                    if !read(owner) {
                        return Err(invalid(
                            "sliced read holds an unread owner's row or is not in key order",
                        ));
                    }
                    last_owner = Some(owner);
                    fresh_owners += 1;
                }
            }
            merge_owners(previous, fresh, fresh_owners, |owner| {
                read(owner) || !live(owner)
            })
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
            kept_masks: Vec::new(),
            rows: fresh,
        })
    }

    pub fn diff_owners(
        previous: &TableRows,
        fresh: TableRows,
        read: impl Fn(u32) -> bool,
        live: impl Fn(u32) -> bool,
    ) -> io::Result<Self> {
        fn diff<R: SlicedRow>(
            previous: &[R],
            fresh: Vec<R>,
            read: impl Fn(u32) -> bool,
            live: impl Fn(u32) -> bool,
        ) -> io::Result<(Vec<u128>, Vec<R>)> {
            let mut prev_key: Option<u128> = None;
            let mut last_owner: Option<u32> = None;
            let mut fresh_owners = 0_usize;
            for row in &fresh {
                let key = row.key();
                if prev_key.is_some_and(|prev| prev >= key) {
                    return Err(invalid(
                        "sliced read holds an unread owner's row or is not in key order",
                    ));
                }
                prev_key = Some(key);
                let owner = row.owner();
                if last_owner != Some(owner) {
                    if !read(owner) {
                        return Err(invalid(
                            "sliced read holds an unread owner's row or is not in key order",
                        ));
                    }
                    last_owner = Some(owner);
                    fresh_owners += 1;
                }
            }
            let owners = merge_owners(previous, &fresh, fresh_owners, |owner| {
                read(owner) || !live(owner)
            })?;
            let mut keys = Vec::new();
            let mut rows = Vec::new();
            let mut next_old = 0;
            let mut fresh_iter = fresh.into_iter().peekable();
            for owner in owners {
                let old_range = owner_range_from(previous, next_old, owner);
                next_old = old_range.end;
                let mut old_at = old_range.start;
                loop {
                    let old_row = (old_at < old_range.end).then(|| &previous[old_at]);
                    let new_key = fresh_iter
                        .peek()
                        .filter(|row| row.owner() == owner)
                        .map(R::key);
                    match (old_row, new_key) {
                        (None, None) => break,
                        (Some(old), Some(key)) if old.key() == key => {
                            let row = fresh_iter.next().unwrap();
                            if row != *old {
                                allocation::reserve_push_batch(&mut keys, 1)?;
                                allocation::reserve_push_batch(&mut rows, 1)?;
                                keys.push(key);
                                rows.push(row);
                            }
                            old_at += 1;
                        }
                        (Some(old), Some(key)) if key < old.key() => {
                            let row = fresh_iter.next().unwrap();
                            allocation::reserve_push_batch(&mut keys, 1)?;
                            allocation::reserve_push_batch(&mut rows, 1)?;
                            keys.push(key);
                            rows.push(row);
                        }
                        (None, Some(key)) => {
                            let row = fresh_iter.next().unwrap();
                            allocation::reserve_push_batch(&mut keys, 1)?;
                            allocation::reserve_push_batch(&mut rows, 1)?;
                            keys.push(key);
                            rows.push(row);
                        }
                        (Some(old), _) => {
                            allocation::reserve_push_batch(&mut keys, 1)?;
                            keys.push(old.key());
                            old_at += 1;
                        }
                    }
                }
            }
            Ok((keys, rows))
        }
        let (keys, rows) = sliced_pair!(
            previous,
            fresh,
            |old, new, wrap| {
                let (keys, changed) = diff(old, new, read, live)?;
                (keys, wrap(changed))
            },
            return Err(invalid("cannot merge different sliced table types"))
        );
        Ok(Self {
            owners: Vec::new(),
            ranges: Vec::new(),
            keys,
            kept_masks: Vec::new(),
            rows,
        })
    }

    pub fn for_owners(
        previous_owners: &[u32],
        fresh: TableRows,
        read: impl Fn(u32) -> bool,
        live: impl Fn(u32) -> bool,
    ) -> io::Result<(Self, Vec<u32>)> {
        Self::for_owners_and_carried(previous_owners, fresh, read, |_| false, live)
    }

    pub fn for_owners_and_carried(
        previous_owners: &[u32],
        fresh: TableRows,
        read: impl Fn(u32) -> bool,
        carried: impl Fn(u32) -> bool,
        live: impl Fn(u32) -> bool,
    ) -> io::Result<(Self, Vec<u32>)> {
        fn validate<R: SlicedRow>(
            fresh: &[R],
            read: impl Fn(u32) -> bool,
            is_carried: impl Fn(u32) -> bool,
        ) -> io::Result<(Vec<u32>, Vec<u32>, Vec<u128>)> {
            let mut prev_key: Option<u128> = None;
            let mut last_owner: Option<(u32, bool)> = None;
            let mut fresh_all = Vec::new();
            let mut fresh_read = Vec::new();
            let mut keys = Vec::new();
            for row in fresh {
                let key = row.key();
                if prev_key.is_some_and(|prev| prev >= key) {
                    return Err(invalid(
                        "sliced read holds an unread owner's row or is not in key order",
                    ));
                }
                prev_key = Some(key);
                let owner = row.owner();
                let carried_owner = match last_owner {
                    Some((last, flag)) if last == owner => flag,
                    _ => {
                        if !read(owner) {
                            return Err(invalid(
                                "sliced read holds an unread owner's row or is not in key order",
                            ));
                        }
                        let flag = is_carried(owner);
                        last_owner = Some((owner, flag));
                        allocation::reserve_push_batch(&mut fresh_all, 1)?;
                        fresh_all.push(owner);
                        if !flag {
                            allocation::reserve_push_batch(&mut fresh_read, 1)?;
                            fresh_read.push(owner);
                        }
                        flag
                    }
                };
                if carried_owner {
                    allocation::reserve_push_batch(&mut keys, 1)?;
                    keys.push(key);
                }
            }
            Ok((fresh_all, fresh_read, keys))
        }
        let is_carried = |owner| carried(owner) && live(owner);
        let (fresh_all, fresh_read, keys) = sliced_pair!(
            &fresh,
            &fresh,
            |rows, _other, _wrap| validate(rows, &read, &is_carried)?,
            return Err(invalid("table does not support owner slice updates"))
        );
        let replaced = merge_owner_lists(previous_owners, &fresh_read, |owner| {
            (read(owner) && !is_carried(owner)) || !live(owner)
        })?;
        let next = merge_owner_lists(previous_owners, &fresh_all, |owner| {
            (!read(owner) || is_carried(owner)) && live(owner)
        })?;
        Ok((
            Self {
                owners: replaced,
                ranges: Vec::new(),
                keys,
                kept_masks: Vec::new(),
                rows: fresh,
            },
            next,
        ))
    }

    pub fn fits_untracked(&self, table: TableId) -> bool {
        if self.rows.table() != table || !self.ranges.is_empty() || !self.keys.is_empty() {
            return false;
        }
        if self.owners.is_empty() && self.rows.is_empty() {
            return true;
        }
        fn fits<R: SlicedRow>(fresh: &[R], listed: &[u32]) -> bool {
            fresh.windows(2).all(|pair| pair[0].key() < pair[1].key())
                && owner_ranges(fresh).all(|(owner, _)| listed.binary_search(&owner).is_ok())
        }
        self.owners.windows(2).all(|pair| pair[0] < pair[1])
            && sliced_pair!(
                &self.rows,
                &self.rows,
                |new, _other, _wrap| fits(new, &self.owners),
                false
            )
    }

    pub fn fits_meta(&self, table: TableId) -> bool {
        if self.rows.table() != table
            || (!self.kept_masks.is_empty() && self.kept_masks.len() != self.rows.len())
        {
            return false;
        }
        if !self.ranges.is_empty() {
            if !self.owners.is_empty() || !self.keys.is_empty() {
                return false;
            }
            if !self.ranges.iter().all(|range| range.start < range.end)
                || !self
                    .ranges
                    .windows(2)
                    .all(|pair| pair[0].end <= pair[1].start)
            {
                return false;
            }
            struct Check<'a>(&'a [Range<u128>]);
            impl crate::rows::RowsVisit for Check<'_> {
                type Output = bool;
                fn rows<R: crate::rows::RowCodec>(self, rows: &[R]) -> bool {
                    let mut at = 0;
                    let mut previous = None;
                    for row in rows {
                        let key = row.key();
                        if previous.is_some_and(|prev| prev >= key) {
                            return false;
                        }
                        previous = Some(key);
                        while self.0.get(at).is_some_and(|range| range.end <= key) {
                            at += 1;
                        }
                        if !self.0.get(at).is_some_and(|range| range.contains(&key)) {
                            return false;
                        }
                    }
                    true
                }
            }
            return self.rows.visit(Check(&self.ranges));
        }
        if !self.keys.is_empty() {
            if !self.owners.is_empty() {
                return self.fits_mixed(table);
            }
            return self.keys.windows(2).all(|pair| pair[0] < pair[1])
                && keyed_pair!(
                    &self.rows,
                    &self.rows,
                    |_old, new, _wrap| key_fits(new, &self.keys),
                    false
                );
        }
        self.fits_untracked(table)
    }

    fn fits_mixed(&self, table: TableId) -> bool {
        if !self.owners.windows(2).all(|pair| pair[0] < pair[1])
            || !self.keys.windows(2).all(|pair| pair[0] < pair[1])
        {
            return false;
        }
        let shift = table.key_bits().saturating_sub(32);
        let mut owner_cursor = 0;
        let disjoint = self.keys.iter().all(|&key| {
            let owner = (key >> shift) as u32;
            owner_cursor += gallop(&self.owners[owner_cursor..], |&candidate| candidate < owner);
            self.owners.get(owner_cursor) != Some(&owner)
        });
        disjoint
            && sliced_pair!(
                &self.rows,
                &self.rows,
                |new, _other, _wrap| mixed_fits(new, &self.owners, &self.keys),
                false
            )
    }

    pub fn owners(&self) -> &[u32] {
        &self.owners
    }

    pub fn rows(&self) -> &TableRows {
        &self.rows
    }

    pub fn merged(&self, previous: &TableRows) -> io::Result<TableRows> {
        let mut rows = previous.try_clone()?;
        self.clone().apply(&mut rows)?;
        Ok(rows)
    }

    pub fn fits(&self, previous: &TableRows) -> bool {
        if !self.kept_masks.is_empty() {
            return false;
        }
        if !self.ranges.is_empty() {
            return self.owners.is_empty() && self.rows.range_update_fits(previous, &self.ranges);
        }
        if !self.keys.is_empty() {
            if !self.owners.is_empty() {
                return self.rows.table() == previous.table() && self.fits_mixed(previous.table());
            }
            return self.keys.windows(2).all(|pair| pair[0] < pair[1])
                && keyed_pair!(
                    previous,
                    &self.rows,
                    |_old, new, _wrap| key_fits(new, &self.keys),
                    false
                );
        }
        if self.owners.is_empty() && self.rows.is_empty() {
            return self.rows.table() == previous.table();
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

    #[cfg(any(test, feature = "capture", feature = "test-support"))]
    pub fn encode_changes(
        &self,
        previous: &TableRows,
        output: &mut Vec<u8>,
        deleted: &mut Vec<u8>,
    ) -> io::Result<bool> {
        self.encode_changes_with(previous, output, deleted, None)
    }

    #[cfg(any(test, feature = "capture", feature = "test-support"))]
    pub fn encode_changes_recorded(
        &self,
        previous: &TableRows,
        output: &mut Vec<u8>,
        deleted: &mut Vec<u8>,
        changed: &mut ChangedRows,
    ) -> io::Result<bool> {
        self.encode_changes_with(previous, output, deleted, Some(changed))
    }

    #[cfg(any(test, feature = "capture", feature = "test-support"))]
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

    pub fn apply(self, previous: &mut TableRows) -> io::Result<TableRows> {
        if !self.ranges.is_empty() {
            return self.rows.apply_range_update(previous, &self.ranges);
        }
        if !self.keys.is_empty() {
            if !self.fits(previous) {
                return Err(invalid("invalid key update"));
            }
            if !self.owners.is_empty() {
                let Self {
                    owners, keys, rows, ..
                } = self;
                return sliced_pair!(
                    &mut *previous,
                    rows,
                    |old, new, wrap| apply_mixed(old, new, &owners, &keys).map(wrap),
                    Err(invalid("slice update rows differ from the baseline type"))
                );
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
        if owners.is_empty() && rows.is_empty() && rows.table() == previous.table() {
            return Ok(TableRows::empty(previous.table()));
        }
        sliced_pair!(
            &mut *previous,
            rows,
            |old, new, wrap| apply(old, new, &owners).map(wrap),
            Err(invalid("slice update rows differ from the baseline type"))
        )
    }
}

fn mixed_fits<R: SlicedRow>(fresh: &[R], listed: &[u32], keys: &[u128]) -> bool {
    let mut prev: Option<u128> = None;
    let mut owner_cursor = 0;
    let mut key_cursor = 0;
    for row in fresh {
        let key = row.key();
        if prev.is_some_and(|before| before >= key) {
            return false;
        }
        prev = Some(key);
        let owner = row.owner();
        owner_cursor += gallop(&listed[owner_cursor..], |&candidate| candidate < owner);
        if listed.get(owner_cursor) == Some(&owner) {
            continue;
        }
        key_cursor += gallop(&keys[key_cursor..], |&candidate| candidate < key);
        if keys.get(key_cursor) != Some(&key) {
            return false;
        }
        key_cursor += 1;
    }
    true
}

fn apply_mixed<R: SlicedRow>(
    old: &mut Vec<R>,
    fresh: Vec<R>,
    owners: &[u32],
    keys: &[u128],
) -> io::Result<Vec<R>> {
    let mut removed = 0_usize;
    let mut owner_cursor = 0;
    let mut key_cursor = 0;
    for row in old.iter() {
        let owner = row.owner();
        while owners
            .get(owner_cursor)
            .is_some_and(|&candidate| candidate < owner)
        {
            owner_cursor += 1;
        }
        if owners.get(owner_cursor) == Some(&owner) {
            removed += 1;
            continue;
        }
        let key = row.key();
        while keys
            .get(key_cursor)
            .is_some_and(|&candidate| candidate < key)
        {
            key_cursor += 1;
        }
        if keys.get(key_cursor) == Some(&key) {
            removed += 1;
        }
    }
    let mut rebuilt = allocation::vec(allocation::add(old.len() - removed, fresh.len())?)?;
    let mut replaced = allocation::vec(removed)?;
    let mut fresh = fresh.into_iter().peekable();
    let mut owner_cursor = 0;
    let mut key_cursor = 0;
    for row in std::mem::take(old) {
        let owner = row.owner();
        while owners
            .get(owner_cursor)
            .is_some_and(|&candidate| candidate < owner)
        {
            owner_cursor += 1;
        }
        let key = row.key();
        while keys
            .get(key_cursor)
            .is_some_and(|&candidate| candidate < key)
        {
            key_cursor += 1;
        }
        if owners.get(owner_cursor) == Some(&owner) || keys.get(key_cursor) == Some(&key) {
            replaced.push(row);
            continue;
        }
        while let Some(new) = fresh.next_if(|new| new.key() < key) {
            rebuilt.push(new);
        }
        rebuilt.push(row);
    }
    rebuilt.extend(fresh);
    *old = rebuilt;
    Ok(replaced)
}

fn key_fits<R: crate::rows::RowCodec>(fresh: &[R], keys: &[u128]) -> bool {
    let mut prev: Option<u128> = None;
    let mut cursor = 0;
    for row in fresh {
        let key = row.key();
        if prev.is_some_and(|before| before >= key) {
            return false;
        }
        prev = Some(key);
        cursor += gallop(&keys[cursor..], |&candidate| candidate < key);
        if keys.get(cursor) != Some(&key) {
            return false;
        }
        cursor += 1;
    }
    true
}
#[cfg(any(test, feature = "capture", feature = "test-support"))]
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
    let mut old_cursor = 0;
    let mut fresh_cursor = 0;
    for &key in keys {
        old_cursor += gallop(&old[old_cursor..], |row| row.key() < key);
        let before = old
            .get(old_cursor)
            .filter(|row| row.key() == key)
            .map(|_| old_cursor);
        let previous = before.map(|i| &old[i]);
        let at = fresh
            .get(fresh_cursor)
            .filter(|row| row.key() == key)
            .map(|_| {
                let at = fresh_cursor;
                fresh_cursor += 1;
                at
            });
        if let Some(at) = at {
            if fresh[at].encode_change(previous, changed_key, output)? {
                changed = changed
                    .checked_add(1)
                    .ok_or(invalid("too many changed rows"))?;
                changed_key = key;
                if let Some(recorded) = recorded.as_deref_mut() {
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
    let mut old_cursor = 0;
    let mut fresh_cursor = 0;
    for &key in keys {
        old_cursor += gallop(&old[old_cursor..], |row| row.key() < key);
        let present = old.get(old_cursor).is_some_and(|row| row.key() == key);
        removed += usize::from(present);
        let in_fresh = fresh
            .get(fresh_cursor)
            .is_some_and(|row| row.key() == key);
        if in_fresh {
            fresh_cursor += 1;
        }
        same_keys &= present == in_fresh;
    }
    if same_keys {
        let mut cursor = 0;
        for row in &mut fresh {
            let key = row.key();
            cursor += gallop(&old[cursor..], |existing| existing.key() < key);
            if old.get(cursor).is_none_or(|existing| existing.key() != key) {
                return Err(invalid("key update lost its baseline"));
            }
            std::mem::swap(row, &mut old[cursor]);
            cursor += 1;
        }
        return Ok(fresh);
    }
    let mut rebuilt = allocation::vec(allocation::add(old.len() - removed, fresh.len())?)?;
    let mut replaced = allocation::vec(removed)?;
    let mut fresh = fresh.into_iter().peekable();
    let mut key_cursor = 0;
    for row in std::mem::take(old) {
        let row_key = row.key();
        while keys.get(key_cursor).is_some_and(|&key| key < row_key) {
            key_cursor += 1;
        }
        if keys.get(key_cursor) == Some(&row_key) {
            replaced.push(row);
            key_cursor += 1;
            continue;
        }
        while let Some(new) = fresh.next_if(|new| new.key() < row_key) {
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
    use crate::rows::{LendLeaseAgreementRow, ResearchInputRow};

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
                            kept_masks: Vec::new(),
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

    fn input(country: u32, index: u32, progress: i64) -> ResearchInputRow {
        ResearchInputRow {
            country,
            index,
            progress_raw: progress,
            ..Default::default()
        }
    }

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
            (
                grouped(1)
                    .into_iter()
                    .filter(|row| row.country % 7 == 3)
                    .collect(),
                |owner| owner % 7 == 3,
                |_| true,
            ),
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
            (Vec::new(), |_| false, |_| true),
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
    fn diff_owners_emits_only_changed_keys_and_matches_full_merge() {
        let row = |country: u32, id: u64, fuel: i64| LendLeaseAgreementRow {
            country,
            id,
            fuel_sent_raw: fuel,
            ..Default::default()
        };
        let previous = TableRows::LendLeaseAgreements(vec![
            row(1, 1, 10),
            row(1, 2, 20),
            row(2, 1, 30),
            row(3, 1, 40),
        ]);
        let fresh = TableRows::LendLeaseAgreements(vec![
            row(1, 1, 10),
            row(1, 2, 25),
            row(1, 3, 99),
        ]);
        let read = |owner| owner == 1;
        let live = |owner| owner != 3;
        let full = SliceUpdate::new(&previous, fresh.clone(), read, live).unwrap();
        let diff = SliceUpdate::diff_owners(&previous, fresh, read, live).unwrap();
        assert!(diff.owners.is_empty());
        assert!(diff.fits(&previous));
        assert_eq!(diff.rows.len(), 2);
        assert_eq!(diff.keys.len(), 3);
        assert_eq!(
            diff.merged(&previous).unwrap(),
            full.merged(&previous).unwrap()
        );
        let unchanged_fresh = TableRows::LendLeaseAgreements(vec![row(2, 1, 30)]);
        let empty = SliceUpdate::diff_owners(&previous, unchanged_fresh, |o| o == 2, |_| true)
            .unwrap();
        assert!(empty.keys.is_empty());
        assert!(empty.rows.is_empty());
        assert!(empty.fits(&previous));
        assert_eq!(empty.merged(&previous).unwrap(), previous);
    }

    #[test]
    fn for_owners_and_carried_updates_read_owners_and_carried_keys_disjointly() {
        let previous = TableRows::ResearchInputs(vec![
            input(1, 0, 10),
            input(1, 1, 20),
            input(2, 0, 30),
            input(2, 1, 40),
            input(3, 0, 50),
            input(4, 0, 60),
        ]);
        let fresh = TableRows::ResearchInputs(vec![
            input(1, 1, 25),
            input(2, 0, 35),
            input(2, 2, 45),
        ]);
        let (update, next_owners) = SliceUpdate::for_owners_and_carried(
            &[1, 2, 3, 4],
            fresh,
            |owner| owner == 1 || owner == 2,
            |owner| owner == 1,
            |owner| owner != 4,
        )
        .unwrap();
        assert_eq!(update.owners, vec![2, 4]);
        assert_eq!(update.keys, vec![input(1, 1, 25).key()]);
        assert_eq!(next_owners, vec![1, 2, 3]);
        assert!(update.fits_meta(TableId::ResearchInputs));
        assert!(update.fits(&previous));
        assert_eq!(
            update.merged(&previous).unwrap(),
            TableRows::ResearchInputs(vec![
                input(1, 0, 10),
                input(1, 1, 25),
                input(2, 0, 35),
                input(2, 2, 45),
                input(3, 0, 50),
            ])
        );
        let overlapping = SliceUpdate {
            owners: vec![1, 2],
            keys: vec![input(1, 1, 25).key()],
            ranges: Vec::new(),
            kept_masks: Vec::new(),
            rows: TableRows::ResearchInputs(vec![input(1, 1, 25)]),
        };
        assert!(!overlapping.fits_meta(TableId::ResearchInputs));
        assert!(!overlapping.fits(&previous));
    }
}
