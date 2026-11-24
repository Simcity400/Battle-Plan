
use crate::{hook::Hook, hook_errors};
use collector_primitives::shared_lane::SharedLane;
use engine_allocation as allocation;
use engine_intelligence::{
    cstrategic_operative,
    propaganda_history::{self as native, Effort, Ring},
    strategic_operatives,
};
use engine_native_memory::{Memory, cpdx_array::Array, read, walk::Outcome};
use std::{
    ops::Range,
    ptr,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    },
};

static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static EPOCH: AtomicU64 = AtomicU64::new(1);
static ORDER: AtomicU64 = AtomicU64::new(0);
static FAILED: AtomicU64 = AtomicU64::new(0);
const UNPREPARED: usize = 0;
const ALLOCATION: usize = 1;
const INCOMPLETE: usize = 2;
const FAILURE_NAMES: [&str; 3] = [
    "propaganda append lane not prepared",
    "propaganda append allocation failed",
    "propaganda writer did not complete",
];
static FAILURES: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static CALLS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static ORIGINAL: [AtomicUsize; 2] = [const { AtomicUsize::new(0) }; 2];
static CURRENT: AtomicPtr<SharedLane<Append>> = AtomicPtr::new(ptr::null_mut());
static STORE: Mutex<Store> = Mutex::new(Store::new());

#[derive(Clone, Copy, Debug, Default)]
struct Append {
    order: u64,
    object: usize,
    reset: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct DailyAppend {
    length: u32,
    capacity: u32,
    effort: Effort,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub slot: u32,
    pub country: u32,
    pub sequence: u64,
    pub stability: i64,
    pub war_support: i64,
    pub derived_drift: u8,
}

#[derive(Default)]
struct Owner {
    object: usize,
    country: u32,
    slot: u32,
    capacity: u32,
    next: u64,
    history: Vec<Row>,
    drift: Effort,
    derived: bool,
    reacquire: bool,
    appends: u32,
}

impl Owner {
    fn bootstrap(
        &mut self,
        memory: &impl Memory,
        row: &strategic_operatives::Row,
        independent_drift: bool,
    ) -> Result<(), &'static str> {
        self.object = row.native_pointer;
        self.country = row.country;
        self.slot = row.slot;
        let ring = Ring::read(memory, self.object)?;
        self.capacity = ring.capacity;
        self.history.clear();
        allocation::reserve(&mut self.history, ring.len() as usize)
            .map_err(|_| "propaganda history storage allocation failed")?;
        for ordinal in 0..ring.len() {
            let effort = ring.effort(memory, ordinal)?;
            self.history.push(Row {
                slot: row.slot,
                country: row.country,
                sequence: u64::from(ordinal),
                stability: effort.stability,
                war_support: effort.war_support,
                derived_drift: 0,
            });
        }
        self.next = u64::from(ring.len());
        self.drift = if independent_drift {
            native::weekly_drift(memory, self.object)?
        } else {
            Effort::default()
        };
        self.derived = !independent_drift;
        Ok(())
    }

    fn append(&mut self, append: DailyAppend) -> Result<bool, &'static str> {
        if append.capacity != self.capacity
            || (append.length == 0 && self.capacity != 1)
            || append.length
                != (self.history.len() as u64 + 1).min(u64::from(self.capacity - 1)) as u32
        {
            return Err("propaganda append does not continue retained logical history");
        }
        let mut changed = !self.derived;
        self.derived = true;
        self.drift = Effort::default();
        let same = append.length as usize == self.history.len()
            && self.history.iter().all(|row| {
                row.stability == append.effort.stability
                    && row.war_support == append.effort.war_support
            });
        if !same {
            if append.length as usize == self.history.len() {
                self.history.remove(0);
            }
            if self.next == u64::MAX {
                return Err("propaganda history sequence exhausted");
            }
            allocation::reserve(&mut self.history, 1)
                .map_err(|_| "propaganda history storage allocation failed")?;
            self.history.push(Row {
                slot: self.slot,
                country: self.country,
                sequence: self.next,
                stability: append.effort.stability,
                war_support: append.effort.war_support,
                derived_drift: 0,
            });
            self.next += 1;
            changed = true;
        }
        Ok(changed)
    }
}

