//! Hourly division companions. Reuses the existing authenticated source census;
//! movement and counter reads retain separate costs and outcomes.
use super::production::Sources;
use crate::{
    projection::{Projection, project_table, scalar_table},
    sampling::qpc,
    spooling::unavailable,
    table_capture::{Arena, CaptureRows, grow_capacity},
};
use engine::{
    Memory, carmy, carmy::DivisionSeed, corders_group_member::MembershipCapacity, cprovince,
    walk::Outcome,
};
use replay::{
    frame::TableInput,
    rows::{DivisionMovementRow, DivisionStatusRow, TableId, tables},
};

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::DivisionMovement, TableId::DivisionStatus];
// Both depend on the shared division census; the combined source read group in
// extended::read_groups includes their individually measured costs.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, Debug)]
pub(crate) struct CaptureContext {
    pub base: usize,
    pub game_state: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CaptureReport {
    pub ticks: [u64; N],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    movement: usize,
    status: usize,
    paths: usize,
    geometry: cprovince::GeometryCapacities,
    /// Shared DivisionStatus membership storage prepared for pooled packets.
    /// Worker acquisition retains larger original demand in the same pass.
    membership: MembershipCapacity,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            movement: 8192,
            status: 8192,
            paths: 128 * 1024,
            geometry: cprovince::GeometryCapacities::default(),
            membership: carmy::StatusCapacities::new(8192).membership,
        }
    }
}
fn status_packet(c: &Capacities) -> carmy::StatusPacket {
    let mut packet = carmy::StatusPacket::new(c.status);
    packet.grow(c.status, c.membership);
    packet
}

pub struct Captured {
    movement: CaptureRows<carmy::MovementRow>,
    status: CaptureRows<carmy::StatusRow>,
    paths: Arena<u32>,
    movement_packet: carmy::MovementPacket,
    geometry: cprovince::GeometryPacket,
    status_packet: carmy::StatusPacket,
    membership: MembershipCapacity,
    source_capacity: usize,
    source_required: usize,
    decode_pending: bool,
}

