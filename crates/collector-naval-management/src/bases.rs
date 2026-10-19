//! Repair bases are admitted by the existing map-base census. Only a native writer mark rereads
//! an admitted base's repair priority, allocation or ordered queue.
use crate::{Snapshot, changes};
use engine_native::{Memory, naval_management as native, read};
use replay_data::rows::*;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex, MutexGuard},
};

pub const TABLES: [TableId; 2] = [TableId::NavalRepairBases, TableId::NavalRepairQueue];
pub type Captured = Snapshot<2>;
#[derive(Default)]
struct Store {
    epoch: Option<u64>,
    entries: HashMap<usize, u32>,
    bases: HashMap<u32, NavalRepairBaseRow>,
    queues: HashMap<u128, NavalRepairQueueRow>,
    queue_lengths: HashMap<u32, usize>,
    scratch: Vec<u64>,
    dirty: Vec<usize>,
    changed: bool,
    failure: Option<&'static str>,
    snapshot: Captured,
}
static STORE: LazyLock<Mutex<Store>> = LazyLock::new(|| Mutex::new(Store::default()));
pub struct Session<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    observed: bool,
    store: MutexGuard<'static, Store>,
}
impl<'a, M: Memory> Session<'a, M> {
    pub fn begin(memory: &'a M, base: usize) -> Self {
        let mut store = STORE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut dirty = std::mem::take(&mut store.dirty);
        dirty.clear();
        let mut shortage = false;
        let epoch = changes::take_bases(|pointer| {
            if engine_allocation::reserve(&mut dirty, 1).is_err() {
                shortage = true;
            } else {
                dirty.push(pointer);
            }
        });
        if epoch != store.epoch {
            let scratch = std::mem::take(&mut store.scratch);
            *store = Store {
                epoch,
                scratch,
                changed: true,
                ..Store::default()
            };
            dirty.clear();
        }
        store.dirty = dirty;
        if epoch.is_none() || shortage {
            // Never reuse a hold after a take that could have lost a selected receiver.
            store.epoch = None;
        }
        let mut session = Self {
            memory,
            base,
            observed: epoch.is_some() && !shortage,
            store,
        };
        if session.observed {
            // Lifetime marks take precedence over every value mark from this interval.
            for index in 0..session.store.dirty.len() {
                let pointer = session.store.dirty[index];
                if pointer & 1 != 0 {
                    session.retire(pointer & !1);
                }
            }
            for index in 0..session.store.dirty.len() {
                let pointer = session.store.dirty[index];
                if pointer & 1 == 0
                    && let Some(province) = session.store.entries.get(&pointer).copied()
                {
                    session.update(pointer, province, false);
                }
            }
        }
        session
    }
    /// Called only after the existing map census authenticated this base and its province key.
    pub fn visit(&mut self, pointer: usize, row: &engine_native::cnaval_base::Row) {
        if !self.observed || self.store.entries.get(&pointer) == Some(&row.province) {
            return;
        }
        self.retire(pointer);
        if engine_allocation::reserve_map(&mut self.store.entries, 1).is_err() {
            self.store.failure = Some("naval repair base receiver allocation failed");
            return;
        }
        self.store.entries.insert(pointer, row.province);
        if !changes::register_base(pointer) {
            self.store.failure = Some("naval repair base registration refused");
            return;
        }
        self.update(pointer, row.province, true);
    }
    fn update(&mut self, pointer: usize, province: u32, authenticated: bool) {
        if let Err(reason) = self.read(pointer, province, authenticated) {
            changes::read_refused(7);
            changes::read_refused(8);
            self.store.failure = Some(reason);
        }
    }
    fn read(
        &mut self,
        pointer: usize,
        province: u32,
        authenticated: bool,
    ) -> Result<(), &'static str> {
        if !authenticated
            && pointer
                .checked_add(engine_native::cnaval_base::PROVINCE_ID)
                .and_then(|at| read::<u32>(self.memory, at))
                != Some(province)
        {
            return Err("naval repair held base province changed");
        }
        let value = if authenticated {
            native::base_settings_authenticated(self.memory, pointer, &mut self.store.scratch)?
        } else {
            native::base_settings(self.memory, self.base, pointer, &mut self.store.scratch)?
        };
        let row = NavalRepairBaseRow {
            province,
            priority: value.priority,
            allocated_dockyards: value.dockyards,
        };
        if self.store.bases.get(&province) != Some(&row) {
            if !self.store.bases.contains_key(&province) {
                engine_allocation::reserve_map(&mut self.store.bases, 1)
                    .map_err(|_| "naval repair base allocation failed")?;
            }
            self.store.bases.insert(province, row);
            self.store.changed = true;
        }
        let len = self.store.scratch.len();
        for ordinal in 0..len {
            let ship = self.store.scratch[ordinal];
            let ordinal =
                u32::try_from(ordinal).map_err(|_| "naval repair queue ordinal overflow")?;
            let row = NavalRepairQueueRow {
                province,
                ordinal,
                ship,
            };
            if self.store.queues.get(&row.key()) != Some(&row) {
                if !self.store.queues.contains_key(&row.key()) {
                    engine_allocation::reserve_map(&mut self.store.queues, 1)
                        .map_err(|_| "naval repair queue allocation failed")?;
                }
                self.store.queues.insert(row.key(), row);
                self.store.changed = true;
            }
        }
        let previous = self
            .store
            .queue_lengths
            .get(&province)
            .copied()
            .unwrap_or(0);
        for ordinal in len..previous {
            let key = (u128::from(province) << 32) | ordinal as u128;
            self.store.changed |= self.store.queues.remove(&key).is_some();
        }
        if !self.store.queue_lengths.contains_key(&province) {
            engine_allocation::reserve_map(&mut self.store.queue_lengths, 1)
                .map_err(|_| "naval repair queue demand allocation failed")?;
        }
        self.store.queue_lengths.insert(province, len);
        Ok(())
    }
    fn retire(&mut self, pointer: usize) {
        if let Some(province) = self.store.entries.remove(&pointer) {
            self.store.changed |= self.store.bases.remove(&province).is_some();
            let length = self.store.queue_lengths.remove(&province).unwrap_or(0);
            for ordinal in 0..length {
                let key = (u128::from(province) << 32) | ordinal as u128;
                self.store.changed |= self.store.queues.remove(&key).is_some();
            }
        }
    }
    pub fn finish(mut self, parent: engine_native::walk::Outcome) -> Captured {
        let failure = if !self.observed {
            Some(changes::UNAVAILABLE)
        } else if !matches!(parent, engine_native::walk::Outcome::Rows { .. }) {
            self.store.epoch = None;
            Some("naval repair source base census incomplete")
        } else {
            self.store.failure
        };
        if let Some(reason) = failure {
            return Captured {
                tables: std::array::from_fn(|_| Err(reason)),
            };
        }
        if self.store.changed {
            let bases = engine_allocation::collect_vec(self.store.bases.values().copied());
            let queues = engine_allocation::collect_vec(self.store.queues.values().copied());
            self.store.snapshot.tables = [
                bases
                    .map(|mut rows| {
                        rows.sort_unstable_by_key(RowCodec::key);
                        Arc::new(TableRows::NavalRepairBases(rows))
                    })
                    .map_err(|_| "naval repair base snapshot allocation failed"),
                queues
                    .map(|mut rows| {
                        rows.sort_unstable_by_key(RowCodec::key);
                        Arc::new(TableRows::NavalRepairQueue(rows))
                    })
                    .map_err(|_| "naval repair queue snapshot allocation failed"),
            ];
        }
        changes::base_demand(self.store.entries.len());
        self.store.changed = false;
        self.store.snapshot.clone()
    }
}
