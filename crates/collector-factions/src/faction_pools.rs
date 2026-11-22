use engine_allocation as allocation;
use engine_native::Memory;
use engine_resources::cstrategic_resource_pool as native;
use replay_data::rows::{FactionResourceRow, TableRows, tables};
use std::{
    ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicU8, AtomicU64, AtomicUsize, Ordering},
    },
};
static POINTER: AtomicPtr<Storage> = AtomicPtr::new(ptr::null_mut());
static CLOSED: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static READERS: AtomicUsize = AtomicUsize::new(0);
static VERSION: AtomicU64 = AtomicU64::new(0);
static NEXT_PHASE: AtomicU64 = AtomicU64::new(0);
static MISSED: AtomicBool = AtomicBool::new(false);
pub fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
}
struct Flight;
impl Flight {
    fn new() -> Self {
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        VERSION.fetch_add(1, Ordering::SeqCst);
        Self
    }
}
impl Drop for Flight {
    fn drop(&mut self) {
        VERSION.fetch_add(1, Ordering::SeqCst);
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}
fn stamp(ignore_missed: bool) -> Option<u64> {
    let version = VERSION.load(Ordering::SeqCst);
    (active() == 0
        && (ignore_missed || !MISSED.load(Ordering::SeqCst))
        && VERSION.load(Ordering::SeqCst) == version)
        .then_some(version)
}
struct Cell {
    id: u64,
    address: usize,
    pool: usize,
    projection: [AtomicI64; 3],
    resources: Vec<AtomicI64>,
    resource_dirty: Vec<AtomicU64>,
    phases: [AtomicU64; 4],
    phase_members: [AtomicU64; 2],
    awards: AtomicI64,
    flags: AtomicU8,
}
struct Storage {
    cells: Vec<Cell>,
    owners: Vec<(usize, usize)>,
    pools: Vec<(usize, usize)>,
    dirty: Vec<AtomicU64>,
    invalid: AtomicU8,
}
impl Storage {
    fn new(held: &[Held]) -> Result<Box<Self>, &'static str> {
        let mut cells = Vec::new();
        let mut owners = Vec::new();
        let mut pools = Vec::new();
        let mut dirty = Vec::new();
        allocation::reserve(&mut cells, held.len())
            .map_err(|_| "faction pool cell allocation failed")?;
        allocation::reserve(&mut owners, held.len())
            .map_err(|_| "faction pool owner allocation failed")?;
        allocation::reserve(&mut pools, held.len())
            .map_err(|_| "faction pool index allocation failed")?;
        allocation::reserve(&mut dirty, held.len().div_ceil(64))
            .map_err(|_| "faction pool mark allocation failed")?;
        for (index, value) in held.iter().enumerate() {
            let mut resources = Vec::new();
            allocation::reserve(&mut resources, value.resources.len())
                .map_err(|_| "faction pool operands allocation failed")?;
            resources.extend(
                value
                    .resources
                    .iter()
                    .map(|value| AtomicI64::new(value.raw)),
            );
            let mut resource_dirty = Vec::new();
            allocation::reserve(&mut resource_dirty, value.resources.len().div_ceil(64))
                .map_err(|_| "faction resource marks allocation failed")?;
            resource_dirty
                .extend((0..value.resources.len().div_ceil(64)).map(|_| AtomicU64::new(0)));
            cells.push(Cell {
                id: value.id,
                address: value.pointer,
                pool: value.pool,
                projection: value.projection.map(AtomicI64::new),
                resources,
                resource_dirty,
                phases: value.phases.map(AtomicU64::new),
                phase_members: value.phase_members.map(AtomicU64::new),
                awards: AtomicI64::new(value.awards),
                flags: AtomicU8::new(0),
            });
            if value.id != 0 {
                owners.push((value.pointer, index));
            }
            pools.push((value.pool, index));
        }
        owners.sort_unstable();
        pools.sort_unstable();
        dirty.extend((0..held.len().div_ceil(64)).map(|_| AtomicU64::new(0)));
        allocation::boxed(Self {
            cells,
            owners,
            pools,
            dirty,
            invalid: AtomicU8::new(0),
        })
        .map_err(|_| "faction pool storage allocation failed")
    }
    fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.cells.capacity() * size_of::<Cell>()
            + (self.owners.capacity() + self.pools.capacity()) * size_of::<(usize, usize)>()
            + self.dirty.capacity() * size_of::<AtomicU64>()
            + self
                .cells
                .iter()
                .map(|cell| {
                    cell.resources.capacity() * size_of::<AtomicI64>()
                        + cell.resource_dirty.capacity() * size_of::<AtomicU64>()
                })
                .sum::<usize>()
    }
    fn write(&self, receiver: usize, resource: bool, index: usize, value: i64, add: bool) {
        let map = if resource { &self.pools } else { &self.owners };
        let Ok(at) = map.binary_search_by_key(&receiver, |value| value.0) else {
            return;
        };
        let slot = map[at].1;
        let cell = &self.cells[slot];
        let target = if resource {
            cell.resources.get(index)
        } else {
            cell.projection.get(index)
        };
        let flag = if resource { 2 } else { 1 };
        let Some(target) = target else {
            self.invalid.fetch_or(flag, Ordering::SeqCst);
            return;
        };
        if !resource && index == 1 {
            if add {
                cell.awards.fetch_add(value, Ordering::Relaxed);
            } else {
                cell.awards.store(0, Ordering::Relaxed);
            }
        }
        if add {
            target.fetch_add(value, Ordering::Relaxed);
        } else {
            target.store(value, Ordering::Relaxed);
        }
        if resource {
            cell.resource_dirty[index / 64].fetch_or(1u64 << (index % 64), Ordering::SeqCst);
        }
        cell.flags.fetch_or(flag, Ordering::Relaxed);
        self.dirty[slot / 64].fetch_or(1u64 << (slot % 64), Ordering::SeqCst);
    }
    fn rebuild(&self, memory: &impl Memory) {
        for (index, cell) in self.cells.iter().enumerate() {
            let header = cell
                .pool
                .checked_add(native::VALUES)
                .and_then(|at| engine_native::read::<[u8; 16]>(memory, at));
            let source = header.and_then(|header| {
                let data =
                    usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap())).ok()?;
                let capacity = i32::from_le_bytes(header[8..12].try_into().unwrap());
                let count = i32::from_le_bytes(header[12..16].try_into().unwrap());
                (count >= 0
                    && count <= capacity
                    && count as usize == cell.resources.len()
                    && (count == 0 || data != 0))
                    .then_some(data)
            });
            let Some(data) = source else {
                self.invalid.fetch_or(2, Ordering::SeqCst);
                continue;
            };
            let mut changed = false;
            for ordinal in 1..cell.resources.len() {
                let value = ordinal
                    .checked_mul(native::VALUE_STRIDE)
                    .and_then(|offset| data.checked_add(offset))
                    .and_then(|at| engine_native::read::<i64>(memory, at));
                match value {
                    Some(value) => {
                        let target = &cell.resources[ordinal];
                        if target.load(Ordering::Relaxed) != value {
                            target.store(value, Ordering::Relaxed);
                            cell.resource_dirty[ordinal / 64]
                                .fetch_or(1u64 << (ordinal % 64), Ordering::Relaxed);
                            changed = true;
                        }
                    }
                    None => {
                        self.invalid.fetch_or(2, Ordering::SeqCst);
                        break;
                    }
                }
            }
            if changed {
                cell.flags.fetch_or(2, Ordering::Relaxed);
                self.dirty[index / 64].fetch_or(1u64 << (index % 64), Ordering::SeqCst);
            }
        }
    }

    fn drain(&self, held: &mut [Held]) -> [bool; 2] {
        let mut changed = [false; 2];
        for (word, bits) in self.dirty.iter().enumerate() {
            let mut bits = bits.swap(0, Ordering::SeqCst);
            while bits != 0 {
                let index = word * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let cell = &self.cells[index];
                let Ok(at) = held.binary_search_by_key(&cell.id, |held| held.id) else {
                    continue;
                };
                let value = &mut held[at];
                if value.pointer != cell.address || value.pool != cell.pool {
                    continue;
                }
                let flags = cell.flags.swap(0, Ordering::SeqCst);
                if flags & 1 != 0 {
                    value.phases = cell
                        .phases
                        .each_ref()
                        .map(|value| value.load(Ordering::Relaxed));
                    value.phase_members = cell
                        .phase_members
                        .each_ref()
                        .map(|value| value.load(Ordering::Relaxed));
                    value.awards = cell.awards.load(Ordering::Relaxed);
                    value.projection = cell
                        .projection
                        .each_ref()
                        .map(|value| value.load(Ordering::Relaxed));
                    changed[0] = true;
                }
                if flags & 2 != 0 {
                    for (word, bits) in cell.resource_dirty.iter().enumerate() {
                        let mut bits = bits.swap(0, Ordering::SeqCst);
                        while bits != 0 {
                            let slot = word * 64 + bits.trailing_zeros() as usize;
                            bits &= bits - 1;
                            value.resources[slot].raw =
                                cell.resources[slot].load(Ordering::Relaxed);
                        }
                    }
                    changed[1] = true;
                }
            }
        }
        changed
    }
}
pub fn observe(receiver: usize, resource: bool, index: usize, value: i64, add: bool) {
    let hazard = Hazard::new();
    if CLOSED.load(Ordering::SeqCst) {
        return;
    }
    let pointer = POINTER.load(Ordering::SeqCst);
    if pointer.is_null() {
        return;
    }
    let storage = unsafe { &*pointer };
    let map = if resource {
        &storage.pools
    } else {
        &storage.owners
    };
    if map
        .binary_search_by_key(&receiver, |value| value.0)
        .is_err()
    {
        return;
    }
    let _flight = Flight::new();
    storage.write(receiver, resource, index, value, add);
    drop(hazard);
}
thread_local! {static PHASES:std::cell::Cell<[Option<(u64,u64,u32)>;2]>=const{std::cell::Cell::new([None;2])};}
pub fn phase(kind: usize) -> Option<(u64, u64, u32)> {
    PHASES.with(|phases| phases.get()[kind])
}
pub fn rank_current(faction: u64) -> Option<(u64, u32)> {
    let _hazard = Hazard::new();
    if CLOSED.load(Ordering::SeqCst) {
        return None;
    }
    let pointer = POINTER.load(Ordering::SeqCst);
    if pointer.is_null() {
        return None;
    }
    let storage = unsafe { &*pointer };
    let index = storage
        .cells
        .binary_search_by_key(&faction, |cell| cell.id)
        .ok()?;
    let cell = &storage.cells[index];
    Some((
        cell.phases[1].load(Ordering::Relaxed),
        cell.phase_members[1].load(Ordering::Relaxed) as u32,
    ))
}
pub fn phase_call(receiver: usize, kind: usize, original: impl FnOnce() -> usize) -> usize {
    let hazard = std::mem::ManuallyDrop::new(Hazard::new());
    let pointer = POINTER.load(Ordering::SeqCst);
    let selected = if CLOSED.load(Ordering::SeqCst) || pointer.is_null() {
        None
    } else {
        let storage = unsafe { &*pointer };
        storage
            .owners
            .binary_search_by_key(&receiver, |entry| entry.0)
            .ok()
            .map(|at| (storage, storage.owners[at].1))
    };
    let flight = std::mem::ManuallyDrop::new(selected.map(|_| Flight::new()));
    let previous = PHASES.with(|phases| {
        let previous = phases.get();
        let mut current = previous;
        current[kind] = None;
        phases.set(current);
        previous
    });
    let mut completed = None;
    if let Some((storage, index)) = selected {
        let cell = &storage.cells[index];
        let _reads =
            collector_memory::read_accounting::table(replay_data::rows::TableId::FactionProjection);
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        if let Some(count) = receiver
            .checked_add(engine_native::cfaction::MEMBER_COUNT)
            .and_then(|at| engine_native::read::<i32>(&memory, at))
            .filter(|count| *count >= 0)
        {
            let epoch = NEXT_PHASE
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                    value.checked_add(1)
                })
                .ok()
                .and_then(|value| value.checked_add(1));
            if let Some(epoch) = epoch {
                let mut values = previous;
                values[kind] = Some((cell.id, epoch, count as u32));
                PHASES.with(|phases| phases.set(values));
                completed = Some((epoch, count));
            } else {
                storage.invalid.fetch_or(1, Ordering::SeqCst);
            }
        } else {
            storage.invalid.fetch_or(1, Ordering::SeqCst);
        }
    }
    let result = original();
    PHASES.with(|phases| phases.set(previous));
    if let (Some((storage, index)), Some((epoch, count))) = (selected, completed) {
        let cell = &storage.cells[index];
        cell.phases[kind].store(epoch, Ordering::Relaxed);
        cell.phase_members[kind].store(count as u64, Ordering::Relaxed);
        cell.flags.fetch_or(1, Ordering::Relaxed);
        storage.dirty[index / 64].fetch_or(1u64 << (index % 64), Ordering::SeqCst);
    }
    drop(std::mem::ManuallyDrop::into_inner(flight));
    drop(std::mem::ManuallyDrop::into_inner(hazard));
    result
}
pub fn boundary_call(receiver: usize, kind: usize, original: impl FnOnce() -> usize) -> usize {
    let hazard = std::mem::ManuallyDrop::new(Hazard::new());
    let pointer = POINTER.load(Ordering::SeqCst);
    let selected = if CLOSED.load(Ordering::SeqCst) || pointer.is_null() {
        None
    } else {
        let storage = unsafe { &*pointer };
        storage
            .owners
            .binary_search_by_key(&receiver, |entry| entry.0)
            .ok()
            .map(|at| (storage, storage.owners[at].1))
    };
    let flight = std::mem::ManuallyDrop::new(selected.map(|_| Flight::new()));
    let result = original();
    if let Some((storage, index)) = selected {
        if let Some(phase) = NEXT_PHASE
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                value.checked_add(1)
            })
            .ok()
            .and_then(|value| value.checked_add(1))
        {
            let cell = &storage.cells[index];
            cell.phases[kind].store(phase, Ordering::Relaxed);
            cell.flags.fetch_or(1, Ordering::Relaxed);
            storage.dirty[index / 64].fetch_or(1u64 << (index % 64), Ordering::SeqCst);
        } else {
            storage.invalid.fetch_or(1, Ordering::SeqCst);
        }
    }
    drop(std::mem::ManuallyDrop::into_inner(flight));
    drop(std::mem::ManuallyDrop::into_inner(hazard));
    result
}
struct Hazard;
impl Hazard {
    fn new() -> Self {
        READERS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}
impl Drop for Hazard {
    fn drop(&mut self) {
        READERS.fetch_sub(1, Ordering::SeqCst);
    }
}
pub fn rebuild() {
    let _hazard = Hazard::new();
    if CLOSED.load(Ordering::SeqCst) {
        MISSED.store(true, Ordering::SeqCst);
        return;
    }
    let pointer = POINTER.load(Ordering::SeqCst);
    if pointer.is_null() {
        return;
    }
    let storage = unsafe { &*pointer };
    let _flight = Flight::new();
    let _reads =
        collector_memory::read_accounting::table(replay_data::rows::TableId::FactionResources);
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    storage.rebuild(&memory);
}

#[derive(Default)]
struct Held {
    id: u64,
    pointer: usize,
    pool: usize,
    seen: u64,
    certified: bool,
    projection: [i64; 3],
    phases: [u64; 4],
    phase_members: [u64; 2],
    awards: i64,
    influence_derived: bool,
    influence_accounts: Vec<u32>,
    resources: Vec<native::StoredAmount>,
    failure: [Option<&'static str>; 2],
}
#[derive(Default)]
pub struct Store {
    ticks: u64,
    held: Vec<Held>,

