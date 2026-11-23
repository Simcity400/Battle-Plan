use crate::{
    clock::qpc,
    raid_changes::{Changes, Counts, Family},
    slicing::{SliceFingerprints, SliceSelection},
    table_capture::{Arena, CaptureRows},
    table_conversion::{table, unavailable},
};
use engine_allocation::{copy_vec, owned_vec};
use engine_native::{Memory, ccountry, walk::Outcome};
use engine_native_air::raid_launch_inputs as native;
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

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: [usize; 4],
    reservations: usize,
    command_power: usize,
}

collector_primitives::capacity_max!(Capacities {
    rows,
    reservations,
    command_power
});

impl Capacities {
    pub fn raise_for_countries(&mut self, slots: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[0], rows(slots, 1));
        raise(&mut self.command_power, rows(slots, 1));
    }

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
    command_power: CaptureRows<native::CommandPowerRow>,
    selection: SliceSelection,
    essentials: Vec<native::Essential>,
    memo: engine_native::modifier_definition::DefinitionMemo,
    volunteer_air: native::VolunteerAir,
}

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
        self.selection.reset_hooked(hour, force_full);
    }
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
            selection: SliceSelection::new(),
            essentials: Vec::new(),
            memo: Default::default(),
            volunteer_air: Default::default(),
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
        self.volunteer_air.allocated_bytes()
            + self.countries.allocated_bytes()
            + self.archetypes.allocated_bytes()
            + self.sites.allocated_bytes()
            + self.aces.allocated_bytes()
            + self.reservations.allocated_bytes()
            + self.command_power.allocated_bytes()
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
    pub fn reset_selection(&mut self, hour: i32, force_full: bool) {
        self.selection.reset_hooked(hour, force_full);
    }

    pub fn selection(&self) -> &SliceSelection {
        &self.selection
    }

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
        self.selection
            .reset_hooked(hour, force_full || owners.is_none());
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
            self.selection.finish_pass(store, census);
            if census {
                links = reader
                    .as_ref()
                    .map(|countries| countries.arrays()[1].count as usize);
            }
            let selection = &mut self.selection;
            let mut resolved = true;
            let epoch = owners.changes.take(Family::Sites, &mut |raw| match reader
                .as_ref()
                .map(|reader| reader.resolve(raw as u32))
            {
                Some(Ok(country)) => selection.mark(country.canonical_slot),
                _ => resolved = false,
            });
            selection.certify(store, epoch.filter(|_| resolved));
            self.selection.scan_ticks = qpc().saturating_sub(scan).max(0) as u64;
        }
        let keys = native::Keys::FIXED;
        let selection = &self.selection;
        use collector_model_inputs::local_subunit_changes::{Changes as _, Observer};
        let certificate = collector_lifetime::world_lifetime::token().and_then(|world| {
            Observer
                .quiet()
                .map(|(epoch, revision)| (world, epoch, revision))
        });
        self.volunteer_air.prepare(certificate);
        let walk = native::walk_with_volunteer_air(
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
            Some(&mut self.volunteer_air),
        );
        let after = collector_lifetime::world_lifetime::token().and_then(|world| {
            Observer
                .quiet()
                .map(|(epoch, revision)| (world, epoch, revision))
        });
        if certificate.is_none() || certificate != after {
            for row in &mut self.countries.rows[..walk.countries.len()] {
                row.volunteer_stockpile_planes = None;
            }
            self.volunteer_air.prepare(None);
        }
        self.countries.outcome = walk.countries;
        self.command_power.outcome = walk.command_power;
        self.archetypes.outcome = walk.archetypes;
        self.sites.outcome = walk.sites;
        self.aces.outcome = walk.aces;
        self.reservations.len = walk.reservations_len;
        self.reservations.required = walk.required_reservations;
        if owners.is_some() {
            self.record_sites(links);
        }
        let mut ticks = [0; N];
        ticks[0] = qpc().saturating_sub(started).max(0) as u64;
        ticks
    }

    fn record_sites(&self, links: Option<usize>) {
        crate::raid_changes::record(
            Family::Sites,
            &Counts {
                marked: u64::from(self.selection.marked_count),
                read: self.selection.read_owners().count() as u64,
                carried: self.selection.carried() as u64,
                early: u64::from(self.selection.marked_early()),
                full: self.selection.force_full,
            },
            links,
        );
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
                    normalization: row.normalization.into(),
                    fuel_modifier: row.fuel_modifier.into(),
                    combat_modifiers_known: (row.combat_modifiers.is_some()).into(),
                    speed_modifier_raw: (row.combat_modifiers.and_then(|values| values[0])).into(),
                    attack_modifier_raw: (row.combat_modifiers.and_then(|values| values[1])).into(),
                    defence_modifier_raw: (row.combat_modifiers.and_then(|values| values[2]))
                        .into(),
                    type_speed_modifier_raw: (row.type_speed_modifier_raw).into(),
                    type_combat_flags: (row.type_combat_flags).into(),
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
                volunteer_stockpile_planes: (row.volunteer_stockpile_planes).into(),
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
            volunteer_stockpile_planes: None,
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
                volunteer_stockpile_planes: (None).into(),
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

    #[test]
    fn launch_owners_are_read_only_when_new_marked_or_unpublished() {
        let mut store = SliceFingerprints::new();
        let mut selection = SliceSelection::new();
        let scan = |selection: &mut SliceSelection, store: &mut SliceFingerprints, last: u32| {
            for slot in 1..=last {
                selection.scan(store, slot, Some(0));
            }
            store.finish_scan();
            selection.finish_census(store);
        };
        selection.reset_hooked(0, true);
        scan(&mut selection, &mut store, 48);
        for hour in 1..=24 {
            selection.reset_hooked(hour, false);
            scan(&mut selection, &mut store, 48);
            assert!(!selection.reads_any(), "hour {hour}");
            assert_eq!(selection.carried(), 48);
        }
        selection.reset_hooked(30, false);
        scan(&mut selection, &mut store, 49);
        selection.mark(5);
        assert!(selection.contains(49) && selection.contains(5) && !selection.contains(6));
        selection.invalidate(&mut store);
        selection.reset_hooked(31, false);
        scan(&mut selection, &mut store, 49);
        assert!(selection.contains(49) && selection.contains(5) && !selection.contains(7));
        selection.reset_hooked(32, false);
        scan(&mut selection, &mut store, 49);
        assert!(!selection.reads_any());
    }
}
