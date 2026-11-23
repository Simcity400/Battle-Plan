use collector_primitives::table_capture::{Arena, CaptureRows, grow_vec};
use collector_projection::table_conversion::table_iter;
use engine_native::{Memory, cland_combat, walk::Outcome};
use replay_data::{
    rows::LandCombatProgressRow, rows::LandEquipmentLoss, rows::TableId, rows::tables,
};
use replay_frame::frame::TableView;
use std::{mem::size_of, sync::Mutex};

#[derive(Default)]
struct DailyObservers {
    rows: Vec<replay_data::rows::CombatLogObserverRow>,
    sorted: Vec<u32>,
    charge: collector_primitives::raw_memory::Charge,
}

impl DailyObservers {
    fn clear(&mut self) {
        self.rows.clear();
        self.sorted.clear();
    }

    fn find(
        &self,
        row: &replay_data::rows::CombatLogObserverRow,
        next: &std::cell::Cell<usize>,
    ) -> Option<&replay_data::rows::CombatLogObserverRow> {
        let key = row.semantic_key();
        let at = Some(next.get())
            .filter(|&at| {
                self.rows
                    .get(at)
                    .is_some_and(|held| held.semantic_key() == key)
            })
            .or_else(|| {
                let position = self
                    .sorted
                    .binary_search_by_key(&key, |&at| self.rows[at as usize].semantic_key())
                    .ok()?;
                Some(self.sorted[position] as usize)
            })?;
        next.set(at + 1);
        Some(&self.rows[at])
    }
}
static DAILY_OBSERVERS: Mutex<Option<DailyObservers>> = Mutex::new(None);

#[cfg(test)]
pub(crate) fn test_globals() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    *DAILY_OBSERVERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    guard
}

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::LandCombatProgress, TableId::CombatLogObservers];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    variants: usize,
}

collector_primitives::capacity_max!(Capacities { rows, variants });
impl Capacities {
    pub fn presize(&mut self, memory: &impl Memory, base: usize) -> Result<(), &'static str> {
        let state = engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
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
        collector_primitives::table_capture::grow_capacity(&mut self.rows, rows);
        collector_primitives::table_capture::grow_capacity(&mut self.variants, variants);
        Ok(())
    }
}

