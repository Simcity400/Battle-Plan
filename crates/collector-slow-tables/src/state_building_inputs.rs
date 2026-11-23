use crate::{clock::qpc, table_capture::CaptureRows, table_conversion::table};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::cstate::building_inputs as native;
use replay_data::{
    cadence::SKIPPED_REASON, cadence::should_walk, rows::StateBuildingInputRow, rows::TableId,
    rows::tables,
};
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::StateBuildingInputs];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
}

collector_primitives::capacity_max!(Capacities { rows });

pub struct Captured {
    rows: CaptureRows<native::Row>,
    keys: native::Keys,
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
            keys: native::Keys::default(),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.keys.allocated_bytes()
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
        self.rows.outcome = native::walk(
            memory,
            base,
            game_state,
            &mut self.keys,
            &mut self.rows.rows,
        );
        [qpc().saturating_sub(started).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::StateBuildingInputs, _>(
            self.rows.outcome,
            &self.rows.rows,
            |row| {
                let pair = |values: Option<[i64; 2]>, at: usize| values.map(|values| values[at]);
                let triple = |values: Option<[i64; 3]>, at: usize| values.map(|values| values[at]);
                StateBuildingInputRow {
                    state: row.state,
                    owner: row.owner,
                    controller: row.controller,
                    extra_shared_slots: row.extra_shared_slots,
                    local_building_slots: pair(row.owner_state, 0),
                    local_building_slots_factor: pair(row.owner_state, 1),
                    global_building_slots: pair(row.owner_country, 0),
                    global_building_slots_factor: pair(row.owner_country, 1),
                    local_factories: triple(row.controller_state, 0),
                    local_factory_energy_consumption: triple(row.controller_state, 1),
                    local_factory_energy_consumption_per_infrastructure: triple(
                        row.controller_state,
                        2,
                    ),
                    factory_energy_consumption: row.controller_country,
                    factory_energy_factor: row.energy_factor,
                }
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
    fn a_daily_walk_keeps_each_groups_refusal_apart() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let slot = replay_data::cadence::refresh_slot(TABLES[0]).unwrap() as i32;
        let mut capture = Captured::with_capacities(&Capacities { rows: 1 });
        capture.capture(&memory, 0, 0, slot + 1, false);
        assert_eq!(
            capture.outcomes()[0],
            Outcome::Unavailable {
                reason: SKIPPED_REASON
            }
        );
        capture.capture(&memory, 0, 0, slot, false);
        assert!(matches!(
            capture.outcomes()[0],
            Outcome::Unavailable { reason } if reason != SKIPPED_REASON
        ));
        capture.reset();
        capture.rows.rows[0] = native::Row {
            state: 9,
            owner: 3,
            controller: 4,
            extra_shared_slots: -1,
            owner_state: Some([200_000, -50_000]),
            owner_country: None,
            controller_state: Some([-30_000, 10_000, 5]),
            controller_country: Some(0),
            energy_factor: None,
        };
        capture.rows.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(TableRows::StateBuildingInputs(rows))] = capture.tables() else {
            panic!("complete");
        };
        assert_eq!(
            rows[0],
            StateBuildingInputRow {
                state: 9,
                owner: 3,
                controller: 4,
                extra_shared_slots: -1,
                local_building_slots: Some(200_000),
                local_building_slots_factor: Some(-50_000),
                global_building_slots: None,
                global_building_slots_factor: None,
                local_factories: Some(-30_000),
                local_factory_energy_consumption: Some(10_000),
                local_factory_energy_consumption_per_infrastructure: Some(5),
                factory_energy_consumption: Some(0),
                factory_energy_factor: None,
            }
        );
    }
}
