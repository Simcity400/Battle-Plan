//! Hourly land combat losses, sharing the existing land-combat census.
use crate::{
    spooling::table_iter,
    table_capture::{Arena, CaptureRows, grow_vec},
};
use engine_native::{Memory, cland_combat, walk::Outcome};
use replay_data::{
    rows::LandCombatProgressRow, rows::LandEquipmentLoss, rows::TableId, rows::tables,
};
use replay_frame::frame::TableView;
use std::mem::size_of;

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::LandCombatProgress, TableId::CombatLogObservers];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[(
    "land combat casualty and observer operands (census charged to LandCombats)",
    0,
    &[0, 1],
)];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    variants: usize,
}
impl Capacities {
    /// At attach, or under the spool's capacity lock: live headers size all
    /// side pools, with no cap.
    pub(crate) fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
    ) -> Result<(), &'static str> {
        let state = base
            .checked_add(engine_native::cgame_state::SINGLETON_RVA)
            .and_then(|address| engine_native::read::<u64>(memory, address))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|pointer| *pointer != 0)
            .ok_or("land casualty sizing state unreadable")?;
        let (mut rows, mut variants) = (0usize, 0usize);
        cland_combat::visit_sides(memory, base, state, |sides| {
            let plans =
                engine_land_inputs::cstats_observer::LossPlan::read_sides(memory, base, sides)?;
            rows = rows.checked_add(1).ok_or("land loss row demand overflow")?;
            for plan in plans {
                variants = variants
                    .checked_add(plan.equipment.count as usize)
                    .ok_or("land loss arena demand overflow")?;
            }
            Ok(())
        })?;
        crate::table_capture::grow_capacity(&mut self.rows, rows);
        crate::table_capture::grow_capacity(&mut self.variants, variants);
        Ok(())
    }
}

pub struct Captured {
    /// The combats' full handles, in census order; playback derives their
    /// progress and direction from LandCombats, Divisions and DivisionMovement.
    rows: CaptureRows<u64>,
    losses: Vec<[engine_land_inputs::cstats_observer::LossSpan; 2]>,
    equipment: Arena<engine_land_inputs::cstats_observer::LossEntry>,
    pointers: Vec<usize>,
    identities: Vec<[u8; 16]>,
    len: usize,
    failure: Option<&'static str>,
    observers: Vec<replay_data::rows::CombatLogObserverRow>,
    observer_failure: Option<&'static str>,
}

