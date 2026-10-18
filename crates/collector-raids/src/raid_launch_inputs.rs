//! The stored inputs of a raid offer's launch gate, which the game computes
//! per viewing country on demand (`engine_native::raid_launch_inputs`).
//! RaidAirSites is the sliced `raid_launch` family (owner: canonical slot):
//! each hour reads that hour's country slice, a country new to the census
//! and the owners of a read that was not published, and every country a
//! site list writer marked since the previous hour ([`crate::raid_changes`],
//! [`SliceSelection::mark`]), so it equals the game's at every hour. A take
//! that does not certify the previous hour's read reads every country
//! ([`SliceSelection::certify`]), and every country read and not marked
//! compares its RaidAirSites rows with its previous read
//! ([`SliceSelection::compare`]): a difference disables the marks.
//! RaidCountryInputs, AceRangeInputs, StockpileArchetypes (the essential
//! stock, capped) and CountryCommandPower, which the stability dashboard
//! also reads, are read for every country each hour in the same walk.
use crate::{
    clock::qpc,
    raid_changes::{Changes, Counts, Family},
    slicing::{Digest, SliceFingerprints, SliceSelection},
    table_capture::{Arena, CaptureRows},
    table_conversion::{table, unavailable},
};
use engine_allocation::{copy_vec, owned_vec};
use engine_native::{Memory, ccountry, raid_launch_inputs as native, walk::Outcome};
use replay_data::{
    cadence::SKIPPED_REASON, cadence::should_walk, rows::AceRangeInputRow,
    rows::CountryCommandPowerRow, rows::RaidAirSiteRow, rows::RaidCountryInputRow,
    rows::StockpileArchetypeRow, rows::TableId, rows::tables,
};
use replay_frame::frame::TableView;

pub const N: usize = 5;
pub const TABLES: [TableId; N] = [
    TableId::RaidCountryInputs,
    TableId::StockpileArchetypes,
    TableId::RaidAirSites,
    TableId::AceRangeInputs,
    TableId::CountryCommandPower,
];
/// One country pass produces all five tables.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[("raid launch inputs", 0, &[0, 1, 2, 3, 4])];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    /// RaidCountryInputs, StockpileArchetypes, RaidAirSites, AceRangeInputs.
    rows: [usize; 4],
    reservations: usize,
    /// Command power rows: one per country.
    command_power: usize,
}

impl Capacities {
    /// One launch-input row and one command-power row per country.
    pub fn raise_for_countries(&mut self, slots: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[0], rows(slots, 1));
        raise(&mut self.command_power, rows(slots, 1));
    }

    /// The live reservation words of every country
    /// ([`crate::country_scale::observe_raid_arenas`]): a country whose
    /// reservations do not fit is not read, so a full hour needs them all.
    pub fn raise_for_reservations(&mut self, words: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.reservations, rows(words, 1));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn holds_reservations(&self, words: usize) -> bool {
        self.reservations >= words
    }
}

impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let demand = native::storage_demand(memory, base, state)?;
        for (floor, count) in self.rows.iter_mut().zip([
            demand.countries,
            demand.archetypes,
            demand.sites,
            demand.aces,
        ]) {
            crate::country_scale::raise(floor, count);
        }
        crate::country_scale::raise(&mut self.command_power, demand.countries);
        crate::country_scale::raise(&mut self.reservations, demand.reservations);
        Ok(())
    }
}

pub struct Captured {
    countries: CaptureRows<native::CountryRow>,
    archetypes: CaptureRows<native::ArchetypeRow>,
    sites: CaptureRows<native::SiteRow>,
    aces: CaptureRows<native::AceRow>,
    reservations: Arena<u32>,
    /// Every country's command power, each hour.
    command_power: CaptureRows<native::CommandPowerRow>,
    /// The canonical slots this hour's sliced rows cover.
    selection: SliceSelection,
    /// The raid types' essential archetypes, read each walk; the storage is
    /// kept.
    essentials: Vec<native::Essential>,
    /// Each read country's RaidAirSites digest, ascending.
    hashes: Vec<(u32, u64)>,
    /// Modifier metadata, read once per walk; its storage is kept.
    memo: engine_native::modifier_definition::DefinitionMemo,
}

