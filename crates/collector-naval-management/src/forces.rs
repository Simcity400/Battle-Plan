use crate::{Snapshot, changes};
use engine_native::{Memory, ccountry, naval_management as native, read};
use engine_sites::Rva;
use replay_data::rows::*;
use std::{
    collections::HashMap,
    hash::Hash,
    sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard},
};

pub const TABLES: [TableId; 7] = [
    TableId::NavalRepairCountries,
    TableId::NavalRepairFleets,
    TableId::NavalRepairTaskForces,
    TableId::NavalTaskForceComposition,
    TableId::NavalRefitShips,
    TableId::NavalRefitProgress,
    TableId::NavalRepairChildren,
];
pub type Captured = Snapshot<7>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    Country {
        country: ccountry::Country,
        production: usize,
        navy: usize,
    },
    Fleet(u64),
    Task(u64),
    Composition {
        task: usize,
        handle: u64,
        kind: u8,
    },
    Ship(u64),
    Line(u64),
    Children {
        task: usize,
        handle: u64,
    },
}
#[derive(Default)]
struct Store {
    loaned: bool,
    epoch: Option<u64>,
    entries: HashMap<usize, Entry>,
    countries: HashMap<u32, NavalRepairCountryRow>,
    fleets: HashMap<u64, NavalRepairFleetRow>,
    tasks: HashMap<u64, NavalRepairTaskForceRow>,
    composition: HashMap<u128, NavalTaskForceCompositionRow>,
    composition_lengths: HashMap<(u64, u8), usize>,
    ships: HashMap<u64, NavalRefitShipRow>,
    lines: HashMap<u64, NavalRefitProgressRow>,
    line_receivers: HashMap<u64, (usize, usize)>,
    composition_scratch: Vec<native::CompositionEntry>,
    children: HashMap<u128, NavalRepairChildRow>,
    child_lengths: HashMap<u64, usize>,
    child_scratch: Vec<u64>,
    dirty: Vec<usize>,
    changed: [bool; 7],
    failures: [Option<&'static str>; 7],
    snapshots: Captured,
}
static STORE: LazyLock<Mutex<Store>> = LazyLock::new(|| Mutex::new(Store::default()));
static RETURNED: Condvar = Condvar::new();

pub struct Owned {
    store: Option<Store>,
    complete: bool,
}
impl Owned {
    pub fn decode(mut self) -> Captured {
        let rows = self.store.as_mut().unwrap().publish();
        self.complete = true;
        rows
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        let mut held = self.store.take().unwrap();
        if !self.complete {
            held.epoch = None;
        }
        let mut store = STORE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(store.loaned, "one naval management publication loan");
        *store = held;
        RETURNED.notify_one();
    }
}

fn insert<K: Copy + Eq + Hash, V: PartialEq>(
    map: &mut HashMap<K, V>,
    key: K,
    value: V,
) -> Result<bool, &'static str> {
    if let Some(current) = map.get_mut(&key) {
        if *current == value {
            return Ok(false);
        }
        *current = value;
        return Ok(true);
    }
    engine_allocation::reserve_map(map, 1).map_err(|_| "naval management map allocation failed")?;
    map.insert(key, value);
    Ok(true)
}
fn field<T: engine_native::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|at| read(memory, at))
        .ok_or("naval management identity unreadable or overflow")
}
fn identity(
    memory: &impl Memory,
    base: usize,
    object: usize,
    vtable: Rva,
    handle_offset: usize,
    handle: u64,
    authenticated: bool,
) -> Result<(), &'static str> {
    if authenticated {
        return Ok(());
    }
    if field::<u64>(memory, object, 0)?
        != engine_sites::at(base, vtable).ok_or("naval management vtable address overflow")? as u64
        || field::<u64>(memory, object, handle_offset)? != handle
    {
        return Err("naval management held object identity changed");
    }
    Ok(())
}

pub struct Session<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    state: usize,
    store: MutexGuard<'static, Store>,
    observed: bool,
    navies: Option<Result<engine_native::cpdx_array::Array, &'static str>>,
}
impl<'a, M: Memory> Session<'a, M> {
    #[cfg(test)]
    pub(crate) fn begin(memory: &'a M, base: usize, state: usize) -> Self {
        Self::begin_inner(memory, base, state, None)
    }

    /// # Safety
    /// Only the admitted naval capture (or an isolated fixture owning the global naval-force store)
    /// may open a session against the authenticated image, and must settle or drop any returned
    /// `Owned` loan before reusing its frame or detaching.
    pub unsafe fn begin_owned(
        memory: &'a M,
        base: usize,
        state: usize,
        stopped: fn() -> bool,
    ) -> Self {
        Self::begin_inner(memory, base, state, Some(stopped))
    }

