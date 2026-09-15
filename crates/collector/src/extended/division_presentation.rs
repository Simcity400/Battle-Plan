//! Hourly division companions. Reuses the existing authenticated source census;
//! movement and counter reads have separate costs and consistency windows.
use super::production::Sources;
use crate::{
    projection::{Projection, project_table, scalar_table},
    sampling::{CONSISTENCY_TARGET, full_match, qpc},
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
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    movement: usize,
    status: usize,
    paths: usize,
    geometry: cprovince::GeometryCapacities,
    /// Shared DivisionStatus membership bounds, so every pooled packet grows
    /// after any buffer overflows rather than each losing its own hour.
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
    movement_second: Option<carmy::MovementPacket>,
    geometry: cprovince::GeometryPacket,
    geometry_second: Option<cprovince::GeometryPacket>,
    status_packet: carmy::StatusPacket,
    status_second: Option<carmy::StatusPacket>,
    membership: MembershipCapacity,
    source_capacity: usize,
    source_required: usize,
    source_lens: [usize; 2],
    captured_current: bool,
    decode_pending: bool,
    seed_rejected: [bool; N],
}

fn same_seed_observation(
    sources: &Sources<'_, DivisionSeed>,
    first_masks: &[u64],
    second_masks: &[u64],
) -> bool {
    sources.first_outcome == sources.second_outcome
        && sources.first.len() == sources.second.len()
        && first_masks.len() == sources.first.len()
        && second_masks.len() == sources.second.len()
        && sources
            .first
            .iter()
            .zip(sources.second)
            .zip(first_masks.iter().zip(second_masks))
            .all(|((first, second), (a, b))| {
                first.pointer == second.pointer
                    && carmy::division_seed_fields_equal(first, second, a | b)
            })
}

