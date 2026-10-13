//! AirRegionCombat, the retained regional air-combat holders, captured by the
//! FinalInputs job from the store [`crate::air_record_changes`] keeps across
//! hours. The table is published shared with that store, so an hour that
//! changed no held row hands the encoder the same rows again.
use crate::{air_record_changes as family, sampling::qpc, spooling::unavailable};
use engine::{Memory, walk::Outcome};
use replay::{
    frame::TableInput,
    rows::{TableId, TableRows},
};
use std::sync::Arc;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::AirRegionCombat];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];
/// The rows live in the family's store, sized from the holders it reads.
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
    /// The shared rows belong to the family's store.
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
    use replay::frame::TableView;

    #[test]
    fn a_capture_publishes_its_shared_rows_or_its_refusal() {
        let mut captured = Captured::vacant();
        assert!(matches!(
            &captured.table_inputs()[0],
            TableInput::Sampled(TableView::Unavailable { reason }) if reason == NOT_CAPTURED
        ));
        let rows = Arc::new(TableRows::AirRegionCombat(vec![
            replay::rows::AirRegionCombatRow {
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
