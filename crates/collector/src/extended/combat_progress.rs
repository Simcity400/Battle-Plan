//! Hourly combat-icon operands, sharing the existing land-combat census.
use crate::{sampling::qpc, spooling::table, table_capture::CaptureRows};
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
    len: [usize; 2],
    missing: [u32; 2],
    failure: [Option<&'static str>; 2],
    second: Outcome,
}

impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            rows: CaptureRows::new(capacity.rows, successful[0]),
            len: [0; 2],
            missing: [0; 2],
            failure: [None; 2],
            second: super::MISSING,
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.len = [0; 2];
        self.missing = [0; 2];
        self.failure = [None; 2];
        self.second = super::MISSING;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        assert_eq!(successful.len(), N);
        self.rows.grow(&mut capacity.rows, successful[0]);
    }
    pub fn begin(&mut self, second: bool) {
        let pass = usize::from(second);
        self.len[pass] = 0;
        self.missing[pass] = 0;
        self.failure[pass] = None;
    }
    pub fn visit(
        &mut self,
        memory: &impl Memory,
        base: usize,
        source: cland_combat::Source,
        second: bool,
    ) {
        let pass = usize::from(second);
        if self.failure[pass].is_some() {
            return;
        }
        let output = if second {
            &mut self.rows.second
        } else {
            &mut self.rows.rows
        };
        if self.len[pass] == output.len() {
            self.missing[pass] = self.missing[pass].saturating_add(1);
            return;
        }
        match cland_combat::read_progress(memory, base, source) {
            Ok(row) => {
                output[self.len[pass]] = row;
                self.len[pass] += 1;
            }
            Err(reason) => self.failure[pass] = Some(reason),
        }
    }
    pub fn finish(&mut self, second: bool, parent: Outcome) {
        let pass = usize::from(second);
        let missing = self.missing[pass].saturating_add(match parent {
            Outcome::Truncated { unread_rows, .. } => unread_rows,
            _ => 0,
        });
        let result = match (self.failure[pass], parent) {
            (Some(reason), _) | (_, Outcome::Unavailable { reason }) => {
                Outcome::Unavailable { reason }
            }
            _ if self.len[pass].saturating_add(self.missing[pass] as usize) != parent.len() => {
                Outcome::Unavailable {
                    reason: "combat progress visits differ from parent census",
                }
            }
            _ if missing != 0 => Outcome::Truncated {
                len: self.len[pass],
                unread_rows: missing,
            },
            _ => Outcome::Rows {
                len: self.len[pass],
            },
        };
        if second {
            self.second = result;
        } else {
            self.rows.outcome = result;
        }
    }
    pub fn capture(
        &mut self,
        _memory: &impl Memory,
        _base: usize,
        _state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        let started = qpc();
        self.rows.checked = needed[0];
        let matched = needed[0] && self.rows.compare(self.second);
        ([matched], [qpc().saturating_sub(started).max(0) as u64])
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [if self.rows.checked && self.rows.mismatches != 0 {
            Outcome::Unavailable {
                reason: "combat progress changed during double read",
            }
        } else {
            self.rows.outcome
        }]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.rows.mismatches]
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
        let mut captured = Captured::with_capacities(&Capacities { rows: 1 }, &[0]);
        captured.begin(false);
        captured.visit(&Unreadable, 0, source(62), false);
        captured.visit(&Unreadable, 0, source(63), false);
        captured.finish(
            false,
            Outcome::Truncated {
                len: 2,
                unread_rows: 3,
            },
        );
        assert_eq!(
            captured.outcomes(),
            [Outcome::Truncated {
                len: 1,
                unread_rows: 4
            }]
        );
        let mut capacity = Capacities { rows: 1 };
        captured.grow(&mut capacity, &[0]);
        assert!(capacity.rows >= 5);
        captured.reset();
        captured.finish(false, Outcome::Rows { len: 1 });
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
        captured.reset();
        captured.begin(false);
        captured.visit(&Unreadable, 0, source(62), false);
        captured.finish(
            false,
            Outcome::Unavailable {
                reason: "parent census failed after prefix",
            },
        );
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
        captured.reset();
        captured.begin(false);
        let mut invalid = source(62);
        invalid.attackers = Array {
            data: 0x1000,
            count: 1,
        };
        captured.visit(&Unreadable, 0, invalid, false);
        captured.finish(false, Outcome::Rows { len: 1 });
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn independent_operand_mismatch_poisoning_and_comparison_retirement() {
        let mut captured = Captured::with_capacities(&Capacities { rows: 1 }, &[0]);
        for second in [false, true] {
            captured.begin(second);
            captured.visit(&Unreadable, 0, source(62), second);
            captured.finish(second, Outcome::Rows { len: 1 });
        }
        assert_eq!(captured.capture(&Unreadable, 0, 0, &[true]).0, [true]);
        captured.rows.second[0].attacker_progress -= 1;
        assert_eq!(captured.capture(&Unreadable, 0, 0, &[true]).0, [false]);
        assert_eq!(captured.checked_mismatches(), [1]);
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
        captured.reset();
        captured.grow(
            &mut Capacities { rows: 1 },
            &[crate::sampling::CONSISTENCY_TARGET],
        );
        assert!(captured.rows.second.is_empty());
        captured.begin(false);
        captured.visit(&Unreadable, 0, source(62), false);
        captured.finish(false, Outcome::Rows { len: 1 });
        assert_eq!(captured.capture(&Unreadable, 0, 0, &[false]).0, [false]);
        assert_eq!(captured.outcomes(), [Outcome::Rows { len: 1 }]);
        let [TableView::Complete(replay::rows::TableRows::LandCombatProgress(rows))] =
            captured.tables()
        else {
            panic!("complete combat progress row lost during conversion")
        };
        assert_eq!(rows[0].attacker_progress, 50_000);
        assert_eq!(rows[0].defender_progress, 50_000);
    }
}
