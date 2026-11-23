use crate::{
    clock::qpc,
    table_capture::{Arena, CaptureRows, grow_capacity},
    table_conversion::{table, unavailable},
};
use engine_allocation::{copy_vec, map_slice, owned_vec};
use engine_deployment::{cdeployment_status, unit_bonuses};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::{cproduction_status, creferenced_division_template::Span};
use replay_data::{
    cadence::SKIPPED_REASON, cadence::should_walk, rows::AvailableEquipmentVariantRow,
    rows::CountryLocalSubunitRow, rows::CountryUnitBonusRow, rows::LocalSubunitRequirement,
    rows::TableId, rows::UnitBonusGrant, rows::UnitBonusStat, rows::UnitBonusTarget, rows::tables,
};
use replay_frame::frame::TableView;

pub const N: usize = 3;
pub const TABLES: [TableId; N] = [
    TableId::CountryLocalSubunits,
    TableId::AvailableEquipmentVariants,
    TableId::CountryUnitBonuses,
];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    local_rows: usize,
    requirements: usize,
    rosters: usize,
    bonuses: unit_bonuses::Counts,
}

collector_primitives::capacity_max!(Capacities {
    local_rows,
    requirements,
    rosters,
    bonuses { rows, targets, grants, stats, label_bytes },
});

impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        self.rosters = self.rosters.max(
            match cproduction_status::count_available_equipment(memory, base, state) {
                Outcome::Rows { len } => len,
                Outcome::Truncated { len, unread_rows } => len
                    .checked_add(unread_rows as usize)
                    .ok_or("equipment roster sizing overflow")?,
                Outcome::Unavailable { reason } => return Err(reason),
            },
        );
        let local = cdeployment_status::count_local_subunits(memory, base, state);
        if let Outcome::Unavailable { reason } = local.outcome {
            return Err(reason);
        }
        self.local_rows = self.local_rows.max(local.required_rows);
        self.requirements = self.requirements.max(local.required_requirements);
        let bonuses = unit_bonuses::walk(memory, base, state, unit_bonuses::Buffers::demand());
        if let Outcome::Unavailable { reason } = bonuses.outcome {
            return Err(reason);
        }
        let (held, required) = (&mut self.bonuses, bonuses.required);
        held.rows = held.rows.max(required.rows);
        held.targets = held.targets.max(required.targets);
        held.grants = held.grants.max(required.grants);
        held.stats = held.stats.max(required.stats);
        held.label_bytes = held.label_bytes.max(required.label_bytes);
        Ok(())
    }
}

pub struct Captured {
    local: CaptureRows<cdeployment_status::LocalSubunitRow>,
    requirements: Arena<cdeployment_status::LocalRequirement>,
    required_local_rows: usize,
    rosters: CaptureRows<cproduction_status::AvailableEquipmentRow>,
    bonuses: CaptureRows<unit_bonuses::Row>,
    bonus_targets: Arena<unit_bonuses::Target>,
    bonus_grants: Arena<unit_bonuses::Grant>,
    bonus_stats: Arena<unit_bonuses::Stat>,
    bonus_labels: Arena<u8>,
    required_bonus_rows: usize,
}