struct Store {
    #[expect(
        clippy::vec_box,
        reason = "Published lane pointers must remain stable when the retention vector grows."
    )]
    lanes: Vec<Box<SharedLane<Append>>>,
    owners: Vec<Owner>,
    staging: Vec<Owner>,
    seeds: Vec<strategic_operatives::Row>,
    objects: Option<Array>,
    lookup: Vec<(usize, usize)>,
    events: Vec<(u64, usize, u64, Append)>,
    epoch: u64,
    failure: u64,
    failures: [u64; 3],
    rejected: u64,
    valid: bool,
    snapshot: crate::retained_snapshot::RetainedSnapshot,
}

impl Store {
    const fn new() -> Self {
        Self {
            lanes: Vec::new(),
            owners: Vec::new(),
            staging: Vec::new(),
            seeds: Vec::new(),
            objects: None,
            lookup: Vec::new(),
            events: Vec::new(),
            epoch: 0,
            failure: 0,
            failures: [0; 3],
            rejected: 0,
            valid: false,
            snapshot: crate::retained_snapshot::RetainedSnapshot::new(),
        }
    }

    fn loss(&self, failures: &[u64; 3]) -> &'static str {
        FAILURE_NAMES
            .into_iter()
            .zip(failures.iter().zip(self.failures))
            .find_map(|(name, (now, seen))| (*now != seen).then_some(name))
            .unwrap_or("propaganda history requires daily or forced bootstrap after lost observation")
    }

    fn prepare(&mut self, entries: usize) -> Result<(), &'static str> {
        if self.lanes.last().is_some_and(|lane| lane.len() >= entries) {
            return Ok(());
        }
        let lane = Box::new(
            SharedLane::new(entries.max(1), 0)
                .map_err(|_| "propaganda append lane allocation failed")?,
        );
        allocation::reserve(&mut self.lanes, 1)
            .map_err(|_| "propaganda append lane retention allocation failed")?;
        self.lanes.push(lane);
        CURRENT.store(self.lanes.last_mut().unwrap().as_mut(), Ordering::SeqCst);
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Native census inputs, recovery gates and reusable output remain independently borrowed."
    )]
    fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        objects: Option<Array>,
        roster: &[strategic_operatives::Row],
        force_full: bool,
        recovery_due: bool,
        observing: bool,
        output: &mut Vec<Row>,
    ) -> Result<Option<usize>, &'static str> {
        let sequence = ORDER.load(Ordering::SeqCst);
        if ACTIVE.load(Ordering::SeqCst) != 0 {
            return Err("propaganda writer still running at capture");
        }
        let epoch = EPOCH.load(Ordering::SeqCst);
        let failure = FAILED.load(Ordering::SeqCst);
        let failures = FAILURES.each_ref().map(|count| count.load(Ordering::SeqCst));
        let rejected = self.lanes.iter().map(|lane| lane.rejected()).sum::<u64>();
        let lost = self.epoch != epoch || self.failure != failure || self.rejected != rejected;
        if lost {
            self.valid = false;
        }
        if !observing {
            self.valid = false;
        }
        let bootstrap = force_full || (!self.valid && recovery_due);
        let mut events = std::mem::take(&mut self.events);
        events.clear();
        let mut demand = roster.len();
        for (lane_index, lane) in self.lanes.iter().enumerate() {
            demand = demand.max(lane.required_entries());
            for (order, index) in lane.published(|event| event.order)? {
                allocation::reserve(&mut events, 1)
                    .map_err(|_| "propaganda append drain allocation failed")?;
                events.push((order, lane_index, index, lane.record(index).0));
            }
        }
        events.sort_unstable_by_key(|event| event.0);
        if self.valid && !bootstrap && self.objects == objects && events.is_empty() {
            if ACTIVE.load(Ordering::SeqCst) != 0
                || ORDER.load(Ordering::SeqCst) != sequence
                || EPOCH.load(Ordering::SeqCst) != epoch
                || FAILED.load(Ordering::SeqCst) != failure
            {
                self.valid = false;
                return Err("propaganda observation changed during quiet carry");
            }
            self.events = events;
            return Ok(None);
        }
        let mut changed = bootstrap || !self.valid;
        if !self.valid && !bootstrap {
            return Err(self.loss(&failures));
        }
        if bootstrap || self.objects != objects {
            self.seeds.clear();
            if let Some(objects) = objects {
                for slot in 0..objects.count {
                    let seed = if let Ok(index) = roster.binary_search_by_key(&slot, |row| row.slot)
                    {
                        roster[index]
                    } else {
                        let pointer = objects
                            .element(slot, 8)
                            .and_then(|at| read::<u64>(memory, at))
                            .and_then(|pointer| usize::try_from(pointer).ok())
                            .ok_or("propaganda bootstrap object slot unreadable")?;
                        if pointer == 0 {
                            continue;
                        }
                        let vtable = engine_sites::at(base, cstrategic_operative::VTABLE_RVA)
                            .ok_or("propaganda bootstrap object vtable mismatch")?;
                        if read::<u64>(memory, pointer) != Some(vtable as u64) {
                            return Err("propaganda bootstrap object vtable mismatch");
                        }
                        let country = pointer
                            .checked_add(cstrategic_operative::OWNER)
                            .and_then(|at| read::<u32>(memory, at))
                            .ok_or("propaganda bootstrap object owner unreadable")?;
                        strategic_operatives::Row {
                            native_pointer: pointer,
                            slot,
                            country,
                            ..strategic_operatives::Row::default()
                        }
                    };
                    allocation::reserve(&mut self.seeds, 1)
                        .map_err(|_| "propaganda bootstrap roster allocation failed")?;
                    self.seeds.push(seed);
                }
            }
            self.objects = objects;
        } else {
            for row in roster {
                match self.seeds.binary_search_by_key(&row.slot, |seed| seed.slot) {
                    Ok(index) => self.seeds[index] = *row,
                    Err(index) => {
                        allocation::reserve(&mut self.seeds, 1)
                            .map_err(|_| "propaganda new owner allocation failed")?;
                        self.seeds.insert(index, *row);
                    }
                }
            }
        }
        let membership = self.owners.len() != self.seeds.len()
            || self.owners.iter().zip(&self.seeds).any(|(old, row)| {
                old.object != row.native_pointer
                    || old.country != row.country
                    || old.slot != row.slot
            });
        if membership || bootstrap {
            self.staging.clear();
            allocation::reserve(&mut self.staging, self.seeds.len())
                .map_err(|_| "propaganda owner storage allocation failed")?;
            let mut old = self.owners.drain(..).peekable();
            for row in &self.seeds {
                while old.peek().is_some_and(|owner| owner.slot < row.slot) {
                    old.next();
                }
                let mut owner = if old.peek().is_some_and(|owner| owner.slot == row.slot) {
                    old.next().unwrap()
                } else {
                    Owner::default()
                };
                owner.reacquire = bootstrap
                    || owner.object != row.native_pointer
                    || owner.country != row.country
                    || owner.slot != row.slot;
                self.staging.push(owner);
            }
            drop(old);
            std::mem::swap(&mut self.owners, &mut self.staging);
            allocation::resize_default(&mut self.lookup, self.seeds.len())
                .map_err(|_| "propaganda owner lookup allocation failed")?;
            for (index, row) in self.seeds.iter().enumerate() {
                self.lookup[index] = (row.native_pointer, index);
            }
            self.lookup.sort_unstable();
            if self.lookup.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                return Err("propaganda manager repeats a native object");
            }
        } else {
            for owner in &mut self.owners {
                owner.reacquire = false;
                owner.appends = 0;
            }
        }
        let owner_index = |event: &Append| {
            self.lookup
                .binary_search_by_key(&event.object, |&(object, _)| object)
                .ok()
                .map(|index| self.lookup[index].1)
        };
        for (owner, row) in self.owners.iter_mut().zip(&self.seeds) {
            if owner.reacquire {
                owner.bootstrap(memory, row, true)?;
                changed = true;
            }
            owner.appends = 0;
        }
        if !bootstrap {
            for &(_, _, _, event) in &events {
                let Some(index) = owner_index(&event) else {
                    return Err("propaganda writer receiver outside retained manager");
                };
                if !self.owners[index].reacquire {
                    let owner = &mut self.owners[index];
                    if event.reset {
                        changed |= !owner.history.is_empty()
                            || owner.drift != Effort::default()
                            || !owner.derived;
                        owner.history.clear();
                        owner.drift = Effort::default();
                        owner.derived = true;
                        owner.appends = 0;
                    } else {
                        owner.appends = owner
                            .appends
                            .checked_add(1)
                            .ok_or("propaganda append count overflow")?;
                    }
                }
            }
            for (owner, seed) in self.owners.iter_mut().zip(&self.seeds) {
                if owner.appends == 1 {
                    let ring = Ring::read(memory, owner.object)?;
                    let effort = if ring.is_empty() {
                        Effort::default()
                    } else {
                        ring.newest(memory)?
                    };
                    changed |= owner.append(DailyAppend {
                        capacity: ring.capacity,
                        length: ring.len(),
                        effort,
                    })?;
                } else if owner.appends > 1 {
                    owner.bootstrap(memory, seed, false)?;
                    changed = true;
                }
            }
        }
        if ACTIVE.load(Ordering::SeqCst) != 0
            || ORDER.load(Ordering::SeqCst) != sequence
            || EPOCH.load(Ordering::SeqCst) != epoch
            || FAILED.load(Ordering::SeqCst) != failure
        {
            self.valid = false;
            return Err("propaganda observation changed during capture");
        }
        for &(_, lane_index, index, _) in &events {
            self.lanes[lane_index].release(index);
        }
        for lane in &self.lanes {
            lane.advance(lane.rejected());
        }
        self.prepare(demand.max(self.seeds.len()))?;
        self.epoch = epoch;
        self.failure = failure;
        self.failures = failures;
        self.rejected = rejected;
        self.valid = true;
        events.clear();
        self.events = events;
        if !changed {
            return Ok(None);
        }
        let count = self
            .owners
            .iter()
            .try_fold(0_usize, |count, owner| {
                count.checked_add(owner.history.len() + 1)
            })
            .ok_or("propaganda output count overflow")?;
        allocation::resize_default(output, count)
            .map_err(|_| "propaganda output allocation failed")?;
        let mut offset = 0;
        for owner in &self.owners {
            output[offset..offset + owner.history.len()].copy_from_slice(&owner.history);
            offset += owner.history.len();
            output[offset] = Row {
                slot: owner.slot,
                country: owner.country,
                sequence: u64::MAX,
                stability: owner.drift.stability,
                war_support: owner.drift.war_support,
                derived_drift: u8::from(owner.derived),
            };
            offset += 1;
        }
        Ok(Some(count))
    }

}

