use collector_primitives::{
    clock::qpc,
    table_capture::{CaptureRows, grow_capacity, grow_vec},
};
use collector_projection::projection::{Projection, project_table};
#[cfg(test)]
use collector_projection::table_conversion::table;
use engine_allocation::{default_vec, owned_vec};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::{carmy_leader, corders_group};
use replay_data::{
    rows::CommanderAttackStats, rows::CommanderMovementStats, rows::OrdersGroupCounterInputRow,
    rows::Recorded, rows::TableId, rows::Witnessed, rows::tables,
};
use replay_frame::frame::TableInput;
#[cfg(test)]
use replay_frame::frame::TableView;
use std::mem::size_of;
#[cfg(test)]
mod startup_tests;
mod templates;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::OrdersGroupCounterInputs];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
}

collector_primitives::capacity_max!(Capacities { rows });

impl Capacities {
    /// # Safety
    /// Run only during attach or the active session's serial sizing pass against the authenticated
    /// session image, while HQ-group observer storage remains live and teardown is excluded.
    pub unsafe fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        _state: usize,
    ) -> Result<(), &'static str> {
        let count = match corders_group::count_roots(memory, base) {
            Outcome::Rows { len } => len,
            Outcome::Truncated { len, unread_rows } => len
                .checked_add(unread_rows as usize)
                .ok_or("orders group demand overflow")?,
            Outcome::Unavailable { reason } => return Err(reason),
        };
        self.rows = self.rows.max(count);
        crate::template_changes::prepare_hq_groups(count);
        collector_observers::retreat_ban_changes::prepare(count);
        collector_observers::orders_group_changes::prepare(count);
        Ok(())
    }
}

pub struct Captured {
    rows: CaptureRows<corders_group::CounterInputsRow>,
    roots: Vec<corders_group::Root>,
    commanders: Vec<(u64, Option<Option<carmy_leader::AttackStats>>)>,
    commander_pool: Vec<carmy_leader::AttackStats>,
    required: usize,
    templates: templates::Store,
    refresh_templates: bool,
    owned_pending: bool,
}

