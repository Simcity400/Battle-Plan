
use crate::{hook::Hook, hook_errors};
use collector_primitives::shared_lane::SharedLane;
use engine_allocation as allocation;
use engine_intelligence::{ccountry_intel_network as network, network_auxiliary as native};
use engine_native_memory::{Memory, walk::Outcome};
use replay_data::rows::IntelNetworkAuxiliaryRow;
use std::{
    cell::Cell,
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
static ORDER: AtomicU64 = AtomicU64::new(1);
static FAILED: AtomicU64 = AtomicU64::new(0);
static CURRENT: AtomicPtr<SharedLane<Event>> = AtomicPtr::new(ptr::null_mut());
static ORIGINAL: [AtomicUsize; 7] = [const { AtomicUsize::new(0) }; 7];
static STORE: Mutex<Store> = Mutex::new(Store::new());
thread_local! {static DEPTH:Cell<usize>=const{Cell::new(0)};}

#[cfg(test)]
fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Clone, Copy, Default)]
struct Event {
    order: u64,
    object: usize,
    lifetime: u8,
}
#[derive(Default)]
struct Owner {
    states: Vec<native::State>,
    seed: network::Summary,
    entries: Vec<(native::Entry, u64)>,
    scratch: Vec<native::Entry>,
    next: Vec<(native::Entry, u64)>,
    dirty: bool,
    fresh: bool,
    alive: bool,
}
impl Owner {
    fn install(&mut self, next_id: &mut u64) -> Result<bool, &'static str> {
        self.next.clear();
        allocation::reserve(&mut self.next, self.scratch.len())
            .map_err(|_| "network auxiliary retained occurrence allocation failed")?;
        for &entry in &self.scratch {
            let id = if !self.fresh {
                self.entries
                    .binary_search_by_key(&entry.key(), |(row, _)| row.key())
                    .ok()
                    .map(|i| self.entries[i].1)
            } else {
                None
            };
            let id = match id {
                Some(id) => id,
                None => {
                    let id = *next_id;
                    *next_id = next_id
                        .checked_add(1)
                        .ok_or("network auxiliary occurrence identity exhausted")?;
                    id
                }
            };
            self.next.push((entry, id));
        }
        let changed = self.next != self.entries;
        std::mem::swap(&mut self.next, &mut self.entries);
        self.dirty = false;
        self.fresh = false;
        Ok(changed)
    }
}
struct Store {
    pairs: Vec<(u32, u32, usize)>,
    seed_order: Vec<usize>,
    owners: Vec<Owner>,
    #[expect(
        clippy::vec_box,
        reason = "Published lane pointers must remain stable when the retention vector grows."
    )]
    lanes: Vec<Box<SharedLane<Event>>>,
    epoch: u64,
    failure: u64,
    published: u64,
    membership_order: u64,
    membership: bool,
    valid: bool,
    next_id: u64,
    reason: Option<&'static str>,
    snapshot: crate::retained_snapshot::RetainedSnapshot,
}
impl Store {
    const fn new() -> Self {
        Self {
            pairs: Vec::new(),
            seed_order: Vec::new(),
            owners: Vec::new(),
            lanes: Vec::new(),
            epoch: 0,
            failure: 0,
            published: 0,
            membership_order: 0,
            membership: false,
            valid: false,
            next_id: 1,
            reason: None,
            snapshot: crate::retained_snapshot::RetainedSnapshot::new(),
        }
    }
    fn prepare(&mut self, demand: usize) -> Result<(), &'static str> {
        if self.lanes.last().is_some_and(|lane| lane.len() >= demand) {
            return Ok(());
        }
        let lane = allocation::boxed(
            SharedLane::new(demand, 0)
                .map_err(|_| "network auxiliary event lane allocation failed")?,
        )
        .map_err(|_| "network auxiliary event lane allocation failed")?;
        allocation::reserve(&mut self.lanes, 1)
            .map_err(|_| "network auxiliary event lane registry allocation failed")?;
        self.lanes.push(lane);
        CURRENT.store(
            (&raw const **self.lanes.last().unwrap()).cast_mut(),
            Ordering::SeqCst,
        );
        Ok(())
    }
    fn membership(
        &mut self,
        seeds: &[network::Summary],
        states: &[network::Row],
        lifetime_changed: bool,
    ) -> Result<(), &'static str> {
        let was_valid = self.valid;
        self.membership = false;
        self.valid = false;
        let epoch = EPOCH.load(Ordering::SeqCst);
        let reset = lifetime_changed || self.epoch != epoch || !was_valid;
        self.seed_order.clear();
        allocation::reserve(&mut self.seed_order, seeds.len())
            .map_err(|_| "network auxiliary seed index allocation failed")?;
        self.seed_order.extend(0..seeds.len());
        self.seed_order
            .sort_unstable_by_key(|&index| seeds[index].native_pointer);
        if self
            .seed_order
            .windows(2)
            .any(|pair| seeds[pair[0]].native_pointer == seeds[pair[1]].native_pointer)
        {
            return Err("network auxiliary duplicate authenticated holder");
        }
        self.owners.retain(|owner| {
            self.seed_order
                .binary_search_by_key(&owner.seed.native_pointer, |&index| {
                    seeds[index].native_pointer
                })
                .ok()
                .is_some_and(|index| {
                    let seed = &seeds[self.seed_order[index]];
                    seed.owner_country == owner.seed.owner_country
                        && seed.target_country == owner.seed.target_country
                })
        });
        for &index in &self.seed_order {
            let seed = seeds[index];
            match self
                .owners
                .binary_search_by_key(&seed.native_pointer, |owner| owner.seed.native_pointer)
            {
                Ok(index) => {
                    let owner = &mut self.owners[index];
                    owner.seed = seed;
                    owner.fresh |= reset;
                    owner.dirty |= reset;
                    owner.alive = true;
                }
                Err(index) => {
                    allocation::reserve(&mut self.owners, 1)
                        .map_err(|_| "network auxiliary membership allocation failed")?;
                    self.owners.insert(
                        index,
                        Owner {
                            seed,
                            dirty: true,
                            fresh: true,
                            alive: true,
                            ..Owner::default()
                        },
                    );
                }
            }
        }
        self.pairs.clear();
        allocation::reserve(&mut self.pairs, self.owners.len())
            .map_err(|_| "network auxiliary acquired-state index allocation failed")?;
        for (index, owner) in self.owners.iter_mut().enumerate() {
            owner.states.clear();
            self.pairs
                .push((owner.seed.owner_country, owner.seed.target_country, index));
        }
        self.pairs.sort_unstable();
        for row in states {
            let index = self
                .pairs
                .binary_search_by_key(&(row.owner_country, row.target_country), |entry| {
                    (entry.0, entry.1)
                })
                .map_err(|_| "network auxiliary state has no authenticated network")?;
            let owner = &mut self.owners[self.pairs[index].2];
            allocation::reserve(&mut owner.states, 1)
                .map_err(|_| "network auxiliary acquired-state storage allocation failed")?;
            owner.states.push(native::State {
                ordinal: row.native_ordinal,
                state: row.state_id,
                selector: row.subnetwork_selector,
            });
        }
        for owner in &mut self.owners {
            owner.states.sort_unstable_by_key(|state| state.selector);
        }

        self.membership_order = ORDER.load(Ordering::SeqCst);
        self.epoch = epoch;
        self.prepare(seeds.len().max(1).saturating_mul(native::SITES.len()))?;
        self.membership = true;
        Ok(())
    }
}
pub fn membership(
    seeds: &[network::Summary],
    states: &[network::Row],
    lifetime_changed: bool,
) -> Result<(), &'static str> {
    #[cfg(test)]
    let _lock = test_lock();
    let mut store = STORE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = store.membership(seeds, states, lifetime_changed);
    if let Err(reason) = result {
        store.reason.get_or_insert(reason);
    }
    result
}
pub fn boundary(invalidate: bool) {
    #[cfg(test)]
    let _lock = test_lock();
    if invalidate {
        EPOCH.fetch_add(1, Ordering::SeqCst);
    }
}
pub fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
}
pub fn refused() {
    #[cfg(test)]
    let _lock = test_lock();
    FAILED.fetch_add(1, Ordering::SeqCst);
}