/// The family's owner store and the site list writers' marks.
pub struct Owners<'a> {
    pub store: &'a mut SliceFingerprints,
    pub changes: &'a dyn Changes,
}

impl Captured {
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        let outcomes = TABLES.map(|table| super::failed_capture(table, hour, force_full, reason));
        self.countries.fail(outcomes[0]);
        self.command_power.fail(outcomes[4]);
        self.archetypes.fail(outcomes[1]);
        self.sites.fail(outcomes[2]);
        self.aces.fail(outcomes[3]);
        self.selection.reset(hour, force_full);
    }
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: [0; 4],
            reservations: 0,
            command_power: 0,
        })
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            countries: CaptureRows::new(capacity.rows[0]),
            archetypes: CaptureRows::new(capacity.rows[1]),
            sites: CaptureRows::new(capacity.rows[2]),
            aces: CaptureRows::new(capacity.rows[3]),
            reservations: Arena::new(capacity.reservations),
            command_power: CaptureRows::new(capacity.command_power),
            selection: SliceSelection::new(capacity.rows[0] != 0),
            hashes: Vec::new(),
            essentials: Vec::new(),
            memo: Default::default(),
        }
    }
    pub fn reset(&mut self) {
        self.countries.reset();
        self.archetypes.reset();
        self.sites.reset();
        self.aces.reset();
        self.reservations.reset();
        self.command_power.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.countries.allocated_bytes()
            + self.archetypes.allocated_bytes()
            + self.sites.allocated_bytes()
            + self.aces.allocated_bytes()
            + self.reservations.allocated_bytes()
            + self.command_power.allocated_bytes()
            + self.hashes.capacity() * size_of::<(u32, u64)>()
            + self.essentials.capacity() * size_of::<native::Essential>()
            + self.memo.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.countries.grow(&mut capacity.rows[0]);
        self.command_power.grow(&mut capacity.command_power);
        self.archetypes.grow(&mut capacity.rows[1]);
        self.sites.grow(&mut capacity.rows[2]);
        self.aces.grow(&mut capacity.rows[3]);
        self.reservations.grow(&mut capacity.reservations);
    }
    /// Start a frame's selection at its hour; the capture resets it again.
    pub fn reset_selection(&mut self, hour: i32, force_full: bool) {
        self.selection.reset(hour, force_full);
    }

    /// The canonical slots whose sliced rows this frame's read replaced.
    pub fn selection(&self) -> &SliceSelection {
        &self.selection
    }

    /// Fixture read of every country without the family's owner store.
    #[cfg(any(test, feature = "test-support"))]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.capture_sliced(memory, base, game_state, hour, force_full, None)
    }

    /// Scan the country census into the store with a constant print (a new
    /// country, or the owner of an unpublished read, joins this hour's read)
    /// and add every country a site list writer marked, then read the sliced
    /// tables for this hour's slice and those countries, and the country
    /// inputs, aces and command power for every country.
    pub fn capture_sliced(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        mut owners: Option<Owners<'_>>,
    ) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        self.reset();
        self.selection.reset(hour, force_full || owners.is_none());
        if !should_walk(TABLES[0], hour, force_full) {
            let skipped = Outcome::Unavailable {
                reason: SKIPPED_REASON,
            };
            self.countries.outcome = skipped;
            self.archetypes.outcome = skipped;
            self.sites.outcome = skipped;
            self.aces.outcome = skipped;
            self.command_power.outcome = skipped;
            return [0; N];
        }
        let started = qpc();
        // The link ids a complete census saw, sizing the site mark set.
        let mut links = None;
        if let Some(owners) = owners.as_mut() {
            let store = &mut *owners.store;
            let scan = qpc();
            let reader = ccountry::Reader::new(memory, base, game_state).ok();
            let census = match reader.as_ref() {
                None => false,
                Some(countries) => {
                    let mut complete = true;
                    for slot in 1..countries.count() {
                        match countries.at(slot) {
                            Ok(Some(_)) => self.selection.scan(store, slot, Some(0)),
                            Ok(None) => {}
                            Err(_) => complete = false,
                        }
                    }
                    complete
                }
            };
            store.finish_scan();
            // Without a complete census every owner is read this hour.
            self.selection.finish_pass(store, census);
            if census {
                links = reader
                    .as_ref()
                    .map(|countries| countries.arrays()[1].count as usize);
            }
            // Every country a site list writer marked joins this hour's
            // read, by its link's canonical slot; a take that does not
            // certify the previous hour's read, or an unresolved link, reads
            // every country.
            let selection = &mut self.selection;
            let mut resolved = true;
            let epoch = owners.changes.take(Family::Sites, &mut |raw| match reader
                .as_ref()
                .map(|reader| reader.resolve(raw as u32))
            {
                Some(Ok(country)) => selection.mark(country.canonical_slot),
                _ => resolved = false,
            });
            selection.certify(
                store,
                epoch.filter(|_| resolved),
                crate::raid_changes::VERIFY,
            );
            self.selection.scan_ticks = qpc().saturating_sub(scan).max(0) as u64;
        }
        // The launch keys are fixed built-ins, checked by the fixed layout
        // contract, which withholds these tables on a mismatch.
        let keys = native::Keys::FIXED;
        let selection = &self.selection;
        let walk = native::walk(
            memory,
            base,
            game_state,
            native::Buffers {
                memo: &self.memo,
                keys: &keys,
                essentials: &mut self.essentials,
                countries: engine_native::walk::Buffer::Growing(&mut self.countries.rows),
                command_power: engine_native::walk::Buffer::Growing(&mut self.command_power.rows),
                reservations: engine_native::walk::Buffer::Growing(&mut self.reservations.first),
                archetypes: engine_native::walk::Buffer::Growing(&mut self.archetypes.rows),
                sites: engine_native::walk::Buffer::Growing(&mut self.sites.rows),
                aces: engine_native::walk::Buffer::Growing(&mut self.aces.rows),
            },
            &|slot| selection.contains(slot),
        );
        self.countries.outcome = walk.countries;
        self.command_power.outcome = walk.command_power;
        self.archetypes.outcome = walk.archetypes;
        self.sites.outcome = walk.sites;
        self.aces.outcome = walk.aces;
        self.reservations.len = walk.reservations_len;
        self.reservations.required = walk.required_reservations;
        if let Some(owners) = owners {
            self.compare_sites(owners, links);
        }
        let mut ticks = [0; N];
        ticks[0] = qpc().saturating_sub(started).max(0) as u64;
        ticks
    }

    /// Compare every country this hour read and no site list writer marked
    /// with its previous read, disable the marks at a difference, and record
    /// the hour's counts.
    fn compare_sites(&mut self, owners: Owners<'_>, links: Option<usize>) {
        let complete = matches!(self.sites.outcome, Outcome::Rows { .. }) && self.site_hashes();
        let (verified, mismatches) = self
            .selection
            .compare(owners.store, complete.then_some(&self.hashes[..]));
        if mismatches != 0 {
            owners.changes.mismatch(Family::Sites);
        }
        crate::raid_changes::record(
            Family::Sites,
            &Counts {
                marked: u64::from(self.selection.marked_count),
                read: self.selection.read_owners().count() as u64,
                early: u64::from(self.selection.marked_early()),
                full: self.selection.force_full,
                verified,
                mismatches,
            },
            links,
        );
    }

    /// The RaidAirSites digest of every country this hour read, rows or
    /// none, ascending; `false` when they do not fit.
    fn site_hashes(&mut self) -> bool {
        self.hashes.clear();
        let rows = &self.sites.rows[..self.sites.outcome.len()];
        let mut at = 0;
        for owner in self.selection.read_owners() {
            while rows.get(at).is_some_and(|row| row.canonical_slot < owner) {
                at += 1;
            }
            let mut digest = Digest::new();
            while let Some(row) = rows.get(at).filter(|row| row.canonical_slot == owner) {
                digest
                    .u64(u64::from(row.ordinal))
                    .u64(u64::from(row.list))
                    .u64(row.base)
                    .u64(u64::from(row.container_capacity_raw.is_some()))
                    .i64(row.container_capacity_raw.unwrap_or(0));
                at += 1;
            }
            if engine_allocation::reserve(&mut self.hashes, 1).is_err() {
                return false;
            }
            self.hashes.push((owner, digest.finish()));
        }
        true
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.countries.outcome,
            self.archetypes.outcome,
            self.sites.outcome,
            self.aces.outcome,
            self.command_power.outcome,
        ]
    }
    pub fn tables(&self) -> [TableView; N] {
        [
            self.countries_table(),
            table::<tables::StockpileArchetypes, _>(
                self.archetypes.outcome,
                &self.archetypes.rows,
                |row| StockpileArchetypeRow {
                    canonical_slot: row.canonical_slot,
                    archetype: row.archetype,
                    units: row.units,
                },
            ),
            table::<tables::RaidAirSites, _>(self.sites.outcome, &self.sites.rows, |row| {
                RaidAirSiteRow {
                    canonical_slot: row.canonical_slot,
                    ordinal: row.ordinal,
                    list: row.list,
                    base: row.base,
                    container_capacity_raw: row.container_capacity_raw,
                }
            }),
            table::<tables::AceRangeInputs, _>(self.aces.outcome, &self.aces.rows, |row| {
                AceRangeInputRow {
                    canonical_slot: row.canonical_slot,
                    ace: row.ace,
                    wing: row.wing,
                    alive: row.alive,
                    range: row.range,
                    type_range: row.type_range,
                }
            }),
            table::<tables::CountryCommandPower, _>(
                self.command_power.outcome,
                &self.command_power.rows,
                |row| CountryCommandPowerRow {
                    canonical_slot: row.canonical_slot,
                    allocated_command_power_raw: row.allocated_raw,
                    max_command_power: row.maximum,
                    max_command_power_factor: row.maximum_factor,
                    ace_effectiveness_factor: row.ace_effectiveness,
                    air_ace_bonuses_factor: row.ace_bonuses,
                },
            ),
        ]
    }

    fn countries_table(&self) -> TableView {
        let Some(pairs) = self.reservations.first.get(..self.reservations.len) else {
            return unavailable("convoy reservation arena length invalid");
        };
        let Some(rows) = self.countries.rows.get(..self.countries.outcome.len()) else {
            return unavailable("raid country input row count invalid");
        };
        let span = |row: &native::CountryRow| {
            row.reservations_start
                .checked_add(row.reservations_len)
                .and_then(|end| pairs.get(row.reservations_start..end))
        };
        if rows.iter().any(|row| span(row).is_none()) {
            return unavailable("convoy reservations outside captured arena");
        }
        table::<tables::RaidCountryInputs, _>(self.countries.outcome, rows, |row| {
            RaidCountryInputRow {
                canonical_slot: row.canonical_slot,
                air_range_factor: row.modifiers[0],
                naval_commando_range_factor: row.modifiers[1],
                autonomous: row.autonomous,
                convoy_reservations: owned_vec(copy_vec(span(row).unwrap())),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The walk's by-name modifier values map to their columns, an
    /// unregistered name stays `None`, and command power with the ace country
    /// modifiers is its own table: the hourly columns are not repeated in
    /// RaidCountryInputs.
    #[test]
    fn the_country_conversion_maps_the_named_modifiers_and_the_allocation() {
        let mut captured = Captured::with_capacities(&Capacities {
            rows: [1, 0, 0, 0],
            reservations: 2,
            command_power: 1,
        });
        captured.countries.rows[0] = native::CountryRow {
            canonical_slot: 3,
            modifiers: [Some(1), Some(2), Some(3), None, Some(5), Some(6)],
            autonomous: Some(1),
            reservations_start: 0,
            reservations_len: 2,
        };
        captured.countries.outcome = Outcome::Rows { len: 1 };
        captured.command_power.rows[0] = native::CommandPowerRow {
            canonical_slot: 3,
            allocated_raw: 7,
            maximum: None,
            maximum_factor: Some(5),
            ace_effectiveness: Some(6),
            ace_bonuses: Some(3),
        };
        captured.command_power.outcome = Outcome::Rows { len: 1 };
        captured.reservations.first[..2].copy_from_slice(&[5, 6]);
        captured.reservations.len = 2;
        let TableView::Complete(rows) = captured.countries_table() else {
            panic!("country inputs converted")
        };
        assert_eq!(
            rows,
            replay_data::rows::TableRows::RaidCountryInputs(vec![RaidCountryInputRow {
                canonical_slot: 3,
                air_range_factor: Some(1),
                naval_commando_range_factor: Some(2),
                autonomous: Some(1),
                convoy_reservations: vec![5, 6],
            }])
        );
        let [.., TableView::Complete(power)] = captured.tables() else {
            panic!("command power converted")
        };
        assert_eq!(
            power,
            replay_data::rows::TableRows::CountryCommandPower(vec![CountryCommandPowerRow {
                canonical_slot: 3,
                allocated_command_power_raw: 7,
                max_command_power: None,
                max_command_power_factor: Some(5),
                ace_effectiveness_factor: Some(6),
                air_ace_bonuses_factor: Some(3),
            }])
        );
    }

    /// A daily slice with a constant print: an unchanged country waits for
    /// its slice, a country new to the census is read at once, and the
    /// owners of a read that was not published are read at the next hour.
    #[test]
    fn launch_owners_are_read_daily_and_new_or_unpublished_ones_at_once() {
        let mut store = SliceFingerprints::new();
        let mut selection = SliceSelection::new(true);
        let scan = |selection: &mut SliceSelection, store: &mut SliceFingerprints, last: u32| {
            for slot in 1..=last {
                selection.scan(store, slot, Some(0));
            }
            store.finish_scan();
            selection.finish_census(store);
        };
        selection.reset(0, true);
        scan(&mut selection, &mut store, 48);
        selection.reset(5, false);
        scan(&mut selection, &mut store, 48);
        assert_eq!(selection.early_count, 0);
        assert!(selection.contains(5) && !selection.contains(6));
        // A new country joins the hour's read.
        selection.reset(6, false);
        scan(&mut selection, &mut store, 49);
        assert!(selection.contains(49) && !selection.contains(5));
        // Hour 6's read was not published: its slice is read again.
        selection.invalidate(&mut store);
        selection.reset(7, false);
        scan(&mut selection, &mut store, 49);
        assert!(selection.contains(6) && selection.contains(49) && selection.contains(7));
        selection.reset(8, false);
        scan(&mut selection, &mut store, 49);
        assert_eq!(selection.early_count, 0);
    }

    /// No writer runs; a mismatch is reported.
    struct Quiet(std::sync::atomic::AtomicBool);
    impl Changes for Quiet {
        fn take(&self, _: Family, _: &mut dyn FnMut(usize)) -> Option<u64> {
            Some(1)
        }
        fn mismatch(&self, family: Family) {
            assert_eq!(family, Family::Sites);
            self.0.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Each read country's RaidAirSites rows are compared with its previous
    /// read in the same epoch: a site that changed without a mark, or a
    /// capacity, is a write no hook saw and disables the marks; an unchanged
    /// country, or one with no sites, compares equal.
    #[test]
    fn an_unmarked_country_whose_sites_changed_disables_the_marks() {
        let mut captured = Captured::with_capacities(&Capacities {
            rows: [0, 0, 2, 0],
            reservations: 0,
            command_power: 0,
        });
        let mut store = SliceFingerprints::new();
        let site = |slot, base, capacity| native::SiteRow {
            canonical_slot: slot,
            ordinal: 0,
            list: 0,
            base,
            container_capacity_raw: capacity,
        };
        let changes = Quiet(std::sync::atomic::AtomicBool::new(false));
        let mut hour = |captured: &mut Captured, rows: &[native::SiteRow]| {
            captured.selection = SliceSelection::for_test(0, true, &[]).with_census(&[3, 5, 7]);
            captured.selection.certify(&mut store, Some(1), false);
            captured.sites.rows[..rows.len()].copy_from_slice(rows);
            captured.sites.outcome = Outcome::Rows { len: rows.len() };
            captured.compare_sites(
                Owners {
                    store: &mut store,
                    changes: &changes,
                },
                None,
            );
            changes.0.load(std::sync::atomic::Ordering::Relaxed)
        };
        assert!(!hour(
            &mut captured,
            &[site(3, 0x41, Some(100_000)), site(5, 0x42, None)]
        ));
        assert!(!hour(
            &mut captured,
            &[site(3, 0x41, Some(100_000)), site(5, 0x42, None)]
        ));
        assert!(hour(
            &mut captured,
            &[site(3, 0x41, Some(0)), site(5, 0x42, None)]
        ));
    }
}
