//! Hourly COrdersGroup withdrawing byte over its own bounded registry census.
use crate::{
    sampling::qpc,
    spooling::table,
    table_capture::{CaptureRows, grow_capacity, grow_vec},
};
use engine::{Memory, corders_group, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{OrdersGroupCounterInputRow, TableId, tables},
};
use std::mem::size_of;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::OrdersGroupCounterInputs];
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
    rows: CaptureRows<corders_group::CounterInputsRow>,
    roots: [Vec<corders_group::Root>; 2],
    required: usize,
}
impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            rows: CaptureRows::new(capacity.rows, successful[0]),
            roots: [
                vec![Default::default(); capacity.rows],
                vec![Default::default(); capacity.rows],
            ],
            required: 0,
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.required = 0;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + (self.roots[0].capacity() + self.roots[1].capacity())
                * size_of::<corders_group::Root>()
    }
    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        grow_capacity(&mut capacity.rows, self.required);
        self.rows.grow(&mut capacity.rows, successful[0]);
        for roots in &mut self.roots {
            grow_vec(roots, capacity.rows);
        }
    }
    fn read(&mut self, memory: &impl Memory, base: usize, second: bool) -> Outcome {
        let pass = usize::from(second);
        let source = corders_group::walk_roots(memory, base, &mut self.roots[pass]);
        if let Outcome::Truncated { len, unread_rows } = source {
            self.required = self.required.max(len + unread_rows as usize);
        }
        let output = if second {
            &mut self.rows.second
        } else {
            &mut self.rows.rows
        };
        corders_group::walk_counter_inputs(memory, base, &self.roots[pass], source, output)
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
        self.rows.outcome = self.read(memory, base, false);
        self.rows.checked = needed[0];
        let matched = if needed[0] {
            let second = self.read(memory, base, true);
            self.rows.compare(second)
        } else {
            false
        };
        // Root pointers have no purpose after acquisition.
        for roots in &mut self.roots {
            roots.fill(Default::default());
        }
        ([matched], [qpc().saturating_sub(begin).max(0) as u64])
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [if self.rows.checked && self.rows.mismatches != 0 {
            Outcome::Unavailable {
                reason: "orders group counter inputs changed during double read",
            }
        } else {
            self.rows.outcome
        }]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.rows.mismatches]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::OrdersGroupCounterInputs, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| OrdersGroupCounterInputRow {
                group: r.group,
                withdrawing: r.withdrawing,
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;
    #[test]
    fn missing_registry_is_unavailable_and_raw_bytes_participate_in_the_window() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capacity = Capacities { rows: 1 };
        let mut capture = Captured::with_capacities(&capacity, &[0]);
        assert!(!capture.capture(&memory, 0, 0, &[true]).0[0]);
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.reset();
        capture.rows.rows[0] = corders_group::CounterInputsRow {
            group: (5 << 32) | 53,
            withdrawing: 255,
        };
        capture.rows.second[0] = capture.rows.rows[0];
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.rows.checked = true;
        assert!(capture.rows.compare(Outcome::Rows { len: 1 }));
        capture.rows.second[0].withdrawing = 1;
        assert!(!capture.rows.compare(Outcome::Rows { len: 1 }));
        capture.required = 9;
        capture.grow(&mut capacity, &[0]);
        assert!(capacity.rows >= 9 && capture.roots[0].len() >= 9);
    }
}
