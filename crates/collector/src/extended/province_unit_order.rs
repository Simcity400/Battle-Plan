//! Hourly ordered native inputs with independently bounded owned payloads.
use crate::{
    sampling::qpc,
    spooling::table,
    table_capture::{Arena, CaptureRows},
};
use engine::{Memory, cprovince, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{ProvinceUnitOrderRow, TableId, tables},
};
pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ProvinceUnitOrder];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    arena: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 16384,
            arena: 32768,
        }
    }
}
pub struct Captured {
    rows: CaptureRows<cprovince::UnitOrderRow>,
    arena: Arena<u64>,
}
impl Captured {
    pub fn with_capacities(c: &Capacities, s: &[u16]) -> Self {
        assert_eq!(s.len(), N);
        Self {
            rows: CaptureRows::new(c.rows, s[0]),
            arena: Arena::new(c.arena, s[0]),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.arena.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.arena.allocated_bytes()
    }
    pub fn grow(&mut self, c: &mut Capacities, s: &[u16]) {
        self.rows.grow(&mut c.rows, s[0]);
        self.arena.grow(&mut c.arena, s[0]);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        let begin = qpc();
        let first = cprovince::walk_unit_order(
            memory,
            base,
            state,
            &mut self.rows.rows,
            &mut self.arena.first,
        );
        self.rows.outcome = first.outcome;
        self.rows.checked = needed[0];
        self.arena.len = first.len;
        self.arena.required = first.required;
        let matched = if needed[0] {
            let second = cprovince::walk_unit_order(
                memory,
                base,
                state,
                &mut self.rows.second,
                &mut self.arena.second,
            );
            let rows = self.rows.compare(second.outcome);
            let arena = self.arena.compare(second.len, second.required);
            if !arena {
                self.rows.mismatches += 1;
            }
            rows && arena
        } else {
            false
        };
        ([matched], [qpc().saturating_sub(begin).max(0) as u64])
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [if self.rows.checked && self.rows.mismatches != 0 {
            Outcome::Unavailable {
                reason: "province_unit_order changed during double read",
            }
        } else {
            self.rows.outcome
        }]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.rows.mismatches]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::ProvinceUnitOrder, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| ProvinceUnitOrderRow {
                province: r.province,
                mixed: self.arena.first[r.mixed[0]..r.mixed[0] + r.mixed[1]].to_vec(),
                naval: self.arena.first[r.naval[0]..r.naval[0] + r.naval[1]].to_vec(),
            },
        )]
    }
}
