//! Hourly exact native leader portrait strings.
use crate::{sampling::qpc, spooling::table, table_capture::CaptureRows};
use engine::{Memory, cunit_leader, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{LeaderPortraitRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::LeaderPortraits];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self { rows: 8192 }
    }
}

pub struct Captured {
    rows: CaptureRows<cunit_leader::PortraitRow>,
}
impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            rows: CaptureRows::new(capacity.rows, successful[0]),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        self.rows.grow(&mut capacity.rows, successful[0]);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        _state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        let begin = qpc();
        self.rows.outcome = cunit_leader::walk_portraits(memory, base, &mut self.rows.rows);
        self.rows.checked = needed[0];
        let matched = if needed[0] {
            let second = cunit_leader::walk_portraits(memory, base, &mut self.rows.second);
            self.rows.compare(second)
        } else {
            false
        };
        ([matched], [qpc().saturating_sub(begin).max(0) as u64])
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [if self.rows.checked && self.rows.mismatches != 0 {
            Outcome::Unavailable {
                reason: "leader portraits changed during double read",
            }
        } else {
            self.rows.outcome
        }]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.rows.mismatches]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::LeaderPortraits, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| LeaderPortraitRow {
                leader: r.leader,
                character: r.character,
                portrait_c0: r.strings[0].as_bytes().to_vec(),
                portrait_e0: r.strings[1].as_bytes().to_vec(),
                portrait_100: r.strings[2].as_bytes().to_vec(),
            },
        )]
    }
}
