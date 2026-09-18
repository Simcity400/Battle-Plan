//! The province-to-`CNavalBase` hash at `CGameState + 0x698`, which the naval
//! base map icon reads for its country, its levels and its own country list.
use crate::{sampling::qpc, spooling::table, table_capture::CaptureRows};
use engine::{Memory, cnaval_base, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{NavalBaseRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::NavalBases];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        // The pinned map holds 556 keys; a mod can add ports, and the capture
        // machinery grows this from the first hour that needs more.
        Self { rows: 1024 }
    }
}

pub struct Captured {
    rows: CaptureRows<cnaval_base::Row>,
}

impl Captured {
    pub fn with_capacities(c: &Capacities, s: &[u16]) -> Self {
        assert_eq!(s.len(), N);
        Self {
            rows: CaptureRows::new(c.rows, s[0]),
        }
    }

    pub fn reset(&mut self) {
        self.rows.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }

    pub fn grow(&mut self, c: &mut Capacities, s: &[u16]) {
        self.rows.grow(&mut c.rows, s[0]);
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        let begin = qpc();
        self.rows.outcome = cnaval_base::walk(memory, base, state, &mut self.rows.rows);
        let matched = if needed[0] {
            self.rows.checked = true;
            let second = cnaval_base::walk(memory, base, state, &mut self.rows.second);
            self.rows.compare(second)
        } else {
            false
        };
        ([matched], [qpc().saturating_sub(begin).max(0) as u64])
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [if self.rows.checked && self.rows.mismatches != 0 {
            Outcome::Unavailable {
                reason: "naval bases changed during double read",
            }
        } else {
            self.rows.outcome
        }]
    }

    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.rows.mismatches]
    }

    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::NavalBases, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| NavalBaseRow {
                province: r.province,
                country: r.country,
                level: r.level,
                healthy_level: r.healthy_level,
                countries: r.countries().to_vec(),
                dropped_countries: r.dropped_countries,
            },
        )]
    }
}