impl Drop for Store {
    fn drop(&mut self) {
        let current = CURRENT.load(Ordering::SeqCst);
        if self
            .lanes
            .iter()
            .any(|lane| std::ptr::eq(lane.as_ref(), current))
        {
            let _ = CURRENT.compare_exchange(
                current,
                ptr::null_mut(),
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "The worker shares native census inputs and transfers acquired rows to the existing lazy snapshot."
)]
pub(crate) fn capture(
    memory: &impl Memory,
    base: usize,
    objects: Option<Array>,
    roster: &[strategic_operatives::Row],
    force_full: bool,
    recovery_due: bool,
    output: &mut Vec<Row>,
    shared: &mut Option<std::sync::Arc<crate::retained_snapshot::Snapshot>>,
) -> Outcome {
    let mut store = STORE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let outcome = match store.capture(
        memory,
        base,
        objects,
        roster,
        force_full,
        recovery_due,
        ENABLED.load(Ordering::Acquire),
        output,
    ) {
        Ok(Some(len)) => Outcome::Rows { len },
        Ok(None) => Outcome::Unavailable {
            reason: replay_data::cadence::SKIPPED_REASON,
        },
        Err(reason) => {
            store.valid = false;
            Outcome::Unavailable { reason }
        }
    };
    let outcome = store.snapshot.publish(
        replay_data::rows::TableId::PropagandaEfforts,
        outcome,
        || crate::retained_snapshot::AcquiredRows::Propaganda(std::mem::take(output)),
        shared,
    );
    if matches!(outcome, Outcome::Unavailable { reason } if reason != replay_data::cadence::SKIPPED_REASON)
    {
        store.valid = false;
    }
    outcome
}

pub fn boundary(invalidate: bool) {
    if invalidate {
        EPOCH.fetch_add(1, Ordering::SeqCst);
    }
}
pub fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
}
pub fn status() -> impl serde::Serialize {
    crate::status::status!({
        "lost": FAILED.load(Ordering::Relaxed),
        "reasons": crate::status::map(
            FAILURE_NAMES
                .into_iter()
                .zip(FAILURES.iter().map(|count| count.load(Ordering::Relaxed)))
                .filter(|(_, count)| *count != 0),
        ),
    })
}


struct Guard {
    complete: bool,
}
impl Guard {
    fn enter() -> Self {
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        Self { complete: false }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if !self.complete {
            fail(INCOMPLETE);
            return;
        }
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

fn fail(reason: usize) {
    FAILURES[reason].fetch_add(1, Ordering::SeqCst);
    FAILED.fetch_add(1, Ordering::SeqCst);
}

fn publish(object: usize, reset: bool) {
    let event = Append {
        order: ORDER.fetch_add(1, Ordering::SeqCst),
        object,
        reset,
    };
    let Some(lane) = (unsafe { CURRENT.load(Ordering::SeqCst).as_ref() }) else {
        return fail(UNPREPARED);
    };
    match lane.reserve() {
        Some(slot) => slot.publish(event),
        None => fail(ALLOCATION),
    }
}

macro_rules! callback {
    ($name:ident, $index:literal, $reset:literal) => {
        unsafe extern "system-unwind" fn $name(object: usize) -> usize {
            let mut guard = Guard::enter();
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::PropagandaChanges);
            CALLS[$index].fetch_add(1, Ordering::Relaxed);
            let original: unsafe extern "system-unwind" fn(usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$index].load(Ordering::Acquire)) };
            let result = timing.original(|| unsafe { original(object) });
            publish(object, $reset);
            guard.complete = true;
            result
        }
    };
}
callback!(daily, 0, false);
callback!(reset, 1, true);

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// # Safety
    /// The host authenticated this executable and owns hook restoration and
    /// quiescence before unloading this module.
    pub unsafe fn new(
        memory: &impl Memory,
        base: usize,
        capture: bool,
    ) -> Result<Self, String> {
        if !capture {
            return Ok(Self { hooks: Vec::new() });
        }
        let state = engine_sites::resolve(base, engine_native::cgame_state::SINGLETON_RVA)
            .ok()
            .and_then(|at| read::<u64>(memory, at))
            .and_then(|state| usize::try_from(state).ok())
            .ok_or("propaganda bootstrap game state unreadable")?;
        let manager = if state == 0 {
            0
        } else {
            state
                .checked_add(engine_native::cgame_state::STRATEGIC_OPERATIVE_MANAGER)
                .and_then(|at| read::<u64>(memory, at))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .ok_or("propaganda bootstrap manager unreadable")?
        };
        let count = if manager == 0 {
            0
        } else {
            manager
                .checked_add(engine_intelligence::cstrategic_operative_manager::OPERATIVES_DATA)
                .and_then(|at| Array::read(memory, at, 8))
                .ok_or("propaganda bootstrap manager header invalid")?
                .count as usize
        };
        STORE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .prepare(count)?;
        let mut hooks = Vec::new();
        for (index, (rva, callback)) in [
            (native::DAILY_UPDATE_RVA, daily as *const () as usize),
            (native::RESET_RVA, reset as *const () as usize),
        ]
        .into_iter()
        .enumerate()
        {
            let name = ["daily append", "reset"][index];
            let (address, original) = hook_errors::locate(base, rva, "propaganda", name, index)?;
            let hook = unsafe { Hook::new(address, callback, original) }.map_err(|cause| {
                hook_errors::site("propaganda", name, index, "construction failed", &cause)
            })?;
            ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
            hooks.push(hook);
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if self.hooks.is_empty() {
            return Ok(true);
        }
        if !enabled {
            ENABLED.store(false, Ordering::Release);
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled).map_err(|cause| {
            hook_errors::publication(
                "propaganda",
                &self.hooks,
                ["daily append", "reset"].into_iter(),
                &cause,
            )
        })? {
            return Ok(false);
        }
        ENABLED.store(enabled, Ordering::Release);
        boundary(true);
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        CURRENT.store(ptr::null_mut(), Ordering::SeqCst);
        *STORE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Store::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn propaganda_owners_carry_without_native_reads_and_only_one_observed_append_changes() {
        use engine_native::ccountry::synthetic::World;
        const COUNTRIES: usize = 5;
        let mut world = World::new(0, 0, 0x200);
        let pointers = world.alloc((COUNTRIES + 1) * 8);
        let roots = world.alloc((COUNTRIES + 1) * cstrategic_operative::SIZE);
        let ring_data = world.alloc(2 * native::EFFORT_STRIDE);
        let mut roster = Vec::new();
        for slot in 0..=COUNTRIES {
            let object = roots + slot * cstrategic_operative::SIZE;
            world.u64(pointers + slot * 8, object as u64);
            world.u64(object, cstrategic_operative::VTABLE_RVA.0 as u64);
            world.u32(object + cstrategic_operative::OWNER, slot as u32);
            world.u64(object + native::HISTORY, ring_data as u64);
            world.u32(object + native::HISTORY + 8, 2);
            if slot != 0 {
                roster.push(strategic_operatives::Row {
                    native_pointer: object,
                    slot: slot as u32,
                    country: slot as u32,
                    ..strategic_operatives::Row::default()
                });
            }
        }
        let objects = Some(Array {
            data: pointers,
            count: COUNTRIES as u32 + 1,
        });
        let mut store = Store::new();
        let mut output = Vec::new();
        assert_eq!(
            store
                .capture(
                    &world.memory(),
                    0,
                    objects,
                    &roster,
                    true,
                    false,
                    true,
                    &mut output
                )
                .unwrap(),
            Some(COUNTRIES + 1)
        );
        assert_eq!(output[0].slot, 0);
        struct NoRead;
        impl Memory for NoRead {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("carried propaganda attempted a native read")
            }
        }
        assert_eq!(
            store
                .capture(
                    &NoRead,
                    0,
                    objects,
                    &roster,
                    false,
                    false,
                    true,
                    &mut output
                )
                .unwrap(),
            None
        );
        world.u32(roots + native::HISTORY + 16, 1);
        world.u64(ring_data, (-7_i64) as u64);
        world.u64(ring_data + 8, 13);
        store
            .lanes
            .last()
            .unwrap()
            .reserve()
            .unwrap()
            .publish(Append {
                object: roots,
                ..Append::default()
            });
        struct OnlyChanged<'a> {
            memory: engine_native_memory::memory::SliceMemory<'a>,
            header: usize,
            effort: usize,
        }
        impl Memory for OnlyChanged<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                assert!(
                    (address == self.header && output.len() == 20)
                        || (address == self.effort && output.len() == 16),
                    "propaganda read an unmarked owner, unsaved channel or redundant weekly cache"
                );
                self.memory.copy(address, output)
            }
        }
        let changed_memory = OnlyChanged {
            memory: world.memory(),
            header: roots + native::HISTORY,
            effort: ring_data,
        };
        assert_eq!(
            store
                .capture(
                    &changed_memory,
                    0,
                    objects,
                    &roster,
                    false,
                    false,
                    true,
                    &mut output
                )
                .unwrap(),
            Some(COUNTRIES + 2)
        );
        assert_eq!(
            output.iter().filter(|row| row.sequence != u64::MAX).count(),
            1
        );
        assert_eq!(output[0].stability, -7);
        assert_eq!(output[0].war_support, 13);
        assert_eq!(
            store
                .capture(
                    &NoRead,
                    0,
                    objects,
                    &roster,
                    false,
                    false,
                    true,
                    &mut output
                )
                .unwrap(),
            None
        );
        world.u32(roots + native::HISTORY + 16, 0);
        for expected in [Some(COUNTRIES + 1), None] {
            let lane = store.lanes.last().unwrap();
            let burst = if expected.is_some() {
                4 * lane.len() as u64 + 3
            } else {
                1
            };
            for order in 1..=burst {
                lane.reserve().unwrap().publish(Append {
                    order,
                    object: roots,
                    reset: true,
                });
            }
            assert_eq!(
                lane.published(|append| append.order)
                    .unwrap()
                    .into_iter()
                    .map(|(order, _)| order)
                    .collect::<Vec<_>>(),
                (1..=burst).collect::<Vec<_>>()
            );
            assert_eq!(lane.rejected(), 0);
            assert_eq!(
                store
                    .capture(
                        &NoRead,
                        0,
                        objects,
                        &roster,
                        false,
                        false,
                        true,
                        &mut output
                    )
                    .unwrap(),
                expected
            );
        }
        let lane = store.lanes.last().unwrap();
        assert!(ptr::eq(CURRENT.load(Ordering::SeqCst), &raw const **lane));
        for order in 0..lane.len() as u64 {
            lane.reserve().unwrap().publish(Append {
                order,
                object: roots,
                reset: true,
            });
        }
        let refused = FAILURES[ALLOCATION].load(Ordering::SeqCst);
        crate::test_support::with_allocations_denied(|| publish(roots, true));
        assert_eq!(
            serde_json::to_value(status()).unwrap()["reasons"]["propaganda append allocation failed"],
            refused + 1
        );
        assert_eq!(
            store.capture(&NoRead, 0, objects, &roster, false, false, true, &mut output),
            Err("propaganda append allocation failed")
        );
        CURRENT.store(ptr::null_mut(), Ordering::SeqCst);
    }

    fn append(stability: i64, war_support: i64) -> DailyAppend {
        DailyAppend {
            length: 2,
            capacity: 3,
            effort: Effort {
                stability,
                war_support,
            },
        }
    }

    #[test]
    fn propaganda_daily_append_keeps_earlier_keys_and_derives_weekly_cache() {
        let mut owner = Owner {
            capacity: 3,
            country: 7,
            next: 2,
            history: vec![
                Row {
                    country: 7,
                    sequence: 0,
                    stability: 11,
                    war_support: -13,
                    ..Row::default()
                },
                Row {
                    country: 7,
                    sequence: 1,
                    stability: 17,
                    war_support: 19,
                    ..Row::default()
                },
            ],
            drift: Effort {
                stability: 999,
                war_support: -999,
            },
            ..Owner::default()
        };
        assert!(owner.append(append(23, 29)).unwrap());
        assert!(owner.derived);
        assert_eq!(owner.drift, Effort::default());
        assert_eq!(
            owner
                .history
                .iter()
                .map(|row| row.sequence)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        let sum = owner.history.iter().fold(Effort::default(), |sum, row| {
            sum.wrapping_add(Effort {
                stability: row.stability,
                war_support: row.war_support,
            })
        });
        assert_eq!(
            sum,
            Effort {
                stability: 40,
                war_support: 48
            }
        );
    }

    #[test]
    fn propaganda_repeated_full_history_is_change_only_and_allocates_nothing() {
        let mut owner = Owner {
            capacity: 3,
            country: 7,
            next: 2,
            derived: true,
            history: vec![
                Row {
                    country: 7,
                    sequence: 0,
                    ..Row::default()
                },
                Row {
                    country: 7,
                    sequence: 1,
                    ..Row::default()
                },
            ],
            ..Owner::default()
        };
        let pointer = owner.history.as_ptr();
        assert!(!owner.append(append(0, 0)).unwrap());
        assert_eq!(owner.next, 2);
        assert_eq!(owner.history.as_ptr(), pointer);
        let mut missed = append(1, 2);
        missed.length = 1;
        assert!(owner.append(missed).is_err());
    }
}
