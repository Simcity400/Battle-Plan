use crate::{air_record_changes as family, clock::qpc, table_conversion::unavailable};
use engine_native_memory::{Memory, walk::Outcome};
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::frame::TableInput;
use std::sync::Arc;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::AirRegionCombat];
pub type Capacities = ();

const NOT_CAPTURED: &str = "air region combat holders not captured";

pub struct Captured {
    rows: Option<Arc<TableRows>>,
    outcome: Outcome,
}

impl Captured {
    pub fn vacant() -> Self {
        Self {
            rows: None,
            outcome: Outcome::Unavailable {
                reason: NOT_CAPTURED,
            },
        }
    }
    pub fn with_capacities(_capacity: &Capacities) -> Self {
        Self::vacant()
    }
    pub fn reset(&mut self) {
        *self = Self::vacant();
    }
    pub fn allocated_bytes(&self) -> usize {
        0
    }
    pub fn grow(&mut self, _capacity: &mut Capacities) {}
    pub fn fail(&mut self, reason: &'static str) {
        self.rows = None;
        self.outcome = Outcome::Unavailable { reason };
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.reset();
        let _reads = crate::read_accounting::table(TableId::AirRegionCombat);
        let start = qpc();
        match family::capture(memory, base, state, hour, force_full) {
            Ok((rows, len)) => {
                self.rows = Some(rows);
                self.outcome = Outcome::Rows { len };
            }
            Err(reason) => self.outcome = Outcome::Unavailable { reason },
        }
        [qpc().saturating_sub(start).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.outcome]
    }
    pub fn table_inputs(&self) -> [TableInput; N] {
        [match (&self.rows, self.outcome) {
            (Some(rows), Outcome::Rows { .. }) => TableInput::SharedComplete(Arc::clone(rows)),
            (_, Outcome::Unavailable { reason }) => TableInput::Sampled(unavailable(reason)),
            _ => TableInput::Sampled(unavailable(NOT_CAPTURED)),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_frame::frame::TableView;

    #[test]
    fn a_capture_publishes_its_shared_rows_or_its_refusal() {
        let mut captured = Captured::vacant();
        assert!(matches!(
            &captured.table_inputs()[0],
            TableInput::Sampled(TableView::Unavailable { reason }) if reason == NOT_CAPTURED
        ));
        let rows = Arc::new(TableRows::AirRegionCombat(vec![
            replay_data::rows::AirRegionCombatRow {
                country: 2,
                region: 7,
                ..Default::default()
            },
        ]));
        captured.rows = Some(Arc::clone(&rows));
        captured.outcome = Outcome::Rows { len: 1 };
        let TableInput::SharedComplete(shared) = &captured.table_inputs()[0] else {
            panic!("published rows are shared")
        };
        assert!(Arc::ptr_eq(shared, &rows));
        captured.fail("synthetic failure");
        assert!(matches!(
            &captured.table_inputs()[0],
            TableInput::Sampled(TableView::Unavailable { reason }) if reason == "synthetic failure"
        ));
        assert_eq!(
            captured.outcomes(),
            [Outcome::Unavailable {
                reason: "synthetic failure"
            }]
        );
    }
}
