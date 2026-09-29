//! Hourly country pride-of-the-fleet reference and owned-state count, and
//! each country's civil-war array (CountryCivilWars), from the same country
//! walk's census.
use crate::{sampling::qpc, spooling::table, table_capture::CaptureRows};
use engine::{Memory, ccountry, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{CountryCivilWarRow, CountryCounterInputRow, TableId, tables},
};

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::CountryCounterInputs, TableId::CountryCivilWars];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    civil_wars: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 512,
            civil_wars: 64,
        }
    }
}
impl Capacities {
    /// One row per canonical country slot.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.rows, crate::country_scale::rows(slots, 1));
    }

    /// CountryCivilWars from the live entry total
    /// (`country_scale::observe_politics`), with the three-quarters growth
    /// rule's headroom. Never lowers.
    pub(crate) fn raise_for_civil_wars(&mut self, entries: usize) {
        crate::country_scale::raise(&mut self.civil_wars, crate::country_scale::rows(entries, 1));
    }
}

pub struct Captured {
    rows: CaptureRows<ccountry::CounterInputsRow>,
    civil_wars: CaptureRows<ccountry::CivilWarRow>,
}
impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            civil_wars: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            civil_wars: CaptureRows::new(capacity.civil_wars),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.civil_wars.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.civil_wars.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        self.civil_wars.grow(&mut capacity.civil_wars);
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let mut ticks = [0; N];
        {
            let _reads = crate::read_accounting::table(TABLES[0]);
            let begin = qpc();
            self.rows.outcome =
                ccountry::walk_counter_inputs(memory, base, state, &mut self.rows.rows);
            ticks[0] = qpc().saturating_sub(begin).max(0) as u64;
        }
        let _reads = crate::read_accounting::table(TABLES[1]);
        let begin = qpc();
        self.civil_wars.outcome =
            ccountry::walk_civil_wars(memory, base, state, &mut self.civil_wars.rows);
        ticks[1] = qpc().saturating_sub(begin).max(0) as u64;
        ticks
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome, self.civil_wars.outcome]
    }

    pub fn tables(&self) -> [TableView; N] {
        [
            table::<tables::CountryCounterInputs, _>(self.rows.outcome, &self.rows.rows, |r| {
                CountryCounterInputRow {
                    country: r.country,
                    pride_of_the_fleet: r.pride_of_the_fleet,
                    owned_state_count: r.owned_state_count,
                    controlled_state_count: r.controlled_state_count,
                }
            }),
            table::<tables::CountryCivilWars, _>(
                self.civil_wars.outcome,
                &self.civil_wars.rows,
                |r| CountryCivilWarRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    target: r.target,
                },
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;
    #[test]
    fn unreadable_census_is_unavailable_and_country_operands_preserve_primary_values() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 1,
            civil_wars: 2,
        });
        capture.capture(&memory, 0, 0);
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        assert!(matches!(capture.tables()[1], TableView::Unavailable { .. }));
        capture.reset();
        capture.rows.rows[0] = ccountry::CounterInputsRow {
            country: 7,
            pride_of_the_fleet: (32 << 32) | 51,
            owned_state_count: 3,
            controlled_state_count: 5,
        };
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.civil_wars.rows[0] = ccountry::CivilWarRow {
            country: 7,
            ordinal: 0,
            target: 0x8000_0002,
        };
        capture.civil_wars.outcome = Outcome::Rows { len: 1 };
        let [
            TableView::Complete(replay::rows::TableRows::CountryCounterInputs(rows)),
            TableView::Complete(replay::rows::TableRows::CountryCivilWars(wars)),
        ] = capture.tables()
        else {
            panic!("complete");
        };
        assert_eq!(rows[0].pride_of_the_fleet, (32 << 32) | 51);
        assert_eq!(rows[0].country, 7);
        assert_eq!(rows[0].owned_state_count, 3);
        assert_eq!(rows[0].controlled_state_count, 5);
        assert_eq!(
            wars,
            [CountryCivilWarRow {
                country: 7,
                ordinal: 0,
                target: 0x8000_0002
            }]
        );
    }

    #[test]
    fn civil_war_capacity_follows_the_live_entry_total() {
        let mut capacity = Capacities::default();
        capacity.raise_for_civil_wars(3_000);
        assert!(capacity.civil_wars >= 4_000);
        let raised = capacity;
        capacity.raise_for_civil_wars(1);
        assert!(capacity == raised);
    }
}