    fn begin_inner(
        memory: &'a M,
        base: usize,
        state: usize,
        stopped: Option<fn() -> bool>,
    ) -> Self {
        let mut store = STORE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while store.loaned {
            if stopped.is_some_and(|stopped| stopped()) {
                return Self {
                    memory,
                    base,
                    state,
                    store,
                    observed: false,
                    navies: None,
                };
            }
            store = RETURNED
                .wait_timeout(store, std::time::Duration::from_millis(10))
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        store.dirty.clear();
        let mut dirty = std::mem::take(&mut store.dirty);
        let mut shortage = false;
        let epoch = changes::take_forces(|pointer| {
            if engine_allocation::reserve(&mut dirty, 1).is_err() {
                shortage = true;
            } else {
                dirty.push(pointer);
            }
        });
        let observed = epoch.is_some() && !shortage;
        if epoch != store.epoch {
            let scratch = std::mem::take(&mut store.composition_scratch);
            *store = Store {
                epoch,
                changed: [true; 7],
                composition_scratch: scratch,
                ..Store::default()
            };
            dirty.clear();
        }
        store.dirty = dirty;
        if !observed {
            store.epoch = None;
        }
        let mut session = Self {
            memory,
            base,
            state,
            store,
            observed,
            navies: None,
        };
        if observed {
            for index in 0..session.store.dirty.len() {
                let pointer = session.store.dirty[index];
                if pointer & 1 != 0 {
                    session.retire(pointer & !1);
                }
            }
            for index in 0..session.store.dirty.len() {
                let pointer = session.store.dirty[index];
                if pointer & 1 == 0
                    && let Some(entry) = session.store.entries.get(&pointer).copied()
                {
                    session.update(pointer, entry, false);
                }
            }
        }
        session
    }

    fn retain(&mut self, pointer: usize, entry: Entry) -> bool {
        if !self.observed {
            return false;
        }
        if self.store.entries.get(&pointer) == Some(&entry) {
            return false;
        }
        if self.store.entries.contains_key(&pointer) {
            self.retire(pointer);
        }
        if insert(&mut self.store.entries, pointer, entry).is_err() {
            self.store
                .failures
                .fill(Some("naval management receiver allocation failed"));
            return false;
        }
        if !changes::register_force(pointer) {
            self.store
                .failures
                .fill(Some("naval management receiver registration refused"));
            return false;
        }
        true
    }

    pub fn visit(&mut self, visit: native::CensusVisit) {
        if !self.observed {
            return;
        }
        match visit {
            native::CensusVisit::Country(country) => {
                if matches!(self.store.entries.get(&country.pointer), Some(Entry::Country { country: old, .. }) if *old == country)
                {
                    return;
                }
                let selected = (|| {
                    let production = usize::try_from(field::<u64>(
                        self.memory,
                        country.pointer,
                        ccountry::PRODUCTION_STATUS,
                    )?)
                    .map_err(|_| "naval repair production pointer overflow")?;
                    let navies = *self.navies.get_or_insert_with(|| {
                        engine_native::cstrategic_navy_manager::navies(
                            self.memory,
                            self.base,
                            self.state,
                        )
                    });
                    let at = navies?
                        .element(country.canonical_slot, 8)
                        .ok_or("naval repair country outside navy array")?;
                    let navy = usize::try_from(field::<u64>(self.memory, at, 0)?)
                        .map_err(|_| "naval repair navy pointer overflow")?;
                    Ok::<_, &'static str>(Entry::Country {
                        country,
                        production,
                        navy,
                    })
                })();
                match selected {
                    Ok(
                        entry @ Entry::Country {
                            production, navy, ..
                        },
                    ) => {
                        for pointer in [country.pointer, production, navy] {
                            self.retain(pointer, entry);
                        }
                        self.update(country.pointer, entry, true);
                    }
                    Ok(_) => unreachable!(),
                    Err(reason) => {
                        changes::read_refused(0);
                        self.store.failures[0] = Some(reason);
                        self.retain(
                            country.pointer,
                            Entry::Country {
                                country,
                                production: 0,
                                navy: 0,
                            },
                        );
                    }
                }
            }
            native::CensusVisit::Fleet { pointer, handle } => {
                let entry = Entry::Fleet(handle);
                if self.retain(pointer, entry) {
                    self.update(pointer, entry, true);
                }
            }
            native::CensusVisit::TaskForce { pointer, handle } => {
                let entry = Entry::Task(handle);
                if self.retain(pointer, entry) {
                    self.update(pointer, entry, true);
                    if let Some(array) = pointer.checked_add(native::REPAIR_CHILDREN) {
                        let entry = Entry::Children {
                            task: pointer,
                            handle,
                        };
                        if self.retain(array, entry) {
                            self.update(array, entry, true);
                        }
                    } else {
                        self.store.failures[6] = Some("naval repair children address overflow");
                    }
                    for (kind, offset) in [(0, native::REQUIREMENTS), (1, native::FULFILLMENT)] {
                        let Some(object) = pointer.checked_add(offset) else {
                            self.store.failures[3] =
                                Some("naval composition object address overflow");
                            continue;
                        };
                        let entry = Entry::Composition {
                            task: pointer,
                            handle,
                            kind,
                        };
                        if self.retain(object, entry) {
                            self.update(object, entry, true);
                        }
                    }
                }
            }
        }
    }

    pub fn ship(&mut self, pointer: usize, row: &engine_native::cship::Row) {
        let entry = Entry::Ship(row.id);
        if self.retain(pointer, entry) {
            self.update(pointer, entry, true);
        }
    }

    fn update(&mut self, pointer: usize, entry: Entry, authenticated: bool) {
        let index = match entry {
            Entry::Country { .. } => 0,
            Entry::Fleet(_) => 1,
            Entry::Task(_) => 2,
            Entry::Composition { .. } => 3,
            Entry::Ship(_) => 4,
            Entry::Line(_) => 5,
            Entry::Children { .. } => 6,
        };
        if let Err(reason) = self.read(pointer, entry, authenticated) {
            changes::read_refused(index);
            self.store.failures[index] = Some(reason);
        }
    }

    fn read(
        &mut self,
        pointer: usize,
        entry: Entry,
        authenticated: bool,
    ) -> Result<(), &'static str> {
        match entry {
            Entry::Country {
                country,
                production,
                navy,
            } => {
                if Some(field::<u64>(self.memory, production, 0)?)
                    != engine_sites::at(self.base, engine_native::cproduction_status::VTABLE_RVA)
                        .map(|vtable| vtable as u64)
                    || field::<u64>(
                        self.memory,
                        production,
                        engine_native::cproduction_status::COUNTRY,
                    )? != country.pointer as u64
                {
                    return Err("naval repair production owner identity mismatch");
                }
                engine_native::cstrategic_navy::authenticate(self.memory, self.base, navy)?;
                let (maximum, used) = native::repair_dockyards(self.memory, navy)?;
                let row = NavalRepairCountryRow {
                    country: country.id,
                    max_factories_for_repair: native::max_factories_for_repair(
                        self.memory,
                        production,
                    )?,
                    max_allowed_dockyards_to_repair: maximum,
                    num_used_dockyards: used,
                };
                let changed = insert(&mut self.store.countries, row.country, row)?;
                self.store.changed[0] |= changed;
            }
            Entry::Fleet(handle) => {
                identity(
                    self.memory,
                    self.base,
                    pointer,
                    engine_native::cfleet::VTABLE_RVA,
                    engine_native::cfleet::HANDLE,
                    handle,
                    authenticated,
                )?;
                let row = NavalRepairFleetRow {
                    fleet: handle,
                    automated_homebase: native::automated_homebase(self.memory, pointer)?,
                };
                let changed = insert(&mut self.store.fleets, handle, row)?;
                self.store.changed[1] |= changed;
            }
            Entry::Task(handle) => {
                identity(
                    self.memory,
                    self.base,
                    pointer,
                    engine_native::ctask_force::VTABLE_RVA,
                    engine_native::cunit::HANDLE,
                    handle,
                    authenticated,
                )?;
                let value = native::task_settings(self.memory, self.base, self.state, pointer)?;
                let row = NavalRepairTaskForceRow {
                    task_force: handle,
                    repair_mode: value.repair_mode,
                    repair_split: value.repair_split,
                    repair_target: value.repair_target,
                    auto_reinforcement: value.auto_reinforcement,
                    refit_variant: value.refit_variant,
                    refit_after_repair: value.refit_after_repair,
                    composition_token: value.composition_token,
                    merge_after_repair: value.merge_after_repair,
                };
                let changed = insert(&mut self.store.tasks, handle, row)?;
                self.store.changed[2] |= changed;
            }
            Entry::Composition { task, handle, kind } => {
                identity(
                    self.memory,
                    self.base,
                    task,
                    engine_native::ctask_force::VTABLE_RVA,
                    engine_native::cunit::HANDLE,
                    handle,
                    authenticated,
                )?;
                native::composition(
                    self.memory,
                    self.base,
                    pointer,
                    &mut self.store.composition_scratch,
                )?;
                let mut changed = false;
                for ordinal in 0..self.store.composition_scratch.len() {
                    let value = self.store.composition_scratch[ordinal];
                    let ordinal =
                        u32::try_from(ordinal).map_err(|_| "naval composition ordinal overflow")?;
                    let row = NavalTaskForceCompositionRow {
                        task_force: handle,
                        kind,
                        ordinal,
                        sub_unit: value.sub_unit,
                        role: value.role,
                        definition: value.definition,
                        amount: value.amount,
                    };
                    changed |= insert(&mut self.store.composition, row.key(), row)?;
                }
                let len = self.store.composition_scratch.len();
                let previous = self
                    .store
                    .composition_lengths
                    .get(&(handle, kind))
                    .copied()
                    .unwrap_or(0);
                for ordinal in len..previous {
                    let key =
                        (u128::from(handle) << 40) | (u128::from(kind) << 32) | ordinal as u128;
                    changed |= self.store.composition.remove(&key).is_some();
                }
                insert(&mut self.store.composition_lengths, (handle, kind), len)?;
                self.store.changed[3] |= changed;
            }
            Entry::Ship(handle) => {
                identity(
                    self.memory,
                    self.base,
                    pointer,
                    engine_native::cship::VTABLE_RVA,
                    engine_native::cship::HANDLE,
                    handle,
                    authenticated,
                )?;
                let line = native::ship_refit_line(self.memory, self.base, pointer)?;
                if line == 0 {
                    if let Some(old) = self.store.ships.remove(&handle) {
                        self.store.changed[4] = true;
                        self.release_line(old.line);
                    }
                } else {
                    let old = self.store.ships.get(&handle).copied();
                    let changed = insert(
                        &mut self.store.ships,
                        handle,
                        NavalRefitShipRow { ship: handle, line },
                    )?;
                    self.store.changed[4] |= changed;
                    if let Some(old) = old
                        && old.line != line
                    {
                        self.release_line(old.line);
                    }
                    if old.is_none_or(|old| old.line != line) {
                        if let Some((_, users)) = self.store.line_receivers.get_mut(&line) {
                            *users = users.checked_add(1).ok_or("naval refit demand overflow")?;
                        } else {
                            insert(&mut self.store.line_receivers, line, (0, 1))?;
                        }
                    }
                    if let Err(reason) = self.observe_line(line) {
                        changes::read_refused(5);
                        self.store.failures[5] = Some(reason);
                    }
                }
            }
            Entry::Children { task, handle } => {
                identity(
                    self.memory,
                    self.base,
                    task,
                    engine_native::ctask_force::VTABLE_RVA,
                    engine_native::cunit::HANDLE,
                    handle,
                    authenticated,
                )?;
                native::repair_children(self.memory, pointer, &mut self.store.child_scratch)?;
                let length = self.store.child_scratch.len();
                let mut changed = false;
                for index in 0..length {
                    let ordinal =
                        u32::try_from(index).map_err(|_| "naval repair child ordinal overflow")?;
                    let row = NavalRepairChildRow {
                        task_force: handle,
                        ordinal,
                        child: self.store.child_scratch[index],
                    };
                    changed |= insert(&mut self.store.children, row.key(), row)?;
                }
                let previous = self.store.child_lengths.get(&handle).copied().unwrap_or(0);
                for ordinal in length..previous {
                    changed |= self
                        .store
                        .children
                        .remove(&((u128::from(handle) << 32) | ordinal as u128))
                        .is_some();
                }
                insert(&mut self.store.child_lengths, handle, length)?;
                self.store.changed[6] |= changed;
            }
            Entry::Line(handle) => {
                identity(
                    self.memory,
                    self.base,
                    pointer,
                    engine_native::cship_refit_production_line::VTABLE_RVA,
                    engine_native::cproduction_line::REFERENCE_TYPE,
                    handle,
                    false,
                )?;
                let row = NavalRefitProgressRow {
                    line: handle,
                    progress_raw: field(
                        self.memory,
                        pointer,
                        engine_native::cproduction_line::PRODUCED,
                    )?,
                };
                let changed = insert(&mut self.store.lines, handle, row)?;
                self.store.changed[5] |= changed;
            }
        }
        Ok(())
    }

    fn observe_line(&mut self, handle: u64) -> Result<(), &'static str> {
        let Some(object) =
            engine_native::creference_registry::resolve(self.memory, self.base, handle)?
        else {
            return Ok(());
        };
        self.store
            .line_receivers
            .get_mut(&handle)
            .expect("raw refit demand registered")
            .0 = object;
        let entry = Entry::Line(handle);
        if self.retain(object, entry) {
            self.update(object, entry, false);
        }
        Ok(())
    }

    fn release_line(&mut self, handle: u64) {
        let Some((pointer, users)) = self.store.line_receivers.get_mut(&handle) else {
            return;
        };
        *users -= 1;
        if *users == 0 {
            let pointer = *pointer;
            self.store.line_receivers.remove(&handle);
            self.store.changed[5] |= self.store.lines.remove(&handle).is_some();
            if self.store.entries.get(&pointer) == Some(&Entry::Line(handle)) {
                self.store.entries.remove(&pointer);
            }
        }
    }
    fn retire(&mut self, pointer: usize) {
        let Some(entry) = self.store.entries.remove(&pointer) else {
            return;
        };
        match entry {
            Entry::Country {
                country,
                production,
                navy,
            } => {
                self.store.changed[0] |= self.store.countries.remove(&country.id).is_some();
                for pointer in [country.pointer, production, navy] {
                    self.store.entries.remove(&pointer);
                }
            }
            Entry::Fleet(handle) => {
                self.store.changed[1] |= self.store.fleets.remove(&handle).is_some()
            }
            Entry::Task(handle) => {
                self.store.changed[2] |= self.store.tasks.remove(&handle).is_some();
                for offset in [
                    native::REQUIREMENTS,
                    native::FULFILLMENT,
                    native::REPAIR_CHILDREN,
                ] {
                    if let Some(at) = pointer.checked_add(offset) {
                        self.retire(at);
                    }
                }
            }
            Entry::Composition { handle, kind, .. } => {
                let length = self
                    .store
                    .composition_lengths
                    .remove(&(handle, kind))
                    .unwrap_or(0);
                for ordinal in 0..length {
                    let key =
                        (u128::from(handle) << 40) | (u128::from(kind) << 32) | ordinal as u128;
                    self.store.changed[3] |= self.store.composition.remove(&key).is_some();
                }
            }
            Entry::Ship(handle) => {
                if let Some(old) = self.store.ships.remove(&handle) {
                    self.store.changed[4] = true;
                    self.release_line(old.line);
                }
            }
            Entry::Line(handle) => {
                self.store.changed[5] |= self.store.lines.remove(&handle).is_some();
                if let Some((receiver, _)) = self.store.line_receivers.get_mut(&handle)
                    && *receiver == pointer
                {
                    *receiver = 0;
                }
            }
            Entry::Children { handle, .. } => {
                let length = self.store.child_lengths.remove(&handle).unwrap_or(0);
                for ordinal in 0..length {
                    self.store.changed[6] |= self
                        .store
                        .children
                        .remove(&((u128::from(handle) << 32) | ordinal as u128))
                        .is_some();
                }
            }
        }
    }

    pub fn finish(mut self, parent: engine_native::cfleet::Capture) -> Captured {
        if let Some(reason) = self.failure(parent) {
            return Captured {
                tables: std::array::from_fn(|_| Err(reason)),
            };
        }
        changes::force_demand(self.store.entries.len());
        self.store.publish()
    }

    pub fn finish_owned(
        mut self,
        parent: engine_native::cfleet::Capture,
    ) -> (Captured, Option<Owned>) {
        if let Some(reason) = self.failure(parent) {
            return (
                Captured {
                    tables: std::array::from_fn(|_| Err(reason)),
                },
                None,
            );
        }
        changes::force_demand(self.store.entries.len());
        if !self.store.changed.iter().any(|changed| *changed)
            && self.store.failures.iter().all(Option::is_none)
        {
            return (self.store.snapshots.clone(), None);
        }
        let held = std::mem::take(&mut *self.store);
        self.store.loaned = true;
        (
            Captured::default(),
            Some(Owned {
                store: Some(held),
                complete: false,
            }),
        )
    }

    fn failure(&mut self, parent: engine_native::cfleet::Capture) -> Option<&'static str> {
        if !self.observed {
            return Some(changes::UNAVAILABLE);
        }
        if !matches!(parent.fleets, engine_native::walk::Outcome::Rows { .. })
            || !matches!(
                parent.task_forces,
                engine_native::walk::Outcome::Rows { .. }
            )
            || !matches!(parent.ships, engine_native::walk::Outcome::Rows { .. })
        {
            self.store.epoch = None;
            return Some("naval management source census incomplete");
        }
        None
    }
}

