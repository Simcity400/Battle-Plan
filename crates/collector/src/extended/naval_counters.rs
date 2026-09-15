//! Hourly task-force mission/link operands and fleet colour, region count and ordered roster.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_capacity},
};
use engine::{Memory, cfleet, ctask_force, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{FleetCounterInputRow, TableId, TaskForceCounterInputRow, tables},
};

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::TaskForceCounterInputs, TableId::FleetCounterInputs];
/// One country/fleet/task-force traversal serves both tables; its time is charged to task forces.
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("fleet and task-force counter operands", 0, &[0, 1])];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    task_forces: usize,
    fleets: usize,
    rosters: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            task_forces: 4096,
            fleets: 1024,
            rosters: 4096,
        }
    }
}

pub struct Captured {
    task_forces: CaptureRows<ctask_force::CounterRow>,
    fleets: CaptureRows<cfleet::CounterRow>,
    rosters: Arena<u64>,
}
impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            task_forces: CaptureRows::new(capacity.task_forces, successful[0]),
            fleets: CaptureRows::new(capacity.fleets, successful[1]),
            rosters: Arena::new(capacity.rosters, successful[1]),
        }
    }
    pub fn reset(&mut self) {
        self.task_forces.reset();
        self.fleets.reset();
        self.rosters.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.task_forces.allocated_bytes()
            + self.fleets.allocated_bytes()
            + self.rosters.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        assert_eq!(successful.len(), N);
        self.task_forces
            .grow(&mut capacity.task_forces, successful[0]);
        self.fleets.grow(&mut capacity.fleets, successful[1]);
        grow_capacity(&mut capacity.rosters, self.rosters.required);
        self.rosters.grow(&mut capacity.rosters, successful[1]);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        let begin = qpc();
        let first = cfleet::walk_counter_inputs(
            memory,
            base,
            state,
            &mut self.fleets.rows,
            &mut self.rosters.first,
            &mut self.task_forces.rows,
        );
        self.task_forces.outcome = first.task_forces;
        self.fleets.outcome = first.fleets;
        self.rosters.len = first.roster_len;
        self.rosters.required = first.roster_required;
        self.task_forces.checked = needed[0];
        self.fleets.checked = needed[1];
        let mut matched = [false; N];
        if needed.iter().any(|v| *v) {
            let second = cfleet::walk_counter_inputs(
                memory,
                base,
                state,
                &mut self.fleets.second,
                &mut self.rosters.second,
                &mut self.task_forces.second,
            );
            if needed[0] {
                matched[0] = self.task_forces.compare(second.task_forces);
            }
            if needed[1] {
                let rows = self.fleets.compare(second.fleets);
                let arena = self
                    .rosters
                    .compare(second.roster_len, second.roster_required);
                if !arena {
                    self.fleets.mismatches += 1;
                }
                matched[1] = rows && arena;
            }
        }
        let ticks = qpc().saturating_sub(begin).max(0) as u64;
        (matched, [ticks, 0])
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            if self.task_forces.checked && self.task_forces.mismatches != 0 {
                Outcome::Unavailable {
                    reason: "task force counter inputs changed during double read",
                }
            } else {
                self.task_forces.outcome
            },
            if self.fleets.checked && self.fleets.mismatches != 0 {
                Outcome::Unavailable {
                    reason: "fleet counter inputs changed during double read",
                }
            } else {
                self.fleets.outcome
            },
        ]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.task_forces.mismatches, self.fleets.mismatches]
    }
    pub fn tables(&self) -> [TableView; N] {
        let [tasks, fleets] = self.outcomes();
        let Some(rosters) = self.rosters.first.get(..self.rosters.len) else {
            return [
                table::<tables::TaskForceCounterInputs, _>(tasks, &self.task_forces.rows, task),
                unavailable("fleet roster arena length invalid"),
            ];
        };
        let fleet_rows = &self.fleets.rows[..fleets.len().min(self.fleets.rows.len())];
        if fleet_rows
            .iter()
            .any(|r| r.start.checked_add(r.len).is_none_or(|end| end > rosters.len()))
        {
            return [
                table::<tables::TaskForceCounterInputs, _>(tasks, &self.task_forces.rows, task),
                unavailable("fleet roster lies outside captured arena"),
            ];
        }
        [
            table::<tables::TaskForceCounterInputs, _>(tasks, &self.task_forces.rows, task),
            table::<tables::FleetCounterInputs, _>(fleets, &self.fleets.rows, |r| {
                FleetCounterInputRow {
                    fleet: r.id,
                    red: r.rgba[0],
                    green: r.rgba[1],
                    blue: r.rgba[2],
                    alpha: r.rgba[3],
                    mission_region_count: r.mission_region_count,
                    task_forces: rosters[r.start..r.start + r.len].to_vec(),
                }
            }),
        ]
    }
}

fn task(r: &ctask_force::CounterRow) -> TaskForceCounterInputRow {
    TaskForceCounterInputRow {
        task_force: r.id,
        is_in_regions: r.is_in_regions,
        mission_byte_391: r.mission_byte_391,
        engagement_rule: r.engagement_rule,
        mission_region_count: r.mission_region_count,
        bombardment_region: r.bombardment_region,
        repair_parent: r.repair_parent,
        detached_activity: r.detached_activity,
        detached_mission: r.detached_mission,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;
    #[test]
    fn unreadable_countries_fail_both_tables_and_rosters_convert_in_native_order() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(
            &Capacities {
                task_forces: 1,
                fleets: 1,
                rosters: 2,
            },
            &[0, 0],
        );
        assert_eq!(capture.capture(&memory, 0, 0, &[true, true]).0, [false, false]);
        assert!(
            capture
                .tables()
                .iter()
                .all(|t| matches!(t, TableView::Unavailable { .. }))
        );
        capture.reset();
        capture.fleets.rows[0] = cfleet::CounterRow {
            id: (1 << 32) | 61,
            rgba: [0x3f80_0000, 0, 0x8000_0000, u32::MAX],
            mission_region_count: 2,
            start: 0,
            len: 2,
        };
        capture.rosters.first[..2].copy_from_slice(&[(54 << 32) | 51, (43 << 32) | 51]);
        capture.rosters.len = 2;
        capture.fleets.outcome = Outcome::Rows { len: 1 };
        capture.task_forces.outcome = Outcome::Rows { len: 0 };
        let [_, TableView::Complete(replay::rows::TableRows::FleetCounterInputs(rows))] =
            capture.tables()
        else {
            panic!("complete");
        };
        assert_eq!(rows[0].task_forces, [(54 << 32) | 51, (43 << 32) | 51]);
        assert_eq!(rows[0].blue, 0x8000_0000);
        capture.fleets.rows[0].len = 3;
        assert!(matches!(capture.tables()[1], TableView::Unavailable { .. }));
    }
}
