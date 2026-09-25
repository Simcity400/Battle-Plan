//! Persistent foreign-building detection bits, in the buildings' cadence.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_capacity},
};
use engine::{Memory, building_detection as native, walk::Outcome};
use replay::{
    cadence::{SKIPPED_REASON, should_walk},
    frame::TableView,
    rows::{BuildingDetectionRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::BuildingDetection];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    detected: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 4096,
            detected: 16_384,
        }
    }
}
impl Capacities {
    /// Each target holds one detection bit per country slot. Never lowers.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        self.detected = self
            .detected
            .max(self.rows.saturating_mul(slots.div_ceil(8)));
    }
}

pub struct Captured {
    rows: CaptureRows<native::Row>,
    detected: Arena<u8>,
    required_rows: usize,
    /// Country slots implied by the widest detection vector seen.
    slots: usize,
}

impl Captured {
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.rows
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
    }
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
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
            slots: 0,
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.detected.reset();
        self.required_rows = 0;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.detected.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        grow_capacity(&mut capacity.rows, self.required_rows);
        capacity.raise_for_countries(self.slots);
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
        self.slots = walk.widest.saturating_mul(8);
        [qpc().saturating_sub(started).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        let Some(bytes) = self.detected.first.get(..self.detected.len) else {
            return [unavailable("building detection arena length invalid")];
        };
        let Some(rows) = self.rows.rows.get(..self.rows.outcome.len()) else {
            return [unavailable("building detection row count invalid")];
        };
        let bits = |row: &native::Row| {
            row.detected_start
                .checked_add(row.detected_len)
                .and_then(|end| bytes.get(row.detected_start..end))
        };
        if rows.iter().any(|row| bits(row).is_none()) {
            return [unavailable("building detection outside captured arena")];
        }
        [table::<tables::BuildingDetection, _>(
            self.rows.outcome,
            rows,
            |row| BuildingDetectionRow {
                ordinal: row.ordinal,
                building_type: row.building_type,
                location: row.location,
                status_flags: row.status_flags,
                country: row.country,
                detected: bits(row).unwrap().to_vec(),
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::TableRows;

    #[test]
    fn growth_sizes_detection_bits_for_every_country_slot() {
        let mut capacity = Capacities {
            rows: 100,
            detected: 1,
        };
        let mut captured = Captured::with_capacities(&capacity);
        captured.slots = 5_000;
        captured.grow(&mut capacity);
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
        let [TableView::Complete(TableRows::BuildingDetection(rows))] = captured.tables() else {
            panic!("expected complete detection rows");
        };
        assert_eq!((rows[0].ordinal, rows[0].detected.as_slice()), (7, &[4, 0x80][..]));
        captured.rows.rows[0].detected_len = 3;
        assert!(matches!(
            captured.tables()[0],
            TableView::Unavailable { .. }
        ));
    }
}