impl Captured {
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            losses: engine_allocation::owned_vec(engine_allocation::default_vec(capacity.rows)),
            equipment: Arena::new(capacity.variants),
            pointers: engine_allocation::owned_vec(engine_allocation::default_vec(
                capacity.variants,
            )),
            identities: engine_allocation::owned_vec(engine_allocation::default_vec(
                capacity.variants,
            )),
            len: 0,
            failure: None,
            observers: Vec::new(),
            observer_failure: None,
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.equipment.reset();
        self.len = 0;
        self.failure = None;
        self.observers.clear();
        self.observer_failure = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.equipment.allocated_bytes()
            + self.losses.capacity()
                * size_of::<[engine_land_inputs::cstats_observer::LossSpan; 2]>()
            + self.pointers.capacity() * size_of::<usize>()
            + self.identities.capacity() * 16
            + self.observers.capacity() * size_of::<replay_data::rows::CombatLogObserverRow>()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        grow_vec(&mut self.losses, capacity.rows);
        capacity.variants = capacity.variants.max(self.equipment.first.len());
        self.equipment.grow(&mut capacity.variants);
        grow_vec(&mut self.pointers, capacity.variants);
        grow_vec(&mut self.identities, capacity.variants);
    }
    pub fn begin(&mut self) {
        self.equipment.reset();
        self.len = 0;
        self.failure = None;
        self.observers.clear();
        self.observer_failure = None;
    }
    pub fn visit(&mut self, memory: &impl Memory, base: usize, source: cland_combat::Source) {
        let _reads = crate::read_accounting::table(TABLES[0]);
        if self.failure.is_some() {
            return;
        }
        let [attacker, defender] = match engine_land_inputs::cstats_observer::LossPlan::read_sides(
            memory,
            base,
            source.sides,
        ) {
            Ok(plans) => plans,
            Err(reason) => {
                self.failure = Some(reason);
                return;
            }
        };
        let storage = (|| {
            let count = (attacker.equipment.count as usize)
                .checked_add(defender.equipment.count as usize)
                .ok_or("land loss arena demand overflow")?;
            self.equipment.required = self
                .equipment
                .required
                .checked_add(count)
                .ok_or("land loss arena demand overflow")?;
            let rows = self
                .len
                .checked_add(1)
                .ok_or("land loss row demand overflow")?;
            // Worker-only World acquisition owns the full combat-side pools.
            // Grow from these original LossPlan headers before reading entries.
            engine_native::walk::Buffer::Growing(&mut self.rows.rows).ensure(rows)?;
            engine_native::walk::Buffer::Growing(&mut self.losses).ensure(rows)?;
            engine_native::walk::Buffer::Growing(&mut self.equipment.first)
                .ensure(self.equipment.required)?;
            engine_native::walk::Buffer::Growing(&mut self.pointers).ensure(count)?;
            engine_native::walk::Buffer::Growing(&mut self.identities).ensure(count)?;
            Ok::<usize, &'static str>(count)
        })();
        let count = match storage {
            Ok(count) => count,
            Err(reason) => {
                self.failure = Some(reason);
                return;
            }
        };
        let start = self.equipment.len;
        if let Err(reason) = engine_land_inputs::cstats_observer::LossPlan::copy_sides(
            [attacker, defender],
            memory,
            base,
            &mut self.equipment.first[start..start + count],
            &mut self.pointers[..count],
            &mut self.identities[..count],
        ) {
            self.failure = Some(reason);
            return;
        }
        self.losses[self.len] = [
            engine_land_inputs::cstats_observer::LossSpan {
                manpower: attacker.manpower,
                start,
                len: attacker.equipment.count as usize,
            },
            engine_land_inputs::cstats_observer::LossSpan {
                manpower: defender.manpower,
                start: start + attacker.equipment.count as usize,
                len: defender.equipment.count as usize,
            },
        ];
        self.equipment.len += count;
        self.rows.rows[self.len] = source.id;
        self.len += 1;
        if self.observer_failure.is_none() {
            for (side, pointer) in source.sides.into_iter().enumerate() {
                let mut storage = Ok(());
                let read = engine_land_inputs::combat_log_events::observer(
                    memory,
                    base,
                    pointer,
                    &mut |fact| {
                        if storage.is_err() {
                            return;
                        }
                        storage = engine_allocation::reserve_push_batch(&mut self.observers, 1)
                            .map_err(|_| "combat log observer arena allocation failed");
                        if storage.is_ok() {
                            match replay_data::rows::CombatLogObserverRow::from_fact(
                                source.id,
                                side == 0,
                                fact.kind as u8,
                                fact.item,
                                fact.values,
                            ) {
                                Ok(row) => self.observers.push(row),
                                Err(reason) => storage = Err(reason),
                            }
                        }
                    },
                );
                if let Err(reason) = read.and(storage) {
                    self.observer_failure = Some(reason);
                    break;
                }
            }
        }
    }
    pub fn finish(&mut self, parent: Outcome) {
        crate::land_events::observe_observer_rows(self.observers.len());
        // This worker already counted the live side/activity operands. Retained
        // history may be empty, but those combats can publish before next hour.
        // Include the aggregate loss entries omitted from the observer packet.
        collector_combat_log::history_events::prepare(
            self.observers.len().saturating_add(self.equipment.len),
        );
        let missing = match parent {
            Outcome::Truncated { unread_rows, .. } => unread_rows,
            _ => 0,
        };
        self.rows.outcome = match (self.failure, parent) {
            (Some(reason), _) | (_, Outcome::Unavailable { reason }) => {
                Outcome::Unavailable { reason }
            }
            _ if self.len != parent.len() => Outcome::Unavailable {
                reason: "combat progress visits differ from parent census",
            },
            _ if missing != 0 => Outcome::Truncated {
                len: self.len,
                unread_rows: missing,
            },
            _ => Outcome::Rows { len: self.len },
        };
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.rows.outcome,
            match (self.observer_failure, self.rows.outcome) {
                (Some(reason), _) | (_, Outcome::Unavailable { reason }) => {
                    Outcome::Unavailable { reason }
                }
                (_, Outcome::Truncated { .. }) => Outcome::Unavailable {
                    reason: "combat log observer census incomplete",
                },
                _ => Outcome::Rows {
                    len: self.observers.len(),
                },
            },
        ]
    }
    pub(crate) fn lost_reads(&self) -> u64 {
        match self.rows.outcome {
            Outcome::Rows { .. } => 0,
            Outcome::Truncated { unread_rows, .. } => u64::from(unread_rows),
            Outcome::Unavailable { .. } => 1,
        }
    }
    #[cfg(test)]
    pub fn losses_complete(&self) -> bool {
        matches!(self.rows.outcome, Outcome::Rows { .. })
    }
    #[cfg(test)]
    pub fn tables(&self) -> [TableView; N] {
        self.tables_with_baseline(&[])
    }
    pub fn table_inputs(
        &self,
        projection: Option<&mut crate::projection::Projection<'_>>,
    ) -> [replay_frame::frame::TableInput; N] {
        let previous = projection
            .and_then(|p| p.observed_baseline::<tables::CombatLogObservers>())
            .unwrap_or_default();
        self.tables_with_baseline(previous)
            .map(replay_frame::frame::TableInput::Sampled)
    }
    fn tables_with_baseline(
        &self,
        previous: &[replay_data::rows::CombatLogObserverRow],
    ) -> [TableView; N] {
        [
            table_iter::<tables::LandCombatProgress, _>(
                self.outcomes()[0],
                self.rows
                    .rows
                    .iter()
                    .zip(&self.losses)
                    .take(self.rows.outcome.len()),
                |(combat, losses)| LandCombatProgressRow {
                    combat: *combat,
                    attacker_manpower_loss: losses[0].manpower.into(),
                    defender_manpower_loss: losses[1].manpower.into(),
                    attacker_equipment_losses: self.equipment_rows(losses[0]).into(),
                    defender_equipment_losses: self.equipment_rows(losses[1]).into(),
                    ..Default::default()
                },
            ),
            match self.outcomes()[1] {
                Outcome::Rows { .. } => match collector_combat_log::observer_projection::project(
                    &self.observers,
                    previous,
                ) {
                    Ok(rows) => {
                        TableView::Complete(replay_data::rows::TableRows::CombatLogObservers(rows))
                    }
                    Err(reason) => TableView::Unavailable {
                        reason: reason.into(),
                    },
                },
                Outcome::Unavailable { reason } => TableView::Unavailable {
                    reason: reason.into(),
                },
                _ => unreachable!("observer census is complete or unavailable"),
            },
        ]
    }
    fn equipment_rows(
        &self,
        span: engine_land_inputs::cstats_observer::LossSpan,
    ) -> Vec<LandEquipmentLoss> {
        engine_allocation::owned_vec(engine_allocation::collect_vec(
            self.equipment.first[span.start..span.start + span.len]
                .iter()
                .map(|entry| LandEquipmentLoss {
                    variant: entry.variant,
                    amount_raw: entry.amount_raw,
                }),
        ))
    }
}

#[cfg(test)]
#[path = "combat_progress/tests.rs"]
pub(crate) mod tests;
