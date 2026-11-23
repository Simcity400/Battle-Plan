use crate::{
    clock::qpc,
    table_capture::{Arena, CaptureRows, grow_capacity},
    table_conversion::{table, unavailable},
};
use engine_allocation::{copy_vec, owned_vec};
use engine_native_memory::{Memory, walk::Outcome};
use engine_raid_inputs::raid_target_statuses as native;
use replay_data::{
    cadence::SKIPPED_REASON, cadence::should_walk, rows::RaidScope, rows::RaidTargetStatusRow,
    rows::TableId, rows::tables,
};
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::RaidTargetStatuses];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    detected: usize,
}

collector_primitives::capacity_max!(Capacities { rows, detected });
impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let demand = native::demand(memory, base, state)?;
        self.rows = self.rows.max(demand.rows);
        self.detected = self.detected.max(demand.detected);
        Ok(())
    }

    pub fn raise_for_countries(&mut self, slots: usize) {
        self.detected = self
            .detected
            .max(self.rows.saturating_mul(slots.div_ceil(8)));
    }
}

pub struct Captured {
    rows: CaptureRows<native::Row>,
    detected: Arena<u8>,
    required_rows: usize,
    refused_leaders: u32,
}

impl Captured {
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.rows
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
    }
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            detected: 0,
        })
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            detected: Arena::new(capacity.detected),
            required_rows: 0,
            refused_leaders: 0,
        }
    }
    pub fn refused_leaders(&self) -> u32 {
        self.refused_leaders
    }

    pub fn reset(&mut self) {
        self.rows.reset();
        self.detected.reset();
        self.required_rows = 0;
        self.refused_leaders = 0;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.detected.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        grow_capacity(&mut capacity.rows, self.required_rows);
        self.rows.grow(&mut capacity.rows);
        self.detected.grow(&mut capacity.detected);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        self.reset();
        if !should_walk(TABLES[0], hour, force_full) {
            self.rows.outcome = Outcome::Unavailable {
                reason: SKIPPED_REASON,
            };
            return [0];
        }
        let started = qpc();
        let walk = native::walk(
            memory,
            base,
            game_state,
            &mut self.rows.rows,
            &mut self.detected.first,
        );
        self.rows.outcome = walk.outcome;
        self.detected.len = walk.detected_len;
        self.detected.required = walk.required_detected;
        self.required_rows = walk.required_rows;
        self.refused_leaders = walk.refused_leaders;
        [qpc().saturating_sub(started).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        let Some(bytes) = self.detected.first.get(..self.detected.len) else {
            return [unavailable("raid target detection arena length invalid")];
        };
        let Some(rows) = self.rows.rows.get(..self.rows.outcome.len()) else {
            return [unavailable("raid target row count invalid")];
        };
        let bits = |row: &native::Row| {
            row.detected_start
                .checked_add(row.detected_len)
                .and_then(|end| bytes.get(row.detected_start..end))
        };
        if rows.iter().any(|row| bits(row).is_none()) {
            return [unavailable("raid target detection outside captured arena")];
        }
        [table::<tables::RaidTargetStatuses, _>(
            self.rows.outcome,
            rows,
            |row| RaidTargetStatusRow {
                ordinal: row.ordinal,
                valid: row.valid,
                scope: scope(&row.scope),
                detected: owned_vec(copy_vec(bits(row).unwrap())),
            },
        )]
    }
}

pub fn scope(row: &engine_native_air::craid_target::Row) -> RaidScope {
    RaidScope {
        province: row.province,
        state: row.state,
        leader_reference: row.leader_reference,
        leader_province: row.leader_province,
        building_type: row.building_type,
        building_location: row.building_location,
        building_flags: row.building_flags,
        building_country: row.building_country,
        leader_country: row.leader_country,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableRows;

    #[test]
    fn detection_capacity_sizes_every_row_for_every_country_slot() {
        let mut capacity = Capacities {
            rows: 100,
            detected: 1,
        };
        capacity.raise_for_countries(5_000);
        assert_eq!(capacity.detected, 100 * 625);
    }

    #[test]
    fn detection_bytes_survive_conversion_and_escapes_are_refused() {
        let mut captured = Captured::with_capacities(&Capacities {
            rows: 1,
            detected: 2,
        });
        captured.rows.rows[0] = native::Row {
            ordinal: 7,
            detected_len: 2,
            ..Default::default()
        };
        captured.detected.first.copy_from_slice(&[4, 0x80]);
        captured.detected.len = 2;
        captured.rows.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(TableRows::RaidTargetStatuses(rows))] = captured.tables() else {
            panic!("expected complete detection rows");
        };
        assert_eq!(
            (rows[0].ordinal, rows[0].detected.as_slice()),
            (7, &[4, 0x80][..])
        );
        captured.rows.rows[0].detected_len = 3;
        assert!(matches!(
            captured.tables()[0],
            TableView::Unavailable { .. }
        ));
    }
}
