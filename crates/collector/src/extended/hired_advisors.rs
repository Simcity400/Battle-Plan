//! Each country's appointed advisors (CCountryCharacters +0x58) in native
//! order, with the advisor's character, idea token and slot. Daily, in the
//! advisor roles' slot.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{Memory, ccountry_characters as native, walk::Outcome};
use replay::{
    cadence::{SKIPPED_REASON, should_walk},
    frame::TableView,
    rows::{HiredAdvisorRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::HiredAdvisors];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// Appointed advisors per country slot the first capture is sized for; the
/// engine's own slot counts come from content, so this only presizes.
const PER_COUNTRY: usize = 4;
/// Idea token and slot bytes per advisor the first capture is sized for.
const BYTES_PER_ADVISOR: usize = 48;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    bytes: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 1024,
            bytes: 1024 * BYTES_PER_ADVISOR,
        }
    }
}

impl Capacities {
    /// A few rows per country slot. Never lowers.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        let rows = crate::country_scale::rows(slots, PER_COUNTRY);
        self.rows = self.rows.max(rows);
        self.bytes = self.bytes.max(rows.saturating_mul(BYTES_PER_ADVISOR));
    }
}

pub struct Captured {
    rows: CaptureRows<native::Row>,
    bytes: Arena<u8>,
}

impl Captured {
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.rows
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
    }
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities { rows: 0, bytes: 0 })
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            bytes: Arena::new(capacity.bytes),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.bytes.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.bytes.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        // A truncated walk reports at least the bytes of its first unread row.
        self.bytes.grow(&mut capacity.bytes);
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
        let walk = native::walk(
            memory,
            base,
            game_state,
            &mut self.rows.rows,
            &mut self.bytes.first,
        );
        self.rows.outcome = walk.outcome;
        self.bytes.len = walk.bytes_len;
        self.bytes.required = walk.required_bytes;
        [qpc().saturating_sub(started).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        let Some(bytes) = self.bytes.first.get(..self.bytes.len) else {
            return [unavailable("hired advisor string arena invalid")];
        };
        let text = |start: usize, len: usize| {
            start
                .checked_add(len)
                .and_then(|end| bytes.get(start..end))
        };
        let Some(rows) = self.rows.rows.get(..self.rows.outcome.len()) else {
            return [unavailable("hired advisor row count invalid")];
        };
        if rows.iter().any(|row| {
            text(row.idea_start, row.idea_len).is_none()
                || text(row.slot_start, row.slot_len).is_none()
        }) {
            return [unavailable("hired advisor strings outside captured arena")];
        }
        [table::<tables::HiredAdvisors, _>(
            self.rows.outcome,
            rows,
            |row| HiredAdvisorRow {
                country: row.country,
                ordinal: row.ordinal,
                character: row.character,
                idea: text(row.idea_start, row.idea_len).unwrap().to_vec(),
                slot: text(row.slot_start, row.slot_len).unwrap().to_vec(),
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::TableRows;

    #[test]
    fn walks_only_on_the_advisor_roles_slot_or_a_full_hour() {
        let memory = engine::memory::SliceMemory {
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
            assert_eq!(skipped, !should_walk(TABLES[0], hour, false), "{hour}");
            assert_eq!(
                should_walk(TABLES[0], hour, false),
                should_walk(TableId::AdvisorRoles, hour, false)
            );
        }
        captured.capture(&memory, 0, 0x1000, 3, true);
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { reason } if reason != SKIPPED_REASON
        ));
    }

    #[test]
    fn capacities_follow_the_live_country_slots() {
        let mut capacity = Capacities::default();
        capacity.raise_for_countries(5_000);
        assert!(capacity.rows >= 5_000 * PER_COUNTRY);
        assert!(capacity.bytes >= capacity.rows * BYTES_PER_ADVISOR);
        let raised = capacity;
        capacity.raise_for_countries(10);
        assert!(capacity == raised);
    }

    #[test]
    fn conversion_keeps_order_and_exact_bytes_and_refuses_escaped_spans() {
        let mut captured = Captured::with_capacities(&Capacities { rows: 2, bytes: 16 });
        captured.bytes.first[..12].copy_from_slice(b"idea\0slotabc");
        captured.bytes.len = 12;
        captured.rows.rows[0] = native::Row {
            country: 7,
            ordinal: 0,
            character: 1482,
            idea_start: 0,
            idea_len: 5,
            slot_start: 5,
            slot_len: 4,
        };
        captured.rows.rows[1] = native::Row {
            country: 7,
            ordinal: 1,
            character: 1451,
            idea_start: 9,
            idea_len: 3,
            slot_start: 12,
            slot_len: 0,
        };
        captured.rows.outcome = Outcome::Rows { len: 2 };
        let [TableView::Complete(TableRows::HiredAdvisors(rows))] = captured.tables() else {
            panic!("complete hired advisors");
        };
        assert_eq!(rows[0].idea, b"idea\0");
        assert_eq!(rows[0].slot, b"slot");
        assert_eq!((rows[1].ordinal, rows[1].character), (1, 1451));
        assert!(rows[1].slot.is_empty());
        captured.rows.rows[1].slot_len = 1;
        assert!(matches!(
            captured.tables()[0],
            TableView::Unavailable { .. }
        ));
    }
}
