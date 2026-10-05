//! Hourly task-force mission, link and supply operands, fleet colour, region count and ordered
//! roster, each task force's movement operands and its registered mission regions, all from
//! one fleet walk.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_capacity},
};
use engine::allocation::{collect_vec, copy_vec, owned_vec};
use engine::{Memory, cfleet, ctask_force, cunit::FirstCombatMemo, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{
        FleetCounterInputRow, TableId, TaskForceCounterInputRow, TaskForceMapMotionRow,
        TaskForceMissionRow, tables,
    },
};

pub const N: usize = 4;
pub const TABLES: [TableId; N] = [
    TableId::TaskForceCounterInputs,
    TableId::FleetCounterInputs,
    TableId::TaskForceMapMotion,
    TableId::TaskForceMissions,
];
/// One country/fleet/task-force traversal serves all four tables; its time is charged to task
/// forces.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[(
    "fleet and task-force counter, motion and mission operands",
    0,
    &[0, 1, 2, 3],
)];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    task_forces: usize,
    fleets: usize,
    rosters: usize,
    motion: usize,
    paths: usize,
    missions: usize,
    regions: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            task_forces: 4096,
            fleets: 1024,
            rosters: 4096,
            motion: 4096,
            paths: 4096,
            missions: 2048,
            regions: 8192,
        }
    }
}

