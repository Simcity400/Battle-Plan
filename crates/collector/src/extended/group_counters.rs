//! Hourly COrdersGroup withdrawing byte over its own bounded registry census, and each group's
//! direct commander's attack operands (`corders_group::LEADER`,
//! `engine_native::carmy_leader::attack_stats`), which the selected army's attack list reads through its
//! direct commander lookup.
use crate::{
    sampling::qpc,
    spooling::table,
    table_capture::{CaptureRows, grow_capacity, grow_vec},
};
use engine_allocation::{default_vec, owned_vec};
use engine_native::{Memory, carmy_leader, corders_group, walk::Outcome};
use replay_data::{
    rows::CommanderAttackStats, rows::CommanderMovementStats, rows::OrdersGroupCounterInputRow,
    rows::Recorded, rows::TableId, rows::Witnessed, rows::tables,
};
use replay_frame::frame::TableView;
use std::mem::size_of;
#[cfg(test)]
mod startup_tests;
mod templates;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::OrdersGroupCounterInputs];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

static COOLDOWN_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static BORDER_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static INVASION_DISTANCE_REFUSALS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
static HQ_NEAREST_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static HQ_CONTEXT_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static HQ_CONTEXT_REASON: std::sync::Mutex<Option<&'static str>> = std::sync::Mutex::new(None);
static COMMANDER_ATTACK_REFUSALS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

pub(crate) fn tooltip_data() -> impl serde::Serialize {
    crate::status::status!({
        "leader_enroute_days_refused": COOLDOWN_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "border_war_presence_refused": BORDER_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "invasion_source_distance_refused": INVASION_DISTANCE_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "hq_nearest_front_refused": HQ_NEAREST_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "hq_template_context_refused": HQ_CONTEXT_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "hq_template_context_last_refusal": *HQ_CONTEXT_REASON.lock().unwrap_or_else(std::sync::PoisonError::into_inner),
        "commander_attack_refused": COMMANDER_ATTACK_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "scope": "cumulative independent orders-group tooltip input refusals; existing withdrawing and HQ-distance operands remain available",
    })
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
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
        crate::template_changes::prepare_hq_groups(count);
        crate::retreat_ban_changes::prepare(count);
        Ok(())
    }
}

pub struct Captured {
    rows: CaptureRows<corders_group::CounterInputsRow>,
    roots: Vec<corders_group::Root>,
    /// Each row's direct commander's attack operands by group handle:
    /// `Some(None)` without a commander, `None` for a read refusal.
    commanders: Vec<(u64, Option<Option<carmy_leader::AttackStats>>)>,
    required: usize,
    templates: templates::Store,
    refresh_templates: bool,
}

/// The direct commander lookup for a group and its
/// attack operands.
fn commander(
    memory: &impl Memory,
    base: usize,
    root: corders_group::Root,
) -> Option<Option<carmy_leader::AttackStats>> {
    let leader = root
        .pointer()
        .checked_add(corders_group::LEADER)
        .and_then(|address| engine_native::read::<u64>(memory, address))
        .and_then(|leader| usize::try_from(leader).ok())?;
    if leader == 0 {
        return Some(None);
    }
    carmy_leader::attack_stats(memory, base, leader)
        .ok()
        .map(Some)
}

fn recorded(commander: &carmy_leader::AttackStats) -> CommanderAttackStats {
    CommanderAttackStats {
        leader: commander.leader,
        terrain: commander.terrain.clone(),
        fort: commander.fort,
        river: commander.river,
        snow: commander.snow,
        night: commander.night,
    }
}

