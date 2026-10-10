//! Worker drain of owned damage events; conversion never follows game pointers.
use crate::{
    building_bombing as producer,
    sampling::qpc,
    spooling::unavailable,
    table_capture::{CaptureRows, grow_vec},
};
use engine::{Memory, allocation, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{BuildingBombingDamageRow, TableId, TableRows},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::BuildingBombingDamage];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];
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
        producer::presize(0);
        *c = (*c).max(producer::capacity());
        grow_vec(&mut self.rows.rows, *c);
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.rows.fail(Outcome::Unavailable { reason });
    }
    pub fn capture_growing(
        &mut self,
        _memory: &impl Memory,
        _base: usize,
        _state: usize,
    ) -> [u64; N] {
        self.reset();
        let started = qpc();
        self.rows.outcome = match producer::drain_growing(&mut self.rows.rows) {
            Ok(len) => Outcome::Rows { len },
            Err(reason) => Outcome::Unavailable { reason },
        };
        [qpc().saturating_sub(started).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        let Outcome::Rows { len } = self.rows.outcome else {
            return [unavailable(match self.rows.outcome {
                Outcome::Unavailable { reason } => reason,
                _ => "building bombing incomplete drain",
            })];
        };
        match project(&self.rows.rows[..len]) {
            Ok(rows) => [TableView::Complete(TableRows::BuildingBombingDamage(rows))],
            Err(reason) => {
                producer::lost_output(len);
                [unavailable(reason)]
            }
        }
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
            cause: engine::building_damage_sites::Cause::AirNaval,
            amount_raw: 9_000_001,
            credited_raw: 3_000_000,
        };
        let second = producer::Entry {
            id: 12,
            attacker: None,
            native_cause: 99,
            cause: engine::building_damage_sites::Cause::Unknown,
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
            replay::rows::BuildingBombingDamageCause::Unknown
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
}