pub struct Captured {
    task_forces: CaptureRows<ctask_force::CounterRow>,
    fleets: CaptureRows<cfleet::CounterRow>,
    rosters: Arena<u64>,
    motion: CaptureRows<ctask_force::MapMotionRow>,
    paths: Arena<u32>,
    /// First-combat answers for this capture's task forces. The fleet walk runs apart from the
    /// division and railway-gun captures, so it keeps its own memo, sized off-thread.
    combats: FirstCombatMemo,
    missions: CaptureRows<ctask_force::MissionRow>,
    regions: Arena<u32>,
}
impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            task_forces: 0,
            fleets: 0,
            rosters: 0,
            motion: 0,
            paths: 0,
            missions: 0,
            regions: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            task_forces: CaptureRows::new(capacity.task_forces),
            fleets: CaptureRows::new(capacity.fleets),
            rosters: Arena::new(capacity.rosters),
            motion: CaptureRows::new(capacity.motion),
            paths: Arena::new(capacity.paths),
            combats: FirstCombatMemo::try_new(capacity.motion)
                .map_err(crate::allocation::io)
                .unwrap_or_default(),
            missions: CaptureRows::new(capacity.missions),
            regions: Arena::new(capacity.regions),
        }
    }
    pub fn reset(&mut self) {
        self.task_forces.reset();
        self.fleets.reset();
        self.rosters.reset();
        self.motion.reset();
        self.paths.reset();
        self.combats.clear();
        self.missions.reset();
        self.regions.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.task_forces.allocated_bytes()
            + self.fleets.allocated_bytes()
            + self.rosters.allocated_bytes()
            + self.motion.allocated_bytes()
            + self.paths.allocated_bytes()
            + self.combats.allocated_bytes()
            + self.missions.allocated_bytes()
            + self.regions.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.task_forces.grow(&mut capacity.task_forces);
        self.fleets.grow(&mut capacity.fleets);
        grow_capacity(&mut capacity.rosters, self.rosters.required);
        self.rosters.grow(&mut capacity.rosters);
        self.motion.grow(&mut capacity.motion);
        grow_capacity(&mut capacity.paths, self.paths.required);
        self.paths.grow(&mut capacity.paths);
        if self
            .combats
            .reserve(capacity.motion)
            .map_err(crate::allocation::io)
            .is_err()
        {
            return;
        }
        self.missions.grow(&mut capacity.missions);
        grow_capacity(&mut capacity.regions, self.regions.required);
        self.regions.grow(&mut capacity.regions);
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        // Answers are valid for one capture only; clearing keeps the reserved room.
        self.combats.clear();
        let mut motion = ctask_force::MapMotionCollector::new(
            memory,
            base,
            state,
            &mut self.motion.rows,
            &mut self.paths.first,
            &mut self.combats,
        );
        let mut missions = ctask_force::MissionCollector::new(
            memory,
            base,
            state,
            &mut self.missions.rows,
            &mut self.regions.first,
        );
        let captured = cfleet::walk_counter_inputs(
            memory,
            base,
            state,
            &mut self.fleets.rows,
            &mut self.rosters.first,
            &mut self.task_forces.rows,
            |pointer, id| {
                motion.visit(pointer, id);
                missions.visit(pointer, id);
            },
        );
        // A walk that fails leaves the motion and mission censuses unknown too.
        let failed = match captured.task_forces {
            Outcome::Unavailable { reason } => Some(reason),
            _ => None,
        };
        let walked = motion.finish(failed);
        let registered = missions.finish(failed);
        self.missions.outcome = registered.outcome;
        self.regions.len = registered.regions_len;
        self.regions.required = registered.required_regions;
        self.task_forces.outcome = captured.task_forces;
        self.fleets.outcome = captured.fleets;
        self.rosters.len = captured.roster_len;
        self.rosters.required = captured.roster_required;
        self.motion.outcome = walked.outcome;
        self.paths.len = walked.paths_len;
        self.paths.required = walked.required_paths;
        [qpc().saturating_sub(begin).max(0) as u64, 0, 0, 0]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.task_forces.outcome,
            self.fleets.outcome,
            self.motion.outcome,
            self.missions.outcome,
        ]
    }

    pub fn tables(&self) -> [TableView; N] {
        let [tasks, fleets, ..] = self.outcomes();
        let task_table =
            || table::<tables::TaskForceCounterInputs, _>(tasks, &self.task_forces.rows, task);
        let motion = self.motion_table();
        let missions = self.mission_table();
        let Some(rosters) = self.rosters.first.get(..self.rosters.len) else {
            return [
                task_table(),
                unavailable("fleet roster arena length invalid"),
                motion,
                missions,
            ];
        };
        let fleet_rows = &self.fleets.rows[..fleets.len().min(self.fleets.rows.len())];
        if fleet_rows.iter().any(|r| {
            r.start
                .checked_add(r.len)
                .is_none_or(|end| end > rosters.len())
        }) {
            return [
                task_table(),
                unavailable("fleet roster lies outside captured arena"),
                motion,
                missions,
            ];
        }
        [
            task_table(),
            table::<tables::FleetCounterInputs, _>(fleets, &self.fleets.rows, |r| {
                FleetCounterInputRow {
                    fleet: r.id,
                    red: r.rgba[0],
                    green: r.rgba[1],
                    blue: r.rgba[2],
                    alpha: r.rgba[3],
                    mission_region_count: r.mission_region_count,
                    task_forces: owned_vec(copy_vec(&rosters[r.start..r.start + r.len])),
                }
            }),
            motion,
            missions,
        ]
    }

    fn mission_table(&self) -> TableView {
        let Some(regions) = self.regions.first.get(..self.regions.len) else {
            return unavailable("task-force mission region arena length invalid");
        };
        let rows = &self.missions.rows[..self.missions.outcome.len().min(self.missions.rows.len())];
        if rows.iter().any(|row| {
            row.regions_start
                .checked_add(row.regions_len)
                .is_none_or(|end| end > regions.len())
        }) {
            return unavailable("task-force mission regions lie outside the captured arena");
        }
        table::<tables::TaskForceMissions, _>(self.missions.outcome, &self.missions.rows, |r| {
            TaskForceMissionRow {
                task_force: r.id,
                owner: r.owner,
                current_region: r.current_region,
                regions: owned_vec(copy_vec(
                    &regions[r.regions_start..r.regions_start + r.regions_len],
                )),
            }
        })
    }

    fn motion_table(&self) -> TableView {
        let Some(paths) = self.paths.first.get(..self.paths.len) else {
            return unavailable("task-force route arena length invalid");
        };
        let rows = &self.motion.rows[..self.motion.outcome.len().min(self.motion.rows.len())];
        if rows.iter().any(|row| {
            row.ordinary_start
                .checked_add(row.ordinary_len)
                .and_then(|end| end.checked_add(row.alternate_len))
                .is_none_or(|end| end > paths.len())
        }) {
            return unavailable("task-force route lies outside the captured arena");
        }
        // Each task force sits in one fleet roster; a repeated handle is a torn census. Checked
        // here, off the game thread.
        let Ok(mut handles) = collect_vec(rows.iter().map(|row| row.id)) else {
            return unavailable("collector allocation failed");
        };
        handles.sort_unstable();
        if handles.windows(2).any(|pair| pair[0] == pair[1]) {
            return unavailable("task force appears in more than one fleet roster");
        }
        table::<tables::TaskForceMapMotion, _>(self.motion.outcome, &self.motion.rows, |r| {
            let alternate = r.ordinary_start + r.ordinary_len;
            TaskForceMapMotionRow {
                task_force: r.id,
                target: r.target,
                combat_role: r.combat_role,
                path: owned_vec(copy_vec(&paths[r.ordinary_start..alternate])),
                alternate_path: owned_vec(copy_vec(&paths[alternate..alternate + r.alternate_len])),
            }
        })
    }
}

