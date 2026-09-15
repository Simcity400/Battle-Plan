//! Hourly ship model operands, sharing the authenticated Ships census.
use crate::{sampling::qpc, spooling::table, table_capture::CaptureRows};
use engine::{Memory, cship, walk::Outcome};
use std::cell::Cell;

// Aggregate copy ceiling applies across the entire ship census, including modifier lookups.
const COPY_LIMIT: usize = 128 * 1024 * 1024;
const READ_LIMIT: usize = 4_000_000;
struct BoundedMemory<'a, M> {
    inner: &'a M,
    used: &'a Cell<(usize, usize)>,
}
impl<M: Memory> Memory for BoundedMemory<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let (calls, bytes) = self.used.get();
        if calls >= READ_LIMIT || output.len() > COPY_LIMIT.saturating_sub(bytes) {
            self.used.set((READ_LIMIT, COPY_LIMIT));
            return false;
        }
        self.used.set((calls + 1, bytes + output.len()));
        self.inner.copy(address, output)
    }
}
use replay::{
    frame::TableView,
    rows::{ShipModelRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ShipModels];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self { rows: 4096 }
    }
}

pub struct Captured {
    copied: [Cell<(usize, usize)>; 2],
    rows: CaptureRows<cship::ModelRow>,
    len: [usize; 2],
    missing: [u32; 2],
    failure: [Option<&'static str>; 2],
    second: Outcome,
}

impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            copied: [Cell::new((0, 0)), Cell::new((0, 0))],
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
        self.copied[pass].set((0, 0));
    }
    pub fn visit(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        source: &cship::Row,
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
        let bounded = BoundedMemory {
            inner: memory,
            used: &self.copied[pass],
        };
        let memory = &bounded;
        match cship::read_model(memory, base, pointer, source).and_then(|mut row| {
            row.maximum_organisation = cship::read_maximum_organisation(memory, base, pointer)?;
            Ok(row)
        }) {
            Ok(row) => {
                output[self.len[pass]] = row;
                self.len[pass] += 1;
            }
            Err(reason) => {
                self.failure[pass] = Some(if self.copied[pass].get() == (READ_LIMIT, COPY_LIMIT) {
                    "ShipModels aggregate read budget exceeded"
                } else {
                    reason
                })
            }
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
                    reason: "ship models visits differ from parent census",
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
                reason: "ship models changed during double read",
            }
        } else {
            self.rows.outcome
        }]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.rows.mismatches]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::ShipModels, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| ShipModelRow {
                ship: r.ship,
                variant: r.variant,
                hull_sprite: r.hull_sprite.as_bytes().to_vec(),
                fallback_sprite: r.fallback_sprite.as_bytes().to_vec(),
                model_number: r.model_number,
                formation_flags: r.formation_flags,
                counter: Some(replay::rows::ShipCounterInputs {
                    roster_ordinal: r.roster_ordinal,
                    definition_valid: r.definition_valid,
                    carrier_air_base: r.carrier_air_base,
                    maximum_strength: r.maximum_strength,
                    maximum_organisation: r.maximum_organisation,
                    positive_equipment_flags: r.positive_equipment_flags,
                }),
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::CONSISTENCY_TARGET;
    use engine::memory::SliceMemory;
    #[test]
    fn source_faults_capacity_and_missing_comparison_remain_explicit() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capacity = Capacities { rows: 0 };
        let mut capture = Captured::with_capacities(&capacity, &[0]);
        capture.begin(false);
        capture.visit(&memory, 0, 0, &cship::Row::default(), false);
        capture.finish(
            false,
            Outcome::Truncated {
                len: 1,
                unread_rows: 3,
            },
        );
        assert_eq!(
            capture.outcomes(),
            [Outcome::Truncated {
                len: 0,
                unread_rows: 4
            }]
        );
        capture.grow(&mut capacity, &[0]);
        assert!(capacity.rows >= 4);
        capture.reset();
        capture.begin(false);
        capture.visit(&memory, 0, 0, &cship::Row::default(), false);
        capture.finish(false, Outcome::Rows { len: 1 });
        assert!(matches!(capture.outcomes()[0], Outcome::Unavailable { .. }));
        capture.reset();
        capture.begin(false);
        capture.finish(false, Outcome::Rows { len: 0 });
        assert!(!capture.capture(&memory, 0, 0, &[true]).0[0]);
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.grow(&mut capacity, &[CONSISTENCY_TARGET]);
        assert!(capture.rows.second.is_empty());
    }
    #[test]
    fn model_window_compares_every_operand_and_conversion_preserves_bytes() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(&Capacities { rows: 1 }, &[0]);
        capture.rows.rows[0].ship = 51 | (1 << 32);
        capture.rows.rows[0].variant = 70 | (2 << 32);
        capture.rows.rows[0].hull_sprite.bytes[..3].copy_from_slice(&[65, 0, 255]);
        capture.rows.rows[0].hull_sprite.len = 3;
        capture.rows.rows[0].model_number = i32::MIN;
        capture.rows.rows[0].formation_flags = 1 << 39;
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.second = capture.rows.outcome;
        capture.rows.second[0] = capture.rows.rows[0];
        assert!(capture.capture(&memory, 0, 0, &[true]).0[0]);
        let [TableView::Complete(replay::rows::TableRows::ShipModels(rows))] = capture.tables()
        else {
            panic!("complete");
        };
        assert_eq!(rows[0].hull_sprite, [65, 0, 255]);
        assert_eq!(rows[0].model_number, i32::MIN);
        for field in 0..5 {
            capture.rows.second[0] = capture.rows.rows[0];
            let row = &mut capture.rows.second[0];
            match field {
                0 => row.variant ^= 1 << 32,
                1 => row.hull_sprite.bytes[0] ^= 1,
                2 => {
                    row.fallback_sprite.len = 1;
                    row.fallback_sprite.bytes[0] = 42;
                }
                3 => row.model_number ^= 1,
                _ => row.formation_flags ^= 1 << 7,
            }
            assert!(!capture.capture(&memory, 0, 0, &[true]).0[0]);
            assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        }
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    struct NoReads;
    impl Memory for NoReads {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("copy budget must stop before inner memory access")
        }
    }
    #[test]
    fn whole_census_budget_refuses_bytes_and_calls_before_memory_access() {
        for value in [(READ_LIMIT, 0), (0, COPY_LIMIT)] {
            let used = Cell::new(value);
            let memory = BoundedMemory {
                inner: &NoReads,
                used: &used,
            };
            assert!(!memory.copy(1, &mut [0]));
            assert_eq!(used.get(), (READ_LIMIT, COPY_LIMIT));
        }
    }
}