impl Captured {
    pub fn with_capacities(c: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            // Native packets own the independent cold observations. Their
            // deterministic output needs only one projected row/arena buffer.
            movement: CaptureRows::new(c.movement, CONSISTENCY_TARGET),
            status: CaptureRows::new(c.status, CONSISTENCY_TARGET),
            paths: Arena::new(c.paths, CONSISTENCY_TARGET),
            movement_packet: carmy::MovementPacket::new(c.movement, c.paths),
            movement_second: (successful[0] < CONSISTENCY_TARGET)
                .then(|| carmy::MovementPacket::new(c.movement, c.paths)),
            geometry: cprovince::GeometryPacket::with_capacities(c.geometry),
            geometry_second: (successful[0] < CONSISTENCY_TARGET)
                .then(|| cprovince::GeometryPacket::with_capacities(c.geometry)),
            status_packet: status_packet(c),
            status_second: (successful[1] < CONSISTENCY_TARGET).then(|| status_packet(c)),
            membership: c.membership,
            source_capacity: c.movement.max(c.status),
            source_required: 0,
            source_lens: [0; 2],
            captured_current: false,
            decode_pending: false,
            seed_rejected: [false; N],
        }
    }
    pub fn reset(&mut self) {
        self.movement.reset();
        self.status.reset();
        self.paths.reset();
        // Each engine acquisition resets its owned packet inside that table's
        // measured scope. Invalidating exposure here avoids duplicate clears.
        self.source_lens = [0; 2];
        self.captured_current = false;
        self.decode_pending = false;
        self.seed_rejected = [false; N];
    }
    pub fn allocated_bytes(&self) -> usize {
        self.movement.allocated_bytes()
            + self.status.allocated_bytes()
            + self.paths.allocated_bytes()
            + self.movement_packet.allocated_bytes()
            + self
                .movement_second
                .as_ref()
                .map_or(0, |packet| packet.allocated_bytes())
            + self.geometry.allocated_bytes()
            + self
                .geometry_second
                .as_ref()
                .map_or(0, |packet| packet.allocated_bytes())
            + self.status_packet.allocated_bytes()
            + self
                .status_second
                .as_ref()
                .map_or(0, |packet| packet.allocated_bytes())
    }
    pub fn grow(&mut self, c: &mut Capacities, successful: &[u16]) {
        assert_eq!(successful.len(), N);
        self.movement.grow(&mut c.movement, CONSISTENCY_TARGET);
        self.status.grow(&mut c.status, CONSISTENCY_TARGET);
        for packet in std::iter::once(&self.status_packet).chain(&self.status_second) {
            c.membership = c
                .membership
                .grown(packet.required_capacities().membership);
        }
        self.membership = c.membership;
        self.paths.grow(&mut c.paths, CONSISTENCY_TARGET);
        let mut geometry_required = self.geometry.required_capacities();
        if let Some(geometry) = &self.geometry_second {
            let second = geometry.required_capacities();
            geometry_required.provinces = geometry_required.provinces.max(second.provinces);
            geometry_required.neighbour_entries = geometry_required
                .neighbour_entries
                .max(second.neighbour_entries);
        }
        grow_capacity(&mut c.geometry.provinces, geometry_required.provinces);
        grow_capacity(
            &mut c.geometry.neighbour_entries,
            geometry_required.neighbour_entries,
        );
        self.geometry.grow(c.geometry);
        if successful[0] >= CONSISTENCY_TARGET {
            self.movement_second = None;
            self.geometry_second = None;
        } else {
            let packet = self
                .movement_second
                .get_or_insert_with(|| carmy::MovementPacket::new(c.movement, c.paths));
            packet.reserve(c.movement, c.paths);
            self.geometry_second
                .get_or_insert_with(|| cprovince::GeometryPacket::with_capacities(c.geometry))
                .grow(c.geometry);
        }
        if successful[1] >= CONSISTENCY_TARGET {
            self.status_second = None;
        } else {
            self.status_second
                .get_or_insert_with(|| status_packet(c))
                .grow(c.status, c.membership);
        }
        self.movement_packet.reserve(c.movement, c.paths);
        self.reserve_sources(
            self.source_capacity
                .max(self.source_required)
                .max(c.movement)
                .max(c.status),
        );
    }
    /// Off-thread mask capacity follows the complete shared census separately
    /// from either table's independently bounded replay row capacity.
    pub(crate) fn reserve_sources(&mut self, capacity: usize) {
        self.source_capacity = self.source_capacity.max(capacity);
        self.movement_packet
            .reserve(self.source_capacity, self.paths.first.len());
        self.status_packet
            .grow(self.source_capacity, self.membership);
        if let Some(packet) = &mut self.movement_second {
            packet.reserve(self.source_capacity, self.paths.first.len());
        }
        if let Some(packet) = &mut self.status_second {
            packet.grow(self.source_capacity, self.membership);
        }
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
        sources: Sources<'_, DivisionSeed>,
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        self.reset();
        let bounded = |rows: &[DivisionSeed], outcome: Outcome| {
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
        self.source_lens = [first.len(), second.len()];
        self.captured_current = true;
        self.source_required = self.source_required.max(first.len());
        if needed.iter().any(|needed| *needed) {
            self.source_required = self.source_required.max(second.len());
        }
        let sources = Sources {
            first,
            first_outcome,
            second,
            second_outcome,
        };
        let mut pass = [false; N];
        let start = qpc();
        let _ = self.geometry.begin(memory, base, state);
        let a = carmy::capture_movement_inputs(
            memory,
            base,
            state,
            first,
            first_outcome,
            self.movement.rows.len(),
            self.paths.first.len(),
            &mut self.geometry,
            &mut self.movement_packet,
        );
        self.movement.outcome = a.outcome;
        self.paths.len = a.paths_len;
        self.paths.required = a.required_paths;
        if needed[0] {
            self.movement.checked = true;
            if let Some((packet, geometry)) = self
                .movement_second
                .as_mut()
                .zip(self.geometry_second.as_mut())
            {
                let _ = geometry.begin(memory, base, state);
                let b = carmy::capture_movement_inputs(
                    memory,
                    base,
                    state,
                    second,
                    second_outcome,
                    self.movement.rows.len(),
                    self.paths.first.len(),
                    geometry,
                    packet,
                );
                self.paths.required = self.paths.required.max(b.required_paths);
                self.movement.mismatches = usize::from(
                    !self.movement_packet.same_observation(packet)
                        || !self.geometry.same_observation(geometry)
                        || !same_seed_observation(
                            &sources,
                            self.movement_packet.consumed_seed_fields(),
                            packet.consumed_seed_fields(),
                        ),
                );
                pass[0] = full_match(a.outcome, b.outcome, self.movement.mismatches);
            } else {
                self.movement.mismatches = 1;
            }
        }
        let movement_ticks = qpc().saturating_sub(start).max(0) as u64;
        let start = qpc();
        self.status.outcome = self.status_packet.capture(
            memory,
            base,
            state,
            first,
            first_outcome,
            self.status.rows.len(),
        );
        if needed[1] {
            self.status.checked = true;
            if let Some(packet) = &mut self.status_second {
                let b = packet.capture(
                    memory,
                    base,
                    state,
                    second,
                    second_outcome,
                    self.status.rows.len(),
                );
                self.status.mismatches = usize::from(
                    !self.status_packet.same_observation(packet)
                        || !same_seed_observation(
                            &sources,
                            self.status_packet.consumed_seed_masks(),
                            packet.consumed_seed_masks(),
                        ),
                );
                pass[1] = full_match(self.status.outcome, b, self.status.mismatches);
            } else {
                self.status.mismatches = 1;
            }
        }
        self.decode_pending = true;
        (
            pass,
            [movement_ticks, qpc().saturating_sub(start).max(0) as u64],
        )
    }
    pub(crate) fn consumed_seed_masks(&self, pass: bool) -> [&[u64]; N] {
        if !self.captured_current {
            return [&[], &[]];
        }
        if pass {
            [
                self.movement_second
                    .as_ref()
                    .filter(|_| self.movement.checked)
                    .map_or(&[], |packet| packet.consumed_seed_fields()),
                self.status_second
                    .as_ref()
                    .filter(|_| self.status.checked)
                    .map_or(&[], |packet| packet.consumed_seed_masks()),
            ]
        } else {
            [
                self.movement_packet.consumed_seed_fields(),
                self.status_packet.consumed_seed_masks(),
            ]
        }
    }
    /// The caller checks the union once after live readers join. Attribute a
    /// changed field only to tables which actually consumed that field.
    pub(crate) fn reject_changed_seeds(&mut self, pass: bool, changed: &[u64]) -> [bool; N] {
        let active = if pass {
            [self.movement.checked, self.status.checked]
        } else {
            [self.captured_current; N]
        };
        let masks = self.consumed_seed_masks(pass);
        let valid = std::array::from_fn(|index| {
            let consumed = masks[index];
            !active[index]
                || (consumed.len() == self.source_lens[usize::from(pass)]
                    && consumed.len() == changed.len()
                    && consumed
                        .iter()
                        .zip(changed)
                        .all(|(consumed, changed)| consumed & changed == 0))
        });
        for (rejected, valid) in self.seed_rejected.iter_mut().zip(valid) {
            *rejected |= !valid;
        }
        if !valid[0] {
            if !matches!(self.movement.outcome, Outcome::Unavailable { .. }) {
                self.movement.outcome = Outcome::Unavailable {
                    reason: "division movement fixed inputs changed or unreadable",
                };
            }
            self.movement.mismatches = self.movement.mismatches.saturating_add(1);
        }
        if !valid[1] {
            if !matches!(self.status.outcome, Outcome::Unavailable { .. }) {
                self.status.outcome = Outcome::Unavailable {
                    reason: "division status fixed inputs changed or unreadable",
                };
            }
            self.status.mismatches = self.status.mismatches.saturating_add(1);
        }
        valid
    }
    pub(crate) fn reject_source_census(&mut self) -> [bool; N] {
        let valid = [!self.movement.checked, !self.status.checked];
        for (rejected, valid) in self.seed_rejected.iter_mut().zip(valid) {
            *rejected |= !valid;
        }
        if !valid[0] {
            self.movement.mismatches = self.movement.mismatches.saturating_add(1);
            if !matches!(self.movement.outcome, Outcome::Unavailable { .. }) {
                self.movement.outcome = Outcome::Unavailable {
                    reason: "division presentation source census changed during double read",
                };
            }
        }
        if !valid[1] {
            self.status.mismatches = self.status.mismatches.saturating_add(1);
            if !matches!(self.status.outcome, Outcome::Unavailable { .. }) {
                self.status.outcome = Outcome::Unavailable {
                    reason: "division presentation source census changed during double read",
                };
            }
        }
        valid
    }
    /// Pure decode-worker projection. Publication and cold consistency already
    /// depend on the acquired packets, never on stale pooled output rows.
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
        if !self.seed_rejected[0] {
            self.movement.outcome = movement.outcome;
        }
        self.paths.len = movement.paths_len;
        self.paths.required = self.paths.required.max(movement.required_paths);
        let movement_ticks = qpc().saturating_sub(started).max(0) as u64;
        let started = qpc();
        let status = self.status_packet.project(&mut self.status.rows);
        if !self.seed_rejected[1] {
            self.status.outcome = status;
        }
        [movement_ticks, qpc().saturating_sub(started).max(0) as u64]
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
                training_requested: r.training_requested,
                exercising: r.exercising,
                has_order: r.has_order,
                no_orders: r.no_orders,
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

    #[test]
    fn presentation_packet_checks_precede_pure_decode_and_rejection_survives_it() {
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
        let mut capture = Captured::with_capacities(&capacity, &[0, 0]);
        capture.reserve_sources(1);
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
        let first = [seed];
        let mut second = first;
        // Status admits no row, so its zero mask must ignore this handle.
        second[0].division = 8;
        second[0].handle = Some(8);
        let sources = Sources {
            first: &first,
            first_outcome: Outcome::Rows { len: 1 },
            second: &second,
            second_outcome: Outcome::Rows { len: 1 },
        };
        assert!(same_seed_observation(&sources, &[0], &[0]));
        assert!(!same_seed_observation(
            &sources,
            &[DivisionSeed::HANDLE],
            &[0]
        ));

        for reject in [false, true] {
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
            capture.movement.rows[0].division = 999;
            let (matched, _) = capture.capture(
                &engine::memory::SliceMemory {
                    base: 0x1000,
                    bytes: &bytes,
                },
                0,
                0x1000,
                &[true, true],
                Sources {
                    first: &first,
                    first_outcome: Outcome::Rows { len: 1 },
                    second: &first,
                    second_outcome: Outcome::Rows { len: 1 },
                },
            );
            assert_eq!(matched, [true, false]);
            assert_eq!(capture.checked_mismatches(), [0, 0]);
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
                capture.table_inputs(None).iter().all(|input| matches!(
                    input,
                    TableInput::Sampled(TableView::Unavailable { .. })
                ))
            );
            assert_eq!(capture.consumed_seed_masks(false)[1], &[0]);
            if reject {
                assert_eq!(
                    capture.reject_changed_seeds(false, &[DivisionSeed::MOVEMENT_NUMERATOR]),
                    [false, true]
                );
            }
            bytes.fill(0xff);
            capture.decode_inputs();
            assert_eq!(capture.movement.rows[0].division, 7);
            assert_eq!(capture.movement.rows[0].numerator, 13);
            assert_eq!(
                matches!(capture.outcomes()[0], Outcome::Unavailable { .. }),
                reject
            );
            assert_eq!(
                capture.decode_inputs(),
                [0, 0],
                "projection runs once per capture"
            );
            capture.reset();
            assert!(
                capture
                    .consumed_seed_masks(false)
                    .iter()
                    .all(|masks| masks.is_empty())
            );
            assert!(
                capture
                    .consumed_seed_masks(true)
                    .iter()
                    .all(|masks| masks.is_empty())
            );
        }
    }

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
                    tables: baseline,
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
        assert!(matches!(
            movement,
            TableInput::Sampled(TableView::Complete(_))
        ));
        assert!(matches!(status, TableInput::ComparedEqual(_)));
        capture.paths.first[3] ^= 1;
        capture.status.rows[0].no_orders = 1;
        let [movement, status] = capture.table_inputs(Some(&mut projection));
        assert!(matches!(movement, TableInput::ComparedEqual(_)));
        assert!(matches!(
            status,
            TableInput::Sampled(TableView::Complete(_))
        ));
        capture.status.rows[0].no_orders = 0;
        capture.movement.rows[0].alternate_start = usize::MAX;
        let [movement, status] = capture.table_inputs(Some(&mut projection));
        assert!(matches!(
            movement,
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
        assert!(matches!(status, TableInput::ComparedEqual(_)));
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
            ..Capacities::default()
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
        capture.decode_inputs();
        assert!(
            capture
                .tables()
                .iter()
                .all(|t| matches!(t, TableView::Unavailable { .. }))
        );
        capture.paths.required = 30;
        capture.grow(&mut capacity, &[200, 0]);
        assert!(capture.movement_second.is_none() && capture.geometry_second.is_none());
        assert!(capture.status_second.is_some());
        assert!(
            capture.movement.second.is_empty()
                && capture.status.second.is_empty()
                && capture.paths.second.is_empty()
        );
        assert!(capture.paths.first.len() >= 30);
        capture.grow(&mut capacity, &[200, 200]);
        assert!(capture.status_second.is_none());
        capture.reset();
        assert_eq!(capture.checked_mismatches(), [0, 0]);
    }
}