pub struct Captured {
    rows: CaptureRows<u64>,
    losses: Vec<[engine_land_inputs::cstats_observer::LossSpan; 2]>,
    equipment: Arena<engine_land_inputs::cstats_observer::LossEntry>,
    pointers: Vec<usize>,
    identities: Vec<[u8; 16]>,
    len: usize,
    failure: Option<&'static str>,
    observers: Vec<replay_data::rows::CombatLogObserverRow>,
    observer_failure: Option<&'static str>,
    observer_scratch: std::sync::Mutex<collector_combat_log::observer_projection::Scratch>,
    daily_refresh: bool,
    daily_previous: Option<DailyObservers>,
    observer_cursor: usize,
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
            observer_scratch: Default::default(),
            daily_refresh: true,
            daily_previous: None,
            observer_cursor: 0,
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
            + self
                .observer_scratch
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        grow_vec(&mut self.losses, capacity.rows);
        capacity.variants = capacity.variants.max(self.equipment.first.len());
        self.equipment.grow(&mut capacity.variants);
        grow_vec(&mut self.pointers, capacity.variants);
        grow_vec(&mut self.identities, capacity.variants);
    }
    /// # Safety
    /// Only the admitted hour's combat capture setup (or an isolated fixture holding
    /// `test_globals()`) may clear the process-wide daily observer baseline for a refresh hour.
    pub unsafe fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        self.daily_refresh = force_full
            || replay_data::cadence::Cadence::Daily(replay_data::cadence::COMBAT_LOG_NUMERIC_SLOT)
                .is_refresh(hour);
        if self.daily_refresh
            && let Some(previous) = DAILY_OBSERVERS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
        {
            previous.clear();
        }
    }

    /// # Safety
    /// Only the admitted hour's sole combat capture (or an isolated fixture holding
    /// `test_globals()`) may take the process-wide daily observer baseline loan; pair with
    /// [`Self::finish`] to return it.
    pub unsafe fn begin(&mut self) {
        let previous = DAILY_OBSERVERS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        self.daily_previous = previous;
        self.observer_cursor = 0;
        if self.daily_refresh
            && let Some(previous) = &mut self.daily_previous
        {
            previous.clear();
        }
        self.equipment.reset();
        self.len = 0;
        self.failure = None;
        self.observers.clear();
        self.observer_failure = None;
    }
    pub fn visit(&mut self, memory: &impl Memory, base: usize, source: cland_combat::Source) {
        let _reads = collector_memory::read_accounting::table(TABLES[0]);
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
            let cursor = std::cell::Cell::new(self.observer_cursor);
            for (side, pointer) in source.sides.into_iter().enumerate() {
                let mut storage = Ok(());
                use replay_data::rows::{CombatLogObserverKind as K, CombatLogObserverRow};
                let previous = self.daily_previous.as_ref();
                let mut parent_held = false;
                let lookup = |row: &CombatLogObserverRow| {
                    previous
                        .and_then(|held| held.find(row, &cursor))
                        .filter(|held| {
                            (held.identity, held.identity_2, held.identity_3)
                                == (row.identity, row.identity_2, row.identity_3)
                        })
                };
                let mut put = |fact: engine_land_inputs::combat_log_events::Fact| {
                    if storage.is_err() {
                        return;
                    }
                    storage = engine_allocation::reserve_push_batch(&mut self.observers, 1)
                        .map_err(|_| "combat log observer arena allocation failed");
                    if storage.is_ok() {
                        match CombatLogObserverRow::from_fact(
                            source.id,
                            side == 0,
                            fact.kind as u8,
                            fact.item,
                            fact.values,
                        ) {
                            Ok(mut row) => {
                                if row.kind == K::Activity as u8 {
                                    parent_held = lookup(&row).is_some();
                                }
                                let child = row.kind == K::ActivityEquipment as u8
                                    || row.kind == K::ActivityDamage as u8;
                                if !self.daily_refresh
                                    && (!child || parent_held)
                                    && let Some(held) = lookup(&row)
                                {
                                    row.a = held.a;
                                    row.b = held.b;
                                }
                                self.observers.push(row)
                            }
                            Err(reason) => storage = Err(reason),
                        }
                    }
                };
                let read = engine_land_inputs::combat_log_events::observer(
                    memory, base, pointer, &mut put,
                );
                if let Err(reason) = read.and(storage) {
                    self.observer_failure = Some(reason);
                    break;
                }
            }
            self.observer_cursor = cursor.get();
        }
    }
    /// # Safety
    /// Only the admitted hour's sole combat capture (or an isolated fixture holding
    /// `test_globals()`) may publish the updated daily observer baseline and report observer sizing
    /// demand while observer storage remains live.
    pub unsafe fn finish(&mut self, parent: Outcome) {
        crate::land_events::observe_observer_rows(self.observers.len());
        // SAFETY: Caller guarantees admitted combat capture while history-event storage is live.
        unsafe {
            collector_combat_log::history_events::prepare_active(
                self.observers.len().saturating_add(self.equipment.len),
            );
        }
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
        self.remember_daily();
    }
    fn remember_daily(&mut self) {
        let mut next = self.daily_previous.take().unwrap_or_default();
        next.clear();
        let complete =
            self.observer_failure.is_none() && matches!(self.rows.outcome, Outcome::Rows { .. });
        let stored = complete
            && u32::try_from(self.observers.len()).is_ok()
            && engine_allocation::advisory(|| {
                engine_allocation::reserve(&mut next.rows, self.observers.len())?;
                engine_allocation::reserve(&mut next.sorted, self.observers.len())
            })
            .is_ok();
        let next = if stored {
            next.rows.extend(self.observers.iter().cloned());
            let DailyObservers { rows, sorted, .. } = &mut next;
            sorted.extend(0..rows.len() as u32);
            sorted.sort_unstable_by_key(|&at| rows[at as usize].semantic_key());
            let unique = !sorted.windows(2).any(|pair| {
                rows[pair[0] as usize].semantic_key() == rows[pair[1] as usize].semantic_key()
            });
            next.charge.set(collector_primitives::raw_memory::Capacity {
                auxiliary_vectors: collector_primitives::raw_memory::vector_bytes(&next.rows)
                    + collector_primitives::raw_memory::vector_bytes(&next.sorted),
                ..Default::default()
            });
            unique.then_some(next)
        } else {
            None
        };
        *DAILY_OBSERVERS.lock().unwrap_or_else(|e| e.into_inner()) = next;
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
    pub fn lost_reads(&self) -> u64 {
        match self.rows.outcome {
            Outcome::Rows { .. } => 0,
            Outcome::Truncated { unread_rows, .. } => u64::from(unread_rows),
            Outcome::Unavailable { .. } => 1,
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn losses_complete(&self) -> bool {
        matches!(self.rows.outcome, Outcome::Rows { .. })
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn tables(&self) -> [TableView; N] {
        self.tables_with_baseline(&[])
    }
    pub fn table_inputs(
        &self,
        projection: Option<&mut collector_projection::projection::Projection<'_>>,
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
                Outcome::Rows { .. } => {
                    match collector_combat_log::observer_projection::project_with(
                        &self.observers,
                        previous,
                        &mut self
                            .observer_scratch
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner),
                    ) {
                        Ok(rows) => TableView::Complete(
                            replay_data::rows::TableRows::CombatLogObservers(rows),
                        ),
                        Err(reason) => TableView::Unavailable {
                            reason: reason.into(),
                        },
                    }
                }
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
pub mod tests;
