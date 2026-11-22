use crate::division_census::Census;
use collector_primitives::table_capture::CaptureRows;
use collector_projection::table_conversion::table;
use engine_native::walk::Outcome;
use replay_data::{rows::DivisionCensusRow, rows::TableId, rows::tables};
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::DivisionCensus];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
}

collector_primitives::capacity_max!(Capacities { rows });

impl Capacities {
    pub fn raise_for_countries(&mut self, slots: usize) {
        collector_primitives::country_slots::raise(
            &mut self.rows,
            collector_primitives::country_slots::rows(slots, 1),
        );
    }
}

pub struct Captured {
    rows: CaptureRows<DivisionCensusRow>,
}
impl Captured {
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
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::DivisionCensus, _>(
            self.rows.outcome,
            &self.rows.rows,
            Clone::clone,
        )]
    }
    pub fn install(&mut self, census: &Census) {
        if let Err(reason) =
            engine_native::walk::Buffer::Growing(&mut self.rows.rows).ensure(census.country_count())
        {
            self.rows.outcome = Outcome::Unavailable { reason };
            return;
        }
        self.rows.outcome = census.install_counts(&mut self.rows.rows);
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.rows.fail(Outcome::Unavailable { reason });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_failure_keeps_truncated_country_growth_demand() {
        for reason in [
            "accepted division acquisition panicked",
            "missing active decode payload",
        ] {
            let mut capacity = Capacities { rows: 2 };
            let mut captured = Captured::with_capacities(&capacity);
            captured.rows.outcome = Outcome::Truncated {
                len: 2,
                unread_rows: 19,
            };
            captured.fail(reason);
            captured.fail(reason);
            assert_eq!(captured.outcomes(), [Outcome::Unavailable { reason }]);
            assert_eq!(
                captured.tables(),
                [collector_projection::table_conversion::unavailable(reason)]
            );
            captured.grow(&mut capacity);
            assert!(capacity.rows >= 21);
            assert!(captured.rows.rows.len() >= 21);
        }
    }
}
