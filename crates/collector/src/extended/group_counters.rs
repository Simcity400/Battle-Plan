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
    rows::{OrdersGroupCounterInputRow, Recorded, TableId, tables},
};
use std::mem::size_of;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::OrdersGroupCounterInputs];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

static COOLDOWN_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static BORDER_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static INVASION_DISTANCE_REFUSALS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

pub(crate) fn tooltip_data() -> impl serde::Serialize {
    crate::status::status!({
        "leader_enroute_days_refused": COOLDOWN_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "border_war_presence_refused": BORDER_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "invasion_source_distance_refused": INVASION_DISTANCE_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "scope": "cumulative independent orders-group tooltip input refusals; existing withdrawing and HQ-distance operands remain available",
    })
}

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
    fn read(&mut self, memory: &impl Memory, base: usize, state: usize) -> Outcome {
        let source = corders_group::walk_roots(memory, base, &mut self.roots);
        if let Outcome::Truncated { len, unread_rows } = source {
            self.required = self.required.max(len + unread_rows as usize);
        }
        corders_group::walk_counter_inputs_in_state(
            memory,
            base,
            state,
            &self.roots,
            source,
            &mut self.rows.rows,
        )
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        self.rows.outcome = self.read(memory, base, state);
        let rows = &self.rows.rows[..self.rows.outcome.len()];
        COOLDOWN_REFUSALS.fetch_add(
            rows.iter()
                .filter(|row| row.leader_enroute_days.is_none())
                .count() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        BORDER_REFUSALS.fetch_add(
            rows.iter()
                .filter(|row| row.border_war_presence.is_none())
                .count() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        INVASION_DISTANCE_REFUSALS.fetch_add(
            rows.iter()
                .filter(|row| row.invasion_source_distance.is_none())
                .count() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        // Root pointers have no purpose after acquisition.
        self.roots.fill(Default::default());
        [qpc().saturating_sub(begin).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }

    pub(crate) fn observe_counter_inputs(
        &self,
        work: &mut super::division_presentation::CounterWork,
    ) {
        for row in &self.rows.rows[..self.rows.outcome.len()] {
            work.observe_hq(row.hq_distance);
        }
    }

    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::OrdersGroupCounterInputs, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| OrdersGroupCounterInputRow {
                group: r.group,
                withdrawing: r.withdrawing,
                hq_distance: Recorded::Value(r.hq_distance),
                leader_enroute_days: Recorded::Value(r.leader_enroute_days),
                border_war_presence: Recorded::Value(r.border_war_presence),
                invasion_source_distance: Recorded::Value(r.invasion_source_distance),
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
            hq_distance: Some(i32::MIN),
            leader_enroute_days: Some(-3),
            border_war_presence: Some(1),
            invasion_source_distance: Some(-17),
        };
        capture.rows.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(replay::rows::TableRows::OrdersGroupCounterInputs(rows))] =
            capture.tables()
        else {
            panic!("complete primary row");
        };
        assert_eq!(rows[0].group, (5 << 32) | 53);
        assert_eq!(rows[0].withdrawing, 255);
        assert_eq!(rows[0].hq_distance, Recorded::Value(Some(i32::MIN)));
        assert_eq!(rows[0].leader_enroute_days, Recorded::Value(Some(-3)));
        assert_eq!(rows[0].border_war_presence, Recorded::Value(Some(1)));
        assert_eq!(rows[0].invasion_source_distance, Recorded::Value(Some(-17)));
        let mut work = super::super::division_presentation::CounterWork::default();
        capture.observe_counter_inputs(&mut work);
        let status = serde_json::to_value(work.status()).unwrap();
        assert_eq!(status["hq_distance"]["captured"], 1);
        capture.rows.rows[0].hq_distance = None;
        capture.observe_counter_inputs(&mut work);
        let status = serde_json::to_value(work.status()).unwrap();
        assert_eq!(status["hq_distance"]["refused"], 1);
        capture.required = 9;
        capture.grow(&mut capacity);
        assert!(capacity.rows >= 9 && capture.roots.len() >= 9);
    }
}