fn recorded_movement(commander: &carmy_leader::AttackStats) -> CommanderMovementStats {
    CommanderMovementStats {
        terrain: commander.movement_terrain.clone(),
    }
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
            commanders: Vec::new(),
            required: 0,
            templates: templates::Store::default(),
            refresh_templates: false,
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.roots.fill(Default::default());
        self.commanders.clear();
        self.required = 0;
    }
    pub(crate) fn prepare_hour(&mut self, force: bool) {
        self.refresh_templates |= force;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.templates.allocated_bytes()
            + self.roots.capacity() * size_of::<corders_group::Root>()
            + self.commanders.capacity()
                * size_of::<(u64, Option<Option<carmy_leader::AttackStats>>)>()
            + self
                .commanders
                .iter()
                .filter_map(|(_, commander)| commander.as_ref()?.as_ref())
                .map(|commander| {
                    (commander.terrain.capacity() + commander.movement_terrain.capacity()) * 8
                })
                .sum::<usize>()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        grow_capacity(&mut capacity.rows, self.required);
        self.rows.grow(&mut capacity.rows);
        grow_vec(&mut self.roots, capacity.rows);
    }
    fn read(&mut self, memory: &impl Memory, base: usize, state: usize) -> Outcome {
        let source = corders_group::walk_roots_growing(memory, base, &mut self.roots);
        if !matches!(source, Outcome::Unavailable { .. })
            && let Err(reason) =
                engine_native::walk::Buffer::Growing(&mut self.rows.rows).ensure(source.len())
        {
            return Outcome::Unavailable { reason };
        }
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
        if std::mem::take(&mut self.refresh_templates) {
            self.templates.forget();
        }
        self.rows.outcome = self.read(memory, base, state);
        // The census rows follow the roots one for one.
        self.commanders.clear();
        let len = self.rows.outcome.len();
        crate::template_changes::record_hq_groups(len);
        for (row, root) in self.rows.rows[..len].iter().zip(&self.roots) {
            if row.hq_nearest_front.is_none() {
                HQ_NEAREST_REFUSALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            if let Err(reason) =
                self.templates
                    .read(memory, base, *root, &crate::template_changes::Observer)
            {
                HQ_CONTEXT_REFUSALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                *HQ_CONTEXT_REASON
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reason);
            }
            let commander = commander(memory, base, *root);
            if commander.is_none() {
                COMMANDER_ATTACK_REFUSALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            self.commanders.push((row.group, commander));
        }
        self.commanders.sort_unstable_by_key(|(group, _)| *group);
        if matches!(self.rows.outcome, Outcome::Rows { .. }) {
            self.templates.prune(&self.roots[..len]);
        }
        // No later consumer borrows the root controls.
        self.roots.fill(Default::default());
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

    fn commander_attack(&self, group: u64) -> Option<Option<CommanderAttackStats>> {
        let at = self
            .commanders
            .binary_search_by_key(&group, |(group, _)| *group)
            .ok()?;
        self.commanders[at]
            .1
            .as_ref()
            .map(|commander| commander.as_ref().map(recorded))
    }

    fn commander_movement(&self, group: u64) -> Option<Option<CommanderMovementStats>> {
        let at = self
            .commanders
            .binary_search_by_key(&group, |(group, _)| *group)
            .ok()?;
        self.commanders[at]
            .1
            .as_ref()
            .map(|commander| commander.as_ref().map(recorded_movement))
    }

    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::OrdersGroupCounterInputs, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| OrdersGroupCounterInputRow {
                group: r.group,
                // No commander operand is witnessed; current writers leave it empty.
                attack: Recorded::Value(Witnessed(None)),
                commander_attack: Recorded::Value(self.commander_attack(r.group)),
                commander_movement: Recorded::Value(self.commander_movement(r.group)),
                withdrawing: r.withdrawing,
                hq_distance: Recorded::Value(r.hq_distance),
                leader_enroute_days: Recorded::Value(r.leader_enroute_days),
                border_war_presence: Recorded::Value(r.border_war_presence),
                invasion_source_distance: Recorded::Value(r.invasion_source_distance),
                hq_nearest_front: Recorded::Value(r.hq_nearest_front),
                template_context: Recorded::Value(self.templates.value(r.group)),
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::memory::SliceMemory;
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
            hq_nearest_front: Some(99),
        };
        capture.rows.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(replay_data::rows::TableRows::OrdersGroupCounterInputs(rows))] =
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
        assert_eq!(rows[0].hq_nearest_front, Recorded::Value(Some(99)));
        // No commander was read: the operands are refused, never zero.
        assert_eq!(rows[0].attack, Recorded::Value(Witnessed(None)));
        assert_eq!(rows[0].commander_attack, Recorded::Value(None));
        assert_eq!(rows[0].commander_movement, Recorded::Value(None));
        // A read commander publishes its terrain movement beside its attack.
        capture.commanders = vec![(
            (5 << 32) | 53,
            Some(Some(carmy_leader::AttackStats {
                leader: 9,
                terrain: vec![1, 2],
                movement_terrain: vec![-3, 4],
                ..Default::default()
            })),
        )];
        let [TableView::Complete(replay_data::rows::TableRows::OrdersGroupCounterInputs(rows))] =
            capture.tables()
        else {
            panic!("complete primary row");
        };
        assert_eq!(
            rows[0].commander_movement,
            Recorded::Value(Some(Some(CommanderMovementStats {
                terrain: vec![-3, 4],
            })))
        );
        capture.commanders.clear();
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
