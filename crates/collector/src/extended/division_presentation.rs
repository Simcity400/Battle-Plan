//! Hourly division companions. Reuses the existing authenticated source census;
//! movement and counter reads have separate costs and consistency windows.
use super::production::Sources;
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{Memory, carmy, cequipment_variant_pool::DivisionSource, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{DivisionMovementRow, DivisionStatusRow, TableId, tables},
};

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::DivisionMovement, TableId::DivisionStatus];
// Both depend on the shared division census; the combined source read group in
// extended::read_groups includes their individually measured costs.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    movement: usize,
    status: usize,
    paths: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            movement: 8192,
            status: 8192,
            paths: 128 * 1024,
        }
    }
}

pub struct Captured {
    movement: CaptureRows<carmy::MovementRow>,
    status: CaptureRows<carmy::StatusRow>,
    paths: Arena<u32>,
}
impl Captured {
    pub fn with_capacities(c: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            movement: CaptureRows::new(c.movement, successful[0]),
            status: CaptureRows::new(c.status, successful[1]),
            paths: Arena::new(c.paths, successful[0]),
        }
    }
    pub fn reset(&mut self) {
        self.movement.reset();
        self.status.reset();
        self.paths.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.movement.allocated_bytes()
            + self.status.allocated_bytes()
            + self.paths.allocated_bytes()
    }
    pub fn grow(&mut self, c: &mut Capacities, successful: &[u16]) {
        assert_eq!(successful.len(), N);
        self.movement.grow(&mut c.movement, successful[0]);
        self.status.grow(&mut c.status, successful[1]);
        self.paths.grow(&mut c.paths, successful[0]);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
        sources: Sources<'_, DivisionSource>,
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        self.reset();
        let bounded = |rows: &[DivisionSource], outcome: Outcome| {
            if outcome.len() <= rows.len() {
                outcome
            } else {
                Outcome::Unavailable {
                    reason: "division presentation source count invalid",
                }
            }
        };
        let first_outcome = bounded(sources.first, sources.first_outcome);
        let second_outcome = bounded(sources.second, sources.second_outcome);
        let first = &sources.first[..first_outcome.len()];
        let second = &sources.second[..second_outcome.len()];
        let start = qpc();
        let a = carmy::walk_movement(
            memory,
            base,
            state,
            first,
            first_outcome,
            &mut self.movement.rows,
            &mut self.paths.first,
        );
        self.movement.outcome = a.outcome;
        self.paths.len = a.paths_len;
        self.paths.required = a.required_paths;
        let mut pass = [false; N];
        if needed[0] {
            self.movement.checked = true;
            let b = carmy::walk_movement(
                memory,
                base,
                state,
                second,
                second_outcome,
                &mut self.movement.second,
                &mut self.paths.second,
            );
            let rows = self.movement.compare(b.outcome);
            let paths = self.paths.compare(b.paths_len, b.required_paths);
            self.movement.mismatches += usize::from(!paths);
            pass[0] = rows && paths;
        }
        let movement_ticks = qpc().saturating_sub(start).max(0) as u64;
        let start = qpc();
        self.status.outcome = carmy::walk_status(
            memory,
            base,
            state,
            first,
            first_outcome,
            &mut self.status.rows,
        );
        if needed[1] {
            self.status.checked = true;
            let b = carmy::walk_status(
                memory,
                base,
                state,
                second,
                second_outcome,
                &mut self.status.second,
            );
            pass[1] = self.status.compare(b);
        }
        (
            pass,
            [movement_ticks, qpc().saturating_sub(start).max(0) as u64],
        )
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            (
                self.movement.checked,
                self.movement.mismatches,
                self.movement.outcome,
            ),
            (
                self.status.checked,
                self.status.mismatches,
                self.status.outcome,
            ),
        ]
        .map(|(checked, mismatch, outcome)| {
            if checked && mismatch != 0 {
                Outcome::Unavailable {
                    reason: "division presentation changed during double read",
                }
            } else {
                outcome
            }
        })
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.movement.mismatches, self.status.mismatches]
    }
    pub fn tables(&self) -> [TableView; N] {
        let [movement, status] = self.outcomes();
        let status =
            table::<tables::DivisionStatus, _>(status, &self.status.rows, |r| DivisionStatusRow {
                division: r.division,
                maximum_hit_points: r.maximum_hit_points,
                maximum_organisation: r.maximum_organisation,
                entrenchment: r.entrenchment,
                entrenchment_cap: r.entrenchment_cap,
                supply_ratio: r.supply_ratio,
                attrition_value: r.attrition_value,
                attrition_threshold: r.attrition_threshold,
                attrition: r.attrition,
                training_requested: r.training_requested,
                exercising: r.exercising,
                has_order: r.has_order,
                no_orders: r.no_orders,
            });
        let Some(paths) = self.paths.first.get(..self.paths.len) else {
            return [unavailable("movement arena invalid"), status];
        };
        let span =
            |start: usize, len: usize| start.checked_add(len).and_then(|end| paths.get(start..end));
        if self.movement.rows.get(..movement.len()).is_none_or(|rows| {
            rows.iter().any(|r| {
                span(r.path_start, r.path_len).is_none()
                    || span(r.alternate_start, r.alternate_len).is_none()
            })
        }) {
            return [unavailable("movement path spans invalid"), status];
        }
        [
            table::<tables::DivisionMovement, _>(movement, &self.movement.rows, |r| {
                DivisionMovementRow {
                    division: r.division,
                    source: r.source,
                    target: r.target,
                    next: r.next,
                    destination: r.destination,
                    numerator: r.numerator,
                    denominator: r.denominator,
                    progress: r.progress,
                    priority: r.priority,
                    retreat: r.retreat,
                    strategic_redeployment: r.strategic_redeployment,
                    naval_transfer: r.naval_transfer,
                    combat_count: r.combat_count,
                    path_total: r.path_total,
                    path: span(r.path_start, r.path_len).unwrap().to_vec(),
                    combat_role: r.combat_role,
                    stack_kind: r.stack_kind,
                    alternate_total: r.alternate_total,
                    alternate_path: span(r.alternate_start, r.alternate_len).unwrap().to_vec(),
                }
            }),
            status,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::TableRows;

    #[test]
    fn division_presentation_conversion_keeps_capped_counts_and_scalar_bits() {
        let mut capture = Captured::with_capacities(&Capacities::default(), &[0, 0]);
        capture.paths.first[..4].copy_from_slice(&[7, 3, 7, u32::MAX]);
        capture.paths.len = 4;
        capture.movement.rows[0] = carmy::MovementRow {
            division: u64::MAX,
            numerator: i64::MIN,
            denominator: i64::MAX,
            path_start: 0,
            path_len: 3,
            path_total: 300,
            alternate_start: 3,
            alternate_len: 1,
            alternate_total: 1,
            stack_kind: -1,
            ..Default::default()
        };
        capture.movement.outcome = Outcome::Rows { len: 1 };
        capture.status.outcome = Outcome::Rows { len: 1 };
        capture.status.rows[0] = carmy::StatusRow {
            division: u64::MAX,
            entrenchment: i64::MIN,
            maximum_hit_points: i64::MAX,
            has_order: 0,
            no_orders: 0,
            ..Default::default()
        };
        let [
            TableView::Complete(TableRows::DivisionMovement(m)),
            TableView::Complete(TableRows::DivisionStatus(s)),
        ] = capture.tables()
        else {
            panic!("complete companions");
        };
        assert_eq!(
            (
                m[0].division,
                m[0].numerator,
                m[0].denominator,
                m[0].stack_kind
            ),
            (u64::MAX, i64::MIN, i64::MAX, -1)
        );
        assert_eq!(
            (&m[0].path[..], m[0].path_total, &m[0].alternate_path[..]),
            (&[7, 3, 7][..], 300, &[u32::MAX][..])
        );
        assert_eq!(
            (s[0].entrenchment, s[0].maximum_hit_points, s[0].no_orders),
            (i64::MIN, i64::MAX, 0)
        );
        capture.movement.rows[0].alternate_start = usize::MAX;
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        assert!(matches!(capture.tables()[1], TableView::Complete(_)));
        capture.status.checked = true;
        capture.status.mismatches = 1;
        assert!(matches!(capture.tables()[1], TableView::Unavailable { .. }));
    }

    #[test]
    fn division_presentation_source_failure_and_retirement_are_independent() {
        let mut capacity = Capacities {
            movement: 1,
            status: 1,
            paths: 2,
        };
        let mut capture = Captured::with_capacities(&capacity, &[0, 0]);
        let unavailable = Outcome::Unavailable {
            reason: "census failed",
        };
        let (pass, _) = capture.capture(
            &engine::memory::SliceMemory {
                base: 0,
                bytes: &[],
            },
            0,
            0,
            &[true, true],
            Sources {
                first: &[],
                first_outcome: unavailable,
                second: &[],
                second_outcome: unavailable,
            },
        );
        assert_eq!(pass, [false, false]);
        assert!(
            capture
                .tables()
                .iter()
                .all(|t| matches!(t, TableView::Unavailable { .. }))
        );
        capture.paths.required = 30;
        capture.grow(&mut capacity, &[200, 0]);
        assert!(capture.movement.second.is_empty() && capture.paths.second.is_empty());
        assert!(!capture.status.second.is_empty());
        assert!(capture.paths.first.len() >= 30);
        capture.reset();
        assert_eq!(capture.checked_mismatches(), [0, 0]);
    }
}