    storage: Option<Box<Storage>>,
    epoch: Option<u64>,
    before: Option<u64>,
    scalar_before: Option<u64>,
    generation: u64,
    seen: usize,
    force: bool,
    shape: bool,
    changed: [bool; 2],
    failure: [Option<&'static str>; 2],
    read_failure: [Option<&'static str>; 2],
    system_epoch: Option<u64>,
    system_failure: Option<&'static str>,
    incomplete_epoch: Option<u64>,
    rows: [Option<Arc<TableRows>>; 2],
    spares: [Vec<Arc<TableRows>>; 2],
}
impl Store {
    pub fn begin(&mut self, force: bool, epoch: Option<u64>) {
        let started = collector_primitives::clock::qpc();
        self.ticks = 0;
        self.failure = [None; 2];
        self.read_failure = [None; 2];
        self.before = epoch;
        if self.incomplete_epoch != epoch {
            self.incomplete_epoch = None;
        }
        self.force = force || self.epoch != self.before;
        self.scalar_before = stamp(self.force);
        self.seen = 0;
        if let Some(generation) = self.generation.checked_add(1) {
            self.generation = generation;
        } else {
            self.failure
                .fill(Some("faction pool visitation generation exhausted"));
        }
        if self.before.is_none() {
            self.failure
                .fill(Some("faction pools could not be followed in the game"));
            return;
        }
        if self.scalar_before.is_none() {
            self.failure
                .fill(Some("faction pool scalar writer overlaps acquisition"));
            return;
        }
        if let Some(storage) = &self.storage {
            let changed = storage.drain(&mut self.held);
            for (current, changed) in self.changed.iter_mut().zip(changed) {
                *current |= changed;
            }
        }
        self.ticks += collector_primitives::clock::qpc()
            .saturating_sub(started)
            .max(0) as u64;
    }
    pub fn visit(&mut self, memory: &impl Memory, base: usize, pointer: usize, id: u64) {
        if self.before.is_none() || self.incomplete_epoch.is_some() {
            return;
        }
        let index = match self.held.binary_search_by_key(&id, |value| value.id) {
            Ok(index) => index,
            Err(index) => {
                if allocation::reserve(&mut self.held, 1).is_err() {
                    self.failure
                        .fill(Some("faction pool owner allocation failed"));
                    return;
                }
                self.held.insert(
                    index,
                    Held {
                        id,
                        pointer,
                        ..Default::default()
                    },
                );
                self.shape = true;
                index
            }
        };
        let value = &mut self.held[index];
        if value.seen != self.generation {
            value.seen = self.generation;
            self.seen += 1;
        }
        let fresh = self.force || !value.certified || value.pointer != pointer;
        if !fresh {
            for kind in 0..2 {
                if let Some(reason) = value.failure[kind] {
                    self.read_failure[kind].get_or_insert(reason);
                }
            }
            return;
        }
        let started = collector_primitives::clock::qpc();
        value.pointer = pointer;
        value.phases = [0; 4];
        value.phase_members = [0; 2];
        value.awards = 0;
        value.certified = false;
        value.failure = [None; 2];
        self.changed = [true; 2];
        self.shape = true;
        if id != 0 {
            let _reads = collector_memory::read_accounting::table(
                replay_data::rows::TableId::FactionProjection,
            );
            let result = pointer
                .checked_add(engine_native::cfaction::POWER_PROJECTION_TOTAL)
                .and_then(|at| {
                    engine_native::read::<[u8; 24]>(memory, at).map(|bytes| {
                        std::array::from_fn(|index| {
                            i64::from_le_bytes(bytes[index * 8..index * 8 + 8].try_into().unwrap())
                        })
                    })
                });
            match result {
                Some(values) => value.projection = values,
                None => value.failure[0] = Some("faction projection totals unreadable"),
            }
        }
        let offset = if id == 0 {
            engine_native::cfaction_system::EXTRACTED_RESOURCES
        } else {
            engine_native::cfaction::EXTRACTED_RESOURCES
        };
        let Some(pool) = pointer.checked_add(offset) else {
            value.failure[1] = Some("faction resource pool address overflow");
            self.read_failure[1] = value.failure[1];
            return;
        };
        value.pool = pool;
        let _reads =
            collector_memory::read_accounting::table(replay_data::rows::TableId::FactionResources);
        if let Err(reason) = native::read_into(memory, base, pool, &mut value.resources) {
            value.failure[1] = Some(reason);
        }
        for kind in 0..2 {
            if let Some(reason) = value.failure[kind] {
                self.read_failure[kind].get_or_insert(reason);
            }
        }
        self.ticks += collector_primitives::clock::qpc()
            .saturating_sub(started)
            .max(0) as u64;
    }
    pub fn system(&mut self, memory: &impl Memory, base: usize, state: usize) {
        if self.before.is_none() || self.incomplete_epoch.is_some() {
            return;
        }
        if !self.force
            && self.system_epoch == self.before
            && let Some(reason) = self.system_failure
        {
            self.read_failure[1] = Some(reason);
            return;
        }
        if !self.force && self.held.first().is_some_and(|value| value.id == 0) {
            let pointer = self.held[0].pointer;
            self.visit(memory, base, pointer, 0);
            return;
        }
        let pointer = state
            .checked_add(engine_native::cgame_state::FACTION_SYSTEM)
            .and_then(|at| engine_native::read::<u64>(memory, at))
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value != 0);
        let Some(pointer) = pointer else {
            self.system_epoch = self.before;
            self.system_failure = Some("faction system resource owner unavailable");
            self.read_failure[1] = self.system_failure;
            return;
        };
        let vtable = engine_sites::at(base, engine_native::cfaction_system::VTABLE_RVA);
        if vtable.is_none()
            || engine_native::read::<u64>(memory, pointer) != vtable.map(|v| v as u64)
        {
            self.system_epoch = self.before;
            self.system_failure = Some("faction system resource owner vtable mismatch");
            self.read_failure[1] = self.system_failure;
            return;
        }
        self.system_epoch = self.before;
        self.system_failure = None;
        self.visit(memory, base, pointer, 0);
    }
    pub fn finish(
        &mut self,
        complete: bool,
        epoch: Option<u64>,
        accounts: Option<&crate::faction_accounts::Store>,
    ) {
        for value in &mut self.held {
            let cohort =
                accounts.and_then(|accounts| accounts.influence_cohort(value.id, value.phases[0]));
            let derived = value.phases[0] != 0
                && cohort.is_some_and(|rows| rows.len() == value.phase_members[0] as usize);
            if derived != value.influence_derived {
                self.changed[0] = true;
                value.influence_derived = derived;
            }
            if let Some(cohort) = cohort.filter(|_| derived)
                && (cohort.len() != value.influence_accounts.len()
                    || cohort
                        .iter()
                        .zip(&value.influence_accounts)
                        .any(|(source, target)| source.1 != *target))
            {
                let additional = cohort.len().saturating_sub(value.influence_accounts.len());
                if allocation::reserve(&mut value.influence_accounts, additional).is_err() {
                    self.read_failure[0] = Some("faction influence cohort allocation failed");
                } else {
                    value.influence_accounts.clear();
                    value
                        .influence_accounts
                        .extend(cohort.iter().map(|row| row.1));
                    self.changed[0] = true;
                }
            }
        }
        if self.incomplete_epoch.is_some() {
            self.failure.fill(Some(
                "faction pool baseline awaits a complete identity roster",
            ));
            if complete {
                self.incomplete_epoch = None;
            }
            return;
        }
        if !complete {
            self.incomplete_epoch = self.before;
            self.failure
                .fill(Some("faction pool identity roster incomplete"));
        }
        if self.before.is_none() || self.before != epoch {
            self.failure.fill(Some(
                "faction pool writer lifetime changed during acquisition",
            ));
        }
        if self.scalar_before.is_none() || self.scalar_before != stamp(self.force) {
            self.failure.fill(Some(
                "faction pool scalar writes changed during acquisition",
            ));
        }
        if self.failure.iter().any(Option::is_some) {
            if self.force || self.incomplete_epoch.is_some() || self.before != epoch {
                self.epoch = None;
            }
            return;
        }
        if self.seen != self.held.len() {
            self.held.retain(|value| value.seen == self.generation);
            self.shape = true;
            self.changed = [true; 2];
        }
        if self.shape || self.force || self.storage.is_none() {
            let result = Storage::new(&self.held).and_then(|mut storage| {
                CLOSED.store(true, Ordering::SeqCst);
                if active() != 0 || READERS.load(Ordering::SeqCst) != 0 {
                    CLOSED.store(false, Ordering::SeqCst);
                    return Err("faction pool publication overlaps a writer");
                }
                MISSED.store(false, Ordering::SeqCst);
                POINTER.store(&mut *storage, Ordering::SeqCst);
                let old = self.storage.replace(storage);
                CLOSED.store(false, Ordering::SeqCst);
                drop(old);
                Ok(())
            });
            if let Err(reason) = result {
                self.failure.fill(Some(reason));
                if self.force {
                    self.epoch = None;
                }
                return;
            }
        }
        if self.scalar_before != stamp(false) {
            self.failure
                .fill(Some("faction pool writer overlapped operand publication"));
            if self.force {
                self.epoch = None;
            }
            return;
        }
        if self.shape || self.force {
            for value in &mut self.held {
                value.certified = true;
            }
        }
        self.shape = false;
        for kind in 0..2 {
            if let Some(reason) = self.read_failure[kind] {
                self.failure[kind].get_or_insert(reason);
            }
        }
        if let Some(storage) = &self.storage {
            let invalid = storage.invalid.load(Ordering::SeqCst);
            if invalid & 1 != 0 {
                self.failure[0] = Some("faction projection writer lane invalid");
            }
            if invalid & 2 != 0 {
                self.failure[1] = Some("faction resource store index exceeds its certified pool");
            }
        }
        for kind in 0..2 {
            if self.failure[kind].is_none() && (self.changed[kind] || self.rows[kind].is_none()) {
                if let Err(reason) = self.publish(kind) {
                    self.failure[kind] = Some(reason);
                } else {
                    self.changed[kind] = false;
                }
            }
        }
        self.epoch = self.before;
    }
    fn publish(&mut self, kind: usize) -> Result<(), &'static str> {
        let mut table =
            crate::faction_goals::writable(&mut self.rows[kind], &mut self.spares[kind], || {
                if kind == 0 {
                    TableRows::FactionProjection(Vec::new())
                } else {
                    TableRows::FactionResources(Vec::new())
                }
            })?;
        if kind == 0 {
            let rows = Arc::get_mut(&mut table)
                .unwrap()
                .rows_mut::<tables::FactionProjection>()
                .unwrap();
            allocation::resize_default(
                rows,
                self.held.iter().filter(|value| value.id != 0).count(),
            )
            .map_err(|_| "faction projection replay allocation failed")?;
            for (row, value) in rows
                .iter_mut()
                .zip(self.held.iter().filter(|value| value.id != 0))
            {
                let additional = value
                    .influence_accounts
                    .len()
                    .saturating_sub(row.influence_accounts.len());
                allocation::reserve(&mut row.influence_accounts, additional)
                    .map_err(|_| "faction influence replay cohort allocation failed")?;
                row.influence_accounts.clear();
                row.influence_accounts
                    .extend_from_slice(&value.influence_accounts);
            }
            for (row, value) in rows
                .iter_mut()
                .zip(self.held.iter().filter(|value| value.id != 0))
            {
                row.faction = value.id;
                row.projection_raw = value.projection[0];
                row.influence_raw = if value.influence_derived {
                    0
                } else {
                    value.projection[1]
                };
                row.influence_phase = if value.influence_derived {
                    value.phases[0]
                } else {
                    0
                };
                row.influence_members = value.phase_members[0] as u32;
                row.influence_awards_raw = value.awards;
                row.effects_raw = value.projection[2];
                row.rank_phase = value.phases[1];
                row.rank_members = value.phase_members[1] as u32;
                row.projection_phase = value.phases[2];
                row.influence_share_phase = value.phases[3];
            }
        } else {
            let rows = Arc::get_mut(&mut table)
                .unwrap()
                .rows_mut::<tables::FactionResources>()
                .unwrap();
            rows.clear();
            let count = self.held.iter().map(|value| value.resources.len()).sum();
            allocation::reserve(rows, count)
                .map_err(|_| "faction resource replay allocation failed")?;
            for value in &self.held {
                for (ordinal, resource) in value.resources.iter().enumerate() {
                    rows.push(FactionResourceRow {
                        faction: value.id,
                        ordinal: ordinal as u32,
                        token: resource.token,
                        raw: resource.raw,
                    });
                }
            }
        }
        self.rows[kind] = Some(table);
        Ok(())
    }
    pub fn output(&self, kind: usize) -> (Option<Arc<TableRows>>, Option<&'static str>) {
        (
            self.failure[kind]
                .is_none()
                .then(|| self.rows[kind].as_ref().map(Arc::clone))
                .flatten(),
            self.failure[kind],
        )
    }
    pub fn ticks(&self) -> u64 {
        self.ticks
    }
    pub fn allocated_bytes(&self) -> usize {
        self.storage
            .as_ref()
            .map_or(0, |storage| storage.allocated_bytes())
            + self
                .spares
                .iter()
                .map(|spares| spares.capacity() * size_of::<Arc<TableRows>>())
                .sum::<usize>()
            + self.held.capacity() * size_of::<Held>()
            + self
                .held
                .iter()
                .map(|value| {
                    value.resources.capacity() * size_of::<native::StoredAmount>()
                        + value.influence_accounts.capacity() * size_of::<u32>()
                })
                .sum::<usize>()
            + self
                .rows
                .iter()
                .flatten()
                .chain(self.spares.iter().flatten())
                .map(|value| value.allocated_bytes() + size_of::<TableRows>())
                .sum::<usize>()
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        let Some(storage) = self.storage.take() else {
            return;
        };
        if std::ptr::eq(POINTER.load(Ordering::SeqCst), &*storage) {
            CLOSED.store(true, Ordering::SeqCst);
            POINTER.store(ptr::null_mut(), Ordering::SeqCst);
            if active() != 0 || READERS.load(Ordering::SeqCst) != 0 {
                std::mem::forget(storage);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell as ReadCount;
    const FIRST: usize = 0x1000;
    const STRIDE: usize = 0x1000;
    struct MemoryCount {
        bytes: Vec<u8>,
        reads: ReadCount<usize>,
    }
    impl Memory for MemoryCount {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            engine_native::memory::SliceMemory {
                base: 0,
                bytes: &self.bytes,
            }
            .copy(address, output)
        }
    }
    fn put(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn fixture(count: usize) -> MemoryCount {
        let mut bytes = vec![0; FIRST + count * STRIDE];
        for index in 0..count {
            let owner = FIRST + index * STRIDE;
            let pool = owner + engine_native::cfaction::EXTRACTED_RESOURCES;
            put(&mut bytes, pool, native::VTABLE_RVA.0 as u64);
            put(&mut bytes, pool + 8, (owner + 0xa00) as u64);
            put(&mut bytes, pool + 16, 3u64 << 32 | 3);
            for slot in 0..3 {
                put(
                    &mut bytes,
                    owner + 0xa00 + slot * native::VALUE_STRIDE + 8,
                    slot as u64 + 10,
                );
            }
        }
        MemoryCount {
            bytes,
            reads: ReadCount::new(0),
        }
    }
    fn pass(store: &mut Store, memory: &MemoryCount, count: usize, epoch: u64) {
        store.begin(false, Some(epoch));
        for index in 0..count {
            store.visit(memory, 0, FIRST + index * STRIDE, index as u64 + 1);
        }
        store.finish(true, Some(epoch), None);
    }
    #[test]
    fn final_extraction_reads_only_registered_pools_and_preserves_lifetime_slot_zero() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut memory = fixture(2);
        let mut store = Store::default();
        pass(&mut store, &memory, 1, 1);
        put(&mut memory.bytes, FIRST + 0xa00, 999);
        put(&mut memory.bytes, FIRST + 0xa00 + native::VALUE_STRIDE, 17);
        put(
            &mut memory.bytes,
            FIRST + STRIDE + 0xa00 + native::VALUE_STRIDE,
            23,
        );
        let reads = memory.reads.get();
        let storage = store.storage.as_ref().unwrap();
        collector_test_support::with_allocations_denied(|| {
            storage.rebuild(&memory);
            assert_eq!(storage.drain(&mut store.held), [false, true]);
            assert_eq!(store.held[0].resources[0].raw, 0);
            assert_eq!(store.held[0].resources[1].raw, 17);
            storage.rebuild(&memory);
            assert_eq!(storage.drain(&mut store.held), [false, false]);
        });
        assert_eq!(memory.reads.get() - reads, 6);
    }
    #[test]
    fn owned_scalar_overlap_preserves_unpublished_operands_without_full_baselines() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let memory = fixture(1);
        let mut store = Store::default();
        pass(&mut store, &memory, 1, 1);
        let reads = memory.reads.get();
        store
            .storage
            .as_ref()
            .unwrap()
            .write(FIRST, false, 2, 17, false);
        store.begin(false, Some(1));
        store.visit(&memory, 0, FIRST, 1);
        drop(Flight::new());
        store.finish(true, Some(1), None);
        assert!(store.output(0).0.is_none());
        pass(&mut store, &memory, 1, 1);
        assert_eq!(memory.reads.get(), reads);
        assert_eq!(
            store
                .output(0)
                .0
                .unwrap()
                .rows::<tables::FactionProjection>()
                .unwrap()[0]
                .effects_raw,
            17
        );
    }
    #[test]
    fn unregistered_faction_phases_shadow_outer_cohorts_and_restore_them_afterward() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = PHASES.with(|phases| phases.replace([Some((11, 12, 2)), Some((21, 22, 3))]));
        collector_test_support::with_allocations_denied(|| {
            assert_eq!(
                phase_call(0xdead0000, 1, || {
                    assert_eq!(phase(0), Some((11, 12, 2)));
                    assert_eq!(phase(1), None);
                    assert_eq!(
                        phase_call(0xdead1000, 0, || {
                            assert_eq!(phase(0), None);
                            assert_eq!(phase(1), None);
                            17
                        }),
                        17
                    );
                    assert_eq!(phase(0), Some((11, 12, 2)));
                    assert_eq!(phase(1), None);
                    29
                }),
                29
            );
        });
        assert_eq!(phase(0), Some((11, 12, 2)));
        assert_eq!(phase(1), Some((21, 22, 3)));
        PHASES.with(|phases| phases.set(previous));
    }

