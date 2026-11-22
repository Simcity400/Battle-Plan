use crate::{
    clock::qpc,
    table_capture::{CaptureRows, grow_vec},
    table_conversion::unavailable,
};
use collector_observers::building_bombing as producer;
use engine_allocation as allocation;
use engine_native::{Memory, walk::Outcome};
use replay_data::{rows::BuildingBombingDamageRow, rows::TableId, rows::TableRows};
use replay_frame::frame::{TableInput, TableView};
use std::sync::atomic::{AtomicBool, Ordering};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::BuildingBombingDamage];
pub type Capacities = usize;
pub struct Captured {
    rows: CaptureRows<producer::Entry>,
    loss_recorded: AtomicBool,
}
impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&0)
    }
    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(*c),
            loss_recorded: AtomicBool::new(false),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.loss_recorded.store(false, Ordering::Relaxed);
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }
    /// # Safety
    /// Caller holds exclusive `&mut` access to this frame while building-bombing observer storage
    /// stays live for the active session (or an isolated test) and teardown is excluded. Different
    /// frames may grow concurrently across spool and sizing threads because `EventLedger::prepare`
    /// locks its spare and retired slots.
    pub unsafe fn grow(&mut self, c: &mut Capacities) {
        // SAFETY: Caller guarantees live building-bombing observer storage with teardown excluded.
        unsafe { producer::presize(0) };
        *c = (*c).max(producer::capacity());
        grow_vec(&mut self.rows.rows, *c);
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.rows.fail(Outcome::Unavailable { reason });
        self.loss_recorded.store(false, Ordering::Relaxed);
    }
    /// # Safety
    /// Caller must hold hourly capture admission as the sole consumer of building-bombing events
    /// while observer storage stays live and teardown is excluded.
    pub unsafe fn capture(
        &mut self,
        _memory: &impl Memory,
        _base: usize,
        _state: usize,
    ) -> [u64; N] {
        self.reset();
        let started = qpc();
        // SAFETY: Caller holds hourly capture admission as the sole building-bombing consumer.
        self.rows.outcome = match unsafe { producer::drain(&mut self.rows.rows) } {
            Ok(len) => Outcome::Rows { len },
            Err(reason) => Outcome::Unavailable { reason },
        };
        [qpc().saturating_sub(started).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    fn project_tables(&self, record_loss: bool) -> [TableView; N] {
        let Outcome::Rows { len } = self.rows.outcome else {
            return [unavailable(match self.rows.outcome {
                Outcome::Unavailable { reason } => reason,
                _ => "building bombing incomplete drain",
            })];
        };
        match project(&self.rows.rows[..len]) {
            Ok(rows) => [TableView::Complete(TableRows::BuildingBombingDamage(rows))],
            Err(reason) => {
                if record_loss && !self.loss_recorded.swap(true, Ordering::AcqRel) {
                    // SAFETY: Caller is the owning publication path recording loss once.
                    unsafe { producer::lost_output(len) };
                }
                [unavailable(reason)]
            }
        }
    }
    pub fn tables(&self) -> [TableView; N] {
        self.project_tables(false)
    }
    /// # Safety
    /// Caller must be the owning spool publication path converting this frame while observer
    /// storage stays live; conversion failure records `lost_output` at most once per frame.
    pub unsafe fn table_inputs(&self) -> [TableInput; N] {
        self.project_tables(true).map(TableInput::Sampled)
    }
}
fn project(events: &[producer::Entry]) -> Result<Vec<BuildingBombingDamageRow>, &'static str> {
    if events.iter().any(|event| event.amount_raw == 0)
        || events.windows(2).any(|pair| pair[0].id >= pair[1].id)
    {
        return Err("building bombing invalid event order or amount");
    }
    allocation::collect_vec(events.iter().map(|event| BuildingBombingDamageRow {
        id: event.id,
        game_hour: event.game_hour,
        attacker: event.attacker,
        victim: event.victim,
        state: event.state,
        province: event.province,
        building_type: event.building_type,
        native_cause: event.native_cause,
        cause: event.cause.into(),
        amount_raw: event.amount_raw,
        credited_raw: event.credited_raw,
    }))
    .map_err(|_| "collector allocation failed")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn events_keep_native_units_hour_identity_null_attacker_and_credit_separate() {
        let first = producer::Entry {
            id: 11,
            game_hour: 60827795,
            attacker: Some(4999),
            victim: 1500,
            state: 77777,
            province: Some(88888),
            building_type: 0xf004,
            native_cause: 1,
            cause: engine_events::building_damage_sites::Cause::AirNaval,
            amount_raw: 9_000_001,
            credited_raw: 3_000_000,
        };
        let second = producer::Entry {
            id: 12,
            attacker: None,
            native_cause: 99,
            cause: engine_events::building_damage_sites::Cause::Unknown,
            ..first
        };
        let rows = project(&[first, second]).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, 11);
        assert_eq!(rows[0].game_hour, first.game_hour);
        assert_eq!(rows[0].attacker, Some(4999));
        assert_eq!(rows[0].building_type, 0xf004);
        assert_eq!(rows[0].state, 77777);
        assert_eq!(rows[0].province, Some(88888));
        assert_eq!(rows[0].amount_raw, 9_000_001);
        assert_eq!(rows[0].credited_raw, 3_000_000);
        assert_eq!(
            project(&[producer::Entry {
                amount_raw: -1,
                ..first
            }])
            .unwrap()[0]
                .amount_raw,
            -1
        );
        assert_eq!(rows[1].attacker, None);
        assert_eq!(rows[1].native_cause, 99);
        assert_eq!(
            rows[1].cause,
            replay_data::rows::BuildingBombingDamageCause::Unknown
        );
        assert!(project(&[second, first]).is_err());
        assert!(
            project(&[producer::Entry {
                amount_raw: 0,
                ..first
            }])
            .is_err()
        );
        assert!(project(&[]).unwrap().is_empty());
    }

    #[test]
    fn tables_is_pure_and_table_inputs_records_loss_at_most_once() {
        let mut captured = Captured::with_capacities(&1);
        captured.rows.rows = vec![producer::Entry {
            id: 1,
            game_hour: 1,
            attacker: None,
            victim: 1,
            state: 1,
            province: None,
            building_type: 1,
            native_cause: 1,
            cause: engine_events::building_damage_sites::Cause::Unknown,
            amount_raw: 0,
            credited_raw: 0,
        }];
        captured.rows.outcome = Outcome::Rows { len: 1 };
        assert!(matches!(captured.tables()[0], TableView::Unavailable { .. }));
        assert!(matches!(captured.tables()[0], TableView::Unavailable { .. }));
        assert!(!captured.loss_recorded.load(Ordering::Acquire));
        // SAFETY: Test exercises owning publication loss reporting on an isolated frame.
        let first = unsafe { captured.table_inputs() };
        assert!(matches!(
            first[0],
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
        assert!(captured.loss_recorded.load(Ordering::Acquire));
        // SAFETY: Test verifies repeated publication on the same frame does not double-record.
        let second = unsafe { captured.table_inputs() };
        assert!(matches!(
            second[0],
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
        captured.reset();
        assert!(!captured.loss_recorded.load(Ordering::Acquire));
    }
}
