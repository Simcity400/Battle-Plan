//! Daily inputs of the 3D land-model selectors that the game computes on demand
//! while drawing: country-local sub-unit clones whose requirements differ from
//! their global definition, and every country's equipment rosters.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_capacity},
};
use engine::allocation::{map_slice, owned_vec};
use engine::{Memory, cdeployment_status, cproduction_status, walk::Outcome};
use replay::{
    cadence::{SKIPPED_REASON, should_walk},
    frame::TableView,
    rows::{
        AvailableEquipmentVariantRow, CountryLocalSubunitRow, LocalSubunitRequirement, TableId,
        tables,
    },
};

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [
    TableId::CountryLocalSubunits,
    TableId::AvailableEquipmentVariants,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    local_rows: usize,
    requirements: usize,
    rosters: usize,
}

impl Capacities {
    /// Serialize@140E5D3B0 (+A0/+E8) and AddEquipmentVariant@140E48C70 (+B8)
    /// provide three independent live roster counts, with duplicates retained.
    pub(crate) fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        self.rosters = self.rosters.max(
            match cproduction_status::walk_available_equipment(memory, base, state, &mut []) {
                Outcome::Rows { len } => len,
                Outcome::Truncated { len, unread_rows } => len
                    .checked_add(unread_rows as usize)
                    .ok_or("equipment roster sizing overflow")?,
                Outcome::Unavailable { reason } => return Err(reason),
            },
        );
        let local = cdeployment_status::walk_local_subunits(memory, base, state, &mut [], &mut []);
        if let Outcome::Unavailable { reason } = local.outcome {
            return Err(reason);
        }
        self.local_rows = self.local_rows.max(local.required_rows);
        self.requirements = self.requirements.max(local.required_requirements);
        Ok(())
    }
}

pub struct Captured {
    local: CaptureRows<cdeployment_status::LocalSubunitRow>,
    requirements: Arena<cdeployment_status::LocalRequirement>,
    required_local_rows: usize,
    rosters: CaptureRows<cproduction_status::AvailableEquipmentRow>,
}

impl Captured {
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.local
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.rosters
            .fail(super::failed_capture(TABLES[1], hour, force_full, reason));
    }
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            local_rows: 0,
            requirements: 0,
            rosters: 0,
        })
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            local: CaptureRows::new(capacity.local_rows),
            requirements: Arena::new(capacity.requirements),
            required_local_rows: 0,
            rosters: CaptureRows::new(capacity.rosters),
        }
    }
    pub fn reset(&mut self) {
        self.local.reset();
        self.requirements.reset();
        self.required_local_rows = 0;
        self.rosters.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.local.allocated_bytes()
            + self.requirements.allocated_bytes()
            + self.rosters.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        grow_capacity(&mut capacity.local_rows, self.required_local_rows);
        self.local.grow(&mut capacity.local_rows);
        self.requirements.grow(&mut capacity.requirements);
        self.rosters.grow(&mut capacity.rosters);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.reset();
        let skipped = Outcome::Unavailable {
            reason: SKIPPED_REASON,
        };
        let mut ticks = [0; N];
        if should_walk(TABLES[0], hour, force_full) {
            let _reads = crate::read_accounting::table(TABLES[0]);
            let started = qpc();
            let walk = cdeployment_status::walk_local_subunits_growing(
                memory,
                base,
                game_state,
                &mut self.local.rows,
                &mut self.requirements.first,
            );
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
            self.rosters.outcome = cproduction_status::walk_available_equipment_growing(
                memory,
                base,
                game_state,
                &mut self.rosters.rows,
            );
            ticks[1] = qpc().saturating_sub(started).max(0) as u64;
        } else {
            self.rosters.outcome = skipped;
        }
        ticks
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.local.outcome, self.rosters.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        [self.local_table(), self.roster_table()]
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::TableRows;

    #[test]
    fn each_table_walks_only_on_its_own_daily_slot() {
        let memory = engine::memory::SliceMemory {
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
        }
    }

    #[test]
    fn requirements_survive_conversion_and_escapes_are_refused() {
        let mut captured = Captured::with_capacities(&Capacities {
            local_rows: 1,
            requirements: 2,
            rosters: 1,
        });
        captured.local.rows[0] = cdeployment_status::LocalSubunitRow {
            country: 3,
            token: 5,
            requirements: engine::creferenced_division_template::Span { start: 0, len: 2 },
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
    fn five_thousand_countries_use_live_three_roster_demand_and_same_hour_growth() {
        use engine::{ccountry, cequipment_variant, cgame_state, csub_unit_definition};
        const COUNTRIES: usize = 5_000;
        const STATE: usize = 0x1000;
        const FIRST: usize = 0x20000;
        const STRIDE: usize = 0x1800;
        const DATABASE: usize = 0x11000;
        let variant = FIRST + COUNTRIES * STRIDE;
        let pointers = variant + 0x100;
        let mut bytes = vec![0; pointers + 64 * 8];
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
        word(&mut bytes, variant, cequipment_variant::VTABLE_RVA as u64);
        word(
            &mut bytes,
            variant + cequipment_variant::TYPE,
            0x8000_0001_0000_0046,
        );
        for index in 0..64 {
            word(&mut bytes, pointers + index * 8, variant as u64);
        }
        for index in 0..COUNTRIES {
            let id = index + 1;
            let country = FIRST + index * STRIDE;
            let production = country + 0x1000;
            let deployment = country + 0x1400;
            word(&mut bytes, 0x2000 + id * 8, country as u64);
            bytes[0xc000 + id * 4..0xc004 + id * 4].copy_from_slice(&(id as u32).to_le_bytes());
            word(&mut bytes, country, ccountry::VTABLE_RVA as u64);
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
                cproduction_status::VTABLE_RVA as u64,
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
                cdeployment_status::VTABLE_RVA as u64,
            );
            word(
                &mut bytes,
                deployment + cdeployment_status::OWNER,
                country as u64,
            );
        }
        struct Fixture<'a>(&'a [u8]);
        impl Memory for Fixture<'_> {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                if at == csub_unit_definition::DATABASE_RVA && output.len() == 8 {
                    output.copy_from_slice(&(DATABASE as u64).to_le_bytes());
                    return true;
                }
                engine::memory::SliceMemory {
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
        // Demand grows with no country-count change or presize poll. The
        // original changed header supplies enough space in this same capture.
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
