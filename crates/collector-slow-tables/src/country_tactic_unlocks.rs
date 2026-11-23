use crate::{
    clock::qpc,
    table_capture::{Arena, CaptureRows, grow_capacity},
    table_conversion::{table, unavailable},
};
use engine_allocation::{copy_vec, owned_vec};
use engine_land_inputs::country_tactic_unlocks as native;
use engine_native_memory::{Memory, walk::Outcome};
use replay_data::{
    cadence::{SKIPPED_REASON, should_walk},
    rows::{CountryTacticUnlockRow, TableId, tables},
};
use replay_frame::frame::{TableInput, TableView};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

pub const N: usize = 1 + crate::doctrines::N;
pub const TABLES: [TableId; N] = [
    TableId::CountryTacticUnlocks,
    TableId::DoctrineCountries,
    TableId::DoctrineFolders,
    TableId::DoctrineTracks,
    TableId::DoctrineLeaderMastery,
    TableId::DoctrineCostReductions,
    TableId::DoctrineMasteryGrants,
    TableId::DoctrineEquipmentBonuses,
];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    unlocks: usize,
    registry: usize,
    list: usize,
}

collector_primitives::capacity_max!(Capacities {
    rows,
    unlocks,
    registry,
    list
});

impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let demand = native::count(memory, base, state);
        if let Outcome::Unavailable { reason } = demand.outcome {
            return Err(reason);
        }
        self.rows = self.rows.max(demand.required_rows);
        self.unlocks = self.unlocks.max(demand.required_unlocks);
        self.registry = self.registry.max(demand.required_registry);
        self.list = self.list.max(demand.required_list);
        Ok(())
    }
    pub fn raise_for_countries(&mut self, slots: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows, rows(slots, 1));
    }
}

