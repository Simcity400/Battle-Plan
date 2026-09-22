//! Hourly combat-icon operands, sharing the existing land-combat census.
use crate::{spooling::table, table_capture::CaptureRows};
use engine::{Memory, cland_combat, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{LandCombatProgressRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::LandCombatProgress];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[(
    "land combat progress operands (census charged to LandCombats)",
    0,
    &[0],
)];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self { rows: 512 }
    }
}

pub struct Captured {
    rows: CaptureRows<cland_combat::ProgressRow>,
    len: usize,
    missing: u32,
    failure: Option<&'static str>,
}

impl Captured {
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            len: 0,
            missing: 0,
            failure: None,
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.len = 0;
        self.missing = 0;
        self.failure = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
    }
    pub fn begin(&mut self) {
        self.len = 0;
        self.missing = 0;
        self.failure = None;
    }
    pub fn visit(&mut self, memory: &impl Memory, base: usize, source: cland_combat::Source) {
        let _reads = crate::read_accounting::table(TABLES[0]);
        if self.failure.is_some() {
            return;
        }
        if self.len == self.rows.rows.len() {
            self.missing = self.missing.saturating_add(1);
            return;
        }
        match cland_combat::read_progress(memory, base, source) {
            Ok(row) => {
                self.rows.rows[self.len] = row;
                self.len += 1;
            }
            Err(reason) => self.failure = Some(reason),
        }
    }
    pub fn finish(&mut self, parent: Outcome) {
        let missing = self.missing.saturating_add(match parent {
            Outcome::Truncated { unread_rows, .. } => unread_rows,
            _ => 0,
        });
        self.rows.outcome = match (self.failure, parent) {
            (Some(reason), _) | (_, Outcome::Unavailable { reason }) => {
                Outcome::Unavailable { reason }
            }
            _ if self.len.saturating_add(self.missing as usize) != parent.len() => {
                Outcome::Unavailable {
                    reason: "combat progress visits differ from parent census",
                }
            }
            _ if missing != 0 => Outcome::Truncated {
                len: self.len,
                unread_rows: missing,
            },
            _ => Outcome::Rows { len: self.len },
        };
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::LandCombatProgress, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| LandCombatProgressRow {
                combat: r.id,
                attacker_progress: r.attacker_progress,
                defender_progress: r.defender_progress,
                direction_attacker: r.direction_attacker,
                direction_province: r.direction_province,
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::cpdx_array::Array;

    struct Unreadable;
    impl Memory for Unreadable {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
    }
    fn source(id: u64) -> cland_combat::Source {
        cland_combat::Source {
            pointer: 0,
            id,
            attackers: Array { data: 0, count: 0 },
            defenders: Array { data: 0, count: 0 },
            provinces: Array { data: 0, count: 0 },
        }
    }

    #[test]
    fn shared_census_faults_and_capacity_never_become_complete() {
        let mut captured = Captured::with_capacities(&Capacities { rows: 1 });
        captured.begin();
        captured.visit(&Unreadable, 0, source(62));
        captured.visit(&Unreadable, 0, source(63));
        captured.finish(Outcome::Truncated {
            len: 2,
            unread_rows: 3,
        });
        assert_eq!(
            captured.outcomes(),
            [Outcome::Truncated {
                len: 1,
                unread_rows: 4
            }]
        );
        let mut capacity = Capacities { rows: 1 };
        captured.grow(&mut capacity);
        assert!(capacity.rows >= 5);
        captured.reset();
        captured.finish(Outcome::Rows { len: 1 });
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
        captured.reset();
        captured.begin();
        captured.visit(&Unreadable, 0, source(62));
        captured.finish(Outcome::Unavailable {
            reason: "parent census failed after prefix",
        });
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
        captured.reset();
        captured.begin();
        let mut invalid = source(62);
        invalid.attackers = Array {
            data: 0x1000,
            count: 1,
        };
        captured.visit(&Unreadable, 0, invalid);
        captured.finish(Outcome::Rows { len: 1 });
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn primary_operand_rows_preserve_native_progress_values() {
        let mut captured = Captured::with_capacities(&Capacities { rows: 1 });
        captured.begin();
        captured.visit(&Unreadable, 0, source(62));
        captured.finish(Outcome::Rows { len: 1 });
        assert_eq!(captured.outcomes(), [Outcome::Rows { len: 1 }]);
        let [TableView::Complete(replay::rows::TableRows::LandCombatProgress(rows))] =
            captured.tables()
        else {
            panic!("complete combat progress row lost during conversion");
        };
        assert_eq!(rows[0].combat, 62);
        assert_eq!(rows[0].attacker_progress, 50_000);
        assert_eq!(rows[0].defender_progress, 50_000);
    }
}
