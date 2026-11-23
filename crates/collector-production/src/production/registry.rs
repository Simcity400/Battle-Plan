
use super as production;
use crate::table_capture::CaptureRows;
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::cequipment_variant;
use replay_data::rows::TableId;

pub struct Registry {
    rows: CaptureRows<cequipment_variant::RegistryRow>,
}
impl Registry {
    pub fn new(capacity: usize) -> Self {
        Self {
            rows: CaptureRows::new(capacity),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
    }
    pub fn acquire(&mut self, memory: &impl Memory, base: usize, game_state: usize) {
        let _reads = crate::read_accounting::table(TableId::VariantDefinitions);
        self.rows.outcome = bounded_registry(
            cequipment_variant::walk_registry(memory, base, game_state, &mut self.rows.rows),
            self.rows.rows.len(),
        );
    }
    pub fn mark_skipped(&mut self) {
        self.rows.outcome = Outcome::Unavailable {
            reason: replay_data::cadence::SKIPPED_REASON,
        };
    }
    pub fn sources(&self) -> production::Sources<'_, cequipment_variant::RegistryRow> {
        production::Sources {
            rows: &self.rows.rows,
            outcome: self.rows.outcome,
        }
    }
    pub fn grow(&mut self, capacity: &mut usize) {
        *capacity = (*capacity).max(self.rows.outcome.len());
        self.rows.grow(capacity);
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.rows.outcome = Outcome::Unavailable { reason };
    }
}
fn bounded_registry(outcome: Outcome, capacity: usize) -> Outcome {
    if outcome.len() > capacity {
        Outcome::Unavailable {
            reason: "variant registry count exceeds buffer",
        }
    } else if matches!(outcome, Outcome::Truncated { unread_rows: 0, .. }) {
        Outcome::Unavailable {
            reason: "variant registry truncation has no omitted rows",
        }
    } else {
        outcome
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_false_counts_are_rejected_and_incomplete_censuses_remain_incomplete() {
        assert!(matches!(
            bounded_registry(Outcome::Rows { len: 3 }, 2),
            Outcome::Unavailable { .. }
        ));
        assert!(matches!(
            bounded_registry(
                Outcome::Truncated {
                    len: 0,
                    unread_rows: 0
                },
                2
            ),
            Outcome::Unavailable { .. }
        ));
        let empty = bounded_registry(Outcome::Rows { len: 0 }, 0);
        assert_eq!(empty, Outcome::Rows { len: 0 });
        let truncated = Outcome::Truncated {
            len: 1,
            unread_rows: 9,
        };
        assert_eq!(bounded_registry(truncated, 1), truncated);
    }
}