pub struct Captured {
    rows: CaptureRows<native::Row>,
    unlocks: Arena<u32>,
    registry: Arena<native::RegistryEntry>,
    list: Arena<u8>,
    required_rows: usize,
    doctrines: Mutex<crate::doctrines::Frame>,
    doctrine_bytes: AtomicUsize,
}
impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            unlocks: 0,
            registry: 0,
            list: 0,
        })
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            unlocks: Arena::new(capacity.unlocks),
            registry: Arena::new(capacity.registry),
            list: Arena::new(capacity.list),
            required_rows: 0,
            doctrines: Mutex::new(crate::doctrines::Frame::default()),
            doctrine_bytes: AtomicUsize::new(0),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.unlocks.reset();
        self.registry.reset();
        self.list.reset();
        self.required_rows = 0;
        self.doctrines.get_mut().expect("doctrine frame").reset();
    }
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.doctrines
            .get_mut()
            .expect("doctrine frame")
            .fail(reason);
        self.rows
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
    }
    pub fn doctrine_store_bytes(&mut self) -> Option<usize> {
        self.doctrines
            .get_mut()
            .expect("exclusive doctrine frame")
            .store_bytes
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.unlocks.allocated_bytes()
            + self.registry.allocated_bytes()
            + self.list.allocated_bytes()
            + self.doctrine_bytes.load(Ordering::Relaxed)
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        grow_capacity(&mut capacity.rows, self.required_rows);
        self.rows.grow(&mut capacity.rows);
        self.unlocks.grow(&mut capacity.unlocks);
        self.registry.grow(&mut capacity.registry);
        self.list.grow(&mut capacity.list);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.capture_carried(memory, base, state, hour, force_full, None)
    }
    pub fn capture_carried(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
        store: Option<&Mutex<crate::doctrines::Store>>,
    ) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        self.reset();
        if !should_walk(TABLES[0], hour, force_full) {
            self.rows.outcome = Outcome::Unavailable {
                reason: SKIPPED_REASON,
            };
            return [0; N];
        }
        let begin = qpc();
        let result = native::capture(
            memory,
            base,
            state,
            &mut self.rows.rows,
            &mut self.unlocks.first,
            &mut self.registry.first,
            &mut self.list.first,
        );
        self.rows.outcome = result.outcome;
        self.required_rows = result.required_rows;
        self.unlocks.required = result.required_unlocks;
        self.registry.required = result.required_registry;
        self.list.required = result.required_list;
        self.unlocks.len = self
            .rows
            .rows
            .get(..result.outcome.len())
            .and_then(|rows| {
                rows.iter().try_fold(0, |end, row| {
                    row.unlock_start
                        .checked_add(row.unlock_count)
                        .filter(|&next| next <= self.unlocks.first.len())
                        .map(|next| end.max(next))
                })
            })
            .unwrap_or(0);
        let mut costs = [0; N];
        costs[0] = qpc().saturating_sub(begin).max(0) as u64;
        if let Outcome::Rows { .. } = result.outcome {
            if let Some(store) = store {
                let begin = qpc();
                let acquired = {
                    let _reads = crate::read_accounting::table(TABLES[1]);
                    store
                        .try_lock()
                        .map_err(|_| "doctrine capture store poisoned")
                        .and_then(|mut store| {
                            store.capture(
                                memory,
                                base,
                                (result.status_data, result.status_count),
                                force_full,
                                self.doctrines.get_mut().expect("doctrine frame"),
                            )
                        })
                };
                match acquired {
                    Ok(()) => {}
                    Err(reason) => self
                        .doctrines
                        .get_mut()
                        .expect("doctrine frame")
                        .fail(reason),
                }
                costs[1] = qpc().saturating_sub(begin).max(0) as u64;
            } else {
                self.doctrines
                    .get_mut()
                    .expect("doctrine frame")
                    .fail("doctrine carry store not supplied");
            }
        }
        self.doctrine_bytes.store(
            self.doctrines
                .get_mut()
                .expect("exclusive doctrine frame")
                .allocated_bytes(),
            Ordering::Relaxed,
        );
        costs
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        let doctrines = self.doctrines.lock().expect("doctrine frame");
        std::array::from_fn(|index| {
            if index == 0 {
                self.rows.outcome
            } else if doctrines.complete() {
                Outcome::Rows {
                    len: doctrines.counts[index - 1],
                }
            } else {
                Outcome::Unavailable {
                    reason: doctrines.refusal,
                }
            }
        })
    }
    pub fn tables(&self) -> [TableView; N] {
        self.table_inputs().map(|input| match input {
            TableInput::Sampled(view) => view,
            TableInput::SharedComplete(rows) => TableView::Complete((*rows).clone()),
            _ => unreachable!("doctrine inputs are sampled or shared"),
        })
    }
    pub fn table_inputs(&self) -> [TableInput; N] {
        let mut doctrine_frame = self.doctrines.lock().expect("doctrine frame");
        let mut doctrine_inputs = doctrine_frame.inputs().into_iter();
        self.doctrine_bytes
            .store(doctrine_frame.allocated_bytes(), Ordering::Relaxed);
        drop(doctrine_frame);
        let mut inputs = std::array::from_fn(|index| {
            if index == 0 {
                unavailable("country tactic unlocks not acquired").into()
            } else {
                doctrine_inputs.next().expect("doctrine input")
            }
        });
        let Some(rows) = self.rows.rows.get(..self.rows.outcome.len()) else {
            inputs[0] = unavailable("country tactic unlock rows exceed buffer").into();
            return inputs;
        };
        if self.unlocks.len > self.unlocks.first.len()
            || rows.iter().any(|row| {
                row.unlock_start
                    .checked_add(row.unlock_count)
                    .is_none_or(|end| end > self.unlocks.len)
            })
        {
            inputs[0] = unavailable("country tactic unlocks outside captured arena").into();
            return inputs;
        }
        inputs[0] = table::<tables::CountryTacticUnlocks, _>(self.rows.outcome, rows, |row| {
            CountryTacticUnlockRow {
                country: row.country,
                unlocked_tactic_ids: owned_vec(copy_vec(
                    &self.unlocks.first[row.unlock_start..row.unlock_start + row.unlock_count],
                )),
            }
        })
        .into();
        inputs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableRows;

    #[test]
    fn country_lists_preserve_empty_rows_and_native_unlock_order() {
        let mut captured = Captured::with_capacities(&Capacities {
            rows: 2,
            unlocks: 3,
            registry: 0,
            list: 0,
        });
        captured.rows.rows[0] = native::Row {
            country: 7,
            unlock_start: 0,
            unlock_count: 3,
        };
        captured.rows.rows[1] = native::Row {
            country: 3,
            unlock_start: 3,
            unlock_count: 0,
        };
        captured.unlocks.first.copy_from_slice(&[18, 4, 9]);
        captured.unlocks.len = 3;
        captured.rows.outcome = Outcome::Rows { len: 2 };
        let tables = captured.tables();
        let TableView::Complete(TableRows::CountryTacticUnlocks(rows)) = &tables[0] else {
            panic!("complete country unlocks were not projected");
        };
        assert_eq!(rows[0].country, 3);
        assert!(rows[0].unlocked_tactic_ids.is_empty());
        assert_eq!(rows[1].country, 7);
        assert_eq!(rows[1].unlocked_tactic_ids, [18, 4, 9]);
        captured.rows.rows[0].unlock_count = 4;
        assert!(matches!(
            captured.tables()[0],
            TableView::Unavailable { .. }
        ));
    }

    #[test]
    fn failure_preserves_capacity_demand_without_publishing_partial_lists() {
        let mut capacity = Capacities {
            rows: 1,
            unlocks: 1,
            registry: 1,
            list: 1,
        };
        let mut captured = Captured::with_capacities(&capacity);
        captured.required_rows = 7;
        captured.unlocks.required = 19;
        captured.registry.required = 97;
        captured.list.required = 776;
        captured.rows.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 6,
        };
        captured.fail_capture(0, true, "slow worker stopped");
        assert!(matches!(
            captured.tables()[0],
            TableView::Unavailable { .. }
        ));
        captured.grow(&mut capacity);
        assert!(capacity.rows >= 7);
        assert!(capacity.unlocks >= 19);
        assert!(capacity.registry >= 97);
        assert!(capacity.list >= 776);
    }
}