    #[test]
    fn foreign_pool_traffic_never_enters_the_owned_epoch_or_requests_baselines() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let memory = fixture(1);
        let mut store = Store::default();
        pass(&mut store, &memory, 1, 1);
        let stamp_before = stamp(false);
        let reads = memory.reads.get();
        collector_test_support::with_allocations_denied(|| {
            for ordinal in 0..5000 {
                observe(0xdead0000, true, ordinal, 9, false);
            }
            assert_eq!(stamp(false), stamp_before);
        });
        pass(&mut store, &memory, 1, 1);
        assert_eq!(memory.reads.get(), reads);
        assert_eq!(store.failure, [None; 2]);
    }
    #[test]
    fn faction_system_pool_observer_selects_one_scalar_at_mod_resource_scale() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut held = vec![Held {
            id: 0,
            pointer: 0x1000,
            pool: 0x1038,
            resources: (0..5000)
                .map(|token| native::StoredAmount { token, raw: 11 })
                .collect(),
            ..Default::default()
        }];
        let storage = Storage::new(&held).unwrap();
        collector_test_support::with_allocations_denied(|| {
            storage.write(0x1038, true, 4999, -3, false);
            assert_eq!(storage.drain(&mut held), [false, true]);
            assert_eq!(storage.drain(&mut held), [false, false]);
        });
        assert_eq!(held[0].resources[4999].raw, -3);
        assert_eq!(held[0].resources[4998].raw, 11);
        assert_eq!(held[0].resources[0].raw, 11);
    }
    #[test]
    fn faction_pool_incomplete_identity_rosters_do_not_authorize_repeated_baseline_reads() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let memory = fixture(1);
        let mut store = Store::default();
        pass(&mut store, &memory, 1, 1);
        let reads = memory.reads.get();
        for _ in 0..4 {
            store.begin(false, Some(1));
            store.visit(&memory, 0, FIRST, 1);
            store.finish(false, Some(1), None);
            assert!(store.output(0).0.is_none());
            assert_eq!(memory.reads.get(), reads);
        }
        pass(&mut store, &memory, 1, 1);
        assert_eq!(memory.reads.get(), reads);
        pass(&mut store, &memory, 1, 1);
        assert_eq!(store.failure, [None; 2]);
        assert!(memory.reads.get() > reads);
    }
    #[test]
    fn faction_pool_owners_carry_and_scalar_updates_have_no_native_reads() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let memory = fixture(40);
        let mut store = Store::default();
        pass(&mut store, &memory, 40, 1);
        assert_eq!(store.failure, [None; 2]);
        let reads = memory.reads.get();
        let first = Arc::clone(store.rows[0].as_ref().unwrap());
        collector_test_support::with_allocations_denied(|| pass(&mut store, &memory, 40, 1));
        assert!(Arc::ptr_eq(&first, store.rows[0].as_ref().unwrap()));
        assert_eq!(memory.reads.get(), reads);
        let storage = store.storage.as_ref().unwrap();
        collector_test_support::with_allocations_denied(|| {
            storage.write(FIRST + 39 * STRIDE, false, 2, i64::MAX, false);
            storage.write(FIRST + 39 * STRIDE, false, 2, 1, true);
            storage.write(
                FIRST + 39 * STRIDE + engine_native::cfaction::EXTRACTED_RESOURCES,
                true,
                0,
                -71,
                false,
            );
            storage.write(8, true, usize::MAX, 1, false);
        });
        pass(&mut store, &memory, 40, 1);
        assert_eq!(memory.reads.get(), reads);
        assert_eq!(store.held[39].projection[2], i64::MIN);
        assert_eq!(store.held[39].resources[0].raw, -71);
        assert_eq!(store.held[38].resources[0].raw, 0);
        assert_eq!(store.failure, [None; 2]);
    }
    #[test]
    fn faction_pool_failures_carry_independently_and_new_epoch_reauthenticates() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut memory = fixture(1);
        memory.bytes[FIRST + engine_native::cfaction::EXTRACTED_RESOURCES] ^= 1;
        let mut store = Store::default();
        pass(&mut store, &memory, 1, 1);
        assert!(store.output(0).0.is_some());
        assert_eq!(store.output(1).1, Some("resource pool vtable mismatch"));
        let reads = memory.reads.get();
        pass(&mut store, &memory, 1, 1);
        assert_eq!(memory.reads.get(), reads);
        memory.bytes[FIRST + engine_native::cfaction::EXTRACTED_RESOURCES] ^= 1;
        pass(&mut store, &memory, 1, 2);
        assert_eq!(store.failure, [None; 2]);
        assert!(memory.reads.get() > reads);
        store.storage.as_ref().unwrap().write(
            FIRST + engine_native::cfaction::EXTRACTED_RESOURCES,
            true,
            3,
            1,
            false,
        );
        pass(&mut store, &memory, 1, 2);
        assert!(store.output(0).0.is_some());
        assert_eq!(
            store.output(1).1,
            Some("faction resource store index exceeds its certified pool")
        );
    }
}

#[cfg(test)]
mod root_failure_tests {
    use super::*;
    #[test]
    fn faction_system_pool_root_failure_carries_without_rebaselining_healthy_factions() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        struct Missing(std::cell::Cell<usize>);
        impl Memory for Missing {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                self.0.set(self.0.get() + 1);
                false
            }
        }
        let memory = Missing(std::cell::Cell::new(0));
        let mut store = Store::default();
        store.begin(false, Some(1));
        store.system(&memory, 0, 0);
        store.finish(true, Some(1), None);
        assert!(store.output(0).0.is_some());
        assert!(store.output(1).1.is_some());
        let reads = memory.0.get();
        collector_test_support::with_allocations_denied(|| {
            store.begin(false, Some(1));
            store.system(&memory, 0, 0);
            store.finish(true, Some(1), None);
        });
        assert_eq!(memory.0.get(), reads);
        assert!(store.output(0).0.is_some());
    }
}

pub fn rebuild_call(original: impl FnOnce() -> usize) -> usize {
    let flight = std::mem::ManuallyDrop::new(Flight::new());
    let result = original();
    rebuild();
    drop(std::mem::ManuallyDrop::into_inner(flight));
    result
}
