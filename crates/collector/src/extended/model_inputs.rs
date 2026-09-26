//! Daily inputs of the 3D land-model selectors that the game computes on demand
//! while drawing: country-local sub-unit clones whose requirements differ from
//! their global definition, and every country's equipment rosters.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_capacity},
};
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    local_rows: usize,
    requirements: usize,
    rosters: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            local_rows: 256,
            requirements: 1024,
            rosters: 8192,
        }
    }
}

impl Capacities {
    /// Every country holds two rosters, tens of variants for a small country.
    /// Never lowers.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        self.rosters = self.rosters.max(slots.saturating_mul(32));
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
            let walk = cdeployment_status::walk_local_subunits(
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
                requirements: span(row)
                    .unwrap()
                    .iter()
                    .map(|requirement| LocalSubunitRequirement {
                        archetype_token: requirement.archetype_token,
                        amount_raw: requirement.amount_raw,
                    })
                    .collect(),
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
                assert_eq!(skipped, !should_walk(table, hour, false), "{table:?} {hour}");
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
}
