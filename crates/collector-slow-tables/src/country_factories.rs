use crate::{clock::qpc, table_capture::CaptureRows, table_conversion::table};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::cproduction_status as native;
use replay_data::{
    cadence::SKIPPED_REASON, cadence::should_walk, rows::CountryFactoriesRow, rows::TableId,
    rows::tables,
};
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::CountryFactories];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
}

collector_primitives::capacity_max!(Capacities { rows });
impl Capacities {
    pub fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.rows, crate::country_scale::rows(slots, 1));
    }
}

pub struct Captured {
    rows: CaptureRows<native::FactoryRow>,
}

impl Captured {
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.rows
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
    }
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities { rows: 0 })
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
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
        if !should_walk(TABLES[0], hour, force_full) {
            self.rows.outcome = Outcome::Unavailable {
                reason: SKIPPED_REASON,
            };
            return [0];
        }
        let _reads = crate::read_accounting::table(TABLES[0]);
        let started = qpc();
        self.rows.outcome =
            native::walk_factories(memory, base, game_state, &mut self.rows.rows);
        [qpc().saturating_sub(started).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::CountryFactories, _>(
            self.rows.outcome,
            &self.rows.rows,
            |row| CountryFactoriesRow {
                country: row.country,
                military_levels: row.military.levels,
                military_total_raw: row.military.total_raw,
                military_controlled_raw: row.military.controlled_raw,
                military_owned_raw: row.military.owned_raw,
                military_damaged_raw: row.military.damaged_raw,
                military_used: row.military.used,
                military_unavailable: row.military.unavailable,
                military_from_subjects: row.military.from_subjects,
                military_to_overlord: row.military.to_overlord,
                military_from_others: row.military.from_others,
                military_to_others: row.military.to_others,
                naval_levels: row.naval.levels,
                naval_total_raw: row.naval.total_raw,
                naval_controlled_raw: row.naval.controlled_raw,
                naval_owned_raw: row.naval.owned_raw,
                naval_damaged_raw: row.naval.damaged_raw,
                naval_used: row.naval.used,
                naval_unavailable: row.naval.unavailable,
                naval_from_subjects: row.naval.from_subjects,
                naval_to_overlord: row.naval.to_overlord,
                naval_from_others: row.naval.from_others,
                naval_to_others: row.naval.to_others,
                civilian_levels: row.civilian.levels,
                civilian_total_raw: row.civilian.total_raw,
                civilian_controlled_raw: row.civilian.controlled_raw,
                civilian_owned_raw: row.civilian.owned_raw,
                civilian_damaged_raw: row.civilian.damaged_raw,
                civilian_used: row.civilian.used,
                civilian_unavailable: row.civilian.unavailable,
                civilian_from_subjects: row.civilian.from_subjects,
                civilian_to_overlord: row.civilian.to_overlord,
                civilian_from_others: row.civilian.from_others,
                civilian_to_others: row.civilian.to_others,
                civilian_license_exports: row.civilian_license_exports,
                civilian_trade_exports: row.civilian_trade_exports,
                consumer_goods: row.consumer_goods,
                special_projects: row.special_projects,
                license_imports: row.license_imports,
                trade_imports: row.trade_imports,
                contract_payments: row.contract_payments,
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
    use replay_data::rows::TableRows;

    #[test]
    fn unreadable_census_is_unavailable_and_rows_keep_each_group_apart() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(&Capacities { rows: 1 });
        capture.capture(&memory, 0, 0, 0, true);
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.reset();
        let group = |base: i32| native::FactoryGroup {
            levels: base,
            total_raw: i64::from(base) * 100_000 + 1,
            controlled_raw: i64::MIN,
            owned_raw: i64::MAX,
            damaged_raw: -1,
            used: base + 1,
            unavailable: base + 2,
            from_subjects: base + 3,
            to_overlord: base + 4,
            from_others: base + 5,
            to_others: base + 6,
        };
        capture.rows.rows[0] = native::FactoryRow {
            country: 17,
            military: group(10),
            naval: group(20),
            civilian: group(30),
            civilian_license_exports: 41,
            civilian_trade_exports: 42,
            consumer_goods: 43,
            special_projects: 44,
            license_imports: 45,
            trade_imports: 46,
            contract_payments: i32::MIN,
        };
        capture.rows.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(TableRows::CountryFactories(rows))] = capture.tables() else {
            panic!("complete");
        };
        let row = &rows[0];
        assert_eq!(row.country, 17);
        assert_eq!(
            [row.military_levels, row.naval_levels, row.civilian_levels],
            [10, 20, 30]
        );
        assert_eq!(row.naval_total_raw, 2_000_001);
        assert_eq!(row.civilian_controlled_raw, i64::MIN);
        assert_eq!(row.military_owned_raw, i64::MAX);
        assert_eq!(row.naval_to_others, 26);
        assert_eq!(row.civilian_used, 31);
        assert_eq!(
            [row.civilian_license_exports, row.civilian_trade_exports],
            [41, 42]
        );
        assert_eq!(
            [
                row.consumer_goods,
                row.special_projects,
                row.license_imports,
                row.trade_imports,
                row.contract_payments
            ],
            [43, 44, 45, 46, i32::MIN]
        );
    }

    #[test]
    fn a_skipped_hour_reads_nothing_and_a_failed_job_keeps_its_reason() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(&Capacities { rows: 1 });
        capture.fail_capture(5, false, "slow acquisition failed");
        assert_eq!(
            capture.outcomes()[0],
            Outcome::Unavailable {
                reason: "slow acquisition failed"
            }
        );
        capture.capture(&memory, 0, 0, 7, false);
        assert!(matches!(
            capture.outcomes()[0],
            Outcome::Unavailable { reason } if reason != SKIPPED_REASON
        ));
    }
}
