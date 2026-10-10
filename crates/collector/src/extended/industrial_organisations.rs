//! Each country's military-industrial organisations
//! in array order, and each one's produced-units history. Daily, in the
//! production lines' slot, from one walk.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::allocation::{copy_vec, owned_vec};
use engine::{Memory, nindustrial_organisation_corganisation as native, walk::Outcome};
use replay::{
    cadence::{SKIPPED_REASON, should_walk},
    frame::TableView,
    rows::{IndustrialOrganisationHistoryRow, IndustrialOrganisationRow, TableId, tables},
};

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [
    TableId::IndustrialOrganisations,
    TableId::IndustrialOrganisationHistory,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("industrial organisations and their history", 0, &[0, 1])];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    organisations: usize,
    history: usize,
    bytes: usize,
}

impl Capacities {
    pub(crate) fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let (organisations, history, bytes) = native::demand(memory, base, state)?;
        crate::table_capture::grow_capacity(&mut self.organisations, organisations);
        crate::table_capture::grow_capacity(&mut self.history, history);
        crate::table_capture::grow_capacity(&mut self.bytes, bytes);
        Ok(())
    }
}

pub struct Captured {
    organisations: CaptureRows<native::Row>,
    history: CaptureRows<native::HistoryRow>,
    bytes: Arena<u8>,
}

impl Captured {
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.organisations
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.history
            .fail(super::failed_capture(TABLES[1], hour, force_full, reason));
    }
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            organisations: 0,
            history: 0,
            bytes: 0,
        })
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            organisations: CaptureRows::new(capacity.organisations),
            history: CaptureRows::new(capacity.history),
            bytes: Arena::new(capacity.bytes),
        }
    }
    pub fn reset(&mut self) {
        self.organisations.reset();
        self.history.reset();
        self.bytes.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.organisations.allocated_bytes()
            + self.history.allocated_bytes()
            + self.bytes.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.organisations.grow(&mut capacity.organisations);
        self.history.grow(&mut capacity.history);
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
            let skipped = Outcome::Unavailable {
                reason: SKIPPED_REASON,
            };
            self.organisations.outcome = skipped;
            self.history.outcome = skipped;
            return [0; N];
        }
        let _reads = crate::read_accounting::table(TABLES[0]);
        let started = qpc();
        let walk = native::walk_growing(
            memory,
            base,
            game_state,
            &mut self.organisations.rows,
            &mut self.history.rows,
            &mut self.bytes.first,
        );
        self.organisations.outcome = walk.organisations;
        self.history.outcome = walk.history;
        self.bytes.len = walk.bytes_len;
        self.bytes.required = walk.required_bytes;
        // One walk reads both; the anchor carries its cost.
        [qpc().saturating_sub(started).max(0) as u64, 0]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.organisations.outcome, self.history.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        fn text(bytes: &[u8], span: engine::cequipment_variant::TextSpan) -> Option<&[u8]> {
            span.start
                .checked_add(span.len)
                .and_then(|end| bytes.get(span.start..end))
        }
        let organisations = match (
            self.bytes.first.get(..self.bytes.len),
            self.organisations
                .rows
                .get(..self.organisations.outcome.len()),
        ) {
            (Some(bytes), Some(rows))
                if rows.iter().all(|row| {
                    text(bytes, row.name).is_some() && text(bytes, row.icon).is_some()
                }) =>
            {
                table::<tables::IndustrialOrganisations, _>(
                    self.organisations.outcome,
                    rows,
                    |row| IndustrialOrganisationRow {
                        organisation: row.reference,
                        country: row.country,
                        ordinal: row.ordinal,
                        owner: row.owner,
                        token: row.token,
                        name: owned_vec(copy_vec(text(bytes, row.name).unwrap())),
                        icon: owned_vec(copy_vec(text(bytes, row.icon).unwrap())),
                    },
                )
            }
            _ => unavailable("organisation strings outside captured arena"),
        };
        [
            organisations,
            table::<tables::IndustrialOrganisationHistory, _>(
                self.history.outcome,
                &self.history.rows,
                |row| IndustrialOrganisationHistoryRow {
                    organisation: row.organisation,
                    variant: row.variant,
                    units: row.units,
                },
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::TableRows;

    #[test]
    fn walks_only_on_the_production_lines_slot_or_a_full_hour() {
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut captured = Captured::with_capacities(&Capacities::default());
        for hour in 0..24 {
            captured.capture(&memory, 0, 0x1000, hour, false);
            let walks = should_walk(TableId::ProductionLines, hour, false);
            for outcome in captured.outcomes() {
                let skipped = outcome
                    == Outcome::Unavailable {
                        reason: SKIPPED_REASON,
                    };
                assert_eq!(skipped, !walks, "{hour}");
            }
        }
        captured.capture(&memory, 0, 0x1000, 0, true);
        assert!(
            captured
                .outcomes()
                .iter()
                .all(|outcome| !matches!(outcome, Outcome::Unavailable { reason } if *reason == SKIPPED_REASON))
        );
    }

    #[test]
    fn conversion_keeps_native_references_bytes_and_signed_units() {
        let mut captured = Captured::with_capacities(&Capacities {
            organisations: 1,
            history: 1,
            bytes: 4,
        });
        captured.bytes.first[..3].copy_from_slice(&[0xff, 0, b'k']);
        captured.bytes.len = 3;
        captured.organisations.rows[0] = native::Row {
            reference: u64::MAX,
            country: u32::MAX,
            ordinal: 7,
            owner: 0x8000_0000,
            token: u32::MAX - 1,
            name: engine::cequipment_variant::TextSpan { start: 0, len: 3 },
            icon: engine::cequipment_variant::TextSpan { start: 3, len: 0 },
        };
        captured.organisations.outcome = Outcome::Rows { len: 1 };
        captured.history.rows[0] = native::HistoryRow {
            organisation: u64::MAX,
            variant: (5 << 32) | 70,
            units: i32::MIN,
        };
        captured.history.outcome = Outcome::Rows { len: 1 };
        let [organisations, history] = captured.tables();
        let TableView::Complete(TableRows::IndustrialOrganisations(organisations)) = organisations
        else {
            panic!("{organisations:?}")
        };
        assert_eq!(
            organisations[0],
            IndustrialOrganisationRow {
                organisation: u64::MAX,
                country: u32::MAX,
                ordinal: 7,
                owner: 0x8000_0000,
                token: u32::MAX - 1,
                name: vec![0xff, 0, b'k'],
                icon: Vec::new(),
            }
        );
        let TableView::Complete(TableRows::IndustrialOrganisationHistory(history)) = history else {
            panic!("{history:?}")
        };
        assert_eq!(history[0].units, i32::MIN);
        // A span outside the captured arena withholds only the organisations.
        captured.organisations.rows[0].icon.start = 4;
        assert!(matches!(
            captured.tables()[0],
            TableView::Unavailable { .. }
        ));
        assert!(matches!(captured.tables()[1], TableView::Complete(_)));
        // A truncated walk stays truncated and grows off-thread.
        captured.history.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 3,
        };
        let mut capacity = Capacities {
            organisations: 1,
            history: 1,
            bytes: 4,
        };
        captured.grow(&mut capacity);
        assert!(capacity.history >= 4);
    }
}