pub(crate) fn capture(
    memory: &impl Memory,
    base: usize,
    output: &mut Vec<IntelNetworkAuxiliaryRow>,
    shared: &mut Option<std::sync::Arc<crate::retained_snapshot::Snapshot>>,
) -> Outcome {
    #[cfg(test)]
    let _lock = test_lock();
    let mut store = STORE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let outcome = capture_store(&mut store, memory, base, output);
    let outcome = store.snapshot.publish(
        replay_data::rows::TableId::IntelNetworkAuxiliaries,
        outcome,
        || crate::retained_snapshot::AcquiredRows::NetworkAuxiliaries(std::mem::take(output)),
        shared,
    );
    if matches!(outcome, Outcome::Unavailable { reason } if reason != replay_data::cadence::SKIPPED_REASON)
    {
        store.valid = false;
        store.membership = false;
    }
    outcome
}
fn capture_store(
    store: &mut Store,
    memory: &impl Memory,
    base: usize,
    output: &mut Vec<IntelNetworkAuxiliaryRow>,
) -> Outcome {
    let result = (|| {
        if !ENABLED.load(Ordering::Acquire) {
            return Err("network auxiliary observers disabled");
        }
        if ACTIVE.load(Ordering::SeqCst) != 0 {
            return Err("network auxiliary native writer in progress");
        }
        let epoch = EPOCH.load(Ordering::SeqCst);
        let failure = FAILED.load(Ordering::SeqCst);
        let order = ORDER.load(Ordering::SeqCst);
        if store.epoch != epoch || store.failure != failure {
            store.valid = false;
            if !store.membership {
                return Err("network auxiliary lost writer coverage; awaiting existing census");
            }
            for owner in &mut store.owners {
                owner.fresh = true;
                owner.dirty = true;
            }
            store.epoch = epoch;
            store.failure = failure;
        }
        if !store.valid && !store.membership {
            return Err("network auxiliary membership not acquired");
        }
        if store.valid && !store.membership && store.published == order {
            if ACTIVE.load(Ordering::SeqCst) != 0
                || EPOCH.load(Ordering::SeqCst) != epoch
                || FAILED.load(Ordering::SeqCst) != failure
                || ORDER.load(Ordering::SeqCst) != order
            {
                return Err("network auxiliary writer changed during quiet carry");
            }
            return Ok(false);
        }
        let mut changed = store.membership;
        let demand = store
            .lanes
            .iter()
            .map(|lane| lane.required_entries())
            .max()
            .unwrap_or(1);
        store.prepare(demand.max(1))?;
        for lane_index in 0..store.lanes.len() {
            for (_, index) in store.lanes[lane_index].published(|event| event.order)? {
                let (event, _) = store.lanes[lane_index].record(index);
                if let Ok(owner_index) = store
                    .owners
                    .binary_search_by_key(&event.object, |owner| owner.seed.native_pointer)
                {
                    let before_membership = event.order < store.membership_order;
                    let has_membership = store.membership;
                    let owner = &mut store.owners[owner_index];
                    if event.lifetime != 0 {
                        owner.fresh = true;
                        owner.dirty = true;
                        if !before_membership {
                            if event.lifetime != 2 {
                                return Err(
                                    "network auxiliary new lifetime awaiting existing census",
                                );
                            }
                            owner.alive = false;
                            changed = true;
                        }
                    } else if !before_membership && !has_membership {
                        return Err(
                            "network auxiliary topology changed; awaiting existing acquired-state census",
                        );
                    } else if !(owner.fresh && before_membership) {
                        owner.dirty = true;
                    }
                } else if !store.membership && event.lifetime != 2 {
                    return Err("network auxiliary new holder awaits authenticated census");
                }
                store.lanes[lane_index].release(index);
            }
            let rejected = store.lanes[lane_index].rejected();
            store.lanes[lane_index].advance(rejected);
        }
        let mut next_id = store.next_id;
        for owner in &mut store.owners {
            if owner.dirty && owner.alive {
                native::acquire(memory, base, &owner.seed, &owner.states, &mut owner.scratch)?;
                changed |= owner.install(&mut next_id)?;
            }
        }
        store.next_id = next_id;
        if ACTIVE.load(Ordering::SeqCst) != 0
            || EPOCH.load(Ordering::SeqCst) != epoch
            || FAILED.load(Ordering::SeqCst) != failure
            || ORDER.load(Ordering::SeqCst) != order
        {
            return Err("network auxiliary writer changed during acquisition");
        }
        store.valid = true;
        store.membership = false;
        store.published = order;
        if !changed {
            return Ok(false);
        }
        output.clear();
        let count = store
            .owners
            .iter()
            .filter(|owner| owner.alive)
            .map(|owner| owner.entries.len())
            .sum();
        allocation::reserve(output, count)
            .map_err(|_| "network auxiliary replay rows allocation failed")?;
        for owner in &store.owners {
            if owner.alive {
                for &(entry, id) in &owner.entries {
                    output.push(IntelNetworkAuxiliaryRow {
                        owner: owner.seed.owner_country,
                        target: owner.seed.target_country,
                        id,
                        kind: entry.kind,
                        subnetwork: entry.subnetwork,
                        ordinal: entry.ordinal,
                        country: entry.country,
                        count_a: entry.count_a,
                        count_b: entry.count_b,
                        count_c: entry.count_c,
                        first: entry.first,
                        second: entry.second,
                        third: entry.third,
                    });
                }
            }
        }
        Ok(true)
    })();
    match result {
        Ok(true) => Outcome::Rows { len: output.len() },
        Ok(false) => Outcome::Unavailable {
            reason: replay_data::cadence::SKIPPED_REASON,
        },
        Err(reason) => {
            store.valid = false;
            store.membership = false;
            if store.reason.is_none() {
                store.reason = Some(reason);
            }
            Outcome::Unavailable { reason }
        }
    }
}