impl Captured {
    /// Empty ownership placeholder; constructing it never provisions packets.
    pub(crate) fn vacant() -> Self {
        Self {
            movement: CaptureRows::new(0),
            status: CaptureRows::new(0),
            paths: Arena::new(0),
            movement_packet: carmy::MovementPacket::new(0, 0),
            geometry: cprovince::GeometryPacket::with_capacities(cprovince::GeometryCapacities {
                provinces: 0,
                neighbour_entries: 0,
            }),
            status_packet: carmy::StatusPacket::vacant(),
            membership: MembershipCapacity::default(),
            source_capacity: 0,
            source_required: 0,
            decode_pending: false,
        }
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            movement: CaptureRows::new(c.movement),
            status: CaptureRows::new(c.status),
            paths: Arena::new(c.paths),
            movement_packet: carmy::MovementPacket::new(c.movement, c.paths),
            geometry: cprovince::GeometryPacket::with_capacities(c.geometry),
            status_packet: status_packet(c),
            membership: c.membership,
            source_capacity: c.movement.max(c.status),
            source_required: 0,
            decode_pending: false,
        }
    }

    pub fn reset(&mut self) {
        self.movement.reset();
        self.status.reset();
        self.paths.reset();
        // Engine acquisition resets each owned packet in its measured scope.
        self.decode_pending = false;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.movement.allocated_bytes()
            + self.status.allocated_bytes()
            + self.paths.allocated_bytes()
            + self.movement_packet.allocated_bytes()
            + self.geometry.allocated_bytes()
            + self.status_packet.allocated_bytes()
    }

    pub fn grow(&mut self, c: &mut Capacities) {
        self.movement.grow(&mut c.movement);
        self.status.grow(&mut c.status);
        let membership = self.status_packet.membership_capacity();
        c.membership.queries = c.membership.queries.max(membership.queries);
        c.membership.groups = c.membership.groups.max(membership.groups);
        c.membership.roots = c.membership.roots.max(membership.roots);
        c.membership.chunks = c.membership.chunks.max(membership.chunks);
        c.membership = c
            .membership
            .grown(self.status_packet.required_capacities().membership);
        self.membership = c.membership;
        self.paths.grow(&mut c.paths);
        let geometry_required = self.geometry.required_capacities();
        grow_capacity(&mut c.geometry.provinces, geometry_required.provinces);
        grow_capacity(
            &mut c.geometry.neighbour_entries,
            geometry_required.neighbour_entries,
        );
        self.geometry.grow(c.geometry);
        self.movement_packet.reserve(c.movement, c.paths);
        self.reserve_sources(
            self.source_capacity
                .max(self.source_required)
                .max(c.movement)
                .max(c.status),
        );
    }

    /// Prepare primary packet storage for the complete shared source census.
    pub(crate) fn reserve_sources(&mut self, capacity: usize) {
        self.source_capacity = self.source_capacity.max(capacity);
        self.movement_packet
            .reserve(self.source_capacity, self.paths.first.len());
        self.status_packet
            .grow(self.source_capacity, self.membership);
    }

    /// Acquire on the reserved worker using the immutable source lease.
    /// Membership storage may grow at the original demanded root or chunk.
    pub(crate) fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        sources: Sources<'_, DivisionSeed>,
    ) -> CaptureReport {
        CaptureReport {
            ticks: self.capture(memory, context.base, context.game_state, sources),
        }
    }

    /// Finish an accepted worker failure without retrying the live acquisition.
    pub(crate) fn fail_owned_capture(&mut self, reason: &'static str) -> CaptureReport {
        self.movement.outcome = Outcome::Unavailable { reason };
        self.status.outcome = Outcome::Unavailable { reason };
        self.paths.len = 0;
        self.decode_pending = false;
        CaptureReport::default()
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        sources: Sources<'_, DivisionSeed>,
    ) -> [u64; N] {
        self.reset();
        let outcome = if sources.outcome.len() <= sources.rows.len() {
            sources.outcome
        } else {
            Outcome::Unavailable {
                reason: "division presentation source count invalid",
            }
        };
        let rows = &sources.rows[..outcome.len()];
        self.source_required = self.source_required.max(rows.len());
        let movement_reads = crate::read_accounting::table(TableId::DivisionMovement);
        let start = qpc();
        let _ = self.geometry.begin(memory, base, state);
        let movement = carmy::capture_movement_inputs(
            memory,
            base,
            state,
            rows,
            outcome,
            self.movement.rows.len(),
            self.paths.first.len(),
            &mut self.geometry,
            &mut self.movement_packet,
        );
        self.movement.outcome = movement.outcome;
        self.paths.len = movement.paths_len;
        self.paths.required = movement.required_paths;
        let movement_ticks = qpc().saturating_sub(start).max(0) as u64;
        drop(movement_reads);
        let _status_reads = crate::read_accounting::table(TableId::DivisionStatus);
        let start = qpc();
        self.status.outcome =
            self.status_packet
                .capture(memory, base, state, rows, outcome, self.status.rows.len());
        self.decode_pending = true;
        [movement_ticks, qpc().saturating_sub(start).max(0) as u64]
    }

    /// Pure decode-worker projection from the current owned packets.
    pub(crate) fn decode_inputs(&mut self) -> [u64; N] {
        if !std::mem::take(&mut self.decode_pending) {
            return [0; N];
        }
        let started = qpc();
        let movement = carmy::project_movement(
            &self.movement_packet,
            &mut self.movement.rows,
            &mut self.paths.first,
        );
        self.movement.outcome = movement.outcome;
        self.paths.len = movement.paths_len;
        self.paths.required = self.paths.required.max(movement.required_paths);
        let movement_ticks = qpc().saturating_sub(started).max(0) as u64;
        let started = qpc();
        self.status.outcome = self.status_packet.project(&mut self.status.rows);
        [movement_ticks, qpc().saturating_sub(started).max(0) as u64]
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.movement.outcome, self.status.outcome]
    }

    pub(crate) fn table_inputs(
        &self,
        mut projection: Option<&mut Projection<'_>>,
    ) -> [TableInput; N] {
        if self.decode_pending {
            return std::array::from_fn(|_| {
                unavailable("division presentation projection pending").into()
            });
        }
        let [movement, status] = self.outcomes();
        let status = scalar_table::<tables::DivisionStatus, _>(
            projection.as_deref_mut(),
            status,
            &self.status.rows,
            |r| u128::from(r.division),
            |r| DivisionStatusRow {
                division: r.division,
                maximum_hit_points: r.maximum_hit_points,
                maximum_organisation: r.maximum_organisation,
                entrenchment: r.entrenchment,
                entrenchment_cap: r.entrenchment_cap,
                supply_ratio: r.supply_ratio,
                attrition_value: r.attrition_value,
                attrition_threshold: r.attrition_threshold,
                attrition: r.attrition,
                exercising: r.exercising,
                has_order: r.has_order,
                invalid_orders: r.invalid_orders,
                strength_factor: r.strength_factor,
            },
        );
        let Some(paths) = self.paths.first.get(..self.paths.len) else {
            return [unavailable("movement arena invalid").into(), status];
        };
        let span =
            |start: usize, len: usize| start.checked_add(len).and_then(|end| paths.get(start..end));
        if self.movement.rows.get(..movement.len()).is_none_or(|rows| {
            rows.iter().any(|r| {
                span(r.path_start, r.path_len).is_none()
                    || span(r.alternate_start, r.alternate_len).is_none()
            })
        }) {
            return [unavailable("movement path spans invalid").into(), status];
        }
        [
            project_table::<tables::DivisionMovement, _>(
                projection,
                movement,
                &self.movement.rows,
                |r| u128::from(r.division),
                |r, previous| {
                    r.division == previous.division
                        && r.source == previous.source
                        && r.target == previous.target
                        && r.next == previous.next
                        && r.destination == previous.destination
                        && r.numerator == previous.numerator
                        && r.denominator == previous.denominator
                        && r.progress == previous.progress
                        && r.priority == previous.priority
                        && r.retreat == previous.retreat
                        && r.strategic_redeployment == previous.strategic_redeployment
                        && r.naval_transfer == previous.naval_transfer
                        && r.combat_count == previous.combat_count
                        && r.path_total == previous.path_total
                        && span(r.path_start, r.path_len).unwrap() == previous.path.as_slice()
                        && r.combat_role == previous.combat_role
                        && r.stack_kind == previous.stack_kind
                        && r.alternate_total == previous.alternate_total
                        && span(r.alternate_start, r.alternate_len).unwrap()
                            == previous.alternate_path.as_slice()
                },
                |r| DivisionMovementRow {
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
                },
            ),
            status,
        ]
    }
    #[cfg(test)]
    pub(super) fn tables(&self) -> [replay::frame::TableView; N] {
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("projection without a baseline always samples"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{frame::TableView, rows::TableRows};

    fn presentation_fixture() -> (DivisionSeed, Vec<u8>) {
        let seed = DivisionSeed {
            pointer: 0x8000,
            division: 7,
            primary_vtable: Some(carmy::VTABLE_RVA as u64),
            handle: Some(7),
            class: Some(0),
            ordinary_route: Some([0; 16]),
            alternate_route: Some([0; 16]),
            combat: Some([0; 16]),
            location: Some(0),
            movement_numerator: Some(13),
            movement_target: Some(0),
            movement_source: Some(0),
            naval_transfer_membership: Some(0),
            map_entity: Some(0),
            movement_priority: Some(0),
            retreat: Some(0),
            strategic_redeployment: Some(0),
            ..DivisionSeed::default()
        };
        let mut bytes = vec![0; 0x400];
        // Status authenticates the country/link roots even when its row
        // capacity admits no unit. Supply the required null/sentinel slots.
        for (offset, data, count) in [
            (engine::cgame_state::COUNTRIES, 0x1380_u64, 2_u32),
            (engine::cgame_state::COUNTRY_LINKS, 0x1390_u64, 1_u32),
        ] {
            bytes[offset..offset + 8].copy_from_slice(&data.to_le_bytes());
            bytes[offset + 8..offset + 12].copy_from_slice(&count.to_le_bytes());
            bytes[offset + 12..offset + 16].copy_from_slice(&count.to_le_bytes());
        }
        (seed, bytes)
    }

    #[test]
    fn vacant_presentation_has_no_owned_allocations_or_rows() {
        let mut capture = Captured::vacant();
        assert_eq!(capture.allocated_bytes(), 0);
        assert_eq!(capture.decode_inputs(), [0; N]);
        assert!(
            capture
                .tables()
                .iter()
                .all(|view| matches!(view, TableView::Unavailable { .. }))
        );
    }

    #[test]
    fn owned_failure_keeps_stale_packets_unpublished() {
        use std::panic::{AssertUnwindSafe, catch_unwind};

        struct PanicAt<'a> {
            memory: engine::memory::SliceMemory<'a>,
            address: usize,
        }
        impl Memory for PanicAt<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                assert_ne!(address, self.address, "synthetic acquisition panic");
                self.memory.copy(address, destination)
            }
        }

        let (seed, bytes) = presentation_fixture();
        let seeds = [seed];
        let context = CaptureContext {
            base: 0,
            game_state: 0x1000,
        };
        let sources = || Sources {
            rows: &seeds,
            outcome: Outcome::Rows { len: 1 },
        };
        for offset in [
            engine::cgame_state::PROVINCES,
            engine::cgame_state::COUNTRIES,
        ] {
            let mut capture = Captured::with_capacities(&Capacities {
                movement: 1,
                status: 1,
                paths: 0,
                geometry: cprovince::GeometryCapacities {
                    provinces: 1,
                    neighbour_entries: 0,
                },
                membership: carmy::StatusCapacities::new(1).membership,
            });
            capture.reserve_sources(1);
            capture.capture_owned(
                &engine::memory::SliceMemory {
                    base: 0x1000,
                    bytes: &bytes,
                },
                context,
                sources(),
            );
            let memory = PanicAt {
                memory: engine::memory::SliceMemory {
                    base: 0x1000,
                    bytes: &bytes,
                },
                address: 0x1000 + offset,
            };
            assert!(
                catch_unwind(AssertUnwindSafe(|| {
                    capture.capture_owned(&memory, context, sources());
                }))
                .is_err()
            );
            let report = capture.fail_owned_capture("worker failed");
            assert_eq!(report.ticks, [0; N]);
            assert_eq!(capture.decode_inputs(), [0; N]);
            assert_eq!(
                capture.outcomes(),
                [Outcome::Unavailable {
                    reason: "worker failed"
                }; N]
            );
            assert!(
                capture
                    .tables()
                    .iter()
                    .all(|view| matches!(view, TableView::Unavailable { .. }))
            );
            capture.reset();
            capture.fail_owned_capture("worker stopped");
            assert_eq!(capture.decode_inputs(), [0; N]);
            assert_eq!(
                capture.outcomes(),
                [Outcome::Unavailable {
                    reason: "worker stopped"
                }; N]
            );
        }
    }

    #[test]
    fn presentation_projects_the_primary_packet_once_after_acquisition() {
        let capacity = Capacities {
            movement: 1,
            status: 0,
            paths: 0,
            geometry: cprovince::GeometryCapacities {
                provinces: 1,
                neighbour_entries: 0,
            },
            membership: carmy::StatusCapacities::new(0).membership,
        };
        let mut capture = Captured::with_capacities(&capacity);
        capture.reserve_sources(1);
        let (seed, mut bytes) = presentation_fixture();
        let seeds = [seed];
        capture.movement.rows[0].division = 999;
        capture.capture(
            &engine::memory::SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            0,
            0x1000,
            Sources {
                rows: &seeds,
                outcome: Outcome::Rows { len: 1 },
            },
        );
        assert_eq!(capture.outcomes()[0], Outcome::Rows { len: 1 });
        assert_eq!(
            capture.outcomes()[1],
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        assert_eq!(
            capture.movement.rows[0].division, 999,
            "acquisition never projects into pooled output"
        );
        assert!(
            capture
                .table_inputs(None)
                .iter()
                .all(|input| matches!(input, TableInput::Sampled(TableView::Unavailable { .. })))
        );
        bytes.fill(0xff);
        capture.decode_inputs();
        assert_eq!(capture.movement.rows[0].division, 7);
        assert_eq!(capture.movement.rows[0].numerator, 13);
        assert_eq!(capture.outcomes()[0], Outcome::Rows { len: 1 });
        assert_eq!(
            capture.decode_inputs(),
            [0, 0],
            "projection runs once per capture"
        );
        capture.reset();
        assert_eq!(capture.decode_inputs(), [0; N]);
    }

    #[test]
    fn division_presentation_conversion_keeps_capped_counts_and_scalar_bits() {
        let mut capture = Captured::with_capacities(&Capacities::default());
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
            strength_factor: 57_038,
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
            (
                s[0].entrenchment,
                s[0].maximum_hit_points,
                s[0].has_order,
                s[0].strength_factor
            ),
            (i64::MIN, i64::MAX, 0, 57_038)
        );
        let mut baseline = std::array::from_fn(|_| unavailable("outside presentation fixture"));
        for (id, view) in TABLES.into_iter().zip(capture.tables()) {
            baseline[id.index()] = view;
        }
        let mut encoder = replay::frame::FrameEncoder::new();
        encoder
            .encode(
                replay::frame::DecodedFrame {
                    branch: 1,
                    hour: 0,
                    tables: baseline.map(Into::into),
                    metrics: replay::frame::Metrics::default(),
                },
                false,
            )
            .unwrap();
        let mut scratch = crate::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 1, false, &mut scratch);
        assert!(
            capture
                .table_inputs(Some(&mut projection))
                .iter()
                .all(|input| matches!(input, TableInput::ComparedEqual(_)))
        );
        // A path-only mutation and an independently changing status must both
        // defeat reuse, without changing their keys or arena spans.
        capture.paths.first[3] ^= 1;
        let [movement, status] = capture.table_inputs(Some(&mut projection));
        let TableInput::ComparedChanges { indices, rows, .. } = movement else {
            panic!("changed movement must replace its freshly compared row");
        };
        assert_eq!(indices, [0]);
        assert_eq!(TableView::Complete(rows), capture.tables()[0]);
        assert!(matches!(status, TableInput::ComparedEqual(_)));
        capture.paths.first[3] ^= 1;
        capture.status.rows[0].has_order = 1;
        let [movement, status] = capture.table_inputs(Some(&mut projection));
        assert!(matches!(movement, TableInput::ComparedEqual(_)));
        let TableInput::ComparedChanges { indices, rows, .. } = status else {
            panic!("changed status must replace its freshly compared row");
        };
        assert_eq!(indices, [0]);
        assert_eq!(TableView::Complete(rows), capture.tables()[1]);
        capture.status.rows[0].has_order = 0;
        capture.movement.rows[0].alternate_start = usize::MAX;
        let [movement, status] = capture.table_inputs(Some(&mut projection));
        assert!(matches!(
            movement,
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
        assert!(matches!(status, TableInput::ComparedEqual(_)));
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        assert!(matches!(capture.tables()[1], TableView::Complete(_)));
    }

    #[test]
    fn division_presentation_preserves_source_failure_and_primary_arena_growth() {
        let mut capacity = Capacities {
            movement: 1,
            status: 1,
            paths: 2,
            membership: MembershipCapacity::default(),
            ..Capacities::default()
        };
        let mut capture = Captured::with_capacities(&capacity);
        let unavailable = Outcome::Unavailable {
            reason: "census failed",
        };
        capture.capture(
            &engine::memory::SliceMemory {
                base: 0,
                bytes: &[],
            },
            0,
            0,
            Sources {
                rows: &[],
                outcome: unavailable,
            },
        );
        capture.decode_inputs();
        assert_eq!(capture.outcomes(), [unavailable; N]);
        assert!(
            capture
                .tables()
                .iter()
                .all(|t| matches!(t, TableView::Unavailable { .. }))
        );
        capture.paths.required = 30;
        let membership = capture.status_packet.membership_capacity();
        capture.grow(&mut capacity);
        assert!(capture.paths.first.len() >= 30);
        assert_eq!(capacity.membership, membership);
        capture.reset();
        assert_eq!(capture.decode_inputs(), [0; N]);
    }
}
