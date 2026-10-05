//! Hourly COrdersGroup withdrawing byte over its own bounded registry census.
use crate::{
    sampling::qpc,
    spooling::table,
    table_capture::{CaptureRows, grow_capacity, grow_vec},
};
use engine::allocation::{default_vec, owned_vec};
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
    roots: Vec<corders_group::Root>,
    required: usize,
}
impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities { rows: 0 })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            roots: owned_vec(default_vec(capacity.rows)),
            required: 0,
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.required = 0;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.roots.capacity() * size_of::<corders_group::Root>()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        grow_capacity(&mut capacity.rows, self.required);
        self.rows.grow(&mut capacity.rows);
        grow_vec(&mut self.roots, capacity.rows);
    }
    fn read(&mut self, memory: &impl Memory, base: usize) -> Outcome {
        let source = corders_group::walk_roots(memory, base, &mut self.roots);
        if let Outcome::Truncated { len, unread_rows } = source {
            self.required = self.required.max(len + unread_rows as usize);
        }
        corders_group::walk_counter_inputs(memory, base, &self.roots, source, &mut self.rows.rows)
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, _state: usize) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        self.rows.outcome = self.read(memory, base);
        // Root pointers have no purpose after acquisition.
        self.roots.fill(Default::default());
        [qpc().saturating_sub(begin).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
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
    fn missing_registry_and_capacity_pressure_preserve_primary_rows() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capacity = Capacities { rows: 1 };
        let mut capture = Captured::with_capacities(&capacity);
        capture.capture(&memory, 0, 0);
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.reset();
        capture.rows.rows[0] = corders_group::CounterInputsRow {
            group: (5 << 32) | 53,
            withdrawing: 255,
        };
        capture.rows.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(replay::rows::TableRows::OrdersGroupCounterInputs(rows))] =
            capture.tables()
        else {
            panic!("complete primary row");
        };
        assert_eq!(rows[0].group, (5 << 32) | 53);
        assert_eq!(rows[0].withdrawing, 255);
        capture.required = 9;
        capture.grow(&mut capacity);
        assert!(capacity.rows >= 9 && capture.roots.len() >= 9);
    }
}
