use crate::naval_result_sources as producer;
use collector_primitives::{
    clock::qpc,
    table_capture::{CaptureRows, grow_vec},
};
use collector_projection::table_conversion::unavailable;
use engine_allocation as allocation;
use engine_native::{Memory, walk::Outcome};
use replay_data::{rows::NavalResultSourceRow, rows::TableId, rows::TableRows};
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::NavalResultSources];
pub type Capacities = usize;
pub struct Captured {
    rows: CaptureRows<producer::Entry>,
}
impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&0)
    }
    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(*c),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        *c = (*c).max(producer::demand());
        grow_vec(&mut self.rows.rows, *c);
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.rows.fail(Outcome::Unavailable { reason });
    }
    pub fn capture(&mut self, _memory: &impl Memory, _base: usize, _state: usize) -> [u64; N] {
        self.reset();
        let start = qpc();
        self.rows.outcome = match producer::drain(&mut self.rows.rows) {
            Ok(len) => Outcome::Rows { len },
            Err(reason) => Outcome::Unavailable { reason },
        };
        [qpc().saturating_sub(start).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        let Outcome::Rows { len } = self.rows.outcome else {
            let reason = match self.rows.outcome {
                Outcome::Unavailable { reason } => reason,
                _ => "naval result sources incomplete drain",
            };
            return [unavailable(reason)];
        };
        match project(&self.rows.rows[..len]) {
            Ok(rows) => [TableView::Complete(TableRows::NavalResultSources(rows))],
            Err(reason) => {
                producer::lost_drain();
                [unavailable(reason)]
            }
        }
    }
}

fn project(events: &[producer::Entry]) -> Result<Vec<NavalResultSourceRow>, &'static str> {
    allocation::collect_vec(events.iter().copied().map(producer::Entry::into_row))
        .map_err(|_| "collector allocation failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_preserves_each_combat_and_transient_native_reference() {
        let _observer = producer::tests::Observation::new();
        let new_result = producer::Entry {
            id: 1,
            game_hour: 40,
            result: 0xf003_0000_0046,
            combat: 0xf001_0000_0046,
            merged: false,
            transient: false,
            combat_created_hour: Some(38),
        };
        let events = [
            new_result,
            producer::Entry {
                id: 2,
                result: new_result.result + 1,
                combat: u64::MAX,
                transient: true,
                combat_created_hour: Some(40),
                ..new_result
            },
            producer::Entry {
                id: 3,
                result: new_result.result + 1,
                combat: u64::MAX,
                merged: true,
                transient: true,
                combat_created_hour: Some(40),
                ..new_result
            },
        ];
        let rows = project(&events).unwrap();
        assert_eq!(rows, events.map(producer::Entry::into_row));
        assert_eq!(rows.len(), 3);
        assert_ne!(rows[0].result, rows[1].result);
        assert_eq!(rows[1].result, rows[2].result);
        assert_ne!(rows[0].combat, rows[1].combat);
        assert!(!rows[0].merged);
        assert!(!rows[1].merged && rows[2].merged);
        assert!(rows[2].transient);
        assert_eq!(rows[2].combat, u64::MAX);
        assert_eq!(rows[2].combat_created_hour, Some(40));
        assert!(project(&[]).unwrap().is_empty());
    }

    #[test]
    fn unavailable_interval_is_never_projected_as_zero_events() {
        let _observer = producer::tests::Observation::new();
        let mut captured = Captured::with_capacities(&0);
        captured.rows.outcome = Outcome::Unavailable {
            reason: "naval result sources lost publication",
        };
        assert!(matches!(
            captured.tables(),
            [TableView::Unavailable { reason }]
                if reason == "naval result sources lost publication"
        ));
        captured.rows.outcome = Outcome::Rows { len: 0 };
        assert!(matches!(
            captured.tables(),
            [TableView::Complete(TableRows::NavalResultSources(rows))] if rows.is_empty()
        ));
    }
}
