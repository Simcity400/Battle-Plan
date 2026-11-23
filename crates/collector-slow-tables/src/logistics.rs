use crate::{clock::qpc, table_capture::CaptureRows, table_conversion::table};
use engine_land_inputs::clogistics_status as native;
use engine_native_memory::{Memory, walk::Outcome};
use replay_data::{
    cadence::SKIPPED_REASON, cadence::should_walk, rows::LogisticsEquipmentRow, rows::TableId,
    rows::tables,
};
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::LogisticsEquipment];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
}

collector_primitives::capacity_max!(Capacities { rows });
impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        self.raise(native::row_demand(memory, base, state)?);
        Ok(())
    }
    pub fn raise(&mut self, rows: usize) {
        self.rows = self.rows.max(rows);
    }
}

pub struct Captured {
    rows: CaptureRows<native::Row>,
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
        self.rows.outcome = native::walk(memory, base, game_state, &mut self.rows.rows);
        crate::logistics_balance::fill(
            memory,
            base,
            &mut self.rows.rows[..self.rows.outcome.len()],
        );
        [qpc().saturating_sub(started).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::LogisticsEquipment, _>(
            self.rows.outcome,
            &self.rows.rows,
            |row| LogisticsEquipmentRow {
                country: row.country,
                archetype: row.archetype,
                token: row.token,
                balance_raw: row.balance_raw,
                stock_raw: row.stock_raw,
                available_raw: replay_data::rows::Recorded::Value(row.available_raw),
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableRows;

    #[test]
    fn walks_only_on_the_first_slot_after_the_daily_update_or_a_full_hour() {
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut captured = Captured::with_capacities(&Capacities::default());
        for hour in 0..24 {
            captured.capture(&memory, 0, 0x1000, hour, false);
            let skipped = captured.outcomes()[0]
                == Outcome::Unavailable {
                    reason: SKIPPED_REASON,
                };
            assert_eq!(skipped, hour != 0, "{hour}");
            assert_eq!(
                should_walk(TABLES[0], hour, false),
                should_walk(TableId::CountryIntel, hour, false)
            );
        }
        captured.capture(&memory, 0, 0x1000, 3, true);
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { reason } if reason != SKIPPED_REASON
        ));
    }

    #[test]
    fn conversion_keeps_record_order_and_native_values() {
        let mut captured = Captured::with_capacities(&Capacities { rows: 2 });
        captured.rows.rows[0] = native::Row {
            country: 4,
            archetype: 0,
            token: 0x5001,
            balance_raw: i64::MIN,
            stock_raw: 14_507_500_000,
            available_raw: Some(300_000),
            ..Default::default()
        };
        captured.rows.rows[1] = native::Row {
            country: 4,
            archetype: 1,
            token: 0x5002,
            balance_raw: -11_200_000,
            stock_raw: 0,
            available_raw: Some(0),
            ..Default::default()
        };
        captured.rows.outcome = Outcome::Rows { len: 2 };
        let [TableView::Complete(TableRows::LogisticsEquipment(rows))] = captured.tables() else {
            panic!("complete logistics rows");
        };
        assert_eq!(
            rows[..],
            [
                LogisticsEquipmentRow {
                    country: 4,
                    archetype: 0,
                    token: 0x5001,
                    balance_raw: i64::MIN,
                    stock_raw: 14_507_500_000,
                    available_raw: replay_data::rows::Recorded::Value(Some(300_000)),
                },
                LogisticsEquipmentRow {
                    country: 4,
                    archetype: 1,
                    token: 0x5002,
                    balance_raw: -11_200_000,
                    stock_raw: 0,
                    available_raw: replay_data::rows::Recorded::Value(Some(0)),
                },
            ]
        );
        captured.rows.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 81,
        };
        assert!(matches!(captured.tables()[0], TableView::Truncated { .. }));
    }
}