impl Store {
    fn publish(&mut self) -> Captured {
        macro_rules! publish {
            ($index:expr, $map:ident, $table:ident) => {
                if let Some(reason) = self.failures[$index] {
                    self.snapshots.tables[$index] = Err(reason);
                } else if self.changed[$index] {
                    let rows = engine_allocation::collect_vec(self.$map.values().copied()).map(
                        |mut rows| {
                            rows.sort_unstable_by_key(RowCodec::key);
                            rows
                        },
                    );
                    self.snapshots.tables[$index] = rows
                        .map(|rows| Arc::new(TableRows::$table(rows)))
                        .map_err(|_| "naval management snapshot allocation failed");
                }
            };
        }
        publish!(0, countries, NavalRepairCountries);
        publish!(1, fleets, NavalRepairFleets);
        publish!(2, tasks, NavalRepairTaskForces);
        publish!(3, composition, NavalTaskForceComposition);
        publish!(4, ships, NavalRefitShips);
        publish!(5, lines, NavalRefitProgress);
        publish!(6, children, NavalRepairChildren);
        self.changed.fill(false);
        self.snapshots.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, collections::BTreeMap};
    #[derive(Default)]
    struct MemoryFixture {
        bytes: BTreeMap<usize, u8>,
        reads: Cell<usize>,
    }
    impl Memory for MemoryFixture {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            for (index, byte) in out.iter_mut().enumerate() {
                let Some(value) = self.bytes.get(&(at + index)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }
    impl MemoryFixture {
        fn put(&mut self, at: usize, bytes: &[u8]) {
            for (offset, value) in bytes.iter().enumerate() {
                self.bytes.insert(at + offset, *value);
            }
        }
        fn fleet(&mut self, pointer: usize, handle: u64, automation: u8) {
            self.put(
                pointer,
                &(engine_native::cfleet::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            self.put(
                pointer + engine_native::cfleet::HANDLE,
                &handle.to_le_bytes(),
            );
            self.put(pointer + native::AUTOMATED_HOMEBASE, &[automation]);
        }
        fn task(&mut self, pointer: usize, handle: u64, rejoin: u64) {
            self.put(
                pointer,
                &(engine_native::ctask_force::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            self.put(
                pointer + engine_native::cunit::HANDLE,
                &handle.to_le_bytes(),
            );
            self.put(pointer + native::REPAIR_MODE, &1_i32.to_le_bytes());
            self.put(pointer + native::REPAIR_SPLIT, &[1]);
            self.put(pointer + native::AUTO_REINFORCEMENT, &[0]);
            for offset in [
                engine_native::ctask_force::REPAIR_TARGET,
                native::REFIT_VARIANT,
                native::REFIT_AFTER_REPAIR,
            ] {
                self.put(pointer + offset, &0_u64.to_le_bytes());
            }
            self.put(pointer + native::MERGE_AFTER_REPAIR, &rejoin.to_le_bytes());
            self.put(
                pointer + native::COMPOSITION_TOKEN,
                &native::NO_AI_COMPOSITION_TOKEN.to_le_bytes(),
            );
            self.put(pointer + native::REPAIR_CHILDREN, &[0; 16]);
            for offset in [native::REQUIREMENTS, native::FULFILLMENT] {
                self.put(
                    pointer + offset,
                    &(native::ENTRIES_VTABLE_RVA.0 as u64).to_le_bytes(),
                );
                self.put(pointer + offset + native::COMPOSITION_ENTRIES, &[0; 16]);
            }
        }
        fn ship(&mut self, pointer: usize, handle: u64, line: u64) {
            self.put(
                pointer,
                &(engine_native::cship::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            self.put(
                pointer + engine_native::cship::HANDLE,
                &handle.to_le_bytes(),
            );
            self.put(pointer + native::SHIP_REFIT_LINE, &line.to_le_bytes());
        }
        fn refit(&mut self, pointer: usize, handle: u64) {
            use engine_native::{creference_registry as registry, reference_bucket as bucket};
            const TABLE: usize = 0x40000;
            const BUCKETS: usize = 0x41000;
            self.put(
                registry::LOW_ROOTS_RVA.0 + handle as u32 as usize * 8,
                &(TABLE as u64).to_le_bytes(),
            );
            self.put(TABLE + registry::ENTRIES, &(BUCKETS as u64).to_le_bytes());
            self.put(TABLE + registry::COUNT, &1_u32.to_le_bytes());
            self.put(TABLE + registry::MASK, &7_u32.to_le_bytes());
            self.put(TABLE + registry::OVERFLOW_DISTANCE, &[5]);
            self.put(BUCKETS, &[0; 14 * bucket::STRIDE]);
            self.put(BUCKETS + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
            let hash = handle
                .rotate_left(32)
                .wrapping_mul(registry::HASH_MULTIPLIER);
            let index = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
            let cell = BUCKETS + index * bucket::STRIDE;
            self.put(cell + bucket::DISTANCE, &[1]);
            self.put(cell + bucket::TYPE, &handle.to_le_bytes());
            self.put(cell + bucket::OBJECT, &(pointer as u64).to_le_bytes());
            self.put(
                pointer,
                &(engine_native::cship_refit_production_line::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            self.put(
                pointer + engine_native::cproduction_line::REFERENCE_TYPE,
                &handle.to_le_bytes(),
            );
            self.put(
                pointer + engine_native::cproduction_line::PRODUCED,
                &123_i64.to_le_bytes(),
            );
        }
    }
    fn complete() -> engine_native::cfleet::Capture {
        let rows = engine_native::walk::Outcome::Rows { len: 0 };
        engine_native::cfleet::Capture {
            fleets: rows,
            task_forces: rows,
            ships: rows,
        }
    }
    #[test]
    fn naval_management_transferred_repair_detachment_carries_raw_rejoin_after_destination_retires()
    {
        let _guard = changes::fixture::SERIAL.lock().unwrap();
        changes::fixture::prepare();
        const DESTINATION: usize = 0x10000;
        const DETACHMENT: usize = 0x20000;
        const CHILDREN: usize = 0x30000;
        let destination =
            u64::from(engine_native::ctask_force::DEFAULT_HANDLE_TYPE) | (7_u64 << 32);
        let detachment = u64::from(engine_native::ctask_force::DEFAULT_HANDLE_TYPE) | (8_u64 << 32);
        let mut memory = MemoryFixture::default();
        memory.task(DESTINATION, destination, 0);
        memory.task(DETACHMENT, detachment, destination);
        memory.put(
            DETACHMENT + engine_native::ctask_force::FLEET,
            &1_u64.to_le_bytes(),
        );
        memory.put(
            DETACHMENT + engine_native::ctask_force::REPAIR_PARENT,
            &destination.to_le_bytes(),
        );
        memory.put(
            DESTINATION + native::REPAIR_CHILDREN,
            &(CHILDREN as u64).to_le_bytes(),
        );
        memory.put(
            DESTINATION + native::REPAIR_CHILDREN + 8,
            &1_u32.to_le_bytes(),
        );
        memory.put(
            DESTINATION + native::REPAIR_CHILDREN + 12,
            &1_u32.to_le_bytes(),
        );
        memory.put(CHILDREN, &detachment.to_le_bytes());
        let mut initial = Session::begin(&memory, 0, 0);
        for (pointer, handle) in [(DESTINATION, destination), (DETACHMENT, detachment)] {
            initial.visit(native::CensusVisit::TaskForce { pointer, handle });
        }
        let initial = initial.finish(complete());
        let initial_tasks = initial.tables[2].as_ref().unwrap();

        memory.put(
            DETACHMENT + engine_native::ctask_force::FLEET,
            &2_u64.to_le_bytes(),
        );
        memory.put(
            DETACHMENT + engine_native::ctask_force::REPAIR_PARENT,
            &0_u64.to_le_bytes(),
        );
        memory.put(
            DESTINATION + native::REPAIR_CHILDREN + 12,
            &0_u32.to_le_bytes(),
        );
        changes::fixture::mark(DESTINATION + native::REPAIR_CHILDREN, false, false);
        let transferred = Session::begin(&memory, 0, 0).finish(complete());
        assert!(Arc::ptr_eq(
            initial_tasks,
            transferred.tables[2].as_ref().unwrap()
        ));
        assert_eq!(transferred.tables[6].as_ref().unwrap().len(), 0);

        memory
            .bytes
            .retain(|address, _| *address < DESTINATION || *address >= DETACHMENT);
        memory.reads.set(0);
        changes::fixture::mark(DESTINATION, false, true);
        let mut surviving = Session::begin(&memory, 0, 0);
        surviving.visit(native::CensusVisit::TaskForce {
            pointer: DETACHMENT,
            handle: detachment,
        });
        let surviving = surviving.finish(complete());
        let TableRows::NavalRepairTaskForces(rows) = surviving.tables[2].as_ref().unwrap().as_ref()
        else {
            panic!("task settings snapshot");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].task_force, detachment);
        assert_eq!(rows[0].merge_after_repair, destination);
        assert_eq!(rows[0].repair_mode, 1);
        assert_eq!(
            memory.reads.get(),
            0,
            "destination retirement must not reread its surviving source"
        );
        let quiet = Session::begin(&memory, 0, 0).finish(complete());
        assert!(Arc::ptr_eq(
            surviving.tables[2].as_ref().unwrap(),
            quiet.tables[2].as_ref().unwrap()
        ));
        assert_eq!(memory.reads.get(), 0);
    }

    #[test]
    fn naval_management_cancelled_or_pruned_refit_retires_progress_without_rewriting_raw_links() {
        let _guard = changes::fixture::SERIAL.lock().unwrap();
        const LINE: usize = 0x30000;
        let line =
            u64::from(engine_native::cship_refit_production_line::REFERENCE_TYPE) | (7_u64 << 32);
        let root = engine_native::creference_registry::LOW_ROOTS_RVA.0 + line as u32 as usize * 8;
        for cancel_clears_ship in [true, false] {
            changes::fixture::prepare();
            let mut memory = MemoryFixture::default();
            memory.put(
                engine_native::creference_registry::INVALID_REFERENCE_RVA.0,
                &0_u64.to_le_bytes(),
            );
            memory.refit(LINE, line);
            for (pointer, id) in [(0x10000, 1), (0x20000, 2)] {
                memory.ship(pointer, id, line);
            }
            let mut initial = Session::begin(&memory, 0, 0);
            for (pointer, id) in [(0x10000, 1), (0x20000, 2)] {
                initial.ship(
                    pointer,
                    &engine_native::cship::Row {
                        id,
                        ..Default::default()
                    },
                );
            }
            let initial = initial.finish(complete());
            assert_eq!(initial.tables[4].as_ref().unwrap().len(), 2);
            assert_eq!(initial.tables[5].as_ref().unwrap().len(), 1);

            if cancel_clears_ship {
                memory.put(0x10000 + native::SHIP_REFIT_LINE, &0_u64.to_le_bytes());
            }
            memory.put(root, &0_u64.to_le_bytes());
            memory
                .bytes
                .retain(|address, _| *address < LINE || *address >= LINE + 0x1000);
            memory.reads.set(0);
            changes::fixture::mark(LINE, false, true);
            let retired = Session::begin(&memory, 0, 0);
            assert_eq!(retired.store.line_receivers[&line], (0, 2));
            let retired = retired.finish(complete());
            assert!(Arc::ptr_eq(
                initial.tables[4].as_ref().unwrap(),
                retired.tables[4].as_ref().unwrap()
            ));
            assert_eq!(retired.tables[5].as_ref().unwrap().len(), 0);
            assert_eq!(memory.reads.get(), 0);
            let quiet = Session::begin(&memory, 0, 0).finish(complete());
            assert!(Arc::ptr_eq(
                retired.tables[4].as_ref().unwrap(),
                quiet.tables[4].as_ref().unwrap()
            ));
            assert_eq!(memory.reads.get(), 0);

            memory.ship(0x50000, 3, line);
            let mut admitted = Session::begin(&memory, 0, 0);
            admitted.ship(
                0x50000,
                &engine_native::cship::Row {
                    id: 3,
                    ..Default::default()
                },
            );
            assert_eq!(admitted.store.line_receivers[&line], (0, 3));
            let admitted = admitted.finish(complete());
            assert_eq!(admitted.tables[4].as_ref().unwrap().len(), 3);
            assert_eq!(admitted.tables[5].as_ref().unwrap().len(), 0);
            for pointer in [0x10000, 0x20000, 0x50000] {
                changes::fixture::mark(pointer, false, true);
            }
            let last = Session::begin(&memory, 0, 0);
            assert!(!last.store.line_receivers.contains_key(&line));
            assert_eq!(last.finish(complete()).tables[4].as_ref().unwrap().len(), 0);
        }
    }

    #[test]
    fn naval_management_raw_refit_link_survives_progress_lookup_refusal() {
        let _guard = changes::fixture::SERIAL.lock().unwrap();
        changes::fixture::prepare();
        let mut memory = MemoryFixture::default();
        memory.put(
            engine_native::creference_registry::INVALID_REFERENCE_RVA.0,
            &0_u64.to_le_bytes(),
        );
        let line =
            u64::from(engine_native::cship_refit_production_line::REFERENCE_TYPE) | (7_u64 << 32);
        memory.ship(0x10000, 1, line);
        let mut session = Session::begin(&memory, 0, 0);
        session.ship(
            0x10000,
            &engine_native::cship::Row {
                id: 1,
                ..Default::default()
            },
        );
        let captured = session.finish(complete());
        let TableRows::NavalRefitShips(rows) = captured.tables[4].as_ref().unwrap().as_ref() else {
            panic!("raw refit link snapshot");
        };
        assert_eq!(rows, &[NavalRefitShipRow { ship: 1, line }]);
        assert!(captured.tables[5].is_err());
    }

    #[test]
    fn naval_management_shared_refit_survives_one_ship_and_retires_with_the_last() {
        let _guard = changes::fixture::SERIAL.lock().unwrap();
        changes::fixture::prepare();
        let memory = MemoryFixture::default();
        let mut session = Session::begin(&memory, 0, 0);
        for (pointer, ship) in [(0x10000, 1), (0x20000, 2)] {
            assert!(session.retain(pointer, Entry::Ship(ship)));
            session
                .store
                .ships
                .insert(ship, NavalRefitShipRow { ship, line: 7 });
        }
        assert!(session.retain(0x30000, Entry::Line(7)));
        assert!(session.retain(0x40000, Entry::Fleet(9)));
        session.store.line_receivers.insert(7, (0x30000, 2));
        session.store.lines.insert(
            7,
            NavalRefitProgressRow {
                line: 7,
                progress_raw: 123,
            },
        );
        session.finish(complete());

        changes::fixture::mark(0x10000, false, true);
        let first = Session::begin(&memory, 0, 0);
        assert!(first.store.lines.contains_key(&7));
        assert_eq!(
            first.finish(complete()).tables[4].as_ref().unwrap().len(),
            1
        );
        changes::fixture::mark(0x20000, false, true);
        let last = Session::begin(&memory, 0, 0);
        assert_eq!(last.store.entries.get(&0x40000), Some(&Entry::Fleet(9)));
        assert!(!last.store.entries.contains_key(&0x30000));
        let last = last.finish(complete());
        assert_eq!(last.tables[4].as_ref().unwrap().len(), 0);
        assert_eq!(last.tables[5].as_ref().unwrap().len(), 0);
        assert_eq!(memory.reads.get(), 0);
    }
    #[test]
    fn naval_management_quiet_census_reads_nothing_dirty_reads_one_and_retire_precedes_read() {
        let _guard = changes::fixture::SERIAL.lock().unwrap();
        changes::fixture::prepare();
        let mut memory = MemoryFixture::default();
        for index in 0..5_000 {
            memory.fleet(0x10000 + index * 0x200, index as u64 + 1, 1);
        }
        let mut first = Session::begin(&memory, 0, 0);
        for index in 0..5_000 {
            first.visit(native::CensusVisit::Fleet {
                pointer: 0x10000 + index * 0x200,
                handle: index as u64 + 1,
            });
        }
        let first = first.finish(complete());
        assert_eq!(first.tables[1].as_ref().unwrap().len(), 5_000);
        memory.reads.set(0);
        let mut quiet = Session::begin(&memory, 0, 0);
        for index in 0..5_000 {
            let country = ccountry::Country {
                pointer: 0x2000000 + index * 0x10000,
                id: index as u32 + 1,
                canonical_slot: index as u32,
            };
            quiet.store.entries.insert(
                country.pointer,
                Entry::Country {
                    country,
                    production: 0,
                    navy: 0,
                },
            );
            quiet.visit(native::CensusVisit::Country(country));
            quiet.visit(native::CensusVisit::Fleet {
                pointer: 0x10000 + index * 0x200,
                handle: index as u64 + 1,
            });
        }
        let quiet = quiet.finish(complete());
        assert_eq!(memory.reads.get(), 0);
        assert!(Arc::ptr_eq(
            first.tables[1].as_ref().unwrap(),
            quiet.tables[1].as_ref().unwrap()
        ));
        let allocation_free = collector_test_support::with_allocations_denied(|| {
            let mut held = Session::begin(&memory, 0, 0);
            for index in 0..5_000 {
                held.visit(native::CensusVisit::Country(ccountry::Country {
                    pointer: 0x2000000 + index * 0x10000,
                    id: index as u32 + 1,
                    canonical_slot: index as u32,
                }));
                held.visit(native::CensusVisit::Fleet {
                    pointer: 0x10000 + index * 0x200,
                    handle: index as u64 + 1,
                });
            }
            held.finish(complete())
        });
        assert_eq!(memory.reads.get(), 0);
        assert_eq!(
            collector_test_support::ATTEMPTS.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert!(Arc::ptr_eq(
            quiet.tables[1].as_ref().unwrap(),
            allocation_free.tables[1].as_ref().unwrap()
        ));
        memory.fleet(0x10000, 1, 0);
        changes::fixture::mark(0x10000, false, false);
        let changed = Session::begin(&memory, 0, 0).finish(complete());
        assert_eq!(memory.reads.get(), 3);
        assert!(!Arc::ptr_eq(
            quiet.tables[1].as_ref().unwrap(),
            changed.tables[1].as_ref().unwrap()
        ));
        memory.bytes.retain(|address, _| *address >= 0x10200);
        memory.reads.set(0);
        changes::fixture::mark(0x10000, false, false);
        changes::fixture::mark(0x10000, false, true);
        let retired = Session::begin(&memory, 0, 0).finish(complete());
        assert_eq!(memory.reads.get(), 0);
        assert_eq!(retired.tables[1].as_ref().unwrap().len(), 4_999);
        memory.fleet(0x10000, 90_000, 7);
        let mut reused = Session::begin(&memory, 0, 0);
        reused.visit(native::CensusVisit::Fleet {
            pointer: 0x10000,
            handle: 90_000,
        });
        let reused = reused.finish(complete());
        assert_eq!(reused.tables[1].as_ref().unwrap().len(), 5_000);
        memory.bytes.remove(&(0x10000 + native::AUTOMATED_HOMEBASE));
        changes::fixture::mark(0x10000, false, false);
        let failed = Session::begin(&memory, 0, 0).finish(complete());
        assert!(failed.tables[1].is_err());
        assert!(failed.tables[2].is_ok());
        memory.reads.set(0);
        let _ = Session::begin(&memory, 0, 0).finish(complete());
        assert_eq!(memory.reads.get(), 0);
        // SAFETY: Unit test exclusively owns naval-management watch tables.
        unsafe { changes::boundary(true) };
        memory.fleet(0x10000, 90_000, 9);
        let mut reset = Session::begin(&memory, 0, 0);
        reset.visit(native::CensusVisit::Fleet {
            pointer: 0x10000,
            handle: 90_000,
        });
        assert_eq!(
            reset.finish(complete()).tables[1].as_ref().unwrap().len(),
            1
        );
        const BASE: usize = 0x9000;
        memory.put(
            BASE,
            &(engine_native::cnaval_base::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        memory.put(
            BASE + engine_native::cnaval_base::PROVINCE_ID,
            &17_u32.to_le_bytes(),
        );
        memory.put(BASE + native::BASE_REPAIR_PRIORITY, &1_i32.to_le_bytes());
        memory.put(BASE + native::BASE_REPAIR_DOCKYARDS, &0_i32.to_le_bytes());
        memory.put(BASE + native::BASE_REPAIR_QUEUE, &[0; 16]);
        // SAFETY: test holds `changes::fixture::SERIAL` as the sole naval-base census caller.
        let mut bases = unsafe { crate::bases::Session::begin(&memory, 0) };
        bases.visit(
            BASE,
            &engine_native::cnaval_base::Row {
                province: 17,
                ..Default::default()
            },
        );
        let initial = bases.finish(engine_native::walk::Outcome::Rows { len: 1 });
        memory.reads.set(0);
        // SAFETY: test holds `changes::fixture::SERIAL` as the sole naval-base census caller.
        let mut bases = unsafe { crate::bases::Session::begin(&memory, 0) };
        bases.visit(
            BASE,
            &engine_native::cnaval_base::Row {
                province: 17,
                ..Default::default()
            },
        );
        let quiet = bases.finish(engine_native::walk::Outcome::Rows { len: 1 });
        assert_eq!(memory.reads.get(), 0);
        assert!(Arc::ptr_eq(
            initial.tables[0].as_ref().unwrap(),
            quiet.tables[0].as_ref().unwrap()
        ));
        memory.put(BASE + native::BASE_REPAIR_DOCKYARDS, &3_i32.to_le_bytes());
        changes::fixture::mark(BASE, true, false);
        // SAFETY: test holds `changes::fixture::SERIAL` as the sole naval-base census caller.
        let changed = unsafe { crate::bases::Session::begin(&memory, 0) }
            .finish(engine_native::walk::Outcome::Rows { len: 1 });
        assert!(memory.reads.get() > 0);
        assert!(!Arc::ptr_eq(
            quiet.tables[0].as_ref().unwrap(),
            changed.tables[0].as_ref().unwrap()
        ));
        memory.reads.set(0);
        changes::fixture::mark(BASE, true, false);
        changes::fixture::mark(BASE, true, true);
        // SAFETY: test holds `changes::fixture::SERIAL` as the sole naval-base census caller.
        let gone = unsafe { crate::bases::Session::begin(&memory, 0) }
            .finish(engine_native::walk::Outcome::Rows { len: 0 });
        assert_eq!(memory.reads.get(), 0);
        assert_eq!(gone.tables[0].as_ref().unwrap().len(), 0);
    }
}