impl Captured {
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.local
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.rosters
            .fail(super::failed_capture(TABLES[1], hour, force_full, reason));
        self.bonuses
            .fail(super::failed_capture(TABLES[2], hour, force_full, reason));
    }
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities::default())
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            local: CaptureRows::new(capacity.local_rows),
            requirements: Arena::new(capacity.requirements),
            required_local_rows: 0,
            rosters: CaptureRows::new(capacity.rosters),
            bonuses: CaptureRows::new(capacity.bonuses.rows),
            bonus_targets: Arena::new(capacity.bonuses.targets),
            bonus_grants: Arena::new(capacity.bonuses.grants),
            bonus_stats: Arena::new(capacity.bonuses.stats),
            bonus_labels: Arena::new(capacity.bonuses.label_bytes),
            required_bonus_rows: 0,
        }
    }
    pub fn reset(&mut self) {
        self.local.reset();
        self.requirements.reset();
        self.required_local_rows = 0;
        self.rosters.reset();
        self.bonuses.reset();
        self.bonus_targets.reset();
        self.bonus_grants.reset();
        self.bonus_stats.reset();
        self.bonus_labels.reset();
        self.required_bonus_rows = 0;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.local.allocated_bytes()
            + self.requirements.allocated_bytes()
            + self.rosters.allocated_bytes()
            + self.bonuses.allocated_bytes()
            + self.bonus_targets.allocated_bytes()
            + self.bonus_grants.allocated_bytes()
            + self.bonus_stats.allocated_bytes()
            + self.bonus_labels.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        grow_capacity(&mut capacity.local_rows, self.required_local_rows);
        self.local.grow(&mut capacity.local_rows);
        self.requirements.grow(&mut capacity.requirements);
        self.rosters.grow(&mut capacity.rosters);
        let bonuses = &mut capacity.bonuses;
        grow_capacity(&mut bonuses.rows, self.required_bonus_rows);
        self.bonuses.grow(&mut bonuses.rows);
        self.bonus_targets.grow(&mut bonuses.targets);
        self.bonus_grants.grow(&mut bonuses.grants);
        self.bonus_stats.grow(&mut bonuses.stats);
        self.bonus_labels.grow(&mut bonuses.label_bytes);
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
        self.capture_carried(memory, base, game_state, hour, force_full, None)
    }

    pub fn capture_carried(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        store: Option<&mut LocalStore>,
    ) -> [u64; N] {
        self.reset();
        let skipped = Outcome::Unavailable {
            reason: SKIPPED_REASON,
        };
        let mut ticks = [0; N];
        if should_walk(TABLES[0], hour, force_full) {
            let _reads = crate::read_accounting::table(TABLES[0]);
            let started = qpc();
            let walk = match store {
                Some(store) if crate::local_subunit_changes::CARRY => local_store::capture(
                    memory,
                    base,
                    game_state,
                    store,
                    &crate::local_subunit_changes::Observer,
                    &mut self.local.rows,
                    &mut self.requirements.first,
                ),
                _ => cdeployment_status::walk_local_subunits(
                    memory,
                    base,
                    game_state,
                    &mut self.local.rows,
                    &mut self.requirements.first,
                ),
            };
            self.local.outcome = walk.outcome;
            self.required_local_rows = walk.required_rows;
            self.requirements.len = walk.requirements_len;
            self.requirements.required = walk.required_requirements;
            ticks[0] = qpc().saturating_sub(started).max(0) as u64;
        } else {
            self.local.outcome = skipped;
        }
        if should_walk(TABLES[1], hour, force_full) {
            let _reads = crate::read_accounting::table(TABLES[1]);
            let started = qpc();
            self.rosters.outcome = cproduction_status::walk_available_equipment(
                memory,
                base,
                game_state,
                &mut self.rosters.rows,
            );
            ticks[1] = qpc().saturating_sub(started).max(0) as u64;
        } else {
            self.rosters.outcome = skipped;
        }
        if should_walk(TABLES[2], hour, force_full) {
            let _reads = crate::read_accounting::table(TABLES[2]);
            let started = qpc();
            let walk = unit_bonuses::walk(
                memory,
                base,
                game_state,
                unit_bonuses::Buffers::growing(
                    &mut self.bonuses.rows,
                    &mut self.bonus_targets.first,
                    &mut self.bonus_grants.first,
                    &mut self.bonus_stats.first,
                    &mut self.bonus_labels.first,
                ),
            );
            let (written, required) = (walk.written, walk.required);
            self.bonuses.outcome = walk.outcome;
            self.required_bonus_rows = required.rows;
            (self.bonus_targets.len, self.bonus_targets.required) =
                (written.targets, required.targets);
            (self.bonus_grants.len, self.bonus_grants.required) = (written.grants, required.grants);
            (self.bonus_stats.len, self.bonus_stats.required) = (written.stats, required.stats);
            (self.bonus_labels.len, self.bonus_labels.required) =
                (written.label_bytes, required.label_bytes);
            ticks[2] = qpc().saturating_sub(started).max(0) as u64;
        } else {
            self.bonuses.outcome = skipped;
        }
        ticks
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.local.outcome,
            self.rosters.outcome,
            self.bonuses.outcome,
        ]
    }
    pub fn tables(&self) -> [TableView; N] {
        [self.local_table(), self.roster_table(), self.bonus_table()]
    }
    fn local_table(&self) -> TableView {
        let Some(requirements) = self.requirements.first.get(..self.requirements.len) else {
            return unavailable("local subunit requirement arena invalid");
        };
        let Some(rows) = self.local.rows.get(..self.local.outcome.len()) else {
            return unavailable("local subunit row count invalid");
        };
        let span = |row: &cdeployment_status::LocalSubunitRow| {
            row.requirements
                .start
                .checked_add(row.requirements.len)
                .and_then(|end| requirements.get(row.requirements.start..end))
        };
        if rows.iter().any(|row| span(row).is_none()) {
            return unavailable("local subunit requirements outside captured arena");
        }
        table::<tables::CountryLocalSubunits, _>(self.local.outcome, rows, |row| {
            CountryLocalSubunitRow {
                country: row.country,
                token: row.token,
                requirements: owned_vec(map_slice(span(row).unwrap(), |requirement| {
                    LocalSubunitRequirement {
                        archetype_token: requirement.archetype_token,
                        amount_raw: requirement.amount_raw,
                    }
                })),
            }
        })
    }
    fn roster_table(&self) -> TableView {
        table::<tables::AvailableEquipmentVariants, _>(
            self.rosters.outcome,
            &self.rosters.rows,
            |row| AvailableEquipmentVariantRow {
                country: row.country,
                roster_kind: row.roster_kind,
                ordinal: row.ordinal,
                variant: row.variant,
            },
        )
    }
    fn bonus_table(&self) -> TableView {
        let (Some(targets), Some(grants), Some(stats), Some(labels)) = (
            self.bonus_targets.first.get(..self.bonus_targets.len),
            self.bonus_grants.first.get(..self.bonus_grants.len),
            self.bonus_stats.first.get(..self.bonus_stats.len),
            self.bonus_labels.first.get(..self.bonus_labels.len),
        ) else {
            return unavailable("unit bonus arena invalid");
        };
        let Some(rows) = self.bonuses.rows.get(..self.bonuses.outcome.len()) else {
            return unavailable("unit bonus row count invalid");
        };
        let inside = |row: &unit_bonuses::Row| {
            span(labels, row.label).is_some()
                && span(targets, row.targets).is_some_and(|targets| {
                    targets.iter().all(|target| {
                        span(stats, target.stats).is_some()
                            && span(grants, target.grants).is_some_and(|grants| {
                                grants
                                    .iter()
                                    .all(|grant| span(stats, grant.stats).is_some())
                            })
                    })
                })
        };
        if !rows.iter().all(inside) {
            return unavailable("unit bonus spans outside captured arenas");
        }
        let stat = |stat: &unit_bonuses::Stat| UnitBonusStat {
            selector: stat.selector,
            value_raw: stat.value_raw,
        };
        table::<tables::CountryUnitBonuses, _>(self.bonuses.outcome, rows, |row| {
            CountryUnitBonusRow {
                country: row.country,
                ordinal: row.ordinal,
                source_kind: row.source_kind,
                source_id: row.source_id,
                source_number: row.source_number,
                label: owned_vec(copy_vec(span(labels, row.label).unwrap())),
                targets: owned_vec(map_slice(span(targets, row.targets).unwrap(), |target| {
                    UnitBonusTarget {
                        category: target.category,
                        token: target.token,
                        combat_width_raw: target.combat_width_raw,
                        stats: owned_vec(map_slice(span(stats, target.stats).unwrap(), stat)),
                        grants: owned_vec(map_slice(
                            span(grants, target.grants).unwrap(),
                            |grant| UnitBonusGrant {
                                category_token: grant.category_token,
                                add: grant.add,
                                display_as_percentage: grant.display_as_percentage,
                                stats: owned_vec(map_slice(
                                    span(stats, grant.stats).unwrap(),
                                    stat,
                                )),
                            },
                        )),
                    }
                })),
            }
        })
    }
}

