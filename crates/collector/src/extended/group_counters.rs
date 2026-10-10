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

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    attack: super::attack_inputs::GroupCapacities,
}

impl Capacities {
    pub(crate) fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        _state: usize,
    ) -> Result<(), &'static str> {
        let count = match corders_group::walk_roots(memory, base, &mut []) {
            Outcome::Rows { len } => len,
            Outcome::Truncated { len, unread_rows } => len
                .checked_add(unread_rows as usize)
                .ok_or("orders group demand overflow")?,
            Outcome::Unavailable { reason } => return Err(reason),
        };
        self.rows = self.rows.max(count);
        self.attack.0.rows = self.attack.0.rows.max(count);
        Ok(())
    }
}

pub struct Captured {
    rows: CaptureRows<corders_group::CounterInputsRow>,
    roots: Vec<corders_group::Root>,
    commanders: Vec<corders_group::DirectCommander>,
    root_outcome: Outcome,
    pub(super) attack: super::attack_inputs::Groups,
    required: usize,
}
impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            attack: super::attack_inputs::GroupCapacities(
                engine::selected_attack::GroupCapacity::default(),
            ),
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            roots: owned_vec(default_vec(capacity.rows)),
            commanders: owned_vec(default_vec(capacity.rows)),
            root_outcome: Outcome::Unavailable {
                reason: "orders group roots not acquired",
            },
            attack: super::attack_inputs::Groups::new(capacity.attack),
            required: 0,
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.roots.fill(Default::default());
        self.commanders.fill(Default::default());
        self.root_outcome = Outcome::Unavailable {
            reason: "orders group roots not acquired",
        };
        self.attack.reset();
        self.required = 0;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.roots.capacity() * size_of::<corders_group::Root>()
            + self.commanders.capacity() * size_of::<corders_group::DirectCommander>()
            + self.attack.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        grow_capacity(&mut capacity.rows, self.required);
        self.rows.grow(&mut capacity.rows);
        grow_vec(&mut self.roots, capacity.rows);
        grow_vec(&mut self.commanders, capacity.rows);
        self.attack.grow(&mut capacity.attack, capacity.rows);
    }
    fn read(&mut self, memory: &impl Memory, base: usize, state: usize) -> Outcome {
        let source = corders_group::walk_roots_growing(memory, base, &mut self.roots);
        self.root_outcome = source;
        if !matches!(source, Outcome::Unavailable { .. }) {
            let prepared = (|| {
                engine::walk::Buffer::Growing(&mut self.rows.rows).ensure(source.len())?;
                engine::walk::Buffer::Growing(&mut self.commanders).ensure(source.len())?;
                self.attack
                    .packet
                    .reserve(engine::selected_attack::GroupCapacity {
                        rows: source.len(),
                        values: 0,
                    })
                    .map_err(|error| {
                        crate::allocation::observe(&error);
                        "group attack allocation failed"
                    })?;
                Ok::<_, &'static str>(())
            })();
            if let Err(reason) = prepared {
                self.root_outcome = Outcome::Unavailable { reason };
                return self.root_outcome;
            }
        }
        if let Outcome::Truncated { len, unread_rows } = source {
            self.required = self.required.max(len + unread_rows as usize);
        }
        corders_group::walk_counter_inputs_with_commanders(
            memory,
            base,
            state,
            &self.roots,
            source,
            &mut self.rows.rows,
            &mut self.commanders,
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
        // Root/direct-leader controls remain held until the final joined
        // commander consumer; no pointer is used by decode or spooling.
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

    pub(super) fn capture_attack(
        &mut self,
        memory: &impl Memory,
        base: usize,
        hour: i32,
        units: &mut super::attack_inputs::Units,
        geometry: &mut engine::cprovince::GeometryPacket,
        current_geometry: engine::selected_attack::Field<()>,
    ) {
        self.attack.capture(
            memory,
            base,
            hour,
            &self.commanders,
            self.root_outcome,
            units,
            geometry,
            current_geometry,
        );
        self.roots.fill(Default::default());
        self.commanders.fill(Default::default());
    }

    pub fn tables(&self) -> [TableView; N] {
        self.tables_attack(Some(&self.attack))
    }
    fn tables_attack(&self, attack: Option<&super::attack_inputs::Groups>) -> [TableView; N] {
        [table::<tables::OrdersGroupCounterInputs, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| OrdersGroupCounterInputRow {
                group: r.group,
                attack: replay::rows::Recorded::Value(
                    attack
                        .map(|attack| attack.group(r.group))
                        .unwrap_or_default(),
                ),
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
        let mut capacity = Capacities {
            rows: 1,
            attack: Default::default(),
        };
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

    fn joined_group_fixture(
        capture: &mut Captured,
        group: u64,
        leader: Result<Option<u64>, &'static str>,
    ) {
        engine::walk::Buffer::Growing(&mut capture.rows.rows)
            .ensure(1)
            .unwrap();
        engine::walk::Buffer::Growing(&mut capture.roots)
            .ensure(2)
            .unwrap();
        engine::walk::Buffer::Growing(&mut capture.commanders)
            .ensure(2)
            .unwrap();
        capture.rows.rows[0] = corders_group::CounterInputsRow {
            group,
            withdrawing: 255,
            hq_distance: Some(i32::MIN),
            leader_enroute_days: Some(-3),
            border_war_presence: Some(1),
            invasion_source_distance: Some(-17),
        };
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.root_outcome = Outcome::Rows { len: 1 };
        capture.roots[0].handle = group;
        capture.roots[0].country = 3;
        capture.commanders[0] = corders_group::DirectCommander {
            group,
            pointer: Ok(0),
            leader,
            enroute_days: Ok(-3),
        };
        // This stale tail is outside the admitted census. Retirement still owns it.
        capture.roots[1].handle = (99_u64 << 32) | u64::from(corders_group::REFERENCE_TYPE);
        capture.commanders[1] = corders_group::DirectCommander {
            group: capture.roots[1].handle,
            pointer: Ok(0xdead_beef),
            leader: Ok(Some(
                (99_u64 << 32) | u64::from(engine::cunit_leader::REFERENCE_TYPE),
            )),
            enroute_days: Ok(7),
        };
    }

    #[test]
    fn direct_null_and_failed_commander_project_differently_and_retire_the_whole_control_buffer() {
        struct NoGroupReads;
        impl Memory for NoGroupReads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("joined null/refused commander controls must not be reacquired")
            }
        }
        let group = (5_u64 << 32) | u64::from(corders_group::REFERENCE_TYPE);
        let capacity = Capacities {
            rows: 2,
            attack: super::super::attack_inputs::GroupCapacities(
                engine::selected_attack::GroupCapacity { rows: 2, values: 0 },
            ),
        };
        let mut capture = Captured::with_capacities(&capacity);
        let mut units = super::super::attack_inputs::Units::vacant();
        let mut geometry = engine::cprovince::GeometryPacket::vacant();
        for (hour, observed, expected) in [
            (71, Ok(None), Some(None)),
            (72, Err("direct commander unreadable"), None),
        ] {
            joined_group_fixture(&mut capture, group, observed);
            let allocated = capture.allocated_bytes();
            capture.capture_attack(
                &NoGroupReads,
                0,
                hour,
                &mut units,
                &mut geometry,
                Err("current geometry deliberately unavailable"),
            );
            assert_eq!(capture.attack.packet.outcome(), Outcome::Rows { len: 1 });
            assert!(
                capture
                    .roots
                    .iter()
                    .all(|root| *root == corders_group::Root::default())
            );
            assert!(
                capture
                    .commanders
                    .iter()
                    .all(|control| *control == corders_group::DirectCommander::default())
            );
            assert_eq!(capture.allocated_bytes(), allocated);
            let [TableView::Complete(replay::rows::TableRows::OrdersGroupCounterInputs(rows))] =
                capture.tables()
            else {
                panic!("independent primary group row")
            };
            assert_eq!(
                rows[0].attack,
                Recorded::Value(replay::rows::OrdersGroupAttackInputs {
                    sample_hour: hour,
                    commander: expected,
                })
            );
            assert_eq!(
                (
                    rows[0].withdrawing,
                    rows[0].hq_distance,
                    rows[0].leader_enroute_days,
                    rows[0].border_war_presence,
                    rows[0].invasion_source_distance
                ),
                (
                    255,
                    Recorded::Value(Some(i32::MIN)),
                    Recorded::Value(Some(-3)),
                    Recorded::Value(Some(1)),
                    Recorded::Value(Some(-17))
                )
            );
        }
    }

    #[test]
    fn capture_tables_all_executes_the_final_group_consumer_and_current_hour_spooling() {
        // Completed worker markers retain the same-census controls for the actual
        // final capture_tables_all call. We do not call capture_attack_map ourselves.
        let mut all = super::super::Captured::with_capacities(&super::super::Capacities::default());
        let group = (7_u64 << 32) | u64::from(corders_group::REFERENCE_TYPE);
        joined_group_fixture(&mut all.group_counters, group, Ok(None));
        all.install_unit_inputs(Some(std::sync::Arc::new(
            super::super::UnitInputs::empty_rows(),
        )));
        all.ordinary_ticks = Some([0; replay::frame::TABLE_COUNT]);
        all.late_ticks = Some([0; replay::frame::TABLE_COUNT]);
        all.presentation_report = Some(super::super::division_presentation::CaptureReport {
            ticks: [0; super::super::division_presentation::N],
        });
        all.division_presentation.stamp_attack_epoch(93, true);
        all.set_attack_guard(Err("fixture fixed modifier layout refused"));
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let _ticks = all.capture_tables_all(&memory, 0, 0x1000, 93, false);
        assert!(
            all.presentation_report.is_none(),
            "actual table routing consumed the completed presentation report"
        );
        assert_eq!(
            all.group_counters.attack.packet.outcome(),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(
            all.group_counters.attack.group(group),
            replay::rows::OrdersGroupAttackInputs {
                sample_hour: 93,
                commander: Some(None),
            }
        );
        assert!(
            all.group_counters
                .roots
                .iter()
                .all(|root| *root == corders_group::Root::default())
        );
        assert!(
            all.group_counters
                .commanders
                .iter()
                .all(|control| *control == corders_group::DirectCommander::default())
        );
        let [TableView::Complete(replay::rows::TableRows::OrdersGroupCounterInputs(rows))] =
            all.group_counters.tables()
        else {
            panic!("joined primary row remains complete")
        };
        assert_eq!(
            rows[0].attack,
            Recorded::Value(replay::rows::OrdersGroupAttackInputs {
                sample_hour: 93,
                commander: Some(None),
            })
        );
        assert_eq!(rows[0].withdrawing, 255);
        assert_eq!(
            all.division_presentation
                .attack
                .division((2_u64 << 32) | 51)
                .sample_hour,
            93
        );
        assert!(
            all.division_presentation
                .attack
                .packet
                .originals()
                .is_empty()
        );
        assert_eq!(
            all.province_attack(1).sample_hour,
            93,
            "the final map suffix also receives the actual assembly hour"
        );
    }
}