fn task(r: &ctask_force::CounterRow) -> TaskForceCounterInputRow {
    TaskForceCounterInputRow {
        task_force: r.id,
        is_in_regions: r.is_in_regions,
        running_away: r.running_away,
        engagement_rule: r.engagement_rule,
        mission_region_count: r.mission_region_count,
        bombardment_region: r.bombardment_region,
        repair_parent: r.repair_parent,
        detached_activity: r.detached_activity,
        repair_last_mission: r.repair_last_mission,
        received_supply_ratio: r.received_supply_ratio,
        attached_headquarters: r.attached_headquarters,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;
    use replay::rows::TableRows;

    #[test]
    fn unreadable_countries_fail_every_table_and_rosters_convert_in_native_order() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capture = Captured::with_capacities(&Capacities {
            task_forces: 1,
            fleets: 1,
            rosters: 2,
            motion: 1,
            paths: 1,
            missions: 1,
            regions: 1,
        });
        capture.capture(&memory, 0, 0);
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
        capture.motion.outcome = Outcome::Rows { len: 0 };
        let [
            _,
            TableView::Complete(TableRows::FleetCounterInputs(rows)),
            ..,
        ] = capture.tables()
        else {
            panic!("complete");
        };
        assert_eq!(rows[0].task_forces, [(54 << 32) | 51, (43 << 32) | 51]);
        assert_eq!(rows[0].blue, 0x8000_0000);
        capture.fleets.rows[0].len = 3;
        assert!(matches!(capture.tables()[1], TableView::Unavailable { .. }));
    }

    #[test]
    fn motion_routes_split_from_the_arena_and_torn_censuses_are_refused() {
        let mut capacity = Capacities {
            task_forces: 1,
            fleets: 1,
            rosters: 1,
            motion: 2,
            paths: 3,
            missions: 1,
            regions: 1,
        };
        let mut capture = Captured::with_capacities(&capacity);
        capture.motion.rows[0] = ctask_force::MapMotionRow {
            id: (5 << 32) | 61,
            target: 9,
            combat_role: 2,
            ordinary_start: 0,
            ordinary_len: 2,
            alternate_len: 1,
        };
        capture.paths.first.copy_from_slice(&[7, 9, 11]);
        capture.paths.len = 3;
        capture.motion.outcome = Outcome::Rows { len: 1 };
        let TableView::Complete(TableRows::TaskForceMapMotion(rows)) = capture.motion_table()
        else {
            panic!("complete motion table");
        };
        assert_eq!(
            (rows[0].path.as_slice(), rows[0].alternate_path.as_slice()),
            (&[7, 9][..], &[11][..])
        );
        assert_eq!((rows[0].target, rows[0].combat_role), (9, 2));
        capture.motion.rows[0].alternate_len = 2;
        assert!(matches!(
            capture.motion_table(),
            TableView::Unavailable { .. }
        ));
        capture.motion.rows[0].alternate_len = 0;
        capture.motion.rows[1] = capture.motion.rows[0];
        capture.motion.outcome = Outcome::Rows { len: 2 };
        assert!(matches!(
            capture.motion_table(),
            TableView::Unavailable { .. }
        ));
        capture.motion.outcome = Outcome::Truncated {
            len: 0,
            unread_rows: 3,
        };
        capture.paths.required = 12;
        capture.grow(&mut capacity);
        assert!(capacity.paths >= 12 && capture.paths.first.len() >= 12);
    }

    #[test]
    fn mission_regions_split_from_the_arena_and_stray_ranges_are_refused() {
        let mut capacity = Capacities {
            task_forces: 1,
            fleets: 1,
            rosters: 1,
            motion: 1,
            paths: 1,
            missions: 1,
            regions: 3,
        };
        let mut capture = Captured::with_capacities(&capacity);
        capture.missions.rows[0] = ctask_force::MissionRow {
            id: (5 << 32) | 61,
            owner: 42,
            current_region: 2,
            regions_start: 0,
            regions_len: 3,
        };
        capture.regions.first.copy_from_slice(&[2, 1, 2]);
        capture.regions.len = 3;
        capture.missions.outcome = Outcome::Rows { len: 1 };
        let TableView::Complete(TableRows::TaskForceMissions(rows)) = capture.mission_table()
        else {
            panic!("complete mission table");
        };
        assert_eq!(
            (rows[0].task_force, rows[0].owner, rows[0].current_region),
            ((5 << 32) | 61, 42, 2)
        );
        assert_eq!(rows[0].regions, [2, 1, 2]);
        capture.missions.rows[0].regions_len = 4;
        assert!(matches!(
            capture.mission_table(),
            TableView::Unavailable { .. }
        ));
        capture.missions.outcome = Outcome::Truncated {
            len: 0,
            unread_rows: 1,
        };
        capture.regions.required = 9;
        capture.grow(&mut capacity);
        assert!(capacity.regions >= 9 && capture.regions.first.len() >= 9);
    }
}
