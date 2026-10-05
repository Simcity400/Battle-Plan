//! The owning hourly game thread drains the bounded hook log before its tick.
//! Conversion and aggregation use only frame-owned rows on the spool thread.
use crate::{
    equipment_destruction as producer,
    sampling::qpc,
    spooling::unavailable,
    table_capture::{CaptureRows, grow_vec},
};
use engine::{Memory, allocation, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{EquipmentDestructionRow, TableId, TableRows},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::EquipmentDestruction];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];
pub type Capacities = usize;
pub struct Captured {
    rows: CaptureRows<producer::Entry>,
}
impl Captured {
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
        *c = (*c).max(producer::capacity());
        grow_vec(&mut self.rows.rows, *c);
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
                _ => "equipment destruction incomplete drain",
            };
            return [unavailable(reason)];
        };
        match aggregate(&self.rows.rows[..len]) {
            Ok(rows) => [TableView::Complete(TableRows::EquipmentDestruction(rows))],
            Err(reason) => {
                producer::lost_drain();
                [unavailable(reason)]
            }
        }
    }
}

fn aggregate(events: &[producer::Entry]) -> Result<Vec<EquipmentDestructionRow>, &'static str> {
    let mut rows = allocation::collect_vec(events.iter().map(|r| EquipmentDestructionRow {
        country: r.country,
        cause: r.cause.into(),
        variant: r.variant,
        amount_raw: r.amount_raw,
    }))
    .map_err(|_| "collector allocation failed")?;
    rows.sort_unstable_by_key(EquipmentDestructionRow::key);
    let mut write = 0usize;
    for read in 0..rows.len() {
        if rows[read].amount_raw <= 0 {
            return Err("equipment destruction nonpositive event");
        }
        if write > 0 && rows[write - 1].key() == rows[read].key() {
            rows[write - 1].amount_raw = rows[write - 1]
                .amount_raw
                .checked_add(rows[read].amount_raw)
                .ok_or("equipment destruction hourly total overflow")?;
        } else {
            rows[write] = rows[read].clone();
            write += 1;
        }
    }
    rows.truncate(write);
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::EquipmentDestructionCause;
    #[test]
    fn hourly_groups_preserve_all_owners_causes_and_full_designs() {
        let event = producer::Entry {
            country: 4999,
            cause: engine::equipment_destruction::Cause::Attrition,
            variant: 0xf00100000046,
            amount_raw: 9,
        };
        let other = producer::Entry {
            country: 1500,
            ..event
        };
        let rows = aggregate(&[
            event,
            other,
            event,
            producer::Entry {
                cause: engine::equipment_destruction::Cause::Unknown,
                ..event
            },
        ])
        .unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].country, 1500);
        assert_eq!(rows[1].cause, EquipmentDestructionCause::Unknown);
        assert_eq!(rows[2].cause, EquipmentDestructionCause::Attrition);
        assert_eq!(rows[2].amount_raw, 18);
        assert!(
            aggregate(&[
                producer::Entry {
                    amount_raw: i64::MAX,
                    ..event
                },
                event
            ])
            .is_err()
        );
        assert!(
            aggregate(&[producer::Entry {
                amount_raw: 0,
                ..event
            }])
            .is_err()
        );
        assert!(aggregate(&[]).unwrap().is_empty());
    }
}