struct Flight {
    complete: bool,
    outer: bool,
}
impl Flight {
    fn new() -> Self {
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        Self {
            complete: false,
            outer: DEPTH.with(|depth| {
                let old = depth.get();
                depth.set(old + 1);
                old == 0
            }),
        }
    }
}
impl Drop for Flight {
    fn drop(&mut self) {
        DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
        if self.complete {
            ACTIVE.fetch_sub(1, Ordering::SeqCst);
        } else {
            refused();
        }
    }
}
fn publish(guard: &Flight, object: usize, lifetime: u8) {
    if !guard.outer {
        return;
    }
    let order = ORDER.fetch_add(1, Ordering::SeqCst);
    let lane = CURRENT.load(Ordering::SeqCst);
    if let Some(slot) = unsafe { lane.as_ref() }.and_then(SharedLane::reserve) {
        slot.publish(Event {
            order,
            object,
            lifetime,
        });
    } else {
        refused();
    }
}
macro_rules! callback{($name:ident,$i:literal,$life:literal,$offset:expr,($($arg:ident),*),$ret:ty)=>{unsafe extern "system-unwind" fn $name(object:usize,$($arg:usize),*)->$ret{let mut guard=Flight::new();let timing=crate::hook_timing::enter(crate::hook_timing::Family::NetworkAuxiliary);let original:unsafe extern "system-unwind" fn(usize,$(callback!(@ty $arg)),*)->$ret=unsafe{std::mem::transmute(ORIGINAL[$i].load(Ordering::Acquire))};let result=timing.original(||unsafe{original(object,$($arg),*)});publish(&guard,object.saturating_sub($offset),$life);guard.complete=true;result}};(@ty $arg:ident)=>{usize};}
callback!(rebuild, 0, 0, 0, (source, operatives, gain), ());
callback!(load, 1, 1, 0, (parser, token), ());
callback!(construct, 2, 1, 0, (target), usize);
callback!(construct_empty, 3, 1, 0, (), usize);
callback!(destroy, 4, 2, 0, (flags), usize);
callback!(local_load, 5, 1, network::LOCAL_NETWORK, (parser), ());
callback!(prepare, 6, 0, 0, (source, gain), ());

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// # Safety
    /// Host authenticates the executable and restores/quiesces before unload.
    pub unsafe fn new(
        _memory: &impl Memory,
        base: usize,
        capture: bool,
    ) -> Result<Self, String> {
        if !capture {
            return Ok(Self { hooks: Vec::new() });
        }
        STORE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .prepare(native::SITES.len())?;
        let callbacks = [
            rebuild as *const () as usize,
            load as *const () as usize,
            construct as *const () as usize,
            construct_empty as *const () as usize,
            destroy as *const () as usize,
            local_load as *const () as usize,
            prepare as *const () as usize,
        ];
        let mut hooks = Vec::new();
        for (index, site) in native::SITES.iter().enumerate() {
            let (address, original) =
                hook_errors::locate(base, site.rva, "network auxiliary", site.name, index)?;
            let hook =
                unsafe { Hook::new(address, callbacks[index], original) }.map_err(|cause| {
                    hook_errors::site(
                        "network auxiliary",
                        site.name,
                        index,
                        "construction failed",
                        &cause,
                    )
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
                "network auxiliary",
                &self.hooks,
                native::SITES.iter().map(|site| site.name),
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
    fn network_auxiliary_failed_census_is_one_attempt_then_waits_without_reads_or_allocations() {
        let _lock = test_lock();
        struct Unreadable(std::cell::Cell<usize>);
        impl Memory for Unreadable {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                self.0.set(self.0.get() + 1);
                false
            }
        }
        let memory = Unreadable(std::cell::Cell::new(0));
        let mut store = Store::new();
        let seed = network::Summary {
            native_pointer: 4096,
            owner_country: 1,
            target_country: 2,
            ..Default::default()
        };
        ENABLED.store(true, Ordering::SeqCst);
        store.membership(&[seed], &[], false).unwrap();
        let mut rows = Vec::new();
        assert!(matches!(
            capture_store(&mut store, &memory, 0, &mut rows),
            Outcome::Unavailable { .. }
        ));
        let reads = memory.0.get();
        assert!(reads > 0);
        assert!(!store.membership && !store.valid);
        for _ in 0..4 {
            crate::test_support::with_allocations_denied(|| {
                assert!(matches!(
                    capture_store(&mut store, &memory, 0, &mut rows),
                    Outcome::Unavailable { .. }
                ));
            });
            assert_eq!(memory.0.get(), reads);
        }
        store.membership(&[seed], &[], false).unwrap();
        assert!(store.membership(&[seed, seed], &[], false).is_err());
        assert!(!store.membership && !store.valid);
        assert!(matches!(
            capture_store(&mut store, &memory, 0, &mut rows),
            Outcome::Unavailable { .. }
        ));
        assert_eq!(memory.0.get(), reads);
        store.membership(&[seed], &[], false).unwrap();
        assert!(matches!(
            capture_store(&mut store, &memory, 0, &mut rows),
            Outcome::Unavailable { .. }
        ));
        assert!(memory.0.get() > reads);
        ENABLED.store(false, Ordering::SeqCst);
    }
    #[test]
    fn network_auxiliary_occurrences_keep_duplicate_zero_positions_and_revoke_lifetimes() {
        let mut owner = Owner::default();
        let mut next = 1;
        owner.scratch = vec![
            native::Entry {
                kind: native::kind::RECORD,
                ordinal: 0,
                ..native::Entry::default()
            },
            native::Entry {
                kind: native::kind::RECORD,
                ordinal: 1,
                ..native::Entry::default()
            },
        ];
        assert!(owner.install(&mut next).unwrap());
        assert_eq!(next, 3);
        let ids: Vec<_> = owner.entries.iter().map(|row| row.1).collect();
        assert!(!owner.install(&mut next).unwrap());
        assert_eq!(next, 3);
        owner.scratch[1].first = 77;
        assert!(owner.install(&mut next).unwrap());
        assert_eq!(
            owner.entries.iter().map(|row| row.1).collect::<Vec<_>>(),
            ids
        );
        owner.fresh = true;
        assert!(owner.install(&mut next).unwrap());
        assert!(owner.entries.iter().all(|row| !ids.contains(&row.1)));
    }
    #[test]
    fn network_auxiliary_quiet_owners_reads_and_allocates_nothing_and_loss_waits_for_census() {
        let _lock = test_lock();
        struct NoRead;
        impl Memory for NoRead {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("quiet or revoked network read");
            }
        }
        let mut store = Store::new();
        for index in 0..5 {
            store.owners.push(Owner {
                seed: network::Summary {
                    native_pointer: index + 1,
                    ..Default::default()
                },
                ..Default::default()
            });
        }
        store.valid = true;
        store.epoch = EPOCH.load(Ordering::SeqCst);
        store.failure = FAILED.load(Ordering::SeqCst);
        store.published = ORDER.load(Ordering::SeqCst);
        ENABLED.store(true, Ordering::SeqCst);
        let mut rows = Vec::new();
        crate::test_support::with_allocations_denied(|| {
            assert!(matches!(
                capture_store(&mut store, &NoRead, 0, &mut rows),
                Outcome::Unavailable {
                    reason: replay_data::cadence::SKIPPED_REASON
                }
            ))
        });
        store.epoch = store.epoch.wrapping_sub(1);
        assert!(matches!(
            capture_store(&mut store, &NoRead, 0, &mut rows),
            Outcome::Unavailable {
                reason: "network auxiliary lost writer coverage; awaiting existing census"
            }
        ));
        ENABLED.store(false, Ordering::SeqCst);
    }
    #[test]
    fn network_auxiliary_destroy_removes_old_rows_and_new_lifetime_requires_census() {
        let _lock = test_lock();
        struct NoRead;
        impl Memory for NoRead {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("lifetime must not read stale holder");
            }
        }
        let mut store = Store::new();
        store.prepare(2).unwrap();
        store.owners.push(Owner {
            seed: network::Summary {
                native_pointer: 17,
                ..Default::default()
            },
            alive: true,
            entries: vec![(native::Entry::default(), 1)],
            ..Default::default()
        });
        store.valid = true;
        store.epoch = EPOCH.load(Ordering::SeqCst);
        store.failure = FAILED.load(Ordering::SeqCst);
        store.published = ORDER.load(Ordering::SeqCst).wrapping_sub(1);
        let burst = 4 * store.lanes[0].len() + 3;
        for _ in 0..burst {
            store.lanes[0].reserve().unwrap().publish(Event {
                order: ORDER.load(Ordering::SeqCst),
                object: 17,
                lifetime: 2,
            });
        }
        assert_eq!(
            store.lanes[0].published(|event| event.order).unwrap().len(),
            burst
        );
        ENABLED.store(true, Ordering::SeqCst);
        let mut rows = Vec::new();
        assert_eq!(
            capture_store(&mut store, &NoRead, 0, &mut rows),
            Outcome::Rows { len: 0 }
        );
        assert!(store.lanes[0].is_drained());
        assert_eq!(store.lanes[0].rejected(), 0);
        assert!(store.lanes.last().unwrap().len() >= burst);
        store.published = ORDER.load(Ordering::SeqCst).wrapping_sub(1);
        store.lanes[0].reserve().unwrap().publish(Event {
            order: ORDER.load(Ordering::SeqCst),
            object: 17,
            lifetime: 1,
        });
        assert_eq!(
            capture_store(&mut store, &NoRead, 0, &mut rows),
            Outcome::Unavailable {
                reason: "network auxiliary new lifetime awaiting existing census"
            },
        );
        ENABLED.store(false, Ordering::SeqCst);
    }
}