fn span<T>(values: &[T], range: Span) -> Option<&[T]> {
    range
        .start
        .checked_add(range.len)
        .and_then(|end| values.get(range.start..end))
}

mod local_store;
pub use local_store::LocalStore;

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableRows;

    #[test]
    fn each_table_walks_on_its_own_cadence() {
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut captured = Captured::with_capacities(&Capacities::default());
        for hour in 0..24 {
            captured.capture(&memory, 0, 0x1000, hour, false);
            for (table, outcome) in TABLES.into_iter().zip(captured.outcomes()) {
                let skipped = outcome
                    == Outcome::Unavailable {
                        reason: SKIPPED_REASON,
                    };
                assert_eq!(
                    skipped,
                    !should_walk(table, hour, false),
                    "{table:?} {hour}"
                );
            }
            assert!(should_walk(TableId::CountryLocalSubunits, hour, false));
        }
    }

    #[test]
    fn requirements_survive_conversion_and_escapes_are_refused() {
        let mut captured = Captured::with_capacities(&Capacities {
            local_rows: 1,
            requirements: 2,
            rosters: 1,
            ..Capacities::default()
        });
        captured.local.rows[0] = cdeployment_status::LocalSubunitRow {
            country: 3,
            token: 5,
            requirements: engine_native::creferenced_division_template::Span { start: 0, len: 2 },
        };
        captured.local.outcome = Outcome::Rows { len: 1 };
        captured.requirements.first[1] = cdeployment_status::LocalRequirement {
            archetype_token: 9,
            amount_raw: -1,
        };
        captured.requirements.len = 2;
        let TableView::Complete(TableRows::CountryLocalSubunits(rows)) = captured.local_table()
        else {
            panic!("complete local subunits");
        };
        assert_eq!((rows[0].country, rows[0].token), (3, 5));
        assert_eq!(
            rows[0].requirements[1],
            LocalSubunitRequirement {
                archetype_token: 9,
                amount_raw: -1
            }
        );
        captured.local.rows[0].requirements.len = 3;
        assert!(matches!(
            captured.local_table(),
            TableView::Unavailable { .. }
        ));
    }

    #[test]
    fn countries_use_live_roster_and_unit_bonus_demand_and_same_hour_growth() {
        use engine_native::{ccountry, cequipment_variant, cgame_state};
        use engine_native_parts::csub_unit_definition;
        use engine_native_memory::msvc_string;
        const COUNTRIES: usize = 5;
        const STATE: usize = 0x1000;
        const FIRST: usize = 0x20000;
        const STRIDE: usize = 0x1800;
        const DATABASE: usize = 0x11000;
        let variant = FIRST + COUNTRIES * STRIDE;
        let pointers = variant + 0x100;
        let entries = pointers + 64 * 8;
        let definition = entries + 3 * unit_bonuses::STRIDE;
        let exact = definition + 0x400;
        let mut bytes = vec![0; exact + 8];
        let word = |bytes: &mut [u8], at: usize, value: u64| {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        };
        let array = |bytes: &mut [u8], at: usize, data: usize, count: usize| {
            word(bytes, at, data as u64);
            bytes[at + 8..at + 12].copy_from_slice(&(count as u32).to_le_bytes());
            bytes[at + 12..at + 16].copy_from_slice(&(count as u32).to_le_bytes());
        };
        array(
            &mut bytes,
            STATE + cgame_state::COUNTRIES,
            0x2000,
            COUNTRIES + 1,
        );
        array(
            &mut bytes,
            STATE + cgame_state::COUNTRY_LINKS,
            0xc000,
            COUNTRIES + 1,
        );
        array(
            &mut bytes,
            DATABASE + csub_unit_definition::DATABASE_VECTOR,
            pointers,
            1,
        );
        word(&mut bytes, variant, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
        word(
            &mut bytes,
            variant + cequipment_variant::TYPE,
            0x8000_0001_0000_0046,
        );
        for index in 0..64 {
            word(&mut bytes, pointers + index * 8, variant as u64);
        }
        for entry in (0..3).map(|index| entries + index * unit_bonuses::STRIDE) {
            word(&mut bytes, entry, unit_bonuses::VTABLE_RVA.0 as u64);
            word(
                &mut bytes,
                entry + unit_bonuses::PAYLOAD,
                unit_bonuses::PAYLOAD_VTABLE_RVA.0 as u64,
            );
            word(
                &mut bytes,
                entry + unit_bonuses::LABEL + msvc_string::CAPACITY,
                msvc_string::INLINE_CAPACITY,
            );
        }
        array(
            &mut bytes,
            entries + unit_bonuses::PAYLOAD + unit_bonuses::EXACT_TARGETS,
            exact,
            1,
        );
        word(&mut bytes, exact, definition as u64);
        word(
            &mut bytes,
            definition,
            csub_unit_definition::VTABLE_RVA.0 as u64,
        );
        word(&mut bytes, definition + csub_unit_definition::TOKEN, 41);
        word(&mut bytes, definition + unit_bonuses::STATS, 25_000);
        for index in 0..COUNTRIES {
            let id = index + 1;
            let country = FIRST + index * STRIDE;
            let production = country + 0x1000;
            let deployment = country + 0x1400;
            word(&mut bytes, 0x2000 + id * 8, country as u64);
            bytes[0xc000 + id * 4..0xc004 + id * 4].copy_from_slice(&(id as u32).to_le_bytes());
            word(&mut bytes, country, ccountry::VTABLE_RVA.0 as u64);
            bytes[country + ccountry::ID..country + ccountry::ID + 4]
                .copy_from_slice(&(id as u32).to_le_bytes());
            word(
                &mut bytes,
                country + ccountry::PRODUCTION_STATUS,
                production as u64,
            );
            word(
                &mut bytes,
                production,
                cproduction_status::VTABLE_RVA.0 as u64,
            );
            word(
                &mut bytes,
                production + cproduction_status::COUNTRY,
                country as u64,
            );
            for (offset, count) in [
                (cproduction_status::AVAILABLE_EQUIPMENT, 13),
                (cproduction_status::FOREIGN_EQUIPMENT, 9),
                (cproduction_status::BEST_VARIANT_CANDIDATES, 31),
            ] {
                array(&mut bytes, production + offset, pointers, count);
            }
            word(
                &mut bytes,
                country + ccountry::DEPLOYMENT_MANAGER,
                deployment as u64,
            );
            word(
                &mut bytes,
                deployment,
                cdeployment_status::VTABLE_RVA.0 as u64,
            );
            word(
                &mut bytes,
                deployment + cdeployment_status::OWNER,
                country as u64,
            );
            array(
                &mut bytes,
                deployment + cdeployment_status::UNIT_BONUSES,
                entries,
                3,
            );
        }
        struct Fixture<'a>(&'a [u8]);
        impl Memory for Fixture<'_> {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                if at == csub_unit_definition::DATABASE_RVA.0 && output.len() == 8 {
                    output.copy_from_slice(&(DATABASE as u64).to_le_bytes());
                    return true;
                }
                engine_native_memory::memory::SliceMemory {
                    base: 0,
                    bytes: self.0,
                }
                .copy(at, output)
            }
        }
        let memory = Fixture(&bytes);
        let mut capacity = Capacities::default();
        capacity.presize(&memory, 0, STATE).unwrap();
        assert!(capacity.rosters >= COUNTRIES * 53);
        assert_eq!(
            capacity.bonuses,
            unit_bonuses::Counts {
                rows: COUNTRIES * 3,
                targets: COUNTRIES,
                grants: 0,
                stats: COUNTRIES,
                label_bytes: 0,
            }
        );
        let mut captured = Captured::with_capacities(&capacity);
        captured.capture(&memory, 0, STATE, 0, true);
        assert_eq!(
            captured.rosters.outcome,
            Outcome::Rows {
                len: COUNTRIES * 53
            }
        );
        assert_eq!(
            captured.rosters.rows[COUNTRIES * 53 - 1].country,
            COUNTRIES as u32
        );
        assert_eq!(
            captured.bonuses.outcome,
            Outcome::Rows { len: COUNTRIES * 3 }
        );
        let TableView::Complete(TableRows::CountryUnitBonuses(rows)) = captured.bonus_table()
        else {
            panic!("complete unit bonuses");
        };
        assert_eq!(
            (
                rows[COUNTRIES * 3 - 1].country,
                rows[COUNTRIES * 3 - 1].ordinal
            ),
            (COUNTRIES as u32, 2)
        );
        assert_eq!(
            rows[0].targets[0].stats,
            [UnitBonusStat {
                selector: 0,
                value_raw: 25_000
            }]
        );
        let stats = captured.bonus_stats.len;
        captured.bonus_stats.len = stats - 1;
        assert!(matches!(
            captured.bonus_table(),
            TableView::Unavailable { .. }
        ));
        captured.bonus_stats.len = stats;
        array(
            &mut bytes,
            FIRST + 0x1000 + cproduction_status::BEST_VARIANT_CANDIDATES,
            pointers,
            64,
        );
        let memory = Fixture(&bytes);
        captured.capture(&memory, 0, STATE, 24, true);
        assert_eq!(
            captured.rosters.outcome,
            Outcome::Rows {
                len: COUNTRIES * 53 + 33
            }
        );
        assert_eq!(captured.rosters.rows[13 + 9 + 63].ordinal, 63);
    }
}