fn commander(
    memory: &impl Memory,
    base: usize,
    root: corders_group::Root,
    pool: &mut Vec<carmy_leader::AttackStats>,
) -> Option<Option<carmy_leader::AttackStats>> {
    let leader = root
        .pointer()
        .checked_add(corders_group::LEADER)
        .and_then(|address| engine_native_memory::read::<u64>(memory, address))
        .and_then(|leader| usize::try_from(leader).ok())?;
    if leader == 0 {
        return Some(None);
    }
    let mut stats = pool.pop().unwrap_or_default();
    match carmy_leader::attack_stats_into(memory, base, leader, &mut stats) {
        Ok(()) => Some(Some(stats)),
        Err(_) => {
            stats.terrain.clear();
            stats.movement_terrain.clear();
            pool.push(stats);
            None
        }
    }
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
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities { rows: 0 })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            roots: owned_vec(default_vec(capacity.rows)),
            commanders: Vec::new(),
            commander_pool: Vec::new(),
            required: 0,
            templates: templates::Store::default(),
            refresh_templates: false,
            owned_pending: false,
        }
    }
    fn recycle_commanders(&mut self) {
        for (_, entry) in self.commanders.drain(..) {
            if let Some(Some(mut stats)) = entry {
                stats.terrain.clear();
                stats.movement_terrain.clear();
                self.commander_pool.push(stats);
            }
        }
    }
    pub fn reset(&mut self) {
        self.decode_owned();
        self.rows.reset();
        self.roots.fill(Default::default());
        self.recycle_commanders();
        self.required = 0;
    }
    pub fn prepare_hour(&mut self, force: bool) {
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
            + self.commander_pool.capacity() * size_of::<carmy_leader::AttackStats>()
            + self
                .commander_pool
                .iter()
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
        let source = corders_group::walk_roots(memory, base, &mut self.roots);
        if !matches!(source, Outcome::Unavailable { .. })
            && let Err(reason) =
                engine_native_memory::walk::Buffer::Growing(&mut self.rows.rows).ensure(source.len())
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
    /// # Safety
    /// Caller holds hourly capture admission for the active session (or isolated fixture control)
    /// so global `template_changes` HQ-group demand updates belong to this pass.
    #[cfg(any(test, feature = "test-support"))]
    pub unsafe fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let begin = qpc();
        // SAFETY: Caller upholds the hourly capture admission contract.
        unsafe { self.acquire(memory, base, state) };
        self.decode_owned();
        [qpc().saturating_sub(begin).max(0) as u64]
    }

    /// # Safety
    /// Caller holds hourly capture admission for the active session (or isolated fixture control)
    /// so global `template_changes` HQ-group demand updates belong to this pass.
    pub unsafe fn acquire(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let _reads = collector_memory::read_accounting::table(TABLES[0]);
        let begin = qpc();
        if std::mem::take(&mut self.refresh_templates) {
            self.templates.forget();
        }
        self.rows.outcome = self.read(memory, base, state);
        self.recycle_commanders();
        let len = self.rows.outcome.len();
        crate::template_changes::record_hq_groups(len);
        for (row, root) in self.rows.rows[..len].iter().zip(&self.roots) {
            let _ = self
                .templates
                .read(memory, base, *root, &crate::template_changes::Observer);
            self.commanders
                .push((row.group, commander(memory, base, *root, &mut self.commander_pool)));
        }
        if matches!(self.rows.outcome, Outcome::Rows { .. }) {
            self.templates.prune(&self.roots[..len]);
        }
        self.owned_pending = true;
        [qpc().saturating_sub(begin).max(0) as u64]
    }

    pub fn decode_owned(&mut self) {
        if !std::mem::take(&mut self.owned_pending) {
            return;
        }
        self.commanders.sort_unstable_by_key(|(group, _)| *group);
        self.roots.fill(Default::default());
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }

    fn commander_entry(
        &self,
        group: u64,
    ) -> Option<Option<&carmy_leader::AttackStats>> {
        let at = self
            .commanders
            .binary_search_by_key(&group, |(group, _)| *group)
            .ok()?;
        self.commanders[at].1.as_ref().map(|c| c.as_ref())
    }

    fn commander_attack(&self, group: u64) -> Option<Option<CommanderAttackStats>> {
        self.commander_entry(group)
            .map(|commander| commander.map(recorded))
    }

    fn commander_movement(&self, group: u64) -> Option<Option<CommanderMovementStats>> {
        self.commander_entry(group)
            .map(|commander| commander.map(recorded_movement))
    }

    fn row_equal(
        &self,
        r: &corders_group::CounterInputsRow,
        previous: &OrdersGroupCounterInputRow,
    ) -> bool {
        if previous.group != r.group
            || previous.withdrawing != r.withdrawing
            || previous.attack != Recorded::Value(Witnessed(None))
            || previous.hq_distance != Recorded::Value(r.hq_distance)
            || previous.leader_enroute_days != Recorded::Value(r.leader_enroute_days)
            || previous.border_war_presence != Recorded::Value(r.border_war_presence)
            || previous.invasion_source_distance != Recorded::Value(r.invasion_source_distance)
            || previous.hq_nearest_front != Recorded::Value(r.hq_nearest_front)
        {
            return false;
        }
        let commander_eq = match (
            self.commander_entry(r.group),
            &previous.commander_attack,
            &previous.commander_movement,
        ) {
            (Some(Some(c)), Recorded::Value(Some(Some(pa))), Recorded::Value(Some(Some(pm)))) => {
                pa.leader == c.leader
                    && pa.fort == c.fort
                    && pa.river == c.river
                    && pa.snow == c.snow
                    && pa.night == c.night
                    && pa.terrain == c.terrain
                    && pm.terrain == c.movement_terrain
            }
            (Some(None), Recorded::Value(Some(None)), Recorded::Value(Some(None))) => true,
            (None, Recorded::Value(None), Recorded::Value(None)) => true,
            _ => false,
        };
        if !commander_eq {
            return false;
        }
        match (self.templates.value_ref(r.group), &previous.template_context) {
            (Some(cur), Recorded::Value(Some(prev))) => {
                std::sync::Arc::ptr_eq(cur, prev) || cur == prev
            }
            (None, Recorded::Value(None)) => true,
            _ => false,
        }
    }

    pub fn table_inputs(&self, projection: Option<&mut Projection<'_>>) -> [TableInput; N] {
        [project_table::<tables::OrdersGroupCounterInputs, _>(
            projection,
            self.outcomes()[0],
            &self.rows.rows,
            |r| u128::from(r.group),
            |r, previous| self.row_equal(r, previous),
            |r| OrdersGroupCounterInputRow {
                group: r.group,
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

    #[cfg(test)]
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::OrdersGroupCounterInputs, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| OrdersGroupCounterInputRow {
                group: r.group,
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

    use engine_native_memory::memory::SliceMemory;
    #[test]
    fn missing_registry_and_capacity_pressure_preserve_primary_rows() {
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut capacity = Capacities { rows: 1 };
        let mut capture = Captured::with_capacities(&capacity);
        // SAFETY: Unit test runs against an isolated in-memory fixture.
        unsafe { capture.capture(&memory, 0, 0) };
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
        assert_eq!(rows[0].attack, Recorded::Value(Witnessed(None)));
        assert_eq!(rows[0].commander_attack, Recorded::Value(None));
        assert_eq!(rows[0].commander_movement, Recorded::Value(None));
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
        capture.required = 9;
        capture.grow(&mut capacity);
        assert!(capacity.rows >= 9 && capture.roots.len() >= 9);
    }
}
